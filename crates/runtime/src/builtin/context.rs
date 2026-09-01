//! Value construction and engine operations used by Rust-backed core code.

use crate::builtin::Context;
use crate::builtin::throw::Throw;
use crate::core::classes::names;
use crate::unwrap_option_invariant;
use crate::value::Value;
use crate::value::atom::Atom;
use crate::value::dict::DictObject;
use crate::value::dict::keys::Key;
use crate::value::tuple::TupleObject;
use crate::value::vec::VecObject;

impl Context<'_, '_, '_> {
    #[must_use]
    #[inline(always)]
    pub(crate) fn string(&self, bytes: &[u8]) -> Value {
        Value::from_string_bytes(self.vm.heap(), bytes)
    }

    pub(crate) fn vec(&self, elements: impl IntoIterator<Item = Value>) -> Value {
        let vec = VecObject::with_elements(self.vm.heap(), elements);

        Value::vec(vec)
    }

    pub(crate) fn dict(&self, entries: impl IntoIterator<Item = (Value, Value)>) -> Value {
        let mut dict = DictObject::new(self.vm.heap());
        for (key, value) in entries {
            // SAFETY: the surrounding invariant proves this option contains a value.
            let key = unsafe {
                unwrap_option_invariant(
                    Key::from_owned_value(key),
                    "a built-in dict builder receives only array keys",
                )
            };
            dict.make_mut().insert(key, value);
        }

        Value::dict(dict)
    }

    /// An immutable tuple holding `elements` in order.
    #[must_use]
    pub(crate) fn tuple(
        &self,
        elements: impl IntoIterator<Item = Value, IntoIter: ExactSizeIterator>,
    ) -> Value {
        let tuple = TupleObject::with_elements(self.vm.heap(), elements);

        Value::tuple(tuple)
    }

    pub(crate) fn call_method(
        &mut self,
        receiver: &Value,
        name: Atom,
        arguments: &[Value],
    ) -> Result<Value, Throw> {
        let Some(object) = receiver.as_object() else {
            return Err(self.type_error("the receiver of a method call must be an object"));
        };
        let object = object.clone();
        self.vm.call_method(&object, name, arguments)
    }

    pub(crate) fn type_error(&mut self, message: &str) -> Throw {
        let class = self.vm.intern(names::TYPE_ERROR);
        self.vm.throw(class, message, 0)
    }
}
