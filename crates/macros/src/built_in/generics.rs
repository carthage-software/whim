use std::collections::HashSet;

use proc_macro2::Span;
use proc_macro2::TokenStream;
use quote::quote;
use whim_syn::arena::LocalArena;
use whim_syn::fragment::parse_type_parameters;

use whim_syn::cst::r#type::TypeParameterList;
use whim_syn::cst::r#type::TypeVariance;

use crate::built_in::type_spec;

pub(super) struct Generics {
    pub spec: TokenStream,
    pub names: HashSet<String>,
}

pub(super) fn none() -> Generics {
    Generics {
        spec: quote!(&[]),
        names: HashSet::new(),
    }
}

pub(super) fn lower(tail: &str) -> syn::Result<Generics> {
    if tail.trim().is_empty() {
        return Ok(none());
    }

    let arena = LocalArena::new();
    let list = parse_type_parameters(&arena, tail).map_err(|error| {
        syn::Error::new(
            Span::call_site(),
            format!("invalid type parameters `{tail}`: {error}"),
        )
    })?;

    let names = list
        .parameters
        .iter()
        .map(|parameter| parameter.name.value.to_owned())
        .collect::<HashSet<_>>();
    let spec = lower_list(list, &names)?;

    Ok(Generics { spec, names })
}

/// Lowers a parsed type-parameter list into a `&[TypeParameterSpec]` expression.
/// `names` holds every parameter in scope, so a bound may reference a sibling.
pub(super) fn lower_list(
    list: &TypeParameterList<'_>,
    names: &HashSet<String>,
) -> syn::Result<TokenStream> {
    let specs = list
        .parameters
        .iter()
        .map(|parameter| {
            let name = parameter.name.value;
            let variance = match parameter.variance {
                None => quote!(crate::bytecode::unit::Variance::Invariant),
                Some(TypeVariance::Out(_)) => {
                    quote!(crate::bytecode::unit::Variance::Covariant)
                }
                Some(TypeVariance::In(_)) => {
                    quote!(crate::bytecode::unit::Variance::Contravariant)
                }
            };
            let bounds = match parameter.bound.as_ref() {
                None => quote!(&[]),
                Some(bound) => {
                    let specs = bound
                        .types
                        .iter()
                        .map(|bound_type| type_spec::type_spec(bound_type, names))
                        .collect::<syn::Result<Vec<_>>>()?;
                    quote!(&[#(#specs),*])
                }
            };
            let default = match parameter.default.as_ref() {
                None => quote!(::core::option::Option::None),
                Some(default) => {
                    let spec = type_spec::type_spec(default.r#type, names)?;
                    quote!(::core::option::Option::Some(#spec))
                }
            };
            Ok(quote!(crate::builtin::spec::TypeParameterSpec {
                name: #name,
                variance: #variance,
                bounds: #bounds,
                default: #default,
            }))
        })
        .collect::<syn::Result<Vec<_>>>()?;

    Ok(quote!(&[#(#specs),*]))
}
