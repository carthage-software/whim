use crate::value::heap::metadata::TeardownMode;
use crate::value::heap::metadata::TraceVisitor;
use crate::value::heap::queue::DropQueue;

use crate::builtin::Context;
use crate::builtin::throw::Throw;
use crate::value::Value;

/// # Safety
///
/// Visit every owned collectable box once and no other box. Release that same
/// set once, clear its fields, and never allocate or call the engine.
pub(crate) unsafe trait BuiltInChildren {
    fn enqueue_built_in_children(&mut self, queue: &mut DropQueue, mode: TeardownMode);

    /// Visits the state's collectable boxes.
    fn visit_built_in_children(&self, visitor: &mut TraceVisitor<'_>);
}

pub(crate) fn wrong_built_in_state(context: &mut Context<'_, '_, '_>, state: &str) -> Throw {
    let type_error = context.vm.intern(b"Whim\\Unwind\\TypeError");
    context.vm.throw(
        type_error,
        &format!("the receiver does not carry the {state} built-in state"),
        0,
    )
}

/// Reads inline state by its checked type, not the receiver's class.
#[must_use]
pub(crate) fn state_ref<T: 'static>(receiver: &Value) -> Option<&T> {
    let instance = receiver.as_object()?;
    instance.built_in_ref::<T>()
}
