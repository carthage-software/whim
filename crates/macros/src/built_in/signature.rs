//! Lowering a built-in declaration's Whim signature string into the spec pieces
//! the engine validates against: the type-parameter binders, the parameter
//! table, the return type, and the rendered `fn(...)` string.

use std::collections::HashSet;

use proc_macro2::Span;
use proc_macro2::TokenStream;
use quote::quote;
use whim_syn::arena;
use whim_syn::fragment;

use whim_syn::cst::atom::Literal;
use whim_syn::cst::expression::Expression;
use whim_syn::cst::function::Function;
use whim_syn::cst::function::ParameterDefault;
use whim_syn::cst::function::ParameterList;
use whim_syn::cst::operation::UnaryPrefix;
use whim_syn::cst::operation::UnaryPrefixOperator;
use whim_syn::cst::r#type::TypeParameter;
use whim_syn::cst::r#type::TypeVariance;

use crate::built_in::generics;
use crate::built_in::type_spec;

pub(super) struct LoweredSignature {
    /// A `&'static [TypeParameterSpec]` expression.
    pub type_parameters: TokenStream,
    /// A `&'static [ParameterSpec]` expression.
    pub parameters: TokenStream,
    /// A `TypeSpec` expression.
    pub return_spec: TokenStream,
    /// The rendered `fn(...): ret` type string.
    pub rendered: String,
    /// The number of declared parameters, for splitting a closure's window.
    pub parameter_count: usize,
    /// The number of type parameters declared by the signature itself.
    pub type_parameter_count: usize,
    /// The written return type, or `None` when the declaration omitted it.
    pub return_type: Option<String>,
}

pub(super) fn empty() -> LoweredSignature {
    LoweredSignature {
        type_parameters: quote!(&[]),
        parameters: quote!(&[]),
        return_spec: quote!(crate::builtin::spec::TypeSpec::Mixed),
        rendered: "fn(): mixed".to_owned(),
        parameter_count: 0,
        type_parameter_count: 0,
        return_type: None,
    }
}

pub(super) fn lower(source: &str, inherited: &HashSet<String>) -> syn::Result<LoweredSignature> {
    let arena = arena::LocalArena::new();
    let function = fragment::parse_signature(&arena, source).map_err(|error| {
        syn::Error::new(
            Span::call_site(),
            format!("invalid signature `{source}`: {error}"),
        )
    })?;

    lower_function(function, inherited)
}

fn lower_function(
    function: &Function<'_>,
    inherited: &HashSet<String>,
) -> syn::Result<LoweredSignature> {
    let mut names = inherited.clone();
    if let Some(list) = function.type_parameters.as_ref() {
        for parameter in list.parameters {
            names.insert(parameter.name.value.to_owned());
        }
    }

    let type_parameters = if let Some(list) = function.type_parameters.as_ref() {
        generics::lower_list(list, &names)?
    } else {
        quote!(&[])
    };
    let parameters = lower_parameters(&function.parameter_list, &names)?;
    let return_spec = if let Some(return_type) = function.return_type.as_ref() {
        type_spec::type_spec(return_type.r#type, &names)?
    } else {
        quote!(crate::builtin::spec::TypeSpec::Mixed)
    };
    let rendered = render_signature(function);
    let parameter_count = function.parameter_list.parameters.len();
    let type_parameter_count = function
        .type_parameters
        .as_ref()
        .map_or(0, |parameters| parameters.parameters.len());
    let return_type = function
        .return_type
        .as_ref()
        .map(|return_type| type_spec::render(return_type.r#type));

    Ok(LoweredSignature {
        type_parameters,
        parameters,
        return_spec,
        rendered,
        parameter_count,
        type_parameter_count,
        return_type,
    })
}

fn lower_parameters(list: &ParameterList<'_>, names: &HashSet<String>) -> syn::Result<TokenStream> {
    let specs = list
        .parameters
        .iter()
        .map(|parameter| {
            let name = parameter.variable.name.trim_start_matches('$');
            let type_spec = if let Some(subject) = parameter.r#type {
                type_spec::type_spec(subject, names)?
            } else {
                quote!(crate::builtin::spec::TypeSpec::Mixed)
            };
            let optional = parameter.default.is_some();
            let default = parameter
                .default
                .as_ref()
                .map_or_else(|| Ok(quote!(None)), lower_parameter_default)?;
            let sensitive = parameter.attribute_lists.iter().any(|list| {
                list.attributes
                    .iter()
                    .any(|attribute| attribute.name.last_segment() == "SensitiveParameter")
            });
            Ok(quote!(crate::builtin::spec::ParameterSpec {
                name: #name,
                type_spec: #type_spec,
                optional: #optional,
                default: #default,
                sensitive: #sensitive,
            }))
        })
        .collect::<syn::Result<Vec<_>>>()?;

    Ok(quote!(&[#(#specs),*]))
}

fn lower_parameter_default(default: &ParameterDefault<'_>) -> syn::Result<TokenStream> {
    let value = match default.value.unparenthesized() {
        Expression::Literal(Literal::Null(_)) => {
            quote!(crate::builtin::spec::ParameterDefaultSpec::Null)
        }
        Expression::Literal(Literal::True(_)) => {
            quote!(crate::builtin::spec::ParameterDefaultSpec::Bool(true))
        }
        Expression::Literal(Literal::False(_)) => {
            quote!(crate::builtin::spec::ParameterDefaultSpec::Bool(false))
        }
        Expression::Literal(Literal::Integer(integer)) => {
            let value = i64::try_from(integer.value).map_err(|_| {
                syn::Error::new(
                    Span::call_site(),
                    "the parameter default does not fit in an int",
                )
            })?;
            quote!(crate::builtin::spec::ParameterDefaultSpec::Int(#value))
        }
        Expression::Literal(Literal::Float(float)) => {
            let value = float.value;
            quote!(crate::builtin::spec::ParameterDefaultSpec::Float(#value))
        }
        Expression::Literal(Literal::String(string)) => {
            let value = proc_macro2::Literal::byte_string(string.value);
            quote!(crate::builtin::spec::ParameterDefaultSpec::String(#value))
        }
        Expression::UnaryPrefix(prefix)
            if matches!(
                prefix.operator,
                UnaryPrefixOperator::Plus(_) | UnaryPrefixOperator::Negation(_)
            ) =>
        {
            lower_signed_parameter_default(prefix)?
        }
        _ => {
            return Err(syn::Error::new(
                Span::call_site(),
                "a built-in parameter default must be a scalar literal or null",
            ));
        }
    };

    Ok(quote!(Some(#value)))
}

fn lower_signed_parameter_default(prefix: &UnaryPrefix<'_>) -> syn::Result<TokenStream> {
    let negative = matches!(prefix.operator, UnaryPrefixOperator::Negation(_));
    match prefix.operand.unparenthesized() {
        Expression::Literal(Literal::Integer(integer)) => {
            let magnitude = i128::from(integer.value);
            let value =
                i64::try_from(if negative { -magnitude } else { magnitude }).map_err(|_| {
                    syn::Error::new(
                        Span::call_site(),
                        "the parameter default does not fit in an int",
                    )
                })?;
            Ok(quote!(crate::builtin::spec::ParameterDefaultSpec::Int(#value)))
        }
        Expression::Literal(Literal::Float(float)) => {
            let value = if negative { -float.value } else { float.value };
            Ok(quote!(crate::builtin::spec::ParameterDefaultSpec::Float(#value)))
        }
        _ => Err(syn::Error::new(
            Span::call_site(),
            "a signed built-in parameter default must be an int or float literal",
        )),
    }
}

fn render_signature(function: &Function<'_>) -> String {
    let binders = function
        .type_parameters
        .as_ref()
        .map_or_else(String::new, |list| {
            let parameters = list
                .parameters
                .iter()
                .map(render_type_parameter)
                .collect::<Vec<_>>()
                .join(", ");
            format!("<{parameters}>")
        });
    let parameters = function
        .parameter_list
        .parameters
        .iter()
        .map(|parameter| {
            let prefix = if parameter.default.is_some() { "=" } else { "" };
            let rendered = parameter
                .r#type
                .map_or_else(|| "mixed".to_owned(), type_spec::render);
            format!("{prefix}{rendered}")
        })
        .collect::<Vec<_>>()
        .join(", ");

    let return_type = function.return_type.as_ref().map_or_else(
        || "mixed".to_owned(),
        |return_type| type_spec::render(return_type.r#type),
    );

    format!("fn{binders}({parameters}): {return_type}")
}

fn render_type_parameter(parameter: &TypeParameter<'_>) -> String {
    let variance = match parameter.variance {
        Some(TypeVariance::In(_)) => "in ",
        Some(TypeVariance::Out(_)) => "out ",
        None => "",
    };
    let bound = parameter.bound.as_ref().map_or_else(String::new, |bound| {
        let types = bound
            .types
            .iter()
            .map(|subject| type_spec::render(subject))
            .collect::<Vec<_>>()
            .join(" + ");
        format!(": {types}")
    });
    let default = parameter
        .default
        .as_ref()
        .map_or_else(String::new, |default| {
            format!(" = {}", type_spec::render(default.r#type))
        });

    format!("{variance}{}{bound}{default}", parameter.name.value)
}
