//! The signature-string `#[whim_function("Fq\\name")]` path.

use std::collections::HashSet;

use proc_macro2::Span;
use proc_macro2::TokenStream;
use quote::format_ident;
use quote::quote;
use syn::Ident;
use syn::ItemFn;
use syn::LitStr;
use syn::Token;
use syn::parse::Parse;
use syn::parse::ParseStream;

use crate::built_in::callable_marker_tokens;
use crate::built_in::handler;
use crate::built_in::handler::Receiver;
use crate::built_in::mark_must_use;
use crate::built_in::signature;
use crate::built_in::split_name;
use crate::built_in::validate_must_use;

pub(super) fn expand(attribute: TokenStream, item: TokenStream) -> syn::Result<TokenStream> {
    let FunctionAttribute {
        declaration,
        no_track_caller,
        no_trace_boundary,
        must_use,
    } = syn::parse2(attribute)?;
    let (name, tail) = split_name(&declaration.value());
    let function: ItemFn = syn::parse2(item)?;

    let lowered = if tail.is_empty() {
        signature::empty()
    } else {
        signature::lower(&tail, &HashSet::new())?
    };
    validate_must_use(must_use, lowered.return_type.as_deref(), declaration.span())?;
    let name = LitStr::new(&name, declaration.span());

    let function_identifier = function.sig.ident.clone();
    let handler_identifier = format_ident!("__whim_handler_{}", function_identifier);
    let direct_handler_identifier = format_ident!("__whim_direct_handler_{}", function_identifier);
    let constructor_identifier = format_ident!("__whim_function_{}", function_identifier);

    let body = handler::shim_body(
        &quote!(#function_identifier),
        &function.sig.inputs,
        &function.sig.output,
        Receiver::None,
    )?;

    let type_parameters = &lowered.type_parameters;
    let parameters = &lowered.parameters;
    let return_spec = &lowered.return_spec;
    let rendered = LitStr::new(&lowered.rendered, Span::call_site());
    let markers = callable_marker_tokens(no_track_caller, no_trace_boundary, must_use);
    let body = mark_must_use(body, &name, must_use);

    Ok(quote! {
        #function

        fn #handler_identifier<'call>(
            __whim_scope: &mut crate::builtin::Context<'call, '_, '_>,
            __whim_window: &'call [crate::value::Value],
        ) -> ::core::result::Result<
            crate::value::Value,
            crate::builtin::throw::Throw,
        > {
            #body
        }

        fn #direct_handler_identifier(
            __whim_vm: &mut crate::vm::VirtualMachine<'_>,
            __whim_values: &[crate::value::Value],
        ) -> ::core::result::Result<
            crate::value::Value,
            crate::builtin::throw::Throw,
        > {
            crate::builtin::invoke_direct_built_in(
                __whim_vm,
                __whim_values,
                |__whim_scope, __whim_window| {
                    #body
                },
            )
        }

        pub(crate) fn #constructor_identifier() -> crate::builtin::spec::FunctionDeclaration {
            crate::builtin::spec::FunctionDeclaration {
                callable: crate::builtin::spec::FunctionSpec {
                    name: #name,
                    type_parameters: #type_parameters,
                    parameters: #parameters,
                    return_spec: #return_spec,
                    handler: #handler_identifier,
                    direct_handler: ::core::option::Option::Some(#direct_handler_identifier),
                    signature: #rendered,
                },
                markers: #markers,
            }
        }
    })
}

struct FunctionAttribute {
    declaration: LitStr,
    no_track_caller: bool,
    no_trace_boundary: bool,
    must_use: bool,
}

impl Parse for FunctionAttribute {
    fn parse(input: ParseStream<'_>) -> syn::Result<Self> {
        let declaration = input.parse()?;
        let mut no_track_caller = false;
        let mut no_trace_boundary = false;
        let mut must_use = false;
        while !input.is_empty() {
            input.parse::<Token![,]>()?;
            let option: Ident = input.parse()?;
            match option.to_string().as_str() {
                "no_track_caller" if !no_track_caller => no_track_caller = true,
                "no_trace_boundary" if !no_trace_boundary => no_trace_boundary = true,
                "must_use" if !must_use => must_use = true,
                "no_track_caller" | "no_trace_boundary" | "must_use" => {
                    return Err(syn::Error::new(
                        option.span(),
                        format!("duplicate option `{option}`"),
                    ));
                }
                _ => {
                    return Err(syn::Error::new(
                        option.span(),
                        "unknown whim_function option",
                    ));
                }
            }
        }

        Ok(Self {
            declaration,
            no_track_caller,
            no_trace_boundary,
            must_use,
        })
    }
}
