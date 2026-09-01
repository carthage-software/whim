//! The signature-string `#[whim_enum("Fq\\Name", backing = "int")]` path.

use std::collections::HashSet;

use proc_macro2::Span;
use proc_macro2::TokenStream;
use quote::format_ident;
use quote::quote;
use syn::ItemEnum;
use syn::LitStr;

use crate::built_in;
use crate::built_in::attribute_name;
use crate::built_in::attributes::AttributeArguments;
use crate::built_in::attributes::constant_value_tokens;
use crate::built_in::base_spec;
use crate::built_in::class_constant_spec;
use crate::built_in::empty_method_provider;
use crate::built_in::string_arguments;
use crate::built_in::take_attribute;

pub(super) fn expand(attribute: TokenStream, item: TokenStream) -> syn::Result<TokenStream> {
    let arguments: AttributeArguments = syn::parse2(attribute)?;
    arguments.validate(1, &[], &["backing"])?;
    let declaration = arguments
        .positional_string(0)?
        .ok_or_else(|| syn::Error::new(Span::call_site(), "expected an enum name"))?;
    let (name, _) = built_in::split_name(&declaration.value());
    let name = LitStr::new(&name, declaration.span());

    let backing = match arguments.value_string("backing")?.as_deref() {
        None => quote!(::core::option::Option::None),
        Some("int") => quote!(::core::option::Option::Some(
            crate::bytecode::unit::EnumBacking::Int
        )),
        Some("string") => {
            quote!(::core::option::Option::Some(
                crate::bytecode::unit::EnumBacking::String
            ))
        }
        Some(other) => {
            return Err(syn::Error::new(
                Span::call_site(),
                format!("enum backing must be \"int\" or \"string\", found {other:?}"),
            ));
        }
    };

    let mut definition: ItemEnum = syn::parse2(item)?;
    let enumeration = definition.ident.clone();
    let constructor = format_ident!("__whim_enum_{enumeration}");
    let name_const = format_ident!("__whim_class_name_{enumeration}");
    let method_provider_type = format_ident!("__WhimMethods_{enumeration}");
    let method_provider_definition = empty_method_provider(&enumeration);
    let names = HashSet::new();

    let mut interfaces = Vec::new();
    let mut constants = Vec::new();
    let mut retained = Vec::new();
    for attribute in definition.attrs.drain(..) {
        match attribute_name(&attribute).as_deref() {
            Some("whim_implements") => {
                for entry in string_arguments(&attribute)? {
                    interfaces.push(base_spec(&entry, &names)?);
                }
            }
            Some("whim_class_like_constant") => {
                constants.push(class_constant_spec(
                    &attribute.parse_args::<AttributeArguments>()?,
                )?);
            }
            _ => retained.push(attribute),
        }
    }
    definition.attrs = retained;

    let mut cases = Vec::new();
    for variant in &mut definition.variants {
        let marker = take_attribute(&mut variant.attrs, "whim_case").ok_or_else(|| {
            syn::Error::new(
                Span::call_site(),
                "every enum variant needs #[whim_case(...)]",
            )
        })?;
        let arguments = marker.parse_args::<AttributeArguments>()?;
        arguments.validate(1, &[], &["value"])?;
        let case_name = arguments
            .positional_string(0)?
            .ok_or_else(|| syn::Error::new(Span::call_site(), "an enum case needs a name"))?;
        let value = if let Some(literal) = arguments.value_lit("value")? {
            let value = constant_value_tokens(literal)?;
            quote!(::core::option::Option::Some(#value))
        } else {
            quote!(::core::option::Option::None)
        };
        cases.push(quote! {
            crate::builtin::spec::EnumCaseSpec {
                name: #case_name,
                value: #value,
            }
        });
    }

    definition.attrs.push(syn::parse_quote!(#[allow(
        dead_code,
        reason = "enum declarations exist to generate built-in metadata"
    )]));

    Ok(quote! {
        #definition

        #[doc(hidden)]
        #[allow(non_upper_case_globals)]
        pub(crate) const #name_const: &'static str = #name;

        #method_provider_definition

        #[doc(hidden)]
        pub(crate) fn #constructor() -> crate::builtin::spec::EnumSpec {
            const INTERFACES: &[crate::builtin::spec::BaseSpec] = &[#(#interfaces),*];
            const CASES: &[crate::builtin::spec::EnumCaseSpec] = &[#(#cases),*];
            const CONSTANTS: &[crate::builtin::spec::ClassConstantSpec] = &[#(#constants),*];

            crate::builtin::spec::EnumSpec {
                name: #name,
                interfaces: INTERFACES,
                backing: #backing,
                cases: CASES,
                constants: CONSTANTS,
                methods: #method_provider_type.__whim_methods(),
            }
        }
    })
}
