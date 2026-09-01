//! Canonical type rendering, composition validation, and type-argument
//! arity checking.

use hashbrown::HashMap;

use whim_span::HasSpan;
use whim_span::Span;
use whim_syn::cst::atom::Identifier;
use whim_syn::cst::atom::Literal;
use whim_syn::cst::r#type::ArrayType;
use whim_syn::cst::r#type::DictShapeType;
use whim_syn::cst::r#type::DictType;
use whim_syn::cst::r#type::FunctionType;
use whim_syn::cst::r#type::IntegerRangeBound;
use whim_syn::cst::r#type::IntegerRangeOperator;
use whim_syn::cst::r#type::IntegerRangeType;
use whim_syn::cst::r#type::IntersectionType;
use whim_syn::cst::r#type::NamedType;
use whim_syn::cst::r#type::NegativeLiteralType;
use whim_syn::cst::r#type::SelfType;
use whim_syn::cst::r#type::TupleType;
use whim_syn::cst::r#type::Type;
use whim_syn::cst::r#type::TypeArgumentList;
use whim_syn::cst::r#type::TypeParameterList;
use whim_syn::cst::r#type::UnionType;
use whim_syn::cst::r#type::VecShapeType;
use whim_syn::cst::r#type::VecType;

use crate::compiler::error::CompileError;
use crate::compiler::error::CompileErrorKind;
use crate::compiler::limits::check_sequence;
use crate::compiler::limits::check_tuple_sequence;
use crate::compiler::types::AliasExpansion;
use crate::compiler::types::GenericDecl;
use crate::compiler::types::GenericTable;
use crate::compiler::types::TypeScope;
use crate::compiler::types::lowering::is_wildcard;
use crate::compiler::types::lowering::lower_type;
use crate::limits::MAX_TYPE_DEPTH;
use crate::unreachable_invariant;
use crate::unwrap_option_invariant;

pub(in crate::compiler) fn render_annotation(
    scope: &TypeScope<'_>,
    source: &Type<'_>,
) -> Result<String, CompileError> {
    lower_type(scope, source)?;
    render_type(scope, source)
}

pub(in crate::compiler::types) fn validate_composition(
    scope: &TypeScope<'_>,
    members: &[&Type<'_>],
    is_union: bool,
) -> Result<(), CompileError> {
    check_sequence(
        CompileErrorKind::TooManyTypeCompositionMembers,
        if is_union {
            "a union may have"
        } else {
            "an intersection may have"
        },
        "members",
        members,
    )?;
    let mut rendered = Vec::with_capacity(members.len());
    for member in members {
        rendered.push(render_type_subst(
            scope,
            member.unparenthesized(),
            &HashMap::new(),
            0,
            &mut Vec::new(),
            AliasRendering::Preserve,
        )?);
    }
    for (index, member) in members.iter().enumerate() {
        if matches!(member.unparenthesized(), Type::Mixed(_)) {
            return Err(CompileError::new(
                CompileErrorKind::RedundantTypeComposition,
                if is_union {
                    "`mixed` already contains every type; a union with `mixed` is `mixed`"
                } else {
                    "`mixed` constrains nothing; an intersection with `mixed` is redundant"
                },
                member.span(),
            ));
        }

        if is_union && matches!(member.unparenthesized(), Type::Void(_)) {
            return Err(CompileError::new(
                CompileErrorKind::VoidInUnion,
                "`void` cannot be a member of a union type",
                member.span(),
            ));
        }

        if is_union && matches!(member.unparenthesized(), Type::Never(_)) {
            return Err(CompileError::new(
                CompileErrorKind::RedundantTypeComposition,
                "`never` contains no values; a union with `never` is redundant",
                member.span(),
            ));
        }

        if rendered[..index].contains(&rendered[index]) {
            return Err(CompileError::new(
                CompileErrorKind::RedundantTypeComposition,
                format!(
                    "`{}` appears twice in {}",
                    rendered[index],
                    if is_union {
                        "a union"
                    } else {
                        "an intersection"
                    }
                ),
                member.span(),
            ));
        }

        if is_union {
            let base = match member.unparenthesized() {
                Type::Literal(Literal::Integer(_))
                | Type::NegativeLiteral(NegativeLiteralType::Integer { .. })
                | Type::IntegerRange(_) => Some("int"),
                Type::Literal(Literal::Float(_))
                | Type::NegativeLiteral(NegativeLiteralType::Float { .. }) => Some("float"),
                Type::Literal(Literal::String(_)) => Some("string"),
                Type::Literal(Literal::True(_) | Literal::False(_)) => Some("bool"),
                _ => None,
            };
            if let Some(base) = base
                && rendered.iter().any(|text| text == base)
            {
                return Err(CompileError::new(
                    CompileErrorKind::RedundantTypeComposition,
                    format!("`{}` is already contained in `{base}`", rendered[index]),
                    member.span(),
                ));
            }
        }
    }

    if is_union {
        validate_integer_union_members(members, &rendered)?;
    }

    Ok(())
}

fn validate_integer_union_members(
    members: &[&Type<'_>],
    rendered: &[String],
) -> Result<(), CompileError> {
    for (index, member) in members.iter().enumerate() {
        let Some(candidate) = integer_member_interval(member.unparenthesized()) else {
            continue;
        };
        if candidate.0 > candidate.1 {
            return Err(CompileError::new(
                CompileErrorKind::RedundantTypeComposition,
                format!(
                    "`{}` is empty and contributes no values to a union",
                    rendered[index]
                ),
                member.span(),
            ));
        }

        for (other_index, other) in members.iter().enumerate() {
            if index == other_index {
                continue;
            }
            let Some(container) = integer_member_interval(other.unparenthesized()) else {
                continue;
            };
            if candidate == container && index < other_index {
                continue;
            }
            if container.0 <= candidate.0 && container.1 >= candidate.1 {
                return Err(CompileError::new(
                    CompileErrorKind::RedundantTypeComposition,
                    format!(
                        "`{}` is already contained in `{}`",
                        rendered[index], rendered[other_index]
                    ),
                    member.span(),
                ));
            }
        }
    }

    Ok(())
}

fn integer_member_interval(member: &Type<'_>) -> Option<(i128, i128)> {
    match member {
        Type::Literal(Literal::Integer(literal)) => {
            let value = i64::try_from(literal.value).ok()?;
            let value = i128::from(value);
            Some((value, value))
        }
        Type::NegativeLiteral(NegativeLiteralType::Integer { literal, .. }) => {
            let value = negative_integer_value(literal.value)?;
            let value = i128::from(value);
            Some((value, value))
        }
        Type::IntegerRange(range) => {
            let lower = match &range.lower {
                Some(bound) => i128::from(integer_range_bound_value(bound)?),
                None => i128::from(i64::MIN),
            };
            let upper = match &range.upper {
                Some(bound) => {
                    let upper = integer_range_bound_value(bound)?;
                    i128::from(upper)
                        - i128::from(matches!(range.operator, IntegerRangeOperator::Exclusive(_)))
                }
                None => i128::from(i64::MAX),
            };
            Some((lower, upper))
        }
        _ => None,
    }
}

fn integer_range_bound_value(bound: &IntegerRangeBound<'_>) -> Option<i64> {
    match bound {
        IntegerRangeBound::Positive(literal) => i64::try_from(literal.value).ok(),
        IntegerRangeBound::Negative { literal, .. } => negative_integer_value(literal.value),
    }
}

fn negative_integer_value(magnitude: u64) -> Option<i64> {
    if magnitude == (i64::MAX as u64) + 1 {
        Some(i64::MIN)
    } else {
        i64::try_from(magnitude).ok().map(|magnitude| -magnitude)
    }
}

pub(in crate::compiler::types) fn check_type_arguments(
    arguments: &TypeArgumentList<'_>,
) -> Result<(), CompileError> {
    check_sequence(
        CompileErrorKind::TooManyTypeArguments,
        "a reference may supply",
        "type arguments",
        arguments.arguments,
    )
}

pub(in crate::compiler::types) fn check_tuple_type(
    tuple: &TupleType<'_>,
) -> Result<(), CompileError> {
    check_tuple_sequence(
        CompileErrorKind::TooManyTupleElements,
        "a tuple type may have",
        "elements",
        &tuple.elements,
    )?;
    // The CST stores the optional rest separately, so it is necessarily last.

    Ok(())
}

pub(in crate::compiler::types) fn flatten_union<'result, 'arena>(
    source: &'result Type<'arena>,
) -> Vec<&'result Type<'arena>> {
    let mut members = Vec::new();
    let mut pending = vec![source];
    while let Some(current) = pending.pop() {
        if let Type::Union(union) = current.unparenthesized() {
            pending.push(union.right);
            pending.push(union.left);
        } else {
            members.push(current);
        }
    }

    members
}

pub(in crate::compiler::types) fn flatten_intersection<'result, 'arena>(
    source: &'result Type<'arena>,
) -> Vec<&'result Type<'arena>> {
    let mut members = Vec::new();
    let mut current = source;
    while let Type::Intersection(intersection) = current {
        members.push(intersection.right);
        current = intersection.left;
    }
    members.push(current);
    members.reverse();

    members
}

pub(in crate::compiler) fn render_type(
    scope: &TypeScope<'_>,
    source: &Type<'_>,
) -> Result<String, CompileError> {
    render_type_subst(
        scope,
        source,
        &HashMap::new(),
        0,
        &mut Vec::new(),
        AliasRendering::Expand,
    )
}

#[derive(Clone, Copy, Eq, PartialEq)]
enum AliasRendering {
    Expand,
    Preserve,
}

#[derive(Clone, Copy)]
struct RenderState<'scope, 'compilation, 'substitution> {
    scope: &'scope TypeScope<'compilation>,
    substitution: &'substitution HashMap<String, String>,
    depth: usize,
    alias_rendering: AliasRendering,
}

fn render_type_subst(
    scope: &TypeScope<'_>,
    source: &Type<'_>,
    substitution: &HashMap<String, String>,
    depth: usize,
    expanding_aliases: &mut Vec<String>,
    alias_rendering: AliasRendering,
) -> Result<String, CompileError> {
    render_type_with_state(
        RenderState {
            scope,
            substitution,
            depth,
            alias_rendering,
        },
        source,
        expanding_aliases,
    )
}

fn render_type_with_state(
    state: RenderState<'_, '_, '_>,
    source: &Type<'_>,
    expanding_aliases: &mut Vec<String>,
) -> Result<String, CompileError> {
    let RenderState {
        scope,
        substitution,
        depth,
        alias_rendering,
    } = state;

    Ok(match source {
        Type::Parenthesized(parenthesized) => {
            format!(
                "({})",
                render_type_subst(
                    scope,
                    parenthesized.r#type,
                    substitution,
                    depth,
                    expanding_aliases,
                    alias_rendering,
                )?
            )
        }
        Type::Mixed(_) => "mixed".to_string(),
        Type::Bool(_) => "bool".to_string(),
        Type::Int(_) => "int".to_string(),
        Type::Float(_) => "float".to_string(),
        Type::String(_) => "string".to_string(),
        Type::Object(_) => "object".to_string(),
        Type::Void(_) => "void".to_string(),
        Type::Never(_) => "never".to_string(),
        Type::Negated(negated) => {
            format!(
                "!{}",
                render_type_subst(
                    scope,
                    negated.r#type,
                    substitution,
                    depth,
                    expanding_aliases,
                    alias_rendering,
                )?
            )
        }
        Type::Self_(self_type) => render_self_type(state, source, self_type, expanding_aliases)?,
        Type::Parent(_) => scope.parent_name(source)?,
        Type::Static(_) => "static".to_string(),
        Type::Literal(literal) => match literal {
            Literal::Null(_) => "null".to_string(),
            Literal::True(_) => "true".to_string(),
            Literal::False(_) => "false".to_string(),
            Literal::Integer(integer) => integer.raw.to_string(),
            Literal::Float(float) => float.raw.to_string(),
            Literal::String(string) => {
                format!("'{}'", String::from_utf8_lossy(string.value))
            }
        },
        Type::NegativeLiteral(literal) => match literal {
            NegativeLiteralType::Integer { literal, .. } => format!("-{}", literal.raw),
            NegativeLiteralType::Float { literal, .. } => format!("-{}", literal.raw),
        },
        Type::IntegerRange(range) => render_integer_range(range),
        Type::Named(named) => render_named_type(state, source, named, expanding_aliases)?,
        Type::Array(array) => render_array_type(state, array, expanding_aliases)?,
        Type::Vec(vector) => render_vec_type(state, vector, expanding_aliases)?,
        Type::Dict(dictionary) => render_dict_type(state, dictionary, expanding_aliases)?,
        Type::VecShape(shape) => render_vec_shape_type(state, shape, expanding_aliases)?,
        Type::DictShape(shape) => render_dict_shape_type(state, shape, expanding_aliases)?,
        Type::Classname(classname) => {
            format!(
                "classname<{}>",
                render_type_subst(
                    scope,
                    classname.inner,
                    substitution,
                    depth,
                    expanding_aliases,
                    alias_rendering,
                )?
            )
        }
        Type::Tuple(tuple) => render_tuple_type(state, tuple, expanding_aliases)?,
        Type::Union(union) => render_union_type(state, source, union, expanding_aliases)?,
        Type::Intersection(intersection) => {
            render_intersection_type(state, source, intersection, expanding_aliases)?
        }
        Type::Function(function) => render_function_type(state, function, expanding_aliases)?,
    })
}

fn render_child(
    state: RenderState<'_, '_, '_>,
    source: &Type<'_>,
    expanding_aliases: &mut Vec<String>,
) -> Result<String, CompileError> {
    render_type_subst(
        state.scope,
        source,
        state.substitution,
        state.depth,
        expanding_aliases,
        state.alias_rendering,
    )
}

fn render_arguments(
    state: RenderState<'_, '_, '_>,
    arguments: &TypeArgumentList<'_>,
    expanding_aliases: &mut Vec<String>,
) -> Result<String, CompileError> {
    arguments
        .arguments
        .iter()
        .map(|argument| render_child(state, argument.r#type, expanding_aliases))
        .collect::<Result<Vec<_>, _>>()
        .map(|parts| parts.join(", "))
}

fn render_self_type(
    state: RenderState<'_, '_, '_>,
    source: &Type<'_>,
    self_type: &SelfType<'_>,
    expanding_aliases: &mut Vec<String>,
) -> Result<String, CompileError> {
    let Some(member) = &self_type.member else {
        return state.scope.class_name(source);
    };

    let mut rendered = format!("{}::{}", state.scope.class_name(source)?, member.name.value);
    if let Some(arguments) = &member.type_arguments {
        rendered = format!(
            "{rendered}<{}>",
            render_arguments(state, arguments, expanding_aliases)?
        );
    }

    Ok(rendered)
}

fn render_integer_range(range: &IntegerRangeType<'_>) -> String {
    let lower = range
        .lower
        .as_ref()
        .map(render_integer_range_bound)
        .unwrap_or_default();
    let operator = match range.operator {
        IntegerRangeOperator::Exclusive(_) => "..",
        IntegerRangeOperator::Inclusive(_) => "..=",
    };
    let upper = range
        .upper
        .as_ref()
        .map(render_integer_range_bound)
        .unwrap_or_default();
    format!("{lower}{operator}{upper}")
}

fn render_binder_type(
    state: RenderState<'_, '_, '_>,
    named: &NamedType<'_>,
) -> Result<Option<String>, CompileError> {
    if !state.scope.is_binder(&named.identifier) {
        return Ok(None);
    }

    let Identifier::Local(local) = &named.identifier else {
        // SAFETY: only local identifiers can name in-scope binders.
        unsafe { unreachable_invariant("a binder identifier is always local") }
    };
    if let Some(arguments) = &named.type_arguments {
        return Err(CompileError::new(
            CompileErrorKind::TypeArgumentArityMismatch,
            format!(
                "the type parameter `{}` is not generic and takes no type arguments",
                local.value
            ),
            arguments.span(),
        ));
    }

    Ok(Some(local.value.to_string()))
}

fn render_expanded_alias(
    state: RenderState<'_, '_, '_>,
    source: &Type<'_>,
    named: &NamedType<'_>,
    resolved: &str,
    expanding_aliases: &mut Vec<String>,
) -> Result<Option<String>, CompileError> {
    let argument_count = named
        .type_arguments
        .as_ref()
        .map_or(0, |arguments| arguments.arguments.len());
    if state.alias_rendering == AliasRendering::Expand
        && let Some(declaration) = state.scope.generics.get(resolved)
        && arity_admits(declaration, argument_count)
        && let Some(alias) = &declaration.alias
        && !expanding_aliases.iter().any(|name| name == resolved)
    {
        if state.depth > MAX_TYPE_DEPTH {
            return Err(CompileError::new(
                CompileErrorKind::RecursiveTypeAlias,
                format!("the type alias `{resolved}` expands into itself"),
                source.span(),
            ));
        }
        let inner = alias_substitution(
            state.scope,
            alias,
            named.type_arguments.as_ref(),
            state.substitution,
            state.depth,
            expanding_aliases,
            state.alias_rendering,
        )?;
        expanding_aliases.push(resolved.to_string());
        let rendered = render_type_subst(
            state.scope,
            alias.aliased,
            &inner,
            state.depth + 1,
            expanding_aliases,
            state.alias_rendering,
        );
        expanding_aliases.pop();
        return rendered.map(Some);
    }

    if state.alias_rendering == AliasRendering::Expand && named.type_arguments.is_none() {
        let interned = state
            .scope
            .resolver
            .resolve(state.scope.heap, &named.identifier);
        if let Some(alias) = state
            .scope
            .aliases
            .iter()
            .find(|alias| alias.name == interned)
        {
            return Ok(Some(alias.rendered.to_string()));
        }
    }

    Ok(None)
}

fn render_named_type(
    state: RenderState<'_, '_, '_>,
    source: &Type<'_>,
    named: &NamedType<'_>,
    expanding_aliases: &mut Vec<String>,
) -> Result<String, CompileError> {
    if is_wildcard(source) {
        return Ok("_".to_string());
    }
    if let Some(arguments) = &named.type_arguments {
        check_type_arguments(arguments)?;
    }
    if let Identifier::Local(local) = &named.identifier
        && named.type_arguments.is_none()
        && let Some(replacement) = state.substitution.get(local.value)
    {
        return Ok(replacement.clone());
    }
    if let Some(rendered) = render_binder_type(state, named)? {
        return Ok(rendered);
    }

    let resolved = state.scope.resolver.resolve_text(&named.identifier);
    if let Some(rendered) =
        render_expanded_alias(state, source, named, &resolved, expanding_aliases)?
    {
        return Ok(rendered);
    }

    let mut rendered = resolved;
    if let Some(arguments) = &named.type_arguments {
        rendered = format!(
            "{rendered}<{}>",
            render_arguments(state, arguments, expanding_aliases)?
        );
    }
    if let Some(member) = &named.member {
        rendered = format!("{rendered}::{}", member.name.value);
        if let Some(arguments) = &member.type_arguments {
            rendered = format!(
                "{rendered}<{}>",
                render_arguments(state, arguments, expanding_aliases)?
            );
        }
    }

    Ok(rendered)
}

fn render_array_type(
    state: RenderState<'_, '_, '_>,
    array: &ArrayType<'_>,
    expanding_aliases: &mut Vec<String>,
) -> Result<String, CompileError> {
    let Some(arguments) = &array.type_arguments else {
        return Ok("array".to_string());
    };

    Ok(format!(
        "array<{}, {}>",
        render_child(
            state,
            arguments.arguments.as_slice()[0].r#type,
            expanding_aliases,
        )?,
        render_child(
            state,
            arguments.arguments.as_slice()[1].r#type,
            expanding_aliases,
        )?
    ))
}

fn render_vec_type(
    state: RenderState<'_, '_, '_>,
    vector: &VecType<'_>,
    expanding_aliases: &mut Vec<String>,
) -> Result<String, CompileError> {
    let Some(arguments) = &vector.type_arguments else {
        return Ok("vec".to_string());
    };

    Ok(format!(
        "vec<{}>",
        render_child(
            state,
            arguments.arguments.as_slice()[0].r#type,
            expanding_aliases,
        )?
    ))
}

fn render_dict_type(
    state: RenderState<'_, '_, '_>,
    dictionary: &DictType<'_>,
    expanding_aliases: &mut Vec<String>,
) -> Result<String, CompileError> {
    let Some(arguments) = &dictionary.type_arguments else {
        return Ok("dict".to_string());
    };

    Ok(format!(
        "dict<{}, {}>",
        render_child(
            state,
            arguments.arguments.as_slice()[0].r#type,
            expanding_aliases,
        )?,
        render_child(
            state,
            arguments.arguments.as_slice()[1].r#type,
            expanding_aliases,
        )?
    ))
}

fn render_vec_shape_type(
    state: RenderState<'_, '_, '_>,
    shape: &VecShapeType<'_>,
    expanding_aliases: &mut Vec<String>,
) -> Result<String, CompileError> {
    let mut parts = shape
        .elements
        .iter()
        .map(|r#type| render_child(state, r#type, expanding_aliases))
        .collect::<Result<Vec<_>, _>>()?;
    if let Some(trailing) = &shape.trailing_type {
        parts.push(trailing.r#type.map_or_else(
            || Ok("...".to_string()),
            |r#type| {
                render_child(state, r#type, expanding_aliases)
                    .map(|rendered| format!("...{rendered}"))
            },
        )?);
    }

    Ok(format!("vec[{}]", parts.join(", ")))
}

fn render_dict_shape_type(
    state: RenderState<'_, '_, '_>,
    shape: &DictShapeType<'_>,
    expanding_aliases: &mut Vec<String>,
) -> Result<String, CompileError> {
    let mut entries = shape
        .entries
        .iter()
        .map(|entry| {
            Ok(format!(
                "{} => {}",
                render_literal(&entry.key),
                render_child(state, entry.value, expanding_aliases)?
            ))
        })
        .collect::<Result<Vec<_>, CompileError>>()?;
    if let Some(rest) = &shape.rest {
        entries.push(format!(
            "...<{}, {}>",
            render_child(state, rest.type_arguments.key, expanding_aliases)?,
            render_child(state, rest.type_arguments.value, expanding_aliases)?,
        ));
    }

    Ok(format!("dict[{}]", entries.join(", ")))
}

fn render_tuple_type(
    state: RenderState<'_, '_, '_>,
    tuple: &TupleType<'_>,
    expanding_aliases: &mut Vec<String>,
) -> Result<String, CompileError> {
    check_tuple_type(tuple)?;
    let mut parts = tuple
        .elements
        .iter()
        .map(|element| render_child(state, element, expanding_aliases))
        .collect::<Result<Vec<_>, _>>()?;
    if let Some(rest) = &tuple.trailing_type {
        parts.push(rest.r#type.map_or_else(
            || Ok("...".to_string()),
            |r#type| {
                render_child(state, r#type, expanding_aliases)
                    .map(|rendered| format!("...{rendered}"))
            },
        )?);
    }

    Ok(if parts.len() == 1 {
        format!("({},)", parts[0])
    } else {
        format!("({})", parts.join(", "))
    })
}

fn render_union_type(
    state: RenderState<'_, '_, '_>,
    source: &Type<'_>,
    union: &UnionType<'_>,
    expanding_aliases: &mut Vec<String>,
) -> Result<String, CompileError> {
    validate_composition(state.scope, &flatten_union(source), true)?;
    Ok(format!(
        "{}|{}",
        render_child(state, union.left, expanding_aliases)?,
        render_child(state, union.right, expanding_aliases)?,
    ))
}

fn render_intersection_type(
    state: RenderState<'_, '_, '_>,
    source: &Type<'_>,
    intersection: &IntersectionType<'_>,
    expanding_aliases: &mut Vec<String>,
) -> Result<String, CompileError> {
    validate_composition(state.scope, &flatten_intersection(source), false)?;
    Ok(format!(
        "{}&{}",
        render_child(state, intersection.left, expanding_aliases)?,
        render_child(state, intersection.right, expanding_aliases)?,
    ))
}

fn render_function_type(
    state: RenderState<'_, '_, '_>,
    function: &FunctionType<'_>,
    expanding_aliases: &mut Vec<String>,
) -> Result<String, CompileError> {
    let Some(signature) = &function.signature else {
        return Ok("fn".to_string());
    };
    check_sequence(
        CompileErrorKind::TooManyParameters,
        "a function type may declare",
        "parameters",
        signature.parameters,
    )?;
    let parts = signature
        .parameters
        .iter()
        .map(|parameter| {
            render_child(state, parameter.r#type, expanding_aliases).map(|rendered| {
                if parameter.equals.is_some() {
                    format!("={rendered}")
                } else {
                    rendered
                }
            })
        })
        .collect::<Result<Vec<_>, _>>()?;

    Ok(format!(
        "fn({}): {}",
        parts.join(", "),
        render_child(state, signature.return_type, expanding_aliases)?,
    ))
}

fn render_integer_range_bound(bound: &IntegerRangeBound<'_>) -> String {
    match bound {
        IntegerRangeBound::Positive(literal) => literal.raw.to_string(),
        IntegerRangeBound::Negative { literal, .. } => format!("-{}", literal.raw),
    }
}

fn render_literal(literal: &Literal<'_>) -> String {
    match literal {
        Literal::String(string) => string.raw.to_owned(),
        Literal::Integer(integer) => integer.raw.to_owned(),
        Literal::Float(float) => float.raw.to_owned(),
        Literal::True(keyword) | Literal::False(keyword) | Literal::Null(keyword) => {
            keyword.value.to_owned()
        }
    }
}

pub(in crate::compiler) fn binder_arity(list: &TypeParameterList<'_>) -> (usize, usize) {
    let total = list.parameters.len();
    let required = list
        .parameters
        .iter()
        .position(|parameter| parameter.default.is_some())
        .unwrap_or(total);
    (required, total)
}

pub(in crate::compiler::types) fn check_arity(
    declaration: &GenericDecl<'_>,
    count: usize,
    name: &str,
    span: Span,
) -> Result<(), CompileError> {
    if count >= declaration.required && count <= declaration.total {
        return Ok(());
    }
    if declaration.total == 0 {
        return Err(CompileError::new(
            CompileErrorKind::TypeArgumentArityMismatch,
            format!("`{name}` is not generic and takes no type arguments"),
            span,
        ));
    }
    let expected = if declaration.required == declaration.total {
        format!("exactly {}", declaration.total)
    } else {
        format!("{} to {}", declaration.required, declaration.total)
    };
    Err(CompileError::new(
        CompileErrorKind::TypeArgumentArityMismatch,
        format!("`{name}` expects {expected} type argument(s), but {count} were supplied"),
        span,
    ))
}

const fn arity_admits(declaration: &GenericDecl<'_>, count: usize) -> bool {
    count >= declaration.required && count <= declaration.total
}

pub(in crate::compiler) fn check_type_argument_arity(
    generics: &GenericTable<'_>,
    resolved: &str,
    arguments: Option<&TypeArgumentList<'_>>,
) -> Result<(), CompileError> {
    let Some(arguments) = arguments else {
        return Ok(());
    };
    check_type_arguments(arguments)?;
    let Some(declaration) = generics.get(resolved) else {
        return Ok(());
    };
    check_arity(
        declaration,
        arguments.arguments.len(),
        resolved,
        arguments.span(),
    )
}

pub(in crate::compiler) fn check_call_type_argument_arity(
    generics: &GenericTable<'_>,
    resolved: &str,
    arguments: Option<&TypeArgumentList<'_>>,
    span: Span,
) -> Result<(), CompileError> {
    if let Some(arguments) = arguments {
        check_type_arguments(arguments)?;
    }
    let Some(declaration) = generics.get(resolved) else {
        return Ok(());
    };
    check_arity(
        declaration,
        arguments.map_or(0, |arguments| arguments.arguments.len()),
        resolved,
        span,
    )
}

fn alias_substitution(
    scope: &TypeScope<'_>,
    alias: &AliasExpansion<'_>,
    arguments: Option<&TypeArgumentList<'_>>,
    outer: &HashMap<String, String>,
    depth: usize,
    expanding_aliases: &mut Vec<String>,
    alias_rendering: AliasRendering,
) -> Result<HashMap<String, String>, CompileError> {
    let provided: Vec<&Type<'_>> = arguments
        .map(|arguments| {
            arguments
                .arguments
                .iter()
                .map(|argument| argument.r#type)
                .collect()
        })
        .unwrap_or_default();
    let mut substitution = HashMap::new();
    if let Some(parameters) = alias.type_parameters {
        for (index, parameter) in parameters.parameters.iter().enumerate() {
            let rendered = if index < provided.len() {
                render_type_subst(
                    scope,
                    provided[index],
                    outer,
                    depth,
                    expanding_aliases,
                    alias_rendering,
                )?
            } else {
                // SAFETY: a type parameter past the required count always declares a default.
                let default = unsafe {
                    unwrap_option_invariant(
                        parameter.default.as_ref(),
                        "a parameter past the required count declares a default",
                    )
                };
                render_type_subst(
                    scope,
                    default.r#type,
                    &substitution,
                    depth,
                    expanding_aliases,
                    alias_rendering,
                )?
            };
            substitution.insert(parameter.name.value.to_string(), rendered);
        }
    }
    Ok(substitution)
}
