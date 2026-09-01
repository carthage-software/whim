//! Expressions, operators, and literals.

use hashbrown::HashSet;

use whim_syn::cst::array::DictExpression;
use whim_syn::cst::array::DictPair;
use whim_syn::cst::array::TupleExpression;
use whim_syn::cst::array::VecExpression;
use whim_syn::cst::array::VecFillExpression;
use whim_syn::cst::expression::Break;
use whim_syn::cst::expression::Continue;
use whim_syn::cst::expression::InterpolatedString;
use whim_syn::cst::expression::InterpolatedStringLiteral;
use whim_syn::cst::expression::InterpolatedStringPart;
use whim_syn::cst::expression::Return;
use whim_syn::cst::expression::Throw;
use whim_syn::cst::operation::TypeOperation;
use whim_syn::cst::operation::TypeOperator;
use whim_syn::cst::operation::UnaryPrefixOperator;

use crate::compiler::emit::Access;
use crate::compiler::emit::AsMode;
use crate::compiler::emit::BodyCompiler;
use crate::compiler::emit::BytecodeLiteral;
use crate::compiler::emit::Call;
use crate::compiler::emit::ChainStep;
use crate::compiler::emit::CompileError;
use crate::compiler::emit::CompileErrorKind;
use crate::compiler::emit::ConstantIndex;
use crate::compiler::emit::Count;
use crate::compiler::emit::DictEntry;
use crate::compiler::emit::Expression;
use crate::compiler::emit::HasSpan;
use crate::compiler::emit::IcDescriptor;
use crate::compiler::emit::ImmediateInt;
use crate::compiler::emit::Instruction;
use crate::compiler::emit::JumpOffset;
use crate::compiler::emit::Literal;
use crate::compiler::emit::LoopJump;
use crate::compiler::emit::Place;
use crate::compiler::emit::Register;
use crate::compiler::emit::Scope;
use crate::compiler::emit::Span;
use crate::compiler::emit::TupleElement;
use crate::compiler::emit::TypeDescriptor;
use crate::compiler::emit::ValueUse;
use crate::compiler::emit::VecElement;
use crate::compiler::emit::check_tuple_sequence;
use crate::compiler::emit::integer_gate;
use crate::compiler::emit::lower_checked_type;
use crate::compiler::emit::side_table_limit;
use crate::compiler::emit::tuple_window_gate;
use crate::unreachable_invariant;
use crate::unwrap_result_invariant;

pub(in crate::compiler::emit) mod operators;

fn literal_check_descriptor(descriptor: &TypeDescriptor) -> bool {
    match descriptor {
        TypeDescriptor::Never
        | TypeDescriptor::Null
        | TypeDescriptor::TrueLiteral
        | TypeDescriptor::FalseLiteral
        | TypeDescriptor::IntLiteral(_)
        | TypeDescriptor::FloatLiteral(_)
        | TypeDescriptor::StringLiteral(_) => true,
        TypeDescriptor::Union(members) | TypeDescriptor::Intersection(members) => {
            !members.is_empty() && members.iter().all(literal_check_descriptor)
        }
        TypeDescriptor::Negated(inner) => literal_check_descriptor(inner),
        _ => false,
    }
}

impl BodyCompiler<'_, '_> {
    pub(in crate::compiler) fn expression(
        &mut self,
        scope: &Scope<'_>,
        expression: &Expression<'_>,
    ) -> Result<Register, CompileError> {
        match expression {
            Expression::Parenthesized(parenthesized) => {
                self.expression(scope, parenthesized.expression)
            }
            Expression::Literal(literal) => self.literal(literal),
            Expression::InterpolatedString(string) => self.interpolated_string(scope, string),
            Expression::Variable(variable) => self.read_local(variable.name, variable.span()),
            Expression::Vec(vector) => self.vector(scope, vector),
            Expression::VecFill(fill) => self.filled_vector(scope, fill),
            Expression::Dict(dictionary) => self.dictionary(scope, dictionary),
            Expression::Tuple(tuple) => self.tuple(scope, tuple),
            Expression::ArrayAppend(append) => Err(CompileError::new(
                CompileErrorKind::AppendTargetUsedAsValue,
                "an append target cannot be read as a value",
                append.span(),
            )),
            Expression::ArrayAccess(_) => self.chain_root(scope, expression),
            Expression::Binary(binary) => self.binary(scope, binary),
            Expression::UnaryPrefix(unary) => self.unary_prefix(scope, unary),
            Expression::UnaryPostfix(unary) => self.unary_postfix(scope, unary),
            Expression::TypeOperation(operation) => self.type_operation(scope, operation),
            Expression::Assignment(assignment) => self.assignment(scope, assignment),
            Expression::Access(_) | Expression::Call(Call::Method(_) | Call::NullSafeMethod(_)) => {
                self.chain_root(scope, expression)
            }
            Expression::Call(call) => self.call(scope, call, ValueUse::Needed),
            Expression::PartialApplication(application) => {
                self.partial_application(scope, application)
            }
            Expression::Instantiation(instantiation) => self.instantiation(scope, instantiation),
            Expression::Closure(closure) => self.closure(scope, closure),
            Expression::ShortClosure(closure) => self.short_closure(scope, closure),
            Expression::Match(matching) => self.matching(scope, matching),
            Expression::Throw(throw) => self.throw_expression(scope, throw),
            Expression::Break(r#break) => self.break_expression(scope, r#break),
            Expression::Continue(r#continue) => self.continue_expression(scope, r#continue),
            Expression::Return(r#return) => self.return_expression(scope, r#return),
            Expression::Construct(construct) => self.construct(scope, construct),
        }
    }

    pub(in crate::compiler) fn expression_discarded(
        &mut self,
        scope: &Scope<'_>,
        expression: &Expression<'_>,
    ) -> Result<(), CompileError> {
        match expression {
            Expression::Parenthesized(parenthesized) => {
                self.expression_discarded(scope, parenthesized.expression)
            }
            Expression::Call(call) => {
                let result = self.call(scope, call, ValueUse::Discarded)?;
                self.chunk.emit(
                    Instruction::CheckDiscardedResult { source: result },
                    expression.span(),
                );
                Ok(())
            }
            Expression::UnaryPrefix(unary) => self.unary_prefix_discarded(scope, unary),
            Expression::UnaryPostfix(unary) => self.unary_postfix_discarded(scope, unary),
            Expression::Assignment(assignment) => self.assignment_discarded(scope, assignment),
            Expression::Return(r#return) => self.emit_return(scope, r#return),
            _ => {
                self.expression(scope, expression)?;
                Ok(())
            }
        }
    }

    fn vector(
        &mut self,
        scope: &Scope<'_>,
        vector: &VecExpression<'_>,
    ) -> Result<Register, CompileError> {
        let destination = self.allocate(vector.span())?;
        let mark = self.registers.mark();
        let spread = vector.elements.iter().any(VecElement::is_spread);
        if let Some(element_count) = u8::try_from(vector.elements.len()).ok().filter(|_| !spread) {
            let first = self.window(
                scope,
                vector.elements.iter().map(|element| element.value),
                vector.elements.len(),
                vector.span(),
            )?;
            self.chunk.emit(
                Instruction::NewVec {
                    element_count: Count::new(element_count),
                    destination,
                    first_element: first,
                },
                vector.span(),
            );
        } else {
            self.chunk.emit(
                Instruction::NewVec {
                    element_count: Count::new(0),
                    destination,
                    first_element: Register::new(self.registers.mark()),
                },
                vector.span(),
            );
            self.reserve_collection(
                destination,
                vector
                    .elements
                    .iter()
                    .filter(|element| !element.is_spread())
                    .count(),
                vector.span(),
            )?;
            for element in &vector.elements {
                let inner = self.registers.mark();
                let value = self.expression(scope, element.value)?;
                let instruction = if element.is_spread() {
                    Instruction::Spread {
                        container: destination,
                        value,
                    }
                } else {
                    Instruction::Append {
                        container: destination,
                        value,
                    }
                };
                self.chunk.emit(instruction, element.span());
                self.registers.release_to(inner);
            }
        }

        self.registers.release_to(mark);
        Ok(destination)
    }

    fn filled_vector(
        &mut self,
        scope: &Scope<'_>,
        fill: &VecFillExpression<'_>,
    ) -> Result<Register, CompileError> {
        let value = self.expression(scope, fill.value)?;
        let size = self.expression(scope, fill.size)?;
        let destination = self.allocate(fill.span())?;
        self.chunk.emit(
            Instruction::NewFilledVec {
                destination,
                value,
                size,
            },
            fill.span(),
        );
        Ok(destination)
    }

    fn dictionary(
        &mut self,
        scope: &Scope<'_>,
        dictionary: &DictExpression<'_>,
    ) -> Result<Register, CompileError> {
        check_duplicate_dict_keys(dictionary)?;
        let destination = self.allocate(dictionary.span())?;
        let mark = self.registers.mark();
        let pairs = dictionary
            .entries
            .iter()
            .filter_map(|entry| match entry {
                DictEntry::Pair(pair) => Some(pair),
                DictEntry::Spread(_) => None,
            })
            .collect::<Vec<_>>();
        let pair_count = u8::try_from(dictionary.entries.len())
            .ok()
            .filter(|_| pairs.len() == dictionary.entries.len());
        if let Some(pair_count) = pair_count {
            self.windowed_dictionary(scope, dictionary, destination, &pairs, pair_count)?;
        } else {
            self.incremental_dictionary(scope, dictionary, destination)?;
        }

        self.registers.release_to(mark);
        Ok(destination)
    }

    fn windowed_dictionary(
        &mut self,
        scope: &Scope<'_>,
        dictionary: &DictExpression<'_>,
        destination: Register,
        pairs: &[&DictPair<'_>],
        pair_count: u8,
    ) -> Result<(), CompileError> {
        let mut first = None;
        for pair in pairs {
            let key_slot = self.allocate(pair.key.span())?;
            first.get_or_insert(key_slot);
            let value_slot = self.allocate(pair.value.span())?;
            let inner = self.registers.mark();
            let key = self.expression(scope, pair.key)?;
            self.move_into(key_slot, key, pair.key.span());
            let value = self.expression(scope, pair.value)?;
            self.move_into(value_slot, value, pair.value.span());
            self.registers.release_to(inner);
        }
        let first = first.unwrap_or_else(|| Register::new(self.registers.mark()));
        self.chunk.emit(
            Instruction::NewDict {
                pair_count: Count::new(pair_count),
                destination,
                first_pair: first,
            },
            dictionary.span(),
        );

        Ok(())
    }

    fn incremental_dictionary(
        &mut self,
        scope: &Scope<'_>,
        dictionary: &DictExpression<'_>,
        destination: Register,
    ) -> Result<(), CompileError> {
        self.chunk.emit(
            Instruction::NewDict {
                pair_count: Count::new(0),
                destination,
                first_pair: Register::new(self.registers.mark()),
            },
            dictionary.span(),
        );
        self.reserve_collection(
            destination,
            dictionary
                .entries
                .iter()
                .filter(|entry| matches!(entry, DictEntry::Pair(_)))
                .count(),
            dictionary.span(),
        )?;
        for entry in &dictionary.entries {
            let inner = self.registers.mark();
            match entry {
                DictEntry::Pair(pair) => {
                    let key = self.expression(scope, pair.key)?;
                    let value = self.expression(scope, pair.value)?;
                    self.chunk.emit(
                        Instruction::IndexSet {
                            container: destination,
                            index: key,
                            value,
                        },
                        pair.value.span(),
                    );
                }
                DictEntry::Spread(spread) => {
                    let value = self.expression(scope, spread.value)?;
                    self.chunk.emit(
                        Instruction::Spread {
                            container: destination,
                            value,
                        },
                        spread.span(),
                    );
                }
            }
            self.registers.release_to(inner);
        }

        Ok(())
    }

    fn reserve_collection(
        &mut self,
        container: Register,
        count: usize,
        span: Span,
    ) -> Result<(), CompileError> {
        if count == 0 {
            return Ok(());
        }
        // SAFETY: source collection lengths fit in the language's integer range.
        let count = unsafe {
            unwrap_result_invariant(
                i64::try_from(count),
                "a source collection length fits in a Whim integer",
            )
        };
        let additional = self.allocate(span)?;
        self.load_integer(additional, count, span)?;
        self.chunk.emit(
            Instruction::ReserveCollection {
                container,
                additional,
            },
            span,
        );
        Ok(())
    }

    fn tuple(
        &mut self,
        scope: &Scope<'_>,
        tuple: &TupleExpression<'_>,
    ) -> Result<Register, CompileError> {
        check_tuple_sequence(
            CompileErrorKind::TooManyTupleElements,
            "a tuple may have",
            "elements",
            tuple.elements,
        )?;
        let mut values = Vec::new();
        for element in &tuple.elements {
            match element {
                TupleElement::Value(value) => values.push(*value),
                TupleElement::Rest(rest) => {
                    return Err(CompileError::new(
                        CompileErrorKind::SpreadInTuple,
                        "a tuple literal cannot spread; a tuple's arity is fixed at compile time, \
                         while a spread's length is known only at runtime. `...` inside \
                         parentheses is a destructuring rest, and needs an assignment to its left",
                        rest.span(),
                    ));
                }
            }
        }
        let destination = self.allocate(tuple.span())?;
        let mark = self.registers.mark();
        let first = self.window(scope, values.iter().copied(), values.len(), tuple.span())?;
        self.chunk.emit(
            Instruction::NewTuple {
                element_count: Count::new(tuple_window_gate(tuple.elements.len(), tuple.span())?),
                destination,
                first_element: first,
            },
            tuple.span(),
        );
        self.registers.release_to(mark);
        Ok(destination)
    }

    fn type_operation(
        &mut self,
        scope: &Scope<'_>,
        operation: &TypeOperation<'_>,
    ) -> Result<Register, CompileError> {
        let source = self.expression(scope, operation.operand)?;
        let descriptor = lower_checked_type(&self.types(scope), operation.r#type)?;
        if matches!(operation.operator, TypeOperator::Check(_))
            && literal_check_descriptor(&descriptor)
        {
            let destination = self.allocate(operation.span())?;
            self.emit_literal_check(source, destination, &descriptor, operation.span())?;
            return Ok(destination);
        }
        let descriptor = self
            .chunk
            .add_type_descriptor(descriptor)
            .map_err(|full| side_table_limit(full, operation.span()))?;
        let destination = self.allocate(operation.span())?;
        let instruction = match operation.operator {
            TypeOperator::Check(_) => Instruction::Is {
                destination,
                source,
                descriptor,
            },
            TypeOperator::Assert(_) => Instruction::AsCheck {
                destination,
                source,
                descriptor,
                mode: AsMode::Cast,
            },
            TypeOperator::AssertOrNull(_, _) => Instruction::AsOrNull {
                destination,
                source,
                descriptor,
            },
        };
        self.chunk.emit(instruction, operation.span());
        Ok(destination)
    }

    fn emit_literal_check(
        &mut self,
        source: Register,
        destination: Register,
        descriptor: &TypeDescriptor,
        span: Span,
    ) -> Result<(), CompileError> {
        match descriptor {
            TypeDescriptor::Never => {
                self.chunk
                    .emit(Instruction::LoadFalse { destination }, span);
            }
            TypeDescriptor::Null
            | TypeDescriptor::TrueLiteral
            | TypeDescriptor::FalseLiteral
            | TypeDescriptor::IntLiteral(_)
            | TypeDescriptor::FloatLiteral(_)
            | TypeDescriptor::StringLiteral(_) => {
                self.emit_literal_equality(source, destination, descriptor, span)?;
            }
            TypeDescriptor::Union(members) => {
                self.emit_literal_members(source, destination, members, true, span)?;
            }
            TypeDescriptor::Intersection(members) => {
                self.emit_literal_members(source, destination, members, false, span)?;
            }
            TypeDescriptor::Negated(inner) => {
                self.emit_literal_check(source, destination, inner, span)?;
                self.chunk.emit(
                    Instruction::Not {
                        destination,
                        source: destination,
                    },
                    span,
                );
            }
            // SAFETY: the surrounding invariant makes this path unreachable.
            _ => unsafe {
                unreachable_invariant("literal check lowering receives a literal descriptor")
            },
        }

        Ok(())
    }

    fn emit_literal_equality(
        &mut self,
        source: Register,
        destination: Register,
        descriptor: &TypeDescriptor,
        span: Span,
    ) -> Result<(), CompileError> {
        let mark = self.registers.mark();
        let expected = self.allocate(span)?;
        match descriptor {
            TypeDescriptor::Null => {
                self.chunk.emit(
                    Instruction::LoadNull {
                        destination: expected,
                    },
                    span,
                );
            }
            TypeDescriptor::TrueLiteral => {
                self.chunk.emit(
                    Instruction::LoadTrue {
                        destination: expected,
                    },
                    span,
                );
            }
            TypeDescriptor::FalseLiteral => {
                self.chunk.emit(
                    Instruction::LoadFalse {
                        destination: expected,
                    },
                    span,
                );
            }
            TypeDescriptor::IntLiteral(value) => {
                self.load_integer(expected, *value, span)?;
            }
            TypeDescriptor::FloatLiteral(value) => {
                let constant = self.add_constant(BytecodeLiteral::Float(*value), span)?;
                self.chunk.emit(
                    Instruction::LoadConstant {
                        destination: expected,
                        constant,
                    },
                    span,
                );
            }
            TypeDescriptor::StringLiteral(value) => {
                let constant = self.add_constant(BytecodeLiteral::String(value.clone()), span)?;
                self.chunk.emit(
                    Instruction::LoadConstant {
                        destination: expected,
                        constant,
                    },
                    span,
                );
            }
            // SAFETY: the surrounding invariant makes this path unreachable.
            _ => unsafe { unreachable_invariant("literal equality receives a literal descriptor") },
        }
        self.chunk.emit(
            Instruction::Equal {
                destination,
                left: source,
                right: expected,
            },
            span,
        );
        self.registers.release_to(mark);
        Ok(())
    }

    fn emit_literal_members(
        &mut self,
        source: Register,
        destination: Register,
        members: &[TypeDescriptor],
        accept_on_match: bool,
        span: Span,
    ) -> Result<(), CompileError> {
        let mut exits = Vec::with_capacity(members.len().saturating_sub(1));
        for (index, member) in members.iter().enumerate() {
            self.emit_literal_check(source, destination, member, span)?;
            if index + 1 == members.len() {
                continue;
            }
            exits.push(self.chunk.emit(
                if accept_on_match {
                    Instruction::JumpIfTrue {
                        condition: destination,
                        offset: JumpOffset::new(0),
                    }
                } else {
                    Instruction::JumpIfFalse {
                        condition: destination,
                        offset: JumpOffset::new(0),
                    }
                },
                span,
            ));
        }
        let after = self.code_position();
        for exit in exits {
            self.chunk.patch_jump(exit, after);
        }
        Ok(())
    }

    fn throw_expression(
        &mut self,
        scope: &Scope<'_>,
        throw: &Throw<'_>,
    ) -> Result<Register, CompileError> {
        let source = self.expression(scope, throw.exception)?;
        self.chunk.emit(Instruction::Throw { source }, throw.span());
        let destination = self.allocate(throw.span())?;
        self.chunk
            .emit(Instruction::LoadNull { destination }, throw.span());
        Ok(destination)
    }

    fn break_expression(
        &mut self,
        scope: &Scope<'_>,
        r#break: &Break<'_>,
    ) -> Result<Register, CompileError> {
        let level = r#break.level.as_ref().map_or(1, |literal| literal.value);
        self.loop_jump(scope, level, LoopJump::Break, r#break.span())?;
        self.unreachable_expression(r#break.span())
    }

    fn continue_expression(
        &mut self,
        scope: &Scope<'_>,
        r#continue: &Continue<'_>,
    ) -> Result<Register, CompileError> {
        let level = r#continue.level.as_ref().map_or(1, |literal| literal.value);
        self.loop_jump(scope, level, LoopJump::Continue, r#continue.span())?;
        self.unreachable_expression(r#continue.span())
    }

    fn return_expression(
        &mut self,
        scope: &Scope<'_>,
        r#return: &Return<'_>,
    ) -> Result<Register, CompileError> {
        self.emit_return(scope, r#return)?;
        self.unreachable_expression(r#return.span())
    }

    fn unreachable_expression(&mut self, span: Span) -> Result<Register, CompileError> {
        let destination = self.allocate(span)?;
        self.chunk.emit(Instruction::LoadNull { destination }, span);
        Ok(destination)
    }

    fn interpolated_string(
        &mut self,
        scope: &Scope<'_>,
        string: &InterpolatedString<'_>,
    ) -> Result<Register, CompileError> {
        let Some(InterpolatedStringPart::Literal(first)) = string.parts.first() else {
            // SAFETY: the surrounding invariant makes this path unreachable.
            unsafe { unreachable_invariant("an interpolated string begins with a literal prefix") }
        };

        let mut accumulator = self.interpolated_literal(first)?;

        for part in &string.parts[1..] {
            let right = match part {
                InterpolatedStringPart::Literal(literal) if literal.value.is_empty() => continue,
                InterpolatedStringPart::Literal(literal) => self.interpolated_literal(literal)?,
                InterpolatedStringPart::Variable(variable) => {
                    self.read_local(variable.name, variable.span())?
                }
                InterpolatedStringPart::Expression(interpolation) => {
                    self.expression(scope, interpolation.expression)?
                }
            };

            let span = string.opening_quote.join(part.span());
            let destination = self.allocate(span)?;
            self.chunk.emit(
                Instruction::Concatenate {
                    destination,
                    left: accumulator,
                    right,
                },
                span,
            );

            accumulator = destination;
        }

        Ok(accumulator)
    }

    fn interpolated_literal(
        &mut self,
        literal: &InterpolatedStringLiteral<'_>,
    ) -> Result<Register, CompileError> {
        let destination = self.allocate(literal.span)?;
        let constant = self.string_constant(literal.value, literal.span)?;
        self.chunk.emit(
            Instruction::LoadConstant {
                destination,
                constant,
            },
            literal.span,
        );

        Ok(destination)
    }

    fn literal(&mut self, literal: &Literal<'_>) -> Result<Register, CompileError> {
        let destination = self.allocate(literal.span())?;
        match literal {
            Literal::Null(keyword) => {
                self.chunk
                    .emit(Instruction::LoadNull { destination }, keyword.span());
            }
            Literal::True(keyword) => {
                self.chunk
                    .emit(Instruction::LoadTrue { destination }, keyword.span());
            }
            Literal::False(keyword) => {
                self.chunk
                    .emit(Instruction::LoadFalse { destination }, keyword.span());
            }
            Literal::Integer(integer) => {
                let value = integer_gate(integer.value, false, integer.span)?;
                self.load_integer(destination, value, integer.span)?;
            }
            Literal::Float(float) => {
                let constant =
                    self.add_constant(BytecodeLiteral::Float(float.value), float.span)?;
                self.chunk.emit(
                    Instruction::LoadConstant {
                        destination,
                        constant,
                    },
                    float.span,
                );
            }
            Literal::String(string) => {
                let constant = self.string_constant(string.value, string.span)?;
                self.chunk.emit(
                    Instruction::LoadConstant {
                        destination,
                        constant,
                    },
                    string.span,
                );
            }
        }

        Ok(destination)
    }

    /// Emits an integer load: the immediate form when it fits, the pool
    /// otherwise.
    pub(in crate::compiler::emit) fn load_integer(
        &mut self,
        destination: Register,
        value: i64,
        span: Span,
    ) -> Result<(), CompileError> {
        if let Ok(immediate) = i16::try_from(value) {
            self.chunk.emit(
                Instruction::LoadInt {
                    destination,
                    immediate: ImmediateInt::new(immediate),
                },
                span,
            );
        } else {
            let constant = self.add_constant(BytecodeLiteral::Int(value), span)?;
            self.chunk.emit(
                Instruction::LoadConstant {
                    destination,
                    constant,
                },
                span,
            );
        }

        Ok(())
    }

    pub(in crate::compiler::emit) fn string_constant(
        &mut self,
        bytes: &[u8],
        span: Span,
    ) -> Result<ConstantIndex, CompileError> {
        let literal = BytecodeLiteral::String(self.heap.intern(bytes));

        self.add_constant(literal, span)
    }
}

fn check_duplicate_dict_keys(dictionary: &DictExpression<'_>) -> Result<(), CompileError> {
    let mut keys = HashSet::with_capacity(dictionary.entries.len());
    for entry in &dictionary.entries {
        let DictEntry::Pair(pair) = entry else {
            continue;
        };
        let Some(key) = constant_dict_key(pair.key) else {
            continue;
        };
        if !keys.insert(key) {
            return Err(CompileError::new(
                CompileErrorKind::DuplicateDictionaryKey,
                "a dictionary literal cannot repeat a constant key",
                pair.key.span(),
            ));
        }
    }
    Ok(())
}

#[derive(Hash, PartialEq, Eq)]
enum ConstantDictKey<'arena> {
    Integer(i128),
    Boolean(bool),
    String(&'arena [u8]),
}

fn constant_dict_key<'arena>(expression: &Expression<'arena>) -> Option<ConstantDictKey<'arena>> {
    match expression {
        Expression::Parenthesized(expression) => constant_dict_key(expression.expression),
        Expression::Literal(Literal::Integer(integer)) => {
            Some(ConstantDictKey::Integer(i128::from(integer.value)))
        }
        Expression::Literal(Literal::String(string)) => Some(ConstantDictKey::String(string.value)),
        Expression::Literal(Literal::True(_)) => Some(ConstantDictKey::Boolean(true)),
        Expression::Literal(Literal::False(_)) => Some(ConstantDictKey::Boolean(false)),
        Expression::UnaryPrefix(expression)
            if matches!(expression.operator, UnaryPrefixOperator::Negation(_)) =>
        {
            let Expression::Literal(Literal::Integer(integer)) = expression.operand else {
                return None;
            };
            Some(ConstantDictKey::Integer(-i128::from(integer.value)))
        }
        _ => None,
    }
}
