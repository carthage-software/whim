//! Runtime values for the Whim virtual machine.

#![deny(clippy::nursery, clippy::pedantic)]
#![expect(
    clippy::inline_always,
    reason = "value tests and payload accessors are VM hot-path primitives"
)]
#![expect(
    clippy::option_if_let_else,
    reason = "the explicit short-string branch avoids closures on string construction"
)]
#![expect(
    clippy::redundant_pub_crate,
    reason = "the VM and compiler share the internal value representation"
)]

use std::hint;
use std::mem::ManuallyDrop;
use std::ptr;
use std::ptr::NonNull;

use crate::value::dict::DictObject;
use crate::value::function::FunctionObject;
use crate::value::heap::Heap;
use crate::value::heap::handle::ManagedRef;
use crate::value::heap::metadata::HeapBox;
use crate::value::iterator::IteratorObject;
use crate::value::newtype::NewtypeValueId;
use crate::value::object::InstanceObject;
use crate::value::string::ByteStringObject;
use crate::value::string::short::ShortString;
use crate::value::tuple::TupleObject;
use crate::value::vec::VecObject;

pub(crate) mod array;
pub(crate) mod atom;
pub(crate) mod dict;
pub(crate) mod function;
mod gc;
mod hash;
pub(crate) mod heap;
pub(crate) mod iterator;
pub(crate) mod newtype;
pub(crate) mod object;
pub(crate) mod ops;
pub(crate) mod string;
pub(crate) mod tuple;
pub(crate) mod vec;
pub(crate) mod weak;

const NO_NEWTYPE: u32 = u32::MAX;

#[derive(Clone, Copy, PartialEq, Eq)]
#[repr(u32)]
pub(crate) enum ValueKind {
    Uninitialized,
    Null,
    Bool,
    Int,
    Float,
    String,
    ShortString,
    Vec,
    Dict,
    Tuple,
    Function,
    Object,
    Iter,
}

union ValuePayload {
    raw: u64,
    boolean: bool,
    integer: i64,
    float: f64,
    string: ManuallyDrop<ManagedRef<ByteStringObject>>,
    short_string: ShortString,
    vec: ManuallyDrop<ManagedRef<VecObject>>,
    dict: ManuallyDrop<ManagedRef<DictObject>>,
    tuple: ManuallyDrop<ManagedRef<TupleObject>>,
    function: ManuallyDrop<ManagedRef<FunctionObject>>,
    object: ManuallyDrop<ManagedRef<InstanceObject>>,
    iterator: ManuallyDrop<ManagedRef<IteratorObject>>,
}

#[repr(C)]
pub(crate) struct Value {
    payload: ValuePayload,
    kind: ValueKind,
    newtype: u32,
}

const _: () = assert!(size_of::<Value>() == 16);

#[derive(Clone, Copy)]
pub(crate) enum ValueView<'a> {
    Uninitialized,
    Null,
    Bool(&'a bool),
    Int(&'a i64),
    Float(&'a f64),
    String(&'a ManagedRef<ByteStringObject>),
    ShortString(&'a ShortString),
    Vec(&'a ManagedRef<VecObject>),
    Dict(&'a ManagedRef<DictObject>),
    Tuple(&'a ManagedRef<TupleObject>),
    Function(&'a ManagedRef<FunctionObject>),
    Object(&'a ManagedRef<InstanceObject>),
    Iter(&'a ManagedRef<IteratorObject>),
}

impl ValueView<'_> {
    #[must_use]
    #[inline(always)]
    pub(crate) const fn is_string(&self) -> bool {
        matches!(self, Self::String(_) | Self::ShortString(_))
    }

    #[must_use]
    #[inline(always)]
    pub(crate) fn as_string_bytes(&self) -> Option<&[u8]> {
        match self {
            Self::String(value) => Some(ByteStringObject::handle_bytes(value)),
            Self::ShortString(value) => Some(value.as_bytes()),
            _ => None,
        }
    }

    #[must_use]
    pub(crate) const fn kind_name(&self) -> &'static str {
        match self {
            Self::Uninitialized => "uninitialized",
            Self::Null => "null",
            Self::Bool(_) => "bool",
            Self::Int(_) => "int",
            Self::Float(_) => "float",
            Self::String(_) | Self::ShortString(_) => "string",
            Self::Vec(_) => "vec",
            Self::Dict(_) => "dict",
            Self::Tuple(_) => "tuple",
            Self::Function(_) => "function",
            Self::Object(_) => "object",
            Self::Iter(_) => "iterator",
        }
    }
}

impl Clone for Value {
    fn clone(&self) -> Self {
        let mut value = match self.transparent_view() {
            ValueView::Uninitialized => Self::uninitialized(),
            ValueView::Null => Self::null(),
            ValueView::Bool(value) => Self::bool(*value),
            ValueView::Int(value) => Self::int(*value),
            ValueView::Float(value) => Self::float(*value),
            ValueView::String(value) => Self::string(value.clone()),
            ValueView::ShortString(value) => Self::short_string(*value),
            ValueView::Vec(value) => Self::vec(value.clone()),
            ValueView::Dict(value) => Self::dict(value.clone()),
            ValueView::Tuple(value) => Self::tuple(value.clone()),
            ValueView::Function(value) => Self::function(value.clone()),
            ValueView::Object(value) => Self::object(value.clone()),
            ValueView::Iter(value) => Self::iterator(value.clone()),
        };
        value.newtype = self.newtype;
        value
    }
}

impl Drop for Value {
    fn drop(&mut self) {
        // SAFETY: the tag and managed handle prove the payload type and lifetime.
        unsafe {
            match self.kind {
                ValueKind::String => ManuallyDrop::drop(&mut self.payload.string),
                ValueKind::Vec => ManuallyDrop::drop(&mut self.payload.vec),
                ValueKind::Dict => ManuallyDrop::drop(&mut self.payload.dict),
                ValueKind::Tuple => ManuallyDrop::drop(&mut self.payload.tuple),
                ValueKind::Function => ManuallyDrop::drop(&mut self.payload.function),
                ValueKind::Object => ManuallyDrop::drop(&mut self.payload.object),
                ValueKind::Iter => ManuallyDrop::drop(&mut self.payload.iterator),
                ValueKind::Uninitialized
                | ValueKind::Null
                | ValueKind::Bool
                | ValueKind::Int
                | ValueKind::Float
                | ValueKind::ShortString => {}
            }
        }
    }
}

impl Value {
    #[must_use]
    #[inline(always)]
    pub(crate) fn clone_inline_scalar(&self) -> Self {
        if self.is_reference_counted() {
            self.clone()
        } else {
            // SAFETY: the source and target ranges are valid.
            unsafe { ptr::read(self) }
        }
    }

    #[must_use]
    pub(crate) const fn uninitialized() -> Self {
        Self {
            payload: ValuePayload { raw: 0 },
            kind: ValueKind::Uninitialized,
            newtype: NO_NEWTYPE,
        }
    }

    #[must_use]
    pub(crate) const fn null() -> Self {
        Self {
            payload: ValuePayload { raw: 0 },
            kind: ValueKind::Null,
            newtype: NO_NEWTYPE,
        }
    }

    #[must_use]
    pub(crate) const fn bool(value: bool) -> Self {
        Self {
            payload: ValuePayload { boolean: value },
            kind: ValueKind::Bool,
            newtype: NO_NEWTYPE,
        }
    }

    #[must_use]
    pub(crate) const fn int(value: i64) -> Self {
        Self {
            payload: ValuePayload { integer: value },
            kind: ValueKind::Int,
            newtype: NO_NEWTYPE,
        }
    }

    #[must_use]
    pub(crate) const fn float(value: f64) -> Self {
        Self {
            payload: ValuePayload { float: value },
            kind: ValueKind::Float,
            newtype: NO_NEWTYPE,
        }
    }

    #[must_use]
    pub(crate) const fn string(value: ManagedRef<ByteStringObject>) -> Self {
        Self {
            payload: ValuePayload {
                string: ManuallyDrop::new(value),
            },
            kind: ValueKind::String,
            newtype: NO_NEWTYPE,
        }
    }

    #[must_use]
    pub(crate) const fn short_string(value: ShortString) -> Self {
        Self {
            payload: ValuePayload {
                short_string: value,
            },
            kind: ValueKind::ShortString,
            newtype: NO_NEWTYPE,
        }
    }

    #[must_use]
    #[inline(always)]
    pub(crate) fn from_string_bytes(heap: &Heap, bytes: &[u8]) -> Self {
        match ShortString::from_bytes(bytes) {
            Some(string) => Self::short_string(string),
            None => Self::string(ByteStringObject::from_bytes(heap, bytes)),
        }
    }

    #[must_use]
    #[inline(always)]
    pub(crate) fn from_string_vec(heap: &Heap, bytes: Vec<u8>) -> Self {
        match ShortString::from_bytes(&bytes) {
            Some(string) => Self::short_string(string),
            None => Self::string(ByteStringObject::from_vec(heap, bytes)),
        }
    }

    #[must_use]
    pub(crate) const fn vec(value: ManagedRef<VecObject>) -> Self {
        Self {
            payload: ValuePayload {
                vec: ManuallyDrop::new(value),
            },
            kind: ValueKind::Vec,
            newtype: NO_NEWTYPE,
        }
    }

    #[must_use]
    pub(crate) const fn vec_cursor(value: ManagedRef<VecObject>) -> Self {
        Self {
            payload: ValuePayload {
                vec: ManuallyDrop::new(value),
            },
            kind: ValueKind::Vec,
            newtype: 0,
        }
    }

    #[must_use]
    pub(crate) const fn dict(value: ManagedRef<DictObject>) -> Self {
        Self {
            payload: ValuePayload {
                dict: ManuallyDrop::new(value),
            },
            kind: ValueKind::Dict,
            newtype: NO_NEWTYPE,
        }
    }

    #[must_use]
    pub(crate) const fn dict_cursor(value: ManagedRef<DictObject>) -> Self {
        Self {
            payload: ValuePayload {
                dict: ManuallyDrop::new(value),
            },
            kind: ValueKind::Dict,
            newtype: 0,
        }
    }

    #[must_use]
    pub(crate) const fn tuple(value: ManagedRef<TupleObject>) -> Self {
        Self {
            payload: ValuePayload {
                tuple: ManuallyDrop::new(value),
            },
            kind: ValueKind::Tuple,
            newtype: NO_NEWTYPE,
        }
    }

    #[must_use]
    pub(crate) const fn tuple_cursor(value: ManagedRef<TupleObject>) -> Self {
        Self {
            payload: ValuePayload {
                tuple: ManuallyDrop::new(value),
            },
            kind: ValueKind::Tuple,
            newtype: 0,
        }
    }

    #[must_use]
    pub(crate) const fn function(value: ManagedRef<FunctionObject>) -> Self {
        Self {
            payload: ValuePayload {
                function: ManuallyDrop::new(value),
            },
            kind: ValueKind::Function,
            newtype: NO_NEWTYPE,
        }
    }

    #[must_use]
    pub(crate) const fn object(value: ManagedRef<InstanceObject>) -> Self {
        Self {
            payload: ValuePayload {
                object: ManuallyDrop::new(value),
            },
            kind: ValueKind::Object,
            newtype: NO_NEWTYPE,
        }
    }

    #[must_use]
    pub(crate) fn newtype(value: Self, id: NewtypeValueId) -> Self {
        value.with_newtype(Some(id))
    }

    #[must_use]
    pub(crate) const fn iterator(value: ManagedRef<IteratorObject>) -> Self {
        Self {
            payload: ValuePayload {
                iterator: ManuallyDrop::new(value),
            },
            kind: ValueKind::Iter,
            newtype: NO_NEWTYPE,
        }
    }

    #[must_use]
    #[inline(always)]
    pub(crate) fn transparent_view(&self) -> ValueView<'_> {
        // SAFETY: the tag and managed handle prove the payload type and lifetime.
        unsafe {
            match self.kind {
                ValueKind::Uninitialized => ValueView::Uninitialized,
                ValueKind::Null => ValueView::Null,
                ValueKind::Bool => ValueView::Bool(&self.payload.boolean),
                ValueKind::Int => ValueView::Int(&self.payload.integer),
                ValueKind::Float => ValueView::Float(&self.payload.float),
                ValueKind::String => ValueView::String(&self.payload.string),
                ValueKind::ShortString => ValueView::ShortString(&self.payload.short_string),
                ValueKind::Vec => ValueView::Vec(&self.payload.vec),
                ValueKind::Dict => ValueView::Dict(&self.payload.dict),
                ValueKind::Tuple => ValueView::Tuple(&self.payload.tuple),
                ValueKind::Function => ValueView::Function(&self.payload.function),
                ValueKind::Object => ValueView::Object(&self.payload.object),
                ValueKind::Iter => ValueView::Iter(&self.payload.iterator),
            }
        }
    }

    #[must_use]
    #[inline(always)]
    pub(crate) fn transparent(&self) -> ValueView<'_> {
        self.transparent_view()
    }

    #[must_use]
    pub(crate) fn with_newtype(mut self, id: Option<NewtypeValueId>) -> Self {
        self.newtype = id.map_or(NO_NEWTYPE, |id| id.0);
        self
    }

    #[must_use]
    pub(crate) fn clone_with_newtype(&self, id: Option<NewtypeValueId>) -> Self {
        self.clone().with_newtype(id)
    }

    #[must_use]
    #[inline(always)]
    pub(crate) fn newtype_id(&self) -> Option<NewtypeValueId> {
        (self.newtype != NO_NEWTYPE).then_some(NewtypeValueId(self.newtype))
    }

    #[must_use]
    pub(crate) fn is_uninitialized(&self) -> bool {
        self.kind == ValueKind::Uninitialized
    }

    #[must_use]
    #[inline(always)]
    pub(crate) fn is_null(&self) -> bool {
        self.kind == ValueKind::Null
    }

    #[must_use]
    #[inline(always)]
    pub(crate) fn is_bool(&self) -> bool {
        self.kind == ValueKind::Bool
    }

    #[must_use]
    #[inline(always)]
    pub(crate) fn is_int(&self) -> bool {
        self.kind == ValueKind::Int
    }

    #[must_use]
    #[inline(always)]
    pub(crate) fn is_float(&self) -> bool {
        self.kind == ValueKind::Float
    }

    #[must_use]
    #[inline(always)]
    pub(crate) const fn is_string(&self) -> bool {
        matches!(self.kind, ValueKind::String | ValueKind::ShortString)
    }

    #[must_use]
    #[inline(always)]
    pub(crate) fn is_vec(&self) -> bool {
        self.kind == ValueKind::Vec
    }

    #[must_use]
    #[inline(always)]
    pub(crate) fn is_dict(&self) -> bool {
        self.kind == ValueKind::Dict
    }

    #[must_use]
    #[inline(always)]
    pub(crate) fn is_tuple(&self) -> bool {
        self.kind == ValueKind::Tuple
    }

    #[must_use]
    #[inline(always)]
    pub(crate) fn is_function(&self) -> bool {
        self.kind == ValueKind::Function
    }

    #[must_use]
    #[inline(always)]
    pub(crate) fn is_object(&self) -> bool {
        self.kind == ValueKind::Object
    }

    #[must_use]
    #[inline(always)]
    pub(crate) const fn is_reference_counted(&self) -> bool {
        matches!(
            self.kind,
            ValueKind::String
                | ValueKind::Vec
                | ValueKind::Dict
                | ValueKind::Tuple
                | ValueKind::Function
                | ValueKind::Object
                | ValueKind::Iter
        )
    }

    #[must_use]
    pub(crate) fn has_other_strong_references(&self) -> bool {
        match self.transparent_view() {
            ValueView::String(value) => value.has_other_strong_references(),
            ValueView::Vec(value) => value.has_other_strong_references(),
            ValueView::Dict(value) => value.has_other_strong_references(),
            ValueView::Tuple(value) => value.has_other_strong_references(),
            ValueView::Function(value) => value.has_other_strong_references(),
            ValueView::Object(value) => value.has_other_strong_references(),
            ValueView::Iter(value) => value.has_other_strong_references(),
            ValueView::Uninitialized
            | ValueView::Null
            | ValueView::Bool(_)
            | ValueView::Int(_)
            | ValueView::Float(_)
            | ValueView::ShortString(_) => false,
        }
    }

    #[must_use]
    #[inline(always)]
    pub(crate) fn as_bool(&self) -> Option<bool> {
        if self.kind == ValueKind::Bool {
            // SAFETY: the tag and managed handle prove the payload type and lifetime.
            Some(unsafe { self.payload.boolean })
        } else {
            None
        }
    }

    #[must_use]
    #[inline(always)]
    pub(crate) fn as_bool_mut(&mut self) -> Option<&mut bool> {
        if self.kind == ValueKind::Bool {
            // SAFETY: the tag and managed handle prove the payload type and lifetime.
            Some(unsafe { &mut self.payload.boolean })
        } else {
            None
        }
    }

    #[must_use]
    #[inline(always)]
    pub(crate) fn as_int(&self) -> Option<i64> {
        if self.kind == ValueKind::Int {
            // SAFETY: the tag and managed handle prove the payload type and lifetime.
            Some(unsafe { self.payload.integer })
        } else {
            None
        }
    }

    #[must_use]
    #[inline(always)]
    pub(crate) fn as_int_mut(&mut self) -> Option<&mut i64> {
        if self.kind == ValueKind::Int {
            // SAFETY: the tag and managed handle prove the payload type and lifetime.
            Some(unsafe { &mut self.payload.integer })
        } else {
            None
        }
    }

    /// Returns the integer payload without checking the variant.
    ///
    /// # Safety
    ///
    /// This value must be an integer.
    #[must_use]
    #[inline(always)]
    pub(crate) unsafe fn as_int_unchecked(&self) -> i64 {
        if self.kind != ValueKind::Int {
            // SAFETY: the surrounding invariant makes this path unreachable.
            unsafe { hint::unreachable_unchecked() }
        }

        // SAFETY: the tag and managed handle prove the payload type and lifetime.
        unsafe { self.payload.integer }
    }

    #[must_use]
    #[inline(always)]
    pub(crate) fn as_float(&self) -> Option<f64> {
        if self.kind == ValueKind::Float {
            // SAFETY: the tag and managed handle prove the payload type and lifetime.
            Some(unsafe { self.payload.float })
        } else {
            None
        }
    }

    #[must_use]
    #[inline(always)]
    pub(crate) fn as_float_mut(&mut self) -> Option<&mut f64> {
        if self.kind == ValueKind::Float {
            // SAFETY: the tag and managed handle prove the payload type and lifetime.
            Some(unsafe { &mut self.payload.float })
        } else {
            None
        }
    }

    /// Returns the float payload without checking the variant.
    ///
    /// # Safety
    ///
    /// This value must be a float.
    #[must_use]
    #[inline(always)]
    pub(crate) unsafe fn as_float_unchecked(&self) -> f64 {
        if self.kind != ValueKind::Float {
            // SAFETY: the surrounding invariant makes this path unreachable.
            unsafe { hint::unreachable_unchecked() }
        }

        // SAFETY: the tag and managed handle prove the payload type and lifetime.
        unsafe { self.payload.float }
    }

    #[must_use]
    #[inline(always)]
    pub(crate) fn as_string_bytes(&self) -> Option<&[u8]> {
        match self.transparent_view() {
            ValueView::String(value) => Some(ByteStringObject::handle_bytes(value)),
            ValueView::ShortString(value) => Some(value.as_bytes()),
            _ => None,
        }
    }

    #[must_use]
    #[inline(always)]
    pub(crate) fn as_short_string(&self) -> Option<ShortString> {
        if self.kind == ValueKind::ShortString {
            // SAFETY: the tag and managed handle prove the payload type and lifetime.
            Some(unsafe { self.payload.short_string })
        } else {
            None
        }
    }

    /// Moves out the string handle without checking the variant.
    ///
    /// # Safety
    ///
    /// This value must hold a heap string.
    #[must_use]
    #[inline(always)]
    pub(crate) unsafe fn into_string_unchecked(mut self) -> ManagedRef<ByteStringObject> {
        if self.kind != ValueKind::String {
            // SAFETY: the surrounding invariant makes this path unreachable.
            unsafe { hint::unreachable_unchecked() }
        }

        self.kind = ValueKind::Null;
        // SAFETY: the tag and managed handle prove the payload type and lifetime.
        unsafe { ManuallyDrop::take(&mut self.payload.string) }
    }

    #[must_use]
    #[inline(always)]
    pub(crate) fn as_vec(&self) -> Option<&ManagedRef<VecObject>> {
        // SAFETY: the tag and managed handle prove the payload type and lifetime.
        (self.kind == ValueKind::Vec).then(|| unsafe { &*self.payload.vec })
    }

    #[must_use]
    #[inline(always)]
    pub(crate) fn as_vec_mut(&mut self) -> Option<&mut ManagedRef<VecObject>> {
        // SAFETY: the tag and managed handle prove the payload type and lifetime.
        (self.kind == ValueKind::Vec).then(|| unsafe { &mut *self.payload.vec })
    }

    #[must_use]
    #[inline(always)]
    pub(crate) fn as_vec_cursor_mut(&mut self) -> Option<(&ManagedRef<VecObject>, &mut u32)> {
        (self.kind == ValueKind::Vec && self.newtype != NO_NEWTYPE)
            // SAFETY: the tag and managed handle prove the payload type and lifetime.
            .then(|| unsafe { (&*self.payload.vec, &mut self.newtype) })
    }

    #[must_use]
    #[inline(always)]
    pub(crate) fn as_dict(&self) -> Option<&ManagedRef<DictObject>> {
        // SAFETY: the tag and managed handle prove the payload type and lifetime.
        (self.kind == ValueKind::Dict).then(|| unsafe { &*self.payload.dict })
    }

    #[must_use]
    #[inline(always)]
    pub(crate) fn as_dict_mut(&mut self) -> Option<&mut ManagedRef<DictObject>> {
        // SAFETY: the tag and managed handle prove the payload type and lifetime.
        (self.kind == ValueKind::Dict).then(|| unsafe { &mut *self.payload.dict })
    }

    #[must_use]
    #[inline(always)]
    pub(crate) fn as_dict_cursor_mut(&mut self) -> Option<(&ManagedRef<DictObject>, &mut u32)> {
        (self.kind == ValueKind::Dict && self.newtype != NO_NEWTYPE)
            // SAFETY: the tag and managed handle prove the payload type and lifetime.
            .then(|| unsafe { (&*self.payload.dict, &mut self.newtype) })
    }

    #[must_use]
    #[inline(always)]
    pub(crate) fn as_tuple(&self) -> Option<&ManagedRef<TupleObject>> {
        // SAFETY: the tag and managed handle prove the payload type and lifetime.
        (self.kind == ValueKind::Tuple).then(|| unsafe { &*self.payload.tuple })
    }

    #[must_use]
    #[inline(always)]
    pub(crate) fn as_tuple_cursor_mut(&mut self) -> Option<(&ManagedRef<TupleObject>, &mut u32)> {
        (self.kind == ValueKind::Tuple && self.newtype != NO_NEWTYPE)
            // SAFETY: the tag and managed handle prove the payload type and lifetime.
            .then(|| unsafe { (&*self.payload.tuple, &mut self.newtype) })
    }

    #[must_use]
    #[inline(always)]
    pub(crate) fn as_function(&self) -> Option<&ManagedRef<FunctionObject>> {
        // SAFETY: the tag and managed handle prove the payload type and lifetime.
        (self.kind == ValueKind::Function).then(|| unsafe { &*self.payload.function })
    }

    #[must_use]
    #[inline(always)]
    pub(crate) fn as_object(&self) -> Option<&ManagedRef<InstanceObject>> {
        // SAFETY: the tag and managed handle prove the payload type and lifetime.
        (self.kind == ValueKind::Object).then(|| unsafe { &*self.payload.object })
    }

    /// Returns the object handle without checking the variant.
    ///
    /// # Safety
    ///
    /// This value must be an object.
    #[must_use]
    pub(crate) unsafe fn as_object_unchecked(&self) -> &ManagedRef<InstanceObject> {
        if self.kind != ValueKind::Object {
            // SAFETY: the surrounding invariant makes this path unreachable.
            unsafe { hint::unreachable_unchecked() }
        }

        // SAFETY: the tag and managed handle prove the payload type and lifetime.
        unsafe { &self.payload.object }
    }

    /// Moves out the object handle without checking the variant.
    ///
    /// # Safety
    ///
    /// This value must be an object.
    #[must_use]
    pub(crate) unsafe fn into_object_unchecked(mut self) -> ManagedRef<InstanceObject> {
        if self.kind != ValueKind::Object {
            // SAFETY: the surrounding invariant makes this path unreachable.
            unsafe { hint::unreachable_unchecked() }
        }

        self.kind = ValueKind::Null;
        // SAFETY: the tag and managed handle prove the payload type and lifetime.
        unsafe { ManuallyDrop::take(&mut self.payload.object) }
    }

    #[must_use]
    pub(crate) fn as_iterator(&self) -> Option<&ManagedRef<IteratorObject>> {
        // SAFETY: the tag and managed handle prove the payload type and lifetime.
        (self.kind == ValueKind::Iter).then(|| unsafe { &*self.payload.iterator })
    }

    #[must_use]
    pub(crate) fn collectable_box(&self) -> Option<NonNull<HeapBox<()>>> {
        match self.transparent_view() {
            ValueView::Vec(vec) => Some(vec.erased()),
            ValueView::Dict(dict) => Some(dict.erased()),
            ValueView::Tuple(tuple) => Some(tuple.erased()),
            ValueView::Function(function) => Some(function.erased()),
            ValueView::Object(object) => Some(object.erased()),
            ValueView::Uninitialized
            | ValueView::Null
            | ValueView::Bool(_)
            | ValueView::Int(_)
            | ValueView::Float(_)
            | ValueView::String(_)
            | ValueView::ShortString(_)
            | ValueView::Iter(_) => None,
        }
    }

    #[must_use]
    pub(crate) fn kind_name(&self) -> &'static str {
        if self.newtype_id().is_some() {
            return "newtype";
        }

        match self.kind {
            ValueKind::Uninitialized => "uninitialized",
            ValueKind::Null => "null",
            ValueKind::Bool => "bool",
            ValueKind::Int => "int",
            ValueKind::Float => "float",
            ValueKind::String | ValueKind::ShortString => "string",
            ValueKind::Vec => "vec",
            ValueKind::Dict => "dict",
            ValueKind::Tuple => "tuple",
            ValueKind::Function => "function",
            ValueKind::Object => "object",
            ValueKind::Iter => "iterator",
        }
    }
}
