//! Member-access chains: links, feet, and receiver-first method calls.

use whim_syn::cst::r#type::TypeArgumentList;

use crate::compiler::emit::calls::Access;
use crate::compiler::emit::calls::Argument;
use crate::compiler::emit::calls::ArgumentList;
use crate::compiler::emit::calls::ArrayAccess;
use crate::compiler::emit::calls::BodyCompiler;
use crate::compiler::emit::calls::Call;
use crate::compiler::emit::calls::CalleeSource;
use crate::compiler::emit::calls::CompileError;
use crate::compiler::emit::calls::Count;
use crate::compiler::emit::calls::Expression;
use crate::compiler::emit::calls::HasSpan;
use crate::compiler::emit::calls::IcDescriptor;
use crate::compiler::emit::calls::Instruction;
use crate::compiler::emit::calls::JumpOffset;
use crate::compiler::emit::calls::MethodCall;
use crate::compiler::emit::calls::NullSafeMethodCall;
use crate::compiler::emit::calls::NullSafePropertyAccess;
use crate::compiler::emit::calls::PropertyAccess;
use crate::compiler::emit::calls::Register;
use crate::compiler::emit::calls::Scope;
use crate::compiler::emit::calls::Span;
use crate::compiler::emit::calls::ValueUse;
use crate::compiler::emit::calls::argument_gate;
use crate::compiler::emit::calls::call_method_instruction;
use crate::compiler::emit::calls::call_value_instruction;
use crate::compiler::emit::calls::check_named_arguments;
use crate::unreachable_invariant;

enum ChainLink<'source, 'arena> {
    Property(&'source PropertyAccess<'arena>),
    NullSafeProperty(&'source NullSafePropertyAccess<'arena>),
    Index(&'source ArrayAccess<'arena>),
    Method(&'source MethodCall<'arena>),
    NullSafeMethod(&'source NullSafeMethodCall<'arena>),
}

impl BodyCompiler<'_, '_> {
    /// Compiles a member-access chain, joining every null-safe skip to a
    /// single null result.
    pub(in crate::compiler::emit) fn chain_root(
        &mut self,
        scope: &Scope<'_>,
        expression: &Expression<'_>,
    ) -> Result<Register, CompileError> {
        self.chain_root_with_use(scope, expression, ValueUse::Needed)
    }

    pub(in crate::compiler::emit) fn chain_root_with_use(
        &mut self,
        scope: &Scope<'_>,
        expression: &Expression<'_>,
        value_use: ValueUse,
    ) -> Result<Register, CompileError> {
        let mut escapes = Vec::new();
        let result = self.chain(scope, expression, &mut escapes, value_use)?;
        if !escapes.is_empty() {
            let skip = self.chunk.emit(
                Instruction::Jump {
                    offset: JumpOffset::new(0),
                },
                expression.span(),
            );

            let null_join = self.code_position();
            for escape in escapes {
                self.chunk.patch_jump(escape, null_join);
            }

            self.chunk.emit(
                Instruction::LoadNull {
                    destination: result,
                },
                expression.span(),
            );

            let after = self.code_position();
            self.chunk.patch_jump(skip, after);
        }

        Ok(result)
    }

    /// Compiles a member-access chain, joining every null-safe skip to a
    /// single null result, without recursing once per link.
    pub(in crate::compiler::emit) fn chain(
        &mut self,
        scope: &Scope<'_>,
        expression: &Expression<'_>,
        escapes: &mut Vec<u32>,
        value_use: ValueUse,
    ) -> Result<Register, CompileError> {
        let start = expression.leftmost_span();
        let mut links = Vec::new();
        let mut current = expression;
        let foot = loop {
            match current {
                Expression::Parenthesized(parenthesized) => current = parenthesized.expression,
                Expression::Access(Access::Property(access)) => {
                    links.push(ChainLink::Property(access));
                    current = access.object;
                }
                Expression::ArrayAccess(access) => {
                    links.push(ChainLink::Index(access));
                    current = access.array;
                }
                Expression::Access(Access::NullSafeProperty(access)) => {
                    links.push(ChainLink::NullSafeProperty(access));
                    current = access.object;
                }
                Expression::Call(Call::Method(call)) => {
                    links.push(ChainLink::Method(call));
                    current = call.object;
                }
                Expression::Call(Call::NullSafeMethod(call)) => {
                    links.push(ChainLink::NullSafeMethod(call));
                    current = call.object;
                }
                other => break other,
            }
        };

        let mut accumulator = self.chain_foot(scope, foot)?;
        let link_count = links.len();
        for (index, link) in links.into_iter().rev().enumerate() {
            let link_use = if index + 1 == link_count {
                value_use
            } else {
                ValueUse::Needed
            };
            accumulator = self.chain_link(scope, &link, accumulator, start, escapes, link_use)?;
        }

        Ok(accumulator)
    }
    fn chain_link(
        &mut self,
        scope: &Scope<'_>,
        link: &ChainLink<'_, '_>,
        object: Register,
        start: Span,
        escapes: &mut Vec<u32>,
        value_use: ValueUse,
    ) -> Result<Register, CompileError> {
        match link {
            ChainLink::Property(access) => {
                let span = start.join(access.property.span());
                let cache = self.add_ic_descriptor(
                    IcDescriptor::Member {
                        name: self.heap.intern(access.property.value.as_bytes()),
                        type_arguments: None,
                    },
                    span,
                )?;

                let destination = self.allocate(span)?;
                self.chunk.emit(
                    Instruction::PropertyGet {
                        destination,
                        object,
                        cache,
                    },
                    span,
                );

                Ok(destination)
            }
            ChainLink::Index(access) => {
                let span = start.join(access.right_bracket);
                let index = self.expression(scope, access.index)?;
                let destination = self.allocate(span)?;
                self.chunk.emit(
                    Instruction::IndexGet {
                        destination,
                        container: object,
                        index,
                    },
                    span,
                );

                Ok(destination)
            }
            ChainLink::NullSafeProperty(access) => {
                let span = start.join(access.property.span());
                escapes.push(self.chunk.emit(
                    Instruction::JumpIfNull {
                        subject: object,
                        offset: JumpOffset::new(0),
                    },
                    access.question_mark_arrow,
                ));

                let cache = self.add_ic_descriptor(
                    IcDescriptor::Member {
                        name: self.heap.intern(access.property.value.as_bytes()),
                        type_arguments: None,
                    },
                    span,
                )?;

                let destination = self.allocate(span)?;
                self.chunk.emit(
                    Instruction::PropertyGet {
                        destination,
                        object,
                        cache,
                    },
                    span,
                );

                Ok(destination)
            }
            ChainLink::Method(call) => self.method_call_with_receiver(
                scope,
                &CalleeSource::Method {
                    receiver: object,
                    name: call.method.value,
                },
                call.type_arguments.as_ref(),
                &call.argument_list,
                start.join(call.argument_list.span()),
                value_use,
            ),
            ChainLink::NullSafeMethod(call) => {
                escapes.push(self.chunk.emit(
                    Instruction::JumpIfNull {
                        subject: object,
                        offset: JumpOffset::new(0),
                    },
                    call.question_mark_arrow,
                ));
                self.method_call_with_receiver(
                    scope,
                    &CalleeSource::Method {
                        receiver: object,
                        name: call.method.value,
                    },
                    call.type_arguments.as_ref(),
                    &call.argument_list,
                    start.join(call.argument_list.span()),
                    value_use,
                )
            }
        }
    }

    fn chain_foot(
        &mut self,
        scope: &Scope<'_>,
        expression: &Expression<'_>,
    ) -> Result<Register, CompileError> {
        match expression {
            Expression::Access(Access::Constant(access)) => {
                let cache = self.add_ic_descriptor(
                    IcDescriptor::Member {
                        name: scope.resolver.resolve(self.heap, &access.name),
                        type_arguments: None,
                    },
                    access.span(),
                )?;

                let destination = self.allocate(access.span())?;
                self.chunk.emit(
                    Instruction::ConstantGet { destination, cache },
                    access.span(),
                );

                Ok(destination)
            }
            Expression::Access(Access::StaticProperty(access)) => {
                let cache = self.static_property_cache(scope, access)?;
                let destination = self.allocate(access.span())?;
                self.chunk.emit(
                    Instruction::StaticPropertyGet { destination, cache },
                    access.span(),
                );

                Ok(destination)
            }
            Expression::Access(Access::ClassConstant(access)) => {
                let class = self.class_reference_atom(scope, &access.class)?;
                let cache = self.add_ic_descriptor(
                    IcDescriptor::ClassMember {
                        class,
                        member: self.heap.intern(access.constant.value.as_bytes()),
                        type_arguments: None,
                    },
                    access.span(),
                )?;

                let destination = self.allocate(access.span())?;
                self.chunk.emit(
                    Instruction::ClassConstantGet { destination, cache },
                    access.span(),
                );

                Ok(destination)
            }
            other => self.expression(scope, other),
        }
    }

    fn method_call_with_receiver(
        &mut self,
        scope: &Scope<'_>,
        source: &CalleeSource<'_, '_>,
        type_arguments: Option<&TypeArgumentList<'_>>,
        argument_list: &ArgumentList<'_>,
        span: Span,
        value_use: ValueUse,
    ) -> Result<Register, CompileError> {
        let CalleeSource::Method { receiver, name } = source else {
            // SAFETY: the surrounding invariant makes this path unreachable.
            unsafe { unreachable_invariant("a method call has a method callee") }
        };
        let receiver = *receiver;
        check_named_arguments(argument_list)?;
        let count = argument_gate(argument_list.arguments.len(), span)?;
        let named = argument_list
            .arguments
            .iter()
            .any(|argument| !argument.is_positional());

        let lowered = self.lower_turbofish(scope, type_arguments)?;
        if type_arguments.is_some() && named {
            let callee = self.callee_value(scope, source, span)?;

            let callee = self.specialize_callee(scope, callee, type_arguments, span)?;
            if argument_list
                .arguments
                .iter()
                .any(|argument| !argument.is_positional())
            {
                return self.shaped_call(
                    scope,
                    &CalleeSource::Value(callee),
                    argument_list,
                    span,
                    value_use,
                );
            }

            let destination = self.allocate(span)?;
            let mark = self.registers.mark();
            let first = self.window(
                scope,
                argument_list.arguments.iter().map(Argument::value),
                argument_list.arguments.len(),
                span,
            )?;

            self.chunk.emit(
                call_value_instruction(value_use, Count::new(count), destination, callee, first),
                span,
            );

            self.registers.release_to(mark);
            return Ok(destination);
        }

        if argument_list
            .arguments
            .iter()
            .any(|argument| !argument.is_positional())
        {
            return self.shaped_call(scope, source, argument_list, span, value_use);
        }

        let destination = self.allocate(span)?;
        let mark = self.registers.mark();
        let window_first = self.allocate(span)?;
        self.move_into(window_first, receiver, span);
        let mut slots = Vec::new();
        for _ in 0..argument_list.arguments.len() {
            slots.push(self.allocate(span)?);
        }

        for (slot, argument) in slots.iter().zip(argument_list.arguments.iter()) {
            let inner = self.registers.mark();
            let value = self.expression(scope, argument.value())?;
            self.move_argument_into(*slot, value, inner, argument.span());
            self.registers.release_to(inner);
        }

        let cache = self.add_ic_descriptor(
            IcDescriptor::Member {
                name: self.heap.intern(name.as_bytes()),
                type_arguments: lowered,
            },
            span,
        )?;

        self.chunk.emit(
            call_method_instruction(
                value_use,
                Count::new(count + 1),
                destination,
                window_first,
                cache,
            ),
            span,
        );

        self.registers.release_to(mark);
        Ok(destination)
    }
}
