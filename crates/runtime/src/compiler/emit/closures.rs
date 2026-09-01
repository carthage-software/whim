//! Closures, short closures, and the bodies synthesized for them.

use whim_syn::cst::declaration::AttributeList;
use whim_syn::cst::function::Closure;
use whim_syn::cst::function::ParameterList;
use whim_syn::cst::function::ShortClosure;
use whim_syn::cst::function::ShortClosureBody;
use whim_syn::cst::r#type::Type;
use whim_syn::cst::r#type::TypeParameterList;

use crate::bytecode::chunk::Chunk;
use crate::bytecode::chunk::descriptors::TypeDescriptor;
use crate::bytecode::unit::CompiledAttribute;
use crate::bytecode::unit::CompiledFunction;
use crate::bytecode::unit::CompiledParameter;
use crate::bytecode::unit::CompiledTypeParameter;
use crate::compiler::declarations::class_likes::validate_variance_use;
use crate::compiler::declarations::functions::DeclarationContext;
use crate::compiler::declarations::functions::compile_attributes;
use crate::compiler::declarations::functions::compile_parameters;
use crate::compiler::declarations::functions::render_signature;
use crate::compiler::declarations::generics::binder_names;
use crate::compiler::declarations::generics::check_type_parameters;
use crate::compiler::declarations::generics::compile_type_parameters;
use crate::compiler::emit::Block;
use crate::compiler::emit::BodyCompiler;
use crate::compiler::emit::BodyShape;
use crate::compiler::emit::CompileError;
use crate::compiler::emit::CompileErrorKind;
use crate::compiler::emit::Count;
use crate::compiler::emit::Expression;
use crate::compiler::emit::HasSpan;
use crate::compiler::emit::Instruction;
use crate::compiler::emit::Register;
use crate::compiler::emit::ReturnKind;
use crate::compiler::emit::Scope;
use crate::compiler::emit::Span;
use crate::compiler::emit::analysis::collect_assigned_in_statements;
use crate::compiler::emit::analysis::collect_free_variables_in_expression;
use crate::compiler::emit::analysis::collect_free_variables_in_statements;
use crate::compiler::emit::analysis::collect_scoped_bindings_in_expression;
use crate::compiler::emit::capture_gate;
use crate::compiler::emit::check_sequence;
use crate::compiler::emit::collect_assigned_in_expression;
use crate::compiler::emit::line_and_column;
use crate::compiler::emit::references_this_in_block;
use crate::compiler::types::TypeScope;
use crate::compiler::types::descriptor_is_never;
use crate::compiler::types::lowering::lower_type;
use crate::compiler::types::lowering::validate_return_annotation;

#[derive(Clone, Copy)]
enum FunctionBodySource<'source, 'arena> {
    Block(&'source Block<'arena>),
    Expression(&'source Expression<'arena>),
}

#[derive(Clone, Copy)]
struct SynthesizedFunctionSource<'source, 'arena, 'captures> {
    span: Span,
    attribute_lists: &'source [AttributeList<'arena>],
    type_parameters: Option<&'source TypeParameterList<'arena>>,
    parameter_list: &'source ParameterList<'arena>,
    return_type: Option<&'source Type<'arena>>,
    body: FunctionBodySource<'source, 'arena>,
    captures: &'captures [String],
    is_short: bool,
}

impl SynthesizedFunctionSource<'_, '_, '_> {
    fn captures_this(self) -> bool {
        self.captures
            .first()
            .is_some_and(|capture| capture == "$this")
    }
}

struct SynthesizedFunctionMetadata {
    signature: String,
    type_parameters: Vec<CompiledTypeParameter>,
    parameters: Vec<CompiledParameter>,
    return_type: Option<TypeDescriptor>,
    attributes: Vec<CompiledAttribute>,
    return_kind: ReturnKind,
}

impl BodyCompiler<'_, '_> {
    pub(in crate::compiler) fn closure(
        &mut self,
        scope: &Scope<'_>,
        closure: &Closure<'_>,
    ) -> Result<Register, CompileError> {
        if let Some(list) = &closure.type_parameters {
            check_type_parameters(self.heap, scope, self.aliases, list)?;
        }

        let mut captures: Vec<String> = Vec::new();
        if let Some(use_clause) = &closure.use_clause {
            check_sequence(
                CompileErrorKind::TooManyCaptures,
                "a `use` clause may capture",
                "variables",
                use_clause.variables,
            )?;

            for variable in use_clause.variables {
                if captures.iter().any(|capture| capture == variable.name) {
                    return Err(CompileError::new(
                        CompileErrorKind::DuplicateCapture,
                        format!("the variable {} is captured twice", variable.name),
                        variable.span(),
                    ));
                }
                if closure
                    .parameter_list
                    .parameters
                    .iter()
                    .any(|parameter| parameter.variable.name == variable.name)
                {
                    return Err(CompileError::new(
                        CompileErrorKind::DuplicateCapture,
                        format!(
                            "the variable {} is both captured and declared as a parameter",
                            variable.name
                        ),
                        variable.span(),
                    ));
                }

                captures.push(variable.name.to_string());
            }
        }

        if self.shape.is_instance_method && references_this_in_block(&closure.body) {
            captures.insert(0, "$this".to_string());
        }

        self.synthesize_function(
            scope,
            SynthesizedFunctionSource {
                span: closure.span(),
                attribute_lists: closure.attribute_lists,
                type_parameters: closure.type_parameters.as_ref(),
                parameter_list: &closure.parameter_list,
                return_type: closure
                    .return_type
                    .as_ref()
                    .map(|annotation| annotation.r#type),
                body: FunctionBodySource::Block(&closure.body),
                captures: &captures,
                is_short: false,
            },
        )
    }

    pub(in crate::compiler) fn short_closure(
        &mut self,
        scope: &Scope<'_>,
        closure: &ShortClosure<'_>,
    ) -> Result<Register, CompileError> {
        if let Some(list) = &closure.type_parameters {
            check_type_parameters(self.heap, scope, self.aliases, list)?;
        }

        if matches!(closure.body, ShortClosureBody::Expression { .. })
            && let Some(return_type) = &closure.return_type
            && matches!(return_type.r#type.unparenthesized(), Type::Void(_))
        {
            return Err(CompileError::new(
                CompileErrorKind::VoidExpressionShortClosure,
                "an expression-bodied short closure always returns its expression's value and cannot declare `void`",
                return_type.r#type.span(),
            ));
        }

        let (referenced, body) = match &closure.body {
            ShortClosureBody::Expression { expression, .. } => (
                collect_free_variables_in_expression(expression),
                FunctionBodySource::Expression(expression),
            ),
            ShortClosureBody::Block(block) => (
                collect_free_variables_in_statements(block.statements),
                FunctionBodySource::Block(block),
            ),
        };
        let parameter_names: Vec<&str> = closure
            .parameter_list
            .parameters
            .iter()
            .map(|parameter| parameter.variable.name)
            .collect();

        let mut captures: Vec<String> = Vec::new();
        for name in referenced {
            if name == "$this" {
                if self.shape.is_instance_method && !captures.iter().any(|c| c == "$this") {
                    captures.insert(0, "$this".to_string());
                }
                continue;
            }

            if parameter_names.contains(&name.as_str()) {
                continue;
            }

            if self.local_position(&name).is_some() && !captures.contains(&name) {
                captures.push(name);
            }
        }

        self.synthesize_function(
            scope,
            SynthesizedFunctionSource {
                span: closure.span(),
                attribute_lists: closure.attribute_lists,
                type_parameters: closure.type_parameters.as_ref(),
                parameter_list: &closure.parameter_list,
                return_type: closure
                    .return_type
                    .as_ref()
                    .map(|annotation| annotation.r#type),
                body,
                captures: &captures,
                is_short: true,
            },
        )
    }

    fn compile_synthesized_metadata(
        &mut self,
        function_scope: &Scope<'_>,
        source: &SynthesizedFunctionSource<'_, '_, '_>,
    ) -> Result<SynthesizedFunctionMetadata, CompileError> {
        let (parameters, attributes) = {
            let mut context = DeclarationContext::new(self.aliases, &mut *self.synthesized);
            let parameters = compile_parameters(
                self.heap,
                function_scope,
                source.parameter_list,
                self.path,
                self.source_text,
                &mut context,
            )?;
            let attributes = compile_attributes(
                self.heap,
                function_scope,
                source.attribute_lists,
                self.path,
                self.source_text,
                &mut context,
            )?;
            (parameters, attributes)
        };

        let signature = render_signature(
            self.heap,
            function_scope,
            self.aliases,
            source.type_parameters,
            source.parameter_list,
            source.return_type,
        )?;
        let returns_void = matches!(
            source.return_type.map(Type::unparenthesized),
            Some(Type::Void(_))
        );
        let type_scope = TypeScope {
            heap: self.heap,
            resolver: function_scope.resolver,
            class: function_scope.class,
            aliases: self.aliases,
            binders: &function_scope.binders,
            forbidden_binders: &function_scope.forbidden_binders,
            generics: function_scope.generics,
        };
        let return_type = source
            .return_type
            .map(|annotation| {
                validate_return_annotation(annotation)?;
                lower_type(&type_scope, annotation)
            })
            .transpose()?;
        let returns_never = return_type
            .as_ref()
            .is_some_and(|descriptor| descriptor_is_never(descriptor, self.aliases));
        let type_parameters = compile_type_parameters(
            self.heap,
            function_scope,
            self.aliases,
            source.type_parameters,
        )?;
        for parameter in &parameters {
            if let Some(descriptor) = &parameter.declared_type {
                validate_variance_use(
                    descriptor,
                    -1,
                    &type_parameters,
                    function_scope.generics,
                    source.span,
                )?;
            }
        }
        if let Some(descriptor) = &return_type {
            validate_variance_use(
                descriptor,
                1,
                &type_parameters,
                function_scope.generics,
                source.span,
            )?;
        }

        Ok(SynthesizedFunctionMetadata {
            signature,
            type_parameters,
            parameters,
            return_type,
            attributes,
            return_kind: ReturnKind::callable(returns_void, returns_never),
        })
    }

    fn compile_synthesized_body(
        &mut self,
        function_scope: &Scope<'_>,
        source: &SynthesizedFunctionSource<'_, '_, '_>,
        return_kind: ReturnKind,
    ) -> Result<Chunk, CompileError> {
        let captured_this = source.captures_this();
        let final_captures: Vec<(String, Span)> = source
            .captures
            .iter()
            .filter_map(|capture| {
                self.final_local_span(capture)
                    .map(|span| (capture.clone(), span))
            })
            .collect();
        let mut inner = BodyCompiler::new(
            self.heap,
            self.path,
            self.runtime_path,
            self.source_text,
            self.synthesized,
            self.aliases,
            BodyShape {
                is_instance_method: captured_this,
                return_kind,
                promote_parameters: false,
                trusted_returns: function_scope.trusted_returns,
            },
        );
        if captured_this {
            inner.declare_local("$this", true, source.span)?;
        }
        for parameter in &source.parameter_list.parameters {
            inner.declare_local(parameter.variable.name, true, source.span)?;
        }
        for capture in source.captures {
            if capture != "$this" {
                inner.declare_local(capture, true, source.span)?;
            }
        }
        for (capture, declaration_span) in final_captures {
            inner.mark_local_final(&capture, declaration_span);
        }
        Ok(match source.body {
            FunctionBodySource::Block(block) => {
                inner.declare_assigned_locals(block.statements, block.left_brace)?;
                let assigned = collect_assigned_in_statements(block.statements);
                inner.prepare_trace_arguments(
                    source.parameter_list,
                    &assigned,
                    block.left_brace,
                )?;
                inner.parameter_prologue(function_scope, source.parameter_list)?;
                inner.statements(function_scope, block.statements)?;
                inner.finish(block.right_brace)
            }
            FunctionBodySource::Expression(expression) => {
                let names = collect_assigned_in_expression(expression);
                for name in &names {
                    inner.declare_local(name, false, expression.span())?;
                }
                let mut bindings = Vec::new();
                collect_scoped_bindings_in_expression(expression, &mut bindings);
                inner.declare_scoped_bindings(bindings)?;
                inner.prepare_trace_arguments(source.parameter_list, &names, expression.span())?;
                inner.parameter_prologue(function_scope, source.parameter_list)?;
                let register = inner.expression(function_scope, expression)?;
                let instruction = inner.return_instruction(register);
                inner.chunk.emit(instruction, expression.span());
                inner.finish(expression.span())
            }
        })
    }

    fn emit_closure(
        &mut self,
        name: &str,
        captures: &[String],
        span: Span,
    ) -> Result<Register, CompileError> {
        let destination = self.allocate(span)?;
        let mark = self.registers.mark();
        let capture_count = capture_gate(captures.len(), span)?;
        let mut slots = Vec::new();
        for _ in 0..captures.len() {
            slots.push(self.allocate(span)?);
        }

        for (slot, capture) in slots.iter().zip(captures) {
            let register = if capture == "$this" {
                Register::new(0)
            } else {
                self.read_local(capture, span)?
            };
            self.move_into(*slot, register, span);
        }
        let first_capture = slots
            .first()
            .copied()
            .unwrap_or_else(|| Register::new(self.registers.mark()));
        let prototype = self.string_constant(name.as_bytes(), span)?;
        self.chunk.emit(
            Instruction::MakeClosure {
                capture_count: Count::new(capture_count),
                destination,
                prototype,
                first_capture,
            },
            span,
        );

        self.registers.release_to(mark);
        Ok(destination)
    }

    fn synthesize_function(
        &mut self,
        scope: &Scope<'_>,
        source: SynthesizedFunctionSource<'_, '_, '_>,
    ) -> Result<Register, CompileError> {
        let (line, column) = line_and_column(scope.line_starts, source.span.start.offset);
        let name = format!("{{closure:{line}:{column}}}");
        let mut binders = scope.binders.clone();
        binders.extend(binder_names(source.type_parameters));
        let function_scope = Scope {
            heap: scope.heap,
            runtime_path: scope.runtime_path,
            line_starts: scope.line_starts,
            resolver: scope.resolver,
            class: scope.class,
            binders,
            forbidden_binders: scope.forbidden_binders.clone(),
            generics: scope.generics,
            embedded_files: scope.embedded_files,
            trusted_returns: scope.trusted_returns,
        };

        let metadata = self.compile_synthesized_metadata(&function_scope, &source)?;
        let chunk =
            self.compile_synthesized_body(&function_scope, &source, metadata.return_kind)?;

        self.synthesized.push(CompiledFunction {
            name: self.heap.intern(name.as_bytes()),
            span: source.span,
            signature: self.heap.intern(metadata.signature.as_bytes()),
            type_parameters: metadata.type_parameters,
            parameters: metadata.parameters,
            return_type: metadata.return_type,
            attributes: metadata.attributes,
            captures_this: source.captures_this(),
            capture_names: source
                .captures
                .iter()
                .map(|capture| self.heap.intern(capture.as_bytes()))
                .collect(),
            is_short_closure: source.is_short,
            capture_types: Vec::new(),
            chunk,
        });

        self.emit_closure(&name, source.captures, source.span)
    }
}
