//! The language constructs.

use whim_syn::cst::construct::AssertConstruct;
use whim_syn::cst::construct::CloneConstruct;
use whim_syn::cst::construct::DropConstruct;
use whim_syn::cst::construct::EmbedConstruct;
use whim_syn::cst::construct::RemoveConstruct;
use whim_syn::cst::construct::SwapRemoveConstruct;

use crate::bytecode::instruction::operands::PropertyRemoveMode;
use crate::compiler::emit::BodyCompiler;
use crate::compiler::emit::BytecodeLiteral;
use crate::compiler::emit::ChainStep;
use crate::compiler::emit::CompileError;
use crate::compiler::emit::CompileErrorKind;
use crate::compiler::emit::Construct;
use crate::compiler::emit::Count;
use crate::compiler::emit::Expression;
use crate::compiler::emit::HasSpan;
use crate::compiler::emit::IcDescriptor;
use crate::compiler::emit::Instruction;
use crate::compiler::emit::Place;
use crate::compiler::emit::Register;
use crate::compiler::emit::Scope;
use crate::compiler::emit::Span;
use crate::compiler::emit::WriteTarget;
use crate::compiler::emit::binary_instruction;
use crate::compiler::emit::lower_checked_type;
use crate::compiler::emit::side_table_limit;
use crate::compiler::emit::written_value_gate;
use crate::unreachable_invariant;

/// The position as `i16`; the arity gate keeps it in range.
pub(in crate::compiler::emit) fn tuple_index(position: usize) -> i16 {
    let Ok(index) = i16::try_from(position) else {
        // SAFETY: the surrounding invariant makes this path unreachable.
        unsafe { unreachable_invariant("the arity gate bounds the position") }
    };
    index
}

enum UnaryConstruct {
    Length,
}

impl UnaryConstruct {
    const fn instruction(self, destination: Register, source: Register) -> Instruction {
        match self {
            Self::Length => Instruction::Length {
                destination,
                source,
            },
        }
    }
}

enum BinaryConstruct {
    Contains,
    ContainsKey,
}

impl BinaryConstruct {
    const fn instruction(
        self,
        destination: Register,
        left: Register,
        right: Register,
    ) -> Instruction {
        match self {
            Self::Contains => Instruction::Contains {
                destination,
                array: left,
                value: right,
            },
            Self::ContainsKey => Instruction::ContainsKey {
                destination,
                array: left,
                key: right,
            },
        }
    }
}

impl WriteTarget {
    const fn instruction(self, value_count: Count, first_value: Register) -> Instruction {
        match self {
            Self::Output => Instruction::Write {
                value_count,
                first_value,
            },
            Self::OutputLine => Instruction::WriteLine {
                value_count,
                first_value,
            },
            Self::Error => Instruction::WriteError {
                value_count,
                first_value,
            },
            Self::ErrorLine => Instruction::WriteErrorLine {
                value_count,
                first_value,
            },
            Self::Diagnostic => Instruction::Debug {
                value_count,
                first_value,
            },
        }
    }
}

impl BodyCompiler<'_, '_> {
    pub(in crate::compiler::emit) fn construct(
        &mut self,
        scope: &Scope<'_>,
        construct: &Construct<'_>,
    ) -> Result<Register, CompileError> {
        match construct {
            Construct::Length(length) => {
                self.unary_construct(scope, length.value, length.span(), UnaryConstruct::Length)
            }
            Construct::Contains(contains) => self.binary_construct(
                scope,
                contains.array,
                contains.value,
                contains.span(),
                BinaryConstruct::Contains,
            ),
            Construct::ContainsKey(contains) => self.binary_construct(
                scope,
                contains.array,
                contains.key,
                contains.span(),
                BinaryConstruct::ContainsKey,
            ),
            Construct::Remove(remove) => self.remove_construct(scope, remove),
            Construct::SwapRemove(remove) => self.swap_remove_construct(scope, remove),
            Construct::RemoveFirst(remove) => self.remove_end_construct(
                scope,
                remove.array,
                remove.span(),
                PropertyRemoveMode::First,
            ),
            Construct::RemoveLast(remove) => self.remove_end_construct(
                scope,
                remove.array,
                remove.span(),
                PropertyRemoveMode::Last,
            ),
            Construct::Clone(clone) => self.clone_construct(scope, clone),
            Construct::Assert(assert) => self.assert_construct(scope, assert),
            Construct::Exit(exit) => {
                let code = match exit.code {
                    Some(code) => self.expression(scope, code)?,
                    None => Register::NONE,
                };
                self.chunk.emit(Instruction::Exit { code }, exit.span());
                self.null_result(exit.span())
            }
            Construct::Panic(panic) => {
                let message = self.string_constant(panic.message.value, panic.span())?;
                self.chunk
                    .emit(Instruction::Panic { message }, panic.span());
                self.null_result(panic.span())
            }
            Construct::Write(write) => self.write_construct(
                scope,
                write.arguments.iter().map(|argument| argument.value),
                write.arguments.len(),
                write.span(),
                WriteTarget::Output,
            ),
            Construct::WriteLine(write) => self.write_construct(
                scope,
                write.arguments.iter().map(|argument| argument.value),
                write.arguments.len(),
                write.span(),
                WriteTarget::OutputLine,
            ),
            Construct::WriteError(write) => self.write_construct(
                scope,
                write.arguments.iter().map(|argument| argument.value),
                write.arguments.len(),
                write.span(),
                WriteTarget::Error,
            ),
            Construct::WriteErrorLine(write) => self.write_construct(
                scope,
                write.arguments.iter().map(|argument| argument.value),
                write.arguments.len(),
                write.span(),
                WriteTarget::ErrorLine,
            ),
            Construct::Debug(debug) => self.write_construct(
                scope,
                debug.arguments.iter().map(|argument| argument.value),
                debug.arguments.len(),
                debug.span(),
                WriteTarget::Diagnostic,
            ),
            Construct::Discard(discard) => {
                self.expression(scope, discard.value)?;
                self.null_result(discard.span())
            }
            Construct::Drop(drop) => self.drop_construct(drop),
            Construct::Require(require) => {
                self.require_construct(scope, require.value, require.span(), false)
            }
            Construct::RequireOnce(require) => {
                self.require_construct(scope, require.value, require.span(), true)
            }
            Construct::File(file) => self.file_construct(file.span()),
            Construct::Directory(directory) => self.directory_construct(directory.span()),
            Construct::Embed(embed) => self.embed_construct(scope, embed),
        }
    }

    fn unary_construct(
        &mut self,
        scope: &Scope<'_>,
        value: &Expression<'_>,
        span: Span,
        construct: UnaryConstruct,
    ) -> Result<Register, CompileError> {
        let source = self.expression(scope, value)?;
        let destination = self.allocate(span)?;
        self.chunk
            .emit(construct.instruction(destination, source), span);
        Ok(destination)
    }

    fn binary_construct(
        &mut self,
        scope: &Scope<'_>,
        left: &Expression<'_>,
        right: &Expression<'_>,
        span: Span,
        construct: BinaryConstruct,
    ) -> Result<Register, CompileError> {
        let left = self.expression(scope, left)?;
        let right = self.expression(scope, right)?;
        let destination = self.allocate(span)?;
        self.chunk
            .emit(construct.instruction(destination, left, right), span);
        Ok(destination)
    }

    fn indexed_remove_construct<'arena>(
        &mut self,
        scope: &Scope<'_>,
        value: &'arena Expression<'arena>,
        operand: &Expression<'_>,
        span: Span,
        mode: PropertyRemoveMode,
    ) -> Result<Register, CompileError> {
        let mut place = self.prepare_construct_place(scope, value)?;
        let operand = self.expression(scope, operand)?;
        if let Place::Property { object, cache } = &place {
            let destination = self.allocate(span)?;
            let implicit_operand = self.allocate(span)?;
            self.move_into(implicit_operand, operand, span);
            self.chunk.emit(
                Instruction::PropertyRemove {
                    object: *object,
                    destination,
                    cache: *cache,
                    mode,
                },
                span,
            );

            return Ok(destination);
        }

        self.materialize_place(&mut place, span)?;
        let container = self.read_place(&place, span)?;
        let destination = self.allocate(span)?;
        self.chunk.emit(
            match mode {
                PropertyRemoveMode::Key => Instruction::Remove {
                    destination,
                    container,
                    key: operand,
                },
                PropertyRemoveMode::Swap => Instruction::SwapRemove {
                    destination,
                    container,
                    index: operand,
                },
                // SAFETY: the surrounding invariant makes this path unreachable.
                PropertyRemoveMode::First | PropertyRemoveMode::Last => unsafe {
                    unreachable_invariant("indexed_remove_construct receives an end mode")
                },
            },
            span,
        );
        self.write_place(scope, &place, container, span)?;
        Ok(destination)
    }

    fn remove_construct(
        &mut self,
        scope: &Scope<'_>,
        remove: &RemoveConstruct<'_>,
    ) -> Result<Register, CompileError> {
        self.indexed_remove_construct(
            scope,
            remove.array,
            remove.key,
            remove.span(),
            PropertyRemoveMode::Key,
        )
    }

    fn swap_remove_construct(
        &mut self,
        scope: &Scope<'_>,
        remove: &SwapRemoveConstruct<'_>,
    ) -> Result<Register, CompileError> {
        self.indexed_remove_construct(
            scope,
            remove.vector,
            remove.index,
            remove.span(),
            PropertyRemoveMode::Swap,
        )
    }

    fn remove_end_construct<'arena>(
        &mut self,
        scope: &Scope<'_>,
        value: &'arena Expression<'arena>,
        span: Span,
        mode: PropertyRemoveMode,
    ) -> Result<Register, CompileError> {
        let mut place = self.prepare_construct_place(scope, value)?;
        if let Place::Property { object, cache } = &place {
            let destination = self.allocate(span)?;
            self.chunk.emit(
                Instruction::PropertyRemove {
                    object: *object,
                    destination,
                    cache: *cache,
                    mode,
                },
                span,
            );

            return Ok(destination);
        }

        self.materialize_place(&mut place, span)?;
        let container = self.read_place(&place, span)?;
        let destination = self.allocate(span)?;
        self.chunk.emit(
            match mode {
                PropertyRemoveMode::First => Instruction::RemoveFirst {
                    destination,
                    container,
                },
                PropertyRemoveMode::Last => Instruction::RemoveLast {
                    destination,
                    container,
                },
                // SAFETY: the surrounding invariant makes this path unreachable.
                PropertyRemoveMode::Key | PropertyRemoveMode::Swap => unsafe {
                    unreachable_invariant("remove_end_construct receives an end mode")
                },
            },
            span,
        );
        self.write_place(scope, &place, container, span)?;
        Ok(destination)
    }

    fn prepare_construct_place<'arena>(
        &mut self,
        scope: &Scope<'_>,
        value: &'arena Expression<'arena>,
    ) -> Result<Place<'arena>, CompileError> {
        let (root, indexes) = self.prepare_chain(scope, value)?;
        if indexes.is_empty() {
            return Ok(root);
        }

        Ok(Place::Chain {
            root: Box::new(root),
            levels: None,
            steps: indexes.into_iter().map(ChainStep::Index).collect(),
        })
    }

    fn clone_construct(
        &mut self,
        scope: &Scope<'_>,
        clone: &CloneConstruct<'_>,
    ) -> Result<Register, CompileError> {
        let source = self.expression(scope, clone.object)?;
        let destination = self.allocate(clone.span())?;
        self.chunk.emit(
            Instruction::CloneObject {
                destination,
                source,
            },
            clone.span(),
        );
        for field in clone.fields {
            let mark = self.registers.mark();
            let value = self.expression(scope, field.value)?;
            let cache = self.add_ic_descriptor(
                IcDescriptor::Member {
                    name: self.heap.intern(field.name.value.as_bytes()),
                    type_arguments: None,
                },
                clone.span(),
            )?;
            self.chunk.emit(
                Instruction::PropertyInitRaw {
                    object: destination,
                    value,
                    cache,
                },
                field.value.span(),
            );
            self.registers.release_to(mark);
        }
        Ok(destination)
    }

    fn assert_construct(
        &mut self,
        scope: &Scope<'_>,
        assert: &AssertConstruct<'_>,
    ) -> Result<Register, CompileError> {
        let mark = self.registers.mark();
        let (first_value, operand_count) = self.assertion_condition(scope, assert.condition)?;
        let message = match &assert.message {
            Some(message) => self.expression(scope, message.value)?,
            None => Register::NONE,
        };
        let condition_span = assert.condition.span();
        let start = condition_span.start.offset as usize;
        let end = condition_span.end.offset as usize;
        let source = self.source_text.get(start..end).unwrap_or("");
        let source = self.heap.intern(source.as_bytes());
        let text = self.add_constant(BytecodeLiteral::String(source), condition_span)?;
        self.chunk.emit(
            Instruction::Assert {
                operand_count,
                first_value,
                message,
                text,
            },
            assert.span(),
        );
        self.registers.release_to(mark);
        self.null_result(assert.span())
    }

    fn drop_construct(&mut self, drop: &DropConstruct<'_>) -> Result<Register, CompileError> {
        let destination = self.null_result(drop.span())?;
        self.clear_released_temporaries(drop.span());
        for variable in drop.variables {
            if variable.name == "$this" {
                return Err(CompileError::new(
                    CompileErrorKind::CannotDropThis,
                    "`$this` cannot be dropped",
                    variable.span(),
                ));
            }
            self.ensure_local_writable(variable.name, variable.span())?;
        }
        for variable in drop.variables {
            let target = self.local_register(variable.name, variable.span())?;
            if let Some(trace_argument) = self.trace_argument_for(target) {
                self.chunk.emit(
                    Instruction::Clear {
                        target: trace_argument,
                    },
                    variable.span(),
                );
            }
            let message = format!(
                "{} is not the last strong reference to its value",
                variable.name
            );
            let message = self.add_constant(
                BytecodeLiteral::String(self.heap.intern(message.as_bytes())),
                variable.span(),
            )?;
            self.chunk.emit(
                Instruction::CheckSoleReference {
                    source: target,
                    message,
                    chain_previous: false,
                },
                variable.span(),
            );
        }
        for variable in drop.variables {
            let target = self.local_register(variable.name, variable.span())?;
            self.chunk
                .emit(Instruction::Clear { target }, variable.span());
            self.mark_undefined(variable.name);
        }
        self.chunk.emit(Instruction::DrainFinalizers, drop.span());
        Ok(destination)
    }

    fn require_construct(
        &mut self,
        scope: &Scope<'_>,
        value: &Expression<'_>,
        span: Span,
        once: bool,
    ) -> Result<Register, CompileError> {
        let path = self.expression(scope, value)?;
        let destination = self.allocate(span)?;
        self.chunk.emit(
            Instruction::Require {
                once,
                destination,
                path,
            },
            span,
        );
        Ok(destination)
    }

    fn file_construct(&mut self, span: Span) -> Result<Register, CompileError> {
        let constant = self.string_constant(self.runtime_path, span)?;
        let destination = self.allocate(span)?;
        self.chunk.emit(
            Instruction::LoadConstant {
                destination,
                constant,
            },
            span,
        );
        Ok(destination)
    }

    fn embed_construct(
        &mut self,
        scope: &Scope<'_>,
        embed: &EmbedConstruct<'_>,
    ) -> Result<Register, CompileError> {
        let contents = scope
            .embedded_files
            .load(self.heap, self.runtime_path, &embed.path)?;
        let constant = self.add_constant(BytecodeLiteral::String(contents), embed.span())?;
        let destination = self.allocate(embed.span())?;
        self.chunk.emit(
            Instruction::LoadConstant {
                destination,
                constant,
            },
            embed.span(),
        );
        Ok(destination)
    }

    fn directory_construct(&mut self, span: Span) -> Result<Register, CompileError> {
        let separator = self
            .runtime_path
            .iter()
            .rposition(|byte| *byte == b'/' || *byte == b'\\');
        let parent: &[u8] = match separator {
            Some(0) => &self.runtime_path[..1],
            Some(position) => &self.runtime_path[..position],
            None => b".",
        };
        let constant = self.string_constant(parent, span)?;
        let destination = self.allocate(span)?;
        self.chunk.emit(
            Instruction::LoadConstant {
                destination,
                constant,
            },
            span,
        );
        Ok(destination)
    }

    fn null_result(&mut self, span: Span) -> Result<Register, CompileError> {
        let destination = self.allocate(span)?;
        self.chunk.emit(Instruction::LoadNull { destination }, span);
        Ok(destination)
    }

    /// Evaluates an assertion condition once into a diagnostic window: a
    /// comparison keeps both operands, a direct `is` keeps its subject,
    /// anything else keeps only the result.
    fn assertion_condition(
        &mut self,
        scope: &Scope<'_>,
        condition: &Expression<'_>,
    ) -> Result<(Register, Count), CompileError> {
        match condition.unparenthesized() {
            Expression::Binary(binary) if binary.operator.is_comparison() => {
                let first_value = self.allocate(condition.span())?;
                let left = self.allocate(binary.lhs.span())?;
                let right = self.allocate(binary.rhs.span())?;
                let temporaries = self.registers.mark();

                let evaluated = self.expression(scope, binary.lhs)?;
                self.move_into(left, evaluated, binary.lhs.span());
                self.registers.release_to(temporaries);

                let evaluated = self.expression(scope, binary.rhs)?;
                self.move_into(right, evaluated, binary.rhs.span());
                self.registers.release_to(temporaries);

                self.chunk.emit(
                    binary_instruction(binary.operator, first_value, left, right),
                    condition.span(),
                );
                Ok((first_value, Count::new(2)))
            }
            Expression::TypeOperation(operation) if operation.operator.is_check() => {
                let first_value = self.allocate(condition.span())?;
                let subject = self.allocate(operation.operand.span())?;
                let temporaries = self.registers.mark();

                let evaluated = self.expression(scope, operation.operand)?;
                self.move_into(subject, evaluated, operation.operand.span());
                self.registers.release_to(temporaries);

                let descriptor = lower_checked_type(&self.types(scope), operation.r#type)?;
                let descriptor = self
                    .chunk
                    .add_type_descriptor(descriptor)
                    .map_err(|full| side_table_limit(full, operation.span()))?;
                self.chunk.emit(
                    Instruction::Is {
                        destination: first_value,
                        source: subject,
                        descriptor,
                    },
                    condition.span(),
                );
                Ok((first_value, Count::new(1)))
            }
            _ => {
                let first_value = self.allocate(condition.span())?;
                let temporaries = self.registers.mark();
                let evaluated = self.expression(scope, condition)?;
                self.move_into(first_value, evaluated, condition.span());
                self.registers.release_to(temporaries);
                Ok((first_value, Count::new(0)))
            }
        }
    }

    fn write_construct<'expression>(
        &mut self,
        scope: &Scope<'_>,
        values: impl Iterator<Item = &'expression Expression<'expression>>,
        count: usize,
        span: Span,
        target: WriteTarget,
    ) -> Result<Register, CompileError> {
        let mark = self.registers.mark();
        let first = self.window(scope, values, count, span)?;
        let value_count = Count::new(written_value_gate(count, span)?);
        let instruction = target.instruction(value_count, first);
        self.chunk.emit(instruction, span);
        self.registers.release_to(mark);
        let destination = self.allocate(span)?;
        self.chunk.emit(Instruction::LoadNull { destination }, span);
        Ok(destination)
    }
}
