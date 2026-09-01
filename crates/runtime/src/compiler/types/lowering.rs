//! Type lowering: source types to complete runtime descriptors, with the
//! formation gates applied on the way.

use whim_span::HasSpan;
use whim_span::Span;
use whim_syn::cst::atom::Identifier;
use whim_syn::cst::atom::Literal;
use whim_syn::cst::node::Node;
use whim_syn::cst::r#type::ArrayType;
use whim_syn::cst::r#type::ClassnameType;
use whim_syn::cst::r#type::DictShapeType;
use whim_syn::cst::r#type::DictType;
use whim_syn::cst::r#type::FunctionType;
use whim_syn::cst::r#type::IntegerRangeBound;
use whim_syn::cst::r#type::IntegerRangeOperator;
use whim_syn::cst::r#type::IntegerRangeType;
use whim_syn::cst::r#type::MemberType;
use whim_syn::cst::r#type::NamedType;
use whim_syn::cst::r#type::NegatedType;
use whim_syn::cst::r#type::NegativeLiteralType;
use whim_syn::cst::r#type::SelfType;
use whim_syn::cst::r#type::TupleType;
use whim_syn::cst::r#type::Type;
use whim_syn::cst::r#type::TypeArgumentList;
use whim_syn::cst::r#type::VecShapeType;
use whim_syn::cst::r#type::VecType;
use whim_syn::cst::walker::Flow;
use whim_syn::cst::walker::Visitor;
use whim_syn::cst::walker::walk;

use crate::bytecode::aliases::expand_aliases;
use crate::bytecode::chunk::descriptors::FunctionTypeDescriptor;
use crate::bytecode::chunk::descriptors::FunctionTypeParameterDescriptor;
use crate::bytecode::chunk::descriptors::ShapeKey;
use crate::bytecode::chunk::descriptors::TypeDescriptor;
use crate::compiler::error::CompileError;
use crate::compiler::error::CompileErrorKind;
use crate::compiler::limits::check_sequence;
use crate::compiler::types::DeclaredTypeKind;
use crate::compiler::types::TypeScope;
use crate::compiler::types::rendering::check_arity;
use crate::compiler::types::rendering::check_tuple_type;
use crate::compiler::types::rendering::check_type_arguments;
use crate::compiler::types::rendering::flatten_intersection;
use crate::compiler::types::rendering::flatten_union;
use crate::compiler::types::rendering::validate_composition;
use crate::value::heap::Heap;

pub(in crate::compiler) fn lower_checked_type(
    scope: &TypeScope<'_>,
    source: &Type<'_>,
) -> Result<TypeDescriptor, CompileError> {
    reject_bare_uninhabited_checked_type(source)?;
    validate_return_annotation(source)?;
    reject_standalone_wildcard(source)?;
    validate_checked_builtin_arity(source)?;
    let descriptor = lower_type_inner(scope, source, true)?;
    validate_expanded_checked_builtin_arity(
        &expand_aliases(&descriptor, scope.aliases),
        source.span(),
    )?;

    Ok(descriptor)
}

pub(in crate::compiler) fn lower_pattern_type(
    scope: &TypeScope<'_>,
    source: &Type<'_>,
) -> Result<TypeDescriptor, CompileError> {
    reject_bare_uninhabited_checked_type(source)?;
    validate_return_annotation(source)?;
    reject_standalone_wildcard(source)?;
    validate_checked_builtin_arity(source)?;
    lower_type_inner(scope, source, true)
}

/// A checked type must describe at least one possible runtime value.
fn reject_bare_uninhabited_checked_type(source: &Type<'_>) -> Result<(), CompileError> {
    match source.unparenthesized() {
        Type::Void(_) => Err(CompileError::new(
            CompileErrorKind::ReturnOnlyType,
            "`void` is return-only and cannot be used as a checked type",
            source.span(),
        )),
        Type::Never(_) => Err(CompileError::new(
            CompileErrorKind::TypeNotRuntimeCheckable,
            "`never` contains no values and cannot be used as a standalone checked type",
            source.span(),
        )),
        _ => Ok(()),
    }
}

pub(in crate::compiler) fn lower_type(
    scope: &TypeScope<'_>,
    source: &Type<'_>,
) -> Result<TypeDescriptor, CompileError> {
    reject_standalone_wildcard(source)?;
    lower_type_inner(scope, source, false)
}

/// Lowers a concrete generic binding. Unlike an existential type pattern,
/// a turbofish, base specialization, bound, or default must bind every slot
/// to an actual type, so `_` is not admitted anywhere inside it.
pub(in crate::compiler) fn lower_type_argument(
    scope: &TypeScope<'_>,
    source: &Type<'_>,
) -> Result<TypeDescriptor, CompileError> {
    if let Some(span) = wildcard_span(source) {
        return Err(CompileError::new(
            CompileErrorKind::WildcardTypeArgument,
            "`_` is an existential type pattern and cannot bind a generic type parameter",
            span,
        ));
    }

    lower_type(scope, source)
}

/// `_` is meaningful only as one position inside a larger type pattern.
fn reject_standalone_wildcard(source: &Type<'_>) -> Result<(), CompileError> {
    if is_wildcard(source.unparenthesized()) {
        return Err(CompileError::new(
            CompileErrorKind::StandaloneWildcardType,
            "`_` cannot be used as a standalone type; place it inside a composite type such as `vec<_>`",
            source.span(),
        ));
    }

    Ok(())
}

fn builtin_arity_error(name: &str, expected: usize, span: Span) -> CompileError {
    CompileError::new(
        CompileErrorKind::TypeArgumentArityMismatch,
        format!("the built-in type `{name}` expects exactly {expected} type arguments"),
        span,
    )
}

/// Checked built-in collection types always spell their complete arity. A
/// bare named class remains a nominal check, but `vec` and `dict` are closed
/// built-ins and must use wildcards for positions the check ignores.
#[expect(
    clippy::too_many_lines,
    reason = "one exhaustive walk keeps nested arity validation auditable"
)]
fn validate_checked_builtin_arity(source: &Type<'_>) -> Result<(), CompileError> {
    let mut pending = vec![source];
    while let Some(current) = pending.pop() {
        match current {
            Type::Parenthesized(parenthesized) => pending.push(parenthesized.r#type),
            Type::Negated(negated) => pending.push(negated.r#type),
            Type::Array(array) => {
                if let Some(arguments) = &array.type_arguments {
                    if arguments.arguments.len() != 2 {
                        return Err(builtin_arity_error("array", 2, array.span()));
                    }
                    pending.push(arguments.arguments.as_slice()[0].r#type);
                    pending.push(arguments.arguments.as_slice()[1].r#type);
                } else {
                    return Err(CompileError::new(
                        CompileErrorKind::TypeArgumentArityMismatch,
                        "the built-in type `array` expects exactly 2 type arguments in a runtime check; use `array<_, _>` to ignore them",
                        array.span(),
                    ));
                }
            }
            Type::Vec(vector) => {
                if let Some(arguments) = &vector.type_arguments {
                    if arguments.arguments.len() != 1 {
                        return Err(builtin_arity_error("vec", 1, vector.span()));
                    }
                    pending.push(arguments.arguments.as_slice()[0].r#type);
                } else {
                    return Err(CompileError::new(
                        CompileErrorKind::TypeArgumentArityMismatch,
                        "the built-in type `vec` expects exactly 1 type argument in a runtime check; use `vec<_>` to ignore it",
                        vector.span(),
                    ));
                }
            }
            Type::VecShape(shape) => {
                for element in &shape.elements {
                    pending.push(element);
                }
                if let Some(trailing) = &shape.trailing_type
                    && let Some(r#type) = trailing.r#type
                {
                    pending.push(r#type);
                }
            }
            Type::Dict(dictionary) => {
                if let Some(arguments) = &dictionary.type_arguments {
                    if arguments.arguments.len() != 2 {
                        return Err(builtin_arity_error("dict", 2, dictionary.span()));
                    }
                    pending.push(arguments.arguments.as_slice()[0].r#type);
                    pending.push(arguments.arguments.as_slice()[1].r#type);
                } else {
                    return Err(CompileError::new(
                        CompileErrorKind::TypeArgumentArityMismatch,
                        "the built-in type `dict` expects exactly 2 type arguments in a runtime check; use `dict<_, _>` to ignore them",
                        dictionary.span(),
                    ));
                }
            }
            Type::Named(named) => {
                if let Some(arguments) = &named.type_arguments {
                    pending.extend(arguments.arguments.iter().map(|argument| argument.r#type));
                }
            }
            Type::Function(function) => {
                if let Some(signature) = &function.signature {
                    pending.push(signature.return_type);
                    pending.extend(
                        signature
                            .parameters
                            .iter()
                            .map(|parameter| parameter.r#type),
                    );
                }
            }
            Type::Classname(classname) => pending.push(classname.inner),
            Type::DictShape(shape) => {
                for entry in shape.entries {
                    pending.push(entry.value);
                }
                if let Some(rest) = &shape.rest {
                    pending.push(rest.type_arguments.key);
                    pending.push(rest.type_arguments.value);
                }
            }
            Type::Tuple(tuple) => {
                for element in &tuple.elements {
                    pending.push(element);
                }
                if let Some(trailing) = &tuple.trailing_type
                    && let Some(r#type) = trailing.r#type
                {
                    pending.push(r#type);
                }
            }
            Type::Union(union) => {
                pending.push(union.left);
                pending.push(union.right);
            }
            Type::Intersection(intersection) => {
                pending.push(intersection.left);
                pending.push(intersection.right);
            }
            _ => {}
        }
    }

    Ok(())
}

/// Applies the checked built-in arity rule after structural alias expansion.
fn validate_expanded_checked_builtin_arity(
    descriptor: &TypeDescriptor,
    span: Span,
) -> Result<(), CompileError> {
    let mut pending = vec![descriptor];
    while let Some(current) = pending.pop() {
        match current {
            TypeDescriptor::Array(Some((key, value)))
            | TypeDescriptor::Dictionary(Some((key, value))) => {
                pending.push(key);
                pending.push(value);
            }
            TypeDescriptor::Array(None) => {
                return Err(CompileError::new(
                    CompileErrorKind::TypeArgumentArityMismatch,
                    "the built-in type `array` expects exactly 2 type arguments in a runtime check; use `array<_, _>` to ignore them",
                    span,
                ));
            }
            TypeDescriptor::Vector(Some(element)) => pending.push(element),
            TypeDescriptor::Vector(None) => {
                return Err(CompileError::new(
                    CompileErrorKind::TypeArgumentArityMismatch,
                    "the built-in type `vec` expects exactly 1 type argument in a runtime check; use `vec<_>` to ignore it",
                    span,
                ));
            }
            TypeDescriptor::Dictionary(None) => {
                return Err(CompileError::new(
                    CompileErrorKind::TypeArgumentArityMismatch,
                    "the built-in type `dict` expects exactly 2 type arguments in a runtime check; use `dict<_, _>` to ignore them",
                    span,
                ));
            }
            TypeDescriptor::Named {
                arguments: Some(arguments),
                ..
            } => pending.extend(arguments),
            TypeDescriptor::Callable(Some(signature)) => {
                pending.push(&signature.return_type);
                pending.extend(
                    signature
                        .parameters
                        .iter()
                        .map(|parameter| &parameter.r#type),
                );
            }
            TypeDescriptor::Classname(inner) | TypeDescriptor::Negated(inner) => {
                pending.push(inner);
            }
            TypeDescriptor::Tuple(members)
            | TypeDescriptor::Union(members)
            | TypeDescriptor::Intersection(members) => pending.extend(members),
            TypeDescriptor::TupleRest { elements, rest } => {
                pending.extend(elements);
                pending.push(rest);
            }
            _ => {}
        }
    }

    Ok(())
}

pub(in crate::compiler::types) fn is_wildcard(source: &Type<'_>) -> bool {
    matches!(
        source,
        Type::Named(named)
            if matches!(
                &named.identifier,
                Identifier::Local(local) if local.value == "_"
            ) && named.type_arguments.is_none()
    )
}

fn wildcard_span(source: &Type<'_>) -> Option<Span> {
    let mut found = None;
    walk(
        Node::Type(source),
        &mut WildcardFinder { found: &mut found },
    );

    found
}

struct WildcardFinder<'found> {
    found: &'found mut Option<Span>,
}

impl<'ast, 'arena> Visitor<'ast, 'arena> for WildcardFinder<'_> {
    fn enter(&mut self, node: Node<'ast, 'arena>) -> Flow {
        if self.found.is_none()
            && let Node::Type(source) = node
            && is_wildcard(source)
        {
            *self.found = Some(source.span());
            return Flow::Skip;
        }

        Flow::Descend
    }
}

pub(in crate::compiler) fn reject_return_only_annotation(
    source: &Type<'_>,
    position: &str,
) -> Result<(), CompileError> {
    validate_return_only_positions(source, false, position)
}

/// Validates every nested `void`. It is legal only as the complete return of a
/// function or function type. `never` needs no positional validation.
pub(in crate::compiler) fn validate_return_annotation(
    source: &Type<'_>,
) -> Result<(), CompileError> {
    validate_return_only_positions(source, true, "return")
}

#[expect(
    clippy::too_many_lines,
    reason = "one exhaustive walk keeps nested return-type rules auditable"
)]
fn validate_return_only_positions(
    source: &Type<'_>,
    allow_return_only: bool,
    position: &str,
) -> Result<(), CompileError> {
    let mut pending = vec![(source, allow_return_only, position)];
    while let Some((current, allowed, position)) = pending.pop() {
        match current {
            Type::Parenthesized(parenthesized) => {
                pending.push((parenthesized.r#type, allowed, position));
            }
            Type::Negated(negated) => {
                pending.push((negated.r#type, false, "negated type"));
            }
            Type::Void(_) if !allowed => {
                return Err(CompileError::new(
                    CompileErrorKind::ReturnOnlyType,
                    format!("`void` is return-only and cannot be used in the {position} position"),
                    current.span(),
                ));
            }
            Type::Function(function) => {
                if let Some(signature) = &function.signature {
                    pending.push((signature.return_type, true, "return"));
                    for parameter in signature.parameters {
                        pending.push((parameter.r#type, false, "function parameter"));
                    }
                }
            }
            Type::Named(named) => {
                if let Some(arguments) = &named.type_arguments {
                    for argument in arguments.arguments {
                        pending.push((argument.r#type, false, "type argument"));
                    }
                }
            }
            Type::Union(union) => {
                for member in [union.left, union.right] {
                    if !matches!(member.unparenthesized(), Type::Void(_)) {
                        pending.push((member, false, "union member"));
                    }
                }
            }
            Type::Intersection(intersection) => {
                for member in [intersection.left, intersection.right] {
                    if !matches!(member.unparenthesized(), Type::Void(_)) {
                        pending.push((member, false, "intersection member"));
                    }
                }
            }
            Type::Array(array) => {
                if let Some(arguments) = &array.type_arguments {
                    if arguments.arguments.len() != 2 {
                        return Err(builtin_arity_error("array", 2, array.span()));
                    }
                    pending.push((arguments.arguments.as_slice()[0].r#type, false, "array key"));
                    pending.push((
                        arguments.arguments.as_slice()[1].r#type,
                        false,
                        "array value",
                    ));
                }
            }
            Type::Vec(vector) => {
                if let Some(arguments) = &vector.type_arguments {
                    if arguments.arguments.len() != 1 {
                        return Err(builtin_arity_error("vec", 1, vector.span()));
                    }
                    pending.push((
                        arguments.arguments.as_slice()[0].r#type,
                        false,
                        "vec element",
                    ));
                }
            }
            Type::VecShape(shape) => {
                for element in &shape.elements {
                    pending.push((element, false, "vec shape element"));
                }
                if let Some(trailing) = &shape.trailing_type
                    && let Some(r#type) = trailing.r#type
                {
                    pending.push((r#type, false, "vec shape rest"));
                }
            }
            Type::Dict(dictionary) => {
                if let Some(arguments) = &dictionary.type_arguments {
                    if arguments.arguments.len() != 2 {
                        return Err(builtin_arity_error("dict", 2, dictionary.span()));
                    }
                    pending.push((arguments.arguments.as_slice()[0].r#type, false, "dict key"));
                    pending.push((
                        arguments.arguments.as_slice()[1].r#type,
                        false,
                        "dict value",
                    ));
                }
            }
            Type::DictShape(shape) => {
                for entry in shape.entries {
                    pending.push((entry.value, false, "dict shape value"));
                }
                if let Some(rest) = &shape.rest {
                    pending.push((rest.type_arguments.key, false, "dict shape rest key"));
                    pending.push((rest.type_arguments.value, false, "dict shape rest value"));
                }
            }
            Type::Classname(classname) => {
                pending.push((classname.inner, false, "type argument"));
            }
            Type::Tuple(tuple) => {
                for element in &tuple.elements {
                    pending.push((element, false, "tuple element"));
                }
                if let Some(trailing) = &tuple.trailing_type
                    && let Some(r#type) = trailing.r#type
                {
                    pending.push((r#type, false, "tuple rest element"));
                }
            }
            _ => {}
        }
    }

    Ok(())
}

fn lower_negated_type(
    scope: &TypeScope<'_>,
    negated: &NegatedType<'_>,
    defer_named_arity: bool,
) -> Result<TypeDescriptor, CompileError> {
    let inner = negated.r#type.unparenthesized();
    if matches!(inner, Type::Void(_)) {
        return Err(CompileError::new(
            CompileErrorKind::ReturnOnlyType,
            "`void` is return-only and cannot be negated",
            negated.r#type.span(),
        ));
    }

    if is_wildcard(inner) {
        return Err(CompileError::new(
            CompileErrorKind::TypeNotRuntimeCheckable,
            "`_` is an existential type pattern and cannot be negated",
            negated.r#type.span(),
        ));
    }

    Ok(TypeDescriptor::Negated(Box::new(lower_type_inner(
        scope,
        negated.r#type,
        defer_named_arity,
    )?)))
}

fn lower_type_arguments(
    scope: &TypeScope<'_>,
    arguments: &TypeArgumentList<'_>,
    defer_named_arity: bool,
) -> Result<Vec<TypeDescriptor>, CompileError> {
    arguments
        .arguments
        .iter()
        .map(|argument| lower_type_inner(scope, argument.r#type, defer_named_arity))
        .collect()
}

fn class_type_arguments(scope: &TypeScope<'_>) -> Option<Vec<TypeDescriptor>> {
    scope.class.and_then(|class| {
        (!class.type_parameters.is_empty()).then(|| {
            class
                .type_parameters
                .iter()
                .map(|parameter| TypeDescriptor::Parameter(scope.heap.intern(parameter.as_bytes())))
                .collect()
        })
    })
}

fn lower_self_type(
    scope: &TypeScope<'_>,
    self_type: &SelfType<'_>,
    span: Span,
    defer_named_arity: bool,
) -> Result<TypeDescriptor, CompileError> {
    let class = scope.class.ok_or_else(|| {
        CompileError::new(
            CompileErrorKind::ClassContextRequired,
            "`self` refers to the enclosing class, and there is none here",
            span,
        )
    })?;

    if !class.type_parameters.is_empty() && !scope.forbidden_binders.is_empty() {
        return Err(CompileError::new(
            CompileErrorKind::ClassTypeParameterInStaticMember,
            "`self` carries the generic class's type parameters, which are unavailable in a static member",
            span,
        ));
    }

    let Some(member) = &self_type.member else {
        return Ok(TypeDescriptor::Named {
            name: scope.heap.intern(class.name.as_bytes()),
            arguments: class_type_arguments(scope),
            recursive: false,
        });
    };

    if let Some(arguments) = &member.type_arguments {
        check_type_arguments(arguments)?;
    }
    if let Some(declaration) = scope
        .generics
        .get(&format!("{}::{}", class.name, member.name.value))
        && !(declaration.is_callable && member.type_arguments.is_none())
    {
        let count = member
            .type_arguments
            .as_ref()
            .map_or(0, |arguments| arguments.arguments.len());
        check_arity(declaration, count, member.name.value, member.span())?;
    }
    let member_arguments = member
        .type_arguments
        .as_ref()
        .map(|arguments| lower_type_arguments(scope, arguments, defer_named_arity))
        .transpose()?;

    Ok(TypeDescriptor::Member {
        class: scope.heap.intern(class.name.as_bytes()),
        class_arguments: class_type_arguments(scope),
        member: scope.heap.intern(member.name.value.as_bytes()),
        member_arguments,
    })
}

fn lower_parent_type(
    scope: &TypeScope<'_>,
    source: &Type<'_>,
) -> Result<TypeDescriptor, CompileError> {
    let class = scope.class.ok_or_else(|| {
        CompileError::new(
            CompileErrorKind::ClassContextRequired,
            "`parent` refers to the enclosing class's parent, and there is no class here",
            source.span(),
        )
    })?;

    if !scope.forbidden_binders.is_empty()
        && class
            .parent_arguments
            .as_ref()
            .is_some_and(|arguments| arguments.iter().any(descriptor_has_parameter))
    {
        return Err(CompileError::new(
            CompileErrorKind::ClassTypeParameterInStaticMember,
            "`parent` carries the generic class's type parameters, which are unavailable in a static member",
            source.span(),
        ));
    }

    Ok(TypeDescriptor::Named {
        name: scope.heap.intern(scope.parent_name(source)?.as_bytes()),
        arguments: class.parent_arguments.clone(),
        recursive: false,
    })
}

fn lower_static_type(scope: &TypeScope<'_>, span: Span) -> Result<TypeDescriptor, CompileError> {
    let Some(class) = scope.class else {
        return Err(CompileError::new(
            CompileErrorKind::ClassContextRequired,
            "`static` refers to the late-bound class, and there is no class here",
            span,
        ));
    };

    if !class.type_parameters.is_empty() && !scope.forbidden_binders.is_empty() {
        return Err(CompileError::new(
            CompileErrorKind::ClassTypeParameterInStaticMember,
            "`static` carries the generic class's type parameters, which are unavailable in a static member",
            span,
        ));
    }

    Ok(TypeDescriptor::StaticClass)
}

fn lower_special_named_type(
    scope: &TypeScope<'_>,
    named: &NamedType<'_>,
) -> Result<Option<TypeDescriptor>, CompileError> {
    if matches!(&named.identifier, Identifier::Local(local) if local.value == "_") {
        if let Some(member) = &named.member {
            return Err(CompileError::new(
                CompileErrorKind::InvalidMemberType,
                "the wildcard type `_` has no members",
                member.span(),
            ));
        }
        if let Some(arguments) = &named.type_arguments {
            return Err(CompileError::new(
                CompileErrorKind::TypeArgumentArityMismatch,
                "the wildcard type `_` takes no type arguments",
                arguments.span(),
            ));
        }

        return Ok(Some(TypeDescriptor::Wildcard));
    }

    if scope.is_binder(&named.identifier) {
        if let Some(member) = &named.member {
            return Err(CompileError::new(
                CompileErrorKind::InvalidMemberType,
                "a type parameter has no statically known members",
                member.span(),
            ));
        }
        if let Some(arguments) = &named.type_arguments {
            return Err(CompileError::new(
                CompileErrorKind::TypeArgumentArityMismatch,
                format!(
                    "the type parameter `{}` is not generic and takes no type arguments",
                    named.identifier.value()
                ),
                arguments.span(),
            ));
        }

        return Ok(Some(TypeDescriptor::Parameter(
            scope.heap.intern(named.identifier.value().as_bytes()),
        )));
    }

    if scope.is_forbidden_binder(&named.identifier) {
        return Err(CompileError::new(
            CompileErrorKind::ClassTypeParameterInStaticMember,
            format!(
                "the class type parameter `{}` is unavailable in a static member",
                named.identifier.value()
            ),
            named.span(),
        ));
    }

    Ok(None)
}

fn lower_named_member(
    scope: &TypeScope<'_>,
    named: &NamedType<'_>,
    member: &MemberType<'_>,
    resolved: &str,
    class_arguments: Option<Vec<TypeDescriptor>>,
    defer_named_arity: bool,
) -> Result<TypeDescriptor, CompileError> {
    if let Some(arguments) = &member.type_arguments {
        check_type_arguments(arguments)?;
    }
    if let Some(declaration) = scope
        .generics
        .get(&format!("{resolved}::{}", member.name.value))
        && !(declaration.is_callable && member.type_arguments.is_none())
    {
        let count = member
            .type_arguments
            .as_ref()
            .map_or(0, |arguments| arguments.arguments.len());
        check_arity(declaration, count, member.name.value, member.span())?;
    }
    let member_arguments = member
        .type_arguments
        .as_ref()
        .map(|arguments| lower_type_arguments(scope, arguments, defer_named_arity))
        .transpose()?;

    Ok(TypeDescriptor::Member {
        class: scope.resolver.resolve(scope.heap, &named.identifier),
        class_arguments,
        member: scope.heap.intern(member.name.value.as_bytes()),
        member_arguments,
    })
}

fn lower_named_type(
    scope: &TypeScope<'_>,
    named: &NamedType<'_>,
    defer_named_arity: bool,
) -> Result<TypeDescriptor, CompileError> {
    if let Some(descriptor) = lower_special_named_type(scope, named)? {
        return Ok(descriptor);
    }

    let resolved = scope.resolver.resolve_text(&named.identifier);
    if named.member.is_some()
        && let Some(declaration) = scope.generics.get(&resolved)
        && declaration.kind != DeclaredTypeKind::ClassLike
    {
        return Err(CompileError::new(
            CompileErrorKind::InvalidMemberType,
            format!(
                "the {} type `{resolved}` has no members",
                declaration.kind.name()
            ),
            named.span(),
        ));
    }
    if let Some(arguments) = &named.type_arguments {
        check_type_arguments(arguments)?;
    }

    if let Some(declaration) = scope.generics.get(&resolved)
        && (!defer_named_arity || declaration.is_alias)
        && !(declaration.is_callable && named.type_arguments.is_none())
    {
        let count = named
            .type_arguments
            .as_ref()
            .map_or(0, |arguments| arguments.arguments.len());
        check_arity(declaration, count, &resolved, named.span())?;
    }

    let arguments = named
        .type_arguments
        .as_ref()
        .map(|arguments| lower_type_arguments(scope, arguments, defer_named_arity))
        .transpose()?;
    if let Some(member) = &named.member {
        return lower_named_member(
            scope,
            named,
            member,
            &resolved,
            arguments,
            defer_named_arity,
        );
    }

    Ok(TypeDescriptor::Named {
        name: scope.resolver.resolve(scope.heap, &named.identifier),
        arguments,
        recursive: false,
    })
}

fn lower_array_type(
    scope: &TypeScope<'_>,
    array: &ArrayType<'_>,
    defer_named_arity: bool,
) -> Result<TypeDescriptor, CompileError> {
    let Some(arguments) = &array.type_arguments else {
        return Ok(TypeDescriptor::Array(None));
    };
    if arguments.arguments.len() != 2 {
        return Err(builtin_arity_error("array", 2, array.span()));
    }

    Ok(TypeDescriptor::Array(Some((
        Box::new(lower_type_inner(
            scope,
            arguments.arguments.as_slice()[0].r#type,
            defer_named_arity,
        )?),
        Box::new(lower_type_inner(
            scope,
            arguments.arguments.as_slice()[1].r#type,
            defer_named_arity,
        )?),
    ))))
}

fn lower_vec_type(
    scope: &TypeScope<'_>,
    vector: &VecType<'_>,
    defer_named_arity: bool,
) -> Result<TypeDescriptor, CompileError> {
    let Some(arguments) = &vector.type_arguments else {
        return Ok(TypeDescriptor::Vector(None));
    };
    if arguments.arguments.len() != 1 {
        return Err(builtin_arity_error("vec", 1, vector.span()));
    }

    Ok(TypeDescriptor::Vector(Some(Box::new(lower_type_inner(
        scope,
        arguments.arguments.as_slice()[0].r#type,
        defer_named_arity,
    )?))))
}

fn lower_vec_shape_type(
    scope: &TypeScope<'_>,
    shape: &VecShapeType<'_>,
    defer_named_arity: bool,
) -> Result<TypeDescriptor, CompileError> {
    let elements = shape
        .elements
        .iter()
        .map(|r#type| lower_type_inner(scope, r#type, defer_named_arity))
        .collect::<Result<Vec<_>, _>>()?;
    let rest = shape
        .trailing_type
        .as_ref()
        .map(|trailing| {
            trailing.r#type.map_or_else(
                || Ok(TypeDescriptor::Mixed),
                |r#type| lower_type_inner(scope, r#type, defer_named_arity),
            )
        })
        .transpose()?
        .map(Box::new);
    if elements.is_empty() && rest.is_none() {
        return Ok(TypeDescriptor::Vector(Some(Box::new(
            TypeDescriptor::Mixed,
        ))));
    }

    Ok(TypeDescriptor::VectorShape { elements, rest })
}

fn lower_dict_type(
    scope: &TypeScope<'_>,
    dictionary: &DictType<'_>,
    defer_named_arity: bool,
) -> Result<TypeDescriptor, CompileError> {
    let Some(arguments) = &dictionary.type_arguments else {
        return Ok(TypeDescriptor::Dictionary(None));
    };
    if arguments.arguments.len() != 2 {
        return Err(builtin_arity_error("dict", 2, dictionary.span()));
    }

    Ok(TypeDescriptor::Dictionary(Some((
        Box::new(lower_type_inner(
            scope,
            arguments.arguments.as_slice()[0].r#type,
            defer_named_arity,
        )?),
        Box::new(lower_type_inner(
            scope,
            arguments.arguments.as_slice()[1].r#type,
            defer_named_arity,
        )?),
    ))))
}

fn lower_dict_shape_type(
    scope: &TypeScope<'_>,
    shape: &DictShapeType<'_>,
    defer_named_arity: bool,
) -> Result<TypeDescriptor, CompileError> {
    let entries = shape
        .entries
        .iter()
        .map(|entry| {
            let key = match entry.key {
                Literal::String(string) => ShapeKey::String(scope.heap.intern(string.value)),
                Literal::Integer(integer) => {
                    ShapeKey::Int(i64::try_from(integer.value).map_err(|_| {
                        CompileError::new(
                            CompileErrorKind::IntegerLiteralOutOfRange,
                            "dictionary shape key does not fit in an integer",
                            integer.span,
                        )
                    })?)
                }
                _ => {
                    return Err(CompileError::new(
                        CompileErrorKind::TypeNotRuntimeCheckable,
                        "dictionary shape keys must be strings or integers",
                        entry.key.span(),
                    ));
                }
            };
            Ok((
                key,
                lower_type_inner(scope, entry.value, defer_named_arity)?,
            ))
        })
        .collect::<Result<Vec<_>, CompileError>>()?;
    let rest = shape
        .rest
        .as_ref()
        .map(|rest| {
            Ok((
                Box::new(lower_type_inner(
                    scope,
                    rest.type_arguments.key,
                    defer_named_arity,
                )?),
                Box::new(lower_type_inner(
                    scope,
                    rest.type_arguments.value,
                    defer_named_arity,
                )?),
            ))
        })
        .transpose()?;

    Ok(TypeDescriptor::DictionaryShape { entries, rest })
}

fn lower_tuple_type(
    scope: &TypeScope<'_>,
    tuple: &TupleType<'_>,
    defer_named_arity: bool,
) -> Result<TypeDescriptor, CompileError> {
    check_tuple_type(tuple)?;
    let elements = tuple
        .elements
        .iter()
        .map(|element| lower_type_inner(scope, element, defer_named_arity))
        .collect::<Result<Vec<_>, _>>()?;
    let rest = tuple
        .trailing_type
        .as_ref()
        .map(|rest| {
            rest.r#type.map_or_else(
                || Ok(TypeDescriptor::Mixed),
                |r#type| lower_type_inner(scope, r#type, defer_named_arity),
            )
        })
        .transpose()?
        .map(Box::new);

    Ok(match rest {
        Some(rest) => TypeDescriptor::TupleRest { elements, rest },
        None => TypeDescriptor::Tuple(elements),
    })
}

fn lower_union_type(
    scope: &TypeScope<'_>,
    source: &Type<'_>,
    defer_named_arity: bool,
) -> Result<TypeDescriptor, CompileError> {
    let types = flatten_union(source);
    validate_composition(scope, &types, true)?;
    let mut members = Vec::with_capacity(types.len());
    for member in types {
        if matches!(member.unparenthesized(), Type::Void(_)) {
            return Err(CompileError::new(
                CompileErrorKind::VoidInUnion,
                "`void` cannot be a member of a union type",
                member.span(),
            ));
        }
        members.push(lower_type_inner(scope, member, defer_named_arity)?);
    }

    Ok(TypeDescriptor::Union(members))
}

fn lower_intersection_type(
    scope: &TypeScope<'_>,
    source: &Type<'_>,
    defer_named_arity: bool,
) -> Result<TypeDescriptor, CompileError> {
    let types = flatten_intersection(source);
    validate_composition(scope, &types, false)?;
    let members = types
        .into_iter()
        .map(|member| lower_type_inner(scope, member, defer_named_arity))
        .collect::<Result<Vec<_>, _>>()?;

    Ok(TypeDescriptor::Intersection(members))
}

fn lower_function_type(
    scope: &TypeScope<'_>,
    function: &FunctionType<'_>,
    defer_named_arity: bool,
) -> Result<TypeDescriptor, CompileError> {
    let Some(signature) = &function.signature else {
        return Ok(TypeDescriptor::Callable(None));
    };

    check_sequence(
        CompileErrorKind::TooManyParameters,
        "a function type may declare",
        "parameters",
        signature.parameters,
    )?;

    let parameters = signature
        .parameters
        .iter()
        .map(|parameter| {
            Ok(FunctionTypeParameterDescriptor {
                r#type: lower_type_inner(scope, parameter.r#type, defer_named_arity)?,
                optional: parameter.equals.is_some(),
            })
        })
        .collect::<Result<Vec<_>, CompileError>>()?;
    Ok(TypeDescriptor::Callable(Some(FunctionTypeDescriptor {
        parameters,
        return_type: Box::new(lower_type_inner(
            scope,
            signature.return_type,
            defer_named_arity,
        )?),
    })))
}

fn lower_classname_type(
    scope: &TypeScope<'_>,
    classname: &ClassnameType<'_>,
    defer_named_arity: bool,
) -> Result<TypeDescriptor, CompileError> {
    let inner = lower_type_inner(scope, classname.inner, defer_named_arity)?;
    if !descriptor_may_be_class_like(&inner) {
        return Err(CompileError::new(
            CompileErrorKind::InvalidClassnameType,
            "`classname<T>` requires an inner type that can contain a class-like type",
            classname.inner.span(),
        ));
    }

    Ok(TypeDescriptor::Classname(Box::new(inner)))
}

fn lower_type_inner(
    scope: &TypeScope<'_>,
    source: &Type<'_>,
    defer_named_arity: bool,
) -> Result<TypeDescriptor, CompileError> {
    match source {
        Type::Parenthesized(parenthesized) => {
            lower_type_inner(scope, parenthesized.r#type, defer_named_arity)
        }
        Type::Mixed(_) => Ok(TypeDescriptor::Mixed),
        Type::Bool(_) => Ok(TypeDescriptor::Bool),
        Type::Int(_) => Ok(TypeDescriptor::Int),
        Type::Float(_) => Ok(TypeDescriptor::Float),
        Type::String(_) => Ok(TypeDescriptor::String),
        Type::Object(_) => Ok(TypeDescriptor::Object),
        Type::Void(_) => Ok(TypeDescriptor::Void),
        Type::Never(_) => Ok(TypeDescriptor::Never),
        Type::Negated(negated) => lower_negated_type(scope, negated, defer_named_arity),
        Type::Literal(literal) => lower_literal(scope.heap, literal),
        Type::NegativeLiteral(literal) => lower_negative_literal(literal),
        Type::IntegerRange(range) => lower_integer_range(range),
        Type::Self_(self_type) => {
            lower_self_type(scope, self_type, source.span(), defer_named_arity)
        }
        Type::Parent(_) => lower_parent_type(scope, source),
        Type::Static(_) => lower_static_type(scope, source.span()),
        Type::Named(named) => lower_named_type(scope, named, defer_named_arity),
        Type::Array(array) => lower_array_type(scope, array, defer_named_arity),
        Type::Vec(vector) => lower_vec_type(scope, vector, defer_named_arity),
        Type::VecShape(shape) => lower_vec_shape_type(scope, shape, defer_named_arity),
        Type::Dict(dictionary) => lower_dict_type(scope, dictionary, defer_named_arity),
        Type::DictShape(shape) => lower_dict_shape_type(scope, shape, defer_named_arity),
        Type::Tuple(tuple) => lower_tuple_type(scope, tuple, defer_named_arity),
        Type::Union(_) => lower_union_type(scope, source, defer_named_arity),
        Type::Intersection(_) => lower_intersection_type(scope, source, defer_named_arity),
        Type::Function(function) => lower_function_type(scope, function, defer_named_arity),
        Type::Classname(classname) => lower_classname_type(scope, classname, defer_named_arity),
    }
}

fn descriptor_may_be_class_like(descriptor: &TypeDescriptor) -> bool {
    match descriptor {
        TypeDescriptor::Wildcard
        | TypeDescriptor::Mixed
        | TypeDescriptor::Object
        | TypeDescriptor::Named { .. }
        | TypeDescriptor::Member { .. }
        | TypeDescriptor::Parameter(_)
        | TypeDescriptor::StaticClass
        | TypeDescriptor::Intersection(_)
        | TypeDescriptor::Negated(_) => true,
        TypeDescriptor::Union(members) => members.iter().any(descriptor_may_be_class_like),
        TypeDescriptor::Void
        | TypeDescriptor::Never
        | TypeDescriptor::Null
        | TypeDescriptor::Bool
        | TypeDescriptor::Int
        | TypeDescriptor::Float
        | TypeDescriptor::String
        | TypeDescriptor::TrueLiteral
        | TypeDescriptor::FalseLiteral
        | TypeDescriptor::IntLiteral(_)
        | TypeDescriptor::IntRange { .. }
        | TypeDescriptor::FloatLiteral(_)
        | TypeDescriptor::StringLiteral(_)
        | TypeDescriptor::Array(_)
        | TypeDescriptor::Vector(_)
        | TypeDescriptor::VectorShape { .. }
        | TypeDescriptor::Dictionary(_)
        | TypeDescriptor::DictionaryShape { .. }
        | TypeDescriptor::Callable(_)
        | TypeDescriptor::Classname(_)
        | TypeDescriptor::Tuple(_)
        | TypeDescriptor::TupleRest { .. }
        | TypeDescriptor::TupleAny => false,
    }
}

fn descriptor_has_parameter(descriptor: &TypeDescriptor) -> bool {
    match descriptor {
        TypeDescriptor::Parameter(_) => true,
        TypeDescriptor::Named { arguments, .. } => arguments
            .as_ref()
            .is_some_and(|arguments| arguments.iter().any(descriptor_has_parameter)),
        TypeDescriptor::Array(arguments) => arguments.as_ref().is_some_and(|(key, value)| {
            descriptor_has_parameter(key) || descriptor_has_parameter(value)
        }),
        TypeDescriptor::Vector(element) => element
            .as_ref()
            .is_some_and(|element| descriptor_has_parameter(element)),
        TypeDescriptor::Dictionary(arguments) => arguments.as_ref().is_some_and(|(key, value)| {
            descriptor_has_parameter(key) || descriptor_has_parameter(value)
        }),
        TypeDescriptor::VectorShape { elements, rest } => {
            elements.iter().any(descriptor_has_parameter)
                || rest
                    .as_ref()
                    .is_some_and(|rest| descriptor_has_parameter(rest))
        }
        TypeDescriptor::DictionaryShape { entries, rest } => {
            entries
                .iter()
                .any(|(_, value)| descriptor_has_parameter(value))
                || rest.as_ref().is_some_and(|(key, value)| {
                    descriptor_has_parameter(key) || descriptor_has_parameter(value)
                })
        }
        TypeDescriptor::Callable(signature) => signature.as_ref().is_some_and(|signature| {
            signature
                .parameters
                .iter()
                .any(|parameter| descriptor_has_parameter(&parameter.r#type))
                || descriptor_has_parameter(&signature.return_type)
        }),
        TypeDescriptor::Classname(inner) | TypeDescriptor::Negated(inner) => {
            descriptor_has_parameter(inner)
        }
        TypeDescriptor::TupleRest { elements, rest } => {
            elements.iter().any(descriptor_has_parameter) || descriptor_has_parameter(rest)
        }
        TypeDescriptor::Tuple(members)
        | TypeDescriptor::Union(members)
        | TypeDescriptor::Intersection(members) => members.iter().any(descriptor_has_parameter),
        TypeDescriptor::Wildcard
        | TypeDescriptor::Mixed
        | TypeDescriptor::Void
        | TypeDescriptor::Never
        | TypeDescriptor::Null
        | TypeDescriptor::Bool
        | TypeDescriptor::Int
        | TypeDescriptor::Float
        | TypeDescriptor::String
        | TypeDescriptor::Object
        | TypeDescriptor::TrueLiteral
        | TypeDescriptor::FalseLiteral
        | TypeDescriptor::IntLiteral(_)
        | TypeDescriptor::IntRange { .. }
        | TypeDescriptor::FloatLiteral(_)
        | TypeDescriptor::StringLiteral(_)
        | TypeDescriptor::Member { .. }
        | TypeDescriptor::StaticClass
        | TypeDescriptor::TupleAny => false,
    }
}

fn lower_literal(heap: &Heap, literal: &Literal<'_>) -> Result<TypeDescriptor, CompileError> {
    match literal {
        Literal::Null(_) => Ok(TypeDescriptor::Null),
        Literal::True(_) => Ok(TypeDescriptor::TrueLiteral),
        Literal::False(_) => Ok(TypeDescriptor::FalseLiteral),
        Literal::Integer(integer) => {
            let value = i64::try_from(integer.value).map_err(|_| {
                CompileError::new(
                    CompileErrorKind::IntegerLiteralOutOfRange,
                    format!("`{}` does not fit a 64-bit signed integer", integer.raw),
                    integer.span,
                )
            })?;

            Ok(TypeDescriptor::IntLiteral(value))
        }
        Literal::Float(float) => Ok(TypeDescriptor::FloatLiteral(float.value)),
        Literal::String(string) => Ok(TypeDescriptor::StringLiteral(heap.intern(string.value))),
    }
}

fn lower_negative_literal(
    literal: &NegativeLiteralType<'_>,
) -> Result<TypeDescriptor, CompileError> {
    match literal {
        NegativeLiteralType::Integer { minus, literal } => {
            let magnitude = literal.value;
            let value = if magnitude == (i64::MAX as u64) + 1 {
                i64::MIN
            } else {
                let magnitude = i64::try_from(magnitude).map_err(|_| {
                    CompileError::new(
                        CompileErrorKind::IntegerLiteralOutOfRange,
                        format!("`-{}` does not fit a 64-bit signed integer", literal.raw),
                        minus.join(literal.span),
                    )
                })?;

                -magnitude
            };

            Ok(TypeDescriptor::IntLiteral(value))
        }
        NegativeLiteralType::Float { literal, .. } => {
            Ok(TypeDescriptor::FloatLiteral(-literal.value))
        }
    }
}

fn lower_integer_range(range: &IntegerRangeType<'_>) -> Result<TypeDescriptor, CompileError> {
    let min = range
        .lower
        .as_ref()
        .map(lower_integer_range_bound)
        .transpose()?;
    let mut max = range
        .upper
        .as_ref()
        .map(lower_integer_range_bound)
        .transpose()?;
    if matches!(range.operator, IntegerRangeOperator::Exclusive(_))
        && let Some(upper) = max
    {
        let Some(inclusive) = upper.checked_sub(1) else {
            return Ok(TypeDescriptor::Never);
        };

        max = Some(inclusive);
    }

    Ok(TypeDescriptor::integer_range(min, max))
}

fn lower_integer_range_bound(bound: &IntegerRangeBound<'_>) -> Result<i64, CompileError> {
    match bound {
        IntegerRangeBound::Positive(literal) => i64::try_from(literal.value).map_err(|_| {
            CompileError::new(
                CompileErrorKind::IntegerLiteralOutOfRange,
                format!("`{}` does not fit a 64-bit signed integer", literal.raw),
                literal.span,
            )
        }),
        IntegerRangeBound::Negative { minus, literal } => {
            if literal.value == (i64::MAX as u64) + 1 {
                return Ok(i64::MIN);
            }

            i64::try_from(literal.value)
                .map(|value| -value)
                .map_err(|_| {
                    CompileError::new(
                        CompileErrorKind::IntegerLiteralOutOfRange,
                        format!("`-{}` does not fit a 64-bit signed integer", literal.raw),
                        minus.join(literal.span),
                    )
                })
        }
    }
}
