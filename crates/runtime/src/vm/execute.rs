//! The dispatch loop: fetching instructions, returning from a frame, and
//! unwinding a throw.

use std::cmp::Ordering;
use std::iter::repeat_n;
use std::mem;
use std::ptr;
use std::rc::Rc;
use std::slice;

use crate::bytecode::REFERENCE_REGISTER_LIMIT;
use crate::bytecode::chunk::descriptors::FloatPairUpdateDescriptor;
use crate::bytecode::chunk::descriptors::FloatSquaresSumBranchDescriptor;
use crate::bytecode::chunk::descriptors::IntStepLoopDescriptor;
use crate::bytecode::chunk::descriptors::PreparedIntLoopDescriptor;
use crate::bytecode::chunk::descriptors::PropertyInitializationEntry;
use crate::bytecode::chunk::descriptors::ShapeKey;
use crate::bytecode::chunk::descriptors::SwitchTable;
use crate::bytecode::chunk::descriptors::check_trivial_descriptor;
use crate::bytecode::chunk::descriptors::string_switch_lookup;
use crate::bytecode::instruction::operands::ArrayValueMode;
use crate::bytecode::instruction::operands::Comparison as BytecodeComparison;
use crate::bytecode::instruction::operands::IndexAddMode;
use crate::bytecode::instruction::operands::PropertyIndexUpdateMode;
use crate::bytecode::instruction::operands::PropertyInitializationDescriptorIndex;
use crate::bytecode::instruction::operands::PropertyReadMode;
use crate::bytecode::instruction::operands::PropertySlot;
use crate::bytecode::instruction::operands::PropertyValueMode;
use crate::bytecode::unit::ClassLikeKind;
use crate::bytecode::unit::literal_value;
use crate::core::private::syscall::StandardStream;
use crate::engine::Engine;
use crate::unwrap_option_invariant;
use crate::value::ValueView;
use crate::value::heap::metadata::HeapBox;
use crate::vm::AsMode;
use crate::vm::ByteStringObject;
use crate::vm::CachedIsCheck;
use crate::vm::CallTarget;
use crate::vm::Chunk;
use crate::vm::ClassId;
use crate::vm::DictObject;
use crate::vm::Fault;
use crate::vm::FrameTeardown;
use crate::vm::FuncId;
use crate::vm::FunctionObject;
use crate::vm::IndexAddFault;
use crate::vm::InstanceObject;
use crate::vm::Instruction;
use crate::vm::InstructionKind;
use crate::vm::InstructionWord;
use crate::vm::IsCheckWays;
use crate::vm::Literal;
use crate::vm::ManagedRef;
use crate::vm::NonNull;
use crate::vm::PendingUnwind;
use crate::vm::RegionSite;
use crate::vm::Register;
use crate::vm::TupleObject;
use crate::vm::TypeDescriptor;
use crate::vm::TypeEnvironmentId;
use crate::vm::Value;
use crate::vm::VecObject;
use crate::vm::VirtualMachine;
use crate::vm::VirtualMachineControl;
use crate::vm::advance_cursor;
use crate::vm::advance_dict_cursor;
use crate::vm::advance_dict_cursor_int_values;
use crate::vm::advance_vec_cursor;
use crate::vm::advance_vec_int_cursor;
use crate::vm::append_value;
use crate::vm::arithmetic_add;
use crate::vm::arithmetic_divide;
use crate::vm::arithmetic_modulo;
use crate::vm::arithmetic_multiply;
use crate::vm::arithmetic_power;
use crate::vm::arithmetic_subtract;
use crate::vm::array_length;
use crate::vm::arrays::array_contains;
use crate::vm::arrays::array_contains_key;
use crate::vm::arrays::array_length_hint;
use crate::vm::arrays::dict_index_set;
use crate::vm::arrays::int_position;
use crate::vm::arrays::reserve_array_hint;
use crate::vm::bitwise_and;
use crate::vm::bitwise_or;
use crate::vm::bitwise_xor;
use crate::vm::class_member_names;
use crate::vm::compare_greater;
use crate::vm::compare_greater_or_equal;
use crate::vm::compare_less;
use crate::vm::compare_less_or_equal;
use crate::vm::compare_spaceship;
use crate::vm::concatenate;
use crate::vm::concatenate_left_constant;
use crate::vm::concatenate_right_constant;
use crate::vm::debug_render;
use crate::vm::dict_add_assign_any_key_int_value;
use crate::vm::dict_add_assign_string_key_int_value;
use crate::vm::dict_index_get_int_key;
use crate::vm::dict_index_get_int_key_int_value;
use crate::vm::dict_index_get_string_key;
use crate::vm::dict_index_set_int_key;
use crate::vm::dict_index_set_string_key;
use crate::vm::dict_key;
use crate::vm::index_add_assign;
use crate::vm::index_get;
use crate::vm::index_replace_existing;
use crate::vm::index_set;
use crate::vm::integer_add;
use crate::vm::integer_modulo;
use crate::vm::integer_multiply;
use crate::vm::integer_shift_left;
use crate::vm::integer_shift_right;
use crate::vm::integer_subtract;
use crate::vm::literal_text;
use crate::vm::name_atom;
use crate::vm::negate;
use crate::vm::numeric_loop::NumericLoopOutcome;
use crate::vm::ops;
use crate::vm::remove_end;
use crate::vm::remove_entry;
use crate::vm::shift_left;
use crate::vm::shift_right;
use crate::vm::spread_into;
use crate::vm::step_by;
use crate::vm::swap_remove_entry;
use crate::vm::unreachable_invariant;
use crate::vm::vec_append;
use crate::vm::vec_index_get;
use crate::vm::vec_index_set;
use crate::vm::vec_int_index_get;

enum ForeachAdvance {
    Array(Option<(Value, Value)>),
    Object {
        instance: NonNull<HeapBox<InstanceObject>>,
        next: Option<(FuncId, ClassId)>,
        environment: TypeEnvironmentId,
    },
    Returned(Value),
}

#[inline(always)]
fn register_mask(register: Register) -> u64 {
    let index = register.index();
    if index < REFERENCE_REGISTER_LIMIT {
        1u64 << index
    } else {
        0
    }
}

/// The relative target a `SwitchInt` dispatch takes: the matching table
/// entry when the subject is an int within the table, the default
/// otherwise. Match uses `==` semantics, so a non-int subject matches no
/// int arm and simply takes the default path.
fn switch_int_target(table: &SwitchTable, subject: &Value) -> i32 {
    let SwitchTable::Int {
        base,
        targets,
        default,
    } = table
    else {
        // SAFETY: the surrounding invariant makes this path unreachable.
        unsafe { unreachable_invariant("a SwitchInt site references an int table") }
    };
    let Some(subject) = subject.as_int() else {
        return *default;
    };
    let Some(offset) = subject
        .checked_sub(*base)
        .and_then(|offset| usize::try_from(offset).ok())
    else {
        return *default;
    };
    if offset >= targets.len() {
        *default
    } else {
        // SAFETY: the surrounding invariant keeps this index in bounds.
        unsafe { *targets.get_unchecked(offset) }
    }
}

/// The relative target a `SwitchString` dispatch takes; a non-string
/// subject matches no arm.
fn switch_string_target(table: &SwitchTable, subject: &Value) -> i32 {
    let Some(bytes) = subject.as_string_bytes() else {
        return match table {
            SwitchTable::String { default, .. } | SwitchTable::StringByte { default, .. } => {
                *default
            }
            SwitchTable::Int { .. }
            | SwitchTable::Pattern { .. }
            | SwitchTable::DictionaryShape { .. }
            | SwitchTable::Bool { .. }
            // SAFETY: the surrounding invariant makes this path unreachable.
            | SwitchTable::Float { .. } => unsafe {
                unreachable_invariant("a SwitchString site references a string table")
            },
        };
    };

    match table {
        SwitchTable::String {
            arms,
            buckets,
            default,
        } => string_switch_lookup(arms, buckets, bytes).map_or(*default, |index| arms[index].1),
        SwitchTable::StringByte {
            base,
            targets,
            default,
        } => {
            let [byte] = bytes else {
                return *default;
            };
            let Some(offset) = byte.checked_sub(*base) else {
                return *default;
            };

            let offset = usize::from(offset);
            if offset < targets.len() {
                // SAFETY: the surrounding invariant keeps this index in bounds.
                unsafe { *targets.get_unchecked(offset) }
            } else {
                *default
            }
        }
        SwitchTable::Int { .. }
        | SwitchTable::Pattern { .. }
        | SwitchTable::DictionaryShape { .. }
        | SwitchTable::Bool { .. }
        // SAFETY: the surrounding invariant makes this path unreachable.
        | SwitchTable::Float { .. } => unsafe {
            unreachable_invariant("a SwitchString site references a string table")
        },
    }
}

fn switch_pattern_target(table: &SwitchTable, subject: &Value) -> i32 {
    match table {
        SwitchTable::Pattern {
            descriptors,
            targets,
            default,
        } => {
            for (descriptor, target) in descriptors.iter().zip(targets) {
                match check_trivial_descriptor(descriptor, subject) {
                    Some(true) => return *target,
                    Some(false) => {}
                    // SAFETY: the surrounding invariant makes this path unreachable.
                    None => unsafe {
                        unreachable_invariant("a pattern switch contains only trivial descriptors")
                    },
                }
            }

            *default
        }
        SwitchTable::DictionaryShape {
            keys,
            patterns,
            targets,
            default,
        } => {
            let Some(dictionary) = subject.as_dict() else {
                return *default;
            };
            if dictionary.len() != keys.len() {
                return *default;
            }

            let mut values = [None; 8];
            for (position, key) in keys.iter().enumerate() {
                values[position] = match key {
                    ShapeKey::Int(key) => dictionary.get_int(*key),
                    ShapeKey::String(key) => dictionary.get_string(key.as_handle()),
                };
                if values[position].is_none() {
                    return *default;
                }
            }

            for (pattern, target) in patterns.iter().zip(targets) {
                let matches = pattern.iter().enumerate().all(|(position, descriptor)| {
                    // SAFETY: the surrounding invariant makes this path unreachable.
                    let value = values[position].unwrap_or_else(|| unsafe {
                        unreachable_invariant("a dictionary shape switch found every shared key")
                    });
                    // SAFETY: the surrounding invariant makes this path unreachable.
                    check_trivial_descriptor(descriptor, value).unwrap_or_else(|| unsafe {
                        unreachable_invariant(
                            "a dictionary shape switch contains only trivial descriptors",
                        )
                    })
                });
                if matches {
                    return *target;
                }
            }

            *default
        }
        SwitchTable::Int { .. }
        | SwitchTable::String { .. }
        | SwitchTable::StringByte { .. }
        | SwitchTable::Bool { .. }
        // SAFETY: the surrounding invariant makes this path unreachable.
        | SwitchTable::Float { .. } => unsafe {
            unreachable_invariant("a SwitchPattern site references another switch table")
        },
    }
}

#[inline]
fn switch_float_target(table: &SwitchTable, subject: &Value) -> i32 {
    let SwitchTable::Float {
        values,
        targets,
        default,
    } = table
    else {
        // SAFETY: the surrounding invariant makes this path unreachable.
        unsafe { unreachable_invariant("a SwitchFloat site references a float table") }
    };
    let Some(subject) = subject.as_float() else {
        return *default;
    };
    values
        .iter()
        .zip(targets)
        .find_map(|(value, target)| (*value == subject).then_some(*target))
        .unwrap_or(*default)
}

fn tuple_window_matches(descriptor: &TypeDescriptor, elements: &[Value]) -> bool {
    match descriptor {
        TypeDescriptor::Tuple(descriptors) => {
            descriptors.len() == elements.len()
                && descriptors.iter().zip(elements).all(|(descriptor, value)| {
                    // SAFETY: the surrounding invariant makes this path unreachable.
                    check_trivial_descriptor(descriptor, value).unwrap_or_else(|| unsafe {
                        unreachable_invariant(
                            "a tuple pattern window contains only trivial descriptors",
                        )
                    })
                })
        }
        TypeDescriptor::TupleRest {
            elements: fixed,
            rest,
        } => {
            fixed.len() <= elements.len()
                && fixed.iter().zip(elements).all(|(descriptor, value)| {
                    // SAFETY: the surrounding invariant makes this path unreachable.
                    check_trivial_descriptor(descriptor, value).unwrap_or_else(|| unsafe {
                        unreachable_invariant(
                            "a tuple pattern window contains only trivial descriptors",
                        )
                    })
                })
                && elements.iter().skip(fixed.len()).all(|value| {
                    // SAFETY: the surrounding invariant makes this path unreachable.
                    check_trivial_descriptor(rest, value).unwrap_or_else(|| unsafe {
                        unreachable_invariant(
                            "a tuple pattern window contains only trivial descriptors",
                        )
                    })
                })
        }
        TypeDescriptor::Union(members) => members
            .iter()
            .any(|member| tuple_window_matches(member, elements)),
        TypeDescriptor::Intersection(members) => members
            .iter()
            .all(|member| tuple_window_matches(member, elements)),
        // SAFETY: the surrounding invariant makes this path unreachable.
        _ => unsafe {
            unreachable_invariant("a tuple pattern window contains only tuple descriptors")
        },
    }
}

fn switch_tuple_pattern_target(table: &SwitchTable, elements: &[Value]) -> i32 {
    let SwitchTable::Pattern {
        descriptors,
        targets,
        default,
    } = table
    else {
        // SAFETY: the surrounding invariant makes this path unreachable.
        unsafe { unreachable_invariant("a SwitchTuplePattern site references a pattern table") }
    };
    descriptors
        .iter()
        .zip(targets)
        .find_map(|(descriptor, target)| {
            tuple_window_matches(descriptor, elements).then_some(*target)
        })
        .unwrap_or(*default)
}

/// The jump target of a relative offset: the jumping instruction's own index
/// plus the offset.
fn jump_target(next_ip: usize, offset: i32) -> usize {
    (next_ip as i64 - 1 + i64::from(offset)) as usize
}

#[inline(always)]
unsafe fn fused_foreach_exit_offset(code: *const Instruction, ip: usize) -> i32 {
    // SAFETY: the caller provides a live instruction pointer and in-bounds index.
    let Instruction::Jump { offset } = (unsafe { *code.add(ip) }) else {
        // SAFETY: the surrounding invariant makes this path unreachable.
        unsafe { unreachable_invariant("a foreach next is followed by its fused jump") }
    };
    offset.offset()
}

#[inline(always)]
unsafe fn float_register(registers: *const Value, register: Register) -> f64 {
    // SAFETY: the caller guarantees an in-bounds register containing a float.
    unsafe { (&*registers.add(register.index() as usize)).as_float_unchecked() }
}

#[inline(always)]
unsafe fn int_register(registers: *const Value, register: Register) -> i64 {
    // SAFETY: the caller guarantees an in-bounds register containing an integer.
    unsafe { (&*registers.add(register.index() as usize)).as_int_unchecked() }
}

#[inline(always)]
unsafe fn object_register<'a>(
    registers: *const Value,
    register: Register,
) -> &'a ManagedRef<InstanceObject> {
    // SAFETY: the surrounding invariant makes this path unreachable.
    unsafe { (&*registers.add(register.index() as usize)).as_object() }.unwrap_or_else(|| unsafe {
        unreachable_invariant("a specialized property instruction received a non-object")
    })
}

#[inline]
fn comparison_matches(
    comparison: BytecodeComparison,
    left: &Value,
    right: &Value,
) -> Result<bool, Fault> {
    if let (ValueView::Float(left), ValueView::Float(right)) =
        (left.transparent(), right.transparent())
    {
        return Ok(float_comparison_matches(comparison, *left, *right));
    }

    if let (ValueView::Int(left), ValueView::Int(right)) = (left.transparent(), right.transparent())
    {
        return Ok(int_comparison_matches(comparison, *left, *right));
    }

    match comparison {
        BytecodeComparison::Equal => Ok(ops::equals(left, right)),
        BytecodeComparison::NotEqual => Ok(!ops::equals(left, right)),
        ordered => ops::compare(left, right)
            .map(|ordering| match ordered {
                BytecodeComparison::LessThan => matches!(ordering, Some(Ordering::Less)),
                BytecodeComparison::LessThanOrEqual => {
                    matches!(ordering, Some(Ordering::Less | Ordering::Equal))
                }
                BytecodeComparison::GreaterThan => matches!(ordering, Some(Ordering::Greater)),
                BytecodeComparison::GreaterThanOrEqual => {
                    matches!(ordering, Some(Ordering::Greater | Ordering::Equal))
                }
                // SAFETY: the surrounding invariant makes this path unreachable.
                _ => unsafe {
                    unreachable_invariant("equality comparisons take their direct path")
                },
            })
            .map_err(|_| Fault::Incompatible),
    }
}

#[inline(always)]
fn float_comparison_matches(comparison: BytecodeComparison, left: f64, right: f64) -> bool {
    match comparison {
        BytecodeComparison::Equal => left == right,
        BytecodeComparison::NotEqual => left != right,
        BytecodeComparison::LessThan => left < right,
        BytecodeComparison::LessThanOrEqual => left <= right,
        BytecodeComparison::GreaterThan => left > right,
        BytecodeComparison::GreaterThanOrEqual => left >= right,
    }
}

#[inline(always)]
fn int_comparison_matches(comparison: BytecodeComparison, left: i64, right: i64) -> bool {
    match comparison {
        BytecodeComparison::Equal => left == right,
        BytecodeComparison::NotEqual => left != right,
        BytecodeComparison::LessThan => left < right,
        BytecodeComparison::LessThanOrEqual => left <= right,
        BytecodeComparison::GreaterThan => left > right,
        BytecodeComparison::GreaterThanOrEqual => left >= right,
    }
}

#[inline(always)]
fn string_comparison_matches(comparison: BytecodeComparison, left: &[u8], right: &[u8]) -> bool {
    match comparison {
        BytecodeComparison::Equal => left == right,
        BytecodeComparison::NotEqual => left != right,
        BytecodeComparison::LessThan => left < right,
        BytecodeComparison::LessThanOrEqual => left <= right,
        BytecodeComparison::GreaterThan => left > right,
        BytecodeComparison::GreaterThanOrEqual => left >= right,
    }
}

enum NumericLoopTransition {
    Next(usize),
    Control {
        resume_ip: usize,
        control: VirtualMachineControl,
    },
}

impl VirtualMachine<'_> {
    unsafe fn continue_numeric_loop<const PREPARED_FLOATS: bool>(
        &mut self,
        chunk: &Chunk,
        registers: *mut Value,
        body: usize,
        exit: usize,
        float_registers: u64,
        dirty_registers: u64,
    ) -> NumericLoopTransition {
        // SAFETY: verified bytecode keeps operands in the live frame and proves their types.
        match unsafe {
            self.run_numeric_loop::<PREPARED_FLOATS>(
                chunk,
                registers,
                body,
                exit,
                float_registers,
                dirty_registers,
            )
        } {
            NumericLoopOutcome::Completed => NumericLoopTransition::Next(exit),
            NumericLoopOutcome::Deoptimize(target) => NumericLoopTransition::Next(target),
            NumericLoopOutcome::Fault {
                resume_ip,
                fault,
                operator,
                left,
                right,
            } => {
                // SAFETY: verified bytecode keeps operands in the live frame and proves their types.
                let left_kind = unsafe { (*registers.add(left.index() as usize)).kind_name() };
                let right_kind = match right {
                    // SAFETY: verified bytecode keeps operands in the live frame and proves their types.
                    Some(right) => unsafe { (*registers.add(right.index() as usize)).kind_name() },
                    None => "int",
                };

                NumericLoopTransition::Control {
                    resume_ip,
                    control: self.binary_fault(fault, operator, left_kind, right_kind),
                }
            }
            NumericLoopOutcome::Array { resume_ip, fault } => NumericLoopTransition::Control {
                resume_ip,
                control: self.array_fault(fault),
            },
        }
    }

    /// # Safety
    ///
    /// `registers` must be the live register window for `chunk`.
    #[inline(never)]
    #[expect(
        clippy::too_many_arguments,
        reason = "the entry mirrors the executor's full state"
    )]
    unsafe fn enter_numeric_region<const PREPARED_FLOATS: bool>(
        &mut self,
        chunk: &Chunk,
        registers: *mut Value,
        body: usize,
        exit: usize,
        float_registers: u64,
        dirty_registers: u64,
        site: RegionSite,
    ) -> NumericLoopTransition {
        debug_assert_eq!(site.chunk, NonNull::from(chunk));
        if !self.region_jump_strikes.is_empty()
            && self
                .region_jump_strikes
                .get(&site)
                .is_some_and(|strikes| *strikes >= 3)
        {
            return NumericLoopTransition::Next(body);
        }

        // SAFETY: verified bytecode keeps operands in the live frame and proves their types.
        let transition = unsafe {
            self.continue_numeric_loop::<PREPARED_FLOATS>(
                chunk,
                registers,
                body,
                exit,
                float_registers,
                dirty_registers,
            )
        };

        if let NumericLoopTransition::Next(next) = &transition {
            if *next != exit {
                // SAFETY: a numeric-loop transition always points inside the chunk.
                let terminal = matches!(
                    unsafe { chunk.code.get_unchecked(*next) },
                    Instruction::Return { .. }
                        | Instruction::ReturnUnchecked { .. }
                        | Instruction::ReturnReferenceUnchecked { .. }
                        | Instruction::ReturnPairUnchecked { .. }
                        | Instruction::ReturnScalarUnchecked { .. }
                        | Instruction::ReturnIntUnchecked { .. }
                        | Instruction::ReturnNull
                        | Instruction::ReturnNullUnchecked
                );

                if !terminal && self.region_jump_strikes.len() < 1024 {
                    *self.region_jump_strikes.entry(site).or_insert(0) += 1;
                }
            } else if !self.region_jump_strikes.is_empty() {
                self.region_jump_strikes.remove(&site);
            }
        }

        transition
    }
}

impl VirtualMachine<'_> {
    pub(crate) fn run(&mut self, floor: usize) -> Result<Value, VirtualMachineControl> {
        if self.heap.has_finalizable_objects() || self.heap.has_pending_finalizers() {
            self.run_dispatch::<true>(floor)
        } else {
            self.run_dispatch::<false>(floor)
        }
    }

    fn run_with_finalizers(&mut self, floor: usize) -> Result<Value, VirtualMachineControl> {
        self.run_dispatch::<true>(floor)
    }

    fn run_dispatch<const FINALIZERS: bool>(
        &mut self,
        floor: usize,
    ) -> Result<Value, VirtualMachineControl> {
        macro_rules! enter_finalizer_dispatch {
            ($self:ident, $ip:ident, $floor:ident) => {
                if !FINALIZERS && $self.heap.has_finalizable_objects() {
                    $self.sync_ip($ip);
                    return $self.run_with_finalizers($floor);
                }
            };
        }

        macro_rules! string_byte_compare {
            (
                $self:ident,
                $registers:ident,
                $ip:ident,
                $floor:ident,
                $label:lifetime,
                $destination:expr,
                $container:expr,
                $index:expr,
                $byte:expr,
                $operator:tt
            ) => {{
                // SAFETY: verified bytecode keeps the container in the active frame.
                let container = unsafe {
                    &*$registers.add($container.index() as usize)
                };
                // SAFETY: this opcode is emitted only for a proven string container.
                let bytes = unsafe { container.as_string_bytes().unwrap_unchecked() };
                // SAFETY: this opcode is emitted only for a proven int index.
                let index = unsafe { int_register($registers, $index) };
                match int_position(index, bytes.len()) {
                    Ok(position) => {
                        // SAFETY: `int_position` checked the byte index.
                        let actual = unsafe { *bytes.get_unchecked(position) };
                        write_register!(
                            $registers,
                            $destination,
                            Value::bool(actual $operator $byte)
                        );
                    }
                    Err(fault) => {
                        fail!(
                            $self,
                            $ip,
                            $floor,
                            $label,
                            $self.array_fault(fault)
                        );
                    }
                }
            }};
        }

        macro_rules! predicate {
            (identity, $value:expr) => {
                $value
            };
            (negate, $value:expr) => {
                !$value
            };
        }

        macro_rules! signed_immediate {
            (positive, $immediate:expr) => {
                i64::from($immediate.value())
            };
            (negative, $immediate:expr) => {
                -i64::from($immediate.value())
            };
        }

        macro_rules! unary_outcome {
            (negate, $value:expr) => {
                negate($value)
            };
            (plus, $value:expr) => {
                match $value.transparent() {
                    ValueView::Int(_) | ValueView::Float(_) => Ok($value.clone_with_newtype(None)),
                    _ => Err(Fault::Incompatible),
                }
            };
            (bitwise_not, $value:expr) => {
                match $value.transparent() {
                    ValueView::Int(operand) => Ok(Value::int(!operand)),
                    _ => Err(Fault::Incompatible),
                }
            };
        }

        macro_rules! execute_dispatch {
            (
                $word:ident, $vm:ident, $registers:ident, $ip:ident, $floor:ident,
                $dispatch:lifetime {
                    binary {
                        $($binary_variant:ident => $binary_operation:path, $binary_operator:literal;)*
                    }
                    integer {
                        $($integer_variant:ident => $integer_operation:path, $integer_operator:literal;)*
                    }
                    integer_immediate {
                        $($immediate_variant:ident => $immediate_operation:path, $immediate_operator:literal;)*
                    }
                    integer_bitwise {
                        $($bitwise_variant:ident => $bitwise_operator:tt;)*
                    }
                    float {
                        $($float_variant:ident => $float_operator:tt;)*
                    }
                    immediate_step {
                        $(
                            $immediate_step_variant:ident =>
                                $immediate_step_direction:ident,
                                $immediate_step_operator:literal;
                        )*
                    }
                    unary {
                        $(
                            $unary_variant:ident =>
                                $unary_operation:ident,
                                $unary_operator:literal;
                        )*
                    }
                    equality {
                        $($equality_variant:ident => $equality_transform:ident;)*
                    }
                    boolean_jump {
                        $($boolean_jump_variant:ident => $boolean_jump_transform:ident;)*
                    }
                    null_jump {
                        $($null_jump_variant:ident => $null_jump_transform:ident;)*
                    }
                    string_byte {
                        $($string_byte_variant:ident => $string_byte_operator:tt;)*
                    }
                    string_byte_jump {
                        $($string_byte_jump_variant:ident => $string_byte_jump_operator:tt;)*
                    }
                    $($rest:tt)*
                }
            ) => {
                dispatch_instruction!($word {
                    $(
                        Instruction::$binary_variant { destination, left, right } => {
                            binary_arithmetic!(
                                $vm,
                                $registers,
                                $ip,
                                $floor,
                                $dispatch,
                                destination,
                                left,
                                right,
                                $binary_operation,
                                $binary_operator
                            );
                        }
                    )*
                    $(
                        Instruction::$integer_variant { destination, left, right } => {
                            integer_arithmetic!(
                                $vm,
                                $registers,
                                $ip,
                                $floor,
                                $dispatch,
                                destination,
                                left,
                                right,
                                $integer_operation,
                                $integer_operator
                            );
                        }
                    )*
                    $(
                        Instruction::$immediate_variant { destination, source, immediate } => {
                            // SAFETY: type flow proves the source register contains an int.
                            let source_value = unsafe { int_register($registers, source) };
                            match $immediate_operation(
                                source_value,
                                i64::from(immediate.value()),
                            ) {
                                Ok(value) => {
                                    write_register!($registers, destination, Value::int(value));
                                }
                                Err(fault) => {
                                    fail!(
                                        $vm,
                                        $ip,
                                        $floor,
                                        $dispatch,
                                        $vm.binary_fault(
                                            fault,
                                            $immediate_operator,
                                            "int",
                                            "int",
                                        )
                                    );
                                }
                            }
                        }
                    )*
                    $(
                        Instruction::$bitwise_variant { destination, left, right } => {
                            // SAFETY: type flow proves both source registers contain ints.
                            let value = unsafe {
                                int_register($registers, left)
                                    $bitwise_operator int_register($registers, right)
                            };
                            write_register!($registers, destination, Value::int(value));
                        }
                    )*
                    $(
                        Instruction::$float_variant { destination, left, right } => {
                            // SAFETY: type flow proves both source registers contain floats.
                            let value = unsafe {
                                float_register($registers, left)
                                    $float_operator float_register($registers, right)
                            };
                            write_register!($registers, destination, Value::float(value));
                        }
                    )*
                    $(
                        Instruction::$immediate_step_variant {
                            destination,
                            source,
                            immediate,
                        } => {
                            let outcome = {
                                // SAFETY: verified bytecode keeps the source in the active frame.
                                let value = unsafe {
                                    &*$registers.add(source.index() as usize)
                                };
                                step_by(
                                    value,
                                    signed_immediate!(
                                        $immediate_step_direction,
                                        immediate
                                    ),
                                )
                            };
                            match outcome {
                                Ok(value) => {
                                    write_register!($registers, destination, value);
                                }
                                Err(fault) => {
                                    // SAFETY: verified bytecode keeps the source in the active frame.
                                    let kind = unsafe {
                                        (*$registers.add(source.index() as usize)).kind_name()
                                    };
                                    fail!(
                                        $vm,
                                        $ip,
                                        $floor,
                                        $dispatch,
                                        $vm.binary_fault(
                                            fault,
                                            $immediate_step_operator,
                                            kind,
                                            "int",
                                        )
                                    );
                                }
                            }
                        }
                    )*
                    $(
                        Instruction::$unary_variant { destination, source } => {
                            let outcome = {
                                // SAFETY: verified bytecode keeps the source in the active frame.
                                let value = unsafe {
                                    &*$registers.add(source.index() as usize)
                                };
                                unary_outcome!($unary_operation, value)
                            };
                            match outcome {
                                Ok(value) => {
                                    write_register!($registers, destination, value);
                                }
                                Err(fault) => {
                                    // SAFETY: verified bytecode keeps the source in the active frame.
                                    let kind = unsafe {
                                        (*$registers.add(source.index() as usize)).kind_name()
                                    };
                                    fail!(
                                        $vm,
                                        $ip,
                                        $floor,
                                        $dispatch,
                                        $vm.unary_fault(fault, $unary_operator, kind)
                                    );
                                }
                            }
                        }
                    )*
                    $(
                        Instruction::$equality_variant { destination, left, right } => {
                            let equal = {
                                // SAFETY: verified bytecode keeps both sources in the active frame.
                                let left_value = unsafe {
                                    &*$registers.add(left.index() as usize)
                                };
                                // SAFETY: verified bytecode keeps both sources in the active frame.
                                let right_value = unsafe {
                                    &*$registers.add(right.index() as usize)
                                };
                                ops::equals(left_value, right_value)
                            };
                            write_register!(
                                $registers,
                                destination,
                                Value::bool(predicate!($equality_transform, equal))
                            );
                        }
                    )*
                    $(
                        Instruction::$boolean_jump_variant { condition, offset } => {
                            let state = {
                                // SAFETY: verified bytecode keeps the condition in the active frame.
                                let value = unsafe {
                                    &*$registers.add(condition.index() as usize)
                                };
                                (value.as_bool(), value.kind_name())
                            };
                            match state.0 {
                                Some(value) => {
                                    if predicate!($boolean_jump_transform, value) {
                                        let relative = offset.offset();
                                        $ip = jump_target($ip, relative);
                                    }
                                }
                                None => {
                                    fail!(
                                        $vm,
                                        $ip,
                                        $floor,
                                        $dispatch,
                                        $vm.throw_well_known(
                                            $vm.engine.tables.well_known.type_error,
                                            format!(
                                                "a condition must be bool, {} given",
                                                state.1,
                                            ),
                                        )
                                    );
                                }
                            }
                        }
                    )*
                    $(
                        Instruction::$null_jump_variant { subject, offset } => {
                            // SAFETY: verified bytecode keeps the subject in the active frame.
                            let is_null = unsafe {
                                (*$registers.add(subject.index() as usize)).is_null()
                            };
                            if predicate!($null_jump_transform, is_null) {
                                $ip = jump_target($ip, offset.offset());
                            }
                        }
                    )*
                    $(
                        Instruction::$string_byte_variant {
                            destination,
                            container,
                            index,
                            byte,
                        } => {
                            string_byte_compare!(
                                $vm,
                                $registers,
                                $ip,
                                $floor,
                                $dispatch,
                                destination,
                                container,
                                index,
                                byte,
                                $string_byte_operator
                            );
                        }
                    )*
                    $(
                        Instruction::$string_byte_jump_variant {
                            container,
                            index,
                            byte,
                            offset,
                        } => {
                            // SAFETY: verified bytecode keeps the container in the active frame.
                            let container = unsafe {
                                &*$registers.add(container.index() as usize)
                            };
                            // SAFETY: this opcode is emitted only for a proven string container.
                            let bytes = unsafe {
                                container.as_string_bytes().unwrap_unchecked()
                            };
                            // SAFETY: this opcode is emitted only for a proven int index.
                            let index = unsafe { int_register($registers, index) };
                            match int_position(index, bytes.len()) {
                                Ok(position) => {
                                    // SAFETY: `int_position` checked the byte index.
                                    let actual = unsafe { *bytes.get_unchecked(position) };
                                    if actual $string_byte_jump_operator byte {
                                        let relative = i32::from(offset.offset());
                                        $ip = jump_target($ip, relative);
                                    }
                                }
                                Err(fault) => {
                                    fail!(
                                        $vm,
                                        $ip,
                                        $floor,
                                        $dispatch,
                                        $vm.array_fault(fault)
                                    );
                                }
                            }
                        }
                    )*
                    $($rest)*
                })
            };
        }

        'dispatch: loop {
            // SAFETY: the current frame owns its chunk for the whole dispatch turn.
            let mut chunk = unsafe { self.current_frame().chunk.as_ref() };
            let mut code = chunk.code.as_ptr();
            let mut ip = self.current_frame().ip as usize;
            // SAFETY: the current frame base lies within the live VM stack.
            let mut registers = unsafe { self.stack.as_mut_ptr().add(self.current_base()) };
            'instructions: loop {
                if FINALIZERS && !self.draining_finalizers && self.heap.has_pending_finalizers() {
                    self.sync_ip(ip);
                    if let Err(control) = self.drain_finalizers(false) {
                        self.handle_control(control, floor)?;
                    }

                    continue 'dispatch;
                }

                // SAFETY: verified bytecode and the frame IP keep this read inside the chunk.
                let instruction = unsafe { InstructionWord::read(code.add(ip)) };
                ip += 1;

                execute_dispatch!(instruction, self, registers, ip, floor, 'dispatch {
                    binary {
                        Add => arithmetic_add, "+";
                        Subtract => arithmetic_subtract, "-";
                        Multiply => arithmetic_multiply, "*";
                        Divide => arithmetic_divide, "/";
                        Modulo => arithmetic_modulo, "%";
                        Power => arithmetic_power, "**";
                        BitwiseAnd => bitwise_and, "&";
                        BitwiseOr => bitwise_or, "|";
                        BitwiseXor => bitwise_xor, "^";
                        ShiftLeft => shift_left, "<<";
                        ShiftRight => shift_right, ">>";
                        LessThan => compare_less, "<";
                        LessThanOrEqual => compare_less_or_equal, "<=";
                        GreaterThan => compare_greater, ">";
                        GreaterThanOrEqual => compare_greater_or_equal, ">=";
                    }
                    integer {
                        IntAdd => integer_add, "+";
                        IntSubtract => integer_subtract, "-";
                        IntMultiply => integer_multiply, "*";
                        IntModulo => integer_modulo, "%";
                        IntShiftLeft => integer_shift_left, "<<";
                        IntShiftRight => integer_shift_right, ">>";
                    }
                    integer_immediate {
                        IntMultiplyImmediate => integer_multiply, "*";
                        IntModuloImmediate => integer_modulo, "%";
                    }
                    integer_bitwise {
                        IntBitwiseAnd => &;
                        IntBitwiseOr => |;
                        IntBitwiseXor => ^;
                    }
                    float {
                        FloatAdd => +;
                        FloatSubtract => -;
                        FloatMultiply => *;
                    }
                    immediate_step {
                        AddImmediate => positive, "+";
                        SubtractImmediate => negative, "-";
                    }
                    unary {
                        Negate => negate, "-";
                        UnaryPlus => plus, "+";
                        BitwiseNot => bitwise_not, "~";
                    }
                    equality {
                        Equal => identity;
                        NotEqual => negate;
                    }
                    boolean_jump {
                        JumpIfFalse => negate;
                        JumpIfTrue => identity;
                    }
                    null_jump {
                        JumpIfNull => identity;
                        JumpIfNotNull => negate;
                    }
                    string_byte {
                        StringByteEqual => ==;
                        StringByteNotEqual => !=;
                        StringByteLessThan => <;
                        StringByteLessThanOrEqual => <=;
                        StringByteGreaterThan => >;
                        StringByteGreaterThanOrEqual => >=;
                    }
                    string_byte_jump {
                        StringByteJumpUnlessEqual => !=;
                        StringByteJumpUnlessNotEqual => ==;
                    }
                    Instruction::Move {
                        destination,
                        source,
                    } => {
                        write_register!(registers, destination, read_register!(registers, source));
                    }
                    Instruction::MoveOwned {
                        destination,
                        source,
                    } => {
                        // SAFETY: verified bytecode keeps operands in the live frame and proves their types.
                        let value = unsafe {
                            mem::replace(
                                &mut *registers.add(source.index() as usize),
                                Value::uninitialized(),
                            )
                        };

                        write_register!(registers, destination, value);
                    }
                    Instruction::Clear { target } => {
                        // SAFETY: verified bytecode keeps operands in the live frame and proves their types.
                        let value = unsafe {
                            mem::replace(
                                &mut *registers.add(target.index() as usize),
                                Value::uninitialized(),
                            )
                        };
                        drop(value);
                    }
                    Instruction::CheckSoleReference {
                        source,
                        message,
                        chain_previous,
                    } => {
                        // SAFETY: verified bytecode keeps operands in the live frame and proves their types.
                        let leaked = unsafe {
                            (&*registers.add(source.index() as usize))
                                .has_other_strong_references()
                        };

                        if leaked {
                            let message = match &chunk.constants[message.index() as usize] {
                                Literal::String(message) => {
                                    message.to_string_lossy().into_owned()
                                }
                                // SAFETY: the surrounding invariant makes this path unreachable.
                                _ => unsafe {
                                    unreachable_invariant(
                                        "a CheckSoleReference message is a pooled string",
                                    )
                                },
                            };
                            let previous = if chain_previous {
                                let frame = self.frames.len() - 1;
                                self.pending_unwinds
                                    .last()
                                    .filter(|pending| pending.frame == frame)
                                    .map_or_else(Value::null, |pending| pending.value.clone())
                            } else {
                                Value::null()
                            };

                            fail!(
                                self,
                                ip,
                                floor,
                                'dispatch,
                                self.throw_well_known_with_previous(
                                    self.engine.tables.well_known.leaked_resource_error,
                                    message,
                                    previous,
                                )
                            );
                        }
                    }
                    Instruction::DrainFinalizers => {
                        if self.draining_finalizers || !self.heap.has_pending_finalizers() {
                            continue 'instructions;
                        }

                        self.sync_ip(ip);
                        if let Err(control) = self.drain_finalizers(false) {
                            self.handle_control(control, floor)?;
                        }

                        continue 'dispatch;
                    }
                    Instruction::LoadConstant {
                        destination,
                        constant,
                    } => {
                        let literal = &chunk.constants[constant.index() as usize];
                        write_register!(registers, destination, literal_value(literal));
                    }
                    Instruction::LoadNull { destination } => {
                        write_register!(registers, destination, Value::null());
                    }
                    Instruction::LoadTrue { destination } => {
                        write_register!(registers, destination, Value::bool(true));
                    }
                    Instruction::LoadFalse { destination } => {
                        write_register!(registers, destination, Value::bool(false));
                    }
                    Instruction::LoadInt {
                        destination,
                        immediate,
                    } => {
                        write_register!(
                            registers,
                            destination,
                            Value::int(immediate.value().into())
                        );
                    }
                    Instruction::IntAddAssign { target, source } => {
                        // SAFETY: verified bytecode keeps operands in the live frame and proves their types.
                        let target_value = unsafe { int_register(registers, target) };
                        // SAFETY: verified bytecode keeps operands in the live frame and proves their types.
                        let source_value = unsafe { int_register(registers, source) };
                        match integer_add(target_value, source_value) {
                            Ok(value) => {
                                write_proven_int_register!(registers, target, value);
                            }
                            Err(fault) => {
                                fail!(
                                    self,
                                    ip,
                                    floor,
                                    'dispatch,
                                    self.binary_fault(fault, "+", "int", "int")
                                );
                            }
                        }
                    }
                    Instruction::IntBitwiseNot {
                        destination,
                        source,
                    } => {
                        // SAFETY: verified bytecode keeps operands in the live frame and proves their types.
                        let value = !unsafe { int_register(registers, source) };
                        write_register!(registers, destination, Value::int(value));
                    }
                    Instruction::Squares {
                        first_destination,
                        first_source,
                        second_source,
                    } => {
                        let first = {
                            // SAFETY: verified bytecode keeps operands in the live frame and proves their types.
                            let source = unsafe { &*registers.add(first_source.index() as usize) };
                            arithmetic_multiply(&self.heap, source, source)
                        };

                        match first {
                            Ok(value) => {
                                write_register!(registers, first_destination, value)
                            }
                            Err(fault) => {
                                // SAFETY: verified bytecode keeps operands in the live frame and proves their types.
                                let kind = unsafe {
                                    (*registers.add(first_source.index() as usize)).kind_name()
                                };

                                fail!(
                                    self,
                                    ip,
                                    floor,
                                    'dispatch,
                                    self.binary_fault(fault, "*", kind, kind)
                                );
                            }
                        }

                        let second_destination = Register::new(first_destination.index() + 1);
                        let second = {
                            // SAFETY: verified bytecode keeps operands in the live frame and proves their types.
                            let source = unsafe { &*registers.add(second_source.index() as usize) };
                            arithmetic_multiply(&self.heap, source, source)
                        };

                        match second {
                            Ok(value) => {
                                write_register!(registers, second_destination, value)
                            }
                            Err(fault) => {
                                // SAFETY: verified bytecode keeps operands in the live frame and proves their types.
                                let kind = unsafe {
                                    (*registers.add(second_source.index() as usize)).kind_name()
                                };

                                fail!(
                                    self,
                                    ip,
                                    floor,
                                    'dispatch,
                                    self.binary_fault(fault, "*", kind, kind)
                                );
                            }
                        }
                    }
                    Instruction::FloatSquares {
                        first_destination,
                        first_source,
                        second_source,
                    } => {
                        // SAFETY: verified bytecode keeps operands in the live frame and proves their types.
                        let first = unsafe { float_register(registers, first_source) };
                        // SAFETY: verified bytecode keeps operands in the live frame and proves their types.
                        let second = unsafe { float_register(registers, second_source) };
                        write_register!(
                            registers,
                            first_destination,
                            Value::float(first * first)
                        );
                        write_register!(
                            registers,
                            Register::new(first_destination.index() + 1),
                            Value::float(second * second)
                        );
                    }
                    Instruction::FloatSquaresSum {
                        first_destination,
                        first_source,
                        second_source,
                    } => {
                        // SAFETY: verified bytecode keeps operands in the live frame and proves their types.
                        let first = unsafe { float_register(registers, first_source) };
                        // SAFETY: verified bytecode keeps operands in the live frame and proves their types.
                        let second = unsafe { float_register(registers, second_source) };
                        let first_square = first * first;
                        let second_square = second * second;
                        write_register!(
                            registers,
                            Register::new(first_destination.index() + 1),
                            Value::float(first_square)
                        );
                        write_register!(
                            registers,
                            Register::new(first_destination.index() + 2),
                            Value::float(second_square)
                        );
                        write_register!(
                            registers,
                            first_destination,
                            Value::float(first_square + second_square)
                        );
                    }
                    Instruction::FloatSquaresSumBranch { descriptor, offset } => {
                        let FloatSquaresSumBranchDescriptor {
                            sum_destination,
                            first_square_destination,
                            second_square_destination,
                            first_source,
                            second_source,
                            comparison,
                            constant,
                        } = *chunk.float_squares_sum_branch_descriptor(descriptor);

                        // SAFETY: verified bytecode keeps operands in the live frame and proves their types.
                        let first = unsafe { float_register(registers, first_source) };
                        // SAFETY: verified bytecode keeps operands in the live frame and proves their types.
                        let second = unsafe { float_register(registers, second_source) };
                        let first_square = first * first;
                        let second_square = second * second;
                        let Literal::Float(constant) =
                            chunk.constants[usize::from(constant.index())]
                        else {
                            // SAFETY: the surrounding invariant makes this path unreachable.
                            unsafe {
                                unreachable_invariant(
                                    "a verified square-sum branch references a float constant",
                                )
                            }
                        };

                        write_register!(
                            registers,
                            first_square_destination,
                            Value::float(first_square)
                        );
                        write_register!(
                            registers,
                            second_square_destination,
                            Value::float(second_square)
                        );
                        write_register!(
                            registers,
                            sum_destination,
                            Value::float(first_square + second_square)
                        );

                        if !float_comparison_matches(
                            comparison,
                            first_square + second_square,
                            constant,
                        ) {
                            let relative = offset.offset();
                            ip = jump_target(ip, relative);
                        }
                    }
                    Instruction::FloatMultiplyConstant {
                        destination,
                        source,
                        constant,
                    } => {
                        // SAFETY: verified bytecode keeps operands in the live frame and proves their types.
                        let value = unsafe { float_register(registers, source) };
                        let Literal::Float(constant) =
                            chunk.constants[constant.index() as usize]
                        else {
                            // SAFETY: the surrounding invariant makes this path unreachable.
                            unsafe {
                                unreachable_invariant(
                                    "a verified float instruction references a float constant",
                                )
                            }
                        };

                        write_register!(
                            registers,
                            destination,
                            Value::float(value * constant)
                        );
                    }
                    Instruction::FloatDifferenceAdd {
                        destination,
                        first_operand,
                        addend,
                    } => {
                        // SAFETY: verified bytecode keeps operands in the live frame and proves their types.
                        let left = unsafe { float_register(registers, first_operand) };
                        // SAFETY: verified bytecode keeps operands in the live frame and proves their types.
                        let right = unsafe {
                            float_register(registers, Register::new(first_operand.index() + 1))
                        };

                        // SAFETY: verified bytecode keeps operands in the live frame and proves their types.
                        let addend = unsafe { float_register(registers, addend) };
                        let difference = left - right;
                        write_register!(
                            registers,
                            destination,
                            Value::float(difference + addend)
                        );
                    }
                    Instruction::FloatScaleProductAdd {
                        destination,
                        first_operand,
                        constant,
                    } => {
                        // SAFETY: verified bytecode keeps operands in the live frame and proves their types.
                        let addend = unsafe { float_register(registers, first_operand) };
                        // SAFETY: verified bytecode keeps operands in the live frame and proves their types.
                        let left = unsafe {
                            float_register(registers, Register::new(first_operand.index() + 1))
                        };

                        // SAFETY: verified bytecode keeps operands in the live frame and proves their types.
                        let right = unsafe {
                            float_register(registers, Register::new(first_operand.index() + 2))
                        };

                        let Literal::Float(constant) =
                            chunk.constants[constant.index() as usize]
                        else {
                            // SAFETY: the surrounding invariant makes this path unreachable.
                            unsafe {
                                unreachable_invariant(
                                    "a verified float instruction references a float constant",
                                )
                            }
                        };

                        let scaled = left * constant;
                        let product = scaled * right;
                        write_register!(
                            registers,
                            destination,
                            Value::float(product + addend)
                        );
                    }
                    Instruction::FloatPairUpdate { descriptor } => {
                        let FloatPairUpdateDescriptor {
                            first_destination,
                            first_operand,
                            constant,
                            second_destination,
                            second_operand,
                            second_addend,
                        } = *chunk.float_pair_update_descriptor(descriptor);

                        // SAFETY: verified bytecode keeps operands in the live frame and proves their types.
                        let addend = unsafe { float_register(registers, first_operand) };
                        // SAFETY: verified bytecode keeps operands in the live frame and proves their types.
                        let left = unsafe {
                            float_register(
                                registers,
                                Register::new(first_operand.index() + 1),
                            )
                        };

                        // SAFETY: verified bytecode keeps operands in the live frame and proves their types.
                        let right = unsafe {
                            float_register(
                                registers,
                                Register::new(first_operand.index() + 2),
                            )
                        };

                        let Literal::Float(constant) =
                            chunk.constants[usize::from(constant.index())]
                        else {
                            // SAFETY: the surrounding invariant makes this path unreachable.
                            unsafe {
                                unreachable_invariant(
                                    "a verified pair update references a float constant",
                                )
                            }
                        };

                        let scaled = left * constant;
                        let product = scaled * right;
                        write_register!(
                            registers,
                            first_destination,
                            Value::float(product + addend)
                        );

                        // SAFETY: verified bytecode keeps operands in the live frame and proves their types.
                        let left = unsafe { float_register(registers, second_operand) };
                        // SAFETY: verified bytecode keeps operands in the live frame and proves their types.
                        let right = unsafe {
                            float_register(
                                registers,
                                Register::new(second_operand.index() + 1),
                            )
                        };

                        // SAFETY: verified bytecode keeps operands in the live frame and proves their types.
                        let addend = unsafe { float_register(registers, second_addend) };
                        let difference = left - right;
                        write_register!(
                            registers,
                            second_destination,
                            Value::float(difference + addend)
                        );
                    }
                    Instruction::Concatenate {
                        destination,
                        left,
                        right,
                    } => {
                        let appended = destination.index() == left.index()
                            && left.index() != right.index()
                            && {
                                // SAFETY: verified bytecode keeps operands in the live frame and proves their types.
                                let left_value = unsafe { &*registers.add(left.index() as usize) };
                                let right_value =
                                    // SAFETY: verified bytecode keeps operands in the live frame and proves their types.
                                    unsafe { &*registers.add(right.index() as usize) };
                                match (left_value.transparent(), right_value.transparent()) {
                                    (ValueView::String(target), ValueView::String(extra)) => {
                                        // SAFETY: verified bytecode keeps operands in the live frame and proves their types.
                                        unsafe {
                                            ByteStringObject::append_unique_string(target, extra)
                                        }
                                    }
                                    _ => false,
                                }
                            };

                        if !appended {
                            binary_arithmetic!(
                                self,
                                registers,
                                ip,
                                floor,
                                'dispatch,
                                destination,
                                left,
                                right,
                                concatenate,
                                "."
                            );
                        }
                    }
                    Instruction::ConcatenateRightConstant {
                        destination,
                        source,
                        constant,
                    } => {
                        // SAFETY: verification proves this constant is a string.
                        let Literal::String(extra) = (unsafe {
                            chunk.constants.get_unchecked(usize::from(constant.index()))
                        }) else {
                            // SAFETY: verification rejects every other literal kind.
                            unsafe {
                                unreachable_invariant(
                                    "a concatenation constant is always a string",
                                )
                            }
                        };
                        // SAFETY: verified bytecode keeps the source in the live frame.
                        let source_value = unsafe { &*registers.add(source.index() as usize) };
                        let outcome = concatenate_right_constant(
                            &self.heap,
                            source_value,
                            extra,
                            destination == source,
                        );
                        match outcome {
                            Ok(None) => continue 'instructions,
                            Ok(Some(value)) => write_register!(registers, destination, value),
                            Err(fault) => {
                                let source_kind = source_value.kind_name();
                                fail!(
                                    self,
                                    ip,
                                    floor,
                                    'dispatch,
                                    self.binary_fault(fault, ".", source_kind, "string")
                                );
                            }
                        }
                    }
                    Instruction::ConcatenateLeftConstant {
                        destination,
                        source,
                        constant,
                    } => {
                        // SAFETY: verification proves this constant is a string.
                        let Literal::String(extra) = (unsafe {
                            chunk.constants.get_unchecked(usize::from(constant.index()))
                        }) else {
                            // SAFETY: verification rejects every other literal kind.
                            unsafe {
                                unreachable_invariant(
                                    "a concatenation constant is always a string",
                                )
                            }
                        };
                        // SAFETY: verified bytecode keeps the source in the live frame.
                        let source_value = unsafe { &*registers.add(source.index() as usize) };
                        match concatenate_left_constant(&self.heap, extra, source_value) {
                            Ok(value) => write_register!(registers, destination, value),
                            Err(fault) => {
                                let source_kind = source_value.kind_name();
                                fail!(
                                    self,
                                    ip,
                                    floor,
                                    'dispatch,
                                    self.binary_fault(fault, ".", "string", source_kind)
                                );
                            }
                        }
                    }
                    Instruction::IncrementJump {
                        target,
                        immediate,
                        offset,
                    } => {
                        let outcome = {
                            // SAFETY: verified bytecode keeps operands in the live frame and proves their types.
                            let value = unsafe { &*registers.add(target.index() as usize) };
                            step_by(value, i64::from(immediate.value()))
                        };
                        match outcome {
                            Ok(value) => write_register!(registers, target, value),
                            Err(fault) => {
                                // SAFETY: verified bytecode keeps operands in the live frame and proves their types.
                                let kind = unsafe {
                                    (*registers.add(target.index() as usize)).kind_name()
                                };

                                fail!(
                                    self,
                                    ip,
                                    floor,
                                    'dispatch,
                                    self.binary_fault(fault, "+", kind, "int")
                                );
                            }
                        }

                        let relative = i32::from(offset.offset());
                        ip = jump_target(ip, relative);
                    }
                    Instruction::CounterLoop {
                        comparison,
                        counter,
                        limit,
                        offset,
                    } => {
                        let outcome = {
                            // SAFETY: verified bytecode keeps operands in the live frame and proves their types.
                            let value = unsafe { &*registers.add(counter.index() as usize) };
                            step_by(value, 1)
                        };

                        match outcome {
                            Ok(value) => write_register!(registers, counter, value),
                            Err(fault) => {
                                // SAFETY: verified bytecode keeps operands in the live frame and proves their types.
                                let kind = unsafe {
                                    (*registers.add(counter.index() as usize)).kind_name()
                                };

                                fail!(
                                    self,
                                    ip,
                                    floor,
                                    'dispatch,
                                    self.binary_fault(fault, "+", kind, "int")
                                );
                            }
                        }
                        let outcome = {
                            let counter_value =
                                // SAFETY: verified bytecode keeps operands in the live frame and proves their types.
                                unsafe { &*registers.add(counter.index() as usize) };
                            // SAFETY: verified bytecode keeps operands in the live frame and proves their types.
                            let limit_value = unsafe { &*registers.add(limit.index() as usize) };
                            comparison_matches(comparison, counter_value, limit_value)
                        };

                        match outcome {
                            Ok(true) => {
                                ip = jump_target(ip, i32::from(offset.offset()));
                            }
                            Ok(false) => {}
                            Err(fault) => {
                                // SAFETY: verified bytecode keeps operands in the live frame and proves their types.
                                let (counter_kind, limit_kind) = unsafe {
                                    (
                                        (*registers.add(counter.index() as usize)).kind_name(),
                                        (*registers.add(limit.index() as usize)).kind_name(),
                                    )
                                };

                                fail!(
                                    self,
                                    ip,
                                    floor,
                                    'dispatch,
                                    self.binary_fault(
                                        fault,
                                        comparison.operator(),
                                        counter_kind,
                                        limit_kind,
                                    )
                                );
                            }
                        }
                    }
                    Instruction::IntCounterLoop {
                        comparison,
                        counter,
                        limit,
                        offset,
                    } => {
                        // SAFETY: verified bytecode keeps operands in the live frame and proves their types.
                        let counter_value = unsafe { int_register(registers, counter) };
                        // SAFETY: verified bytecode keeps operands in the live frame and proves their types.
                        let limit_value = unsafe { int_register(registers, limit) };
                        let Some(next) = counter_value.checked_add(1) else {
                            let fault = if counter_value >= 0 {
                                Fault::Overflow
                            } else {
                                Fault::Underflow
                            };

                            fail!(
                                self,
                                ip,
                                floor,
                                'dispatch,
                                self.binary_fault(fault, "+", "int", "int")
                            );
                        };

                        write_proven_int_register!(registers, counter, next);
                        if int_comparison_matches(comparison, next, limit_value) {
                            ip = jump_target(ip, i32::from(offset.offset()));
                        }
                    }
                    Instruction::IntStepLoop { descriptor, offset } => {
                        let IntStepLoopDescriptor {
                            comparison,
                            counter,
                            limit,
                            step,
                        } = *chunk.int_step_loop_descriptor(descriptor);
                        // SAFETY: verified bytecode keeps operands in the live frame and proves their types.
                        let counter_value = unsafe { int_register(registers, counter) };
                        // SAFETY: verified bytecode keeps operands in the live frame and proves their types.
                        let limit_value = unsafe { int_register(registers, limit) };
                        // SAFETY: verified bytecode keeps operands in the live frame and proves their types.
                        let step_value = unsafe { int_register(registers, step) };
                        let Some(next) = counter_value.checked_add(step_value) else {
                            let fault = if step_value >= 0 {
                                Fault::Overflow
                            } else {
                                Fault::Underflow
                            };

                            fail!(
                                self,
                                ip,
                                floor,
                                'dispatch,
                                self.binary_fault(fault, "+", "int", "int")
                            );
                        };

                        write_proven_int_register!(registers, counter, next);
                        if int_comparison_matches(comparison, next, limit_value) {
                            ip = jump_target(ip, i32::from(offset.offset()));
                        }
                    }
                    Instruction::NumericLoop {
                        comparison,
                        left,
                        right,
                        offset,
                    } => {
                        refine_live_tail!(
                            self,
                            chunk,
                            code,
                            ip,
                            registers,
                            floor,
                            'dispatch,
                            'instructions
                        );
                        let outcome = {
                            // SAFETY: verified bytecode keeps operands in the live frame and proves their types.
                            let left_value = unsafe { &*registers.add(left.index() as usize) };
                            // SAFETY: verified bytecode keeps operands in the live frame and proves their types.
                            let right_value = unsafe { &*registers.add(right.index() as usize) };
                            comparison_matches(comparison, left_value, right_value)
                        };

                        match outcome {
                            Ok(false) => {
                                ip = jump_target(ip, i32::from(offset.offset()));
                            }
                            Ok(true) => {
                                let exit = jump_target(ip, i32::from(offset.offset()));
                                resume_numeric_loop!(
                                    self,
                                    chunk,
                                    registers,
                                    ip,
                                    exit,
                                    floor,
                                    'dispatch,
                                    false,
                                    0,
                                    0,
                                    RegionSite::new(chunk, ip - 1)
                                );
                            }
                            Err(fault) => {
                                // SAFETY: verified bytecode keeps operands in the live frame and proves their types.
                                let (left_kind, right_kind) = unsafe {
                                    (
                                        (*registers.add(left.index() as usize)).kind_name(),
                                        (*registers.add(right.index() as usize)).kind_name(),
                                    )
                                };

                                fail!(
                                    self,
                                    ip,
                                    floor,
                                    'dispatch,
                                    self.binary_fault(
                                        fault,
                                        comparison.operator(),
                                        left_kind,
                                        right_kind,
                                    )
                                );
                            }
                        }
                    }
                    Instruction::IntNumericLoop {
                        comparison,
                        left,
                        right,
                        offset,
                    } => {
                        refine_live_tail!(
                            self,
                            chunk,
                            code,
                            ip,
                            registers,
                            floor,
                            'dispatch,
                            'instructions
                        );
                        // SAFETY: verified bytecode keeps operands in the live frame and proves their types.
                        let left_value = unsafe { int_register(registers, left) };
                        // SAFETY: verified bytecode keeps operands in the live frame and proves their types.
                        let right_value = unsafe { int_register(registers, right) };
                        if !int_comparison_matches(comparison, left_value, right_value) {
                            ip = jump_target(ip, i32::from(offset.offset()));
                        } else {
                            let exit = jump_target(ip, i32::from(offset.offset()));
                            resume_numeric_loop!(
                                self,
                                chunk,
                                registers,
                                ip,
                                exit,
                                floor,
                                'dispatch,
                                false,
                                0,
                                0,
                                RegionSite::new(chunk, ip - 1)
                            );
                        }
                    }
                    Instruction::PreparedIntNumericLoop { descriptor, offset } => {
                        refine_live_tail!(
                            self,
                            chunk,
                            code,
                            ip,
                            registers,
                            floor,
                            'dispatch,
                            'instructions
                        );
                        let PreparedIntLoopDescriptor {
                            comparison,
                            counter,
                            limit,
                            float_registers,
                        } = *chunk.prepared_int_loop_descriptor(descriptor);

                        // SAFETY: verified bytecode keeps operands in the live frame and proves their types.
                        let counter_value = unsafe { int_register(registers, counter) };
                        // SAFETY: verified bytecode keeps operands in the live frame and proves their types.
                        let limit_value = unsafe { int_register(registers, limit) };
                        if !int_comparison_matches(comparison, counter_value, limit_value) {
                            ip = jump_target(ip, i32::from(offset.offset()));
                        } else {
                            let exit = jump_target(ip, i32::from(offset.offset()));
                            resume_numeric_loop!(
                                self,
                                chunk,
                                registers,
                                ip,
                                exit,
                                floor,
                                'dispatch,
                                true,
                                float_registers,
                                float_registers | (1u64 << u32::from(counter.index())),
                                RegionSite::new(chunk, ip - 1)
                            );
                        }
                    }
                    Instruction::JumpUnless {
                        comparison,
                        left,
                        right,
                        offset,
                    } => {
                        let outcome = {
                            // SAFETY: verified bytecode keeps operands in the live frame and proves their types.
                            let left_value = unsafe { &*registers.add(left.index() as usize) };
                            // SAFETY: verified bytecode keeps operands in the live frame and proves their types.
                            let right_value = unsafe { &*registers.add(right.index() as usize) };
                            comparison_matches(comparison, left_value, right_value)
                        };

                        match outcome {
                            Ok(value) => {
                                if !value {
                                    let relative = i32::from(offset.offset());
                                    ip = jump_target(ip, relative);
                                }
                            }
                            Err(fault) => {
                                // SAFETY: verified bytecode keeps operands in the live frame and proves their types.
                                let (left_kind, right_kind) = unsafe {
                                    (
                                        (*registers.add(left.index() as usize)).kind_name(),
                                        (*registers.add(right.index() as usize)).kind_name(),
                                    )
                                };

                                fail!(
                                    self,
                                    ip,
                                    floor,
                                    'dispatch,
                                    self.binary_fault(
                                        fault,
                                        comparison.operator(),
                                        left_kind,
                                        right_kind,
                                    )
                                );
                            }
                        }
                    }
                    Instruction::IntJumpUnless {
                        comparison,
                        left,
                        right,
                        offset,
                    } => {
                        refine_live_tail!(
                            self,
                            chunk,
                            code,
                            ip,
                            registers,
                            floor,
                            'dispatch,
                            'instructions
                        );
                        // SAFETY: verified bytecode keeps operands in the live frame and proves their types.
                        let left_value = unsafe { int_register(registers, left) };
                        // SAFETY: verified bytecode keeps operands in the live frame and proves their types.
                        let right_value = unsafe { int_register(registers, right) };

                        if !int_comparison_matches(comparison, left_value, right_value) {
                            let relative = i32::from(offset.offset());
                            ip = jump_target(ip, relative);
                        }
                    }
                    Instruction::StringJumpUnless {
                        comparison,
                        left,
                        right,
                        offset,
                    } => {
                        // SAFETY: verified bytecode keeps operands in the live frame and proves their types.
                        let left = unsafe {
                            (&*registers.add(left.index() as usize))
                                .as_string_bytes()
                                .unwrap_unchecked()
                        };
                        // SAFETY: verified bytecode keeps operands in the live frame and proves their types.
                        let right = unsafe {
                            (&*registers.add(right.index() as usize))
                                .as_string_bytes()
                                .unwrap_unchecked()
                        };
                        if !string_comparison_matches(comparison, left, right) {
                            let relative = i32::from(offset.offset());
                            ip = jump_target(ip, relative);
                        }
                    }
                    Instruction::IntJumpUnlessImmediate {
                        comparison,
                        source,
                        immediate,
                        offset,
                    } => {
                        // SAFETY: verified bytecode keeps operands in the live frame and proves their types.
                        let source_value = unsafe { int_register(registers, source) };
                        let immediate_value = i64::from(immediate.value());
                        if !int_comparison_matches(comparison, source_value, immediate_value) {
                            let relative = i32::from(offset.offset());
                            ip = jump_target(ip, relative);
                        }
                    }
                    Instruction::JumpUnlessConstant {
                        comparison,
                        source,
                        constant,
                        offset,
                    } => {
                        let constant =
                            literal_value(&chunk.constants[constant.index() as usize]);
                        let outcome = {
                            let source_value =
                                // SAFETY: verified bytecode keeps operands in the live frame and proves their types.
                                unsafe { &*registers.add(source.index() as usize) };
                            comparison_matches(comparison, source_value, &constant)
                        };

                        match outcome {
                            Ok(value) => {
                                if !value {
                                    let relative = i32::from(offset.offset());
                                    ip = jump_target(ip, relative);
                                }
                            }
                            Err(fault) => {
                                // SAFETY: verified bytecode keeps operands in the live frame and proves their types.
                                let source_kind = unsafe {
                                    (*registers.add(source.index() as usize)).kind_name()
                                };

                                fail!(
                                    self,
                                    ip,
                                    floor,
                                    'dispatch,
                                    self.binary_fault(
                                        fault,
                                        comparison.operator(),
                                        source_kind,
                                        constant.kind_name(),
                                    )
                                );
                            }
                        }
                    }
                    Instruction::Compare {
                        destination,
                        left,
                        right,
                    } => {
                        binary_arithmetic!(
                            self,
                            registers,
                            ip,
                            floor,
                            'dispatch,
                            destination,
                            left,
                            right,
                            compare_spaceship,
                            "<=>"
                        );
                    }
                    Instruction::Not {
                        destination,
                        source,
                    } => {
                        let state = {
                            // SAFETY: verified bytecode keeps operands in the live frame and proves their types.
                            let value = unsafe { &*registers.add(source.index() as usize) };
                            (value.as_bool(), value.kind_name())
                        };

                        match state.0 {
                            Some(operand) => {
                                write_register!(registers, destination, Value::bool(!operand));
                            }
                            None => {
                                fail!(
                                    self,
                                    ip,
                                    floor,
                                    'dispatch,
                                    self.throw_well_known(
                                        self.engine.tables.well_known.type_error,
                                        format!("the ! operand must be bool, {} given", state.1),
                                    )
                                );
                            }
                        }
                    }
                    Instruction::Jump { offset } => {
                        let relative = offset.offset();
                        ip = jump_target(ip, relative);
                    }
                    Instruction::NumericRegionJump { offset } => {
                        let relative = offset.offset();
                        let exit = ip;
                        ip = jump_target(ip, relative);
                        resume_numeric_loop!(
                            self,
                            chunk,
                            registers,
                            ip,
                            exit,
                            floor,
                            'dispatch,
                            false,
                            0,
                            0,
                            RegionSite::new(chunk, exit - 1)
                        );
                    }
                    Instruction::CheckDefined { subject, name } => {
                        // SAFETY: verified bytecode keeps operands in the live frame and proves their types.
                        let is_uninitialized = unsafe {
                            (*registers.add(subject.index() as usize)).is_uninitialized()
                        };

                        if is_uninitialized {
                            let variable = literal_text(&chunk.constants[name.index() as usize]);
                            fail!(
                                self,
                                ip,
                                floor,
                                'dispatch,
                                self.throw_well_known(
                                    self.engine.tables.well_known.undefined_variable_error,
                                    format!("the variable {variable} has not been assigned"),
                                )
                            );
                        }
                    }
                    Instruction::FillDefault { target, offset } => {
                        let is_uninitialized =
                            // SAFETY: verified bytecode keeps operands in the live frame and proves their types.
                            unsafe { (*registers.add(target.index() as usize)).is_uninitialized() };

                        if !is_uninitialized {
                            ip = jump_target(ip, offset.offset());
                        }
                    }
                    Instruction::CallNamed {
                        argument_count,
                        destination,
                        first_argument,
                        cache,
                    } => {
                        self.sync_ip(ip);
                        let window_start = self.current_base() + first_argument.index() as usize;
                        let count = usize::from(argument_count.value());
                        let stack_pointer = self.stack.as_ptr();
                        if let Some(outcome) = self.call_cached_built_in_function_site(
                            cache.index() as usize,
                            chunk,
                            destination.index(),
                            window_start,
                            count,
                        ) {
                            if let Err(control) = outcome {
                                self.handle_control(control, floor)?;
                                continue 'dispatch;
                            }

                            if stack_pointer == self.stack.as_ptr()
                                && self.current_frame().chunk == NonNull::from(chunk)
                            {
                                continue 'instructions;
                            }

                            continue 'dispatch;
                        }

                        if let Err(control) = self.call_named_site(
                            cache.index() as usize,
                            chunk,
                            destination.index(),
                            window_start,
                            count,
                            false,
                        ) {
                            self.handle_control(control, floor)?;
                        }

                        continue 'dispatch;
                    }
                    Instruction::CallNamedDiscarded {
                        argument_count,
                        destination,
                        first_argument,
                        cache,
                    } => {
                        self.sync_ip(ip);
                        self.begin_discarded_result_check();
                        let window_start = self.current_base() + first_argument.index() as usize;
                        let count = usize::from(argument_count.value());
                        let stack_pointer = self.stack.as_ptr();
                        if let Some(outcome) = self.call_cached_built_in_function_site(
                            cache.index() as usize,
                            chunk,
                            destination.index(),
                            window_start,
                            count,
                        ) {
                            if let Err(control) = outcome {
                                self.handle_control(control, floor)?;
                                continue 'dispatch;
                            }

                            if stack_pointer == self.stack.as_ptr()
                                && self.current_frame().chunk == NonNull::from(chunk)
                            {
                                continue 'instructions;
                            }

                            continue 'dispatch;
                        }

                        let outcome = self.call_named_site(
                            cache.index() as usize,
                            chunk,
                            destination.index(),
                            window_start,
                            count,
                            true,
                        );
                        if let Err(control) = outcome {
                            self.handle_control(control, floor)?;
                        }

                        continue 'dispatch;
                    }
                    Instruction::CallNamedUnchecked {
                        argument_count,
                        destination,
                        first_argument,
                        cache,
                    } => {
                        self.sync_ip(ip);
                        let window_start = self.current_base() + first_argument.index() as usize;
                        let count = usize::from(argument_count.value());
                        if let Err(control) = self.call_exact_function_site(
                            cache.index() as usize,
                            destination.index(),
                            window_start,
                            count,
                        ) {
                            self.handle_control(control, floor)?;
                            continue 'dispatch;
                        }

                        reload_frame!(self, chunk, code, ip, registers);
                        continue 'instructions;
                    }
                    Instruction::CallNamedConstantUnchecked {
                        destination,
                        constant,
                        cache,
                        borrowed,
                    } => {
                        self.sync_ip(ip);
                        // SAFETY: verified bytecode keeps operands in the live frame and proves their types.
                        let literal = unsafe {
                            chunk
                                .constants
                                .get_unchecked(constant.index() as usize)
                        };

                        if let Err(control) = self.call_exact_constant_function_site(
                            cache.index() as usize,
                            destination.index(),
                            literal,
                            borrowed,
                        ) {
                            self.handle_control(control, floor)?;
                            continue 'dispatch;
                        }

                        reload_frame!(self, chunk, code, ip, registers);
                        continue 'instructions;
                    }
                    Instruction::CallSelfUnchecked {
                        argument_count,
                        destination,
                        first_argument,
                    } => {
                        self.sync_ip(ip);
                        let window_start = self.current_base() + first_argument.index() as usize;
                        let count = usize::from(argument_count.value());
                        if let Err(control) =
                            self.call_exact_self(destination.index(), window_start, count)
                        {
                            self.handle_control(control, floor)?;
                            continue 'dispatch;
                        }

                        reload_frame!(self, chunk, code, ip, registers);
                        continue 'instructions;
                    }
                    Instruction::CallValue {
                        argument_count,
                        destination,
                        callee,
                        first_argument,
                    } => {
                        self.sync_ip(ip);
                        let callee_value = read_register!(registers, callee);
                        let window_start = self.current_base() + first_argument.index() as usize;
                        let count = usize::from(argument_count.value());
                        if let Err(control) = self.call_value_in_place(
                            callee_value,
                            destination.index(),
                            window_start,
                            count,
                            false,
                            false,
                        ) {
                            self.handle_control(control, floor)?;
                            continue 'dispatch;
                        }

                        reload_frame!(self, chunk, code, ip, registers);
                        continue 'instructions;
                    }
                    Instruction::CallValueUnchecked {
                        argument_count,
                        destination,
                        callee,
                        first_argument,
                    } => {
                        self.sync_ip(ip);
                        let callee_register = self.current_base() + callee.index() as usize;
                        let window_start =
                            self.current_base() + first_argument.index() as usize;
                        let count = usize::from(argument_count.value());
                        if let Err(control) = self.call_proven_value_site(
                            callee_register,
                            destination.index(),
                            window_start,
                            count,
                        ) {
                            self.handle_control(control, floor)?;
                            continue 'dispatch;
                        }

                        reload_frame!(self, chunk, code, ip, registers);
                        continue 'instructions;
                    }
                    Instruction::CallValueDiscarded {
                        argument_count,
                        destination,
                        callee,
                        first_argument,
                    } => {
                        self.sync_ip(ip);
                        self.begin_discarded_result_check();
                        let callee_value = read_register!(registers, callee);
                        let window_start = self.current_base() + first_argument.index() as usize;
                        let count = usize::from(argument_count.value());
                        let outcome = self.call_value_in_place(
                            callee_value,
                            destination.index(),
                            window_start,
                            count,
                            true,
                            false,
                        );
                        if let Err(control) = outcome {
                            self.handle_control(control, floor)?;
                            continue 'dispatch;
                        }

                        reload_frame!(self, chunk, code, ip, registers);
                        continue 'instructions;
                    }
                    Instruction::CallMethodUnchecked {
                        argument_count,
                        destination,
                        first_argument,
                        cache,
                    } => {
                        self.sync_ip(ip);
                        let count = usize::from(argument_count.value());
                        let window_start = self.current_base() + first_argument.index() as usize;
                        if let Err(control) = self.call_exact_method_site(
                            cache.index() as usize,
                            chunk,
                            destination.index(),
                            window_start,
                            count,
                        ) {
                            self.handle_control(control, floor)?;
                            continue 'dispatch;
                        }

                        reload_frame!(self, chunk, code, ip, registers);
                        continue 'instructions;
                    }
                    Instruction::CallMethodDirect {
                        argument_count,
                        destination,
                        first_argument,
                        cache,
                    } => {
                        self.sync_ip(ip);
                        let count = usize::from(argument_count.value());
                        let window_start = self.current_base() + first_argument.index() as usize;
                        if let Err(control) = self.call_direct_method_site(
                            cache.index() as usize,
                            chunk,
                            destination.index(),
                            window_start,
                            count,
                        ) {
                            self.handle_control(control, floor)?;
                            continue 'dispatch;
                        }

                        reload_frame!(self, chunk, code, ip, registers);
                        continue 'instructions;
                    }
                    Instruction::ConstantGet { destination, cache } => {
                        self.sync_ip(ip);
                        let value = match self.constant_value(cache.index() as usize, chunk) {
                            Ok(value) => value,
                            Err(control) => {
                                self.handle_control(control, floor)?;
                                continue 'dispatch;
                            }
                        };

                        let target = self.current_base() + destination.index() as usize;
                        self.store_result(target, value);
                        continue 'dispatch;
                    }
                    Instruction::NewStatic { destination, cache } => {
                        self.sync_ip(ip);
                        let site = cache.index() as usize;
                        match self.new_static_site(site, chunk) {
                            Ok(value) => {
                                let target = self.current_base() + destination.index() as usize;
                                self.store_result(target, value);
                            }
                            Err(control) => self.handle_control(control, floor)?,
                        }

                        enter_finalizer_dispatch!(self, ip, floor);
                        continue 'dispatch;
                    }
                    Instruction::NewTyped {
                        destination,
                        descriptor,
                    } => {
                        self.sync_ip(ip);
                        let outer = self.current_frame().type_environment;
                        let source = chunk.type_descriptors[descriptor.index() as usize].clone();
                        let concrete = self.substitute_descriptor(&source, outer, 0);
                        let outcome = match concrete {
                            TypeDescriptor::Named {
                                name, arguments, ..
                            } => {
                                self.resolve_class_reference(name).and_then(|class| {
                                    self.new_instance_typed(
                                        class,
                                        arguments.as_deref(),
                                        TypeEnvironmentId::default(),
                                    )
                                })
                            }
                            other => Err(self.throw_well_known(
                                self.engine.tables.well_known.type_error,
                                format!(
                                    "cannot instantiate the non-class type {}",
                                    self.render_descriptor(&other)
                                ),
                            )),
                        };

                        match outcome {
                            Ok(value) => {
                                let target = self.current_base() + destination.index() as usize;
                                self.store_result(target, value);
                            }
                            Err(control) => self.handle_control(control, floor)?,
                        }

                        enter_finalizer_dispatch!(self, ip, floor);
                        continue 'dispatch;
                    }
                    Instruction::CallMethod {
                        argument_count,
                        destination,
                        first_argument,
                        cache,
                    } => {
                        self.sync_ip(ip);
                        let count = usize::from(argument_count.value());
                        let window_start = self.current_base() + first_argument.index() as usize;
                        if let Err(control) = self.call_method_site(
                            cache.index() as usize,
                            chunk,
                            destination.index(),
                            window_start,
                            count,
                            false,
                            false,
                        ) {
                            self.handle_control(control, floor)?;
                        }

                        continue 'dispatch;
                    }
                    Instruction::CallMethodDiscarded {
                        argument_count,
                        destination,
                        first_argument,
                        cache,
                    } => {
                        self.sync_ip(ip);
                        self.begin_discarded_result_check();
                        let count = usize::from(argument_count.value());
                        let window_start = self.current_base() + first_argument.index() as usize;
                        let outcome = self.call_method_site(
                            cache.index() as usize,
                            chunk,
                            destination.index(),
                            window_start,
                            count,
                            false,
                            true,
                        );
                        if let Err(control) = outcome {
                            self.handle_control(control, floor)?;
                        }

                        continue 'dispatch;
                    }
                    Instruction::Return { source } => {
                        // SAFETY: verified bytecode keeps operands in the live frame and proves their types.
                        let result = unsafe { self.take_return_register(registers, source) };
                        match self.return_value_is_valid(&result) {
                            Ok(true) => {}
                            Ok(false) => {
                                self.sync_ip(ip);
                                let control = self.return_type_mismatch(&result);
                                self.handle_control(control, floor)?;
                                continue 'dispatch;
                            }
                            Err(control) => {
                                self.handle_control(control, floor)?;
                                continue 'dispatch;
                            }
                        }

                        if let Some(value) =
                            self.return_from_register_frame(result, floor, source)
                        {
                            return Ok(value);
                        }

                        reload_frame!(self, chunk, code, ip, registers);
                        continue 'instructions;
                    }
                    Instruction::ReturnUnchecked { source } => {
                        // SAFETY: verified bytecode keeps operands in the live frame and proves their types.
                        let result = unsafe { self.take_return_register(registers, source) };
                        if let Some(value) = self.return_from_frame(result, floor) {
                            return Ok(value);
                        }

                        reload_frame!(self, chunk, code, ip, registers);
                        continue 'instructions;
                    }
                    Instruction::ReturnReferenceUnchecked { source } => {
                        // SAFETY: verified bytecode keeps operands in the live frame and proves their types.
                        let result = unsafe { self.take_return_register(registers, source) };
                        if let Some(value) =
                            self.return_from_reference_register_frame(result, floor, source)
                        {
                            return Ok(value);
                        }

                        reload_frame!(self, chunk, code, ip, registers);
                        continue 'instructions;
                    }
                    Instruction::ReturnPairUnchecked { first, second } => {
                        // SAFETY: verified bytecode keeps operands in the live frame and proves their types.
                        let first_value = unsafe { self.take_return_register(registers, first) };
                        let second_value = if first == second {
                            first_value.clone()
                        } else {
                            // SAFETY: verified bytecode keeps operands in the live frame and proves their types.
                            unsafe { self.take_return_register(registers, second) }
                        };
                        if self.current_frame().iterator_step() {
                            self.return_iterator_pair(first_value, second_value);
                            reload_frame!(self, chunk, code, ip, registers);
                            continue 'instructions;
                        }

                        let result = Value::tuple(TupleObject::with_pair(
                            &self.heap,
                            first_value,
                            second_value,
                        ));
                        let moved_mask = register_mask(first) | register_mask(second);
                        if let Some(value) =
                            self.return_from_frame_inner(result, floor, moved_mask, true)
                        {
                            return Ok(value);
                        }

                        reload_frame!(self, chunk, code, ip, registers);
                        continue 'instructions;
                    }
                    Instruction::ReturnScalarUnchecked { source } => {
                        // SAFETY: verified bytecode keeps operands in the live frame and proves their types.
                        let result = unsafe {
                            mem::replace(
                                &mut *registers.add(source.index() as usize),
                                Value::uninitialized(),
                            )
                        };

                        if let Some(value) = self.return_from_scalar_frame(result, floor) {
                            return Ok(value);
                        }
                        reload_frame!(self, chunk, code, ip, registers);
                        continue 'instructions;
                    }
                    Instruction::ReturnIntUnchecked { immediate } => {
                        let result = Value::int(i64::from(immediate.value()));
                        if let Some(value) = self.return_from_scalar_frame(result, floor) {
                            return Ok(value);
                        }

                        reload_frame!(self, chunk, code, ip, registers);
                        continue 'instructions;
                    }
                    Instruction::ReturnNull => {
                        match self.return_value_is_valid(&Value::null()) {
                            Ok(true) => {}
                            Ok(false) => {
                                self.sync_ip(ip);
                                let control = self.return_type_mismatch(&Value::null());
                                self.handle_control(control, floor)?;
                                continue 'dispatch;
                            }
                            Err(control) => {
                                self.handle_control(control, floor)?;
                                continue 'dispatch;
                            }
                        }

                        if let Some(value) = self.return_from_frame(Value::null(), floor) {
                            return Ok(value);
                        }

                        reload_frame!(self, chunk, code, ip, registers);
                        continue 'instructions;
                    }
                    Instruction::ReturnNullUnchecked => {
                        if self.current_frame().iterator_step() {
                            self.return_iterator_exhausted();
                            reload_frame!(self, chunk, code, ip, registers);
                            continue 'instructions;
                        }

                        if let Some(value) = self.return_from_scalar_frame(Value::null(), floor) {
                            return Ok(value);
                        }

                        reload_frame!(self, chunk, code, ip, registers);
                        continue 'instructions;
                    }
                    Instruction::Throw { source } => {
                        let value = read_register!(registers, source);
                        self.sync_ip(ip);
                        let control = match self.validate_throwable(value) {
                            Ok(value) => {
                                self.record_explicit_throw_origin(&value);
                                VirtualMachineControl::Throw(value)
                            }
                            Err(control) => control,
                        };

                        self.handle_control(control, floor)?;
                        continue 'dispatch;
                    }
                    Instruction::Rethrow => {
                        self.sync_ip(ip);
                        let frame = self.frames.len() - 1;
                        let pending = self.pending_unwinds.pop();
                        let Some(PendingUnwind {
                            frame: owner,
                            value,
                        }) = pending
                        else {
                            // SAFETY: the surrounding invariant makes this path unreachable.
                            unsafe {
                                unreachable_invariant(
                                    "a rethrow always follows a caught pending error",
                                )
                            }
                        };

                        if owner != frame {
                            // SAFETY: the surrounding invariant makes this path unreachable.
                            unsafe {
                                unreachable_invariant(
                                    "a rethrow consumes the active frame's pending error",
                                )
                            }
                        }

                        self.handle_control(VirtualMachineControl::Throw(value), floor)?;
                        continue 'dispatch;
                    }
                    Instruction::Write {
                        value_count,
                        first_value,
                    } => {
                        self.sync_ip(ip);
                        let start = self.current_base() + first_value.index() as usize;
                        if let Err(control) =
                            self.write_values(start, usize::from(value_count.value()), false, false)
                        {
                            self.handle_control(control, floor)?;
                            continue 'dispatch;
                        }
                    }
                    Instruction::WriteLine {
                        value_count,
                        first_value,
                    } => {
                        self.sync_ip(ip);
                        let start = self.current_base() + first_value.index() as usize;
                        if let Err(control) =
                            self.write_values(start, usize::from(value_count.value()), false, true)
                        {
                            self.handle_control(control, floor)?;
                            continue 'dispatch;
                        }
                    }
                    Instruction::WriteError {
                        value_count,
                        first_value,
                    } => {
                        self.sync_ip(ip);
                        let start = self.current_base() + first_value.index() as usize;
                        if let Err(control) =
                            self.write_values(start, usize::from(value_count.value()), true, false)
                        {
                            self.handle_control(control, floor)?;
                            continue 'dispatch;
                        }
                    }
                    Instruction::WriteErrorLine {
                        value_count,
                        first_value,
                    } => {
                        self.sync_ip(ip);
                        let start = self.current_base() + first_value.index() as usize;
                        if let Err(control) =
                            self.write_values(start, usize::from(value_count.value()), true, true)
                        {
                            self.handle_control(control, floor)?;
                            continue 'dispatch;
                        }
                    }
                    Instruction::Assert {
                        operand_count,
                        first_value,
                        message,
                        text,
                    } => {
                        let state = {
                            let value =
                                // SAFETY: verified bytecode keeps operands in the live frame and proves their types.
                                unsafe { &*registers.add(first_value.index() as usize) };
                            (value.as_bool(), value.kind_name())
                        };
                        match state.0 {
                            Some(true) => {}
                            Some(false) => {
                                let condition =
                                    literal_text(&chunk.constants[text.index() as usize]);
                                let mut diagnostic =
                                    format!("assertion failed: {condition}");
                                if message != Register::NONE {
                                    let value =
                                        // SAFETY: verified bytecode keeps operands in the live frame and proves their types.
                                        unsafe { &*registers.add(message.index() as usize) };
                                    let rendered = match ops::stringify_for_concat(&self.heap, value)
                                    {
                                        Some(rendered) => {
                                            String::from_utf8_lossy(rendered.flatten())
                                                .into_owned()
                                        }
                                        None => self.assertion_debug_render(value),
                                    };
                                    diagnostic.push_str("\n  message: ");
                                    diagnostic.push_str(&rendered);
                                }

                                match operand_count.value() {
                                    1 => {
                                        // SAFETY: verified bytecode keeps operands in the live frame and proves their types.
                                        let subject = unsafe {
                                            &*registers
                                                .add(first_value.index() as usize + 1)
                                        };

                                        diagnostic.push_str("\n  subject: ");
                                        diagnostic
                                            .push_str(&self.assertion_debug_render(subject));
                                    }
                                    2 => {
                                        // SAFETY: verified bytecode keeps operands in the live frame and proves their types.
                                        let left = unsafe {
                                            &*registers
                                                .add(first_value.index() as usize + 1)
                                        };

                                        // SAFETY: verified bytecode keeps operands in the live frame and proves their types.
                                        let right = unsafe {
                                            &*registers
                                                .add(first_value.index() as usize + 2)
                                        };

                                        diagnostic.push_str("\n  left:  ");
                                        diagnostic.push_str(&self.assertion_debug_render(left));
                                        diagnostic.push_str("\n  right: ");
                                        diagnostic.push_str(&self.assertion_debug_render(right));
                                    }
                                    _ => {}
                                }

                                fail!(
                                    self,
                                    ip,
                                    floor,
                                    'dispatch,
                                    self.throw_well_known(
                                        self.engine.tables.well_known.assertion_error,
                                        diagnostic,
                                    )
                                );
                            }
                            None => {
                                fail!(
                                    self,
                                    ip,
                                    floor,
                                    'dispatch,
                                    self.throw_well_known(
                                        self.engine.tables.well_known.type_error,
                                        format!(
                                            "an assert! condition must be bool, {} given",
                                            state.1
                                        ),
                                    )
                                );
                            }
                        }
                    }
                    Instruction::Exit { code } => {
                        self.sync_ip(ip);
                        let exit_code = if code == Register::NONE {
                            0
                        } else {
                            let state = {
                                // SAFETY: verified bytecode keeps operands in the live frame and proves their types.
                                let value = unsafe { &*registers.add(code.index() as usize) };
                                (value.as_int(), value.kind_name())
                            };

                            match state.0 {
                                Some(value) => value,
                                None => {
                                    fail!(
                                        self,
                                        ip,
                                        floor,
                                        'dispatch,
                                        self.throw_well_known(
                                            self.engine.tables.well_known.type_error,
                                            format!(
                                                "an exit! code must be int, {} given",
                                                state.1
                                            ),
                                        )
                                    );
                                }
                            }
                        };

                        return Err(VirtualMachineControl::Exit((exit_code & 0xFF) as u8));
                    }
                    Instruction::Panic { message } => {
                        self.sync_ip(ip);
                        let message = match &chunk.constants[usize::from(message.index())] {
                            Literal::String(message) => message.clone(),
                            // SAFETY: the surrounding invariant makes this path unreachable.
                            _ => unsafe {
                                unreachable_invariant(
                                    "a verified panic instruction names a string constant",
                                )
                            },
                        };
                        let trace = self.capture_trace();
                        self.engine.write_panic(message.as_bytes(), &trace);

                        return Err(VirtualMachineControl::Exit(255));
                    }
                    Instruction::MakeClosure {
                        capture_count,
                        destination,
                        prototype,
                        first_capture,
                    } => {
                        let name = match &chunk.constants[prototype.index() as usize] {
                            Literal::String(atom) => atom.clone(),
                            // SAFETY: the surrounding invariant makes this path unreachable.
                            _ => unsafe {
                                unreachable_invariant("a closure prototype is a name string")
                            },
                        };

                        let (frame_unit, frame_environment) = {
                            let frame = self.current_frame();
                            (frame.unit, frame.type_environment)
                        };

                        // SAFETY: verified bytecode keeps operands in the live frame and proves their types.
                        let unit = unsafe { frame_unit.as_ref() };
                        let Some(function) = unit.closures.get(&name).copied() else {
                            // SAFETY: the surrounding invariant makes this path unreachable.
                            unsafe {
                                unreachable_invariant(
                                    "every closure prototype is registered at declaration",
                                )
                            }
                        };

                        let signature =
                            self.engine.tables.functions[function.0 as usize].signature.clone();
                        let count = usize::from(capture_count.value());
                        let start = self.current_base() + first_capture.index() as usize;
                        let mut captures = Vec::with_capacity(count);
                        for position in 0..count {
                            captures.push(self.stack[start + position].clone());
                        }

                        let closure = FunctionObject::closure(
                            &self.heap,
                            CallTarget::User(function),
                            captures,
                            signature,
                            self.current_frame().class_scope.get(),
                            frame_environment,
                        );

                        write_register!(registers, destination, Value::function(closure));
                    }
                    Instruction::MakeBound {
                        destination,
                        callee,
                        descriptor,
                    } => {
                        self.sync_ip(ip);
                        let callee_value = read_register!(registers, callee);
                        let window_start = self.current_base() + callee.index() as usize + 1;
                        let outcome = self.bind_partial(
                            &callee_value,
                            &chunk.preset_descriptors[descriptor.index() as usize],
                            descriptor.index() as usize,
                            window_start,
                        );

                        match outcome {
                            Ok(bound) => {
                                let target = self.current_base() + destination.index() as usize;
                                self.store_result(target, bound);
                            }
                            Err(control) => {
                                self.handle_control(control, floor)?;
                            }
                        }

                        continue 'dispatch;
                    }
                    Instruction::CallWithNames {
                        destination,
                        callee,
                        descriptor,
                    } => {
                        self.sync_ip(ip);
                        let callee_value = read_register!(registers, callee);
                        let window_start = self.current_base() + callee.index() as usize + 1;
                        if let Err(control) = self.call_with_names_site(
                            &callee_value,
                            descriptor.index() as usize,
                            &chunk.call_descriptors[descriptor.index() as usize],
                            window_start,
                            destination.index(),
                            false,
                        ) {
                            self.handle_control(control, floor)?;
                        }

                        continue 'dispatch;
                    }
                    Instruction::CallWithNamesDiscarded {
                        destination,
                        callee,
                        descriptor,
                    } => {
                        self.sync_ip(ip);
                        self.begin_discarded_result_check();
                        let callee_value = read_register!(registers, callee);
                        let window_start = self.current_base() + callee.index() as usize + 1;
                        let outcome = self.call_with_names_site(
                            &callee_value,
                            descriptor.index() as usize,
                            &chunk.call_descriptors[descriptor.index() as usize],
                            window_start,
                            destination.index(),
                            true,
                        );
                        if let Err(control) = outcome {
                            self.handle_control(control, floor)?;
                        }

                        continue 'dispatch;
                    }
                    Instruction::CheckDiscardedResult { source: _ } => {
                        let Some(discarded) = self.pending_discarded_result.take() else {
                            continue 'instructions;
                        };
                        self.sync_ip(ip);
                        let callable = discarded.callable.to_string_lossy();
                        let mut message = format!("the result of {callable} was discarded");
                        if let Some(note) = discarded.note {
                            message.push_str(": ");
                            message.push_str(&note.to_string_lossy());
                        }
                        fail!(
                            self,
                            ip,
                            floor,
                            'dispatch,
                            self.throw_well_known(
                                self.engine.tables.well_known.discarded_result_error,
                                message,
                            )
                        );
                    }
                    Instruction::PropertyGet {
                        destination,
                        object,
                        cache,
                    } => {
                        let receiver_class = {
                            // SAFETY: verified bytecode keeps operands in the live frame and proves their types.
                            let value = unsafe { &*registers.add(object.index() as usize) };
                            match value.as_object() {
                                Some(instance) => Ok(instance.class()),
                                None => Err(value.kind_name()),
                            }
                        };

                        let receiver_class = match receiver_class {
                            Ok(receiver_class) => receiver_class,
                            Err(kind) => {
                                self.sync_ip(ip);
                                fail!(
                                    self,
                                    ip,
                                    floor,
                                    'dispatch,
                                    self.throw_well_known(
                                        self.engine.tables.well_known.type_error,
                                        format!("cannot access a property on {kind}"),
                                    )
                                );
                            }
                        };

                        let site = cache.index() as usize;
                        let slot = match self.cached_property_slot(site, receiver_class) {
                            Some(slot) => slot,
                            None => {
                                self.sync_ip(ip);
                                match self.property_slot_for(site, chunk, receiver_class) {
                                    Ok(slot) => slot,
                                    Err(control) => {
                                        fail!(self, ip, floor, 'dispatch, control)
                                    }
                                }
                            }
                        };

                        // SAFETY: verified bytecode keeps operands in the live frame and proves their types.
                        let value = unsafe {
                            object_register(registers, object).read_slot_unchecked(slot as usize)
                        };

                        if value.is_uninitialized() {
                            self.sync_ip(ip);
                            let name = name_atom(chunk, cache.index() as usize);
                            // SAFETY: verified bytecode keeps operands in the live frame and proves their types.
                            let receiver = unsafe { object_register(registers, object) };
                            fail!(
                                self,
                                ip,
                                floor,
                                'dispatch,
                                self.uninitialized_property_error(receiver, name)
                            );
                        }

                        write_register!(registers, destination, value);
                    }
                    Instruction::PropertyGetUnchecked {
                        destination,
                        object,
                        slot,
                        value_mode,
                    } => {
                        // SAFETY: verified bytecode keeps operands in the live frame and proves their types.
                        let receiver = unsafe { object_register(registers, object) };
                        let slot_index = usize::from(slot.index());
                        if value_mode == PropertyReadMode::Clone
                            // SAFETY: verified bytecode keeps operands in the live frame and proves their types.
                            && let Some(value) = unsafe {
                                receiver.read_int_slot_unchecked(slot_index)
                            }
                        {
                            write_register!(registers, destination, Value::int(value));
                            continue 'instructions;
                        }

                        let value = match value_mode {
                            // SAFETY: verified bytecode keeps operands in the live frame and proves their types.
                            PropertyReadMode::Clone => unsafe {
                                receiver.read_slot_unchecked(slot_index)
                            },
                            // SAFETY: verified bytecode keeps operands in the live frame and proves their types.
                            PropertyReadMode::Take => unsafe {
                                receiver.take_slot_unchecked(slot_index)
                            },
                        };
                        if value.is_uninitialized() {
                            self.sync_ip(ip);
                            fail!(
                                self,
                                ip,
                                floor,
                                'dispatch,
                                self.uninitialized_property_slot_error(
                                    receiver,
                                    usize::from(slot.index()),
                                )
                            );
                        }

                        write_register!(registers, destination, value);
                    }
                    Instruction::PropertySet {
                        object,
                        value,
                        cache,
                    } => {
                        self.sync_ip(ip);
                        let receiver = {
                            // SAFETY: verified bytecode keeps operands in the live frame and proves their types.
                            let target = unsafe { &*registers.add(object.index() as usize) };
                            match target.as_object() {
                                Some(instance) => {
                                    let receiver_was_unique = object != value && instance.is_unique();
                                    Ok((instance.clone(), receiver_was_unique))
                                }
                                None => Err(target.kind_name()),
                            }
                        };

                        let (receiver, receiver_was_unique) = match receiver {
                            Ok(receiver) => receiver,
                            Err(kind) => {
                                fail!(
                                    self,
                                    ip,
                                    floor,
                                    'dispatch,
                                    self.throw_well_known(
                                        self.engine.tables.well_known.type_error,
                                        format!("cannot write a property on {kind}"),
                                    )
                                );
                            }
                        };

                        let slot = match self.property_slot_for(
                            cache.index() as usize,
                            chunk,
                            receiver.class(),
                        ) {
                            Ok(slot) => slot,
                            Err(control) => fail!(self, ip, floor, 'dispatch, control),
                        };

                        if let Err(control) = self.check_readonly_write(
                            &receiver,
                            slot,
                            chunk,
                            cache.index() as usize,
                        ) {
                            fail!(self, ip, floor, 'dispatch, control);
                        }

                        let new_value = read_register!(registers, value);
                        if let Err(control) =
                            self.check_instance_property_value(&receiver, slot, &new_value)
                        {
                            fail!(self, ip, floor, 'dispatch, control);
                        }

                        // SAFETY: verified bytecode keeps operands in the live frame and proves their types.
                        drop(unsafe {
                            receiver.write_slot_unchecked_with_unique_receiver(
                                slot as usize,
                                new_value,
                                receiver_was_unique,
                            )
                        });
                    }
                    Instruction::PropertySetUnchecked {
                        object,
                        value,
                        slot,
                        value_mode,
                    } => {
                        // SAFETY: verified bytecode keeps operands in the live frame and proves their types.
                        let receiver = unsafe { object_register(registers, object) };
                        if value_mode.fresh_receiver() {
                            // SAFETY: verified bytecode keeps operands in the live frame and proves their types.
                            unsafe {
                                self.initialize_fresh_property(
                                    registers,
                                    receiver,
                                    value,
                                    slot,
                                    value_mode,
                                );
                            }
                        } else {
                            let value_register = value;
                            let value = if value_mode.moves() {
                                // SAFETY: verified bytecode keeps operands in the live frame and proves their types.
                                unsafe {
                                    self.take_owned_register(
                                        registers,
                                        value,
                                        value_mode.clears_reference_mask(),
                                    )
                                }
                            } else {
                                read_register!(registers, value)
                            };
                            let receiver_was_unique =
                                object != value_register && receiver.is_unique();
                            // SAFETY: verified bytecode keeps operands in the live frame and proves their types.
                            drop(unsafe {
                                receiver.write_slot_unchecked_with_unique_receiver(
                                    usize::from(slot.index()),
                                    value,
                                    receiver_was_unique,
                                )
                            });
                        }
                    }
                    Instruction::InitializeProperties {
                        object,
                        cache,
                        descriptor,
                    } => {
                        let initializer = chunk.property_initialization_descriptor(descriptor);
                        if initializer.allocates {
                            // SAFETY: verified bytecode keeps operands in the live frame and proves their types.
                            if !unsafe {
                                self.try_initialize_cached_static_object(
                                    registers,
                                    object,
                                    cache.index(),
                                    &initializer.entries,
                                )
                            } {
                                self.sync_ip(ip);
                                if let Err(control) = self.initialize_static_object_slow(
                                    object,
                                    cache.index(),
                                    descriptor,
                                    chunk,
                                ) {
                                    self.handle_control(control, floor)?;
                                }

                                enter_finalizer_dispatch!(self, ip, floor);
                                continue 'dispatch;
                            }
                        } else {
                            // SAFETY: verified bytecode keeps operands in the live frame and proves their types.
                            let receiver = unsafe { object_register(registers, object) };
                            for entry in &initializer.entries {
                                // SAFETY: verified bytecode keeps operands in the live frame and proves their types.
                                unsafe {
                                    self.initialize_fresh_property(
                                        registers,
                                        receiver,
                                        entry.value,
                                        entry.slot,
                                        entry.value_mode,
                                    );
                                }
                            }
                        }
                    }
                    Instruction::PropertyIndexSet {
                        object,
                        first_operand,
                        cache,
                    } => {
                        self.sync_ip(ip);
                        let object = read_register!(registers, object);
                        let index = read_register!(registers, first_operand);
                        let value = read_register!(
                            registers,
                            Register::new(first_operand.index() + 1)
                        );
                        if let Err(control) = self.set_property_index(
                            object,
                            index,
                            value,
                            chunk,
                            cache.index() as usize,
                        ) {
                            fail!(self, ip, floor, 'dispatch, control);
                        }
                    }
                    Instruction::PropertyIndexSetUnchecked {
                        object,
                        first_operand,
                        slot,
                    } => {
                        self.sync_ip(ip);
                        // SAFETY: verified bytecode keeps operands in the live frame and proves their types.
                        let receiver = unsafe { object_register(registers, object) };
                        let index = read_register!(registers, first_operand);
                        let value = read_register!(
                            registers,
                            Register::new(first_operand.index() + 1)
                        );
                        if let Err(control) = self.set_property_index_unchecked(
                            receiver,
                            index,
                            value,
                            u32::from(slot.index()),
                        ) {
                            fail!(self, ip, floor, 'dispatch, control);
                        }
                    }
                    Instruction::PropertyRemove {
                        object,
                        destination,
                        cache,
                        mode,
                    } => {
                        self.sync_ip(ip);
                        let object = read_register!(registers, object);
                        let operand = if mode.uses_operand() {
                            Some(read_register!(
                                registers,
                                Register::new(destination.index() + 1)
                            ))
                        } else {
                            None
                        };
                        let removed = match self.remove_property(
                            object,
                            operand,
                            mode,
                            chunk,
                            cache.index() as usize,
                        ) {
                            Ok(removed) => removed,
                            Err(control) => fail!(self, ip, floor, 'dispatch, control),
                        };

                        write_register!(registers, destination, removed);
                    }
                    Instruction::PropertyRemoveUnchecked {
                        object,
                        destination,
                        slot,
                        mode,
                    } => {
                        self.sync_ip(ip);
                        // SAFETY: verified bytecode keeps operands in the live frame and proves their types.
                        let receiver = unsafe { object_register(registers, object) };
                        let operand = if mode.uses_operand() {
                            Some(read_register!(
                                registers,
                                Register::new(destination.index() + 1)
                            ))
                        } else {
                            None
                        };
                        let removed = match self.remove_property_unchecked(
                            receiver,
                            operand,
                            mode,
                            u32::from(slot.index()),
                        ) {
                            Ok(removed) => removed,
                            Err(control) => fail!(self, ip, floor, 'dispatch, control),
                        };

                        write_register!(registers, destination, removed);
                    }
                    Instruction::PropertyIndexUpdate {
                        object,
                        operand,
                        cache,
                        mode,
                    } => {
                        if mode == PropertyIndexUpdateMode::Append {
                            self.sync_ip(ip);
                            let object = read_register!(registers, object);
                            let value = read_register!(registers, operand);
                            if let Err(control) = self.append_property(
                                object,
                                value,
                                chunk,
                                cache.index() as usize,
                            ) {
                                fail!(self, ip, floor, 'dispatch, control);
                            }

                            continue 'instructions;
                        }

                        self.sync_ip(ip);
                        let receiver = {
                            // SAFETY: verified bytecode keeps operands in the live frame and proves their types.
                            let target = unsafe { &*registers.add(object.index() as usize) };
                            match target.as_object() {
                                Some(instance) => Ok(instance.clone()),
                                None => Err(target.kind_name()),
                            }
                        };

                        let receiver = match receiver {
                            Ok(receiver) => receiver,
                            Err(kind) => {
                                fail!(
                                    self,
                                    ip,
                                    floor,
                                    'dispatch,
                                    self.throw_well_known(
                                        self.engine.tables.well_known.type_error,
                                        format!("cannot access a property on {kind}"),
                                    )
                                );
                            }
                        };

                        let slot = match self.property_slot_for(
                            cache.index() as usize,
                            chunk,
                            receiver.class(),
                        ) {
                            Ok(slot) => slot,
                            Err(control) => fail!(self, ip, floor, 'dispatch, control),
                        };

                        if receiver.slot_is_uninitialized(slot as usize) {
                            let name = name_atom(chunk, cache.index() as usize);
                            fail!(
                                self,
                                ip,
                                floor,
                                'dispatch,
                                self.uninitialized_property_error(&receiver, name)
                            );
                        }

                        let index_value = read_register!(registers, operand);
                        if mode == PropertyIndexUpdateMode::Remove {
                            let mut property = receiver.read_slot(slot as usize);
                            let removed = match remove_entry(
                                &self.heap,
                                &mut property,
                                &index_value,
                            ) {
                                Ok(removed) => removed,
                                Err(fault) => {
                                    fail!(
                                        self,
                                        ip,
                                        floor,
                                        'dispatch,
                                        self.array_fault(fault)
                                    );
                                }
                            };

                            if let Err(control) = self.check_readonly_write(
                                &receiver,
                                slot,
                                chunk,
                                cache.index() as usize,
                            ) {
                                fail!(self, ip, floor, 'dispatch, control);
                            }
                            if let Err(control) =
                                self.check_instance_property_value(&receiver, slot, &property)
                            {
                                fail!(self, ip, floor, 'dispatch, control);
                            }

                            drop(receiver.write_slot(slot as usize, property));
                            drop(removed);
                            continue 'instructions;
                        }

                        let property = receiver.read_slot(slot as usize);
                        let previous = match index_get(&self.heap, &property, &index_value) {
                            Ok(previous) => previous,
                            Err(fault) => {
                                fail!(self, ip, floor, 'dispatch, self.array_fault(fault));
                            }
                        };

                        drop(property);
                        let incremented = match step_by(&previous, 1) {
                            Ok(incremented) => incremented,
                            Err(fault) => {
                                let kind = previous.kind_name();
                                fail!(
                                    self,
                                    ip,
                                    floor,
                                    'dispatch,
                                    self.binary_fault(fault, "+", kind, "int")
                                );
                            }
                        };

                        if let Err(control) = self.check_readonly_write(
                            &receiver,
                            slot,
                            chunk,
                            cache.index() as usize,
                        ) {
                            fail!(self, ip, floor, 'dispatch, control);
                        }

                        let preserves_type = self.instance_property_index_update_preserves_type(
                            &receiver,
                            slot,
                            &incremented,
                        );

                        let replaced = receiver.mutate_slot(slot as usize, |property| {
                            index_replace_existing(property, &index_value, incremented)
                        });

                        let replaced = match replaced {
                            Ok(replaced) => replaced,
                            Err(fault) => {
                                fail!(self, ip, floor, 'dispatch, self.array_fault(fault));
                            }
                        };

                        let valid = if preserves_type {
                            Ok(())
                        } else {
                            let updated = receiver.read_slot(slot as usize);
                            let valid =
                                self.check_instance_property_value(&receiver, slot, &updated);
                            drop(updated);
                            valid
                        };

                        if let Err(control) = valid {
                            let rollback = receiver.mutate_slot(slot as usize, |property| {
                                index_replace_existing(property, &index_value, replaced)
                            });

                            match rollback {
                                Ok(incremented) => drop(incremented),
                                // SAFETY: the surrounding invariant makes this path unreachable.
                                Err(_) => unsafe {
                                    unreachable_invariant(
                                        "an indexed property increment rolls back its existing key",
                                    )
                                },
                            }

                            fail!(self, ip, floor, 'dispatch, control);
                        }
                    }
                    Instruction::PropertyIndexUpdateUnchecked {
                        object,
                        operand,
                        slot,
                        mode,
                    } => {
                        self.sync_ip(ip);
                        // SAFETY: verified bytecode keeps operands in the live frame and proves their types.
                        let receiver = unsafe { object_register(registers, object) };
                        let operand = read_register!(registers, operand);
                        match mode {
                            PropertyIndexUpdateMode::Increment => {
                                if let Err(control) = self.increment_property_index_unchecked(
                                    receiver,
                                    operand,
                                    u32::from(slot.index()),
                                ) {
                                    fail!(self, ip, floor, 'dispatch, control);
                                }
                            }
                            PropertyIndexUpdateMode::Remove => {
                                let outcome = receiver.mutate_slot(
                                    usize::from(slot.index()),
                                    |property| remove_entry(&self.heap, property, &operand),
                                );
                                match outcome {
                                    Ok(removed) => drop(removed),
                                    Err(fault) => {
                                        fail!(
                                            self,
                                            ip,
                                            floor,
                                            'dispatch,
                                            self.array_fault(fault)
                                        );
                                    }
                                }
                            }
                            PropertyIndexUpdateMode::Append => {
                                if let Err(control) = self.append_property_unchecked(
                                    receiver,
                                    operand,
                                    u32::from(slot.index()),
                                ) {
                                    fail!(self, ip, floor, 'dispatch, control);
                                }
                            }
                        }
                    }
                    Instruction::PropertyFillIntRange {
                        object,
                        first_operand,
                        cache,
                    } => {
                        self.sync_ip(ip);
                        let object = read_register!(registers, object);
                        let value = read_register!(registers, first_operand);
                        let limit = read_register!(
                            registers,
                            Register::new(first_operand.index() + 1)
                        );

                        if let Err(control) = self.fill_property_int_range(
                            object,
                            value,
                            limit,
                            chunk,
                            cache.index() as usize,
                        ) {
                            fail!(self, ip, floor, 'dispatch, control);
                        }
                    }
                    Instruction::PropertyStep {
                        object,
                        cache,
                        immediate,
                    } => {
                        self.sync_ip(ip);
                        let object = read_register!(registers, object);
                        if let Err(control) = self.step_property(
                            object,
                            chunk,
                            cache.index() as usize,
                            i64::from(immediate.value()),
                        ) {
                            fail!(self, ip, floor, 'dispatch, control);
                        }
                    }
                    Instruction::PropertyStepUnchecked {
                        object,
                        slot,
                        immediate,
                    } => {
                        // SAFETY: verified bytecode keeps operands in the live frame and proves their types.
                        let receiver = unsafe { object_register(registers, object) };
                        let slot_index = usize::from(slot.index());
                        let step = i64::from(immediate.value());
                        if let Some(current) =
                            // SAFETY: verified bytecode keeps operands in the live frame and proves their types.
                            unsafe { receiver.read_int_slot_unchecked(slot_index) }
                        {
                            let Some(updated) = current.checked_add(step) else {
                                self.sync_ip(ip);
                                let fault = if step >= 0 {
                                    Fault::Overflow
                                } else {
                                    Fault::Underflow
                                };
                                fail!(
                                    self,
                                    ip,
                                    floor,
                                    'dispatch,
                                    self.binary_fault(fault, "+", "int", "int")
                                );
                            };
                            // SAFETY: verified bytecode keeps operands in the live frame and proves their types.
                            unsafe { receiver.write_int_slot_unchecked(slot_index, updated) };
                            continue 'instructions;
                        }

                        self.sync_ip(ip);
                        if let Err(control) = self.step_property_unchecked(
                            receiver,
                            u32::from(slot.index()),
                            step,
                        ) {
                            fail!(self, ip, floor, 'dispatch, control);
                        }
                    }
                    Instruction::PropertyAdd {
                        object,
                        source,
                        cache,
                    } => {
                        self.sync_ip(ip);
                        let object = read_register!(registers, object);
                        let source = read_register!(registers, source);
                        if let Err(control) = self.add_to_property(
                            object,
                            source,
                            chunk,
                            cache.index() as usize,
                        ) {
                            fail!(self, ip, floor, 'dispatch, control);
                        }
                    }
                    Instruction::PropertyAddUnchecked {
                        object,
                        source,
                        slot,
                    } => {
                        self.sync_ip(ip);
                        // SAFETY: verified bytecode keeps operands in the live frame and proves their types.
                        let receiver = unsafe { object_register(registers, object) };
                        let source = read_register!(registers, source);
                        if let Err(control) = self.add_to_property_unchecked(
                            receiver,
                            source,
                            u32::from(slot.index()),
                        ) {
                            fail!(self, ip, floor, 'dispatch, control);
                        }
                    }
                    Instruction::PropertyInitRaw {
                        object,
                        value,
                        cache,
                    } => {
                        self.sync_ip(ip);
                        let receiver = {
                            // SAFETY: verified bytecode keeps operands in the live frame and proves their types.
                            let target = unsafe { &*registers.add(object.index() as usize) };
                            // SAFETY: the surrounding invariant makes this path unreachable.
                            target.as_object().cloned().unwrap_or_else(|| unsafe {
                                unreachable_invariant("raw property writes target fresh instances")
                            })
                        };

                        let site = cache.index() as usize;
                        let slot = match self.raw_property_slot(site, chunk, receiver.class()) {
                            Ok(slot) => slot,
                            Err(control) => {
                                fail!(self, ip, floor, 'dispatch, control);
                            }
                        };

                        if let Err(control) = self.check_raw_property_write(&receiver, slot) {
                            fail!(self, ip, floor, 'dispatch, control);
                        }

                        let new_value = read_register!(registers, value);
                        if let Err(control) = self
                            .check_instance_property_value_at_site(site, &receiver, slot, &new_value)
                        {
                            fail!(self, ip, floor, 'dispatch, control);
                        }

                        drop(receiver.write_slot(slot as usize, new_value));
                    }
                    Instruction::CloneObject {
                        destination,
                        source,
                    } => {
                        // SAFETY: verified bytecode keeps operands in the live frame and proves their types.
                        let value = unsafe { &*registers.add(source.index() as usize) };
                        let original = match value.as_object() {
                            Some(instance) => instance.clone(),
                            None => {
                                let kind = value.kind_name();
                                fail!(
                                    self,
                                    ip,
                                    floor,
                                    'dispatch,
                                    self.throw_well_known(
                                        self.engine.tables.well_known.type_error,
                                        format!("clone!() requires an object, {kind} given"),
                                    )
                                );
                            }
                        };

                        if original.has_built_in() {
                            let class_name = String::from_utf8_lossy(
                                self.engine.tables.classes[original.class().0 as usize]
                                    .name
                                    .as_bytes(),
                            )
                            .into_owned();
                            fail!(
                                self,
                                ip,
                                floor,
                                'dispatch,
                                self.throw_well_known(
                                    self.engine.tables.well_known.type_error,
                                    format!(
                                        "cannot clone an instance of the built-in class {class_name}"
                                    ),
                                )
                            );
                        }

                        if self.engine.tables.classes[original.class().0 as usize].kind
                            == ClassLikeKind::Enum
                        {
                            let class_name = String::from_utf8_lossy(
                                self.engine.tables.classes[original.class().0 as usize]
                                    .name
                                    .as_bytes(),
                            )
                            .into_owned();
                            fail!(
                                self,
                                ip,
                                floor,
                                'dispatch,
                                self.throw_well_known(
                                    self.engine.tables.well_known.type_error,
                                    format!(
                                        "cannot clone the enum case {class_name}; every case is a \
                                         single canonical value"
                                    ),
                                )
                            );
                        }

                        let copy = InstanceObject::new_typed(
                            &self.heap,
                            original.class(),
                            original.slot_count(),
                            original.type_environment(),
                        );

                        for slot in 0..original.slot_count() {
                            drop(copy.write_slot(slot, original.read_slot(slot)));
                        }

                        write_register!(registers, destination, Value::object(copy));
                        enter_finalizer_dispatch!(self, ip, floor);
                    }
                    Instruction::StaticPropertyGet { destination, cache } => {
                        self.sync_ip(ip);
                        let resolved = match self.static_slot_for(cache.index() as usize, chunk) {
                            Ok(resolved) => resolved,
                            Err(control) => {
                                self.handle_control(control, floor)?;
                                continue 'dispatch;
                            }
                        };

                        let (class, slot) = resolved;
                        let value = self.engine.tables.classes[class.0 as usize].statics.borrow()
                            [slot as usize]
                            .clone();
                        if value.is_uninitialized() {
                            let (_, member) = class_member_names(chunk, cache.index() as usize);
                            let control = self.throw_well_known(
                                self.engine.tables.well_known.uninitialized_property_error,
                                format!(
                                    "the static property ${member} is read before initialization"
                                ),
                            );

                            self.handle_control(control, floor)?;
                            continue 'dispatch;
                        }

                        let target = self.current_base() + destination.index() as usize;
                        self.store_result(target, value);
                        continue 'dispatch;
                    }
                    Instruction::StaticPropertySet { cache, value } => {
                        self.sync_ip(ip);
                        let new_value = read_register!(registers, value);
                        let resolved = match self.static_slot_for(cache.index() as usize, chunk) {
                            Ok(resolved) => resolved,
                            Err(control) => {
                                self.handle_control(control, floor)?;
                                continue 'dispatch;
                            }
                        };

                        let (class, slot) = resolved;
                        if let Err(control) =
                            self.check_static_property_value(class, slot, &new_value)
                        {
                            self.handle_control(control, floor)?;
                            continue 'dispatch;
                        }

                        drop(mem::replace(
                            &mut self.engine.tables.classes[class.0 as usize].statics.borrow_mut()
                                [slot as usize],
                            new_value,
                        ));

                        continue 'dispatch;
                    }
                    Instruction::ClassConstantGet { destination, cache } => {
                        self.sync_ip(ip);
                        let value = match self.class_constant_for(cache.index() as usize, chunk) {
                            Ok(value) => value,
                            Err(control) => {
                                self.handle_control(control, floor)?;
                                continue 'dispatch;
                            }
                        };

                        let target = self.current_base() + destination.index() as usize;
                        self.store_result(target, value);
                        continue 'dispatch;
                    }
                    Instruction::CallStatic {
                        argument_count,
                        destination,
                        first_argument,
                        cache,
                    } => {
                        self.sync_ip(ip);
                        let count = usize::from(argument_count.value());
                        let window_start = self.current_base() + first_argument.index() as usize;
                        if let Err(control) = self.call_static_site(
                            cache.index() as usize,
                            chunk,
                            destination.index(),
                            window_start,
                            count,
                            false,
                        ) {
                            self.handle_control(control, floor)?;
                        }

                        continue 'dispatch;
                    }
                    Instruction::CallStaticDiscarded {
                        argument_count,
                        destination,
                        first_argument,
                        cache,
                    } => {
                        self.sync_ip(ip);
                        self.begin_discarded_result_check();
                        let count = usize::from(argument_count.value());
                        let window_start = self.current_base() + first_argument.index() as usize;
                        let outcome = self.call_static_site(
                            cache.index() as usize,
                            chunk,
                            destination.index(),
                            window_start,
                            count,
                            true,
                        );
                        if let Err(control) = outcome {
                            self.handle_control(control, floor)?;
                        }

                        continue 'dispatch;
                    }
                    Instruction::NewVec {
                        element_count,
                        destination,
                        first_element,
                    } => {
                        let count = usize::from(element_count.value());
                        let start = self.current_base() + first_element.index() as usize;
                        let mut elements = Vec::with_capacity(count);
                        for position in 0..count {
                            elements.push(self.stack[start + position].clone());
                        }

                        write_register!(
                            registers,
                            destination,
                            Value::vec(VecObject::with_elements(&self.heap, elements))
                        );
                    }
                    Instruction::NewFilledVec {
                        destination,
                        value,
                        size,
                    } => {
                        // SAFETY: verified bytecode keeps operands in the live frame and proves their types.
                        let size_value = unsafe { &*registers.add(size.index() as usize) };
                        let size = match size_value.transparent() {
                            ValueView::Int(size) if *size >= 0 => match usize::try_from(*size) {
                                Ok(size) => size,
                                Err(_) => {
                                    fail!(
                                        self,
                                        ip,
                                        floor,
                                        'dispatch,
                                        self.throw_well_known(
                                            self.engine.tables.well_known.overflow_error,
                                            format!("a filled vec size of {size} is too large"),
                                        )
                                    );
                                }
                            },
                            ValueView::Int(size) => {
                                fail!(
                                    self,
                                    ip,
                                    floor,
                                    'dispatch,
                                    self.throw_well_known(
                                        self.engine.tables.well_known.type_error,
                                        format!("a filled vec size must be non-negative, {size} given"),
                                    )
                                );
                            }
                            other => {
                                fail!(
                                    self,
                                    ip,
                                    floor,
                                    'dispatch,
                                    self.throw_well_known(
                                        self.engine.tables.well_known.type_error,
                                        format!(
                                            "a filled vec size must be int, {} given",
                                            other.kind_name()
                                        ),
                                    )
                                );
                            }
                        };
                        // SAFETY: verified bytecode keeps operands in the live frame and proves their types.
                        let value = unsafe { &*registers.add(value.index() as usize) };
                        let elements = repeat_n(value.clone(), size);
                        write_register!(
                            registers,
                            destination,
                            Value::vec(VecObject::with_elements(&self.heap, elements))
                        );
                    }
                    Instruction::NewTuple {
                        element_count,
                        destination,
                        first_element,
                    } => {
                        let count = usize::from(element_count.value());
                        let start = self.current_base() + first_element.index() as usize;
                        let elements = self.stack[start..start + count].iter().cloned();

                        write_register!(
                            registers,
                            destination,
                            Value::tuple(TupleObject::with_elements(&self.heap, elements))
                        );
                    }
                    Instruction::NewDict {
                        pair_count,
                        destination,
                        first_pair,
                    } => {
                        let count = usize::from(pair_count.value());
                        let start = self.current_base() + first_pair.index() as usize;
                        let mut dict = DictObject::new(&self.heap);
                        let mut fault = None;

                        let Some(entries) = dict.get_mut() else {
                            // SAFETY: the surrounding invariant makes this path unreachable.
                            unsafe {
                                unreachable_invariant("a fresh dict handle is unique")
                            }
                        };

                        for pair in 0..count {
                            let key_value = &self.stack[start + pair * 2];
                            match dict_key(key_value) {
                                Ok(key) => {
                                    let value = self.stack[start + pair * 2 + 1].clone();
                                    entries.insert(key, value);
                                }
                                Err(found) => {
                                    fault = Some(found);
                                    break;
                                }
                            }
                        }

                        if let Some(fault) = fault {
                            fail!(self, ip, floor, 'dispatch, self.array_fault(fault));
                        }

                        write_register!(registers, destination, Value::dict(dict));
                    }
                    Instruction::IndexGet {
                        destination,
                        container,
                        index,
                    } => {
                        let outcome = {
                            let container_value =
                                // SAFETY: verified bytecode keeps operands in the live frame and proves their types.
                                unsafe { &*registers.add(container.index() as usize) };
                            // SAFETY: verified bytecode keeps operands in the live frame and proves their types.
                            let index_value = unsafe { &*registers.add(index.index() as usize) };
                            index_get(&self.heap, container_value, index_value)
                        };

                        match outcome {
                            Ok(value) => write_register!(registers, destination, value),
                            Err(fault) => {
                                fail!(self, ip, floor, 'dispatch, self.array_fault(fault));
                            }
                        }
                    }
                    Instruction::StringIndexGet {
                        destination,
                        container,
                        index,
                    } => {
                        // SAFETY: verified bytecode keeps the container in the active frame.
                        let container = unsafe { &*registers.add(container.index() as usize) };
                        // SAFETY: the value's tag proves this projection is valid.
                        let bytes = unsafe { container.as_string_bytes().unwrap_unchecked() };
                        // SAFETY: verified bytecode keeps operands in the live frame and proves their types.
                        let index = unsafe { int_register(registers, index) };
                        match int_position(index, bytes.len()) {
                            Ok(position) => {
                                // SAFETY: the surrounding invariant keeps this index in bounds.
                                let byte = unsafe { *bytes.get_unchecked(position) };
                                write_register!(
                                    registers,
                                    destination,
                                    Value::string(self.heap.byte_string(byte))
                                );
                            }
                            Err(fault) => {
                                fail!(
                                    self,
                                    ip,
                                    floor,
                                    'dispatch,
                                    self.array_fault(fault)
                                );
                            }
                        }
                    }
                    Instruction::VecIndexGet {
                        destination,
                        container,
                        index,
                        value_mode,
                    } => {
                        let container =
                            // SAFETY: verified bytecode keeps operands in the live frame and proves their types.
                            unsafe { &*registers.add(container.index() as usize) };
                        // SAFETY: verified bytecode keeps operands in the live frame and proves their types.
                        let index = unsafe { int_register(registers, index) };
                        let outcome = match value_mode {
                            ArrayValueMode::Generic | ArrayValueMode::Float => {
                                vec_index_get(container, index)
                            }
                            ArrayValueMode::Int => {
                                vec_int_index_get(container, index).map(Value::int)
                            }
                        };

                        match outcome {
                            Ok(value) => write_register!(registers, destination, value),
                            Err(fault) => {
                                fail!(
                                    self,
                                    ip,
                                    floor,
                                    'dispatch,
                                    self.array_fault(fault)
                                );
                            }
                        }
                    }
                    Instruction::DictIndexGetIntKey {
                        destination,
                        container,
                        index,
                        value_mode,
                    } => {
                        let container =
                            // SAFETY: verified bytecode keeps operands in the live frame and proves their types.
                            unsafe { &*registers.add(container.index() as usize) };
                        // SAFETY: verified bytecode keeps operands in the live frame and proves their types.
                        let index = unsafe { int_register(registers, index) };

                        let outcome = match value_mode {
                            ArrayValueMode::Generic | ArrayValueMode::Float => {
                                dict_index_get_int_key(container, index)
                            }
                            ArrayValueMode::Int => {
                                dict_index_get_int_key_int_value(container, index).map(Value::int)
                            }
                        };

                        match outcome {
                            Ok(value) => write_register!(registers, destination, value),
                            Err(fault) => {
                                fail!(
                                    self,
                                    ip,
                                    floor,
                                    'dispatch,
                                    self.array_fault(fault)
                                );
                            }
                        }
                    }
                    Instruction::DictIndexGetStringKey {
                        destination,
                        container,
                        index,
                        value_mode,
                    } => {
                        let outcome = {
                            let container =
                                // SAFETY: verified bytecode keeps operands in the live frame and proves their types.
                                unsafe { &*registers.add(container.index() as usize) };
                            // SAFETY: verified bytecode keeps operands in the live frame and proves their types.
                            let index = unsafe { &*registers.add(index.index() as usize) };
                            dict_index_get_string_key(&self.heap, container, index, value_mode)
                        };

                        match outcome {
                            Ok(value) => write_register!(registers, destination, value),
                            Err(fault) => {
                                fail!(self, ip, floor, 'dispatch, self.array_fault(fault));
                            }
                        }
                    }
                    Instruction::IndexSet {
                        container,
                        index,
                        value,
                    } => {
                        let index_value = read_register!(registers, index);
                        let new_value = read_register!(registers, value);
                        let outcome = {
                            // SAFETY: verified bytecode keeps operands in the live frame and proves their types.
                            let slot = unsafe { &mut *registers.add(container.index() as usize) };
                            index_set(slot, &index_value, new_value)
                        };

                        if let Err(fault) = outcome {
                            fail!(self, ip, floor, 'dispatch, self.array_fault(fault));
                        }
                    }
                    Instruction::IndexAddAssign {
                        container,
                        index,
                        value,
                        mode,
                    } => {
                        if mode != IndexAddMode::Generic {
                            // SAFETY: verified bytecode keeps operands in the live frame and proves their types.
                            let index = unsafe { &*registers.add(index.index() as usize) };
                            // SAFETY: verified bytecode keeps operands in the live frame and proves their types.
                            let increment = unsafe { int_register(registers, value) };
                            let container =
                                // SAFETY: verified bytecode keeps operands in the live frame and proves their types.
                                unsafe { &mut *registers.add(container.index() as usize) };
                            let outcome = match mode {
                                IndexAddMode::DictAnyKeyIntValue => {
                                    dict_add_assign_any_key_int_value(
                                        &self.heap,
                                        container,
                                        index,
                                        increment,
                                    )
                                }
                                IndexAddMode::DictStringKeyIntValue => dict_add_assign_string_key_int_value(
                                    &self.heap,
                                    container,
                                    index,
                                    increment,
                                ),
                                // SAFETY: the surrounding invariant makes this path unreachable.
                                IndexAddMode::Generic => unsafe {
                                    unreachable_invariant(
                                        "the generic indexed add takes its generic path",
                                    )
                                },
                            };
                            match outcome {
                                Ok(()) => {}
                                Err(IndexAddFault::Array(fault)) => {
                                    fail!(
                                        self,
                                        ip,
                                        floor,
                                        'dispatch,
                                        self.array_fault(fault)
                                    );
                                }
                                Err(IndexAddFault::Arithmetic { fault, .. }) => {
                                    fail!(
                                        self,
                                        ip,
                                        floor,
                                        'dispatch,
                                        self.binary_fault(fault, "+", "int", "int")
                                    );
                                }
                            }

                            continue 'instructions;
                        }

                        let index = read_register!(registers, index);
                        let increment = read_register!(registers, value);
                        let container =
                            // SAFETY: verified bytecode keeps operands in the live frame and proves their types.
                            unsafe { &mut *registers.add(container.index() as usize) };
                        match index_add_assign(&self.heap, container, &index, &increment) {
                            Ok(()) => {}
                            Err(IndexAddFault::Array(fault)) => {
                                fail!(
                                    self,
                                    ip,
                                    floor,
                                    'dispatch,
                                    self.array_fault(fault)
                                );
                            }
                            Err(IndexAddFault::Arithmetic {
                                fault,
                                left_kind,
                                right_kind,
                            }) => {
                                fail!(
                                    self,
                                    ip,
                                    floor,
                                    'dispatch,
                                    self.binary_fault(fault, "+", left_kind, right_kind)
                                );
                            }
                        }
                    }
                    Instruction::VecIndexSet {
                        container,
                        index,
                        value,
                    } => {
                        // SAFETY: verified bytecode keeps operands in the live frame and proves their types.
                        let index = unsafe { int_register(registers, index) };
                        let value = read_register!(registers, value);
                        let outcome = {
                            let container =
                                // SAFETY: verified bytecode keeps operands in the live frame and proves their types.
                                unsafe { &mut *registers.add(container.index() as usize) };
                            vec_index_set(container, index, value)
                        };

                        if let Err(fault) = outcome {
                            fail!(self, ip, floor, 'dispatch, self.array_fault(fault));
                        }
                    }
                    Instruction::DictIndexSetIntKey {
                        container,
                        index,
                        value,
                    } => {
                        // SAFETY: verified bytecode keeps operands in the live frame and proves their types.
                        let index = unsafe { int_register(registers, index) };
                        let value = read_register!(registers, value);
                        // SAFETY: verified bytecode keeps operands in the live frame and proves their types.
                        let container = unsafe { &mut *registers.add(container.index() as usize) };
                        dict_index_set_int_key(container, index, value);
                    }
                    Instruction::DictIndexSetStringKey {
                        container,
                        index,
                        value,
                    } => {
                        let index = read_register!(registers, index);
                        let value = read_register!(registers, value);
                        // SAFETY: verified bytecode keeps operands in the live frame and proves their types.
                        let container = unsafe { &mut *registers.add(container.index() as usize) };
                        dict_index_set_string_key(container, index, value);
                    }
                    Instruction::DictIndexSet {
                        container,
                        index,
                        value,
                    } => {
                        let index = read_register!(registers, index);
                        let value = read_register!(registers, value);
                        // SAFETY: verified bytecode keeps operands in the live frame and proves their types.
                        let container = unsafe { &mut *registers.add(container.index() as usize) };
                        if let Err(fault) = dict_index_set(container, index, value) {
                            fail!(self, ip, floor, 'dispatch, self.array_fault(fault));
                        }
                    }
                    Instruction::Append { container, value } => {
                        let new_value = read_register!(registers, value);
                        let outcome = {
                            // SAFETY: verified bytecode keeps operands in the live frame and proves their types.
                            let slot = unsafe { &mut *registers.add(container.index() as usize) };
                            append_value(slot, new_value)
                        };

                        if let Err(fault) = outcome {
                            fail!(self, ip, floor, 'dispatch, self.array_fault(fault));
                        }
                    }
                    Instruction::VecAppend { container, value } => {
                        let value = read_register!(registers, value);
                        // SAFETY: verified bytecode keeps operands in the live frame and proves their types.
                        let container = unsafe { &mut *registers.add(container.index() as usize) };
                        vec_append(container, value);
                    }
                    Instruction::ReserveArray {
                        container,
                        additional,
                    } => {
                        const MAXIMUM_RESERVATION: i64 = 1 << 24;

                        // SAFETY: verified bytecode keeps operands in the live frame and proves their types.
                        let additional = unsafe { int_register(registers, additional) };
                        if additional > 0 {
                            let additional = additional.min(MAXIMUM_RESERVATION) as usize;
                            let container =
                                // SAFETY: verified bytecode keeps operands in the live frame and proves their types.
                                unsafe { &mut *registers.add(container.index() as usize) };
                            reserve_array_hint(container, additional);
                        }
                    }
                    Instruction::Spread { container, value } => {
                        let source = read_register!(registers, value);
                        let outcome = {
                            // SAFETY: verified bytecode keeps operands in the live frame and proves their types.
                            let slot = unsafe { &mut *registers.add(container.index() as usize) };
                            spread_into(slot, &source)
                        };

                        if let Err(fault) = outcome {
                            fail!(self, ip, floor, 'dispatch, self.array_fault(fault));
                        }
                    }
                    Instruction::Length {
                        destination,
                        source,
                    } => {
                        let outcome = {
                            // SAFETY: verified bytecode keeps operands in the live frame and proves their types.
                            let value = unsafe { &*registers.add(source.index() as usize) };
                            array_length(value)
                        };

                        match outcome {
                            Ok(length) => {
                                write_register!(registers, destination, Value::int(length))
                            }
                            Err(fault) => {
                                fail!(self, ip, floor, 'dispatch, self.array_fault(fault));
                            }
                        }
                    }
                    Instruction::Contains {
                        destination,
                        array,
                        value,
                    } => {
                        let outcome = {
                            // SAFETY: verified bytecode keeps operands in the live frame and proves their types.
                            let array = unsafe { &*registers.add(array.index() as usize) };
                            // SAFETY: verified bytecode keeps operands in the live frame and proves their types.
                            let value = unsafe { &*registers.add(value.index() as usize) };
                            array_contains(array, value)
                        };
                        match outcome {
                            Ok(found) => {
                                write_register!(registers, destination, Value::bool(found));
                            }
                            Err(fault) => {
                                fail!(self, ip, floor, 'dispatch, self.array_fault(fault));
                            }
                        }
                    }
                    Instruction::ContainsKey {
                        destination,
                        array,
                        key,
                    } => {
                        let outcome = {
                            // SAFETY: verified bytecode keeps operands in the live frame and proves their types.
                            let array = unsafe { &*registers.add(array.index() as usize) };
                            // SAFETY: verified bytecode keeps operands in the live frame and proves their types.
                            let key = unsafe { &*registers.add(key.index() as usize) };
                            array_contains_key(array, key)
                        };
                        match outcome {
                            Ok(found) => {
                                write_register!(registers, destination, Value::bool(found));
                            }
                            Err(fault) => {
                                fail!(self, ip, floor, 'dispatch, self.array_fault(fault));
                            }
                        }
                    }
                    Instruction::StringLength {
                        destination,
                        source,
                    } => {
                        // SAFETY: verified bytecode keeps the source in the active frame.
                        let value = unsafe { &*registers.add(source.index() as usize) };
                        // SAFETY: the value's tag proves this projection is valid.
                        let string = unsafe { value.as_string_bytes().unwrap_unchecked() };
                        write_register!(
                            registers,
                            destination,
                            Value::int(string.len() as i64)
                        );
                    }
                    Instruction::Remove {
                        destination,
                        container,
                        key,
                    } => {
                        let key_value = read_register!(registers, key);
                        let outcome = {
                            // SAFETY: verified bytecode keeps operands in the live frame and proves their types.
                            let slot = unsafe { &mut *registers.add(container.index() as usize) };
                            remove_entry(&self.heap, slot, &key_value)
                        };

                        match outcome {
                            Ok(removed) => write_register!(registers, destination, removed),
                            Err(fault) => {
                                fail!(self, ip, floor, 'dispatch, self.array_fault(fault));
                            }
                        }
                    }
                    Instruction::RemoveFirst {
                        destination,
                        container,
                    } => {
                        let outcome = {
                            // SAFETY: verified bytecode keeps operands in the live frame and proves their types.
                            let slot = unsafe { &mut *registers.add(container.index() as usize) };
                            remove_end(slot, true)
                        };

                        match outcome {
                            Ok(removed) => write_register!(registers, destination, removed),
                            Err(fault) => {
                                fail!(self, ip, floor, 'dispatch, self.array_fault(fault));
                            }
                        }
                    }
                    Instruction::RemoveLast {
                        destination,
                        container,
                    } => {
                        let outcome = {
                            // SAFETY: verified bytecode keeps operands in the live frame and proves their types.
                            let slot = unsafe { &mut *registers.add(container.index() as usize) };
                            remove_end(slot, false)
                        };

                        match outcome {
                            Ok(removed) => write_register!(registers, destination, removed),
                            Err(fault) => {
                                fail!(self, ip, floor, 'dispatch, self.array_fault(fault));
                            }
                        }
                    }
                    Instruction::SwapRemove {
                        destination,
                        container,
                        index,
                    } => {
                        let index_value = read_register!(registers, index);
                        let outcome = {
                            // SAFETY: verified bytecode keeps operands in the live frame and proves their types.
                            let slot = unsafe { &mut *registers.add(container.index() as usize) };
                            swap_remove_entry(slot, &index_value)
                        };

                        match outcome {
                            Ok(removed) => write_register!(registers, destination, removed),
                            Err(fault) => {
                                fail!(self, ip, floor, 'dispatch, self.array_fault(fault));
                            }
                        }
                    }
                    Instruction::CheckDestructure {
                        subject,
                        required,
                        arity,
                        rest,
                    } => {
                        let minimum = required.value() as usize;
                        let maximum = arity.value() as usize;
                        let found = {
                            // SAFETY: verified bytecode keeps operands in the live frame and proves their types.
                            let value = unsafe { &*registers.add(subject.index() as usize) };
                            let value = value.transparent();
                            match value {
                                ValueView::Tuple(tuple) => Ok(("a tuple", tuple.len())),
                                ValueView::Vec(vec) => Ok(("a vec", vec.len())),
                                other => Err(other.kind_name()),
                            }
                        };

                        let satisfied = match found {
                            Ok((_, length)) if minimum == maximum && rest => length >= minimum,
                            Ok((_, length)) if minimum == maximum => length == minimum,
                            Ok((_, length)) if rest => length >= minimum,
                            Ok((_, length)) => length >= minimum && length <= maximum,
                            Err(_) => false,
                        };

                        if !satisfied {
                            let expected = if rest {
                                format!("at least {minimum}")
                            } else if minimum == maximum {
                                minimum.to_string()
                            } else {
                                format!("between {minimum} and {maximum}")
                            };

                            let plural = if maximum == 1 { "element" } else { "elements" };
                            let given = match found {
                                Ok((kind, length)) => format!("{kind} of {length}"),
                                Err(kind) => kind.to_string(),
                            };

                            fail!(
                                self,
                                ip,
                                floor,
                                'dispatch,
                                self.throw_well_known(
                                    self.engine.tables.well_known.type_error,
                                    format!(
                                        "destructuring requires a vec or tuple of {expected} {plural}, {given} given"
                                    ),
                                )
                            );
                        }
                    }
                    Instruction::ElementGet {
                        destination,
                        subject,
                        index,
                    } => {
                        let value = {
                            // SAFETY: verified bytecode keeps operands in the live frame and proves their types.
                            let container = unsafe { &*registers.add(subject.index() as usize) };
                            let container = container.transparent();
                            let element = match container {
                                ValueView::Tuple(elements) => elements.get(index.value() as usize),
                                ValueView::Vec(elements) => elements.get(index.value() as usize),
                                // SAFETY: the surrounding invariant makes this path unreachable.
                                _ => unsafe {
                                    unreachable_invariant(
                                        "every element get has a proven vec or tuple subject",
                                    )
                                },
                            };

                            match element {
                                Some(value) => value.clone(),
                                // SAFETY: the surrounding invariant makes this path unreachable.
                                None => unsafe {
                                    unreachable_invariant(
                                        "every element get has a proven in-range index",
                                    )
                                },
                            }
                        };

                        write_register!(registers, destination, value);
                    }
                    Instruction::Rest {
                        destination,
                        subject,
                        from,
                    } => {
                        // SAFETY: verified bytecode keeps operands in the live frame and proves their types.
                        let container = unsafe { &*registers.add(subject.index() as usize) };
                        let container = container.transparent();
                        let all: &[Value] = match container {
                            ValueView::Tuple(elements) => elements.as_slice(),
                            ValueView::Vec(elements) => elements.as_slice(),
                            // SAFETY: the surrounding invariant makes this path unreachable.
                            _ => unsafe {
                                unreachable_invariant("the destructuring guard types every rest")
                            },
                        };

                        let rest = all.get(from.value() as usize..).unwrap_or(&[]);
                        let vec = Value::vec(VecObject::with_elements(&self.heap, rest.iter().cloned()));
                        write_register!(registers, destination, vec);
                    }
                    Instruction::Is {
                        destination,
                        source,
                        descriptor,
                    } => {
                        let checked = &chunk.type_descriptors[descriptor.index() as usize];
                        // SAFETY: verified bytecode keeps operands in the live frame and proves their types.
                        let value = unsafe { &*registers.add(source.index() as usize) };
                        if let Some(matches) = check_trivial_descriptor(checked, value) {
                            write_register!(registers, destination, Value::bool(matches));
                            continue 'instructions;
                        }

                        self.sync_ip(ip);
                        let (frame_called, frame_environment) = {
                            let frame = self.current_frame();
                            (frame.called_class.get(), frame.type_environment)
                        };

                        let site = descriptor.index() as usize;
                        let cached_cacheability = {
                            // SAFETY: verified bytecode keeps operands in the live frame and proves their types.
                            let cache = unsafe {
                                &mut *self.current_frame().cache.as_ref().is_checks()
                            };
                            if cache.len() <= site {
                                cache.resize(
                                    chunk.type_descriptors.len().max(site + 1),
                                    IsCheckWays::EMPTY,
                                );
                            }
                            cache[site].cacheable(frame_environment, frame_called)
                        };
                        let cacheable = match cached_cacheability {
                            Some(cacheable) => cacheable,
                            None => {
                                let cacheable = self.is_check_shape_cacheable(
                                    checked,
                                    frame_environment,
                                    0,
                                );
                                // SAFETY: verified bytecode keeps operands in the live frame and proves their types.
                                let cache = unsafe {
                                    &mut *self.current_frame().cache.as_ref().is_checks()
                                };
                                cache[site].set_cacheable(
                                    frame_environment,
                                    frame_called,
                                    cacheable,
                                );
                                cacheable
                            }
                        };
                        let probe = cacheable
                            // SAFETY: verified bytecode keeps operands in the live frame and proves their types.
                            .then(|| unsafe { &*registers.add(source.index() as usize) }.as_object())
                            .flatten()
                            .map(|object| CachedIsCheck {
                                caller_environment: frame_environment,
                                called_class: frame_called,
                                class: object.class(),
                                environment: object.type_environment(),
                            });

                        if let Some(probe) = &probe {
                            // SAFETY: verified bytecode keeps operands in the live frame and proves their types.
                            let proved = unsafe { &*self.current_frame().cache.as_ref().is_checks() };
                            if proved.get(site).is_some_and(|ways| ways.holds(probe)) {
                                let target = self.current_base() + destination.index() as usize;
                                self.store_result(target, Value::bool(true));
                                continue 'dispatch;
                            }
                        }

                        let value = read_register!(registers, source);
                        match self.check_descriptor(
                            checked,
                            &value,
                            frame_called,
                            frame_environment,
                            0,
                        ) {
                            Ok(matches) => {
                                if matches
                                    && let Some(probe) = probe
                                {
                                    // SAFETY: verified bytecode keeps operands in the live frame and proves their types.
                                    let cache = unsafe {
                                        &mut *self.current_frame().cache.as_ref().is_checks()
                                    };
                                    cache[site].record(probe);
                                }

                                let target = self.current_base() + destination.index() as usize;
                                self.store_result(target, Value::bool(matches));
                            }
                            Err(control) => self.handle_control(control, floor)?,
                        }

                        continue 'dispatch;
                    }
                    Instruction::AsCheck {
                        destination,
                        source,
                        descriptor,
                        mode,
                    } => {
                        self.sync_ip(ip);
                        let (frame_called, frame_environment) = {
                            let frame = self.current_frame();
                            (frame.called_class.get(), frame.type_environment)
                        };

                        let value = read_register!(registers, source);
                        let checked = &chunk.type_descriptors[descriptor.index() as usize];
                        let outcome = match mode {
                            AsMode::Boundary => match self.check_descriptor(
                                checked,
                                &value,
                                frame_called,
                                frame_environment,
                                0,
                            ) {
                                Ok(true) => Ok(value),
                                Ok(false) => {
                                    let expected = self.render_descriptor(checked);
                                    let found = self.value_type_name(&value);
                                    Err(self.throw_well_known(
                                        self.engine.tables.well_known.type_error,
                                        format!("expected {expected}, {found} given"),
                                    ))
                                }
                                Err(control) => Err(control),
                            },
                            AsMode::Cast => match self.cast_value(
                                checked,
                                &value,
                                frame_called,
                                frame_environment,
                            ) {
                                Ok(Some(value)) => Ok(value),
                                Ok(None) => {
                                    let expected = self.render_descriptor(checked);
                                    let found = self.value_type_name(&value);
                                    Err(self.throw_well_known(
                                        self.engine.tables.well_known.type_error,
                                        format!("expected {expected}, {found} given"),
                                    ))
                                }
                                Err(control) => Err(control),
                            },
                        };

                        match outcome {
                            Ok(value) => {
                                let target = self.current_base() + destination.index() as usize;
                                self.store_result(target, value);
                            }
                            Err(control) => self.handle_control(control, floor)?,
                        }

                        continue 'dispatch;
                    }
                    Instruction::AsOrNull {
                        destination,
                        source,
                        descriptor,
                    } => {
                        self.sync_ip(ip);
                        let (frame_called, frame_environment) = {
                            let frame = self.current_frame();
                            (frame.called_class.get(), frame.type_environment)
                        };

                        let value = read_register!(registers, source);
                        let checked = &chunk.type_descriptors[descriptor.index() as usize];
                        match self.cast_value(
                            checked,
                            &value,
                            frame_called,
                            frame_environment,
                        ) {
                            Ok(casted) => {
                                let target = self.current_base() + destination.index() as usize;
                                self.store_result(target, casted.unwrap_or(Value::null()));
                            }
                            Err(control) => self.handle_control(control, floor)?,
                        }

                        continue 'dispatch;
                    }
                    Instruction::ForeachInit {
                        iterator,
                        subject,
                        reserve,
                    } => {
                        if reserve != Register::NONE {
                            // SAFETY: verified bytecode keeps operands in the live frame and proves their types.
                            let additional = unsafe {
                                array_length_hint(&*registers.add(subject.index() as usize))
                            };
                            if let Some(additional) = additional {
                                // SAFETY: verified bytecode keeps operands in the live frame and proves their types.
                                let target = unsafe {
                                    &mut *registers.add(reserve.index() as usize)
                                };
                                reserve_array_hint(target, additional.min(1 << 24));
                            }
                        }

                        let state = {
                            // SAFETY: verified bytecode keeps operands in the live frame and proves their types.
                            let value = unsafe { &*registers.add(subject.index() as usize) };
                            let value = value.transparent();
                            match value {
                                ValueView::Vec(vec) => Ok(Value::vec_cursor(vec.clone())),
                                ValueView::Dict(dict) => Ok(Value::dict_cursor(dict.clone())),
                                ValueView::Tuple(tuple) => Ok(Value::tuple_cursor(tuple.clone())),
                                ValueView::Object(instance) => Err(Ok(instance.clone())),
                                other => Err(Err(format!(
                                    "foreach requires a vec, dict, tuple, Iterator, or ToIterator object, {} given",
                                    other.kind_name()
                                ))),
                            }
                        };

                        match state {
                            Ok(cursor) => write_register!(registers, iterator, cursor),
                            Err(Ok(instance)) => {
                                self.sync_ip(ip);
                                match self.object_cursor(instance) {
                                    Ok(cursor) => {
                                        let target = self.current_base() + iterator.index() as usize;
                                        self.store_result(target, cursor);
                                    }
                                    Err(control) => self.handle_control(control, floor)?,
                                }

                                continue 'dispatch;
                            }
                            Err(Err(message)) => {
                                fail!(
                                    self,
                                    ip,
                                    floor,
                                    'dispatch,
                                    self.throw_well_known(
                                        self.engine.tables.well_known.type_error,
                                        message,
                                    )
                                );
                            }
                        }
                    }
                    Instruction::ForeachNext {
                        iterator,
                        key_destination,
                        value_destination,
                    } => {
                        let advanced = {
                            // SAFETY: verified bytecode keeps operands in the live frame and proves their types.
                            let cursor = unsafe { &mut *registers.add(iterator.index() as usize) };
                            match cursor.as_iterator() {
                                Some(cursor) if cursor.take_pending_object_step() => {
                                    // SAFETY: verified bytecode keeps operands in the live frame and proves their types.
                                    let returned = unsafe {
                                        mem::replace(
                                            &mut *registers
                                                .add(value_destination.index() as usize),
                                            Value::uninitialized(),
                                        )
                                    };
                                    ForeachAdvance::Returned(returned)
                                }
                                Some(cursor) => ForeachAdvance::Object {
                                    instance: cursor.instance().raw_box(),
                                    next: cursor.next_method(),
                                    environment: cursor.next_environment(),
                                },
                                None => ForeachAdvance::Array(advance_cursor(cursor)),
                            }
                        };

                        let advanced = match advanced {
                            ForeachAdvance::Array(advanced) => advanced,
                            ForeachAdvance::Returned(returned) => {
                                match self.decode_object_cursor_result(&returned) {
                                    Ok(advanced) => advanced,
                                    Err(control) => {
                                        self.sync_ip(ip - 1);
                                        self.handle_control(control, floor)?;
                                        continue 'dispatch;
                                    }
                                }
                            }
                            ForeachAdvance::Object {
                                instance,
                                next: Some((function, scope)),
                                environment,
                            } => {
                                let cursor_index =
                                    self.current_base() + iterator.index() as usize;
                                self.sync_ip(ip - 1);
                                match self.push_object_iterator_frame(
                                    function,
                                    instance,
                                    scope,
                                    environment,
                                    value_destination.index(),
                                ) {
                                    Ok(()) => {
                                        let cursor = self.stack[cursor_index]
                                            .as_iterator()
                                            // SAFETY: the surrounding invariant makes this path unreachable.
                                            .unwrap_or_else(|| unsafe {
                                                unreachable_invariant(
                                                    "the iterator register holds the loop cursor",
                                                )
                                            });
                                        cursor.begin_object_step();
                                    }
                                    Err(control) => self.handle_control(control, floor)?,
                                }

                                continue 'dispatch;
                            }
                            ForeachAdvance::Object {
                                instance,
                                next: None,
                                ..
                            } => {
                                self.sync_ip(ip - 1);
                                // SAFETY: verified bytecode keeps operands in the live frame and proves their types.
                                let exit_offset = unsafe { fused_foreach_exit_offset(code, ip) };

                                // SAFETY: verified bytecode keeps operands in the live frame and proves their types.
                                let instance = mem::ManuallyDrop::new(unsafe {
                                    ManagedRef::from_raw(instance)
                                });
                                match self.advance_built_in_object_cursor(&instance) {
                                    Ok(Some((key, value))) => {
                                        if key_destination != Register::NONE {
                                            let key_target = self.current_base()
                                                + key_destination.index() as usize;
                                            self.stack[key_target] = key;
                                        }
                                        let value_target = self.current_base()
                                            + value_destination.index() as usize;
                                        self.stack[value_target] = value;
                                        self.sync_ip(ip + 1);
                                    }
                                    Ok(None) => {
                                        self.sync_ip(jump_target(ip + 1, exit_offset));
                                    }
                                    Err(control) => self.handle_control(control, floor)?,
                                }

                                continue 'dispatch;
                            }
                        };

                        match advanced {
                            Some((key, value)) => {
                                if key_destination != Register::NONE {
                                    write_register!(registers, key_destination, key);
                                }

                                write_register!(registers, value_destination, value);
                                ip += 1;
                            }
                            None => {
                                // SAFETY: verified bytecode keeps operands in the live frame and proves their types.
                                let exit_offset = unsafe { fused_foreach_exit_offset(code, ip) };
                                ip = jump_target(ip + 1, exit_offset);
                            }
                        }
                    }
                    Instruction::VecForeachNext {
                        iterator,
                        key_destination,
                        value_destination,
                        value_mode,
                    } => {
                        if value_mode == ArrayValueMode::Int {
                            let advanced = {
                                let cursor =
                                    // SAFETY: verified bytecode keeps operands in the live frame and proves their types.
                                    unsafe { &mut *registers.add(iterator.index() as usize) };
                                advance_vec_int_cursor(cursor)
                            };

                            match advanced {
                                Some((key, value)) => {
                                    if key_destination != Register::NONE {
                                        write_register!(
                                            registers,
                                            key_destination,
                                            Value::int(key)
                                        );
                                    }

                                    write_register!(
                                        registers,
                                        value_destination,
                                        Value::int(value)
                                    );
                                    ip += 1;
                                }
                                None => {
                                    let exit_offset =
                                        // SAFETY: verified bytecode keeps operands in the live frame and proves their types.
                                        unsafe { fused_foreach_exit_offset(code, ip) };
                                    ip = jump_target(ip + 1, exit_offset);
                                }
                            }
                        } else {
                            let advanced = {
                                let cursor =
                                    // SAFETY: verified bytecode keeps operands in the live frame and proves their types.
                                    unsafe { &mut *registers.add(iterator.index() as usize) };
                                advance_vec_cursor(cursor)
                            };

                            match advanced {
                                Some((key, value)) => {
                                    if key_destination != Register::NONE {
                                        write_register!(registers, key_destination, key);
                                    }

                                    write_register!(registers, value_destination, value);
                                    ip += 1;
                                }
                                None => {
                                    let exit_offset =
                                        // SAFETY: verified bytecode keeps operands in the live frame and proves their types.
                                        unsafe { fused_foreach_exit_offset(code, ip) };
                                    ip = jump_target(ip + 1, exit_offset);
                                }
                            }
                        }
                    }
                    Instruction::DictForeachNext {
                        iterator,
                        key_destination,
                        value_destination,
                        value_mode,
                    } => {
                        if value_mode == ArrayValueMode::Int {
                            let advanced = {
                                let cursor =
                                    // SAFETY: verified bytecode keeps operands in the live frame and proves their types.
                                    unsafe { &mut *registers.add(iterator.index() as usize) };
                                advance_dict_cursor_int_values(
                                    cursor,
                                    key_destination != Register::NONE,
                                )
                            };

                            match advanced {
                                Some((key, value)) => {
                                    if let Some(key) = key {
                                        write_register!(registers, key_destination, key);
                                    }

                                    write_register!(
                                        registers,
                                        value_destination,
                                        Value::int(value)
                                    );
                                    ip += 1;
                                }
                                None => {
                                    let exit_offset =
                                        // SAFETY: verified bytecode keeps operands in the live frame and proves their types.
                                        unsafe { fused_foreach_exit_offset(code, ip) };
                                    ip = jump_target(ip + 1, exit_offset);
                                }
                            }
                        } else {
                            let advanced = {
                                let cursor =
                                    // SAFETY: verified bytecode keeps operands in the live frame and proves their types.
                                    unsafe { &mut *registers.add(iterator.index() as usize) };
                                advance_dict_cursor(cursor)
                            };

                            match advanced {
                                Some((key, value)) => {
                                    if key_destination != Register::NONE {
                                        write_register!(registers, key_destination, key);
                                    }

                                    write_register!(registers, value_destination, value);
                                    ip += 1;
                                }
                                None => {
                                    let exit_offset =
                                        // SAFETY: verified bytecode keeps operands in the live frame and proves their types.
                                        unsafe { fused_foreach_exit_offset(code, ip) };
                                    ip = jump_target(ip + 1, exit_offset);
                                }
                            }
                        }
                    }
                    Instruction::SwitchInt { subject, table } => {
                        let relative = {
                            // SAFETY: verified bytecode keeps operands in the live frame and proves their types.
                            let value = unsafe { &*registers.add(subject.index() as usize) };
                            switch_int_target(&chunk.switch_tables[table.index() as usize], value)
                        };

                        ip = jump_target(ip, relative);
                    }
                    Instruction::SwitchString { subject, table } => {
                        let relative = {
                            // SAFETY: verified bytecode keeps operands in the live frame and proves their types.
                            let value = unsafe { &*registers.add(subject.index() as usize) };
                            switch_string_target(
                                &chunk.switch_tables[table.index() as usize],
                                value,
                            )
                        };

                        ip = jump_target(ip, relative);
                    }
                    Instruction::SwitchBool { subject, table } => {
                        let relative = {
                            // SAFETY: verified bytecode keeps operands in the live frame and proves their types.
                            let value = unsafe { &*registers.add(subject.index() as usize) };
                            let SwitchTable::Bool { targets, default } =
                                &chunk.switch_tables[table.index() as usize]
                            else {
                                // SAFETY: the surrounding invariant makes this path unreachable.
                                unsafe {
                                    unreachable_invariant(
                                        "a SwitchBool site references a bool table",
                                    )
                                }
                            };
                            match value.as_bool() {
                                // SAFETY: the surrounding invariant keeps this index in bounds.
                                Some(value) => unsafe {
                                    *targets.get_unchecked(usize::from(value))
                                },
                                None => *default,
                            }
                        };

                        if relative != 1 {
                            ip = jump_target(ip, relative);
                        }
                    }
                    Instruction::BoolPatternBranch {
                        subject,
                        false_offset,
                        default_offset,
                    } => {
                        // SAFETY: verified bytecode keeps operands in the live frame and proves their types.
                        let value = unsafe { &*registers.add(subject.index() as usize) };
                        let relative = match value.as_bool() {
                            Some(true) => None,
                            Some(false) => Some(i32::from(false_offset.offset())),
                            None => Some(i32::from(default_offset.offset())),
                        };
                        if let Some(relative) = relative {
                            ip = jump_target(ip, relative);
                        }
                    }
                    Instruction::SwitchFloat { subject, table } => {
                        let relative = {
                            // SAFETY: verified bytecode keeps operands in the live frame and proves their types.
                            let value = unsafe { &*registers.add(subject.index() as usize) };
                            switch_float_target(
                                &chunk.switch_tables[table.index() as usize],
                                value,
                            )
                        };

                        ip = jump_target(ip, relative);
                    }
                    Instruction::IntRangeJumpIf {
                        subject,
                        descriptor,
                        offset,
                    } => {
                        // SAFETY: verified bytecode keeps operands in the live frame and proves their types.
                        let value = unsafe { &*registers.add(subject.index() as usize) };
                        let TypeDescriptor::IntRange { min, max } =
                            &chunk.type_descriptors[descriptor.index() as usize]
                        else {
                            // SAFETY: the surrounding invariant makes this path unreachable.
                            unsafe {
                                unreachable_invariant(
                                    "an integer range branch references an integer range",
                                )
                            }
                        };
                        let matches = value.as_int().is_some_and(|value| {
                            min.is_none_or(|min| value >= min)
                                && max.is_none_or(|max| value <= max)
                        });
                        if matches {
                            let relative = i32::from(offset.offset());
                            ip = jump_target(ip, relative);
                        }
                    }
                    Instruction::IntRangeJumpUnless {
                        subject,
                        descriptor,
                        offset,
                    } => {
                        // SAFETY: verified bytecode keeps operands in the live frame and proves their types.
                        let value = unsafe { &*registers.add(subject.index() as usize) };
                        let TypeDescriptor::IntRange { min, max } =
                            &chunk.type_descriptors[descriptor.index() as usize]
                        else {
                            // SAFETY: the surrounding invariant makes this path unreachable.
                            unsafe {
                                unreachable_invariant(
                                    "an integer range branch references an integer range",
                                )
                            }
                        };
                        let matches = value.as_int().is_some_and(|value| {
                            min.is_none_or(|min| value >= min)
                                && max.is_none_or(|max| value <= max)
                        });
                        if !matches {
                            let relative = i32::from(offset.offset());
                            ip = jump_target(ip, relative);
                        }
                    }
                    Instruction::SwitchPattern { subject, table } => {
                        let relative = {
                            // SAFETY: verified bytecode keeps operands in the live frame and proves their types.
                            let value = unsafe { &*registers.add(subject.index() as usize) };
                            switch_pattern_target(
                                &chunk.switch_tables[table.index() as usize],
                                value,
                            )
                        };

                        ip = jump_target(ip, relative);
                    }
                    Instruction::SwitchTuplePattern {
                        first_element,
                        element_count,
                        table,
                    } => {
                        let relative = {
                            // SAFETY: the pointer and length share one live allocation.
                            let elements = unsafe {
                                slice::from_raw_parts(
                                    registers.add(first_element.index() as usize),
                                    element_count.value() as usize,
                                )
                            };
                            switch_tuple_pattern_target(
                                &chunk.switch_tables[table.index() as usize],
                                elements,
                            )
                        };

                        ip = jump_target(ip, relative);
                    }
                    Instruction::ThrowUnhandledMatch { subject } => {
                        let rendered = {
                            // SAFETY: verified bytecode keeps operands in the live frame and proves their types.
                            let value = unsafe { &*registers.add(subject.index() as usize) };
                            debug_render(&self.heap, value, 0)
                        };

                        fail!(
                            self,
                            ip,
                            floor,
                            'dispatch,
                            self.throw_well_known(
                                self.engine.tables.well_known.unhandled_match_error,
                                format!("unhandled match subject: {rendered}"),
                            )
                        );
                    }
                    Instruction::Debug {
                        value_count,
                        first_value,
                    } => {
                        let location = self.debug_location(ip);
                        let start = self.current_base() + first_value.index() as usize;
                        let mut values = Vec::with_capacity(usize::from(value_count.value()));
                        for position in 0..usize::from(value_count.value()) {
                            values.push(self.debug_render_value(&self.stack[start + position]));
                        }

                        let separator = if values.iter().any(|value| value.contains('\n')) {
                            ",\n"
                        } else {
                            ", "
                        };
                        let mut rendered = location;
                        rendered.push_str(&values.join(separator));
                        rendered.push('\n');
                        let _ = Engine::write_standard_stream(
                            StandardStream::Error,
                            rendered.as_bytes(),
                        );
                    }
                    Instruction::NewDynamic {
                        destination,
                        class_name,
                    } => {
                        self.sync_ip(ip);
                        let descriptor = {
                            // SAFETY: verified bytecode keeps operands in the live frame and proves their types.
                            let value = unsafe { &*registers.add(class_name.index() as usize) };
                            let value = value.transparent();
                            match value {
                                ValueView::String(_) | ValueView::ShortString(_) => self
                                    // SAFETY: the value's tag proves this projection is valid.
                                    .parse_runtime_type_name(unsafe {
                                        value.as_string_bytes().unwrap_unchecked()
                                    })
                                    .ok_or("an invalid type name"),
                                other => Err(other.kind_name()),
                            }
                        };

                        let outcome = match descriptor {
                            Ok(TypeDescriptor::Named {
                                name, arguments, ..
                            }) => {
                                self.resolve_class_reference(name).and_then(|class| {
                                    self.new_instance_typed(
                                        class,
                                        arguments.as_deref(),
                                        TypeEnvironmentId::default(),
                                    )
                                })
                            }
                            Ok(other) => Err(self.throw_well_known(
                                self.engine.tables.well_known.type_error,
                                format!(
                                    "new requires a class name, {} given",
                                    self.render_descriptor(&other)
                                ),
                            )),
                            Err(kind) => Err(self.throw_well_known(
                                self.engine.tables.well_known.type_error,
                                format!("new requires a class name string, {kind} given"),
                            )),
                        };

                        match outcome {
                            Ok(value) => {
                                let target = self.current_base() + destination.index() as usize;
                                self.store_result(target, value);
                            }
                            Err(control) => self.handle_control(control, floor)?,
                        }
                        enter_finalizer_dispatch!(self, ip, floor);
                        continue 'dispatch;
                    }
                    Instruction::Require {
                        once,
                        destination,
                        path,
                    } => {
                        self.sync_ip(ip);
                        let path_value = read_register!(registers, path);
                        if let Err(control) =
                            self.require_from_frame(path_value, once, destination.index())
                        {
                            self.handle_control(control, floor)?;
                        }

                        continue 'dispatch;
                    }
                });
            }
        }
    }

    /// Moves a return register out of the active frame. Register zero may be
    /// a receiver borrowed from the suspended caller; returning that receiver
    /// creates a new owner, so retain it before the borrowed frame disappears.
    #[inline(always)]
    unsafe fn take_return_register(&self, registers: *mut Value, source: Register) -> Value {
        // SAFETY: verified bytecode keeps operands in the live frame and proves their types.
        let result = unsafe {
            mem::replace(
                &mut *registers.add(source.index() as usize),
                Value::uninitialized(),
            )
        };

        if source.index() == 0 && self.current_frame().borrows_register_zero() {
            let owned = result.clone();
            mem::forget(result);
            owned
        } else {
            result
        }
    }

    #[inline(always)]
    fn return_iterator_pair(&mut self, key: Value, value: Value) {
        let finished = self.pop_frame();
        self.truncate_frame_stack(&finished);

        let (base, instruction_index, chunk) = {
            let caller = self.current_frame();
            // SAFETY: verified bytecode keeps operands in the live frame and proves their types.
            (caller.base as usize, caller.ip as usize, unsafe {
                caller.chunk.as_ref()
            })
        };
        let Instruction::ForeachNext {
            iterator,
            key_destination,
            value_destination,
            // SAFETY: the surrounding invariant keeps this index in bounds.
        } = (unsafe { *chunk.code.get_unchecked(instruction_index) })
        else {
            // SAFETY: the surrounding invariant makes this path unreachable.
            unsafe { unreachable_invariant("an iterator step returns to its foreach continuation") }
        };

        let cursor = self.stack[base + iterator.index() as usize]
            .as_iterator()
            // SAFETY: the surrounding invariant makes this path unreachable.
            .unwrap_or_else(|| unsafe {
                unreachable_invariant("the iterator register holds the loop cursor")
            });
        let was_pending = cursor.take_pending_object_step();
        debug_assert!(was_pending);

        if key_destination == Register::NONE {
            drop(key);
        } else {
            self.store_result(base + key_destination.index() as usize, key);
        }
        self.store_result(base + value_destination.index() as usize, value);
        self.current_frame_mut().ip = (instruction_index + 2) as u32;
    }

    /// Completes an exhausted object iterator step by taking the `foreach`
    /// exit edge without storing an intermediate `null` result.
    #[inline(always)]
    fn return_iterator_exhausted(&mut self) {
        let finished = self.pop_frame();
        self.truncate_frame_stack(&finished);

        let (base, instruction_index, chunk) = {
            let caller = self.current_frame();
            // SAFETY: verified bytecode keeps operands in the live frame and proves their types.
            (caller.base as usize, caller.ip as usize, unsafe {
                caller.chunk.as_ref()
            })
        };
        let Instruction::ForeachNext { iterator, .. } =
            // SAFETY: the surrounding invariant keeps this index in bounds.
            (unsafe { *chunk.code.get_unchecked(instruction_index) })
        else {
            // SAFETY: the surrounding invariant makes this path unreachable.
            unsafe { unreachable_invariant("an iterator step returns to its foreach continuation") }
        };
        let Instruction::Jump { offset } =
            // SAFETY: the surrounding invariant keeps this index in bounds.
            (unsafe { *chunk.code.get_unchecked(instruction_index + 1) })
        else {
            // SAFETY: the surrounding invariant makes this path unreachable.
            unsafe { unreachable_invariant("`ForeachNext` is followed by its exit jump") }
        };

        let cursor = self.stack[base + iterator.index() as usize]
            .as_iterator()
            // SAFETY: the surrounding invariant makes this path unreachable.
            .unwrap_or_else(|| unsafe {
                unreachable_invariant("the iterator register holds the loop cursor")
            });
        let was_pending = cursor.take_pending_object_step();
        debug_assert!(was_pending);
        self.current_frame_mut().ip = jump_target(instruction_index + 1, offset.offset()) as u32;
    }
    #[inline(always)]
    unsafe fn take_owned_register(
        &mut self,
        registers: *mut Value,
        source: Register,
        clear_reference_mask: bool,
    ) -> Value {
        debug_assert!(
            source.index() != 0 || !self.current_frame().borrows_register_zero(),
            "a borrowed receiver must never be transferred out of its frame"
        );

        // SAFETY: verified bytecode keeps operands in the live frame and proves their types.
        let value = unsafe {
            mem::replace(
                &mut *registers.add(source.index() as usize),
                Value::uninitialized(),
            )
        };

        if clear_reference_mask && source.index() < REFERENCE_REGISTER_LIMIT {
            self.current_frame_mut().reference_register_mask &= !(1u64 << source.index());
        }

        value
    }

    #[inline(always)]
    unsafe fn initialize_fresh_property(
        &mut self,
        registers: *mut Value,
        receiver: &InstanceObject,
        source: Register,
        slot: PropertySlot,
        value_mode: PropertyValueMode,
    ) {
        debug_assert!(value_mode.fresh_receiver());
        let value = if value_mode.moves() {
            // SAFETY: verified bytecode keeps operands in the live frame and proves their types.
            unsafe {
                self.take_owned_register(registers, source, value_mode.clears_reference_mask())
            }
        } else {
            read_register!(registers, source)
        };
        // SAFETY: verified bytecode keeps operands in the live frame and proves their types.
        unsafe { receiver.write_fresh_slot_unchecked(usize::from(slot.index()), value) };
    }

    #[inline(always)]
    unsafe fn try_initialize_cached_static_object(
        &mut self,
        registers: *mut Value,
        destination: Register,
        site: u16,
        entries: &[PropertyInitializationEntry],
    ) -> bool {
        let outer = self.current_frame().type_environment;
        let Some(cached) = self
            .cached_instantiation_environment(usize::from(site), outer)
            .filter(|cached| cached.allocates_plainly)
        else {
            return false;
        };
        debug_assert_eq!(cached.slot_count as usize, entries.len());
        let heap = Rc::clone(&self.heap);
        let value = Value::object(InstanceObject::new_initialized_typed_with_layout(
            &heap,
            cached.class,
            cached.slot_count as usize,
            cached.environment,
            cached.slots_are_acyclic,
            |index| {
                // SAFETY: the surrounding invariant keeps this index in bounds.
                let entry = unsafe { entries.get_unchecked(index) };
                if entry.value_mode.moves() {
                    // SAFETY: verified bytecode keeps operands in the live frame and proves their types.
                    unsafe {
                        self.take_owned_register(
                            registers,
                            entry.value,
                            entry.value_mode.clears_reference_mask(),
                        )
                    }
                } else {
                    // SAFETY: verified bytecode keeps operands in the live frame and proves their types.
                    unsafe { (&*registers.add(entry.value.index() as usize)).clone_inline_scalar() }
                }
            },
        ));
        write_register!(registers, destination, value);
        true
    }

    #[inline(never)]
    fn initialize_static_object_slow(
        &mut self,
        destination: Register,
        site: u16,
        descriptor: PropertyInitializationDescriptorIndex,
        chunk: &Chunk,
    ) -> Result<(), VirtualMachineControl> {
        let value = self.new_static_site(usize::from(site), chunk)?;
        let ValueView::Object(receiver) = value.transparent() else {
            // SAFETY: the surrounding invariant makes this path unreachable.
            unsafe { unreachable_invariant("a static object initializer allocates an object") }
        };
        // SAFETY: verified bytecode keeps operands in the live frame and proves their types.
        let registers = unsafe {
            self.stack
                .as_mut_ptr()
                .add(self.current_frame().base as usize)
        };
        for entry in &chunk.property_initialization_descriptor(descriptor).entries {
            // SAFETY: verified bytecode keeps operands in the live frame and proves their types.
            unsafe {
                self.initialize_fresh_property(
                    registers,
                    receiver,
                    entry.value,
                    entry.slot,
                    entry.value_mode,
                );
            }
        }
        let target = self.current_frame().base as usize + destination.index() as usize;
        self.store_result(target, value);
        Ok(())
    }

    /// Whether the active user function's declared return type accepts
    /// `result`. `void` accepts the VM's null sentinel for a valueless return;
    /// `never` rejects every normal return, including fallthrough.
    fn return_value_is_valid(&mut self, result: &Value) -> Result<bool, VirtualMachineControl> {
        let Some(frame) = self.frames.last() else {
            // SAFETY: the surrounding invariant makes this path unreachable.
            unsafe { unreachable_invariant("a return always has an active frame") }
        };

        let Some(function) = frame.function.get() else {
            return Ok(true);
        };

        self.callable_return_value_is_valid(
            function,
            result,
            frame.called_class.get(),
            frame.type_environment,
        )
    }

    pub(in crate::vm) fn callable_return_value_is_valid(
        &mut self,
        function: FuncId,
        result: &Value,
        called: Option<ClassId>,
        environment: TypeEnvironmentId,
    ) -> Result<bool, VirtualMachineControl> {
        let descriptor = {
            let runtime = &self.engine.tables.functions[function.0 as usize];
            let Some(descriptor) = runtime.return_type.as_deref() else {
                return Ok(true);
            };

            let valid = match descriptor {
                TypeDescriptor::Void => Some(result.is_null()),
                TypeDescriptor::Never => Some(false),
                _ => check_trivial_descriptor(descriptor, result),
            };

            if let Some(valid) = valid {
                return Ok(valid);
            }

            NonNull::from(descriptor)
        };

        // SAFETY: verified bytecode keeps operands in the live frame and proves their types.
        let descriptor = unsafe { descriptor.as_ref() };
        let valid = match descriptor {
            TypeDescriptor::Void => result.is_null(),
            TypeDescriptor::Never => false,
            _ => match check_trivial_descriptor(descriptor, result) {
                Some(valid) => valid,
                None => self.check_descriptor(descriptor, result, called, environment, 0)?,
            },
        };

        Ok(valid)
    }

    /// Builds the return mismatch after dispatch has synchronized the precise
    /// failing instruction. This is cold and deliberately separate from the
    /// valid-return path.
    #[cold]
    #[inline(never)]
    fn return_type_mismatch(&mut self, result: &Value) -> VirtualMachineControl {
        let frame = self.current_frame();
        // SAFETY: the surrounding invariant proves this option contains a value.
        let function = unsafe {
            unwrap_option_invariant(
                frame.function.get(),
                "only a user function has a return type mismatch",
            )
        };

        self.callable_return_type_mismatch(function, result, frame.type_environment)
    }

    pub(in crate::vm) fn callable_return_type_mismatch(
        &mut self,
        function: FuncId,
        result: &Value,
        environment: TypeEnvironmentId,
    ) -> VirtualMachineControl {
        // SAFETY: the surrounding invariant proves this option contains a value.
        let descriptor = unsafe {
            unwrap_option_invariant(
                self.engine.tables.functions[function.0 as usize]
                    .return_type
                    .as_deref(),
                "a return mismatch has a declared return type",
            )
        };

        let concrete = self.substitute_descriptor(descriptor, environment, 0);
        let expected = self.render_descriptor(&concrete);
        let found = self.value_type_name(result);
        let function_name = self.engine.tables.functions[function.0 as usize]
            .name
            .clone();
        self.throw_well_known(
            self.engine.tables.well_known.type_error,
            format!(
                "{} must return {expected}, {found} returned",
                function_name.to_string_lossy()
            ),
        )
    }

    #[inline(always)]
    fn return_from_frame(&mut self, result: Value, floor: usize) -> Option<Value> {
        let reference_counted = result.is_reference_counted();
        self.return_from_frame_inner(result, floor, 0, reference_counted)
    }

    #[inline(always)]
    fn return_from_scalar_frame(&mut self, result: Value, floor: usize) -> Option<Value> {
        debug_assert!(!result.is_reference_counted());
        self.return_from_frame_inner(result, floor, 0, false)
    }

    /// Returns a value moved out of one callee register. Clearing that
    /// register from the teardown mask lets reference-valued returns use the
    /// fast path when the callee owns no other references.
    #[inline(always)]
    fn return_from_register_frame(
        &mut self,
        result: Value,
        floor: usize,
        source: Register,
    ) -> Option<Value> {
        if result.is_reference_counted() {
            self.return_from_reference_register_frame(result, floor, source)
        } else {
            self.return_from_frame_inner(result, floor, 0, false)
        }
    }

    /// Returns a value proven to own a heap reference from one callee
    /// register without rechecking its runtime category.
    #[inline(always)]
    fn return_from_reference_register_frame(
        &mut self,
        result: Value,
        floor: usize,
        source: Register,
    ) -> Option<Value> {
        debug_assert!(result.is_reference_counted());
        let moved_register = usize::from(source.index());
        let moved_mask = if moved_register < usize::from(REFERENCE_REGISTER_LIMIT) {
            1u64 << moved_register
        } else {
            0
        };

        self.return_from_frame_inner(result, floor, moved_mask, true)
    }

    #[inline(always)]
    fn store_result(&mut self, target: usize, value: Value) {
        debug_assert!(target < self.stack.len());
        // SAFETY: verified bytecode keeps operands in the live frame and proves their types.
        let slot = unsafe { self.stack.as_mut_ptr().add(target) };
        // SAFETY: verified bytecode keeps operands in the live frame and proves their types.
        if !unsafe { &*slot }.is_reference_counted() {
            // SAFETY: verified bytecode keeps operands in the live frame and proves their types.
            unsafe { slot.write(value) };
        } else {
            // SAFETY: verified bytecode keeps operands in the live frame and proves their types.
            unsafe { *slot = value };
        }
    }

    #[inline(always)]
    fn return_from_frame_inner(
        &mut self,
        result: Value,
        floor: usize,
        moved_mask: u64,
        result_is_reference: bool,
    ) -> Option<Value> {
        self.remember_discarded_result();
        let frame_count = self.frames.len();
        // SAFETY: the surrounding invariant keeps this index in bounds.
        let finished = unsafe { self.frames.get_unchecked(frame_count - 1) };
        let releasing = finished.reference_register_mask & !moved_mask;
        if self.pending_unwinds.is_empty()
            // SAFETY: verified bytecode keeps operands in the live frame and proves their types.
            && (releasing == 0
                || {
                    // SAFETY: verified bytecode keeps operands in the live frame and proves their types.
                    unsafe { finished.chunk.as_ref() }.register_count <= REFERENCE_REGISTER_LIMIT
                })
        {
            let base = finished.base as usize;
            let stack_floor = finished.stack_floor() as usize;
            let return_register = usize::from(finished.return_register);
            let borrowed_register_zero = finished.borrows_register_zero();
            let scalar_return_target = finished.scalar_return_target();
            // SAFETY: verified bytecode keeps operands in the live frame and proves their types.
            unsafe { self.frames.set_len(frame_count - 1) };
            if borrowed_register_zero {
                // SAFETY: verified bytecode keeps operands in the live frame and proves their types.
                unsafe {
                    self.stack
                        .as_mut_ptr()
                        .add(base)
                        .write(Value::uninitialized());
                }
            }

            let mut mask = releasing;
            while mask != 0 {
                let register = mask.trailing_zeros() as usize;
                // SAFETY: verified bytecode keeps operands in the live frame and proves their types.
                let value = unsafe {
                    ptr::replace(
                        self.stack.as_mut_ptr().add(base + register),
                        Value::uninitialized(),
                    )
                };
                drop(value);
                mask &= mask - 1;
            }

            // SAFETY: verified bytecode keeps operands in the live frame and proves their types.
            unsafe { self.stack.set_len(stack_floor) };

            if frame_count - 1 == floor {
                return Some(result);
            }

            // SAFETY: the surrounding invariant keeps this index in bounds.
            let caller = unsafe { self.frames.get_unchecked_mut(frame_count - 2) };
            let target = caller.base as usize + return_register;
            if scalar_return_target {
                debug_assert!(!result_is_reference);
                // SAFETY: verified bytecode keeps operands in the live frame and proves their types.
                unsafe {
                    self.stack.as_mut_ptr().add(target).write(result);
                }
                return None;
            }

            let scalar_target = caller.reference_register_mask == 0
                || (return_register < usize::from(REFERENCE_REGISTER_LIMIT)
                    // SAFETY: verified bytecode keeps operands in the live frame and proves their types.
                    && unsafe { caller.chunk.as_ref() }.register_count <= REFERENCE_REGISTER_LIMIT
                    && caller.reference_register_mask & (1u64 << return_register) == 0);
            if scalar_target {
                if result_is_reference {
                    debug_assert!(
                        return_register < usize::from(REFERENCE_REGISTER_LIMIT)
                            // SAFETY: verified bytecode keeps operands in the live frame and proves their types.
                            && unsafe { caller.chunk.as_ref() }.register_count
                                <= REFERENCE_REGISTER_LIMIT
                    );
                    caller.reference_register_mask |= 1u64 << return_register;
                }

                // SAFETY: verified bytecode keeps operands in the live frame and proves their types.
                unsafe {
                    self.stack.as_mut_ptr().add(target).write(result);
                }
            } else {
                self.store_result(target, result);
            }

            return None;
        }

        self.return_from_frame_slow(result, floor, result_is_reference)
    }

    #[inline(never)]
    fn return_from_frame_slow(
        &mut self,
        result: Value,
        floor: usize,
        result_is_reference: bool,
    ) -> Option<Value> {
        let finished = self.pop_frame();
        self.truncate_frame_stack(&finished);
        if self.frames.len() == floor {
            return Some(result);
        }

        let register = usize::from(finished.return_register);
        let (target, scalar_target) = {
            let caller = self.current_frame_mut();
            let target = caller.base as usize + register;
            let scalar_target = caller.reference_register_mask == 0
                || (register < usize::from(REFERENCE_REGISTER_LIMIT)
                    // SAFETY: verified bytecode keeps operands in the live frame and proves their types.
                    && unsafe { caller.chunk.as_ref() }.register_count <= REFERENCE_REGISTER_LIMIT
                    && caller.reference_register_mask & (1u64 << register) == 0);
            if scalar_target && result_is_reference {
                debug_assert!(
                    register < usize::from(REFERENCE_REGISTER_LIMIT) && {
                        // SAFETY: verified bytecode keeps operands in the live frame and proves their types.
                        unsafe { caller.chunk.as_ref() }.register_count <= REFERENCE_REGISTER_LIMIT
                    }
                );
                caller.reference_register_mask |= 1u64 << register;
            }

            (target, scalar_target)
        };

        if scalar_target {
            // SAFETY: verified bytecode keeps operands in the live frame and proves their types.
            unsafe {
                self.stack.as_mut_ptr().add(target).write(result);
            }
        } else {
            self.store_result(target, result);
        }

        None
    }

    /// Removes the active frame while copying out only the fields its
    /// teardown needs.
    #[inline(always)]
    fn pop_frame(&mut self) -> FrameTeardown {
        let Some(frame_index) = self.frames.len().checked_sub(1) else {
            // SAFETY: the surrounding invariant makes this path unreachable.
            unsafe { unreachable_invariant("the VM only pops an active frame") }
        };

        let finished = FrameTeardown::from_frame(&self.frames[frame_index]);
        // SAFETY: verified bytecode keeps operands in the live frame and proves their types.
        unsafe { self.frames.set_len(frame_index) };
        if !self.pending_unwinds.is_empty() {
            self.discard_pending_unwind(frame_index);
        }

        finished
    }

    /// Releases the registers that may own heap handles and abandons the
    /// scalar-only remainder without running enum drop glue for every slot.
    #[inline(always)]
    fn truncate_frame_stack(&mut self, frame: &FrameTeardown) {
        if frame.borrows_register_zero {
            // SAFETY: verified bytecode keeps operands in the live frame and proves their types.
            unsafe {
                self.stack
                    .as_mut_ptr()
                    .add(frame.base as usize)
                    .write(Value::uninitialized());
            }
        }

        if frame.reference_register_mask == 0 {
            // SAFETY: verified bytecode keeps operands in the live frame and proves their types.
            unsafe { self.stack.set_len(frame.stack_floor as usize) };
            return;
        }

        // SAFETY: verified bytecode keeps operands in the live frame and proves their types.
        let chunk = unsafe { frame.chunk.as_ref() };
        if chunk.register_count > REFERENCE_REGISTER_LIMIT {
            self.stack.truncate(frame.stack_floor as usize);
            self.stack_initialized_len = self.stack.len();
            return;
        }

        let base = frame.base as usize;
        let mut mask = frame.reference_register_mask;
        while mask != 0 {
            let register = mask.trailing_zeros() as usize;
            // SAFETY: verified bytecode keeps operands in the live frame and proves their types.
            let value = unsafe {
                ptr::replace(
                    self.stack.as_mut_ptr().add(base + register),
                    Value::uninitialized(),
                )
            };

            drop(value);
            mask &= mask - 1;
        }

        // SAFETY: verified bytecode keeps operands in the live frame and proves their types.
        unsafe { self.stack.set_len(frame.stack_floor as usize) };
    }

    #[inline(always)]
    fn discard_pending_unwind(&mut self, frame: usize) {
        if self
            .pending_unwinds
            .last()
            .is_some_and(|pending| pending.frame == frame)
        {
            self.pending_unwinds.pop();
        }
    }

    fn validate_throwable(&mut self, value: Value) -> Result<Value, VirtualMachineControl> {
        let is_throwable = value
            .as_object()
            .is_some_and(|instance| self.engine.is_throwable_instance(instance.class()));

        if is_throwable {
            Ok(value)
        } else {
            Err(self.throw_well_known(
                self.engine.tables.well_known.type_error,
                format!(
                    "only a Whim\\Unwind\\Throwable instance can be thrown, {} given",
                    value.kind_name()
                ),
            ))
        }
    }

    fn clear_catch_temporaries(&mut self, base: usize, floor: u16, ceiling: u16) {
        for slot in &mut self.stack[base + usize::from(floor)..base + usize::from(ceiling)] {
            *slot = Value::uninitialized();
        }
    }

    /// Routes a control transfer: an exit propagates, a throw unwinds.
    pub(in crate::vm) fn handle_control(
        &mut self,
        control: VirtualMachineControl,
        floor: usize,
    ) -> Result<(), VirtualMachineControl> {
        match control {
            VirtualMachineControl::Exit(code) => Err(VirtualMachineControl::Exit(code)),
            VirtualMachineControl::Throw(value) => self.unwind(value, floor),
        }
    }

    /// Finds a matching catch or pops the frame. The compiler emits each
    /// `finally` path, so unwinding does not run it here.
    pub(in crate::vm) fn unwind(
        &mut self,
        thrown: Value,
        floor: usize,
    ) -> Result<(), VirtualMachineControl> {
        let mut thrown = thrown;
        loop {
            if self.frames.len() <= floor {
                return Err(VirtualMachineControl::Throw(thrown));
            }

            let frame_index = self.frames.len() - 1;
            let (chunk_pointer, base, ip) = {
                let frame = &self.frames[frame_index];
                (frame.chunk, frame.base as usize, frame.ip)
            };

            // SAFETY: verified bytecode keeps operands in the live frame and proves their types.
            let chunk = unsafe { chunk_pointer.as_ref() };
            let faulting = ip.saturating_sub(1);
            let mut matched = None;
            for entry in &chunk.catch_table {
                if entry.start <= faulting && faulting < entry.end {
                    match self.descriptor_matches(
                        chunk,
                        entry.type_descriptor,
                        &thrown,
                        frame_index,
                    ) {
                        Ok(true) => {
                            matched = Some(*entry);
                            break;
                        }
                        Ok(false) => {}
                        Err(VirtualMachineControl::Throw(replacement)) => {
                            thrown = replacement;
                            break;
                        }
                        Err(control) => return Err(control),
                    }
                }
            }

            if let Some(entry) = matched {
                self.clear_catch_temporaries(base, entry.temporary_floor, chunk.register_count);
                if let Some(binding) = entry.binding {
                    self.stack[base + binding.index() as usize] = thrown.clone();
                }

                if let Some(pending) = self
                    .pending_unwinds
                    .last_mut()
                    .filter(|pending| pending.frame == frame_index)
                {
                    pending.value = thrown;
                } else {
                    self.pending_unwinds.push(PendingUnwind {
                        frame: frame_index,
                        value: thrown,
                    });
                }

                self.frames[frame_index].ip = entry.handler;
                return Ok(());
            }

            self.relocate_tracked_error_origin(&thrown, frame_index);
            let finished = self.pop_frame();
            self.truncate_frame_stack(&finished);
        }
    }
}

#[cfg(test)]
mod string_switch_tests {
    use super::SwitchTable;
    use super::Value;
    use super::switch_string_target;
    use crate::bytecode::chunk::descriptors::string_switch_buckets;
    use crate::value::heap::Heap;
    use crate::value::newtype::NewtypeValueId;
    use crate::value::string::ByteStringObject;
    use crate::value::string::short::ShortString;

    #[test]
    fn string_switches_keep_first_duplicate_and_strict_subject_type() {
        let heap = Heap::new();
        for count in [2, 4, 5, 16] {
            let arms = (0..count)
                .map(|index| (heap.intern(b"same"), index + 10))
                .collect::<Vec<_>>();
            let table = SwitchTable::String {
                buckets: string_switch_buckets(&arms),
                arms,
                default: -1,
            };

            let subject = Value::string(heap.intern(b"same").to_handle());
            assert_eq!(switch_string_target(&table, &subject), 10);
            assert_eq!(
                switch_string_target(&table, &Value::newtype(subject, NewtypeValueId(0))),
                10,
            );
            assert_eq!(switch_string_target(&table, &Value::int(10)), -1);
            assert_eq!(
                switch_string_target(&table, &Value::string(heap.intern(b"missing").to_handle())),
                -1,
            );
        }
    }

    #[test]
    fn string_switches_compare_bytes_across_representations_and_table_sizes() {
        const LONG: &[u8] =
            b"shared-prefix-with-a-string-long-enough-to-have-a-rope-representation";
        let heap = Heap::new();
        let cases = [
            (b"".as_slice(), 10),
            ("héllo".as_bytes(), 20),
            (b"x\0y", 30),
            (LONG, 40),
        ];

        for count in [4, 5, 16] {
            let mut arms = cases
                .into_iter()
                .map(|(bytes, target)| (heap.intern(bytes), target))
                .collect::<Vec<_>>();
            for index in arms.len()..count {
                arms.push((heap.intern(format!("extra-{index}").as_bytes()), -2));
            }

            let table = SwitchTable::String {
                buckets: string_switch_buckets(&arms),
                arms,
                default: -1,
            };

            for (bytes, expected) in cases {
                let subject = Value::string(ByteStringObject::from_bytes(&heap, bytes));
                assert_eq!(switch_string_target(&table, &subject), expected);
                if let Some(short) = ShortString::from_bytes(bytes) {
                    assert_eq!(
                        switch_string_target(&table, &Value::short_string(short)),
                        expected,
                    );
                }
            }

            let left = ByteStringObject::from_bytes(&heap, &LONG[..24]);
            let right = ByteStringObject::from_bytes(&heap, &LONG[24..]);
            let rope = Value::string(ByteStringObject::concat(&heap, &left, &right));
            assert_eq!(switch_string_target(&table, &rope), 40);
            let padded =
                ByteStringObject::from_vec(&heap, [b"before".as_slice(), LONG, b"after"].concat());
            let slice = Value::string(ByteStringObject::slice(&heap, &padded, 6, LONG.len()));
            assert_eq!(switch_string_target(&table, &slice), 40);
            if count > 4 {
                let last = Value::string(
                    heap.intern(format!("extra-{}", count - 1).as_bytes())
                        .to_handle(),
                );

                assert_eq!(switch_string_target(&table, &last), -2);
            }
        }
    }
}
