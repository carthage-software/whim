//! The signature-string `#[whim_methods]` path for a built-in class `impl`.

use proc_macro2::Span;
use proc_macro2::TokenStream;
use quote::format_ident;
use quote::quote;
use syn::ImplItem;
use syn::ItemImpl;
use syn::LitStr;
use syn::Type;

use crate::built_in::attributes::AttributeArguments;
use crate::built_in::attributes::visibility_tokens;
use crate::built_in::callable_markers;
use crate::built_in::generics;
use crate::built_in::handler;
use crate::built_in::handler::Receiver;
use crate::built_in::mark_must_use;
use crate::built_in::signature;
use crate::built_in::split_name;
use crate::built_in::take_attribute;
use crate::built_in::validate_must_use;

pub(super) fn expand(attribute: TokenStream, item: TokenStream) -> syn::Result<TokenStream> {
    let generics = parse_generics(&attribute)?;
    let mut implementation: ItemImpl = syn::parse2(item)?;
    let representation = self_type_identifier(&implementation.self_ty)?;
    let provider = format_ident!("__WhimMethods_{representation}");

    let mut shims = Vec::new();
    let mut specs = Vec::new();
    for item in &mut implementation.items {
        let ImplItem::Fn(method) = item else {
            continue;
        };
        let Some(marker) = take_attribute(&mut method.attrs, "whim_method") else {
            continue;
        };
        let arguments = marker.parse_args::<AttributeArguments>()?;
        arguments.validate(
            1,
            &["static", "must_use", "no_track_caller", "no_trace_boundary"],
            &["visibility"],
        )?;
        let declaration = arguments
            .positional_string(0)?
            .ok_or_else(|| syn::Error::new(Span::call_site(), "a method needs a declaration"))?;
        let (name, tail) = split_name(&declaration.value());
        let is_static = arguments.has_flag("static");
        let visibility = visibility_tokens(arguments.value_string("visibility")?.as_deref())?;
        let markers = callable_markers(&arguments);

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
        if name == "__destruct" {
            if is_static {
                return Err(syn::Error::new(
                    declaration.span(),
                    "a destructor cannot be static",
                ));
            }
            if arguments
                .value_string("visibility")?
                .is_some_and(|visibility| visibility != "public")
            {
                return Err(syn::Error::new(
                    declaration.span(),
                    "a destructor must be public",
                ));
            }
            if lowered.parameter_count != 0 {
                return Err(syn::Error::new(
                    declaration.span(),
                    "a destructor cannot declare parameters",
                ));
            }
            if lowered.type_parameter_count != 0 {
                return Err(syn::Error::new(
                    declaration.span(),
                    "a destructor cannot declare type parameters",
                ));
            }
            if lowered
                .return_type
                .as_deref()
                .is_some_and(|return_type| return_type != "void")
            {
                return Err(syn::Error::new(
                    declaration.span(),
                    "a destructor may only declare the return type void",
                ));
            }
        }

        let method_identifier = method.sig.ident.clone();
        let shim_identifier = format_ident!("__whim_method_{representation}_{method_identifier}");
        let receiver = if is_static {
            Receiver::Static
        } else {
            Receiver::Instance
        };
        let body = handler::shim_body(
            &quote!(#representation::#method_identifier),
            &method.sig.inputs,
            &method.sig.output,
            receiver,
        )?;

        let name = LitStr::new(&name, declaration.span());
        let body = mark_must_use(body, &name, arguments.has_flag("must_use"));
        shims.push(quote! {
            fn #shim_identifier<'call>(
                __whim_scope: &mut crate::builtin::Context<'call, '_, '_>,
                __whim_window: &'call [crate::value::Value],
            ) -> ::core::result::Result<
                crate::value::Value,
                crate::builtin::throw::Throw,
            > {
                #body
            }
        });

        let type_parameters = &lowered.type_parameters;
        let parameters = &lowered.parameters;
        let return_spec = &lowered.return_spec;
        let rendered = LitStr::new(&lowered.rendered, Span::call_site());
        specs.push(quote! {
            crate::builtin::spec::MethodSpec {
                name: #name,
                visibility: #visibility,
                is_static: #is_static,
                type_parameters: #type_parameters,
                parameters: #parameters,
                return_spec: #return_spec,
                handler: #shim_identifier,
                markers: #markers,
                signature: #rendered,
            }
        });
    }

    Ok(quote! {
        #implementation

        #(#shims)*

        impl #provider {
            #[doc(hidden)]
            pub(crate) fn __whim_methods(&self)
            -> ::std::boxed::Box<[crate::builtin::spec::MethodSpec]> {
                ::std::boxed::Box::from([#(#specs),*])
            }
        }
    })
}

fn parse_generics(attribute: &TokenStream) -> syn::Result<generics::Generics> {
    if attribute.is_empty() {
        return Ok(generics::none());
    }

    let arguments: AttributeArguments = syn::parse2(attribute.clone())?;
    arguments.validate(0, &[], &["generics"])?;
    match arguments.value_string("generics")? {
        Some(tail) => generics::lower(&tail),
        None => Ok(generics::none()),
    }
}

fn self_type_identifier(self_type: &Type) -> syn::Result<syn::Ident> {
    match self_type {
        Type::Path(path) => path
            .path
            .segments
            .last()
            .map(|segment| segment.ident.clone())
            .ok_or_else(|| syn::Error::new(Span::call_site(), "an impl needs a named self type")),
        _ => Err(syn::Error::new(
            Span::call_site(),
            "whim_methods applies to an inherent impl of a built-in class representation",
        )),
    }
}
