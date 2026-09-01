//! Lowering a parsed Whim type into the `TypeSpec` the engine validates
//! against, and rendering it back to canonical diagnostic text.

use std::collections::HashSet;

use proc_macro2::Span;
use proc_macro2::TokenStream;
use quote::quote;

use whim_syn::cst::atom::Literal;
use whim_syn::cst::r#type::ArrayType;
use whim_syn::cst::r#type::DictType;
use whim_syn::cst::r#type::FunctionType;
use whim_syn::cst::r#type::IntegerRangeBound;
use whim_syn::cst::r#type::IntegerRangeOperator;
use whim_syn::cst::r#type::IntegerRangeType;
use whim_syn::cst::r#type::NamedType;
use whim_syn::cst::r#type::NegatedType;
use whim_syn::cst::r#type::NegativeLiteralType;
use whim_syn::cst::r#type::TupleType;
use whim_syn::cst::r#type::Type;
use whim_syn::cst::r#type::VecType;

pub(super) fn type_spec(
    subject: &Type<'_>,
    parameters: &HashSet<String>,
) -> syn::Result<TokenStream> {
    let path = quote!(crate::builtin::spec::TypeSpec);

    let spec = match subject {
        Type::Int(_) => quote!(#path::Int),
        Type::IntegerRange(range) => integer_range_spec(range, &path)?,
        Type::Float(_) => quote!(#path::Float),
        Type::Bool(_) => quote!(#path::Bool),
        Type::String(_) => quote!(#path::String),
        Type::Mixed(_) => quote!(#path::Mixed),
        Type::Void(_) => quote!(#path::Void),
        Type::Never(_) => quote!(#path::Never),
        Type::Object(_) => quote!(#path::Object),
        Type::Static(_) => quote!(#path::Static),
        Type::Self_(self_type) if self_type.member.is_none() => quote!(#path::Static),
        Type::Self_(_) => {
            return Err(unsupported(
                subject,
                "a member is not a built-in signature type",
            ));
        }
        Type::Parent(_) => {
            return Err(unsupported(
                subject,
                "`parent` is not a built-in signature type",
            ));
        }
        Type::Parenthesized(inner) => type_spec(inner.r#type, parameters)?,
        Type::Array(array) => array_spec(array, parameters, &path)?,
        Type::Vec(vector) => vector_spec(vector, parameters, &path)?,
        Type::Dict(dictionary) => dictionary_spec(dictionary, parameters, &path)?,
        Type::Tuple(tuple) => tuple_spec(tuple, parameters, &path)?,
        Type::Union(_) => composite_spec(subject, Composite::Union, parameters, &path)?,
        Type::Intersection(_) => {
            composite_spec(subject, Composite::Intersection, parameters, &path)?
        }
        Type::Negated(negated) => negated_spec(negated, parameters, &path)?,
        Type::Classname(classname) => {
            let inner = type_spec(classname.inner, parameters)?;
            quote!(#path::Classname(&#inner))
        }
        Type::Function(function) => function_spec(function, parameters, &path)?,
        Type::Named(named) if named.identifier.value() == "_" && named.type_arguments.is_none() => {
            quote!(#path::Wildcard)
        }
        Type::Named(named) => named_spec(named, parameters, &path)?,
        Type::VecShape(_) | Type::DictShape(_) => {
            return Err(unsupported(
                subject,
                "shape types are not supported in built-in signatures",
            ));
        }
        Type::Literal(Literal::Null(_)) => quote!(#path::Null),
        Type::Literal(Literal::String(string)) => {
            let value = syn::LitByteStr::new(string.value, Span::call_site());
            quote!(#path::StringLiteral(#value))
        }
        Type::Literal(_) | Type::NegativeLiteral(_) => {
            return Err(unsupported(
                subject,
                "only `null` and string literals are built-in signature types",
            ));
        }
    };

    Ok(spec)
}

fn array_spec(
    array: &ArrayType<'_>,
    parameters: &HashSet<String>,
    path: &TokenStream,
) -> syn::Result<TokenStream> {
    let Some(arguments) = array.type_arguments.as_ref() else {
        return Ok(quote!(#path::Array));
    };
    let [key, value] = arguments.arguments.as_slice() else {
        return Err(syn::Error::new(
            Span::call_site(),
            "an array type needs two arguments",
        ));
    };
    let key = type_spec(key.r#type, parameters)?;
    let value = type_spec(value.r#type, parameters)?;

    Ok(quote!(#path::ArrayOf(&#key, &#value)))
}

fn vector_spec(
    vector: &VecType<'_>,
    parameters: &HashSet<String>,
    path: &TokenStream,
) -> syn::Result<TokenStream> {
    let Some(arguments) = vector.type_arguments.as_ref() else {
        return Ok(quote!(#path::Vec));
    };
    let [element] = arguments.arguments.as_slice() else {
        return Err(syn::Error::new(
            Span::call_site(),
            "a vec type needs one argument",
        ));
    };
    let element = type_spec(element.r#type, parameters)?;

    Ok(quote!(#path::VectorOf(&#element)))
}

fn dictionary_spec(
    dictionary: &DictType<'_>,
    parameters: &HashSet<String>,
    path: &TokenStream,
) -> syn::Result<TokenStream> {
    let Some(arguments) = dictionary.type_arguments.as_ref() else {
        return Ok(quote!(#path::Dict));
    };
    let [key, value] = arguments.arguments.as_slice() else {
        return Err(syn::Error::new(
            Span::call_site(),
            "a dict type needs two arguments",
        ));
    };
    let key = type_spec(key.r#type, parameters)?;
    let value = type_spec(value.r#type, parameters)?;

    Ok(quote!(#path::DictionaryOf(&#key, &#value)))
}

fn tuple_spec(
    tuple: &TupleType<'_>,
    parameters: &HashSet<String>,
    path: &TokenStream,
) -> syn::Result<TokenStream> {
    let elements = tuple
        .elements
        .iter()
        .map(|element| type_spec(element, parameters))
        .collect::<syn::Result<Vec<_>>>()?;
    let Some(rest) = &tuple.trailing_type else {
        return Ok(quote!(#path::TupleOf(&[#(#elements),*])));
    };
    let rest = match rest.r#type {
        Some(r#type) => type_spec(r#type, parameters)?,
        None => quote!(#path::Mixed),
    };

    Ok(quote!(#path::TupleRest(&[#(#elements),*], &#rest)))
}

fn composite_spec(
    subject: &Type<'_>,
    kind: Composite,
    parameters: &HashSet<String>,
    path: &TokenStream,
) -> syn::Result<TokenStream> {
    let specs = flatten(subject, kind)
        .into_iter()
        .map(|member| type_spec(member, parameters))
        .collect::<syn::Result<Vec<_>>>()?;

    Ok(match kind {
        Composite::Union => quote!(#path::Union(&[#(#specs),*])),
        Composite::Intersection => quote!(#path::Intersection(&[#(#specs),*])),
    })
}

fn negated_spec(
    negated: &NegatedType<'_>,
    parameters: &HashSet<String>,
    path: &TokenStream,
) -> syn::Result<TokenStream> {
    if matches!(negated.r#type.unparenthesized(), Type::Void(_)) {
        return Err(unsupported(
            negated.r#type,
            "`void` is return-only and cannot be negated",
        ));
    }
    if matches!(
        negated.r#type.unparenthesized(),
        Type::Named(named) if named.identifier.value() == "_" && named.type_arguments.is_none()
    ) {
        return Err(unsupported(
            negated.r#type,
            "`_` is an existential type pattern and cannot be negated",
        ));
    }
    let inner = type_spec(negated.r#type, parameters)?;

    Ok(quote!(#path::Negated(&#inner)))
}

fn function_spec(
    function: &FunctionType<'_>,
    parameters: &HashSet<String>,
    path: &TokenStream,
) -> syn::Result<TokenStream> {
    let Some(signature) = function.signature.as_ref() else {
        return Ok(quote!(#path::Function));
    };
    let callable_path = quote!(crate::builtin::spec::CallableParameterSpec);
    let parameter_specs = signature
        .parameters
        .iter()
        .map(|parameter| {
            let type_spec = type_spec(parameter.r#type, parameters)?;
            let optional = parameter.equals.is_some();
            Ok(quote!(#callable_path { type_spec: #type_spec, optional: #optional }))
        })
        .collect::<syn::Result<Vec<_>>>()?;
    let return_spec = type_spec(signature.return_type, parameters)?;

    Ok(quote!(#path::CallableOf(&[#(#parameter_specs),*], &#return_spec)))
}

fn named_spec(
    named: &NamedType<'_>,
    parameters: &HashSet<String>,
    path: &TokenStream,
) -> syn::Result<TokenStream> {
    let name = named.identifier.value();
    let Some(arguments) = named.type_arguments.as_ref() else {
        if parameters.contains(name) {
            return Ok(quote!(#path::Parameter(#name)));
        }
        let literal = qualified_name(name);
        return Ok(quote!(#path::Instance(#literal)));
    };
    let argument_specs = arguments
        .arguments
        .iter()
        .map(|argument| type_spec(argument.r#type, parameters))
        .collect::<syn::Result<Vec<_>>>()?;
    let literal = qualified_name(name);

    Ok(quote!(#path::GenericInstance(#literal, &[#(#argument_specs),*])))
}

/// Emits a `BaseSpec` for a parent or implemented interface named by `subject`,
/// which must be a (possibly generic) class-like name.
pub(super) fn base_spec(
    subject: &Type<'_>,
    parameters: &HashSet<String>,
) -> syn::Result<TokenStream> {
    let Type::Named(named) = subject else {
        return Err(unsupported(
            subject,
            "a base must be a class or interface name",
        ));
    };

    let name = qualified_name(named.identifier.value());
    let arguments = if let Some(list) = named.type_arguments.as_ref() {
        let specs = list
            .arguments
            .iter()
            .map(|argument| type_spec(argument.r#type, parameters))
            .collect::<syn::Result<Vec<_>>>()?;
        quote!(::core::option::Option::Some(&[#(#specs),*] as &[crate::builtin::spec::TypeSpec]))
    } else {
        quote!(::core::option::Option::None)
    };

    Ok(quote!(crate::builtin::spec::BaseSpec { name: #name, arguments: #arguments }))
}

pub(super) fn render(subject: &Type<'_>) -> String {
    match subject {
        Type::Int(_) => "int".to_owned(),
        Type::Float(_) => "float".to_owned(),
        Type::Bool(_) => "bool".to_owned(),
        Type::String(_) => "string".to_owned(),
        Type::Mixed(_) | Type::VecShape(_) | Type::DictShape(_) => "mixed".to_owned(),
        Type::Void(_) => "void".to_owned(),
        Type::Never(_) => "never".to_owned(),
        Type::Object(_) => "object".to_owned(),
        Type::Static(_) => "static".to_owned(),
        Type::Self_(self_type) => match &self_type.member {
            Some(member) => format!("self::{}", member.name.value),
            None => "self".to_owned(),
        },
        Type::Parent(_) => "parent".to_owned(),
        Type::Parenthesized(inner) => format!("({})", render(inner.r#type)),
        Type::Array(array) => match array.type_arguments.as_ref() {
            Some(arguments) => format!(
                "array<{}, {}>",
                render(arguments.arguments.as_slice()[0].r#type),
                render(arguments.arguments.as_slice()[1].r#type)
            ),
            None => "array".to_owned(),
        },
        Type::Vec(vector) => match vector.type_arguments.as_ref() {
            Some(arguments) => format!("vec<{}>", render(arguments.arguments.as_slice()[0].r#type)),
            None => "vec".to_owned(),
        },
        Type::Dict(dictionary) => match dictionary.type_arguments.as_ref() {
            Some(arguments) => {
                format!(
                    "dict<{}, {}>",
                    render(arguments.arguments.as_slice()[0].r#type),
                    render(arguments.arguments.as_slice()[1].r#type)
                )
            }
            None => "dict".to_owned(),
        },
        Type::Tuple(tuple) => {
            let elements = tuple.elements.iter().map(render).collect::<Vec<_>>();
            let elements = if let Some(rest) = &tuple.trailing_type {
                let tail = rest.r#type.map_or_else(
                    || "...".to_string(),
                    |r#type| format!("...{}", render(r#type)),
                );
                let mut elements = elements;
                elements.push(tail);
                elements
            } else {
                elements
            };
            format!("({})", elements.join(", "))
        }
        Type::Union(_) => render_all(flatten(subject, Composite::Union).into_iter(), "|"),
        Type::Intersection(_) => {
            render_all(flatten(subject, Composite::Intersection).into_iter(), "&")
        }
        Type::Negated(negated) => format!("!{}", render(negated.r#type)),
        Type::Classname(classname) => format!("classname<{}>", render(classname.inner)),
        Type::Function(function) => match function.signature.as_ref() {
            Some(signature) => {
                let parameters = signature
                    .parameters
                    .iter()
                    .map(|parameter| {
                        let prefix = if parameter.equals.is_some() { "=" } else { "" };
                        format!("{prefix}{}", render(parameter.r#type))
                    })
                    .collect::<Vec<_>>()
                    .join(", ");
                format!("fn({parameters}): {}", render(signature.return_type))
            }
            None => "fn".to_owned(),
        },
        Type::Named(named) => match named.type_arguments.as_ref() {
            Some(arguments) => {
                let rendered = arguments
                    .arguments
                    .iter()
                    .map(|argument| render(argument.r#type))
                    .collect::<Vec<_>>()
                    .join(", ");
                format!("{}<{rendered}>", named.identifier.value())
            }
            None => named.identifier.value().to_owned(),
        },
        Type::Literal(Literal::Null(_)) => "null".to_owned(),
        Type::Literal(literal) => render_literal(literal),
        Type::NegativeLiteral(literal) => match literal {
            NegativeLiteralType::Integer { literal, .. } => format!("-{}", literal.raw),
            NegativeLiteralType::Float { literal, .. } => format!("-{}", literal.raw),
        },
        Type::IntegerRange(range) => render_integer_range(range),
    }
}

fn integer_range_spec(
    range: &IntegerRangeType<'_>,
    path: &TokenStream,
) -> syn::Result<TokenStream> {
    let min = range.lower.as_ref().map(integer_bound).transpose()?;
    let mut max = range.upper.as_ref().map(integer_bound).transpose()?;
    if matches!(range.operator, IntegerRangeOperator::Exclusive(_))
        && let Some(upper) = max
    {
        let Some(inclusive) = upper.checked_sub(1) else {
            return Ok(quote!(#path::Never));
        };
        max = Some(inclusive);
    }
    if min.zip(max).is_some_and(|(min, max)| min > max) {
        return Ok(quote!(#path::Never));
    }
    if min.is_none_or(|min| min == i64::MIN) && max.is_none_or(|max| max == i64::MAX) {
        return Ok(quote!(#path::Int));
    }
    let min = option_integer(min);
    let max = option_integer(max);

    Ok(quote!(#path::IntRange(#min, #max)))
}

fn integer_bound(bound: &IntegerRangeBound<'_>) -> syn::Result<i64> {
    match bound {
        IntegerRangeBound::Positive(literal) => i64::try_from(literal.value)
            .map_err(|_| syn::Error::new(Span::call_site(), "integer range bound is too large")),
        IntegerRangeBound::Negative { literal, .. } => {
            if literal.value == (i64::MAX as u64) + 1 {
                return Ok(i64::MIN);
            }
            i64::try_from(literal.value)
                .map(|value| -value)
                .map_err(|_| syn::Error::new(Span::call_site(), "integer range bound is too small"))
        }
    }
}

fn option_integer(value: Option<i64>) -> TokenStream {
    match value {
        Some(value) => quote!(::core::option::Option::Some(#value)),
        None => quote!(::core::option::Option::None),
    }
}

fn render_integer_range(range: &IntegerRangeType<'_>) -> String {
    let lower = range
        .lower
        .as_ref()
        .map(render_integer_bound)
        .unwrap_or_default();
    let operator = match range.operator {
        IntegerRangeOperator::Exclusive(_) => "..",
        IntegerRangeOperator::Inclusive(_) => "..=",
    };
    let upper = range
        .upper
        .as_ref()
        .map(render_integer_bound)
        .unwrap_or_default();

    format!("{lower}{operator}{upper}")
}

fn render_integer_bound(bound: &IntegerRangeBound<'_>) -> String {
    match bound {
        IntegerRangeBound::Positive(literal) => literal.raw.to_owned(),
        IntegerRangeBound::Negative { literal, .. } => format!("-{}", literal.raw),
    }
}

fn render_all<'a>(types: impl Iterator<Item = &'a Type<'a>>, separator: &str) -> String {
    types.map(render).collect::<Vec<_>>().join(separator)
}

fn render_literal(literal: &Literal<'_>) -> String {
    match literal {
        Literal::True(_) => "true".to_owned(),
        Literal::False(_) => "false".to_owned(),
        Literal::Null(_) => "null".to_owned(),
        Literal::Integer(integer) => integer.value.to_string(),
        Literal::Float(float) => float.value.to_string(),
        Literal::String(string) => string.raw.to_owned(),
    }
}

#[derive(Clone, Copy)]
enum Composite {
    Union,
    Intersection,
}

fn flatten<'a>(subject: &'a Type<'a>, kind: Composite) -> Vec<&'a Type<'a>> {
    let mut members = Vec::new();
    collect(subject, kind, &mut members);
    members
}

fn collect<'a>(subject: &'a Type<'a>, kind: Composite, members: &mut Vec<&'a Type<'a>>) {
    match (subject, kind) {
        (Type::Union(union), Composite::Union) => {
            collect(union.left, kind, members);
            collect(union.right, kind, members);
        }
        (Type::Intersection(intersection), Composite::Intersection) => {
            collect(intersection.left, kind, members);
            collect(intersection.right, kind, members);
        }
        _ => members.push(subject),
    }
}

fn qualified_name(name: &str) -> syn::LitStr {
    syn::LitStr::new(name.trim_start_matches('\\'), Span::call_site())
}

fn unsupported(_subject: &Type<'_>, message: &str) -> syn::Error {
    syn::Error::new(Span::call_site(), message)
}
