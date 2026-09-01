//! Function and static-method call compilation.

use whim_syn::cst::atom::Identifier;
use whim_syn::cst::call::FunctionCall;
use whim_syn::cst::call::StaticMethodCall;

use crate::compiler::emit::calls::Argument;
use crate::compiler::emit::calls::ArgumentList;
use crate::compiler::emit::calls::BodyCompiler;
use crate::compiler::emit::calls::Call;
use crate::compiler::emit::calls::Callee;
use crate::compiler::emit::calls::CalleeSource;
use crate::compiler::emit::calls::ClassReference;
use crate::compiler::emit::calls::CompileError;
use crate::compiler::emit::calls::Count;
use crate::compiler::emit::calls::Expression;
use crate::compiler::emit::calls::HasSpan;
use crate::compiler::emit::calls::IcDescriptor;
use crate::compiler::emit::calls::Register;
use crate::compiler::emit::calls::Scope;
use crate::compiler::emit::calls::TypeDescriptor;
use crate::compiler::emit::calls::ValueUse;
use crate::compiler::emit::calls::argument_gate;
use crate::compiler::emit::calls::call_named_instruction;
use crate::compiler::emit::calls::call_static_instruction;
use crate::compiler::emit::calls::call_value_instruction;
use crate::compiler::emit::calls::check_named_arguments;

fn has_named_arguments(arguments: &ArgumentList<'_>) -> bool {
    arguments
        .arguments
        .iter()
        .any(|argument| !argument.is_positional())
}

impl BodyCompiler<'_, '_> {
    fn direct_function_call(
        &mut self,
        scope: &Scope<'_>,
        call: &FunctionCall<'_>,
        callee: &Identifier<'_>,
        value_use: ValueUse,
    ) -> Result<Register, CompileError> {
        let span = call.span();
        let destination = self.allocate(span)?;
        let mark = self.registers.mark();
        let count = argument_gate(call.argument_list.arguments.len(), span)?;
        let first = self.window(
            scope,
            call.argument_list.arguments.iter().map(Argument::value),
            call.argument_list.arguments.len(),
            span,
        )?;
        let type_arguments = if call.type_arguments.is_some() {
            self.lower_turbofish(scope, call.type_arguments.as_ref())?
        } else {
            None
        };
        let cache = self.add_ic_descriptor(
            IcDescriptor::Member {
                name: scope.resolver.resolve(self.heap, callee),
                type_arguments,
            },
            span,
        )?;
        self.chunk.emit(
            call_named_instruction(value_use, Count::new(count), destination, first, cache),
            span,
        );
        self.registers.release_to(mark);
        Ok(destination)
    }

    fn specialized_function_call(
        &mut self,
        scope: &Scope<'_>,
        call: &FunctionCall<'_>,
        named: bool,
        value_use: ValueUse,
    ) -> Result<Register, CompileError> {
        let span = call.span();
        let callee = self.materialize_callee(scope, &call.function)?;
        let callee = self.specialize_callee(scope, callee, call.type_arguments.as_ref(), span)?;
        if named {
            return self.shaped_call(
                scope,
                &CalleeSource::Value(callee),
                &call.argument_list,
                span,
                value_use,
            );
        }
        let destination = self.allocate(span)?;
        let mark = self.registers.mark();
        let count = argument_gate(call.argument_list.arguments.len(), span)?;
        let first = self.window(
            scope,
            call.argument_list.arguments.iter().map(Argument::value),
            call.argument_list.arguments.len(),
            span,
        )?;
        self.chunk.emit(
            call_value_instruction(value_use, Count::new(count), destination, callee, first),
            span,
        );
        self.registers.release_to(mark);
        Ok(destination)
    }

    fn expression_function_call(
        &mut self,
        scope: &Scope<'_>,
        call: &FunctionCall<'_>,
        expression: &Expression<'_>,
        value_use: ValueUse,
    ) -> Result<Register, CompileError> {
        let span = call.span();
        let destination = self.allocate(span)?;
        let mark = self.registers.mark();
        let count = argument_gate(call.argument_list.arguments.len(), span)?;
        let callee_value = self.callee_expression_value(scope, expression)?;
        let callee = self.allocate(span)?;
        self.move_into(callee, callee_value, span);
        let first = self.window(
            scope,
            call.argument_list.arguments.iter().map(Argument::value),
            call.argument_list.arguments.len(),
            span,
        )?;
        self.chunk.emit(
            call_value_instruction(value_use, Count::new(count), destination, callee, first),
            span,
        );
        self.registers.release_to(mark);
        Ok(destination)
    }

    fn function_call(
        &mut self,
        scope: &Scope<'_>,
        call: &FunctionCall<'_>,
        value_use: ValueUse,
    ) -> Result<Register, CompileError> {
        check_named_arguments(&call.argument_list)?;
        Self::check_callee_turbofish(scope, &call.function, call.type_arguments.as_ref())?;
        let named = has_named_arguments(&call.argument_list);
        if named {
            if call.type_arguments.is_some() {
                return self.specialized_function_call(scope, call, true, value_use);
            }
            return self.shaped_call(
                scope,
                &CalleeSource::Function(&call.function),
                &call.argument_list,
                call.span(),
                value_use,
            );
        }
        match &call.function {
            Callee::Identifier(identifier) => {
                self.direct_function_call(scope, call, identifier, value_use)
            }
            Callee::Expression(_) if call.type_arguments.is_some() => {
                self.specialized_function_call(scope, call, false, value_use)
            }
            Callee::Expression(expression) => {
                self.expression_function_call(scope, call, expression, value_use)
            }
        }
    }

    fn specialized_static_call(
        &mut self,
        scope: &Scope<'_>,
        call: &StaticMethodCall<'_>,
        value_use: ValueUse,
    ) -> Result<Register, CompileError> {
        let callee = self.callee_value(
            scope,
            &CalleeSource::Static {
                class: &call.class,
                name: call.method.value,
            },
            call.span(),
        )?;
        let callee =
            self.specialize_callee(scope, callee, call.type_arguments.as_ref(), call.span())?;
        self.shaped_call(
            scope,
            &CalleeSource::Value(callee),
            &call.argument_list,
            call.span(),
            value_use,
        )
    }

    fn dynamic_static_call(
        &mut self,
        scope: &Scope<'_>,
        call: &StaticMethodCall<'_>,
        value_use: ValueUse,
    ) -> Result<Register, CompileError> {
        let span = call.span();
        let destination = self.allocate(span)?;
        let mark = self.registers.mark();
        let callee = self.callee_value(
            scope,
            &CalleeSource::Static {
                class: &call.class,
                name: call.method.value,
            },
            span,
        )?;
        let count = argument_gate(call.argument_list.arguments.len(), span)?;
        let first = self.window(
            scope,
            call.argument_list.arguments.iter().map(Argument::value),
            call.argument_list.arguments.len(),
            span,
        )?;
        self.chunk.emit(
            call_value_instruction(value_use, Count::new(count), destination, callee, first),
            span,
        );
        self.registers.release_to(mark);
        Ok(destination)
    }

    fn direct_static_call(
        &mut self,
        scope: &Scope<'_>,
        call: &StaticMethodCall<'_>,
        type_arguments: Option<Vec<TypeDescriptor>>,
        value_use: ValueUse,
    ) -> Result<Register, CompileError> {
        let span = call.span();
        let class = self.static_call_class_atom(scope, &call.class)?;
        let destination = self.allocate(span)?;
        let mark = self.registers.mark();
        let count = argument_gate(call.argument_list.arguments.len(), span)?;
        let first = self.window(
            scope,
            call.argument_list.arguments.iter().map(Argument::value),
            call.argument_list.arguments.len(),
            span,
        )?;
        let cache = self.add_ic_descriptor(
            IcDescriptor::ClassMember {
                class,
                member: self.heap.intern(call.method.value.as_bytes()),
                type_arguments,
            },
            span,
        )?;
        self.chunk.emit(
            call_static_instruction(value_use, Count::new(count), destination, first, cache),
            span,
        );
        self.registers.release_to(mark);
        Ok(destination)
    }

    fn static_method_call(
        &mut self,
        scope: &Scope<'_>,
        call: &StaticMethodCall<'_>,
        value_use: ValueUse,
    ) -> Result<Register, CompileError> {
        check_named_arguments(&call.argument_list)?;
        Self::check_method_turbofish(
            scope,
            &call.class,
            call.method.value,
            call.type_arguments.as_ref(),
        )?;
        let named = has_named_arguments(&call.argument_list);
        let type_arguments = self.lower_turbofish(scope, call.type_arguments.as_ref())?;
        if call.type_arguments.is_some() && named {
            return self.specialized_static_call(scope, call, value_use);
        }
        if named {
            return self.shaped_call(
                scope,
                &CalleeSource::Static {
                    class: &call.class,
                    name: call.method.value,
                },
                &call.argument_list,
                call.span(),
                value_use,
            );
        }
        if matches!(&call.class, ClassReference::Expression(_)) {
            return self.dynamic_static_call(scope, call, value_use);
        }
        self.direct_static_call(scope, call, type_arguments, value_use)
    }

    pub(in crate::compiler::emit) fn call(
        &mut self,
        scope: &Scope<'_>,
        call: &Call<'_>,
        value_use: ValueUse,
    ) -> Result<Register, CompileError> {
        match call {
            Call::Function(call) => self.function_call(scope, call, value_use),
            Call::StaticMethod(call) => self.static_method_call(scope, call, value_use),
            Call::Method(_) | Call::NullSafeMethod(_) => {
                self.chain_root_with_use(scope, &Expression::Call(call.clone()), value_use)
            }
        }
    }
}
