//! The per-execution state: the register stack, the frame stack, and the
//! types the dispatch loop and its neighbours share.

use std::mem::size_of;
use std::num::NonZeroU32;
use std::ptr::NonNull;
use std::rc::Rc;

use hashbrown::HashMap;

use crate::builtin::throw::Throw;
use crate::bytecode::REFERENCE_REGISTER_LIMIT;
use crate::bytecode::chunk::Chunk;
use crate::bytecode::chunk::descriptors::IcDescriptor;
use crate::bytecode::chunk::descriptors::Literal;
use crate::bytecode::chunk::descriptors::TypeDescriptor;
use crate::bytecode::instruction::Instruction;
use crate::bytecode::instruction::MAIN_FRAME_REGISTER_HEADROOM;
use crate::bytecode::instruction::operands::AsMode;
use crate::bytecode::instruction::operands::DescriptorIndex;
use crate::bytecode::instruction::operands::Register;
use crate::bytecode::instruction::word::InstructionKind;
use crate::bytecode::instruction::word::InstructionWord;
use crate::bytecode::unit::must_use_note;
use crate::classes::MethodBodyKind;
use crate::classes::is_instance_of;
use crate::classes::visibility_allows;
use crate::core::classes::TRACE_FRAME_SLOT_ARGUMENTS;
use crate::core::classes::TRACE_FRAME_SLOT_FILE;
use crate::core::classes::TRACE_FRAME_SLOT_FUNCTION;
use crate::core::classes::TRACE_FRAME_SLOT_LINE;
use crate::engine::Engine;
use crate::symbols::ArgumentGuard;
use crate::symbols::CacheEntry;
use crate::symbols::CachedArgumentGuards;
use crate::symbols::CachedBoundCallable;
use crate::symbols::CachedCallEnvironment;
use crate::symbols::CachedExactMethod;
use crate::symbols::CachedGuardedMethod;
use crate::symbols::CachedInstantiationEnvironment;
use crate::symbols::CachedIsCheck;
use crate::symbols::CachedMethodArguments;
use crate::symbols::CachedMethodFastPath;
use crate::symbols::CachedParameterGuard;
use crate::symbols::CachedPropertyGuard;
use crate::symbols::CachedPropertySlot;
use crate::symbols::CachedTurbofishEnvironment;
use crate::symbols::ExactFunctionEntry;
use crate::symbols::ExactMethodEntry;
use crate::symbols::ExactMethodWays;
use crate::symbols::FunctionTable;
use crate::symbols::InlineCache;
use crate::symbols::InstantiationWays;
use crate::symbols::IsCheckWays;
use crate::symbols::PropertyGuardWays;
use crate::symbols::SymbolEntry;
use crate::symbols::SymbolKind;
use crate::symbols::UnitContext;
use crate::symbols::line_of;
use crate::unreachable_invariant;
use crate::unwrap_option_invariant;
use crate::unwrap_result_invariant;
use crate::value::Value;
use crate::value::atom::Atom;
use crate::value::dict::DictObject;
use crate::value::dict::keys::Key;
use crate::value::dict::keys::KeyRef;
use crate::value::function::CallTarget;
use crate::value::function::FuncId;
use crate::value::function::FunctionObject;
use crate::value::heap::Heap;
use crate::value::heap::handle::ManagedRef;
use crate::value::iterator::IteratorObject;
use crate::value::object::ClassId;
use crate::value::object::InstanceObject;
use crate::value::object::TypeEnvironmentId;
use crate::value::ops;
use crate::value::string::ByteStringObject;
use crate::value::tuple::TupleObject;
use crate::value::vec::VecObject;
use crate::vm::arithmetic::arithmetic_add;
use crate::vm::arithmetic::arithmetic_divide;
use crate::vm::arithmetic::arithmetic_modulo;
use crate::vm::arithmetic::arithmetic_multiply;
use crate::vm::arithmetic::arithmetic_power;
use crate::vm::arithmetic::arithmetic_subtract;
use crate::vm::arithmetic::bitwise_and;
use crate::vm::arithmetic::bitwise_or;
use crate::vm::arithmetic::bitwise_xor;
use crate::vm::arithmetic::compare_greater;
use crate::vm::arithmetic::compare_greater_or_equal;
use crate::vm::arithmetic::compare_less;
use crate::vm::arithmetic::compare_less_or_equal;
use crate::vm::arithmetic::compare_spaceship;
use crate::vm::arithmetic::concatenate;
use crate::vm::arithmetic::concatenate_left_constant;
use crate::vm::arithmetic::concatenate_right_constant;
use crate::vm::arithmetic::integer_add;
use crate::vm::arithmetic::integer_modulo;
use crate::vm::arithmetic::integer_multiply;
use crate::vm::arithmetic::integer_shift_left;
use crate::vm::arithmetic::integer_shift_right;
use crate::vm::arithmetic::integer_subtract;
use crate::vm::arithmetic::negate;
use crate::vm::arithmetic::shift_left;
use crate::vm::arithmetic::shift_right;
use crate::vm::arithmetic::step_by;
use crate::vm::arrays::IndexAddFault;
use crate::vm::arrays::advance_cursor;
use crate::vm::arrays::advance_dict_cursor;
use crate::vm::arrays::advance_dict_cursor_int_values;
use crate::vm::arrays::advance_vec_cursor;
use crate::vm::arrays::advance_vec_int_cursor;
use crate::vm::arrays::append_value;
use crate::vm::arrays::array_length;
use crate::vm::arrays::dict_add_assign_any_key_int_value;
use crate::vm::arrays::dict_add_assign_string_key_int_value;
use crate::vm::arrays::dict_index_get_int_key;
use crate::vm::arrays::dict_index_get_int_key_int_value;
use crate::vm::arrays::dict_index_get_string_key;
use crate::vm::arrays::dict_index_set_int_key;
use crate::vm::arrays::dict_index_set_string_key;
use crate::vm::arrays::dict_key;
use crate::vm::arrays::index_add_assign;
use crate::vm::arrays::index_get;
use crate::vm::arrays::index_replace_existing;
use crate::vm::arrays::index_set;
use crate::vm::arrays::index_set_reversible;
use crate::vm::arrays::int_position;
use crate::vm::arrays::remove_end;
use crate::vm::arrays::remove_entry;
use crate::vm::arrays::rollback_index_set;
use crate::vm::arrays::spread_into;
use crate::vm::arrays::swap_remove_entry;
use crate::vm::arrays::vec_append;
use crate::vm::arrays::vec_index_get;
use crate::vm::arrays::vec_index_set;
use crate::vm::arrays::vec_int_index_get;
use crate::vm::built_in::reduce_signature;
use crate::vm::errors::debug_render;
use crate::vm::errors::literal_text;
use crate::vm::errors::visibility_name;

#[macro_use]
mod macros;

pub(crate) mod arithmetic;
pub(crate) mod arrays;
pub(crate) mod async_runtime;
pub(crate) mod attributes;
pub(crate) mod built_in;
pub(crate) mod call;
pub(crate) mod coroutines;
pub(crate) mod embed;
pub(crate) mod errors;
pub(crate) mod execute;
pub(crate) mod finalizers;
pub(crate) mod lvalue;
pub(crate) mod method;
pub(crate) mod numeric_loop;
pub(crate) mod objects;
pub(crate) mod property_update;
pub(crate) mod refine;
pub(crate) mod resolve;
pub(crate) mod types;
pub(crate) mod units;

pub(crate) enum VirtualMachineControl {
    Throw(Value),
    Exit(u8),
}

#[derive(Clone, Copy)]
struct OptionalFuncId(Option<NonZeroU32>);

impl OptionalFuncId {
    const NONE: OptionalFuncId = OptionalFuncId(None);

    fn some(value: FuncId) -> OptionalFuncId {
        // SAFETY: the engine never holds u32::MAX functions, so the offset
        // increment cannot overflow.
        let incremented = unsafe {
            unwrap_option_invariant(
                value.0.checked_add(1),
                "a function identifier fits below u32::MAX",
            )
        };

        OptionalFuncId(NonZeroU32::new(incremented))
    }

    fn get(self) -> Option<FuncId> {
        self.0.map(|value| FuncId(value.get() - 1))
    }
}

#[derive(Clone, Copy)]
struct OptionalClassId(Option<NonZeroU32>);

impl OptionalClassId {
    const NONE: OptionalClassId = OptionalClassId(None);

    fn from_option(value: Option<ClassId>) -> OptionalClassId {
        match value {
            Some(value) => OptionalClassId::some(value),
            None => OptionalClassId::NONE,
        }
    }

    fn some(value: ClassId) -> OptionalClassId {
        // SAFETY: the engine never holds u32::MAX classes, so the offset
        // increment cannot overflow.
        let incremented = unsafe {
            unwrap_option_invariant(
                value.0.checked_add(1),
                "a class identifier fits below u32::MAX",
            )
        };

        OptionalClassId(NonZeroU32::new(incremented))
    }

    fn get(self) -> Option<ClassId> {
        self.0.map(|value| ClassId(value.get() - 1))
    }
}

#[derive(Clone, Copy)]
struct FrameFlags(u8);

impl FrameFlags {
    const HAS_THIS: u8 = 1;
    const BORROWS_REGISTER_ZERO: u8 = 1 << 1;
    const IN_CONSTRUCTOR: u8 = 1 << 2;
    const SCALAR_RETURN_TARGET: u8 = 1 << 3;
    const DISCARD_RESULT: u8 = 1 << 4;
    const ITERATOR_STEP: u8 = 1 << 5;

    fn new(has_this: bool, borrows_register_zero: bool, in_constructor: bool) -> FrameFlags {
        FrameFlags(
            (u8::from(has_this) * Self::HAS_THIS)
                | (u8::from(borrows_register_zero) * Self::BORROWS_REGISTER_ZERO)
                | (u8::from(in_constructor) * Self::IN_CONSTRUCTOR),
        )
    }

    fn has_this(self) -> bool {
        self.0 & Self::HAS_THIS != 0
    }

    fn borrows_register_zero(self) -> bool {
        self.0 & Self::BORROWS_REGISTER_ZERO != 0
    }

    fn in_constructor(self) -> bool {
        self.0 & Self::IN_CONSTRUCTOR != 0
    }

    fn with_scalar_return_target(mut self, enabled: bool) -> FrameFlags {
        self.0 |= u8::from(enabled) * Self::SCALAR_RETURN_TARGET;
        self
    }

    fn scalar_return_target(self) -> bool {
        self.0 & Self::SCALAR_RETURN_TARGET != 0
    }

    fn with_discard_result(mut self, enabled: bool) -> FrameFlags {
        self.0 |= u8::from(enabled) * Self::DISCARD_RESULT;
        self
    }

    fn discards_result(self) -> bool {
        self.0 & Self::DISCARD_RESULT != 0
    }

    fn with_iterator_step(mut self) -> FrameFlags {
        self.0 |= Self::ITERATOR_STEP;
        self
    }

    fn iterator_step(self) -> bool {
        self.0 & Self::ITERATOR_STEP != 0
    }
}

#[derive(Clone, Copy)]
struct Frame {
    chunk: NonNull<Chunk>,
    /// The body's inline-cache slots.
    cache: NonNull<InlineCache>,
    unit: NonNull<UnitContext>,
    function: OptionalFuncId,
    ip: u32,
    base: u32,
    argc: u16,
    called_class: OptionalClassId,
    class_scope: OptionalClassId,
    stack_floor_offset: u16,
    /// Registers that may own reference-counted values. Wider frames use
    /// ordinary full-window teardown instead.
    reference_register_mask: u64,
    return_register: u16,
    flags: FrameFlags,
    type_environment: TypeEnvironmentId,
}

const _: () = assert!(size_of::<Frame>() == 64);

impl Frame {
    fn has_this(&self) -> bool {
        self.flags.has_this()
    }

    fn borrows_register_zero(&self) -> bool {
        self.flags.borrows_register_zero()
    }

    fn iterator_step(&self) -> bool {
        self.flags.iterator_step()
    }

    fn in_constructor(&self) -> bool {
        self.flags.in_constructor()
    }

    fn scalar_return_target(&self) -> bool {
        self.flags.scalar_return_target()
    }

    fn discards_result(&self) -> bool {
        self.flags.discards_result()
    }

    fn stack_floor(&self) -> u32 {
        self.base - u32::from(self.stack_floor_offset)
    }
}

fn frame_argument_count(count: usize) -> u16 {
    // SAFETY: an argument count is bounded by the register address space,
    // which the verifier caps below u16::MAX.
    unsafe {
        unwrap_result_invariant(
            u16::try_from(count),
            "a frame's arguments fit the register address space",
        )
    }
}

fn frame_stack_floor_offset(base: usize, stack_floor: u32) -> u16 {
    // SAFETY: the register stack, the frame's floor position within it, and
    // the resulting synthetic window are all bounded by the verified frame
    // layout, so none of the conversions can fail.
    unsafe {
        let base = unwrap_result_invariant(
            u32::try_from(base),
            "the register stack fits the frame address space",
        );
        let offset = unwrap_option_invariant(
            base.checked_sub(stack_floor),
            "a frame's stack floor does not follow its register base",
        );
        unwrap_result_invariant(
            u16::try_from(offset),
            "a synthetic argument window fits the register address space",
        )
    }
}

/// The small subset of a frame needed after it leaves the call stack.
#[derive(Clone, Copy)]
struct FrameTeardown {
    chunk: NonNull<Chunk>,
    base: u32,
    stack_floor: u32,
    reference_register_mask: u64,
    return_register: u16,
    borrows_register_zero: bool,
}

impl FrameTeardown {
    fn from_frame(frame: &Frame) -> FrameTeardown {
        FrameTeardown {
            chunk: frame.chunk,
            base: frame.base,
            stack_floor: frame.stack_floor(),
            reference_register_mask: frame.reference_register_mask,
            return_register: frame.return_register,
            borrows_register_zero: frame.borrows_register_zero(),
        }
    }
}

struct PendingUnwind {
    frame: usize,
    value: Value,
}

struct PendingDiscardedResult {
    callable: Atom,
    note: Option<Atom>,
}

struct CalleeShape {
    target: CallTarget,
    this: Option<ManagedRef<InstanceObject>>,
    holder: Option<ManagedRef<FunctionObject>>,
    method: Option<MethodContext>,
}

enum ArgumentSlot {
    Filled(Value),
    Hole(u32),
    /// A position never bound: it stays missing unless a named argument
    /// lands on it.
    Gap,
}

#[derive(Clone, Copy)]
struct MethodContext {
    scope: ClassId,
    called: ClassId,
    is_constructor: bool,
}

struct UserCallContext<'a> {
    function: FuncId,
    this: Option<ManagedRef<InstanceObject>>,
    captures: &'a [Value],
    arguments: &'a [Value],
    method: Option<MethodContext>,
    declared_scope: Option<ClassId>,
    type_environment: TypeEnvironmentId,
    type_arguments_bound: bool,
}

#[derive(Clone, Copy, PartialEq, Eq, Hash)]
struct RegionSite {
    chunk: NonNull<Chunk>,
    instruction: usize,
}

impl RegionSite {
    fn new(chunk: &Chunk, instruction: usize) -> RegionSite {
        RegionSite {
            chunk: NonNull::from(chunk),
            instruction,
        }
    }
}

pub(crate) struct VirtualMachine<'engine> {
    pub(crate) engine: &'engine mut Engine,
    /// The value heap, shared with the engine; held directly so value
    /// construction reaches it without borrowing the engine.
    heap: Rc<Heap>,
    frames: Vec<Frame>,
    /// The shared register stack; each frame owns the window starting at its
    /// base. It grows at call boundaries only, and every raw pointer into it
    /// is recomputed when `'dispatch` reloads.
    stack: Vec<Value>,
    stack_initialized_len: usize,
    pending_unwinds: Vec<PendingUnwind>,
    /// The exit code smuggled across a built-in boundary: a handler cannot
    /// carry [`VirtualMachineControl::Exit`] through its `Result`, so the callee records
    /// it here and the call site converts it after the handler returns.
    pending_exit: Option<u8>,
    pending_discarded_result: Option<PendingDiscardedResult>,
    region_jump_strikes: HashMap<RegionSite, u32>,
    #[expect(
        clippy::vec_box,
        reason = "active frames retain pointers to refined chunks while the vector grows"
    )]
    refined_chunks: Vec<Box<Chunk>>,
    #[expect(
        clippy::vec_box,
        reason = "active frames retain pointers to refined caches while the vector grows"
    )]
    refined_caches: Vec<Box<InlineCache>>,
    world_refinement_pending: bool,
    draining_finalizers: bool,
}

fn find_double_colon(bytes: &[u8]) -> Option<usize> {
    bytes.windows(2).position(|pair| pair == b"::")
}

/// The member name atom of an inline-cache site.
fn name_atom(chunk: &Chunk, site: usize) -> &Atom {
    match &chunk.ic_descriptors[site] {
        IcDescriptor::Member { name, .. } => name,
        // SAFETY: the surrounding invariant makes this path unreachable.
        IcDescriptor::ClassMember { .. } => unsafe {
            unreachable_invariant("the site resolves a member descriptor")
        },
    }
}

fn site_type_arguments(chunk: &Chunk, site: usize) -> Option<&[TypeDescriptor]> {
    match &chunk.ic_descriptors[site] {
        IcDescriptor::Member { type_arguments, .. }
        | IcDescriptor::ClassMember { type_arguments, .. } => type_arguments.as_deref(),
    }
}

/// The class and member atoms of a class-member inline-cache site.
fn class_member_atoms(chunk: &Chunk, site: usize) -> (&Atom, &Atom) {
    match &chunk.ic_descriptors[site] {
        IcDescriptor::ClassMember { class, member, .. } => (class, member),
        // SAFETY: the surrounding invariant makes this path unreachable.
        IcDescriptor::Member { .. } => unsafe {
            unreachable_invariant("the site resolves a class-member descriptor")
        },
    }
}

/// The class and member texts of a class-member inline-cache site.
fn class_member_names(chunk: &Chunk, site: usize) -> (String, String) {
    let (class, member) = class_member_atoms(chunk, site);
    (
        class.to_string_lossy().into_owned(),
        member.to_string_lossy().into_owned(),
    )
}

pub(crate) struct ArrayFault {
    kind: FaultKind,
    message: String,
}

#[derive(Clone, Copy)]
pub(crate) enum FaultKind {
    TypeError,
    OutOfBounds,
}

impl ArrayFault {
    fn type_error(message: String) -> ArrayFault {
        ArrayFault {
            kind: FaultKind::TypeError,
            message,
        }
    }

    fn out_of_bounds(message: String) -> ArrayFault {
        ArrayFault {
            kind: FaultKind::OutOfBounds,
            message,
        }
    }
}

pub(crate) enum Fault {
    Incompatible,
    /// Compatible numeric operands cannot be ordered because one is NaN.
    Unordered,
    DivisionByZero,
    Overflow,
    Underflow,
    ShiftRange,
}

impl<'engine> VirtualMachine<'engine> {
    pub(crate) fn new(engine: &'engine mut Engine) -> VirtualMachine<'engine> {
        let heap = Rc::clone(&engine.heap);
        VirtualMachine {
            engine,
            heap,
            // Linking also uses a VM for type checks, without executing a frame.
            // Execution grows this buffer on demand and reuses it between calls.
            frames: Vec::new(),
            stack: Vec::new(),
            stack_initialized_len: 0,
            pending_unwinds: Vec::new(),
            pending_exit: None,
            pending_discarded_result: None,
            region_jump_strikes: HashMap::new(),
            refined_chunks: Vec::new(),
            refined_caches: Vec::new(),
            world_refinement_pending: false,
            draining_finalizers: false,
        }
    }

    #[must_use]
    pub(crate) fn intern(&self, bytes: &[u8]) -> Atom {
        self.heap.intern(bytes)
    }

    #[must_use]
    pub(crate) fn heap(&self) -> &Heap {
        &self.heap
    }

    #[inline(always)]
    fn begin_discarded_result_check(&mut self) {
        self.pending_discarded_result = None;
    }

    #[inline]
    pub(crate) fn remember_built_in_must_use(&mut self, name: &str) {
        let frame = self.current_frame();
        // SAFETY: verified bytecode and VM state prove the index, type, and lifetime.
        let chunk = unsafe { frame.chunk.as_ref() };
        if !matches!(
            chunk.code.get(frame.ip as usize),
            Some(Instruction::CheckDiscardedResult { .. })
        ) {
            return;
        }

        self.pending_discarded_result = Some(PendingDiscardedResult {
            callable: self.heap.intern(name.as_bytes()),
            note: None,
        });
    }

    #[inline(always)]
    fn remember_discarded_result(&mut self) {
        let frame = self.current_frame();
        if !frame.discards_result() {
            return;
        }
        let Some(function) = frame.function.get() else {
            // SAFETY: the surrounding invariant makes this path unreachable.
            unsafe { unreachable_invariant("only a user callable frame can discard its result") }
        };
        let runtime = &self.engine.tables.functions[function.0 as usize];
        self.pending_discarded_result =
            must_use_note(runtime.attributes()).map(|note| PendingDiscardedResult {
                callable: runtime.name.clone(),
                note,
            });
    }

    #[inline(always)]
    fn remember_frameless_discarded_result(&mut self, function: FuncId, discarded: bool) {
        if !discarded {
            return;
        }
        let runtime = &self.engine.tables.functions[function.0 as usize];
        self.pending_discarded_result =
            must_use_note(runtime.attributes()).map(|note| PendingDiscardedResult {
                callable: runtime.name.clone(),
                note,
            });
    }

    pub(crate) fn run_main(
        &mut self,
        context: &Rc<UnitContext>,
    ) -> Result<Value, VirtualMachineControl> {
        let chunk = context.main_chunk;
        self.push_detached_frame(chunk, NonNull::from(&*context.main_cache), context);
        self.run(0)
    }

    pub(crate) fn run_initializer(
        &mut self,
        chunk: NonNull<Chunk>,
        context: &Rc<UnitContext>,
    ) -> Result<Value, VirtualMachineControl> {
        let cache = Box::new(InlineCache::new());
        self.push_detached_frame(chunk, NonNull::from(&*cache), context);
        let floor = self.frames.len() - 1;
        let result = self.run(floor);
        drop(cache);
        result
    }

    fn push_detached_frame(
        &mut self,
        chunk: NonNull<Chunk>,
        cache: NonNull<InlineCache>,
        context: &Rc<UnitContext>,
    ) {
        // SAFETY: verified bytecode and VM state prove the index, type, and lifetime.
        let register_count = usize::from(unsafe { chunk.as_ref() }.register_count)
            + usize::from(MAIN_FRAME_REGISTER_HEADROOM);
        let base = self.stack.len();
        self.resize_frame_stack(base + register_count);
        self.reset_uninitialized_locals(base, chunk);
        self.frames.push(Frame {
            chunk,
            cache,
            unit: NonNull::from(&**context),
            function: OptionalFuncId::NONE,
            ip: 0,
            base: base as u32,
            argc: 0,
            called_class: OptionalClassId::NONE,
            class_scope: OptionalClassId::NONE,
            stack_floor_offset: 0,
            // SAFETY: verified bytecode and VM state prove the index, type, and lifetime.
            reference_register_mask: unsafe { chunk.as_ref() }.reference_register_mask,
            return_register: 0,
            flags: FrameFlags::new(false, false, false),
            type_environment: TypeEnvironmentId::default(),
        });
    }

    /// Extends the active stack into storage left initialized by an earlier
    /// narrow frame, initializing only a genuinely new high-water suffix.
    #[inline(always)]
    fn resize_frame_stack(&mut self, new_len: usize) {
        if new_len <= self.stack_initialized_len {
            // SAFETY: verified bytecode and VM state prove the index, type, and lifetime.
            unsafe { self.stack.set_len(new_len) };
            return;
        }

        self.stack.resize_with(new_len, Value::uninitialized);
        self.stack_initialized_len = new_len;
    }

    /// Resets locals that are not initialized by parameters, captures, or
    /// `$this` before a reused frame starts.
    #[inline(always)]
    fn reset_uninitialized_locals(&mut self, base: usize, chunk: NonNull<Chunk>) {
        // SAFETY: verified bytecode and VM state prove the index, type, and lifetime.
        for register in &unsafe { chunk.as_ref() }.uninitialized_registers {
            // SAFETY: verified bytecode and VM state prove the index, type, and lifetime.
            unsafe {
                self.stack
                    .as_mut_ptr()
                    .add(base + register.index() as usize)
                    .write(Value::uninitialized());
            }
        }
    }

    /// Clears trailing parameters omitted by the caller so the callee's
    /// `FillDefault` prologue never observes scalar bits left by an earlier
    /// frame that occupied the same stack window.
    #[inline(always)]
    fn reset_omitted_parameters(
        &mut self,
        base: usize,
        offset: usize,
        argc: usize,
        declared: usize,
    ) {
        for position in argc..declared {
            // SAFETY: verified bytecode and VM state prove the index, type, and lifetime.
            unsafe {
                self.stack
                    .as_mut_ptr()
                    .add(base + offset + position)
                    .write(Value::uninitialized());
            }
        }
    }

    /// Retains original values only for parameters whose source variables can
    /// be overwritten. Trace-only slots are ordinary frame locals, so their
    /// ownership participates in the same narrow-frame teardown mask.
    #[inline(always)]
    fn snapshot_trace_arguments(
        &mut self,
        base: usize,
        chunk: NonNull<Chunk>,
        argc: usize,
        reference_register_mask: &mut u64,
    ) {
        // SAFETY: verified bytecode and VM state prove the index, type, and lifetime.
        let chunk = unsafe { chunk.as_ref() };
        let count = argc.min(chunk.trace_argument_registers.len());
        for (position, target) in chunk.trace_argument_registers[..count]
            .iter()
            .copied()
            .enumerate()
        {
            if target == Register::NONE {
                continue;
            }

            let source = base + usize::from(chunk.parameter_register_start) + position;
            let value = self.stack[source].clone();
            if chunk.register_count <= REFERENCE_REGISTER_LIMIT && value.is_reference_counted() {
                *reference_register_mask |= 1u64 << target.index();
            }
            self.stack[base + usize::from(target.index())] = value;
        }
    }

    /// Releases a compiler-owned call window after a synchronous built-in call.
    fn clear_argument_window(&mut self, start: usize, count: usize) {
        for slot in &mut self.stack[start..start + count] {
            if slot.is_reference_counted() {
                *slot = Value::uninitialized();
            }
        }
    }

    /// Combines the storage bound and the current configured depth limit into
    /// one call-entry guard, including when a suspended VM resumes after the
    /// host reconfigured its engine.
    #[inline(always)]
    fn call_frame_limit(&self) -> usize {
        self.frames
            .capacity()
            .min(self.engine.configuration.call_depth_limit)
    }

    /// Reserves the next call frame, or reports the existing hard depth limit.
    #[cold]
    #[inline(never)]
    fn grow_call_frames(&mut self) -> Result<(), VirtualMachineControl> {
        let length = self.frames.len();
        let limit = self.engine.configuration.call_depth_limit;
        if length >= limit {
            return Err(self.call_depth_exceeded());
        }
        if length == self.frames.capacity() {
            let capacity = self.frames.capacity().saturating_mul(2).max(4).min(limit);
            self.frames.reserve_exact(capacity - length);
        }
        Ok(())
    }

    /// Pushes after the caller checked the combined storage and depth limit.
    #[inline(always)]
    unsafe fn push_frame_unchecked(&mut self, frame: Frame) {
        let length = self.frames.len();
        debug_assert!(length < self.frames.capacity());
        // SAFETY: the call-entry guard reserved storage before any reentrant
        // helpers ran. They may grow storage but cannot shrink its capacity.
        unsafe {
            self.frames.as_mut_ptr().add(length).write(frame);
            self.frames.set_len(length + 1);
        }
    }

    /// Appends a synthetic argument without discarding reusable hidden frame
    /// storage. Hidden values are always scalar or uninitialized.
    #[inline(always)]
    fn push_stack_value(&mut self, value: Value) {
        let index = self.stack.len();
        if index < self.stack_initialized_len {
            // SAFETY: verified bytecode and VM state prove the index, type, and lifetime.
            unsafe {
                self.stack.as_mut_ptr().add(index).write(value);
                self.stack.set_len(index + 1);
            }
            return;
        }

        self.stack.push(value);
        self.stack_initialized_len = self.stack.len();
    }

    #[inline(always)]
    fn truncate_stack(&mut self, len: usize) {
        self.stack.truncate(len);
        self.stack_initialized_len = len;
    }

    #[inline(always)]
    fn current_frame(&self) -> &Frame {
        match self.frames.last() {
            Some(frame) => frame,
            // SAFETY: the surrounding invariant makes this path unreachable.
            None => unsafe { unreachable_invariant("the dispatch loop always has a frame") },
        }
    }

    #[inline(always)]
    fn current_base(&self) -> usize {
        self.current_frame().base as usize
    }

    #[inline(always)]
    fn current_frame_mut(&mut self) -> &mut Frame {
        match self.frames.last_mut() {
            Some(frame) => frame,
            // SAFETY: the surrounding invariant makes this path unreachable.
            None => unsafe { unreachable_invariant("the dispatch loop always has a frame") },
        }
    }

    fn current_this(&self) -> Option<&ManagedRef<InstanceObject>> {
        let frame = self.current_frame();
        if !frame.has_this() {
            return None;
        }

        self.stack[frame.base as usize]
            .as_object()
            // SAFETY: the surrounding invariant makes this path unreachable.
            .or_else(|| unsafe {
                unreachable_invariant("a frame with `$this` owns an object in register zero")
            })
    }

    #[inline(always)]
    fn sync_ip(&mut self, ip: usize) {
        self.current_frame_mut().ip = ip as u32;
    }
}
