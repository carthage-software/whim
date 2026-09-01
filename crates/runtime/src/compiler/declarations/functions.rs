//! Function declarations, parameters, initializers, attributes, and
//! signature rendering.

use whim_span::HasSpan;
use whim_span::Span;
use whim_syn::cst::atom::Literal;
use whim_syn::cst::call::Argument;
use whim_syn::cst::construct::Construct;
use whim_syn::cst::declaration::AttributeList;
use whim_syn::cst::expression::Expression;
use whim_syn::cst::function::Function;
use whim_syn::cst::function::ParameterList;
use whim_syn::cst::operation::UnaryPrefixOperator;
use whim_syn::cst::statement::Block;
use whim_syn::cst::r#type::Type;
use whim_syn::cst::r#type::TypeParameter;
use whim_syn::cst::r#type::TypeParameterList;
use whim_syn::cst::r#type::TypeVariance;

use crate::bytecode::chunk::Chunk;
use crate::bytecode::chunk::descriptors::Literal as BytecodeLiteral;
use crate::bytecode::chunk::descriptors::TypeDescriptor;
use crate::bytecode::instruction::Instruction;
use crate::bytecode::unit::CompiledAttribute;
use crate::bytecode::unit::CompiledFunction;
use crate::bytecode::unit::CompiledParameter;
use crate::bytecode::unit::CompiledTypeAlias;
use crate::bytecode::unit::CompiledUnit;
use crate::bytecode::unit::ConstantInitializer;
use crate::bytecode::unit::STUB_ATTRIBUTE_NAME;
use crate::compiler::declarations::class_likes::validate_variance_use;
use crate::compiler::declarations::generics::compile_type_parameters;
use crate::compiler::emit::BodyCompiler;
use crate::compiler::emit::BodyShape;
use crate::compiler::emit::ReturnKind;
use crate::compiler::emit::Scope;
use crate::compiler::emit::analysis::collect_assigned_in_statements;
use crate::compiler::emit::integer_gate;
use crate::compiler::error::CompileError;
use crate::compiler::error::CompileErrorKind;
use crate::compiler::limits::check_count;
use crate::compiler::rules;
use crate::compiler::types;
use crate::compiler::types::TypeScope;
use crate::compiler::types::lowering::lower_type;
use crate::compiler::types::rendering::render_annotation;
use crate::compiler::types::rendering::render_type;
use crate::value::heap::Heap;

/// State shared by declaration metadata and initializer thunks.
pub(in crate::compiler) struct DeclarationContext<'compilation> {
    aliases: &'compilation [CompiledTypeAlias],
    synthesized: &'compilation mut Vec<CompiledFunction>,
}

impl<'compilation> DeclarationContext<'compilation> {
    pub(in crate::compiler) fn for_unit(unit: &'compilation mut CompiledUnit) -> Self {
        Self {
            aliases: &unit.type_aliases,
            synthesized: &mut unit.functions,
        }
    }

    pub(in crate::compiler) const fn new(
        aliases: &'compilation [CompiledTypeAlias],
        synthesized: &'compilation mut Vec<CompiledFunction>,
    ) -> Self {
        Self {
            aliases,
            synthesized,
        }
    }
}

struct LoweredReturn {
    descriptor: Option<TypeDescriptor>,
    kind: ReturnKind,
}

pub(in crate::compiler::declarations) fn compile_function_declaration(
    heap: &Heap,
    scope: &Scope<'_>,
    function: &Function<'_>,
    path: &str,
    source_text: &str,
    unit: &mut CompiledUnit,
) -> Result<CompiledFunction, CompileError> {
    rules::check_free_function_parameters(&function.parameter_list)?;
    let name = scope.resolver.qualify(function.name.value);
    let attributes = compile_attributes(
        heap,
        scope,
        function.attribute_lists,
        path,
        source_text,
        &mut DeclarationContext::for_unit(unit),
    )?;

    let parameters = compile_parameters(
        heap,
        scope,
        &function.parameter_list,
        path,
        source_text,
        &mut DeclarationContext::for_unit(unit),
    )?;

    let lowered_return = lower_function_return(heap, scope, &unit.type_aliases, function)?;

    let signature = render_signature(
        heap,
        scope,
        &unit.type_aliases,
        function.type_parameters.as_ref(),
        &function.parameter_list,
        function
            .return_type
            .as_ref()
            .map(|annotation| annotation.r#type),
    )?;

    let chunk = if has_attribute(scope, function.attribute_lists, STUB_ATTRIBUTE_NAME) {
        inert_declaration_chunk(function.body.span())
    } else {
        compile_body(
            scope,
            path,
            source_text,
            unit,
            &function.parameter_list,
            &function.body,
            BodyShape {
                is_instance_method: false,
                return_kind: lowered_return.kind,
                promote_parameters: false,
                trusted_returns: scope.trusted_returns,
            },
        )?
    };

    let type_parameters = compile_type_parameters(
        heap,
        scope,
        &unit.type_aliases,
        function.type_parameters.as_ref(),
    )?;
    for parameter in &parameters {
        if let Some(descriptor) = &parameter.declared_type {
            validate_variance_use(
                descriptor,
                -1,
                &type_parameters,
                scope.generics,
                function.span(),
            )?;
        }
    }
    if let Some(descriptor) = &lowered_return.descriptor {
        validate_variance_use(
            descriptor,
            1,
            &type_parameters,
            scope.generics,
            function.span(),
        )?;
    }

    Ok(CompiledFunction {
        name: heap.intern(name.as_bytes()),
        span: function.span(),
        signature: heap.intern(signature.as_bytes()),
        type_parameters,
        parameters,
        return_type: lowered_return.descriptor,
        attributes,
        captures_this: false,
        capture_names: Vec::new(),
        is_short_closure: false,
        capture_types: Vec::new(),
        chunk,
    })
}

fn lower_function_return(
    heap: &Heap,
    scope: &Scope<'_>,
    aliases: &[CompiledTypeAlias],
    function: &Function<'_>,
) -> Result<LoweredReturn, CompileError> {
    let type_scope = TypeScope {
        heap,
        resolver: scope.resolver,
        class: scope.class,
        aliases,
        binders: &scope.binders,
        forbidden_binders: &scope.forbidden_binders,
        generics: scope.generics,
    };
    let descriptor = function
        .return_type
        .as_ref()
        .map(|annotation| {
            types::lowering::validate_return_annotation(annotation.r#type)?;
            lower_type(&type_scope, annotation.r#type)
        })
        .transpose()?;
    let returns_void = matches!(
        function
            .return_type
            .as_ref()
            .map(|annotation| annotation.r#type.unparenthesized()),
        Some(Type::Void(_))
    );
    let returns_never = descriptor
        .as_ref()
        .is_some_and(|return_type| types::descriptor_is_never(return_type, aliases));
    Ok(LoweredReturn {
        descriptor,
        kind: ReturnKind::callable(returns_void, returns_never),
    })
}

pub(in crate::compiler::declarations) fn compile_body(
    scope: &Scope<'_>,
    path: &str,
    source_text: &str,
    unit: &mut CompiledUnit,
    parameter_list: &ParameterList<'_>,
    body: &Block<'_>,
    shape: BodyShape,
) -> Result<Chunk, CompileError> {
    let is_instance_method = shape.is_instance_method;
    let mut compiler = BodyCompiler::new(
        scope.heap,
        path,
        unit.path.as_bytes(),
        source_text,
        &mut unit.functions,
        &unit.type_aliases,
        shape,
    );

    if is_instance_method {
        compiler.declare_local("$this", true, body.left_brace)?;
    }

    for parameter in &parameter_list.parameters {
        compiler.declare_local(parameter.variable.name, true, body.left_brace)?;
    }

    compiler.declare_assigned_locals(body.statements, body.left_brace)?;
    let assigned = collect_assigned_in_statements(body.statements);
    compiler.prepare_trace_arguments(parameter_list, &assigned, body.left_brace)?;
    compiler.parameter_prologue(scope, parameter_list)?;
    compiler.statements(scope, body.statements)?;
    Ok(compiler.finish(body.right_brace))
}

pub(in crate::compiler) fn compile_parameters(
    heap: &Heap,
    scope: &Scope<'_>,
    parameter_list: &ParameterList<'_>,
    path: &str,
    source_text: &str,
    context: &mut DeclarationContext<'_>,
) -> Result<Vec<CompiledParameter>, CompileError> {
    rules::check_parameter_count(parameter_list)?;
    let mut parameters = Vec::new();
    let mut seen: Vec<&str> = Vec::new();
    for parameter in &parameter_list.parameters {
        if seen.contains(&parameter.variable.name) {
            return Err(CompileError::new(
                CompileErrorKind::DuplicateParameter,
                format!(
                    "the parameter `{}` is declared twice",
                    parameter.variable.name
                ),
                parameter.variable.span(),
            ));
        }

        seen.push(parameter.variable.name);
        if let Some(default) = &parameter.default {
            rules::check_parameter_default(default.value)?;
        }

        let sensitive = has_attribute(
            scope,
            parameter.attribute_lists,
            "Whim\\Marker\\SensitiveParameter",
        );

        let declared_type = match parameter.r#type {
            Some(annotation) => {
                types::lowering::reject_return_only_annotation(annotation, "parameter")?;
                let type_scope = TypeScope {
                    heap,
                    resolver: scope.resolver,
                    class: scope.class,
                    aliases: context.aliases,
                    binders: &scope.binders,
                    forbidden_binders: &scope.forbidden_binders,
                    generics: scope.generics,
                };
                Some(lower_type(&type_scope, annotation)?)
            }
            None => None,
        };

        let default = parameter
            .default
            .as_ref()
            .map(|default| {
                compile_initializer(heap, scope, default.value, path, source_text, context)
            })
            .transpose()?;

        parameters.push(CompiledParameter {
            name: heap.intern(
                parameter
                    .variable
                    .name
                    .strip_prefix('$')
                    .unwrap_or(parameter.variable.name)
                    .as_bytes(),
            ),
            span: parameter.span(),
            has_default: parameter.default.is_some(),
            default,
            declared_type,
            sensitive,
            attributes: compile_attributes(
                heap,
                scope,
                parameter.attribute_lists,
                path,
                source_text,
                context,
            )?,
        });
    }

    Ok(parameters)
}

pub(in crate::compiler) fn has_attribute(
    scope: &Scope<'_>,
    attribute_lists: &[AttributeList<'_>],
    expected: &str,
) -> bool {
    attribute_lists.iter().any(|list| {
        list.attributes
            .iter()
            .any(|attribute| scope.resolver.resolve_text(&attribute.name) == expected)
    })
}

/// A verified, unreachable body for a declaration supplied by the engine.
pub(in crate::compiler::declarations) fn inert_declaration_chunk(span: Span) -> Chunk {
    let mut chunk = Chunk::new();
    chunk.emit(Instruction::ReturnNull, span);
    chunk
}

pub(in crate::compiler) fn render_signature(
    heap: &Heap,
    scope: &Scope<'_>,
    aliases: &[CompiledTypeAlias],
    type_parameters: Option<&TypeParameterList<'_>>,
    parameter_list: &ParameterList<'_>,
    return_type: Option<&Type<'_>>,
) -> Result<String, CompileError> {
    let type_scope = TypeScope {
        heap,
        resolver: scope.resolver,
        class: scope.class,
        aliases,
        binders: &scope.binders,
        forbidden_binders: &scope.forbidden_binders,
        generics: scope.generics,
    };

    let binder_prefix = render_binder_prefix(&type_scope, type_parameters)?;
    let mut parts = Vec::new();
    for parameter in &parameter_list.parameters {
        let rendered = match parameter.r#type {
            Some(annotation) => {
                types::lowering::reject_return_only_annotation(annotation, "parameter")?;
                render_annotation(&type_scope, annotation)?
            }
            None => "mixed".to_string(),
        };

        let mut part = String::new();
        if parameter.default.is_some() {
            part.push('=');
        }

        part.push_str(&rendered);
        parts.push(part);
    }

    let rendered_return = match return_type {
        Some(annotation) => render_annotation(&type_scope, annotation)?,
        None => "mixed".to_string(),
    };

    Ok(format!(
        "fn{binder_prefix}({}): {rendered_return}",
        parts.join(", ")
    ))
}

fn render_binder_prefix(
    scope: &TypeScope<'_>,
    type_parameters: Option<&TypeParameterList<'_>>,
) -> Result<String, CompileError> {
    let Some(list) = type_parameters else {
        return Ok(String::new());
    };

    let mut parts = Vec::new();
    for parameter in list.parameters {
        parts.push(render_type_parameter(scope, parameter)?);
    }

    Ok(format!("<{}>", parts.join(", ")))
}

fn render_type_parameter(
    scope: &TypeScope<'_>,
    parameter: &TypeParameter<'_>,
) -> Result<String, CompileError> {
    let mut rendered = String::new();
    match &parameter.variance {
        Some(TypeVariance::In(_)) => rendered.push_str("in "),
        Some(TypeVariance::Out(_)) => rendered.push_str("out "),
        None => {}
    }

    rendered.push_str(parameter.name.value);
    if let Some(bound) = &parameter.bound {
        let mut bounds = Vec::new();
        for r#type in bound.types {
            bounds.push(render_type(scope, r#type)?);
        }
        rendered.push_str(": ");
        rendered.push_str(&bounds.join(" + "));
    }

    if let Some(default) = &parameter.default {
        rendered.push_str(" = ");
        rendered.push_str(&render_type(scope, default.r#type)?);
    }

    Ok(rendered)
}

pub(in crate::compiler) fn compile_initializer(
    heap: &Heap,
    scope: &Scope<'_>,
    value: &Expression<'_>,
    path: &str,
    source_text: &str,
    context: &mut DeclarationContext<'_>,
) -> Result<ConstantInitializer, CompileError> {
    if let Some(literal) = fold_literal(heap, scope, value)? {
        return Ok(ConstantInitializer::Literal(literal));
    }

    let mut compiler = BodyCompiler::new(
        heap,
        path,
        scope.runtime_path,
        source_text,
        &mut *context.synthesized,
        context.aliases,
        BodyShape {
            is_instance_method: false,
            return_kind: ReturnKind::Value,
            promote_parameters: false,
            trusted_returns: scope.trusted_returns,
        },
    );

    compiler.declare_referenced_locals(value)?;
    let register = compiler.expression(scope, value)?;
    Ok(ConstantInitializer::Thunk(Box::new(
        compiler.finish_initializer(register, value.span()),
    )))
}

fn fold_literal(
    heap: &Heap,
    scope: &Scope<'_>,
    value: &Expression<'_>,
) -> Result<Option<BytecodeLiteral>, CompileError> {
    match value.unparenthesized() {
        Expression::Literal(literal) => Ok(Some(match literal {
            Literal::Null(_) => BytecodeLiteral::Null,
            Literal::True(_) => BytecodeLiteral::Bool(true),
            Literal::False(_) => BytecodeLiteral::Bool(false),
            Literal::Integer(integer) => {
                BytecodeLiteral::Int(integer_gate(integer.value, false, integer.span)?)
            }
            Literal::Float(float) => BytecodeLiteral::Float(float.value),
            Literal::String(string) => BytecodeLiteral::String(heap.intern(string.value)),
        })),
        Expression::Construct(Construct::Embed(embed)) => Ok(Some(BytecodeLiteral::String(
            scope
                .embedded_files
                .load(heap, scope.runtime_path, &embed.path)?,
        ))),
        Expression::UnaryPrefix(unary)
            if matches!(unary.operator, UnaryPrefixOperator::Negation(_)) =>
        {
            match unary.operand.unparenthesized() {
                Expression::Literal(Literal::Integer(integer)) => Ok(Some(BytecodeLiteral::Int(
                    integer_gate(integer.value, true, unary.operand.span())?,
                ))),
                Expression::Literal(Literal::Float(float)) => {
                    Ok(Some(BytecodeLiteral::Float(-float.value)))
                }
                _ => Ok(None),
            }
        }
        _ => Ok(None),
    }
}

pub(in crate::compiler) fn compile_attributes(
    heap: &Heap,
    scope: &Scope<'_>,
    attribute_lists: &[AttributeList<'_>],
    path: &str,
    source_text: &str,
    context: &mut DeclarationContext<'_>,
) -> Result<Vec<CompiledAttribute>, CompileError> {
    let mut attributes = Vec::new();
    for list in attribute_lists {
        for attribute in &list.attributes {
            check_count(
                CompileErrorKind::TooManyAttributes,
                "one target may carry",
                "attributes",
                attributes.len() + 1,
                attribute.span(),
            )?;

            let mut arguments = Vec::new();
            let mut named_arguments = Vec::new();
            if let Some(argument_list) = &attribute.argument_list {
                for argument in &argument_list.arguments {
                    match argument {
                        Argument::Positional(positional) => {
                            rules::check_constant_expression(positional.value)?;
                            arguments.push(compile_initializer(
                                heap,
                                scope,
                                positional.value,
                                path,
                                source_text,
                                context,
                            )?);
                        }
                        Argument::Named(named) => {
                            rules::check_constant_expression(named.value)?;
                            named_arguments.push((
                                heap.intern(named.name.value.as_bytes()),
                                compile_initializer(
                                    heap,
                                    scope,
                                    named.value,
                                    path,
                                    source_text,
                                    context,
                                )?,
                            ));
                        }
                    }
                }
            }

            attributes.push(CompiledAttribute {
                class: scope.resolver.resolve(heap, &attribute.name),
                span: attribute.span(),
                arguments,
                named_arguments,
            });
        }
    }

    Ok(attributes)
}
