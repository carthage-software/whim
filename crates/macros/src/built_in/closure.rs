//! The `#[whim_closure("(sig): ret")]` path for closures returned by built-in
//! factories.

use proc_macro2::Span;
use proc_macro2::TokenStream;
use quote::format_ident;
use quote::quote;
use syn::ItemFn;
use syn::LitStr;

use crate::built_in::attributes::AttributeArguments;
use crate::built_in::generics;
use crate::built_in::handler;
use crate::built_in::handler::Receiver;
use crate::built_in::signature;

pub(super) fn expand(attribute: TokenStream, item: TokenStream) -> syn::Result<TokenStream> {
    let arguments: AttributeArguments = syn::parse2(attribute)?;
    arguments.validate(1, &[], &["generics"])?;
    let declaration = arguments
        .positional_string(0)?
        .ok_or_else(|| syn::Error::new(Span::call_site(), "a closure needs a signature"))?;
    let generics = match arguments.value_string("generics")? {
        Some(tail) => generics::lower(&tail)?,
        None => generics::none(),
    };
    let lowered = signature::lower(&declaration.value(), &generics.names)?;

    let function: ItemFn = syn::parse2(item)?;
    let function_identifier = function.sig.ident.clone();
    let handler_identifier = format_ident!("__whim_closure_handler_{}", function_identifier);
    let spec_identifier = format_ident!("{}_spec", function_identifier);

    let body = handler::shim_body(
        &quote!(#function_identifier),
        &function.sig.inputs,
        &function.sig.output,
        Receiver::Closure {
            parameters: lowered.parameter_count,
        },
    )?;

    let type_parameters = &lowered.type_parameters;
    let parameters = &lowered.parameters;
    let return_spec = &lowered.return_spec;
    let rendered = LitStr::new(&lowered.rendered, Span::call_site());
    let name = LitStr::new(&function_identifier.to_string(), Span::call_site());

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

        #[doc(hidden)]
        pub(crate) fn #spec_identifier() -> crate::builtin::spec::FunctionSpec {
            crate::builtin::spec::FunctionSpec {
                name: #name,
                type_parameters: #type_parameters,
                parameters: #parameters,
                return_spec: #return_spec,
                handler: #handler_identifier,
                direct_handler: ::core::option::Option::None,
                signature: #rendered,
            }
        }
    })
}
