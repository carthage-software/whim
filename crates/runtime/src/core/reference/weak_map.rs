//! `Whim\Reference\WeakMap`: strong values behind weak keys.

use std::cell::RefCell;

use whim_macros::whim_class;
use whim_macros::whim_methods;

use crate::builtin::Context;
use crate::builtin::arguments::Arguments;
use crate::builtin::convert::BuiltInChildren;
use crate::builtin::throw::Throw;
use crate::unwrap_option_invariant;
use crate::unwrap_result_invariant;
use crate::value::Value;
use crate::value::heap::handle::ManagedRef;
use crate::value::heap::metadata::TeardownMode;
use crate::value::heap::metadata::TraceVisitor;
use crate::value::heap::queue::DropQueue;
use crate::value::weak::WeakMapObject;

#[whim_class("Whim\\Reference\\WeakMap<K: object, V>", final, traced)]
#[derive(Default)]
struct WeakMap {
    map: RefCell<Option<ManagedRef<WeakMapObject>>>,
}

default_built_in_state!(WeakMap);

// SAFETY: `map` is the sole owned child and teardown clears it.
unsafe impl BuiltInChildren for WeakMap {
    fn enqueue_built_in_children(&mut self, queue: &DropQueue, mode: TeardownMode) {
        if let Some(map) = self.map.get_mut().take() {
            queue.release_child(map, mode);
        }
    }

    fn visit_built_in_children(&self, visitor: &mut TraceVisitor<'_>) {
        if let Some(child) = self
            .map
            .borrow()
            .as_ref()
            .and_then(ManagedRef::collectable_box)
        {
            visitor.visit(child);
        }
    }
}

#[whim_methods(generics = "<K: object, V>")]
impl WeakMap {
    #[whim_method("__construct(): void", no_track_caller)]
    fn construct(cx: &mut Context<'_, '_, '_>) -> Result<Value, Throw> {
        let map = WeakMapObject::new(cx.vm.heap());
        *cx.state::<Self>()?.map.borrow_mut() = Some(map);

        Ok(Value::null())
    }

    #[whim_method("set(K $key, V $value): void", no_track_caller)]
    fn set(cx: &mut Context<'_, '_, '_>, arguments: Arguments<'_>) -> Result<Value, Throw> {
        let key = arguments.instance(0);
        let value = arguments.local(1);
        let map = storage(cx)?;
        drop(WeakMapObject::set(&map, &key, value));

        Ok(Value::null())
    }

    #[whim_method("get(K $key): V", must_use)]
    fn get(cx: &mut Context<'_, '_, '_>, arguments: Arguments<'_>) -> Result<Value, Throw> {
        let key = arguments.instance(0);
        let map = storage(cx)?;
        if let Some(value) = map.get(&key) {
            return Ok(value);
        }

        let class = cx.vm.intern(b"Whim\\Unwind\\OutOfBoundsError");
        Err(cx
            .vm
            .throw(class, "the weak map has no entry for the key", 0))
    }

    #[whim_method("has(K $key): bool", no_track_caller, must_use)]
    fn has(cx: &mut Context<'_, '_, '_>, arguments: Arguments<'_>) -> Result<Value, Throw> {
        let key = arguments.instance(0);
        let map = storage(cx)?;

        Ok(Value::bool(map.has(&key)))
    }

    #[whim_method("remove(K $key): void")]
    fn remove(cx: &mut Context<'_, '_, '_>, arguments: Arguments<'_>) -> Result<Value, Throw> {
        let key = arguments.instance(0);
        let map = storage(cx)?;
        if WeakMapObject::remove(&map, &key).is_some() {
            return Ok(Value::null());
        }

        let class = cx.vm.intern(b"Whim\\Unwind\\OutOfBoundsError");
        Err(cx
            .vm
            .throw(class, "the weak map has no entry for the key", 0))
    }

    #[whim_method("length(): int", no_track_caller, must_use)]
    fn length(cx: &mut Context<'_, '_, '_>) -> Result<Value, Throw> {
        let map = storage(cx)?;
        // SAFETY: the surrounding invariant proves this result is successful.
        let length = unsafe {
            unwrap_result_invariant(
                i64::try_from(map.len()),
                "a weak map cannot exhaust the signed integer range",
            )
        };

        Ok(Value::int(length))
    }
}

/// Clones the storage to release the state borrow before reusing the context.
fn storage(cx: &mut Context<'_, '_, '_>) -> Result<ManagedRef<WeakMapObject>, Throw> {
    let map = cx.state::<WeakMap>()?.map.borrow().as_ref().cloned();
    // SAFETY: a live weak map always holds its storage.
    Ok(unsafe { unwrap_option_invariant(map, "a live weak map holds its storage") })
}
