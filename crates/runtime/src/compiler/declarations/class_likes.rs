//! Class, interface, and enum declarations, their bases, and variance
//! validation.

use whim_span::HasSpan;
use whim_span::Span;
use whim_syn::cst::class::Class;
use whim_syn::cst::class::ClassLikeMember;
use whim_syn::cst::class::Enum;
use whim_syn::cst::class::Interface;
use whim_syn::cst::class::MethodBody;
use whim_syn::cst::r#type::NamedType;

use crate::bytecode::aliases::expand_aliases;
use crate::bytecode::chunk::descriptors::TypeDescriptor;
use crate::bytecode::unit::ClassLikeKind;
use crate::bytecode::unit::CompiledBaseReference;
use crate::bytecode::unit::CompiledClassLike;
use crate::bytecode::unit::CompiledTypeAlias;
use crate::bytecode::unit::CompiledTypeParameter;
use crate::bytecode::unit::CompiledUnit;
use crate::bytecode::unit::EnumBacking;
use crate::bytecode::unit::Variance;
use crate::compiler::declarations::Collection;
use crate::compiler::declarations::functions::DeclarationContext;
use crate::compiler::declarations::functions::compile_attributes;
use crate::compiler::declarations::generics::binder_names;
use crate::compiler::declarations::generics::compile_type_parameters;
use crate::compiler::declarations::members::compile_members;
use crate::compiler::emit::Scope;
use crate::compiler::error::CompileError;
use crate::compiler::error::CompileErrorKind;
use crate::compiler::limits::check_sequence;
use crate::compiler::names::Resolver;
use crate::compiler::rules;
use crate::compiler::types;
use crate::compiler::types::ClassContext;
use crate::compiler::types::GenericTable;
use crate::compiler::types::TypeScope;
use crate::value::heap::Heap;
use crate::variance::incompatible_parameter;

fn base_reference(
    scope: &Scope<'_>,
    aliases: &[CompiledTypeAlias],
    named: &NamedType<'_>,
) -> Result<CompiledBaseReference, CompileError> {
    let type_scope = TypeScope {
        heap: scope.heap,
        resolver: scope.resolver,
        class: scope.class,
        aliases,
        binders: &scope.binders,
        forbidden_binders: &scope.forbidden_binders,
        generics: scope.generics,
    };

    let type_arguments = named
        .type_arguments
        .as_ref()
        .map(|arguments| {
            arguments
                .arguments
                .iter()
                .map(|argument| {
                    types::lowering::reject_return_only_annotation(
                        argument.r#type,
                        "base type argument",
                    )?;
                    types::lowering::lower_type_argument(&type_scope, argument.r#type)
                })
                .collect()
        })
        .transpose()?;

    Ok(CompiledBaseReference {
        name: scope.resolver.resolve(scope.heap, &named.identifier),
        type_arguments,
        span: named.span(),
    })
}

fn base_references<'arena>(
    scope: &Scope<'_>,
    aliases: &[CompiledTypeAlias],
    named_types: impl Iterator<Item = &'arena NamedType<'arena>>,
) -> Result<Vec<CompiledBaseReference>, CompileError> {
    named_types
        .map(|named| base_reference(scope, aliases, named))
        .collect()
}

fn class_parent(resolver: &Resolver, class: &Class<'_>) -> Result<Option<String>, CompileError> {
    if let Some(extends) = &class.extends
        && extends.types.len() > 1
    {
        return Err(CompileError::new(
            CompileErrorKind::MultipleBaseClasses,
            "a class extends at most one base class",
            extends.span(),
        ));
    }

    Ok(class.extends.as_ref().and_then(|extends| {
        extends
            .types
            .first()
            .map(|named| resolver.resolve_text(&named.identifier))
    }))
}

fn check_concrete_class_members(class: &Class<'_>) -> Result<(), CompileError> {
    if class.is_abstract() {
        return Ok(());
    }

    for member in class.members {
        if let ClassLikeMember::Method(method) = member
            && matches!(method.body, MethodBody::Abstract(_))
        {
            return Err(CompileError::new(
                CompileErrorKind::AbstractMethodInConcreteClass,
                "an abstract method may appear only in an abstract class",
                method.name.span(),
            ));
        }
    }

    Ok(())
}

/// Enforces a generic class or interface parameter's declared variance at
/// every occurrence in its public runtime type structure. Polarity is `1` for
/// output, `-1` for input, and `0` for an invariant position.
pub(in crate::compiler) fn validate_variance_use(
    descriptor: &TypeDescriptor,
    polarity: i8,
    class_parameters: &[CompiledTypeParameter],
    generics: &GenericTable<'_>,
    span: Span,
) -> Result<(), CompileError> {
    let incompatible =
        incompatible_parameter(descriptor, polarity, class_parameters, |name, index| {
            generics
                .get(&name.to_string_lossy().into_owned())
                .and_then(|declaration| declaration.variances.get(index))
                .copied()
        });
    let Some(parameter) = incompatible else {
        return Ok(());
    };

    let variance = match parameter.variance {
        Variance::Covariant => "covariant",
        Variance::Contravariant => "contravariant",
        Variance::Invariant => "invariant",
    };

    Err(CompileError::new(
        CompileErrorKind::InvalidVarianceUse,
        format!(
            "the {variance} type parameter `{}` is used in an incompatible position",
            parameter.name.to_string_lossy()
        ),
        span,
    ))
}

fn validate_base_variance(
    base: &CompiledBaseReference,
    class_parameters: &[CompiledTypeParameter],
    generics: &GenericTable<'_>,
) -> Result<(), CompileError> {
    let Some(arguments) = &base.type_arguments else {
        return Ok(());
    };

    let Some(declaration) = generics.get(&base.name.to_string_lossy().into_owned()) else {
        return Ok(());
    };
    for (index, argument) in arguments.iter().enumerate() {
        let polarity = match declaration
            .variances
            .get(index)
            .copied()
            .unwrap_or(Variance::Invariant)
        {
            Variance::Invariant => 0,
            Variance::Covariant => 1,
            Variance::Contravariant => -1,
        };

        validate_variance_use(argument, polarity, class_parameters, generics, base.span)?;
    }

    Ok(())
}

fn validate_bases(
    class: &CompiledClassLike,
    generics: &GenericTable<'_>,
) -> Result<(), CompileError> {
    if let Some(parent) = &class.parent {
        validate_base_variance(parent, &class.type_parameters, generics)?;
    }
    for interface in &class.interfaces {
        validate_base_variance(interface, &class.type_parameters, generics)?;
    }

    Ok(())
}

fn class_like(heap: &Heap, name: &str, kind: ClassLikeKind, span: Span) -> CompiledClassLike {
    CompiledClassLike {
        name: heap.intern(name.as_bytes()),
        span,
        kind,
        type_parameters: Vec::new(),
        is_abstract: false,
        is_final: false,
        is_readonly: false,
        parent: None,
        interfaces: Vec::new(),
        constants: Vec::new(),
        properties: Vec::new(),
        methods: Vec::new(),
        cases: Vec::new(),
        enum_backing: None,
        attributes: Vec::new(),
        sealed_to: None,
    }
}

pub(in crate::compiler::declarations) fn compile_class<'arena>(
    collection: &Collection<'_, 'arena>,
    resolver: &Resolver,
    class: &Class<'arena>,
    unit: &mut CompiledUnit,
) -> Result<CompiledClassLike, CompileError> {
    let &Collection {
        heap,
        path,
        runtime_path,
        source_text,
        line_starts,
        generics,
        embedded_files,
        trusted_returns,
    } = collection;
    rules::check_class(class)?;
    let name = resolver.qualify(class.name.value);
    let parent = class_parent(resolver, class)?;
    check_concrete_class_members(class)?;

    let mut output = class_like(heap, &name, ClassLikeKind::Class, class.span());
    output.is_abstract = class.is_abstract();
    output.is_final = class.is_final();
    output.is_readonly = class.is_readonly();
    output.sealed_to = class.permissions.as_ref().map(|permissions| {
        permissions
            .types
            .iter()
            .map(|name| resolver.resolve(heap, name))
            .collect()
    });
    let type_parameters = binder_names(class.type_parameters.as_ref());
    let preliminary_context = ClassContext {
        name: name.clone(),
        type_parameters: type_parameters.clone(),
        parent: parent.clone(),
        parent_arguments: None,
    };

    let preliminary_scope = Scope {
        heap,
        runtime_path,
        line_starts,
        resolver,
        class: Some(&preliminary_context),
        binders: type_parameters.clone(),
        forbidden_binders: Vec::new(),
        generics,
        embedded_files,
        trusted_returns,
    };

    output.parent = class
        .extends
        .as_ref()
        .and_then(|extends| extends.types.first())
        .map(|named| base_reference(&preliminary_scope, &unit.type_aliases, named))
        .transpose()?;

    let context = ClassContext {
        name,
        type_parameters: type_parameters.clone(),
        parent,
        parent_arguments: output
            .parent
            .as_ref()
            .and_then(|parent| parent.type_arguments.clone()),
    };

    let scope = Scope {
        class: Some(&context),
        binders: type_parameters,
        ..preliminary_scope
    };

    if let Some(implements) = &class.implements {
        check_sequence(
            CompileErrorKind::TooManyInterfaces,
            "a class may implement",
            "interfaces",
            implements.types,
        )?;

        output.interfaces = base_references(&scope, &unit.type_aliases, implements.types.iter())?;
    }

    output.type_parameters = compile_type_parameters(
        heap,
        &scope,
        &unit.type_aliases,
        class.type_parameters.as_ref(),
    )?;

    validate_bases(&output, generics)?;

    output.attributes = compile_attributes(
        heap,
        &scope,
        class.attribute_lists,
        path,
        source_text,
        &mut DeclarationContext::for_unit(unit),
    )?;
    compile_members(
        &scope,
        &context,
        class.members,
        path,
        source_text,
        unit,
        &mut output,
    )?;

    Ok(output)
}

pub(in crate::compiler::declarations) fn compile_interface<'arena>(
    collection: &Collection<'_, 'arena>,
    resolver: &Resolver,
    interface: &Interface<'arena>,
    unit: &mut CompiledUnit,
) -> Result<CompiledClassLike, CompileError> {
    let &Collection {
        heap,
        path,
        runtime_path,
        source_text,
        line_starts,
        generics,
        embedded_files,
        trusted_returns,
    } = collection;
    let name = resolver.qualify(interface.name.value);
    let mut output = class_like(heap, &name, ClassLikeKind::Interface, interface.span());
    let type_parameters = binder_names(interface.type_parameters.as_ref());
    let context = ClassContext {
        name,
        type_parameters: type_parameters.clone(),
        parent: None,
        parent_arguments: None,
    };

    let scope = Scope {
        heap,
        runtime_path,
        line_starts,
        resolver,
        class: Some(&context),
        binders: type_parameters,
        forbidden_binders: Vec::new(),
        generics,
        embedded_files,
        trusted_returns,
    };
    rules::check_interface(interface)?;

    if let Some(extends) = &interface.extends {
        check_sequence(
            CompileErrorKind::TooManyInterfaces,
            "an interface may extend",
            "interfaces",
            extends.types,
        )?;

        output.interfaces = base_references(&scope, &unit.type_aliases, extends.types.iter())?;
    }

    output.type_parameters = compile_type_parameters(
        heap,
        &scope,
        &unit.type_aliases,
        interface.type_parameters.as_ref(),
    )?;

    validate_bases(&output, generics)?;

    output.attributes = compile_attributes(
        heap,
        &scope,
        interface.attribute_lists,
        path,
        source_text,
        &mut DeclarationContext::for_unit(unit),
    )?;

    output.sealed_to = interface.permissions.as_ref().map(|permissions| {
        permissions
            .types
            .iter()
            .map(|name| resolver.resolve(heap, name))
            .collect()
    });

    compile_members(
        &scope,
        &context,
        interface.members,
        path,
        source_text,
        unit,
        &mut output,
    )?;

    Ok(output)
}

pub(in crate::compiler::declarations) fn compile_enum<'arena>(
    collection: &Collection<'_, 'arena>,
    resolver: &Resolver,
    declaration: &Enum<'arena>,
    unit: &mut CompiledUnit,
) -> Result<CompiledClassLike, CompileError> {
    let &Collection {
        heap,
        path,
        runtime_path,
        source_text,
        line_starts,
        generics,
        embedded_files,
        trusted_returns,
    } = collection;
    rules::check_enum(declaration)?;
    let name = resolver.qualify(declaration.name.value);
    if let Some(type_parameters) = &declaration.type_parameters {
        return Err(CompileError::new(
            CompileErrorKind::GenericEnum,
            "an enum cannot declare type parameters; generics are not allowed on enums",
            type_parameters.span(),
        ));
    }

    let mut output = class_like(heap, &name, ClassLikeKind::Enum, declaration.span());
    if let Some(backing) = &declaration.backing_type {
        let type_scope = TypeScope {
            heap,
            resolver,
            class: None,
            aliases: &unit.type_aliases,
            binders: &[],
            forbidden_binders: &[],
            generics,
        };
        let descriptor = types::lowering::lower_type(&type_scope, backing.r#type)?;
        output.enum_backing = Some(match expand_aliases(&descriptor, &unit.type_aliases) {
            TypeDescriptor::Int => EnumBacking::Int,
            TypeDescriptor::String => EnumBacking::String,
            _ => {
                return Err(CompileError::new(
                    CompileErrorKind::InvalidEnumBacking,
                    "an enum backing type must be `int` or `string`",
                    backing.r#type.span(),
                ));
            }
        });
    }

    let context = ClassContext {
        name,
        type_parameters: Vec::new(),
        parent: None,
        parent_arguments: None,
    };

    let scope = Scope {
        heap,
        runtime_path,
        line_starts,
        resolver,
        class: Some(&context),
        binders: Vec::new(),
        forbidden_binders: Vec::new(),
        generics,
        embedded_files,
        trusted_returns,
    };

    if let Some(implements) = &declaration.implements {
        check_sequence(
            CompileErrorKind::TooManyInterfaces,
            "an enum may implement",
            "interfaces",
            implements.types,
        )?;
        output.interfaces = base_references(&scope, &unit.type_aliases, implements.types.iter())?;
    }

    output.attributes = compile_attributes(
        heap,
        &scope,
        declaration.attribute_lists,
        path,
        source_text,
        &mut DeclarationContext::for_unit(unit),
    )?;

    compile_members(
        &scope,
        &context,
        declaration.members,
        path,
        source_text,
        unit,
        &mut output,
    )?;

    Ok(output)
}
