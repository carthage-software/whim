//! The internal boundary between Rust-backed core operations and the VM.

#![deny(clippy::nursery, clippy::pedantic)]
#![expect(
    clippy::inline_always,
    reason = "small built-in boundary helpers must disappear into their handlers"
)]
#![expect(
    clippy::redundant_pub_crate,
    reason = "built-in declarations are shared across sibling runtime modules"
)]

use core::any;
use std::iter;

pub(crate) mod arguments;
pub(crate) mod convert;
pub(crate) mod coroutines;
pub(crate) mod spec;
pub(crate) mod throw;

mod context;

use crate::builtin::convert::state_ref;
use crate::builtin::convert::wrong_built_in_state;
use crate::builtin::spec::FunctionSpec;
use crate::builtin::throw::Throw;
use crate::unwrap_option_invariant;
use crate::value::Value;
use crate::value::ValueView;
use crate::value::function::CallTarget;
use crate::value::function::FunctionObject;
use crate::value::object::ClassId;
use crate::value::object::TypeEnvironmentId;
use crate::vm::VirtualMachine;

#[inline(always)]
pub(crate) fn invoke_direct_built_in<'call, 'vm, 'engine, F>(
    vm: &'vm mut VirtualMachine<'engine>,
    window: &'call [Value],
    body: F,
) -> Result<Value, Throw>
where
    F: FnOnce(&mut Context<'call, 'vm, 'engine>, &'call [Value]) -> Result<Value, Throw>,
{
    let mut context = Context::over_called(vm, None, TypeEnvironmentId::default());
    body(&mut context, window)
}

pub(crate) struct Context<'call, 'vm, 'engine: 'vm> {
    pub(crate) vm: &'vm mut VirtualMachine<'engine>,
    called_class: Option<ClassId>,
    pub(crate) type_environment: TypeEnvironmentId,
    receiver: Option<Value>,
    captures: Option<&'call [Value]>,
}

impl<'call, 'vm, 'engine: 'vm> Context<'call, 'vm, 'engine> {
    pub(crate) const fn over_called(
        vm: &'vm mut VirtualMachine<'engine>,
        called_class: Option<ClassId>,
        type_environment: TypeEnvironmentId,
    ) -> Self {
        Self {
            vm,
            called_class,
            type_environment,
            receiver: None,
            captures: None,
        }
    }

    #[must_use]
    pub(crate) const fn called_class(&self) -> Option<ClassId> {
        self.called_class
    }

    pub(crate) fn set_receiver(&mut self, receiver: Value) {
        self.receiver = Some(receiver);
    }

    #[must_use]
    pub(crate) fn receiver(&self) -> Value {
        // SAFETY: an instance method's shim always installs the receiver before
        // invoking the handler, and only instance methods reach `receiver`.
        let receiver = unsafe {
            unwrap_option_invariant(
                self.receiver.as_ref(),
                "an instance method installs its receiver before running",
            )
        };

        receiver.clone()
    }

    pub(crate) const fn set_captures(&mut self, captures: &'call [Value]) {
        self.captures = Some(captures);
    }

    #[must_use]
    pub(crate) fn capture(&self, index: usize) -> Value {
        // SAFETY: a closure shim installs its captures before invoking the
        // handler, and a handler only reads captures it declared.
        let capture = unsafe {
            unwrap_option_invariant(
                self.captures.and_then(|captures| captures.get(index)),
                "a closure reads a capture it installed",
            )
        };

        capture.clone()
    }

    pub(crate) fn closure(&mut self, spec: FunctionSpec, captures: &[Value]) -> Value {
        let signature = self.vm.heap().intern(spec.signature.as_bytes());
        let target = self.vm.engine.intern_built_in_function(spec);
        let function = FunctionObject::partial(
            self.vm.heap(),
            CallTarget::BuiltIn(target),
            None,
            captures.iter().cloned(),
            iter::empty(),
            signature,
            None,
            None,
            self.type_environment,
            false,
        );
        Value::function(function)
    }

    pub(crate) fn state<T: 'static>(&mut self) -> Result<&T, Throw> {
        let present = self
            .receiver
            .as_ref()
            .is_some_and(|receiver| state_ref::<T>(receiver).is_some());
        if !present {
            return Err(wrong_built_in_state(self, any::type_name::<T>()));
        }

        // SAFETY: presence was just confirmed; the receiver borrow above is
        // released before this reborrow.
        let receiver = self.receiver.as_ref();
        // SAFETY: the surrounding invariant proves this option contains a value.
        Ok(unsafe {
            unwrap_option_invariant(
                receiver.and_then(state_ref::<T>),
                "built-in state presence was confirmed",
            )
        })
    }

    pub(crate) fn get_property(&mut self, object: &Value, name: &str) -> Result<Value, Throw> {
        let Some(instance) = object.as_object().cloned() else {
            return Err(self.type_error("cannot read a property of a non-object"));
        };
        let name = self.vm.intern(name.as_bytes());
        let scope = self.called_class;

        match self.vm.built_in_read_property(&instance, &name, scope) {
            Ok(value) => Ok(value),
            Err(control) => Err(self.vm.control_to_throw(control)),
        }
    }

    pub(crate) fn set_property(
        &mut self,
        object: &Value,
        name: &str,
        value: Value,
    ) -> Result<(), Throw> {
        let Some(instance) = object.as_object().cloned() else {
            return Err(self.type_error("cannot write a property of a non-object"));
        };
        let name = self.vm.intern(name.as_bytes());
        let scope = self.called_class;

        match self
            .vm
            .built_in_write_property(&instance, &name, value, scope)
        {
            Ok(()) => Ok(()),
            Err(control) => Err(self.vm.control_to_throw(control)),
        }
    }

    fn resolve_named_class(&mut self, name: &str) -> Result<ClassId, Throw> {
        let atom = self.vm.intern(name.as_bytes());
        let Some(class) = self.vm.resolve_class(atom) else {
            let error = self.vm.intern(b"Whim\\Unwind\\TypeError");
            return Err(self
                .vm
                .throw(error, &format!("the class {name} is not defined"), 0));
        };

        Ok(class)
    }

    /// Materializes every singleton case of `class`, in declaration order.
    pub(crate) fn enum_cases(&mut self, class: ClassId) -> Value {
        let count = self.vm.engine.tables.classes[class.0 as usize]
            .enum_cases
            .len();
        let mut values = Vec::with_capacity(count);
        for index in 0..count {
            let name = self.vm.engine.tables.classes[class.0 as usize].enum_cases[index]
                .name
                .clone();
            // SAFETY: the surrounding invariant proves this option contains a value.
            let value = unsafe {
                unwrap_option_invariant(
                    self.vm.enum_case_value(class, name),
                    "a declared enum case resolves to its singleton value",
                )
            };
            values.push(value);
        }

        self.vec(values)
    }

    pub(crate) fn enum_case_from_backing(&self, class: ClassId, backing: &Value) -> Option<Value> {
        let class_id = class;
        let wanted = backing;
        let name = self.vm.engine.tables.classes[class_id.0 as usize]
            .enum_cases
            .iter()
            .find(|case| match (&case.backing, wanted) {
                (Some(left), right)
                    if matches!(
                        (left.transparent(), right.transparent()),
                        (ValueView::Int(left), ValueView::Int(right)) if left == right
                    ) =>
                {
                    true
                }
                (Some(left), right) if left.is_string() && right.is_string() => {
                    left.as_string_bytes() == right.as_string_bytes()
                }
                _ => false,
            })?
            .name
            .clone();
        self.vm.enum_case_value(class_id, name)
    }
}
