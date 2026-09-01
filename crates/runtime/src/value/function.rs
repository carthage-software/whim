//! Function values: references, closures, bound methods, and partials.

use std::mem;
use std::ptr::NonNull;

use crate::value::Value;
use crate::value::atom::Atom;
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

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[repr(transparent)]
pub(crate) struct FuncId(pub u32);

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[repr(transparent)]
pub(crate) struct BuiltInId(pub u32);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum CallTarget {
    User(FuncId),
    BuiltIn(BuiltInId),
}

#[derive(Clone)]
pub(crate) enum PresetArg {
    Given(Value),
    Hole(u32),
}

pub(crate) struct FunctionObject {
    target: CallTarget,
    this: Option<ManagedRef<InstanceObject>>,
    captures: Vec<Value>,
    presets: Vec<PresetArg>,
    signature: Atom,
    scope: Option<ClassId>,
    called: Option<ClassId>,
    /// Lexical class arguments and, after specialization, callable arguments.
    type_environment: TypeEnvironmentId,
    /// Whether this callable's own type parameters have already been bound.
    type_arguments_bound: bool,
}

impl FunctionObject {
    #[must_use]
    pub(crate) fn closure(
        heap: &Heap,
        target: CallTarget,
        captures: impl IntoIterator<Item = Value>,
        signature: Atom,
        scope: Option<ClassId>,
        type_environment: TypeEnvironmentId,
    ) -> ManagedRef<Self> {
        let capture_storage: Vec<Value> = captures.into_iter().collect();
        ManagedRef::new_in(
            heap,
            Self {
                target,
                this: None,
                captures: capture_storage,
                presets: Vec::new(),
                signature,
                scope,
                called: None,
                type_environment,
                type_arguments_bound: false,
            },
        )
    }

    #[must_use]
    #[expect(
        clippy::too_many_arguments,
        reason = "a callable preserves every independent part of its call shape"
    )]
    pub(crate) fn partial(
        heap: &Heap,
        target: CallTarget,
        this: Option<ManagedRef<InstanceObject>>,
        captures: impl IntoIterator<Item = Value>,
        presets: impl IntoIterator<Item = PresetArg>,
        signature: Atom,
        scope: Option<ClassId>,
        called: Option<ClassId>,
        type_environment: TypeEnvironmentId,
        type_arguments_bound: bool,
    ) -> ManagedRef<Self> {
        let capture_storage: Vec<Value> = captures.into_iter().collect();
        let preset_storage: Vec<PresetArg> = presets.into_iter().collect();
        ManagedRef::new_in(
            heap,
            Self {
                target,
                this,
                captures: capture_storage,
                presets: preset_storage,
                signature,
                scope,
                called,
                type_environment,
                type_arguments_bound,
            },
        )
    }

    #[must_use]
    pub(crate) const fn scope(&self) -> Option<ClassId> {
        self.scope
    }

    #[must_use]
    pub(crate) const fn called(&self) -> Option<ClassId> {
        self.called
    }

    #[must_use]
    pub(crate) const fn target(&self) -> CallTarget {
        self.target
    }

    #[must_use]
    pub(crate) const fn this(&self) -> Option<&ManagedRef<InstanceObject>> {
        self.this.as_ref()
    }

    #[must_use]
    pub(crate) fn captures(&self) -> &[Value] {
        &self.captures
    }

    #[must_use]
    pub(crate) fn presets(&self) -> &[PresetArg] {
        &self.presets
    }

    #[must_use]
    pub(crate) const fn signature(&self) -> &Atom {
        &self.signature
    }

    #[must_use]
    pub(crate) const fn type_environment(&self) -> TypeEnvironmentId {
        self.type_environment
    }

    #[must_use]
    pub(crate) const fn type_arguments_bound(&self) -> bool {
        self.type_arguments_bound
    }
}

impl Trace for FunctionObject {
    fn type_tag() -> TypeTag {
        TypeTag::Function
    }

    fn enqueue_children(
        &mut self,
        _allocation: NonNull<HeapBox<()>>,
        queue: &mut DropQueue,
        mode: TeardownMode,
    ) {
        if let Some(this) = self.this.take() {
            queue.release_child(this, mode);
        }

        for value in mem::take(&mut self.captures) {
            queue.release_value(value, mode);
        }

        for preset in mem::take(&mut self.presets) {
            if let PresetArg::Given(value) = preset {
                queue.release_value(value, mode);
            }
        }
    }

    fn visit_children(&self, _allocation: NonNull<HeapBox<()>>, visitor: &mut TraceVisitor<'_>) {
        if let Some(this) = &self.this {
            visitor.visit(this.erased());
        }

        for value in &self.captures {
            if let Some(child) = value.collectable_box() {
                visitor.visit(child);
            }
        }

        for preset in &self.presets {
            if let PresetArg::Given(value) = preset
                && let Some(child) = value.collectable_box()
            {
                visitor.visit(child);
            }
        }
    }
}
