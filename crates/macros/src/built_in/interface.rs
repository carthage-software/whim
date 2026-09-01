//! The signature-string `#[whim_interface("Whim\\Result\\Result<out T, out E>")]`
//! path.

use proc_macro2::Span;
use proc_macro2::TokenStream;
use quote::format_ident;
use quote::quote;
use syn::ItemTrait;
use syn::LitStr;
use syn::TraitItem;
use syn::token::Semi;

use crate::built_in::attribute_name;
use crate::built_in::attributes::AttributeArguments;
use crate::built_in::base_spec;
use crate::built_in::callable_markers;
use crate::built_in::class::property_spec;
use crate::built_in::class_constant_spec;
use crate::built_in::generics;
use crate::built_in::handler;
use crate::built_in::handler::Receiver;
use crate::built_in::mark_must_use;
use crate::built_in::signature;
use crate::built_in::split_name;
use crate::built_in::string_arguments;
use crate::built_in::take_attribute;
use crate::built_in::validate_must_use;

pub(super) fn expand(attribute: TokenStream, item: TokenStream) -> syn::Result<TokenStream> {
    let arguments: AttributeArguments = syn::parse2(attribute)?;
    arguments.validate(1, &[], &[])?;
    let declaration = arguments
        .positional_string(0)?
        .ok_or_else(|| syn::Error::new(Span::call_site(), "expected an interface name"))?;
    let (name, generics_tail) = split_name(&declaration.value());
    let generics = generics::lower(&generics_tail)?;
    let name = LitStr::new(&name, declaration.span());

    let mut definition: ItemTrait = syn::parse2(item)?;
    let interface = definition.ident.clone();
    let constructor = format_ident!("__whim_interface_{interface}");
    let mut extends = Vec::new();
    let mut permits = Vec::new();
    let mut constants = Vec::new();
    let mut properties = Vec::new();
    let mut retained = Vec::new();
    for attribute in definition.attrs.drain(..) {
        match attribute_name(&attribute).as_deref() {
            Some("whim_extends") => {
                for entry in string_arguments(&attribute)? {
                    extends.push(base_spec(&entry, &generics.names)?);
                }
            }
            Some("whim_permits") => {
                permits.extend(string_arguments(&attribute)?);
            }
            Some("whim_class_like_constant") => {
                constants.push(class_constant_spec(
                    &attribute.parse_args::<AttributeArguments>()?,
                )?);
            }
            Some("whim_property") => {
                let source = attribute.parse_args::<LitStr>()?;
                properties.push(property_spec(&source, false, &generics.names)?);
            }
            _ => retained.push(attribute),
        }
    }
    definition.attrs = retained;

    let mut lifted = Vec::new();
    let mut specs = Vec::new();
    for item in &mut definition.items {
        let TraitItem::Fn(method) = item else {
            continue;
        };
        let Some(marker) = take_attribute(&mut method.attrs, "whim_method") else {
            continue;
        };
        let arguments = marker.parse_args::<AttributeArguments>()?;
        arguments.validate(
            1,
            &["static", "must_use", "no_track_caller", "no_trace_boundary"],
            &[],
        )?;
        let markers = callable_markers(&arguments);
        let declaration = arguments.positional_string(0)?.ok_or_else(|| {
            syn::Error::new(Span::call_site(), "an interface method needs a declaration")
        })?;
        let (method_name, tail) = split_name(&declaration.value());
        let method_name_literal = LitStr::new(&method_name, declaration.span());
        let is_static = arguments.has_flag("static");
        let lowered = if tail.is_empty() {
            signature::empty()
        } else {
            signature::lower(&tail, &generics.names)?
        };
        validate_must_use(
            arguments.has_flag("must_use"),
            lowered.return_type.as_deref(),
            declaration.span(),
        )?;

        let default_handler = if let Some(body) = method.default.take() {
            let method_identifier = method.sig.ident.clone();
            let lifted_identifier =
                format_ident!("__whim_interface_body_{interface}_{method_identifier}");
            let shim_identifier =
                format_ident!("__whim_interface_default_{interface}_{method_identifier}");
            let receiver = if is_static {
                Receiver::Static
            } else {
                Receiver::Instance
            };
            let shim = handler::shim_body(
                &quote!(#lifted_identifier),
                &method.sig.inputs,
                &method.sig.output,
                receiver,
            )?;
            let shim = mark_must_use(shim, &method_name_literal, arguments.has_flag("must_use"));

            let mut lifted_signature = method.sig.clone();
            lifted_signature.ident = lifted_identifier;
            method.semi_token = Some(Semi::default());
            lifted.push(quote! {
                #[doc(hidden)]
                #lifted_signature #body

                #[doc(hidden)]
                fn #shim_identifier<'call>(
                    __whim_scope: &mut crate::builtin::Context<'call, '_, '_>,
                    __whim_window: &'call [crate::value::Value],
                ) -> ::core::result::Result<
                    crate::value::Value,
                    crate::builtin::throw::Throw,
                > {
                    #shim
                }
            });
            quote!(::core::option::Option::Some(#shim_identifier))
        } else {
            quote!(::core::option::Option::None)
        };

        let type_parameters = &lowered.type_parameters;
        let parameters = &lowered.parameters;
        let return_spec = &lowered.return_spec;
        let rendered = LitStr::new(&lowered.rendered, Span::call_site());
        specs.push(quote! {
            crate::builtin::spec::InterfaceMethodSpec {
                name: #method_name_literal,
                is_static: #is_static,
                type_parameters: #type_parameters,
                parameters: #parameters,
                return_spec: #return_spec,
                signature: #rendered,
                default_handler: #default_handler,
                markers: #markers,
            }
        });
    }

    definition.attrs.push(syn::parse_quote!(#[allow(
        dead_code,
        reason = "interface declarations exist to generate built-in metadata"
    )]));

    let type_parameters = &generics.spec;
    let sealed_to = if permits.is_empty() {
        quote!(::core::option::Option::None)
    } else {
        quote!(::core::option::Option::Some(&[#(#permits),*] as &[&str]))
    };

    Ok(quote! {
        #definition

        #(#lifted)*

        #[doc(hidden)]
        pub(crate) fn #constructor() -> crate::builtin::spec::InterfaceSpec {
            const EXTENDS: &[crate::builtin::spec::BaseSpec] = &[#(#extends),*];
            const CONSTANTS: &[crate::builtin::spec::ClassConstantSpec] = &[#(#constants),*];
            const PROPERTIES: &[crate::builtin::spec::PropertySpec] = &[#(#properties),*];

            crate::builtin::spec::InterfaceSpec {
                name: #name,
                type_parameters: #type_parameters,
                extends: EXTENDS,
                sealed_to: #sealed_to,
                constants: CONSTANTS,
                properties: PROPERTIES,
                methods: ::std::boxed::Box::from([#(#specs),*]),
            }
        }
    })
}
