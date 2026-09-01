//! Operators: binary spines, short-circuit shapes, unary operations, and
//! increment and decrement steps.

use whim_syn::cst::access::PropertyAccess;
use whim_syn::cst::access::StaticPropertyAccess;
use whim_syn::cst::array::ArrayAccess;
use whim_syn::cst::operation::AssignmentOperator;
use whim_syn::cst::operation::Binary;
use whim_syn::cst::operation::BinaryOperator;
use whim_syn::cst::operation::UnaryPostfix;
use whim_syn::cst::operation::UnaryPostfixOperator;
use whim_syn::cst::operation::UnaryPrefix;
use whim_syn::cst::operation::UnaryPrefixOperator;

use crate::bytecode::instruction::operands::PropertyIndexUpdateMode;
use crate::compiler::emit::expressions::Access;
use crate::compiler::emit::expressions::BodyCompiler;
use crate::compiler::emit::expressions::ChainStep;
use crate::compiler::emit::expressions::CompileError;
use crate::compiler::emit::expressions::CompileErrorKind;
use crate::compiler::emit::expressions::Count;
use crate::compiler::emit::expressions::Expression;
use crate::compiler::emit::expressions::HasSpan;
use crate::compiler::emit::expressions::IcDescriptor;
use crate::compiler::emit::expressions::ImmediateInt;
use crate::compiler::emit::expressions::Instruction;
use crate::compiler::emit::expressions::JumpOffset;
use crate::compiler::emit::expressions::Literal;
use crate::compiler::emit::expressions::Place;
use crate::compiler::emit::expressions::Register;
use crate::compiler::emit::expressions::Scope;
use crate::compiler::emit::expressions::Span;
use crate::compiler::emit::expressions::integer_gate;
use crate::unwrap_option_invariant;

/// Which value an increment or decrement yields.
#[derive(Clone, Copy)]
enum StepResult {
    /// The value before the step (postfix).
    Old,
    /// The value after the step (prefix).
    New,
}

/// When a short-circuit jump skips the right operand.
#[derive(Clone, Copy)]
pub(in crate::compiler::emit) enum ShortCircuit {
    And,
    Or,
    Coalesce,
}

pub(in crate::compiler::emit) const fn short_circuit_of(
    operator: BinaryOperator,
) -> Option<ShortCircuit> {
    match operator {
        BinaryOperator::And(_) => Some(ShortCircuit::And),
        BinaryOperator::Or(_) => Some(ShortCircuit::Or),
        BinaryOperator::NullCoalesce(_) => Some(ShortCircuit::Coalesce),
        _ => None,
    }
}

pub(in crate::compiler::emit) const fn short_circuit_jump(
    kind: ShortCircuit,
    condition: Register,
) -> Instruction {
    match kind {
        ShortCircuit::And => Instruction::JumpIfFalse {
            condition,
            offset: JumpOffset::new(0),
        },
        ShortCircuit::Or => Instruction::JumpIfTrue {
            condition,
            offset: JumpOffset::new(0),
        },
        ShortCircuit::Coalesce => Instruction::JumpIfNotNull {
            subject: condition,
            offset: JumpOffset::new(0),
        },
    }
}

/// The instruction of a value-producing binary operator.
#[expect(
    clippy::too_many_lines,
    reason = "one exhaustive mapping keeps every binary operator visible"
)]
pub(in crate::compiler::emit) fn binary_instruction(
    operator: BinaryOperator,
    destination: Register,
    left: Register,
    right: Register,
) -> Instruction {
    match operator {
        BinaryOperator::Addition(_) => Instruction::Add {
            destination,
            left,
            right,
        },
        BinaryOperator::Subtraction(_) => Instruction::Subtract {
            destination,
            left,
            right,
        },
        BinaryOperator::Multiplication(_) => Instruction::Multiply {
            destination,
            left,
            right,
        },
        BinaryOperator::Division(_) => Instruction::Divide {
            destination,
            left,
            right,
        },
        BinaryOperator::Modulo(_) => Instruction::Modulo {
            destination,
            left,
            right,
        },
        BinaryOperator::Exponentiation(_) => Instruction::Power {
            destination,
            left,
            right,
        },
        BinaryOperator::BitwiseAnd(_) => Instruction::BitwiseAnd {
            destination,
            left,
            right,
        },
        BinaryOperator::BitwiseOr(_) => Instruction::BitwiseOr {
            destination,
            left,
            right,
        },
        BinaryOperator::BitwiseXor(_) => Instruction::BitwiseXor {
            destination,
            left,
            right,
        },
        BinaryOperator::LeftShift(_) => Instruction::ShiftLeft {
            destination,
            left,
            right,
        },
        BinaryOperator::RightShift(_) => Instruction::ShiftRight {
            destination,
            left,
            right,
        },
        BinaryOperator::Equal(_) => Instruction::Equal {
            destination,
            left,
            right,
        },
        BinaryOperator::NotEqual(_) => Instruction::NotEqual {
            destination,
            left,
            right,
        },
        BinaryOperator::LessThan(_) => Instruction::LessThan {
            destination,
            left,
            right,
        },
        BinaryOperator::LessThanOrEqual(_) => Instruction::LessThanOrEqual {
            destination,
            left,
            right,
        },
        BinaryOperator::GreaterThan(_) => Instruction::GreaterThan {
            destination,
            left,
            right,
        },
        BinaryOperator::GreaterThanOrEqual(_) => Instruction::GreaterThanOrEqual {
            destination,
            left,
            right,
        },
        BinaryOperator::Spaceship(_) => Instruction::Compare {
            destination,
            left,
            right,
        },
        BinaryOperator::StringConcat(_) => Instruction::Concatenate {
            destination,
            left,
            right,
        },
        BinaryOperator::And(_)
        | BinaryOperator::Or(_)
        | BinaryOperator::NullCoalesce(_)
        | BinaryOperator::Pipe(_) => {
            unreachable!("operators requiring specialized lowering are handled separately")
        }
    }
}

/// The instruction of a compound-assignment operator's operation.
pub(in crate::compiler::emit) fn compound_instruction(
    operator: AssignmentOperator,
    destination: Register,
    left: Register,
    right: Register,
) -> Instruction {
    match operator {
        AssignmentOperator::Addition(_) => Instruction::Add {
            destination,
            left,
            right,
        },
        AssignmentOperator::Subtraction(_) => Instruction::Subtract {
            destination,
            left,
            right,
        },
        AssignmentOperator::Multiplication(_) => Instruction::Multiply {
            destination,
            left,
            right,
        },
        AssignmentOperator::Division(_) => Instruction::Divide {
            destination,
            left,
            right,
        },
        AssignmentOperator::Modulo(_) => Instruction::Modulo {
            destination,
            left,
            right,
        },
        AssignmentOperator::Exponentiation(_) => Instruction::Power {
            destination,
            left,
            right,
        },
        AssignmentOperator::Concat(_) => Instruction::Concatenate {
            destination,
            left,
            right,
        },
        AssignmentOperator::BitwiseAnd(_) => Instruction::BitwiseAnd {
            destination,
            left,
            right,
        },
        AssignmentOperator::BitwiseOr(_) => Instruction::BitwiseOr {
            destination,
            left,
            right,
        },
        AssignmentOperator::BitwiseXor(_) => Instruction::BitwiseXor {
            destination,
            left,
            right,
        },
        AssignmentOperator::LeftShift(_) => Instruction::ShiftLeft {
            destination,
            left,
            right,
        },
        AssignmentOperator::RightShift(_) => Instruction::ShiftRight {
            destination,
            left,
            right,
        },
        AssignmentOperator::Assign(_)
        | AssignmentOperator::Coalesce(_)
        | AssignmentOperator::LogicalAnd(_)
        | AssignmentOperator::LogicalOr(_) => {
            unreachable!("plain and short-circuit assignments are lowered separately")
        }
    }
}

const fn step_instruction(destination: Register, source: Register, step: i16) -> Instruction {
    if step >= 0 {
        Instruction::AddImmediate {
            destination,
            source,
            immediate: ImmediateInt::new(step),
        }
    } else {
        Instruction::SubtractImmediate {
            destination,
            source,
            immediate: ImmediateInt::new(-step),
        }
    }
}

#[derive(Clone, Copy)]
struct ImmediateStep {
    subtract: bool,
    immediate: ImmediateInt,
}

impl ImmediateStep {
    const fn instruction(self, destination: Register, source: Register) -> Instruction {
        if self.subtract {
            Instruction::SubtractImmediate {
                destination,
                source,
                immediate: self.immediate,
            }
        } else {
            Instruction::AddImmediate {
                destination,
                source,
                immediate: self.immediate,
            }
        }
    }
}

fn immediate_step(
    operator: BinaryOperator,
    expression: &Expression<'_>,
) -> Result<Option<ImmediateStep>, CompileError> {
    let value = match expression.unparenthesized() {
        Expression::Literal(Literal::Integer(integer)) => {
            integer_gate(integer.value, false, integer.span)?
        }
        Expression::UnaryPrefix(expression)
            if matches!(expression.operator, UnaryPrefixOperator::Negation(_)) =>
        {
            let Expression::Literal(Literal::Integer(integer)) = expression.operand else {
                return Ok(None);
            };
            integer_gate(integer.value, true, expression.span())?
        }
        _ => return Ok(None),
    };
    let Ok(value) = i16::try_from(value) else {
        return Ok(None);
    };
    let (subtract, magnitude) = match operator {
        BinaryOperator::Addition(_) if value >= 0 => (false, value),
        BinaryOperator::Addition(_) => {
            let Some(magnitude) = value.checked_neg() else {
                return Ok(None);
            };
            (true, magnitude)
        }
        BinaryOperator::Subtraction(_) if value >= 0 => (true, value),
        BinaryOperator::Subtraction(_) => {
            let Some(magnitude) = value.checked_neg() else {
                return Ok(None);
            };
            (false, magnitude)
        }
        _ => return Ok(None),
    };
    Ok(Some(ImmediateStep {
        subtract,
        immediate: ImmediateInt::new(magnitude),
    }))
}

impl BodyCompiler<'_, '_> {
    /// Compiles a binary operation, and the whole left-associative spine it
    /// sits at the top of, without recursing once per link.
    pub(in crate::compiler::emit) fn binary(
        &mut self,
        scope: &Scope<'_>,
        binary: &Binary<'_>,
    ) -> Result<Register, CompileError> {
        let mut spine = Vec::new();
        let mut link = binary;
        loop {
            spine.push(link);
            match link.lhs {
                Expression::Binary(inner) => link = inner,
                _ => break,
            }
        }

        // SAFETY: the loop pushes at least the initial node, so the spine is non-empty.
        let foot =
            *unsafe { unwrap_option_invariant(spine.last(), "the spine holds at least this node") };
        let start = foot.lhs.leftmost_span();

        let mut results = Vec::with_capacity(spine.len());
        for link in &spine {
            results.push(match short_circuit_of(link.operator) {
                Some(kind) => Some((kind, self.allocate(start.join(link.rhs.span()))?)),
                None => None,
            });
        }

        let mut left_span = foot.lhs.span();
        let mut accumulator = self.expression(scope, foot.lhs)?;

        for (link, result) in spine.into_iter().zip(results).rev() {
            let span = start.join(link.rhs.span());
            accumulator = match result {
                Some((kind, result)) => {
                    self.move_into(result, accumulator, left_span);
                    let skip = self
                        .chunk
                        .emit(short_circuit_jump(kind, result), link.operator.span());
                    let saved = self.save_defined();
                    let right = self.expression(scope, link.rhs)?;
                    self.move_into(result, right, link.rhs.span());
                    self.restore_defined(saved);
                    let after = self.code_position();
                    self.chunk.patch_jump(skip, after);
                    result
                }
                None => {
                    if link.operator.is_pipe() {
                        let callee = self.expression(scope, link.rhs)?;
                        let destination = self.allocate(span)?;
                        let mark = self.registers.mark();
                        let argument = self.allocate(left_span)?;
                        self.move_into(argument, accumulator, left_span);
                        self.chunk.emit(
                            Instruction::CallValue {
                                argument_count: Count::new(1),
                                destination,
                                callee,
                                first_argument: argument,
                            },
                            span,
                        );
                        self.registers.release_to(mark);
                        destination
                    } else if let Some(step) = immediate_step(link.operator, link.rhs)? {
                        let destination = self.allocate(span)?;
                        self.chunk
                            .emit(step.instruction(destination, accumulator), span);
                        destination
                    } else {
                        let right = self.expression(scope, link.rhs)?;
                        let destination = self.allocate(span)?;
                        let instruction =
                            binary_instruction(link.operator, destination, accumulator, right);
                        self.chunk.emit(instruction, span);
                        destination
                    }
                }
            };

            left_span = span;
        }

        Ok(accumulator)
    }

    pub(in crate::compiler::emit::expressions) fn unary_prefix(
        &mut self,
        scope: &Scope<'_>,
        unary: &UnaryPrefix<'_>,
    ) -> Result<Register, CompileError> {
        match unary.operator {
            UnaryPrefixOperator::Negation(span) => {
                if let Expression::Literal(Literal::Integer(integer)) =
                    unary.operand.unparenthesized()
                {
                    let value = integer_gate(integer.value, true, unary.span())?;
                    let destination = self.allocate(unary.span())?;
                    self.load_integer(destination, value, span.join(integer.span))?;
                    return Ok(destination);
                }

                let source = self.expression(scope, unary.operand)?;
                let destination = self.allocate(unary.span())?;
                self.chunk.emit(
                    Instruction::Negate {
                        destination,
                        source,
                    },
                    unary.span(),
                );

                Ok(destination)
            }
            UnaryPrefixOperator::Plus(_) => {
                let source = self.expression(scope, unary.operand)?;
                let destination = self.allocate(unary.span())?;
                self.chunk.emit(
                    Instruction::UnaryPlus {
                        destination,
                        source,
                    },
                    unary.span(),
                );

                Ok(destination)
            }
            UnaryPrefixOperator::Not(_) => {
                let source = self.expression(scope, unary.operand)?;
                let destination = self.allocate(unary.span())?;
                self.chunk.emit(
                    Instruction::Not {
                        destination,
                        source,
                    },
                    unary.span(),
                );

                Ok(destination)
            }
            UnaryPrefixOperator::BitwiseNot(_) => {
                let source = self.expression(scope, unary.operand)?;
                let destination = self.allocate(unary.span())?;
                self.chunk.emit(
                    Instruction::BitwiseNot {
                        destination,
                        source,
                    },
                    unary.span(),
                );

                Ok(destination)
            }
            UnaryPrefixOperator::PreIncrement(_) => {
                self.step_target(scope, unary.operand, 1, unary.span(), StepResult::New)
            }
            UnaryPrefixOperator::PreDecrement(_) => {
                self.step_target(scope, unary.operand, -1, unary.span(), StepResult::New)
            }
        }
    }

    pub(in crate::compiler::emit::expressions) fn unary_prefix_discarded(
        &mut self,
        scope: &Scope<'_>,
        unary: &UnaryPrefix<'_>,
    ) -> Result<(), CompileError> {
        match unary.operator {
            UnaryPrefixOperator::PreIncrement(_) => {
                self.step_target_discarded(scope, unary.operand, 1, unary.span())
            }
            UnaryPrefixOperator::PreDecrement(_) => {
                self.step_target_discarded(scope, unary.operand, -1, unary.span())
            }
            _ => {
                self.unary_prefix(scope, unary)?;
                Ok(())
            }
        }
    }

    pub(in crate::compiler::emit::expressions) fn unary_postfix(
        &mut self,
        scope: &Scope<'_>,
        unary: &UnaryPostfix<'_>,
    ) -> Result<Register, CompileError> {
        let step = match unary.operator {
            UnaryPostfixOperator::PostIncrement(_) => 1,
            UnaryPostfixOperator::PostDecrement(_) => -1,
        };
        self.step_target(scope, unary.operand, step, unary.span(), StepResult::Old)
    }

    pub(in crate::compiler::emit::expressions) fn unary_postfix_discarded(
        &mut self,
        scope: &Scope<'_>,
        unary: &UnaryPostfix<'_>,
    ) -> Result<(), CompileError> {
        let step = match unary.operator {
            UnaryPostfixOperator::PostIncrement(_) => 1,
            UnaryPostfixOperator::PostDecrement(_) => -1,
        };
        self.step_target_discarded(scope, unary.operand, step, unary.span())
    }

    fn step_target_discarded(
        &mut self,
        scope: &Scope<'_>,
        operand: &Expression<'_>,
        step: i16,
        span: Span,
    ) -> Result<(), CompileError> {
        match operand.unparenthesized() {
            Expression::Access(Access::Property(access)) if step == 1 => {
                let object = self.expression(scope, access.object)?;
                let cache = self.add_ic_descriptor(
                    IcDescriptor::Member {
                        name: self.heap.intern(access.property.value.as_bytes()),
                        type_arguments: None,
                    },
                    span,
                )?;
                self.chunk.emit(
                    Instruction::PropertyStep {
                        object,
                        cache,
                        immediate: ImmediateInt::new(step),
                    },
                    span,
                );
                Ok(())
            }
            Expression::ArrayAccess(access) if step == 1 => {
                let Expression::Access(Access::Property(property)) = access.array.unparenthesized()
                else {
                    self.step_target(scope, operand, step, span, StepResult::New)?;
                    return Ok(());
                };
                let object = self.expression(scope, property.object)?;
                let cache = self.add_ic_descriptor(
                    IcDescriptor::Member {
                        name: self.heap.intern(property.property.value.as_bytes()),
                        type_arguments: None,
                    },
                    span,
                )?;
                let index = self.expression(scope, access.index)?;
                self.chunk.emit(
                    Instruction::PropertyIndexUpdate {
                        object,
                        operand: index,
                        cache,
                        mode: PropertyIndexUpdateMode::Increment,
                    },
                    span,
                );
                Ok(())
            }
            _ => {
                self.step_target(scope, operand, step, span, StepResult::New)?;
                Ok(())
            }
        }
    }

    fn step_target(
        &mut self,
        scope: &Scope<'_>,
        operand: &Expression<'_>,
        step: i16,
        span: Span,
        result: StepResult,
    ) -> Result<Register, CompileError> {
        match operand.unparenthesized() {
            Expression::Variable(variable) => {
                self.ensure_local_writable(variable.name, variable.span())?;
                let register = self.read_local(variable.name, variable.span())?;
                let old = match result {
                    StepResult::Old => {
                        let old = self.allocate(span)?;
                        self.move_into(old, register, span);
                        Some(old)
                    }
                    StepResult::New => None,
                };

                self.chunk
                    .emit(step_instruction(register, register, step), span);
                self.mark_defined(variable.name);
                Ok(old.unwrap_or(register))
            }
            Expression::Access(Access::Property(access)) => {
                self.step_property(scope, access, step, span, result)
            }
            Expression::ArrayAccess(access) => self.step_index(scope, access, step, span, result),
            Expression::Access(Access::StaticProperty(access)) => {
                self.step_static_property(scope, access, step, span, result)
            }
            _ => Err(CompileError::new(
                CompileErrorKind::InvalidIncrementTarget,
                "increment and decrement require a variable, property, static property, or index target",
                span,
            )),
        }
    }

    fn step_property(
        &mut self,
        scope: &Scope<'_>,
        access: &PropertyAccess<'_>,
        step: i16,
        span: Span,
        result: StepResult,
    ) -> Result<Register, CompileError> {
        let object = self.expression(scope, access.object)?;
        let cache = self.add_ic_descriptor(
            IcDescriptor::Member {
                name: self.heap.intern(access.property.value.as_bytes()),
                type_arguments: None,
            },
            span,
        )?;
        let current = self.allocate(span)?;
        self.chunk.emit(
            Instruction::PropertyGet {
                destination: current,
                object,
                cache,
            },
            span,
        );
        let stepped = self.allocate(span)?;
        self.chunk
            .emit(step_instruction(stepped, current, step), span);
        self.chunk.emit(
            Instruction::PropertySet {
                object,
                value: stepped,
                cache,
            },
            span,
        );
        Ok(step_result(result, current, stepped))
    }

    fn step_index(
        &mut self,
        scope: &Scope<'_>,
        access: &ArrayAccess<'_>,
        step: i16,
        span: Span,
        result: StepResult,
    ) -> Result<Register, CompileError> {
        let (root, mut indexes) = self.prepare_chain(scope, access.array)?;
        indexes.push(self.expression(scope, access.index)?);
        let steps = indexes
            .into_iter()
            .map(ChainStep::Index)
            .collect::<Vec<_>>();
        let mut place = Place::Chain {
            root: Box::new(root),
            levels: None,
            steps,
        };
        self.materialize_place(&mut place, span)?;
        let current = self.read_place(&place, span)?;
        let stepped = self.allocate(span)?;
        self.chunk
            .emit(step_instruction(stepped, current, step), span);
        self.write_place(scope, &place, stepped, span)?;
        Ok(step_result(result, current, stepped))
    }

    fn step_static_property(
        &mut self,
        scope: &Scope<'_>,
        access: &StaticPropertyAccess<'_>,
        step: i16,
        span: Span,
        result: StepResult,
    ) -> Result<Register, CompileError> {
        let cache = self.static_property_cache(scope, access)?;
        let current = self.allocate(span)?;
        self.chunk.emit(
            Instruction::StaticPropertyGet {
                destination: current,
                cache,
            },
            span,
        );
        let stepped = self.allocate(span)?;
        self.chunk
            .emit(step_instruction(stepped, current, step), span);
        self.chunk.emit(
            Instruction::StaticPropertySet {
                cache,
                value: stepped,
            },
            span,
        );
        Ok(step_result(result, current, stepped))
    }
}

const fn step_result(result: StepResult, old: Register, new: Register) -> Register {
    match result {
        StepResult::Old => old,
        StepResult::New => new,
    }
}
