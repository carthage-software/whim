//! `Whim\Reference\Weak`: a single weak reference as a traced built-in state
//! class.

use std::cell::RefCell;

use whim_macros::whim_class;
use whim_macros::whim_methods;

use crate::builtin::Context;
use crate::builtin::arguments::Arguments;
use crate::builtin::convert::BuiltInChildren;
use crate::builtin::throw::Throw;
use crate::value::Value;
use crate::value::heap::handle::ManagedRef;
use crate::value::heap::metadata::TeardownMode;
use crate::value::heap::metadata::TraceVisitor;
use crate::value::heap::queue::DropQueue;
use crate::value::weak::WeakReference;

#[whim_class("Whim\\Reference\\Weak<T: object>", final, traced)]
#[derive(Default)]
struct Weak {
    reference: RefCell<Option<ManagedRef<WeakReference>>>,
}

default_built_in_state!(Weak);

// SAFETY: `reference` is the sole owned child and teardown clears it.
unsafe impl BuiltInChildren for Weak {
    fn enqueue_built_in_children(&mut self, queue: &mut DropQueue, mode: TeardownMode) {
        if let Some(reference) = self.reference.get_mut().take() {
            queue.release_child(reference, mode);
        }
    }

    fn visit_built_in_children(&self, _visitor: &mut TraceVisitor<'_>) {}
}

#[whim_methods(generics = "<T: object>")]
impl Weak {
    #[whim_method("__construct(T $referent): void", no_track_caller)]
    fn construct(
        context: &mut Context<'_, '_, '_>,
        arguments: Arguments<'_>,
    ) -> Result<Value, Throw> {
        let referent = arguments.instance(0);
        let reference = WeakReference::new(&referent);
        *context.state::<Self>()?.reference.borrow_mut() = Some(reference);

        Ok(Value::null())
    }

    #[whim_method("get(): T|null", no_track_caller, must_use)]
    fn get(context: &mut Context<'_, '_, '_>) -> Result<Value, Throw> {
        let upgraded = context
            .state::<Self>()?
            .reference
            .borrow()
            .as_ref()
            .and_then(|reference| reference.upgrade());

        match upgraded {
            Some(instance) => Ok(Value::object(instance)),
            None => Ok(Value::null()),
        }
    }
}
