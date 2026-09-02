//! Declaration collection: the program's namespaces, functions, class-likes,
//! constants, and type aliases into the compiled unit.

use std::mem;

use hashbrown::HashSet;

use whim_span::HasSpan;
use whim_span::Span;
use whim_syn::cst::Program;
use whim_syn::cst::class::Class;
use whim_syn::cst::class::Enum;
use whim_syn::cst::class::Interface;
use whim_syn::cst::declaration::Constant;
use whim_syn::cst::declaration::Namespace;
use whim_syn::cst::function::Function;
use whim_syn::cst::statement::Statement;
use whim_syn::cst::r#type::Newtype;
use whim_syn::cst::r#type::Type;
use whim_syn::cst::r#type::TypeAlias;

use crate::bytecode::chunk::descriptors::Literal as BytecodeLiteral;
use crate::bytecode::chunk::descriptors::TypeDescriptor;
use crate::bytecode::unit::CompiledConstant;
use crate::bytecode::unit::CompiledNewtype;
use crate::bytecode::unit::CompiledTypeAlias;
use crate::bytecode::unit::CompiledUnit;
use crate::bytecode::unit::ConstantInitializer;
use crate::bytecode::unit::STUB_ATTRIBUTE_NAME;
use crate::compiler::Compilation;
use crate::compiler::CompilePath;
use crate::compiler::embed::EmbeddedFiles;
use crate::compiler::emit::Scope;
use crate::compiler::error::CompileError;
use crate::compiler::error::CompileErrorKind;
use crate::compiler::names::Resolver;
use crate::compiler::rules;
use crate::compiler::types;
use crate::compiler::types::AliasGraph;
use crate::compiler::types::GenericTable;
use crate::compiler::types::TypeScope;
use crate::compiler::types::aliases::collect_alias_references;
use crate::compiler::types::aliases::find_alias_cycle;
use crate::compiler::types::lowering::lower_type;
use crate::compiler::types::rendering::render_type;
use crate::value::heap::Heap;

pub(in crate::compiler) mod class_likes;
pub(in crate::compiler) mod functions;
pub(in crate::compiler) mod generics;
mod members;
pub(in crate::compiler) mod sealed;

use crate::compiler::declarations::class_likes::compile_class;
use crate::compiler::declarations::class_likes::compile_enum;
use crate::compiler::declarations::class_likes::compile_interface;
use crate::compiler::declarations::class_likes::validate_variance_use;
use crate::compiler::declarations::functions::DeclarationContext;
use crate::compiler::declarations::functions::compile_attributes;
use crate::compiler::declarations::functions::compile_function_declaration;
use crate::compiler::declarations::functions::compile_initializer;
use crate::compiler::declarations::functions::has_attribute;
use crate::compiler::declarations::generics::binder_names;
use crate::compiler::declarations::generics::compile_type_parameters;

pub(in crate::compiler) struct Region<'source, 'arena> {
    pub(in crate::compiler) resolver: Resolver,
    pub(in crate::compiler) declared_names: HashSet<String>,
    pub(in crate::compiler) main_statements: Vec<(&'source Statement<'arena>, Resolver)>,
}

#[derive(Clone, Copy)]
pub(in crate::compiler::declarations) struct Array<'compilation, 'arena> {
    heap: &'compilation Heap,
    path: &'compilation str,
    runtime_path: &'compilation [u8],
    source_text: &'compilation str,
    line_starts: &'compilation [u32],
    generics: &'compilation GenericTable<'arena>,
    embedded_files: &'compilation EmbeddedFiles,
    trusted_returns: bool,
}

pub(in crate::compiler) fn collect<'source, 'arena>(
    heap: &Heap,
    program: &'source Program<'arena>,
    path: CompilePath<'_>,
    unit: &mut CompiledUnit,
    compilation: &mut Compilation<'_, 'arena>,
) -> Result<Vec<Region<'source, 'arena>>, CompileError> {
    let context = Array {
        heap,
        path: path.diagnostic,
        runtime_path: path.runtime,
        source_text: program.source_text,
        line_starts: compilation.line_starts,
        generics: compilation.generics,
        embedded_files: compilation.embedded_files,
        trusted_returns: compilation.trusted_return_types,
    };
    let mut regions = Vec::new();
    collect_statements(
        &context,
        program.statements,
        Resolver::default(),
        &mut *compilation.aliases,
        unit,
        &mut regions,
    )?;
    Ok(regions)
}

fn collect_statements<'source, 'arena>(
    context: &Array<'_, 'arena>,
    statements: &'source [Statement<'arena>],
    resolver: Resolver,
    aliases: &mut AliasGraph,
    unit: &mut CompiledUnit,
    regions: &mut Vec<Region<'source, 'arena>>,
) -> Result<(), CompileError> {
    let mut region = Region {
        resolver,
        declared_names: HashSet::new(),
        main_statements: Vec::new(),
    };

    for statement in statements {
        match statement {
            Statement::Namespace(namespace) => {
                collect_namespace(context, namespace, &mut region, aliases, unit, regions)?;
            }
            Statement::Use(declaration) => region
                .resolver
                .collect_use(declaration, &region.declared_names)?,
            Statement::Function(function) => {
                collect_function(context, &mut region, function, unit)?;
            }
            Statement::Class(class) => collect_class(context, &mut region, class, unit)?,
            Statement::Interface(interface) => {
                collect_interface(context, &mut region, interface, unit)?;
            }
            Statement::Enum(declaration) => {
                collect_enum(context, &mut region, declaration, unit)?;
            }
            Statement::Constant(constant) => {
                collect_constant(context, &mut region, constant, unit)?;
            }
            Statement::TypeAlias(alias) => {
                collect_type_alias(context, &mut region, alias, aliases, unit)?;
            }
            Statement::Newtype(newtype) => {
                collect_newtype(context, &mut region, newtype, unit)?;
            }
            other => region
                .main_statements
                .push((other, region.resolver.clone())),
        }
    }

    regions.push(region);
    Ok(())
}

fn collect_function(
    context: &Array<'_, '_>,
    region: &mut Region<'_, '_>,
    function: &Function<'_>,
    unit: &mut CompiledUnit,
) -> Result<(), CompileError> {
    note_declared_name(region, function.name.value, function.name.span())?;
    let scope = Scope {
        heap: context.heap,
        runtime_path: context.runtime_path,
        line_starts: context.line_starts,
        resolver: &region.resolver,
        class: None,
        binders: binder_names(function.type_parameters.as_ref()),
        forbidden_binders: Vec::new(),
        generics: context.generics,
        embedded_files: context.embedded_files,
        trusted_returns: context.trusted_returns,
    };
    let compiled = compile_function_declaration(
        context.heap,
        &scope,
        function,
        context.path,
        context.source_text,
        unit,
    )?;
    unit.functions.push(compiled);
    Ok(())
}

fn collect_class(
    context: &Array<'_, '_>,
    region: &mut Region<'_, '_>,
    class: &Class<'_>,
    unit: &mut CompiledUnit,
) -> Result<(), CompileError> {
    note_declared_name(region, class.name.value, class.name.span())?;
    let compiled = compile_class(context, &region.resolver, class, unit)?;
    unit.classes.push(compiled);
    Ok(())
}

fn collect_interface(
    context: &Array<'_, '_>,
    region: &mut Region<'_, '_>,
    interface: &Interface<'_>,
    unit: &mut CompiledUnit,
) -> Result<(), CompileError> {
    note_declared_name(region, interface.name.value, interface.name.span())?;
    let compiled = compile_interface(context, &region.resolver, interface, unit)?;
    unit.classes.push(compiled);
    Ok(())
}

fn collect_enum(
    context: &Array<'_, '_>,
    region: &mut Region<'_, '_>,
    declaration: &Enum<'_>,
    unit: &mut CompiledUnit,
) -> Result<(), CompileError> {
    note_declared_name(region, declaration.name.value, declaration.name.span())?;
    let compiled = compile_enum(context, &region.resolver, declaration, unit)?;
    unit.classes.push(compiled);
    Ok(())
}

fn collect_constant(
    context: &Array<'_, '_>,
    region: &mut Region<'_, '_>,
    constant: &Constant<'_>,
    unit: &mut CompiledUnit,
) -> Result<(), CompileError> {
    note_declared_name(region, constant.name.value, constant.name.span())?;
    let scope = Scope {
        heap: context.heap,
        runtime_path: context.runtime_path,
        line_starts: context.line_starts,
        resolver: &region.resolver,
        class: None,
        binders: Vec::new(),
        forbidden_binders: Vec::new(),
        generics: context.generics,
        embedded_files: context.embedded_files,
        trusted_returns: context.trusted_returns,
    };
    let attributes = compile_attributes(
        context.heap,
        &scope,
        constant.attribute_lists,
        context.path,
        context.source_text,
        &mut DeclarationContext::for_unit(unit),
    )?;
    let initializer = if has_attribute(&scope, constant.attribute_lists, STUB_ATTRIBUTE_NAME) {
        ConstantInitializer::Literal(BytecodeLiteral::Null)
    } else {
        rules::check_constant_initializer(constant.value)?;
        compile_initializer(
            context.heap,
            &scope,
            constant.value,
            context.path,
            context.source_text,
            &mut DeclarationContext::for_unit(unit),
        )?
    };
    unit.constants.push(CompiledConstant {
        name: context
            .heap
            .intern(region.resolver.qualify(constant.name.value).as_bytes()),
        span: constant.span(),
        attributes,
        initializer,
    });
    Ok(())
}

fn collect_type_alias(
    context: &Array<'_, '_>,
    region: &mut Region<'_, '_>,
    alias: &TypeAlias<'_>,
    aliases: &mut AliasGraph,
    unit: &mut CompiledUnit,
) -> Result<(), CompileError> {
    note_declared_name(region, alias.name.value, alias.name.span())?;
    let binders = binder_names(alias.type_parameters.as_ref());
    let scope = Scope {
        heap: context.heap,
        runtime_path: context.runtime_path,
        line_starts: context.line_starts,
        resolver: &region.resolver,
        class: None,
        binders: binders.clone(),
        forbidden_binders: Vec::new(),
        generics: context.generics,
        embedded_files: context.embedded_files,
        trusted_returns: context.trusted_returns,
    };
    if has_attribute(&scope, alias.attribute_lists, STUB_ATTRIBUTE_NAME) {
        return collect_stub_type_alias(context, region, alias, &scope, unit);
    }
    if matches!(alias.aliased.unparenthesized(), Type::Void(_)) {
        return Err(CompileError::new(
            CompileErrorKind::AliasOfVoid,
            "a type alias cannot alias bare `void`",
            alias.aliased.span(),
        ));
    }
    types::lowering::reject_return_only_annotation(alias.aliased, "aliased value")?;
    record_alias(context, region, alias, &binders, aliases);
    let type_scope = TypeScope {
        heap: context.heap,
        resolver: &region.resolver,
        class: None,
        aliases: &unit.type_aliases,
        binders: &binders,
        forbidden_binders: &[],
        generics: context.generics,
    };
    let descriptor = lower_type(&type_scope, alias.aliased)?;
    let rendered = render_type(&type_scope, alias.aliased)?;
    let type_parameters = compile_type_parameters(
        context.heap,
        &scope,
        &unit.type_aliases,
        alias.type_parameters.as_ref(),
    )?;
    validate_variance_use(
        &descriptor,
        1,
        &type_parameters,
        context.generics,
        alias.aliased.span(),
    )?;
    let attributes = compile_attributes(
        context.heap,
        &scope,
        alias.attribute_lists,
        context.path,
        context.source_text,
        &mut DeclarationContext::for_unit(unit),
    )?;
    unit.type_aliases.push(CompiledTypeAlias {
        name: context
            .heap
            .intern(region.resolver.qualify(alias.name.value).as_bytes()),
        span: alias.span(),
        attributes,
        type_parameters,
        descriptor,
        rendered: context.heap.intern(rendered.as_bytes()),
    });
    Ok(())
}

fn collect_stub_type_alias(
    context: &Array<'_, '_>,
    region: &Region<'_, '_>,
    alias: &TypeAlias<'_>,
    scope: &Scope<'_>,
    unit: &mut CompiledUnit,
) -> Result<(), CompileError> {
    let type_parameters = compile_type_parameters(
        context.heap,
        scope,
        &unit.type_aliases,
        alias.type_parameters.as_ref(),
    )?;
    let attributes = compile_attributes(
        context.heap,
        scope,
        alias.attribute_lists,
        context.path,
        context.source_text,
        &mut DeclarationContext::for_unit(unit),
    )?;
    unit.type_aliases.push(CompiledTypeAlias {
        name: context
            .heap
            .intern(region.resolver.qualify(alias.name.value).as_bytes()),
        span: alias.span(),
        attributes,
        type_parameters,
        descriptor: TypeDescriptor::Mixed,
        rendered: context.heap.intern(b"mixed"),
    });
    Ok(())
}

fn record_alias(
    context: &Array<'_, '_>,
    region: &Region<'_, '_>,
    alias: &TypeAlias<'_>,
    binders: &[String],
    aliases: &mut AliasGraph,
) {
    let qualified = region.resolver.qualify(alias.name.value);
    let mut references = Vec::new();
    collect_alias_references(
        &region.resolver,
        context.generics,
        alias.aliased,
        binders,
        &mut references,
    );
    aliases.insert(qualified, references, alias.aliased.span());
}

pub(in crate::compiler) fn validate_alias_cycles(aliases: &AliasGraph) -> Result<(), CompileError> {
    let Some(cycle) = find_alias_cycle(aliases) else {
        return Ok(());
    };
    let message = if cycle.path.len() > 2 {
        format!(
            "the type alias `{}` expands into itself: {}",
            cycle.path[0],
            cycle.path.join(" -> ")
        )
    } else {
        format!("the type alias `{}` expands into itself", cycle.path[0])
    };
    Err(CompileError::new(
        CompileErrorKind::RecursiveTypeAlias,
        message,
        cycle.span,
    ))
}

fn collect_newtype(
    context: &Array<'_, '_>,
    region: &mut Region<'_, '_>,
    newtype: &Newtype<'_>,
    unit: &mut CompiledUnit,
) -> Result<(), CompileError> {
    note_declared_name(region, newtype.name.value, newtype.name.span())?;
    let binders = binder_names(newtype.type_parameters.as_ref());
    let scope = Scope {
        heap: context.heap,
        runtime_path: context.runtime_path,
        line_starts: context.line_starts,
        resolver: &region.resolver,
        class: None,
        binders: binders.clone(),
        forbidden_binders: Vec::new(),
        generics: context.generics,
        embedded_files: context.embedded_files,
        trusted_returns: context.trusted_returns,
    };
    if has_attribute(&scope, newtype.attribute_lists, STUB_ATTRIBUTE_NAME) {
        return collect_stub_newtype(context, region, newtype, &scope, unit);
    }
    if matches!(newtype.backing.unparenthesized(), Type::Void(_)) {
        return Err(CompileError::new(
            CompileErrorKind::AliasOfVoid,
            "a newtype cannot have bare `void` as its backing type",
            newtype.backing.span(),
        ));
    }
    types::lowering::reject_return_only_annotation(newtype.backing, "newtype backing value")?;
    let type_scope = TypeScope {
        heap: context.heap,
        resolver: &region.resolver,
        class: None,
        aliases: &unit.type_aliases,
        binders: &binders,
        forbidden_binders: &[],
        generics: context.generics,
    };
    let backing = lower_type(&type_scope, newtype.backing)?;
    let type_parameters = compile_type_parameters(
        context.heap,
        &scope,
        &unit.type_aliases,
        newtype.type_parameters.as_ref(),
    )?;
    validate_variance_use(
        &backing,
        1,
        &type_parameters,
        context.generics,
        newtype.backing.span(),
    )?;
    let attributes = compile_attributes(
        context.heap,
        &scope,
        newtype.attribute_lists,
        context.path,
        context.source_text,
        &mut DeclarationContext::for_unit(unit),
    )?;
    unit.newtypes.push(CompiledNewtype {
        name: context
            .heap
            .intern(region.resolver.qualify(newtype.name.value).as_bytes()),
        span: newtype.span(),
        attributes,
        type_parameters,
        backing,
    });
    Ok(())
}

fn collect_stub_newtype(
    context: &Array<'_, '_>,
    region: &Region<'_, '_>,
    newtype: &Newtype<'_>,
    scope: &Scope<'_>,
    unit: &mut CompiledUnit,
) -> Result<(), CompileError> {
    let type_parameters = compile_type_parameters(
        context.heap,
        scope,
        &unit.type_aliases,
        newtype.type_parameters.as_ref(),
    )?;
    let attributes = compile_attributes(
        context.heap,
        scope,
        newtype.attribute_lists,
        context.path,
        context.source_text,
        &mut DeclarationContext::for_unit(unit),
    )?;
    unit.newtypes.push(CompiledNewtype {
        name: context
            .heap
            .intern(region.resolver.qualify(newtype.name.value).as_bytes()),
        span: newtype.span(),
        attributes,
        type_parameters,
        backing: TypeDescriptor::Mixed,
    });
    Ok(())
}

fn note_declared_name(
    region: &mut Region<'_, '_>,
    name: &str,
    span: Span,
) -> Result<(), CompileError> {
    if region.resolver.has_alias(name) {
        return Err(CompileError::new(
            CompileErrorKind::DuplicateImportAlias,
            format!("the declaration `{name}` collides with an import of the same name"),
            span,
        ));
    }
    region.declared_names.insert(name.to_string());

    Ok(())
}

fn collect_namespace<'source, 'arena>(
    context: &Array<'_, 'arena>,
    namespace: &'source Namespace<'arena>,
    region: &mut Region<'source, 'arena>,
    aliases: &mut AliasGraph,
    unit: &mut CompiledUnit,
    regions: &mut Vec<Region<'source, 'arena>>,
) -> Result<(), CompileError> {
    let resumed = Region {
        resolver: region.resolver.clone(),
        declared_names: region.declared_names.clone(),
        main_statements: Vec::new(),
    };
    regions.push(mem::replace(region, resumed));
    collect_statements(
        context,
        namespace.statements(),
        Resolver::for_namespace(&namespace_name(namespace)),
        aliases,
        unit,
        regions,
    )
}

fn namespace_name(namespace: &Namespace<'_>) -> String {
    let name = namespace.name.value();

    name.strip_prefix('\\').unwrap_or(name).to_string()
}
