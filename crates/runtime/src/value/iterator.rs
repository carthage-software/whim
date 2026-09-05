//! The engine-internal iteration cursor behind `foreach`.

use std::cell::Cell;
use std::ptr::NonNull;

use crate::unwrap_option_invariant;
use crate::value::function::FuncId;
use crate::value::heap::Heap;
use crate::value::heap::handle::ManagedRef;
use crate::value::heap::metadata::HeapBox;
use crate::value::heap::metadata::TeardownMode;
use crate::value::heap::metadata::Trace;
use crate::value::heap::metadata::TraceVisitor;
use crate::value::heap::metadata::TypeTag;
use crate::value::heap::queue::DropQueue;
use crate::value::object::ClassId;
use crate::value::object::InstanceObject;
use crate::value::object::TypeEnvironmentId;

pub(crate) struct IteratorObject {
    instance: Option<ManagedRef<InstanceObject>>,
    next: Option<(FuncId, ClassId)>,
    next_environment: TypeEnvironmentId,
    object_step_pending: Cell<bool>,
}

impl IteratorObject {
    #[must_use]
    pub(crate) fn new_object(
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
    pub(crate) fn instance(&self) -> &ManagedRef<InstanceObject> {
        // SAFETY: the surrounding invariant proves this option contains a value.
        unsafe {
            unwrap_option_invariant(
                self.instance.as_ref(),
                "a live object iterator retains its instance",
            )
        }
    }

    #[must_use]
    pub(crate) const fn next_method(&self) -> Option<(FuncId, ClassId)> {
        self.next
    }

    #[must_use]
    pub(crate) const fn next_environment(&self) -> TypeEnvironmentId {
        self.next_environment
    }

    pub(crate) fn begin_object_step(&self) {
        debug_assert!(!self.object_step_pending.get());
        self.object_step_pending.set(true);
    }

    #[must_use]
    pub(crate) const fn take_pending_object_step(&self) -> bool {
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
