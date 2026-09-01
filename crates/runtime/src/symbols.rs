//! The engine's symbol table and the stores its entries index into.

#![deny(clippy::nursery, clippy::pedantic)]
#![expect(
    clippy::redundant_pub_crate,
    reason = "symbol metadata is shared across the runtime"
)]

use std::array::from_fn;
use std::cell::UnsafeCell;
use std::ops::Deref;
use std::ptr::NonNull;
use std::rc::Rc;

use hashbrown::HashMap;

use crate::builtin::spec::BuiltInDirectHandler;
use crate::bytecode::REFERENCE_REGISTER_LIMIT;
use crate::bytecode::chunk::Chunk;
use crate::bytecode::chunk::descriptors::Literal;
use crate::bytecode::chunk::descriptors::TypeDescriptor;
use crate::bytecode::unit::CompiledAttribute;
use crate::bytecode::unit::CompiledParameter;
use crate::bytecode::unit::CompiledTypeParameter;
use crate::bytecode::unit::CompiledUnit;
use crate::classes::MethodEntry;
use crate::unwrap_option_invariant;
use crate::value::Value;
use crate::value::atom::Atom;
use crate::value::collection::CollectionTypeCheckId;
use crate::value::function::BuiltInId;
use crate::value::function::CallTarget;
use crate::value::function::FuncId;
use crate::value::function::FunctionObject;
use crate::value::heap::handle::ManagedRef;
use crate::value::newtype::NewtypeId;
use crate::value::newtype::NewtypeValueId;
use crate::value::object::ClassId;
use crate::value::object::TypeEnvironmentId;

#[expect(
    clippy::inline_always,
    reason = "inline-cache updates must remain trivial at each call site"
)]
#[inline(always)]
fn record_stable_way<T, const N: usize>(
    ways: &mut [Option<T>; N],
    entry: T,
    equivalent: impl Fn(&T, &T) -> bool,
) {
    for way in ways {
        match way {
            Some(existing) if equivalent(existing, &entry) => {
                *way = Some(entry);
                return;
            }
            None => {
                *way = Some(entry);
                return;
            }
            Some(_) => {}
        }
    }
}

#[derive(Clone, Copy)]
#[expect(
    clippy::struct_excessive_bools,
    reason = "these independent call facts are read directly on VM hot paths"
)]
pub(crate) struct ExactFunctionEntry {
    pub(crate) chunk: NonNull<Chunk>,
    pub(crate) cache: NonNull<InlineCache>,
    pub(crate) unit: NonNull<UnitContext>,
    pub(crate) function: FuncId,
    pub(crate) reference_parameter_mask: u64,
    pub(crate) declared_parameters: u8,
    pub(crate) scalar_return_target: bool,
    pub(crate) finalized: bool,
    pub(crate) has_type_parameters: bool,
    pub(crate) frameless: bool,
}

#[derive(Clone, Copy)]
pub(crate) struct ExactBuiltInFunctionEntry {
    pub(crate) function: BuiltInId,
    pub(crate) direct_handler: Option<BuiltInDirectHandler>,
}

/// A whole-unit-proven final-class method target, with the immutable frame
/// data copied out of the engine's runtime tables after its first resolution.
#[derive(Clone, Copy)]
pub(crate) struct ExactMethodEntry {
    pub(crate) function: ExactFunctionEntry,
    pub(crate) scope: ClassId,
    pub(crate) called: ClassId,
    pub(crate) is_constructor: bool,
}

pub(crate) struct CachedBoundCallable {
    pub(crate) target: CallTarget,
    pub(crate) argument_environment: TypeEnvironmentId,
    pub(crate) callable: ManagedRef<FunctionObject>,
}

#[derive(Clone, Copy)]
pub(crate) struct CachedInstantiationEnvironment {
    pub(crate) class: ClassId,
    pub(crate) outer: TypeEnvironmentId,
    pub(crate) environment: TypeEnvironmentId,
    pub(crate) allocates_plainly: bool,
    pub(crate) slots_are_acyclic: bool,
    pub(crate) slot_count: u32,
}

#[derive(Clone, Copy)]
pub(crate) struct InstantiationWays([Option<CachedInstantiationEnvironment>; INSTANTIATION_WAYS]);

const INSTANTIATION_WAYS: usize = 4;

impl InstantiationWays {
    pub(crate) const EMPTY: Self = Self([None; INSTANTIATION_WAYS]);

    pub(crate) fn get(&self, outer: TypeEnvironmentId) -> Option<CachedInstantiationEnvironment> {
        for way in &self.0 {
            if let Some(cached) = way
                && cached.outer == outer
            {
                return Some(*cached);
            }
        }
        None
    }

    /// Records a resolved specialization, replacing its own entry or taking a
    /// free way. A full set keeps what it has.
    pub(crate) fn record(&mut self, cached: CachedInstantiationEnvironment) {
        record_stable_way(&mut self.0, cached, |existing, cached| {
            existing.outer == cached.outer
        });
    }
}

#[derive(Clone)]
pub(crate) struct CachedNewtypeConstructor {
    pub(crate) outer: TypeEnvironmentId,
    pub(crate) parent: Option<NewtypeValueId>,
    pub(crate) environment: TypeEnvironmentId,
    pub(crate) backing: Rc<TypeDescriptor>,
    pub(crate) guard: Option<ArgumentGuard>,
    pub(crate) tag: NewtypeValueId,
}

#[derive(Clone)]
pub(crate) struct NewtypeConstructorWays(
    [Option<CachedNewtypeConstructor>; NEWTYPE_CONSTRUCTOR_WAYS],
);

const NEWTYPE_CONSTRUCTOR_WAYS: usize = 4;

impl NewtypeConstructorWays {
    pub(crate) const EMPTY: Self = Self([const { None }; NEWTYPE_CONSTRUCTOR_WAYS]);

    pub(crate) fn get(
        &self,
        outer: TypeEnvironmentId,
        parent: Option<NewtypeValueId>,
    ) -> Option<&CachedNewtypeConstructor> {
        self.0
            .iter()
            .flatten()
            .find(|entry| entry.outer == outer && entry.parent == parent)
    }

    pub(crate) fn record(&mut self, entry: CachedNewtypeConstructor) {
        record_stable_way(&mut self.0, entry, |existing, entry| {
            existing.outer == entry.outer && existing.parent == entry.parent
        });
    }
}

#[derive(Clone, Copy)]
pub(crate) struct CachedCallEnvironment {
    pub(crate) target: CallTarget,
    pub(crate) outer: TypeEnvironmentId,
    pub(crate) environment: TypeEnvironmentId,
}

/// One cheap runtime fact that is sufficient to repeat a successful
/// parameter check without walking its descriptor again.
#[derive(Clone, Copy)]
pub(crate) enum ArgumentGuard {
    Any,
    Null,
    Bool,
    Int,
    Float,
    String,
    Object,
    ExactBool(bool),
    ExactInt(i64),
    IntRange {
        min: Option<i64>,
        max: Option<i64>,
    },
    ExactFloat(u64),
    ExactObject {
        class: ClassId,
        environment: TypeEnvironmentId,
    },
    Callable {
        target: CallTarget,
        environment: TypeEnvironmentId,
    },
}

#[derive(Clone)]
pub(crate) enum CachedParameterGuard {
    Cheap(ArgumentGuard),
    Descriptor {
        descriptor: Rc<TypeDescriptor>,
        collection_id: Option<CollectionTypeCheckId>,
    },
}

#[derive(Clone)]
pub(crate) struct CachedArgumentGuards {
    pub(crate) function: FuncId,
    pub(crate) environment: TypeEnvironmentId,
    pub(crate) guards: Box<[CachedParameterGuard]>,
}

#[derive(Clone)]
pub(crate) struct ArgumentGuardWays([Option<CachedArgumentGuards>; ARGUMENT_GUARD_WAYS]);

const ARGUMENT_GUARD_WAYS: usize = 4;

impl ArgumentGuardWays {
    pub(crate) const EMPTY: Self = Self([const { None }; ARGUMENT_GUARD_WAYS]);

    pub(crate) fn get(
        &self,
        function: FuncId,
        environment: TypeEnvironmentId,
    ) -> Option<&CachedArgumentGuards> {
        self.0
            .iter()
            .flatten()
            .find(|entry| entry.function == function && entry.environment == environment)
    }

    pub(crate) fn record(&mut self, entry: CachedArgumentGuards) {
        record_stable_way(&mut self.0, entry, |existing, entry| {
            existing.function == entry.function && existing.environment == entry.environment
        });
    }
}

#[derive(Clone, Copy)]
pub(crate) struct CachedTurbofishEnvironment {
    /// The compiled body the site resolved to. A built-in body is never cached:
    /// it has no comparable identity here, and it falls through to the bind.
    pub(crate) function: FuncId,
    /// The scope the callee's own parameters are declared in.
    pub(crate) outer: TypeEnvironmentId,
    pub(crate) caller: TypeEnvironmentId,
    pub(crate) environment: TypeEnvironmentId,
}

#[derive(Clone, Copy)]
pub(crate) struct CachedIsCheck {
    pub(crate) caller_environment: TypeEnvironmentId,
    pub(crate) called_class: Option<ClassId>,
    pub(crate) class: ClassId,
    pub(crate) environment: TypeEnvironmentId,
}

#[derive(Clone, Copy)]
pub(crate) struct IsCheckWays {
    cacheability: Option<(TypeEnvironmentId, Option<ClassId>, bool)>,
    ways: [Option<CachedIsCheck>; IS_CHECK_WAYS],
}

const IS_CHECK_WAYS: usize = 4;

impl IsCheckWays {
    pub(crate) const EMPTY: Self = Self {
        cacheability: None,
        ways: [None; IS_CHECK_WAYS],
    };

    pub(crate) fn cacheable(
        &self,
        caller_environment: TypeEnvironmentId,
        called_class: Option<ClassId>,
    ) -> Option<bool> {
        self.cacheability
            .filter(|(cached_environment, cached_class, _)| {
                *cached_environment == caller_environment && *cached_class == called_class
            })
            .map(|(_, _, cacheable)| cacheable)
    }

    pub(crate) const fn set_cacheable(
        &mut self,
        caller_environment: TypeEnvironmentId,
        called_class: Option<ClassId>,
        cacheable: bool,
    ) {
        self.cacheability = Some((caller_environment, called_class, cacheable));
    }

    pub(crate) fn holds(&self, probe: &CachedIsCheck) -> bool {
        self.ways.iter().flatten().any(|cached| {
            cached.class == probe.class
                && cached.environment == probe.environment
                && cached.caller_environment == probe.caller_environment
                && cached.called_class == probe.called_class
        })
    }

    pub(crate) fn record(&mut self, proved: CachedIsCheck) {
        for way in &mut self.ways {
            if way.is_none() {
                *way = Some(proved);
                return;
            }
        }
        self.ways[0] = Some(proved);
    }
}

#[derive(Clone, Copy)]
pub(crate) struct CachedExactMethod {
    pub(crate) class: ClassId,
    pub(crate) entry: MethodEntry,
    pub(crate) is_constructor: bool,
}

#[derive(Clone, Copy)]
pub(crate) struct ExactMethodWays([Option<CachedExactMethod>; EXACT_METHOD_WAYS]);

const EXACT_METHOD_WAYS: usize = 4;

impl ExactMethodWays {
    pub(crate) const EMPTY: Self = Self([None; EXACT_METHOD_WAYS]);

    pub(crate) fn get(&self, class: ClassId) -> Option<(MethodEntry, bool)> {
        for way in &self.0 {
            if let Some(cached) = way
                && cached.class == class
            {
                return Some((cached.entry, cached.is_constructor));
            }
        }
        None
    }

    /// Records a resolved receiver class, replacing its own entry or taking a
    /// free way. A full set keeps what it has: a site with more live receiver
    /// classes than this resolves the extra ones the long way, which is what
    /// every site did before.
    pub(crate) fn record(&mut self, cached: CachedExactMethod) {
        record_stable_way(&mut self.0, cached, |existing, cached| {
            existing.class == cached.class
        });
    }
}

/// A property-write site after one receiver specialization was checked.
#[derive(Clone, Copy)]
pub(crate) struct CachedPropertyGuard {
    pub(crate) class: ClassId,
    pub(crate) environment: TypeEnvironmentId,
    pub(crate) slot: u32,
    pub(crate) guard: ArgumentGuard,
}

#[derive(Clone, Copy)]
pub(crate) struct PropertyGuardWays([Option<CachedPropertyGuard>; PROPERTY_GUARD_WAYS]);

const PROPERTY_GUARD_WAYS: usize = 8;

impl PropertyGuardWays {
    pub(crate) const EMPTY: Self = Self([None; PROPERTY_GUARD_WAYS]);

    pub(crate) fn iter(&self) -> impl Iterator<Item = &Option<CachedPropertyGuard>> {
        self.0.iter()
    }

    /// Records a proved specialization, replacing the one for the same
    /// receiver shape or taking a free way. A full set keeps what it has:
    /// a site with more than this many live specializations pays the full
    /// check, which is what it did before the cache existed.
    pub(crate) fn record(&mut self, entry: CachedPropertyGuard) {
        record_stable_way(&mut self.0, entry, |existing, entry| {
            existing.slot == entry.slot
                && existing.class == entry.class
                && existing.environment == entry.environment
        });
    }
}

/// A late-bound method site after one receiver specialization was resolved.
#[derive(Clone, Copy)]
pub(crate) struct CachedGuardedMethod {
    pub(crate) receiver_class: ClassId,
    pub(crate) receiver_environment: TypeEnvironmentId,
    /// The environment the *caller* was running in when the entry was filled.
    /// A turbofish on the site is substituted there, so two callers with
    /// different bindings reach different method environments through the same
    /// receiver and must not share a cache entry.
    pub(crate) caller_environment: TypeEnvironmentId,
    pub(crate) method_environment: TypeEnvironmentId,
    pub(crate) entry: ExactMethodEntry,
    pub(crate) arguments: CachedMethodArguments,
    pub(crate) trivial_constructor_parameters: Option<u8>,
    pub(crate) fast_path: CachedMethodFastPath,
}

#[derive(Clone, Copy)]
pub(crate) enum CachedMethodArguments {
    Proven,
    One(ArgumentGuard),
    General,
}

#[derive(Clone, Copy)]
pub(crate) enum CachedMethodFastPath {
    None,
    ReturnReceiver,
    ReturnArgument(u8),
    ReturnProperty(u16),
}

/// A small polymorphic cache for late-bound instance method sites.
#[derive(Clone, Copy)]
pub(crate) struct GuardedMethodWays([Option<CachedGuardedMethod>; GUARDED_METHOD_WAYS]);

const GUARDED_METHOD_WAYS: usize = 4;

impl GuardedMethodWays {
    pub(crate) const EMPTY: Self = Self([None; GUARDED_METHOD_WAYS]);

    pub(crate) fn get(
        &self,
        receiver_class: ClassId,
        receiver_environment: TypeEnvironmentId,
        caller_environment: TypeEnvironmentId,
    ) -> Option<CachedGuardedMethod> {
        self.0
            .iter()
            .flatten()
            .find(|entry| {
                entry.receiver_class == receiver_class
                    && entry.receiver_environment == receiver_environment
                    && entry.caller_environment == caller_environment
            })
            .copied()
    }

    pub(crate) fn record(&mut self, entry: CachedGuardedMethod) {
        record_stable_way(&mut self.0, entry, |existing, entry| {
            existing.receiver_class == entry.receiver_class
                && existing.receiver_environment == entry.receiver_environment
                && existing.caller_environment == entry.caller_environment
        });
    }
}

#[derive(Clone)]
pub(crate) struct CachedNamedArguments {
    pub(crate) target: CallTarget,
    pub(crate) positions: Rc<[usize]>,
    pub(crate) final_count: usize,
}

pub(crate) struct NamedArgumentWays([Option<CachedNamedArguments>; NAMED_ARGUMENT_WAYS]);

const NAMED_ARGUMENT_WAYS: usize = 4;

impl Default for NamedArgumentWays {
    fn default() -> Self {
        Self(from_fn(|_| None))
    }
}

impl NamedArgumentWays {
    pub(crate) fn get(&self, target: CallTarget) -> Option<CachedNamedArguments> {
        self.0
            .iter()
            .flatten()
            .find(|entry| entry.target == target)
            .cloned()
    }

    pub(crate) fn record(&mut self, entry: CachedNamedArguments) {
        record_stable_way(&mut self.0, entry, |existing, entry| {
            existing.target == entry.target
        });
    }
}

/// One chunk's lazily populated inline-cache entries.
#[derive(Default)]
pub(crate) struct InlineCache {
    entries: UnsafeCell<Vec<CacheEntry>>,
    property_slots: UnsafeCell<Vec<CachedPropertySlot>>,
    exact_functions: UnsafeCell<Vec<Option<ExactFunctionEntry>>>,
    exact_built_in_functions: UnsafeCell<Vec<Option<ExactBuiltInFunctionEntry>>>,
    exact_methods: UnsafeCell<Vec<Option<ExactMethodEntry>>>,
    bound_callables: UnsafeCell<Vec<Option<CachedBoundCallable>>>,
    instantiation_environments: UnsafeCell<Vec<InstantiationWays>>,
    newtype_constructors: UnsafeCell<Vec<NewtypeConstructorWays>>,
    call_environments: UnsafeCell<Vec<Option<CachedCallEnvironment>>>,
    named_arguments: UnsafeCell<Vec<NamedArgumentWays>>,
    argument_guards: UnsafeCell<Vec<ArgumentGuardWays>>,
    guarded_methods: UnsafeCell<Vec<Option<CachedGuardedMethod>>>,
    polymorphic_guarded_methods: UnsafeCell<Vec<GuardedMethodWays>>,
    property_guards: UnsafeCell<Vec<PropertyGuardWays>>,
    exact_method_ways: UnsafeCell<Vec<ExactMethodWays>>,
    is_checks: UnsafeCell<Vec<IsCheckWays>>,
    turbofish_environments: UnsafeCell<Vec<Option<CachedTurbofishEnvironment>>>,
}

macro_rules! inline_cache_accessors {
    ($($(#[$attribute:meta])* $field:ident: $entry:ty),+ $(,)?) => {
        $(
            $(#[$attribute])*
            /// The caller must hold the active engine or VM exclusively, create
            /// at most one reference from the pointer, and release it before an
            /// operation that can mutate this cache.
            #[inline(always)]
            pub(crate) const fn $field(&self) -> *mut Vec<$entry> {
                self.$field.get()
            }
        )+
    };
}

impl InlineCache {
    pub(crate) fn new() -> Self {
        Self::default()
    }

    inline_cache_accessors! {
        entries: CacheEntry,
        property_slots: CachedPropertySlot,
        exact_functions: Option<ExactFunctionEntry>,
        exact_built_in_functions: Option<ExactBuiltInFunctionEntry>,
        /// The once-resolved exact-method targets.
        exact_methods: Option<ExactMethodEntry>,
        bound_callables: Option<CachedBoundCallable>,
        instantiation_environments: InstantiationWays,
        newtype_constructors: NewtypeConstructorWays,
        call_environments: Option<CachedCallEnvironment>,
        named_arguments: NamedArgumentWays,
        argument_guards: ArgumentGuardWays,
        /// Exact method metadata guarded by a late-bound receiver shape.
        guarded_methods: Option<CachedGuardedMethod>,
        polymorphic_guarded_methods: GuardedMethodWays,
        property_guards: PropertyGuardWays,
        exact_method_ways: ExactMethodWays,
        is_checks: IsCheckWays,
        turbofish_environments: Option<CachedTurbofishEnvironment>,
    }
}

/// One monomorphic property cache entry packed into a single machine word.
#[derive(Clone, Copy)]
pub(crate) struct CachedPropertySlot(u64);

impl CachedPropertySlot {
    pub(crate) const EMPTY: Self = Self(0);

    pub(crate) fn new(class: ClassId, slot: u32) -> Self {
        // SAFETY: a class identifier is always below u32::MAX, so the increment never overflows.
        let class = unsafe {
            unwrap_option_invariant(
                class.0.checked_add(1),
                "a class identifier fits below u32::MAX",
            )
        };
        Self((u64::from(class) << 32) | u64::from(slot))
    }

    #[expect(
        clippy::cast_possible_truncation,
        clippy::inline_always,
        reason = "the packed halves fit u32 and this lookup is a VM hot path"
    )]
    #[inline(always)]
    pub(crate) fn get(self, class: ClassId) -> Option<u32> {
        let cached_class = (self.0 >> 32) as u32;
        (cached_class == class.0 + 1).then_some(self.0 as u32)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) enum SymbolKind {
    Constant,
    Function,
    Class,
    Enum,
    Interface,
    TypeAlias,
    Newtype,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum FunctionTable {
    User,
    BuiltIn,
}

#[derive(Debug, Clone, Copy)]
pub(crate) struct SymbolEntry {
    pub kind: SymbolKind,
    /// The index into the kind's store.
    pub index: u32,
    /// Whether the declaration belongs to the Rust-backed core.
    pub table: FunctionTable,
}

/// One entry of a call site's inline cache.
#[derive(Clone)]
pub(crate) enum CacheEntry {
    Empty,
    Function(FuncId),
    Constant(u32),
    /// The site instantiates this exact named class.
    Class(ClassId),
    /// The site constructs this exact named newtype.
    Newtype(NewtypeId),
    Method {
        class: ClassId,
        entry: MethodEntry,
    },
    /// The site accesses this static property slot; static resolutions are
    /// fixed, so no guard is needed.
    StaticSlot {
        class: ClassId,
        slot: u32,
    },
    ClassConstant(Value),
    BuiltInCallable(u32),
}

pub(crate) struct UnitSourceFile {
    pub(crate) path: Atom,
    pub(crate) start: u32,
    pub(crate) end: u32,
    /// Line starts relative to `start`.
    pub(crate) line_starts: Vec<u32>,
}

#[derive(Clone)]
pub(crate) enum SourceText {
    Shared(Rc<str>),
    Static(&'static str),
}

impl SourceText {
    pub(crate) fn to_rc(&self) -> Rc<str> {
        match self {
            Self::Shared(source) => Rc::clone(source),
            Self::Static(source) => Rc::from(*source),
        }
    }
}

impl Deref for SourceText {
    type Target = str;

    fn deref(&self) -> &Self::Target {
        match self {
            Self::Shared(source) => source,
            Self::Static(source) => source,
        }
    }
}

pub(crate) struct UnitContext {
    pub unit: Rc<CompiledUnit>,
    pub origin: UnitOrigin,
    /// The unit's file path.
    pub path: Atom,
    /// The unit's source text, when it was compiled by this engine.
    pub source: Option<SourceText>,
    /// The byte offset of each line start in the unit's source, for span to
    /// line translation; empty when the source is unavailable, in which case
    /// every line resolves to zero.
    pub line_starts: Vec<u32>,
    pub(crate) source_files: Vec<UnitSourceFile>,
    pub(crate) main_cache: Box<InlineCache>,
    pub(crate) main_chunk: NonNull<Chunk>,
    /// The unit's synthesized closures by name, resolved by `MakeClosure`.
    pub closures: HashMap<Atom, FuncId>,
    pub(crate) lazy_callables: bool,
    pub(crate) optimizer_destructors: [Option<u32>; 2],
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum UnitOrigin {
    User,
    Extension,
}

#[derive(Debug, Clone, Copy)]
pub(crate) enum FunctionLocator {
    TopLevel(u32),
    Method { class: u32, method: u32 },
}

pub(crate) struct RuntimeFunction {
    /// The function's diagnostic name: its fully qualified name, the
    /// synthesized closure name, or `Class::method`.
    pub name: Atom,
    /// The rendered `fn(...)` type of the function.
    pub signature: Atom,
    /// The unit the function was declared by.
    pub unit: Rc<UnitContext>,
    /// Where the body lives within the unit.
    pub locator: FunctionLocator,
    pub(crate) chunk: NonNull<Chunk>,
    /// Stable storage allocated only when a deferred body is first optimized.
    pub(crate) optimized_chunk: Option<Box<Chunk>>,
    pub(crate) optimization: CallableOptimization,
    pub(crate) parameters: NonNull<[CompiledParameter]>,
    pub(crate) type_parameters: NonNull<[CompiledTypeParameter]>,
    pub(crate) attributes: NonNull<[CompiledAttribute]>,
    pub(crate) frameless_literal: Option<Literal>,
    /// The runtime-enforced return type, boxed so checks may keep its address
    /// while re-entrant loading grows the function store.
    pub return_type: Option<Box<TypeDescriptor>>,
    /// Whether the body captures `$this` as its leading capture, expecting
    /// it at register zero before the parameters.
    pub captures_this: bool,
    /// The class that declared the function, when it is a method body.
    pub declaring_class: Option<ClassId>,
    /// How many parameters must be given at every call.
    pub required_parameters: u8,
    pub declared_parameters: u8,
    /// Parameter positions whose declared type may own a heap reference.
    pub reference_parameter_mask: u64,
    pub(crate) cache: Box<InlineCache>,
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) enum CallableOptimization {
    Pending,
    Optimizing,
    Complete,
}

impl RuntimeFunction {
    #[must_use]
    pub(crate) const fn parameters(&self) -> &[CompiledParameter] {
        // SAFETY: the engine owns this stable slice for the function's lifetime.
        unsafe { self.parameters.as_ref() }
    }

    #[must_use]
    pub(crate) const fn type_parameters(&self) -> &[CompiledTypeParameter] {
        // SAFETY: the engine owns this stable slice for the function's lifetime.
        unsafe { self.type_parameters.as_ref() }
    }

    #[must_use]
    pub(crate) const fn attributes(&self) -> &[CompiledAttribute] {
        // SAFETY: the engine owns this stable slice for the function's lifetime.
        unsafe { self.attributes.as_ref() }
    }
}

impl ExactFunctionEntry {
    #[expect(
        clippy::inline_always,
        reason = "call-cache entries must remain direct field copies"
    )]
    #[inline(always)]
    pub(crate) fn from_runtime(
        function: FuncId,
        runtime: &RuntimeFunction,
        scalar_return_target: bool,
    ) -> Self {
        Self {
            chunk: runtime.chunk,
            cache: NonNull::from(&*runtime.cache),
            unit: NonNull::from(&*runtime.unit),
            function,
            reference_parameter_mask: runtime.reference_parameter_mask,
            declared_parameters: runtime.declared_parameters,
            scalar_return_target,
            finalized: runtime.optimization == CallableOptimization::Complete,
            has_type_parameters: !runtime.type_parameters().is_empty(),
            frameless: runtime.frameless_literal.is_some(),
        }
    }

    #[expect(
        clippy::inline_always,
        reason = "call-cache entries must remain direct field copies"
    )]
    #[inline(always)]
    pub(crate) fn from_call_site(
        function: FuncId,
        runtime: &RuntimeFunction,
        caller: &Chunk,
        destination: u16,
    ) -> Self {
        let scalar_return_target = caller.register_count <= REFERENCE_REGISTER_LIMIT
            && caller.reference_register_mask & (1u64 << destination) == 0
            && runtime
                .return_type
                .as_ref()
                .is_some_and(|descriptor| !descriptor.may_hold_reference());

        Self::from_runtime(function, runtime, scalar_return_target)
    }
}

pub(crate) struct RuntimeTypeEnvironment {
    /// The environment visible before this binding was introduced.
    pub parent: Option<TypeEnvironmentId>,
    /// The one binder introduced at this level. The shared empty environment
    /// has no binding.
    pub binding: Option<(Atom, TypeDescriptor)>,
}

#[must_use]
#[expect(
    clippy::cast_possible_truncation,
    reason = "source offsets and line counts are limited to u32"
)]
pub(crate) fn line_of(line_starts: &[u32], offset: u32) -> u32 {
    if line_starts.is_empty() {
        return 0;
    }
    line_starts.partition_point(|start| *start <= offset) as u32
}

#[must_use]
#[expect(
    clippy::cast_possible_truncation,
    reason = "source offsets are limited to u32"
)]
pub(crate) fn line_starts_of(source: &str) -> Vec<u32> {
    let mut starts = vec![0];
    for (position, byte) in source.bytes().enumerate() {
        if byte == b'\n' {
            starts.push(position as u32 + 1);
        }
    }
    starts
}
