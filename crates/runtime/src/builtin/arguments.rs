//! Typed argument extraction at the built-in boundary.

use std::ptr::NonNull;

use crate::unreachable_invariant;
use crate::value::Value;
use crate::value::ValueView;
use crate::value::dict::DictObject;
use crate::value::heap::Heap;
use crate::value::heap::handle::ManagedRef;
use crate::value::object::InstanceObject;
use crate::value::string::ByteStringObject;
use crate::value::vec::VecObject;

#[derive(Clone, Copy)]
pub(crate) struct Arguments<'call> {
    window: &'call [Value],
    heap: NonNull<Heap>,
}

impl<'call> Arguments<'call> {
    #[must_use]
    pub(crate) fn new(window: &'call [Value], heap: &Heap) -> Self {
        Self {
            window,
            heap: NonNull::from(heap),
        }
    }

    pub(crate) fn get(&self, index: usize) -> Option<&'call Value> {
        self.window.get(index)
    }

    #[must_use]
    pub(crate) fn is_absent(&self, index: usize) -> bool {
        self.optional_value(index).is_none()
    }

    #[inline(always)]
    pub(crate) fn local(&self, index: usize) -> Value {
        // SAFETY: the engine validates arity before entering a built-in handler.
        unsafe { self.window.get_unchecked(index).clone() }
    }

    /// # Safety
    ///
    /// `index` must be present.
    #[must_use]
    pub(crate) unsafe fn value_unchecked(&self, index: usize) -> &'call Value {
        // SAFETY: the surrounding invariant keeps this index in bounds.
        unsafe { self.window.get_unchecked(index) }
    }

    #[inline(always)]
    pub(crate) fn int(&self, index: usize) -> i64 {
        // SAFETY: the engine validates built-in arguments before dispatch.
        unsafe { self.int_unchecked(index) }
    }

    /// # Safety
    ///
    /// `index` must be present and contain an integer.
    #[must_use]
    pub(crate) unsafe fn int_unchecked(&self, index: usize) -> i64 {
        // SAFETY: the caller guarantees that this index contains an integer.
        unsafe { self.window.get_unchecked(index).as_int_unchecked() }
    }

    #[inline(always)]
    pub(crate) fn float(&self, index: usize) -> f64 {
        // SAFETY: the engine validates built-in arguments before dispatch.
        unsafe { self.float_unchecked(index) }
    }

    /// # Safety
    ///
    /// `index` must be present and contain a float.
    #[must_use]
    pub(crate) unsafe fn float_unchecked(&self, index: usize) -> f64 {
        // SAFETY: the surrounding invariant keeps this index in bounds.
        let value = unsafe { self.window.get_unchecked(index) }.transparent();
        let ValueView::Float(value) = value else {
            // SAFETY: the surrounding invariant makes this path unreachable.
            unsafe { unreachable_invariant("a validated float argument is a float") }
        };

        *value
    }

    #[inline(always)]
    pub(crate) fn bool(&self, index: usize) -> bool {
        // SAFETY: the engine validates built-in arguments before dispatch.
        unsafe { self.bool_unchecked(index) }
    }

    /// # Safety
    ///
    /// `index` must be present and contain a boolean.
    #[must_use]
    pub(crate) unsafe fn bool_unchecked(&self, index: usize) -> bool {
        // SAFETY: the surrounding invariant keeps this index in bounds.
        let value = unsafe { self.window.get_unchecked(index) }.transparent();
        let ValueView::Bool(value) = value else {
            // SAFETY: the surrounding invariant makes this path unreachable.
            unsafe { unreachable_invariant("a validated boolean argument is a boolean") }
        };

        *value
    }

    #[inline(always)]
    pub(crate) fn bytes(&self, index: usize) -> &'call [u8] {
        // SAFETY: the engine validates built-in arguments before dispatch.
        unsafe { self.bytes_unchecked(index) }
    }

    /// # Safety
    ///
    /// `index` must be present and contain a string.
    #[must_use]
    pub(crate) unsafe fn bytes_unchecked(&self, index: usize) -> &'call [u8] {
        // SAFETY: the caller guarantees that this index contains a string.
        let value = unsafe { self.window.get_unchecked(index) };
        // SAFETY: the value's tag proves this projection is valid.
        unsafe { value.as_string_bytes().unwrap_unchecked() }
    }

    #[inline(always)]
    pub(crate) fn string(&self, index: usize) -> ManagedRef<ByteStringObject> {
        // SAFETY: the engine validates built-in arguments before dispatch.
        let value = unsafe { self.window.get_unchecked(index) }.transparent();
        match value {
            ValueView::String(string) => string.clone(),
            ValueView::ShortString(string) => {
                // SAFETY: dispatch checked the arguments and receiver.
                let heap = unsafe { self.heap.as_ref() };
                ByteStringObject::from_bytes(heap, string.as_bytes())
            }
            // SAFETY: the surrounding invariant makes this path unreachable.
            _ => unsafe { unreachable_invariant("a validated string argument is a string") },
        }
    }

    #[inline(always)]
    pub(crate) fn vec(&self, index: usize) -> ManagedRef<VecObject> {
        // SAFETY: the engine validates built-in arguments before dispatch.
        match unsafe { self.window.get_unchecked(index) }.transparent() {
            ValueView::Vec(vec) => vec.clone(),
            // SAFETY: the surrounding invariant makes this path unreachable.
            _ => unsafe { unreachable_invariant("a validated vec argument is a vec") },
        }
    }

    #[inline(always)]
    pub(crate) fn dict(&self, index: usize) -> ManagedRef<DictObject> {
        // SAFETY: the engine validates built-in arguments before dispatch.
        match unsafe { self.window.get_unchecked(index) }.transparent() {
            ValueView::Dict(dict) => dict.clone(),
            // SAFETY: the surrounding invariant makes this path unreachable.
            _ => unsafe { unreachable_invariant("a validated dict argument is a dict") },
        }
    }

    #[inline(always)]
    pub(crate) fn instance(&self, index: usize) -> ManagedRef<InstanceObject> {
        // SAFETY: the engine validates built-in arguments before dispatch.
        match unsafe { self.window.get_unchecked(index) }.transparent() {
            ValueView::Object(instance) => instance.clone(),
            // SAFETY: the surrounding invariant makes this path unreachable.
            _ => unsafe { unreachable_invariant("a validated object argument is an object") },
        }
    }

    #[inline(always)]
    pub(crate) fn optional_int(&self, index: usize) -> Option<i64> {
        self.optional_value(index)
            .map(|value| match value.transparent() {
                ValueView::Int(value) => *value,
                // SAFETY: the surrounding invariant makes this path unreachable.
                _ => unsafe {
                    unreachable_invariant("a present validated optional integer is an integer")
                },
            })
    }

    #[inline(always)]
    pub(crate) fn optional_instance(&self, index: usize) -> Option<ManagedRef<InstanceObject>> {
        self.optional_value(index)
            .map(|value| match value.transparent() {
                ValueView::Object(instance) => instance.clone(),
                // SAFETY: the surrounding invariant makes this path unreachable.
                _ => unsafe {
                    unreachable_invariant("a present validated optional object is an object")
                },
            })
    }

    #[inline(always)]
    fn optional_value(&self, index: usize) -> Option<&'call Value> {
        self.get(index)
            .filter(|value| !value.is_uninitialized() && !value.is_null())
    }
}
