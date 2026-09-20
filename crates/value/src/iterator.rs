//! The engine-internal iteration cursor behind `foreach`.

use std::cell::Cell;
use std::ptr::NonNull;

use whim_base::unwrap_option_invariant;

use crate::function::FuncId;
use crate::heap::Heap;
use crate::heap::handle::ManagedRef;
use crate::heap::metadata::HeapBox;
use crate::heap::metadata::TeardownMode;
use crate::heap::metadata::Trace;
use crate::heap::metadata::TraceVisitor;
use crate::heap::metadata::TypeTag;
use crate::heap::queue::DropQueue;
use crate::object::ClassId;
use crate::object::InstanceObject;
use crate::object::TypeEnvironmentId;

pub struct IteratorObject {
    instance: Option<ManagedRef<InstanceObject>>,
    next: Option<(FuncId, ClassId)>,
    next_environment: TypeEnvironmentId,
    object_step_pending: Cell<bool>,
}

impl IteratorObject {
    #[must_use]
    pub fn new_object(
        heap: &Heap,
        instance: ManagedRef<InstanceObject>,
        next: Option<(FuncId, ClassId)>,
        next_environment: TypeEnvironmentId,
    ) -> ManagedRef<Self> {
        ManagedRef::new_in(
            heap,
            Self {
                instance: Some(instance),
                next,
                next_environment,
                object_step_pending: Cell::new(false),
            },
        )
    }

    #[must_use]
    pub fn instance(&self) -> &ManagedRef<InstanceObject> {
        // SAFETY: the surrounding invariant proves this option contains a value.
        unsafe {
            unwrap_option_invariant(
                self.instance.as_ref(),
                "a live object iterator retains its instance",
            )
        }
    }

    #[must_use]
    pub const fn next_method(&self) -> Option<(FuncId, ClassId)> {
        self.next
    }

    #[must_use]
    pub const fn next_environment(&self) -> TypeEnvironmentId {
        self.next_environment
    }

    pub fn begin_object_step(&self) {
        debug_assert!(!self.object_step_pending.get());
        self.object_step_pending.set(true);
    }

    #[must_use]
    pub const fn take_pending_object_step(&self) -> bool {
        self.object_step_pending.replace(false)
    }
}

impl Trace for IteratorObject {
    fn type_tag() -> TypeTag {
        TypeTag::Iterator
    }

    fn enqueue_children(
        &mut self,
        _allocation: NonNull<HeapBox<()>>,
        queue: &DropQueue,
        mode: TeardownMode,
    ) {
        if let Some(instance) = self.instance.take() {
            queue.release_child(instance, mode);
        }
    }

    fn visit_children(&self, _allocation: NonNull<HeapBox<()>>, visitor: &mut TraceVisitor<'_>) {
        if let Some(instance) = &self.instance {
            visitor.visit(instance.erased());
        }
    }
}
