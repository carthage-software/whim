//! The signature-string `#[whim_newtype("Whim\\Type\\TypeId", "0..")]` path.

use proc_macro2::Span;
use proc_macro2::TokenStream;
use quote::format_ident;
use quote::quote;
use syn::ItemStruct;
use whim_syn::arena::LocalArena;
use whim_syn::fragment;

use crate::built_in::attributes::AttributeArguments;
use crate::built_in::generics;
use crate::built_in::split_name;
use crate::built_in::type_spec;

pub(super) fn expand(attribute: TokenStream, item: TokenStream) -> syn::Result<TokenStream> {
    let arguments: AttributeArguments = syn::parse2(attribute)?;
    arguments.validate(2, &[], &[])?;
    let declaration = arguments
        .positional_string(0)?
        .ok_or_else(|| syn::Error::new(Span::call_site(), "expected a newtype name"))?;
    let backing = arguments
        .positional_string(1)?
        .ok_or_else(|| syn::Error::new(declaration.span(), "expected a newtype backing type"))?;
    let (name, generics_tail) = split_name(&declaration.value());
    let generics = generics::lower(&generics_tail)?;
    let arena = LocalArena::new();
    let parsed = fragment::parse_type(&arena, &backing.value()).map_err(|error| {
        syn::Error::new(
            backing.span(),
            format!("invalid newtype backing type: {error}"),
        )
    })?;
    let backing = type_spec::type_spec(parsed, &generics.names)?;

    let mut definition: ItemStruct = syn::parse2(item)?;
    if !definition.fields.is_empty() {
        return Err(syn::Error::new_spanned(
            definition.fields,
            "a built-in newtype declaration must be fieldless",
        ));
    }
    definition.attrs.push(syn::parse_quote!(#[allow(
        dead_code,
        reason = "fieldless declarations exist to generate built-in metadata"
    )]));
    let representation = &definition.ident;
    let constructor = format_ident!("__whim_newtype_{representation}");
    let type_parameters = generics.spec;

    Ok(quote! {
        #definition

        pub(crate) fn #constructor() -> crate::builtin::spec::NewtypeSpec {
            crate::builtin::spec::NewtypeSpec {
                name: #name,
                type_parameters: #type_parameters,
                backing: #backing,
            }
        }
    })
}
