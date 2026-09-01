//! Callee values, shaped calls, partial application, and instantiation.

use whim_syn::cst::call::PartialArgumentList;
use whim_syn::cst::expression::Instantiation;
use whim_syn::cst::sequence::TokenSeparatedSequence;

use crate::compiler::emit::calls::Argument;
use crate::compiler::emit::calls::ArgumentList;
use crate::compiler::emit::calls::BodyCompiler;
use crate::compiler::emit::calls::Call;
use crate::compiler::emit::calls::CallDescriptor;
use crate::compiler::emit::calls::Callee;
use crate::compiler::emit::calls::CalleeSource;
use crate::compiler::emit::calls::ClassReference;
use crate::compiler::emit::calls::CompileError;
use crate::compiler::emit::calls::CompileErrorKind;
use crate::compiler::emit::calls::Count;
use crate::compiler::emit::calls::Expression;
use crate::compiler::emit::calls::HasSpan;
use crate::compiler::emit::calls::IcDescriptor;
use crate::compiler::emit::calls::Instruction;
use crate::compiler::emit::calls::PartialApplication;
use crate::compiler::emit::calls::PartialArgument;
use crate::compiler::emit::calls::PresetDescriptor;
use crate::compiler::emit::calls::PresetSlot;
use crate::compiler::emit::calls::Register;
use crate::compiler::emit::calls::Scope;
use crate::compiler::emit::calls::Span;
use crate::compiler::emit::calls::TypeDescriptor;
use crate::compiler::emit::calls::ValueUse;
use crate::compiler::emit::calls::argument_gate;
use crate::compiler::emit::calls::call_with_names_instruction;
use crate::compiler::emit::calls::check_call_type_argument_arity;
use crate::compiler::emit::calls::check_named_arguments;
use crate::compiler::emit::calls::check_sequence;

struct PartialPlan<'arena> {
    slots: Vec<PresetSlot>,
    given: Vec<&'arena Expression<'arena>>,
    open_remaining: bool,
}

fn check_partial_named_arguments(
    argument_list: &PartialArgumentList<'_>,
) -> Result<(), CompileError> {
    let mut seen: Vec<&str> = Vec::new();
    let mut has_named = false;
    for argument in &argument_list.arguments {
        let name = match argument {
            PartialArgument::Named(named) => Some((named.name.value, named.span())),
            PartialArgument::NamedPlaceholder(named) => Some((named.name.value, named.span())),
            PartialArgument::Positional(_) | PartialArgument::Placeholder(_) => {
                if has_named {
                    return Err(CompileError::new(
                        CompileErrorKind::PositionalArgumentAfterNamedArgument,
                        "a positional argument cannot follow a named argument",
                        argument.span(),
                    ));
                }
                None
            }
            PartialArgument::VariadicPlaceholder(_) => None,
        };

        if let Some((name, span)) = name {
            has_named = true;
            if seen.contains(&name) {
                return Err(CompileError::new(
                    CompileErrorKind::DuplicateNamedArgument,
                    format!("the named argument `{name}` is passed twice"),
                    span,
                ));
            }

            seen.push(name);
        }
    }

    Ok(())
}

impl BodyCompiler<'_, '_> {
    /// Compiles an expression callee without recursing through call chains.
    pub(in crate::compiler::emit::calls) fn callee_expression_value(
        &mut self,
        scope: &Scope<'_>,
        expression: &Expression<'_>,
    ) -> Result<Register, CompileError> {
        let start = expression.leftmost_span();
        let mut spine = Vec::new();
        let mut current = expression;
        let foot = loop {
            let Expression::Call(Call::Function(call)) = current else {
                break current;
            };

            let Callee::Expression(inner) = &call.function else {
                break current;
            };

            if call
                .argument_list
                .arguments
                .iter()
                .any(|argument| matches!(argument, Argument::Named(_)))
            {
                break current;
            }

            let span = start.join(call.argument_list.span());
            check_named_arguments(&call.argument_list)?;
            Self::check_callee_turbofish(scope, &call.function, call.type_arguments.as_ref())?;
            let destination = self.allocate(span)?;
            let mark = self.registers.mark();
            let count = argument_gate(call.argument_list.arguments.len(), span)?;
            spine.push((call, destination, mark, count, span));
            current = inner;
        };

        let mut accumulator = self.expression(scope, foot)?;
        for (call, destination, mark, count, span) in spine.into_iter().rev() {
            let callee = self.allocate(span)?;
            self.move_into(callee, accumulator, span);
            let callee =
                self.specialize_callee(scope, callee, call.type_arguments.as_ref(), span)?;

            let first = self.window(
                scope,
                call.argument_list.arguments.iter().map(Argument::value),
                call.argument_list.arguments.len(),
                span,
            )?;

            self.chunk.emit(
                Instruction::CallValue {
                    argument_count: Count::new(count),
                    destination,
                    callee,
                    first_argument: first,
                },
                span,
            );

            self.registers.release_to(mark);
            accumulator = destination;
        }

        Ok(accumulator)
    }

    /// Materializes a callable value.
    pub(in crate::compiler::emit::calls) fn materialize_callee(
        &mut self,
        scope: &Scope<'_>,
        callee: &Callee<'_>,
    ) -> Result<Register, CompileError> {
        match callee {
            Callee::Identifier(identifier) => {
                let text = scope.resolver.resolve_text(identifier);
                let constant = self.string_constant(text.as_bytes(), identifier.span())?;
                let destination = self.allocate(identifier.span())?;
                self.chunk.emit(
                    Instruction::LoadConstant {
                        destination,
                        constant,
                    },
                    identifier.span(),
                );

                Ok(destination)
            }
            Callee::Expression(expression) => self.callee_expression_value(scope, expression),
        }
    }

    /// Builds a callable from its source.
    pub(in crate::compiler::emit::calls) fn callee_value(
        &mut self,
        scope: &Scope<'_>,
        source: &CalleeSource<'_, '_>,
        span: Span,
    ) -> Result<Register, CompileError> {
        match source {
            CalleeSource::Function(callee) => self.materialize_callee(scope, callee),
            CalleeSource::Value(register) => Ok(*register),
            CalleeSource::Method { receiver, name } => {
                let destination = self.allocate(span)?;
                let first = self.allocate(span)?;
                let second = self.allocate(span)?;
                self.move_into(first, *receiver, span);
                let constant = self.string_constant(name.as_bytes(), span)?;
                self.chunk.emit(
                    Instruction::LoadConstant {
                        destination: second,
                        constant,
                    },
                    span,
                );

                self.chunk.emit(
                    Instruction::NewTuple {
                        element_count: Count::new(2),
                        destination,
                        first_element: first,
                    },
                    span,
                );

                Ok(destination)
            }
            CalleeSource::Static { class, name } => match class {
                ClassReference::Expression(expression) => {
                    let destination = self.allocate(span)?;
                    let class_value = self.expression(scope, expression)?;
                    let suffix_slot = self.allocate(span)?;
                    let constant = self.string_constant(format!("::{name}").as_bytes(), span)?;
                    self.chunk.emit(
                        Instruction::LoadConstant {
                            destination: suffix_slot,
                            constant,
                        },
                        span,
                    );

                    self.chunk.emit(
                        Instruction::Concatenate {
                            destination,
                            left: class_value,
                            right: suffix_slot,
                        },
                        span,
                    );

                    Ok(destination)
                }
                reference => {
                    let class = self.class_reference_atom(scope, reference)?;
                    let rendered = format!("{class}::{name}");
                    let constant = self.string_constant(rendered.as_bytes(), span)?;
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
            },
        }
    }

    /// Compiles a call with named arguments.
    pub(in crate::compiler::emit::calls) fn shaped_call(
        &mut self,
        scope: &Scope<'_>,
        source: &CalleeSource<'_, '_>,
        argument_list: &ArgumentList<'_>,
        span: Span,
        value_use: ValueUse,
    ) -> Result<Register, CompileError> {
        let destination = self.allocate(span)?;
        let mark = self.registers.mark();
        let callee = self.callee_value(scope, source, span)?;
        let callee_slot = self.allocate(span)?;
        self.move_into(callee_slot, callee, span);
        argument_gate(argument_list.arguments.len(), span)?;
        let mut counted = 0_usize;
        let mut named = Vec::new();
        let mut positional_slots = Vec::new();
        let mut named_slots = Vec::new();
        for argument in &argument_list.arguments {
            match argument {
                Argument::Positional(_) => counted += 1,
                Argument::Named(argument) => {
                    named.push(self.heap.intern(argument.name.value.as_bytes()));
                }
            }
        }

        let positional = argument_gate(counted, span)?;
        for _ in 0..usize::from(positional) {
            positional_slots.push(self.allocate(span)?);
        }

        for _ in 0..named.len() {
            named_slots.push(self.allocate(span)?);
        }

        let mut positional_cursor = 0;
        let mut named_cursor = 0;
        for argument in &argument_list.arguments {
            let inner = self.registers.mark();
            let value = self.expression(scope, argument.value())?;
            let slot = match argument {
                Argument::Positional(_) => {
                    positional_cursor += 1;
                    positional_slots[positional_cursor - 1]
                }
                Argument::Named(_) => {
                    named_cursor += 1;
                    named_slots[named_cursor - 1]
                }
            };

            self.move_argument_into(slot, value, inner, argument.span());
            self.registers.release_to(inner);
        }

        let descriptor = self.add_call_descriptor(CallDescriptor { positional, named }, span)?;
        self.chunk.emit(
            call_with_names_instruction(value_use, destination, callee_slot, descriptor),
            span,
        );

        self.registers.release_to(mark);
        Ok(destination)
    }

    fn partial_callee(
        &mut self,
        scope: &Scope<'_>,
        application: &PartialApplication<'_>,
    ) -> Result<Register, CompileError> {
        match application {
            PartialApplication::Function(application) => {
                if application.type_arguments.is_some() {
                    Self::check_callee_turbofish(
                        scope,
                        &application.function,
                        application.type_arguments.as_ref(),
                    )?;
                }
                self.materialize_callee(scope, &application.function)
            }
            PartialApplication::Method(application) => {
                let receiver = self.expression(scope, application.object)?;
                self.callee_value(
                    scope,
                    &CalleeSource::Method {
                        receiver,
                        name: application.method.value,
                    },
                    application.span(),
                )
            }
            PartialApplication::StaticMethod(application) => {
                if application.type_arguments.is_some() {
                    Self::check_method_turbofish(
                        scope,
                        &application.class,
                        application.method.value,
                        application.type_arguments.as_ref(),
                    )?;
                }
                self.callee_value(
                    scope,
                    &CalleeSource::Static {
                        class: &application.class,
                        name: application.method.value,
                    },
                    application.span(),
                )
            }
        }
    }

    fn partial_plan<'arena>(
        &self,
        argument_list: &PartialArgumentList<'arena>,
    ) -> PartialPlan<'arena> {
        let mut slots = Vec::new();
        let mut given = Vec::new();
        let mut open_remaining = false;
        for argument in &argument_list.arguments {
            match argument {
                PartialArgument::Positional(argument) => {
                    slots.push(PresetSlot::GivenPositional);
                    given.push(argument.value);
                }
                PartialArgument::Named(argument) => {
                    slots.push(PresetSlot::GivenNamed(
                        self.heap.intern(argument.name.value.as_bytes()),
                    ));
                    given.push(argument.value);
                }
                PartialArgument::Placeholder(_) => slots.push(PresetSlot::HolePositional),
                PartialArgument::NamedPlaceholder(argument) => slots.push(PresetSlot::HoleNamed(
                    self.heap.intern(argument.name.value.as_bytes()),
                )),
                PartialArgument::VariadicPlaceholder(_) => open_remaining = true,
            }
        }
        PartialPlan {
            slots,
            given,
            open_remaining,
        }
    }

    fn compile_partial_arguments(
        &mut self,
        scope: &Scope<'_>,
        arguments: &[&Expression<'_>],
        span: Span,
    ) -> Result<(), CompileError> {
        let mut slots = Vec::new();
        for _ in arguments {
            slots.push(self.allocate(span)?);
        }
        for (slot, argument) in slots.into_iter().zip(arguments) {
            let inner = self.registers.mark();
            let register = self.expression(scope, argument)?;
            self.move_argument_into(slot, register, inner, argument.span());
            self.registers.release_to(inner);
        }

        Ok(())
    }

    /// Compiles a first-class callable or partial application.
    pub(in crate::compiler::emit) fn partial_application(
        &mut self,
        scope: &Scope<'_>,
        application: &PartialApplication<'_>,
    ) -> Result<Register, CompileError> {
        let argument_list = application.get_argument_list();
        check_partial_named_arguments(argument_list)?;
        check_sequence(
            CompileErrorKind::TooManyArguments,
            "a partial application may pass",
            "arguments",
            argument_list.arguments.as_slice(),
        )?;

        let destination = self.allocate(application.span())?;
        let mark = self.registers.mark();
        let type_argument_list = match application {
            PartialApplication::Function(application) => application.type_arguments.as_ref(),
            PartialApplication::Method(application) => application.type_arguments.as_ref(),
            PartialApplication::StaticMethod(application) => application.type_arguments.as_ref(),
        };

        let type_arguments = self.lower_turbofish(scope, type_argument_list)?;
        let callee = self.partial_callee(scope, application)?;
        let callee_slot = self.allocate(application.span())?;
        self.move_into(callee_slot, callee, application.span());
        let plan = self.partial_plan(argument_list);
        self.compile_partial_arguments(scope, &plan.given, application.span())?;
        let descriptor = self.add_preset_descriptor(
            PresetDescriptor {
                slots: plan.slots,
                open_remaining: plan.open_remaining,
                type_arguments,
            },
            application.span(),
        )?;

        self.chunk.emit(
            Instruction::MakeBound {
                destination,
                callee: callee_slot,
                descriptor,
            },
            application.span(),
        );

        self.registers.release_to(mark);
        Ok(destination)
    }

    fn instantiate_class(
        &mut self,
        scope: &Scope<'_>,
        instantiation: &Instantiation<'_>,
        destination: Register,
    ) -> Result<(), CompileError> {
        let span = instantiation.span();
        let reference = &instantiation.class;
        if let ClassReference::Expression(expression) = reference {
            let class_name = self.expression(scope, expression)?;
            self.chunk.emit(
                Instruction::NewDynamic {
                    destination,
                    class_name,
                },
                span,
            );
            return Ok(());
        }
        if let ClassReference::Named(named) = reference
            && self.types(scope).is_binder(&named.identifier)
        {
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
            let descriptor = self.add_type_descriptor(
                TypeDescriptor::Parameter(self.heap.intern(named.identifier.value().as_bytes())),
                named.span(),
            )?;
            self.chunk.emit(
                Instruction::NewTyped {
                    destination,
                    descriptor,
                },
                span,
            );
            return Ok(());
        }
        if let ClassReference::Named(named) = reference {
            let resolved = scope.resolver.resolve_text(&named.identifier);
            check_call_type_argument_arity(
                scope.generics,
                &resolved,
                named.type_arguments.as_ref(),
                named.span(),
            )?;
        }
        let class = self.class_reference_atom(scope, reference)?;
        let type_arguments = match reference {
            ClassReference::Named(named) => {
                self.lower_turbofish(scope, named.type_arguments.as_ref())?
            }
            ClassReference::Self_(_) | ClassReference::Parent(_) | ClassReference::Static(_) => {
                Some(Vec::new())
            }
            ClassReference::Expression(_) => None,
        };
        let cache = self.add_ic_descriptor(
            IcDescriptor::Member {
                name: class,
                type_arguments,
            },
            span,
        )?;
        self.chunk
            .emit(Instruction::NewStatic { destination, cache }, span);
        Ok(())
    }

    fn call_constructor(
        &mut self,
        scope: &Scope<'_>,
        destination: Register,
        argument_list: &ArgumentList<'_>,
        span: Span,
    ) -> Result<(), CompileError> {
        check_named_arguments(argument_list)?;
        let count = argument_gate(argument_list.arguments.len(), span)?;
        if argument_list
            .arguments
            .iter()
            .any(|argument| !argument.is_positional())
        {
            self.shaped_call(
                scope,
                &CalleeSource::Method {
                    receiver: destination,
                    name: "__construct",
                },
                argument_list,
                span,
                ValueUse::Needed,
            )?;
            return Ok(());
        }
        let window_first = self.allocate(span)?;
        self.move_into(window_first, destination, span);
        let mut slots = Vec::new();
        for _ in 0..argument_list.arguments.len() {
            slots.push(self.allocate(span)?);
        }
        for (slot, argument) in slots.into_iter().zip(&argument_list.arguments) {
            let inner = self.registers.mark();
            let value = self.expression(scope, argument.value())?;
            self.move_argument_into(slot, value, inner, argument.span());
            self.registers.release_to(inner);
        }
        let cache = self.add_ic_descriptor(
            IcDescriptor::Member {
                name: self.heap.intern(b"__construct"),
                type_arguments: None,
            },
            span,
        )?;
        let scratch = self.allocate(span)?;
        self.chunk.emit(
            Instruction::CallMethod {
                argument_count: Count::new(count + 1),
                destination: scratch,
                first_argument: window_first,
                cache,
            },
            span,
        );
        Ok(())
    }

    /// Compiles a `new` expression and its constructor call.
    pub(in crate::compiler::emit) fn instantiation(
        &mut self,
        scope: &Scope<'_>,
        instantiation: &Instantiation<'_>,
    ) -> Result<Register, CompileError> {
        let destination = self.allocate(instantiation.span())?;
        self.instantiate_class(scope, instantiation, destination)?;
        let mark = self.registers.mark();
        let empty = ArgumentList {
            left_parenthesis: instantiation.span(),
            arguments: TokenSeparatedSequence::empty(),
            right_parenthesis: instantiation.span(),
        };
        let argument_list = instantiation.argument_list.as_ref().unwrap_or(&empty);
        self.call_constructor(scope, destination, argument_list, instantiation.span())?;
        self.registers.release_to(mark);
        Ok(destination)
    }
}
