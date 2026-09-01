//! The `#[whim_constant("Fq\\NAME", "type")]` path.

use proc_macro2::Span;
use proc_macro2::TokenStream;
use quote::format_ident;
use quote::quote;
use syn::ItemConst;
use syn::LitStr;
use syn::parse::Parser;
use syn::punctuated::Punctuated;
use syn::token::Comma;
use whim_syn::arena;
use whim_syn::fragment;

use whim_syn::cst::r#type::Type;

pub(super) fn expand(attribute: TokenStream, item: TokenStream) -> syn::Result<TokenStream> {
    let arguments = Punctuated::<LitStr, Comma>::parse_terminated.parse2(attribute)?;
    let mut arguments = arguments.into_iter();
    let name = arguments
        .next()
        .ok_or_else(|| syn::Error::new(Span::call_site(), "expected a constant name"))?;
    let type_string = arguments.next().ok_or_else(|| {
        syn::Error::new(
            name.span(),
            "expected a type after the name, e.g. #[whim_constant(\"Whim\\\\Math\\\\PI\", \"float\")]",
        )
    })?;
    if let Some(extra) = arguments.next() {
        return Err(syn::Error::new_spanned(
            extra,
            "unexpected positional argument",
        ));
    }

    let constant: ItemConst = syn::parse2(item)?;
    let identifier = &constant.ident;
    let value = constant_value(&type_string, identifier)?;
    let constructor = format_ident!("__whim_constant_{}", identifier);

    Ok(quote! {
        #constant

        #[doc(hidden)]
        #[allow(non_upper_case_globals)]
        pub(crate) const #constructor: crate::builtin::spec::ConstantSpec =
            crate::builtin::spec::ConstantSpec {
                name: #name,
                value: #value,
            };
    })
}

fn constant_value(type_string: &LitStr, identifier: &syn::Ident) -> syn::Result<TokenStream> {
    let arena = arena::LocalArena::new();
    let parsed = fragment::parse_type(&arena, &type_string.value()).map_err(|error| {
        syn::Error::new(
            type_string.span(),
            format!("invalid constant type: {error}"),
        )
    })?;

    let path = quote!(crate::builtin::spec::ConstantValue);
    let value = match parsed {
        Type::Int(_) => quote!(#path::Int(#identifier)),
        Type::Float(_) => quote!(#path::Float(#identifier)),
        Type::Bool(_) => quote!(#path::Bool(#identifier)),
        Type::String(_) => quote!(#path::String(#identifier)),
        _ => {
            return Err(syn::Error::new(
                type_string.span(),
                "a built-in constant is int, float, bool, or string",
            ));
        }
    };

    Ok(value)
}
