//! Unboxed execution of optimizer-selected numeric counted loops.

use std::mem::MaybeUninit;
use std::ptr;

use crate::bytecode::chunk::descriptors::FloatPairUpdateDescriptor;
use crate::bytecode::chunk::descriptors::FloatSquaresSumBranchDescriptor;
use crate::bytecode::chunk::descriptors::PreparedIntLoopDescriptor;
use crate::bytecode::instruction::NUMERIC_LOOP_REGISTER_LIMIT;
use crate::bytecode::instruction::operands::ArrayValueMode;
use crate::bytecode::instruction::operands::Comparison as BytecodeComparison;
use crate::bytecode::unit::literal_value;
use crate::optimizer::relative_target;
use crate::value::ValueView;
use crate::value::dict::keys::Key;
use crate::vm::ArrayFault;
use crate::vm::ByteStringObject;
use crate::vm::Chunk;
use crate::vm::Fault;
use crate::vm::Instruction;
use crate::vm::InstructionKind;
use crate::vm::InstructionWord;
use crate::vm::Literal;
use crate::vm::Register;
use crate::vm::Value;
use crate::vm::VirtualMachine;
use crate::vm::int_position;
use crate::vm::integer_add;
use crate::vm::integer_modulo;
use crate::vm::integer_multiply;
use crate::vm::integer_subtract;
use crate::vm::unreachable_invariant;

use crate::vm::numeric_loop::arithmetic::add;
use crate::vm::numeric_loop::arithmetic::comparison_matches_numeric;
use crate::vm::numeric_loop::arithmetic::equals_numeric;
use crate::vm::numeric_loop::arithmetic::float_ordered_comparison_matches;
use crate::vm::numeric_loop::arithmetic::int_comparison_matches_any;
use crate::vm::numeric_loop::arithmetic::int_ordered_comparison_matches;
use crate::vm::numeric_loop::arithmetic::multiply;
use crate::vm::numeric_loop::arithmetic::step_counter;
use crate::vm::numeric_loop::arithmetic::stepped_loop_iterations;
use crate::vm::numeric_loop::arithmetic::subtract;

mod arithmetic;

const BATCH_ITERATION_LIMIT: u32 = 65_536;

#[derive(Clone, Copy, PartialEq, Eq)]
enum NumericKind {
    Other,
    Int,
    Float,
    Bool,
}

#[derive(Clone, Copy)]
struct NumericValue {
    bits: u64,
    kind: NumericKind,
}

struct NumericRegisters {
    bits: [MaybeUninit<u64>; NUMERIC_LOOP_REGISTER_LIMIT as usize],
    kinds: [MaybeUninit<NumericKind>; NUMERIC_LOOP_REGISTER_LIMIT as usize],
}

impl NumericRegisters {
    /// Copies numeric registers into an unboxed shadow.
    ///
    /// # Safety
    ///
    /// `registers` must point to at least `register_count` initialized values,
    /// and every subsequent numeric-loop operand must be below
    /// `register_count`.
    unsafe fn from_registers(
        registers: *mut Value,
        register_count: u16,
    ) -> (NumericRegisters, u64) {
        let mut shadow = NumericRegisters {
            bits: [MaybeUninit::uninit(); NUMERIC_LOOP_REGISTER_LIMIT as usize],
            kinds: [MaybeUninit::uninit(); NUMERIC_LOOP_REGISTER_LIMIT as usize],
        };
        let mut numeric = 0u64;
        for index in 0..usize::from(register_count) {
            // SAFETY: the numeric-loop proof covers the instruction, registers, and types.
            let value = NumericValue::from_value(unsafe { &*registers.add(index) });
            shadow.bits[index].write(value.bits);
            shadow.kinds[index].write(value.kind);
            if value.kind != NumericKind::Other {
                numeric |= 1u64 << index;
            }
        }

        (shadow, numeric)
    }

    #[inline(always)]
    fn get(&self, index: usize) -> NumericValue {
        NumericValue {
            // SAFETY: the surrounding invariant keeps this index in bounds.
            bits: unsafe { *self.bits.get_unchecked(index).assume_init_ref() },
            kind: self.kind(index),
        }
    }

    #[inline(always)]
    fn kind(&self, index: usize) -> NumericKind {
        // SAFETY: the surrounding invariant keeps this index in bounds.
        unsafe { *self.kinds.get_unchecked(index).assume_init_ref() }
    }

    #[inline(always)]
    fn set(&mut self, index: usize, value: NumericValue) {
        // SAFETY: the surrounding invariant keeps this index in bounds.
        unsafe {
            *self.bits.get_unchecked_mut(index).assume_init_mut() = value.bits;
            *self.kinds.get_unchecked_mut(index).assume_init_mut() = value.kind;
        }
    }

    #[inline(always)]
    fn set_bits(&mut self, index: usize, bits: u64) {
        // SAFETY: the surrounding invariant keeps this index in bounds.
        unsafe { *self.bits.get_unchecked_mut(index).assume_init_mut() = bits };
    }

    #[inline(always)]
    fn set_kind(&mut self, index: usize, kind: NumericKind) {
        // SAFETY: the surrounding invariant keeps this index in bounds.
        unsafe { *self.kinds.get_unchecked_mut(index).assume_init_mut() = kind };
    }

    #[inline(always)]
    fn float(&self, index: usize) -> f64 {
        debug_assert!(self.kind(index) == NumericKind::Float);
        // SAFETY: the surrounding invariant keeps this index in bounds.
        f64::from_bits(unsafe { *self.bits.get_unchecked(index).assume_init_ref() })
    }

    #[inline(always)]
    fn int(&self, index: usize) -> i64 {
        debug_assert!(self.kind(index) == NumericKind::Int);
        // SAFETY: the surrounding invariant keeps this index in bounds.
        unsafe { *self.bits.get_unchecked(index).assume_init_ref() as i64 }
    }
}

impl NumericValue {
    const OTHER: NumericValue = NumericValue {
        bits: 0,
        kind: NumericKind::Other,
    };

    #[inline(always)]
    fn int(value: i64) -> NumericValue {
        NumericValue {
            bits: value as u64,
            kind: NumericKind::Int,
        }
    }

    #[inline(always)]
    fn float(value: f64) -> NumericValue {
        NumericValue {
            bits: value.to_bits(),
            kind: NumericKind::Float,
        }
    }

    #[inline(always)]
    fn int_value(self) -> i64 {
        self.bits as i64
    }

    #[inline(always)]
    fn float_value(self) -> f64 {
        debug_assert!(self.kind == NumericKind::Float);
        f64::from_bits(self.bits)
    }

    #[inline(always)]
    fn bool(value: bool) -> NumericValue {
        NumericValue {
            bits: u64::from(value),
            kind: NumericKind::Bool,
        }
    }

    fn from_value(value: &Value) -> NumericValue {
        match value.transparent() {
            ValueView::Int(value) => NumericValue::int(*value),
            ValueView::Float(value) => NumericValue::float(*value),
            ValueView::Bool(value) => NumericValue::bool(*value),
            _ => NumericValue::OTHER,
        }
    }

    fn into_value(self) -> Value {
        match self.kind {
            NumericKind::Int => Value::int(self.int_value()),
            NumericKind::Float => Value::float(self.float_value()),
            NumericKind::Bool => Value::bool(self.bits != 0),
            // SAFETY: the surrounding invariant makes this path unreachable.
            NumericKind::Other => unsafe {
                unreachable_invariant("only a numeric shadow value is flushed")
            },
        }
    }
}

pub(in crate::vm) enum NumericLoopOutcome {
    Completed,
    Deoptimize(usize),
    Fault {
        resume_ip: usize,
        fault: Fault,
        operator: &'static str,
        left: Register,
        right: Option<Register>,
    },
    Array {
        resume_ip: usize,
        fault: ArrayFault,
    },
}

impl VirtualMachine<'_> {
    /// Runs a loop with unboxed numeric registers until exit or deoptimization.
    ///
    /// # Safety
    ///
    /// `registers` must be the live register window for `chunk`, whose
    /// verified register count does not exceed [`NUMERIC_LOOP_REGISTER_LIMIT`].
    #[inline(never)]
    pub(in crate::vm) unsafe fn run_numeric_loop<const PREPARED_FLOATS: bool>(
        &mut self,
        chunk: &Chunk,
        registers: *mut Value,
        body: usize,
        exit: usize,
        float_registers: u64,
        dirty_registers: u64,
    ) -> NumericLoopOutcome {
        debug_assert!(chunk.register_count <= NUMERIC_LOOP_REGISTER_LIMIT);
        // SAFETY: the caller provides the active frame's verified register window.
        let (mut values, numeric_registers) =
            unsafe { NumericRegisters::from_registers(registers, chunk.register_count) };
        if PREPARED_FLOATS {
            let mut remaining = float_registers;
            while remaining != 0 {
                let index = remaining.trailing_zeros() as usize;
                values.set_kind(index, NumericKind::Float);
                remaining &= remaining - 1;
            }
        }
        let mut dirty = dirty_registers | numeric_registers;
        let mut cursor = body;
        let mut scan_burst_miss = usize::MAX;
        let mut string_slice_burst_miss = usize::MAX;
        let mut int_body_burst_miss = usize::MAX;
        let mut vec_append_burst_miss = usize::MAX;
        let mut indexed_int_body_burst_miss = usize::MAX;
        let mut marker_burst_miss = usize::MAX;

        let vec_append_targets = chunk.vec_append_register_mask;
        let mut targets = vec_append_targets;
        while targets != 0 {
            let index = targets.trailing_zeros() as usize;
            // SAFETY: the target mask contains only active-frame registers.
            if let Some(vector) = unsafe { &*registers.add(index) }.as_vec() {
                vector.invalidate_type_check();
            }
            targets &= targets - 1;
        }

        // SAFETY: a pin stays valid because every overwrite of a
        // vec-holding register invalidates it and the register's handle
        // keeps the buffer alive.
        let mut pins = Pins::new();

        macro_rules! vec_element_read {
            ($current:expr, $destination:expr, $container:expr, $index_value:expr, $value_mode:expr) => {{
                // SAFETY: the container is an active-frame register tracked by `pins`.
                let Some((elements, length)) = (unsafe { pins.for_read(registers, $container) })
                else {
                    // SAFETY: `dirty` contains only active-frame numeric registers.
                    unsafe { flush(registers, &values, dirty) };
                    return NumericLoopOutcome::Deoptimize($current);
                };

                let position = match int_position($index_value, length) {
                    Ok(position) => position,
                    Err(fault) => {
                        // SAFETY: `dirty` contains only active-frame numeric registers.
                        unsafe { flush(registers, &values, dirty) };

                        return NumericLoopOutcome::Array {
                            resume_ip: cursor,
                            fault,
                        };
                    }
                };

                // SAFETY: `position` is bounded by the pinned length.
                let element = unsafe { &*elements.add(position) };
                match $value_mode {
                    ArrayValueMode::Int => {
                        // SAFETY: the specialized mode proves the element is an int.
                        let value = NumericValue::int(unsafe { element.as_int_unchecked() });

                        // SAFETY: the destination is in the active numeric register window.
                        unsafe {
                            assign(
                                registers,
                                &mut values,
                                &mut dirty,
                                &mut pins,
                                $destination,
                                value,
                            )
                        };
                    }
                    ArrayValueMode::Float => {
                        // SAFETY: the specialized mode proves the element is a float.
                        let value = NumericValue::float(unsafe { element.as_float_unchecked() });

                        // SAFETY: the destination is in the active numeric register window.
                        unsafe {
                            assign(
                                registers,
                                &mut values,
                                &mut dirty,
                                &mut pins,
                                $destination,
                                value,
                            )
                        };
                    }
                    ArrayValueMode::Generic => {
                        // SAFETY: the destination is in the active numeric register window.
                        unsafe {
                            assign_array_element(
                                registers,
                                &mut values,
                                &mut dirty,
                                &mut pins,
                                $destination,
                                element,
                            )
                        }
                    }
                }
            }};
        }

        macro_rules! vec_element_write {
            ($current:expr, $container:expr, $index_value:expr, $value_register:expr) => {{
                let value_index = $value_register.index() as usize;
                if values.kind(value_index) == NumericKind::Other {
                    // SAFETY: `dirty` contains only active-frame numeric registers.
                    unsafe { flush(registers, &values, dirty) };
                    return NumericLoopOutcome::Deoptimize($current);
                }

                // SAFETY: the container is an active-frame register tracked by `pins`.
                let Some((elements, length)) = (unsafe { pins.for_write(registers, $container) })
                else {
                    // SAFETY: `dirty` contains only active-frame numeric registers.
                    unsafe { flush(registers, &values, dirty) };
                    return NumericLoopOutcome::Deoptimize($current);
                };

                let position = match int_position($index_value, length) {
                    Ok(position) => position,
                    Err(fault) => {
                        // SAFETY: `dirty` contains only active-frame numeric registers.
                        unsafe { flush(registers, &values, dirty) };
                        return NumericLoopOutcome::Array {
                            resume_ip: cursor,
                            fault,
                        };
                    }
                };

                // SAFETY: `position` is bounded by the pinned length.
                let target = unsafe { &mut *elements.add(position) };
                let value = values.get(value_index);
                match value.kind {
                    NumericKind::Int if target.is_int() => {
                        // SAFETY: the guard proved the target is an int.
                        *unsafe { target.as_int_mut().unwrap_unchecked() } = value.int_value();
                    }
                    NumericKind::Float if target.is_float() => {
                        // SAFETY: the guard proved the target is a float.
                        *unsafe { target.as_float_mut().unwrap_unchecked() } = value.float_value();
                    }
                    NumericKind::Bool if target.is_bool() => {
                        // SAFETY: the guard proved the target is a bool.
                        *unsafe { target.as_bool_mut().unwrap_unchecked() } = value.bits != 0;
                    }
                    _ => *target = value.into_value(),
                }
            }};
        }

        macro_rules! dict_element_read {
            ($current:expr, $destination:expr, $container:expr, $index_value:expr, $value_mode:expr) => {{
                // SAFETY: the container is an active-frame register tracked by `pins`.
                let Some((elements, length)) =
                    // SAFETY: the numeric-loop proof covers the instruction, registers, and types.
                    (unsafe { pins.for_read_dict(registers, $container) })
                else {
                    // SAFETY: `dirty` contains only active-frame numeric registers.
                    unsafe { flush(registers, &values, dirty) };
                    return NumericLoopOutcome::Deoptimize($current);
                };
                if $index_value as u64 >= length as u64 {
                    // SAFETY: `dirty` contains only active-frame numeric registers.
                    unsafe { flush(registers, &values, dirty) };
                    return NumericLoopOutcome::Deoptimize($current);
                }
                let position = $index_value as usize;

                // SAFETY: `position` is bounded by the pinned length.
                let element = unsafe { &*elements.add(position) };
                match $value_mode {
                    ArrayValueMode::Int => {
                        // SAFETY: the specialized mode proves the element is an int.
                        let value = NumericValue::int(unsafe { element.as_int_unchecked() });

                        // SAFETY: the destination is in the active numeric register window.
                        unsafe {
                            assign(
                                registers,
                                &mut values,
                                &mut dirty,
                                &mut pins,
                                $destination,
                                value,
                            )
                        };
                    }
                    ArrayValueMode::Float => {
                        // SAFETY: the specialized mode proves the element is a float.
                        let value = NumericValue::float(unsafe { element.as_float_unchecked() });

                        // SAFETY: the destination is in the active numeric register window.
                        unsafe {
                            assign(
                                registers,
                                &mut values,
                                &mut dirty,
                                &mut pins,
                                $destination,
                                value,
                            )
                        };
                    }
                    ArrayValueMode::Generic => {
                        // SAFETY: the destination is in the active numeric register window.
                        unsafe {
                            assign_array_element(
                                registers,
                                &mut values,
                                &mut dirty,
                                &mut pins,
                                $destination,
                                element,
                            )
                        }
                    }
                }
            }};
        }

        macro_rules! dict_element_write {
            ($current:expr, $container:expr, $index_value:expr, $value_register:expr) => {{
                let value_index = $value_register.index() as usize;
                if values.kind(value_index) == NumericKind::Other {
                    // SAFETY: `dirty` contains only active-frame numeric registers.
                    unsafe { flush(registers, &values, dirty) };
                    return NumericLoopOutcome::Deoptimize($current);
                }

                // SAFETY: the container is an active-frame register tracked by `pins`.
                let Some((elements, length)) =
                    // SAFETY: the numeric-loop proof covers the instruction, registers, and types.
                    (unsafe { pins.for_write_dict(registers, $container) })
                else {
                    // SAFETY: `dirty` contains only active-frame numeric registers.
                    unsafe { flush(registers, &values, dirty) };
                    return NumericLoopOutcome::Deoptimize($current);
                };
                if $index_value as u64 >= length as u64 {
                    // SAFETY: `dirty` contains only active-frame numeric registers.
                    unsafe { flush(registers, &values, dirty) };
                    return NumericLoopOutcome::Deoptimize($current);
                }
                let position = $index_value as usize;

                // SAFETY: `position` is bounded by the pinned length; the
                // assignment drops the replaced element.
                unsafe { *elements.add(position) = values.get(value_index).into_value() };
            }};
        }

        macro_rules! numeric_comparison_result {
            ($current:expr, $comparison:expr, $destination:expr, $left:expr, $right:expr) => {{
                let Some(result) = comparison_matches_numeric(
                    $comparison,
                    values.get($left.index() as usize),
                    values.get($right.index() as usize),
                ) else {
                    // SAFETY: `dirty` contains only active-frame numeric registers.
                    unsafe { flush(registers, &values, dirty) };
                    return NumericLoopOutcome::Deoptimize($current);
                };

                // SAFETY: the destination is in the active numeric register window.
                unsafe {
                    assign(
                        registers,
                        &mut values,
                        &mut dirty,
                        &mut pins,
                        $destination,
                        NumericValue::bool(result),
                    )
                };
            }};
        }

        macro_rules! int_binary_operation {
            ($current:expr, $destination:expr, $left:expr, $right:expr, $operator:literal, $operation:expr) => {{
                let left_index = $left.index() as usize;
                let right_index = $right.index() as usize;
                if values.kind(left_index) != NumericKind::Int
                    || values.kind(right_index) != NumericKind::Int
                {
                    // SAFETY: `dirty` contains only active-frame numeric registers.
                    unsafe { flush(registers, &values, dirty) };
                    return NumericLoopOutcome::Deoptimize($current);
                }

                match $operation(values.int(left_index), values.int(right_index)) {
                    Ok(result) => {
                        // SAFETY: the destination is in the active numeric register window.
                        unsafe {
                            assign(
                                registers,
                                &mut values,
                                &mut dirty,
                                &mut pins,
                                $destination,
                                NumericValue::int(result),
                            )
                        }
                    }
                    Err(fault) => {
                        // SAFETY: `dirty` contains only active-frame numeric registers.
                        unsafe { flush(registers, &values, dirty) };
                        return NumericLoopOutcome::Fault {
                            resume_ip: cursor,
                            fault,
                            operator: $operator,
                            left: $left,
                            right: Some($right),
                        };
                    }
                }
            }};
        }

        macro_rules! jump_to {
            ($target:expr) => {{
                let target = $target;
                if target == exit {
                    // SAFETY: `dirty` contains only active-frame numeric registers.
                    unsafe { flush(registers, &values, dirty) };
                    return NumericLoopOutcome::Completed;
                }
                cursor = target;
            }};
        }

        macro_rules! fused_counter_tail {
            ($current:expr, $written:expr) => {{
                // SAFETY: `cursor` points inside the verified numeric-loop body.
                let tail = unsafe { InstructionWord::read(chunk.code.as_ptr().add(cursor)) };
                if tail.kind() == InstructionKind::IntCounterLoop {
                    // SAFETY: dispatch matched the instruction tag.
                    let Instruction::IntCounterLoop {
                        comparison,
                        counter,
                        limit,
                        offset,
                        // SAFETY: the numeric-loop proof covers the instruction, registers, and types.
                    } = (unsafe { tail.decode() })
                    else {
                        // SAFETY: `decode` must return the variant selected by the tag.
                        unsafe {
                            unreachable_invariant("an instruction tag selects its own payload")
                        }
                    };

                    if relative_target(cursor, i32::from(offset.offset())) == $current {
                        let counter_value = values.int(counter.index() as usize);
                        let limit_value = values.int(limit.index() as usize);
                        let next =
                            if comparison == BytecodeComparison::LessThan && $written != counter {
                                counter_value + 1
                            } else {
                                let Some(next) = counter_value.checked_add(1) else {
                                    // SAFETY: `dirty` contains only active-frame numeric registers.
                                    unsafe { flush(registers, &values, dirty) };
                                    return NumericLoopOutcome::Fault {
                                        resume_ip: cursor + 1,
                                        fault: if counter_value >= 0 {
                                            Fault::Overflow
                                        } else {
                                            Fault::Underflow
                                        },
                                        operator: "+",
                                        left: counter,
                                        right: None,
                                    };
                                };
                                next
                            };

                        assign_existing_int(&mut values, &mut dirty, counter, next);
                        if int_ordered_comparison_matches(comparison, next, limit_value) {
                            cursor = $current;
                            continue;
                        }

                        if cursor + 1 == exit {
                            // SAFETY: `dirty` contains only active-frame numeric registers.
                            unsafe { flush(registers, &values, dirty) };
                            return NumericLoopOutcome::Completed;
                        }

                        cursor += 1;
                        continue;
                    }
                }
            }};
        }

        macro_rules! fused_array_comparison_tail {
            ($destination:expr, $value_mode:expr) => {{
                if $value_mode != ArrayValueMode::Generic && cursor != exit {
                    let tail_current = cursor;
                    // SAFETY: `cursor` points inside the verified numeric-loop body.
                    let tail = unsafe { InstructionWord::read(chunk.code.as_ptr().add(cursor)) };
                    if tail.kind() == InstructionKind::JumpUnless {
                        // SAFETY: dispatch matched the instruction tag.
                        let Instruction::JumpUnless {
                            comparison,
                            left,
                            right,
                            offset,
                            // SAFETY: the numeric-loop proof covers the instruction, registers, and types.
                        } = (unsafe { tail.decode() })
                        else {
                            // SAFETY: `decode` must return the variant selected by the tag.
                            unsafe {
                                unreachable_invariant("an instruction tag selects its own payload")
                            }
                        };

                        let left_index = left.index() as usize;
                        let right_index = right.index() as usize;
                        let matches = match $value_mode {
                            ArrayValueMode::Float
                                if left == $destination
                                    && values.kind(right_index) == NumericKind::Float =>
                            {
                                Some(float_ordered_comparison_matches(
                                    comparison,
                                    values.float(left_index),
                                    values.float(right_index),
                                ))
                            }
                            ArrayValueMode::Float
                                if right == $destination
                                    && values.kind(left_index) == NumericKind::Float =>
                            {
                                Some(float_ordered_comparison_matches(
                                    comparison,
                                    values.float(left_index),
                                    values.float(right_index),
                                ))
                            }
                            ArrayValueMode::Int
                                if left == $destination
                                    && values.kind(right_index) == NumericKind::Int =>
                            {
                                Some(int_ordered_comparison_matches(
                                    comparison,
                                    values.int(left_index),
                                    values.int(right_index),
                                ))
                            }
                            ArrayValueMode::Int
                                if right == $destination
                                    && values.kind(left_index) == NumericKind::Int =>
                            {
                                Some(int_ordered_comparison_matches(
                                    comparison,
                                    values.int(left_index),
                                    values.int(right_index),
                                ))
                            }
                            ArrayValueMode::Int
                            | ArrayValueMode::Float
                            | ArrayValueMode::Generic => comparison_matches_numeric(
                                comparison,
                                values.get(left_index),
                                values.get(right_index),
                            ),
                        };

                        if let Some(matches) = matches {
                            cursor += 1;
                            if !matches {
                                let relative = i32::from(offset.offset());
                                jump_to!(relative_target(tail_current, relative));
                            }
                            continue;
                        }
                    } else if tail.kind() == InstructionKind::AddImmediate {
                        // SAFETY: dispatch matched the instruction tag.
                        let Instruction::AddImmediate {
                            destination,
                            source,
                            immediate,
                            // SAFETY: the numeric-loop proof covers the instruction, registers, and types.
                        } = (unsafe { tail.decode() })
                        else {
                            // SAFETY: `decode` must return the variant selected by the tag.
                            unsafe {
                                unreachable_invariant("an instruction tag selects its own payload")
                            }
                        };
                        let source_index = source.index() as usize;
                        if destination != source && values.kind(source_index) == NumericKind::Int {
                            let current_value = values.int(source_index);
                            let amount = i64::from(immediate.value());
                            let Some(next) = current_value.checked_add(amount) else {
                                // SAFETY: `dirty` contains only active-frame numeric registers.
                                unsafe { flush(registers, &values, dirty) };
                                return NumericLoopOutcome::Fault {
                                    resume_ip: cursor + 1,
                                    fault: if current_value >= 0 {
                                        Fault::Overflow
                                    } else {
                                        Fault::Underflow
                                    },
                                    operator: "+",
                                    left: source,
                                    right: None,
                                };
                            };
                            // SAFETY: the destination is in the active numeric register window.
                            unsafe {
                                assign(
                                    registers,
                                    &mut values,
                                    &mut dirty,
                                    &mut pins,
                                    destination,
                                    NumericValue::int(next),
                                )
                            };
                            cursor += 1;
                            continue;
                        }
                    }
                }
            }};
        }

        macro_rules! numeric_dispatch {
            (
                $word:ident, $current:ident {
                    generic_binary {
                        $($generic_variant:ident => $generic_operation:path, $generic_operator:literal;)*
                    }
                    int_binary {
                        $($int_variant:ident => $int_operation:path, $int_operator:literal;)*
                    }
                    int_immediate {
                        $($immediate_variant:ident => $immediate_operation:path, $immediate_operator:literal;)*
                    }
                    float_binary {
                        $($float_variant:ident => $float_operator:tt;)*
                    }
                    comparison {
                        $($comparison_variant:ident => $comparison:ident;)*
                    }
                    checked_int {
                        $($checked_variant:ident => $checked_operator:literal, $checked_operation:expr;)*
                    }
                    $($rest:tt)*
                }
            ) => {
                dispatch_instruction!($word {
                    $(
                        Instruction::$generic_variant { destination, left, right } => {
                            let Some(result) = $generic_operation(
                                values.get(left.index() as usize),
                                values.get(right.index() as usize),
                            ) else {
                                // SAFETY: `dirty` contains only active-frame numeric registers.
                                unsafe { flush(registers, &values, dirty) };
                                return NumericLoopOutcome::Deoptimize($current);
                            };
                            let value = match result {
                                Ok(value) => value,
                                Err(fault) => {
                                    // SAFETY: `dirty` contains only active-frame numeric registers.
                                    unsafe { flush(registers, &values, dirty) };
                                    return NumericLoopOutcome::Fault {
                                        resume_ip: cursor,
                                        fault,
                                        operator: $generic_operator,
                                        left,
                                        right: Some(right),
                                    };
                                }
                            };
                            // SAFETY: the destination is in the active numeric register window.
                            unsafe {
                                assign(
                                    registers,
                                    &mut values,
                                    &mut dirty,
                                    &mut pins,
                                    destination,
                                    value,
                                )
                            };
                        }
                    )*
                    $(
                        Instruction::$int_variant { destination, left, right } => {
                            let result = $int_operation(
                                values.int(left.index() as usize),
                                values.int(right.index() as usize),
                            );
                            let value = match result {
                                Ok(value) => value,
                                Err(fault) => {
                                    // SAFETY: `dirty` contains only active-frame numeric registers.
                                    unsafe { flush(registers, &values, dirty) };
                                    return NumericLoopOutcome::Fault {
                                        resume_ip: cursor,
                                        fault,
                                        operator: $int_operator,
                                        left,
                                        right: Some(right),
                                    };
                                }
                            };
                            // SAFETY: the destination is in the active numeric register window.
                            unsafe {
                                assign(
                                    registers,
                                    &mut values,
                                    &mut dirty,
                                    &mut pins,
                                    destination,
                                    NumericValue::int(value),
                                )
                            };
                        }
                    )*
                    $(
                        Instruction::$immediate_variant { destination, source, immediate } => {
                            let result = $immediate_operation(
                                values.int(source.index() as usize),
                                i64::from(immediate.value()),
                            );
                            let value = match result {
                                Ok(value) => value,
                                Err(fault) => {
                                    // SAFETY: `dirty` contains only active-frame numeric registers.
                                    unsafe { flush(registers, &values, dirty) };
                                    return NumericLoopOutcome::Fault {
                                        resume_ip: cursor,
                                        fault,
                                        operator: $immediate_operator,
                                        left: source,
                                        right: None,
                                    };
                                }
                            };
                            // SAFETY: the destination is in the active numeric register window.
                            unsafe {
                                assign(
                                    registers,
                                    &mut values,
                                    &mut dirty,
                                    &mut pins,
                                    destination,
                                    NumericValue::int(value),
                                )
                            };
                        }
                    )*
                    $(
                        Instruction::$float_variant { destination, left, right } => {
                            let value = values.float(left.index() as usize)
                                $float_operator values.float(right.index() as usize);
                            // SAFETY: the destination is in the active numeric register window.
                            unsafe {
                                assign_float::<PREPARED_FLOATS>(
                                    registers,
                                    &mut values,
                                    &mut dirty,
                                    &mut pins,
                                    destination,
                                    value,
                                )
                            };
                        }
                    )*
                    $(
                        Instruction::$comparison_variant { destination, left, right } => {
                            numeric_comparison_result!(
                                $current,
                                BytecodeComparison::$comparison,
                                destination,
                                left,
                                right
                            );
                        }
                    )*
                    $(
                        Instruction::$checked_variant { destination, left, right } => {
                            int_binary_operation!(
                                $current,
                                destination,
                                left,
                                right,
                                $checked_operator,
                                $checked_operation
                            );
                        }
                    )*
                    $($rest)*
                })
            };
        }

        if body > 0 {
            let marker = body - 1;
            // SAFETY: `body - 1` is the verified numeric-loop marker.
            let word = unsafe { InstructionWord::read(chunk.code.as_ptr().add(marker)) };
            let marker_fields = match word.kind() {
                InstructionKind::IntNumericLoop => {
                    // SAFETY: dispatch matched the instruction tag.
                    let Instruction::IntNumericLoop {
                        comparison,
                        left,
                        right,
                        offset,
                    } = (unsafe { word.decode() })
                    else {
                        // SAFETY: `decode` must return the variant selected by the tag.
                        unsafe {
                            unreachable_invariant("an instruction tag selects its own payload")
                        }
                    };

                    int_comparison_matches_any(
                        comparison,
                        values.int(left.index() as usize),
                        values.int(right.index() as usize),
                    )
                    .then_some((comparison, left, right, offset))
                }
                InstructionKind::NumericLoop => {
                    // SAFETY: dispatch matched the instruction tag.
                    let Instruction::NumericLoop {
                        comparison,
                        left,
                        right,
                        offset,
                    } = (unsafe { word.decode() })
                    else {
                        // SAFETY: `decode` must return the variant selected by the tag.
                        unsafe {
                            unreachable_invariant("an instruction tag selects its own payload")
                        }
                    };

                    (comparison_matches_numeric(
                        comparison,
                        values.get(left.index() as usize),
                        values.get(right.index() as usize),
                    ) == Some(true))
                    .then_some((comparison, left, right, offset))
                }
                _ => None,
            };

            if let Some((comparison, left, right, offset)) = marker_fields {
                let marker_exit = relative_target(marker, i32::from(offset.offset()));
                // SAFETY: the marker fields and register window passed validation.
                let burst = unsafe {
                    attempt_marker_bursts(
                        chunk,
                        registers,
                        &mut values,
                        &mut dirty,
                        &mut pins,
                        marker,
                        comparison,
                        left,
                        right,
                        marker_exit,
                    )
                };

                if let Some(next_cursor) = burst {
                    jump_to!(next_cursor);
                }
            }
        }

        loop {
            debug_assert_ne!(cursor, exit);
            let current = cursor;
            cursor += 1;
            // SAFETY: `cursor` remains inside the verified numeric-loop body.
            let instruction = unsafe { InstructionWord::read(chunk.code.as_ptr().add(current)) };
            numeric_dispatch!(instruction, current {
                generic_binary {
                    Add => add, "+";
                    Subtract => subtract, "-";
                    Multiply => multiply, "*";
                }
                int_binary {
                    IntAdd => integer_add, "+";
                    IntSubtract => integer_subtract, "-";
                    IntMultiply => integer_multiply, "*";
                    IntModulo => integer_modulo, "%";
                }
                int_immediate {
                    IntMultiplyImmediate => integer_multiply, "*";
                    IntModuloImmediate => integer_modulo, "%";
                }
                float_binary {
                    FloatAdd => +;
                    FloatSubtract => -;
                    FloatMultiply => *;
                }
                comparison {
                    LessThan => LessThan;
                    LessThanOrEqual => LessThanOrEqual;
                    GreaterThan => GreaterThan;
                    GreaterThanOrEqual => GreaterThanOrEqual;
                }
                checked_int {
                    ShiftLeft => "<<", |a: i64, b: i64| {
                        if !(0..=63).contains(&b) {
                            Err(Fault::ShiftRange)
                        } else {
                            Ok(((a as u64) << b as u32) as i64)
                        }
                    };
                    IntShiftLeft => "<<", |a: i64, b: i64| {
                        if !(0..=63).contains(&b) {
                            Err(Fault::ShiftRange)
                        } else {
                            Ok(((a as u64) << b as u32) as i64)
                        }
                    };
                    ShiftRight => ">>", |a: i64, b: i64| {
                        if !(0..=63).contains(&b) {
                            Err(Fault::ShiftRange)
                        } else {
                            Ok(a >> b as u32)
                        }
                    };
                    IntShiftRight => ">>", |a: i64, b: i64| {
                        if !(0..=63).contains(&b) {
                            Err(Fault::ShiftRange)
                        } else {
                            Ok(a >> b as u32)
                        }
                    };
                    BitwiseAnd => "&", |a: i64, b: i64| Ok::<i64, Fault>(a & b);
                    IntBitwiseAnd => "&", |a: i64, b: i64| Ok::<i64, Fault>(a & b);
                    BitwiseOr => "|", |a: i64, b: i64| Ok::<i64, Fault>(a | b);
                    IntBitwiseOr => "|", |a: i64, b: i64| Ok::<i64, Fault>(a | b);
                    BitwiseXor => "^", |a: i64, b: i64| Ok::<i64, Fault>(a ^ b);
                    IntBitwiseXor => "^", |a: i64, b: i64| Ok::<i64, Fault>(a ^ b);
                }
                Instruction::LoadConstant {
                    destination,
                    constant,
                } => {
                    let value = match &chunk.constants[constant.index() as usize] {
                        Literal::Int(value) => NumericValue::int(*value),
                        Literal::Float(value) => NumericValue::float(*value),
                        literal @ Literal::String(_) => {
                            // SAFETY: the destination is in the active numeric register window.
                            unsafe {
                                load_string_constant(
                                    literal,
                                    registers,
                                    &mut values,
                                    &mut dirty,
                                    &mut pins,
                                    destination,
                                )
                            };
                            continue;
                        }
                        _ => {
                            // SAFETY: numeric-loop formation admits only supported constants.
                            unsafe {
                                unreachable_invariant(
                                    "a numeric loop contains only numeric constants",
                                )
                            }
                        }
                    };
                    // SAFETY: the destination is in the active numeric register window.
                    unsafe {
                        assign(
                            registers,
                            &mut values,
                            &mut dirty,
                            &mut pins,
                            destination,
                            value,
                        )
                    };
                }
                Instruction::StringLength {
                    destination,
                    source,
                } => {
                    // SAFETY: both operands are in the active numeric register window.
                    let handled = unsafe {
                        string_length_operation(
                            registers,
                            &mut values,
                            &mut dirty,
                            &mut pins,
                            destination,
                            source,
                        )
                    };
                    if !handled {
                        // SAFETY: `dirty` contains only active-frame numeric registers.
                        unsafe { flush(registers, &values, dirty) };
                        return NumericLoopOutcome::Deoptimize(current);
                    }
                }
                Instruction::LoadInt {
                    destination,
                    immediate,
                } => {
                    // SAFETY: the destination is in the active numeric register window.
                    unsafe {
                        assign(
                            registers,
                            &mut values,
                            &mut dirty,
                            &mut pins,
                            destination,
                            NumericValue::int(i64::from(immediate.value())),
                        )
                    }
                }
                Instruction::Move {
                    destination,
                    source,
                } => {
                    let value = values.get(source.index() as usize);
                    if value.kind == NumericKind::Other {
                        // SAFETY: both operands are in the active numeric register window.
                        unsafe {
                            move_other_register(
                                registers,
                                &mut values,
                                &mut dirty,
                                &mut pins,
                                destination,
                                source,
                            )
                        };
                        if vec_append_targets & (1u64 << destination.index()) != 0
                            && let Some(vector) =
                                // SAFETY: the destination is an active-frame register.
                                unsafe { &*registers.add(destination.index() as usize) }.as_vec()
                        {
                            vector.invalidate_type_check();
                        }
                        continue;
                    }
                    // SAFETY: the destination is in the active numeric register window.
                    unsafe {
                        assign(
                            registers,
                            &mut values,
                            &mut dirty,
                            &mut pins,
                            destination,
                            value,
                        )
                    };
                }
                Instruction::IntAddAssign { target, source } => {
                    let result = integer_add(
                        values.int(target.index() as usize),
                        values.int(source.index() as usize),
                    );
                    let value = match result {
                        Ok(value) => value,
                        Err(fault) => {
                            // SAFETY: `dirty` contains only active-frame numeric registers.
                            unsafe { flush(registers, &values, dirty) };
                            return NumericLoopOutcome::Fault {
                                resume_ip: cursor,
                                fault,
                                operator: "+",
                                left: target,
                                right: Some(source),
                            };
                        }
                    };
                    assign_existing_int(
                        &mut values,
                        &mut dirty,
                        target,
                        value,
                    );
                    fused_counter_tail!(current, target);
                }
                Instruction::AddImmediate {
                    destination,
                    source,
                    immediate,
                } => {
                    let amount = i64::from(immediate.value());
                    if destination == source
                        && values.kind(source.index() as usize) == NumericKind::Int
                    {
                        let current_value = values.int(source.index() as usize);
                        let Some(next) = current_value.checked_add(amount) else {
                            // SAFETY: `dirty` contains only active-frame numeric registers.
                            unsafe { flush(registers, &values, dirty) };
                            return NumericLoopOutcome::Fault {
                                resume_ip: cursor,
                                fault: if current_value >= 0 {
                                    Fault::Overflow
                                } else {
                                    Fault::Underflow
                                },
                                operator: "+",
                                left: source,
                                right: None,
                            };
                        };
                        assign_existing_int(
                            &mut values,
                            &mut dirty,
                            destination,
                            next,
                        );
                        fused_counter_tail!(current, destination);
                        continue;
                    }
                    let Some(result) =
                        add(values.get(source.index() as usize), NumericValue::int(amount))
                    else {
                        // SAFETY: the numeric-loop proof covers the instruction, registers, and types.
                        unsafe { flush(registers, &values, dirty) };
                        return NumericLoopOutcome::Deoptimize(current);
                    };
                    let value = match result {
                        Ok(value) => value,
                        Err(fault) => {
                            // SAFETY: the numeric-loop proof covers the instruction, registers, and types.
                            unsafe { flush(registers, &values, dirty) };
                            return NumericLoopOutcome::Fault {
                                resume_ip: cursor,
                                fault,
                                operator: "+",
                                left: source,
                                right: None,
                            };
                        }
                    };
                    // SAFETY: the numeric-loop proof covers the instruction, registers, and types.
                    unsafe { assign(registers, &mut values, &mut dirty, &mut pins, destination, value) };
                }
                Instruction::SubtractImmediate {
                    destination,
                    source,
                    immediate,
                } => {
                    let amount = -i64::from(immediate.value());
                    if destination == source
                        && values.kind(source.index() as usize) == NumericKind::Int
                    {
                        let current_value = values.int(source.index() as usize);
                        let Some(next) = current_value.checked_add(amount) else {
                            // SAFETY: the numeric-loop proof covers the instruction, registers, and types.
                            unsafe { flush(registers, &values, dirty) };
                            return NumericLoopOutcome::Fault {
                                resume_ip: cursor,
                                fault: if current_value >= 0 {
                                    Fault::Overflow
                                } else {
                                    Fault::Underflow
                                },
                                operator: "-",
                                left: source,
                                right: None,
                            };
                        };
                        assign_existing_int(&mut values, &mut dirty, destination, next);
                        continue;
                    }
                    let Some(result) =
                        add(values.get(source.index() as usize), NumericValue::int(amount))
                    else {
                        // SAFETY: the numeric-loop proof covers the instruction, registers, and types.
                        unsafe { flush(registers, &values, dirty) };
                        return NumericLoopOutcome::Deoptimize(current);
                    };
                    let value = match result {
                        Ok(value) => value,
                        Err(fault) => {
                            // SAFETY: the numeric-loop proof covers the instruction, registers, and types.
                            unsafe { flush(registers, &values, dirty) };
                            return NumericLoopOutcome::Fault {
                                resume_ip: cursor,
                                fault,
                                operator: "-",
                                left: source,
                                right: None,
                            };
                        }
                    };
                    // SAFETY: the numeric-loop proof covers the instruction, registers, and types.
                    unsafe { assign(registers, &mut values, &mut dirty, &mut pins, destination, value) };
                }
                Instruction::Squares {
                    first_destination,
                    first_source,
                    second_source,
                } => {
                    let Some(first) = multiply(
                        values.get(first_source.index() as usize),
                        values.get(first_source.index() as usize),
                    ) else {
                        // SAFETY: the numeric-loop proof covers the instruction, registers, and types.
                        unsafe { flush(registers, &values, dirty) };
                        return NumericLoopOutcome::Deoptimize(current);
                    };
                    let first = match first {
                        Ok(value) => value,
                        Err(fault) => {
                            // SAFETY: the numeric-loop proof covers the instruction, registers, and types.
                            unsafe { flush(registers, &values, dirty) };
                            return NumericLoopOutcome::Fault {
                                resume_ip: cursor,
                                fault,
                                operator: "*",
                                left: first_source,
                                right: Some(first_source),
                            };
                        }
                    };
                    // SAFETY: the numeric-loop proof covers the instruction, registers, and types.
                    unsafe { assign(registers, &mut values, &mut dirty, &mut pins, first_destination, first) };

                    let Some(second) = multiply(
                        values.get(second_source.index() as usize),
                        values.get(second_source.index() as usize),
                    ) else {
                        // SAFETY: the numeric-loop proof covers the instruction, registers, and types.
                        unsafe { flush(registers, &values, dirty) };
                        return NumericLoopOutcome::Deoptimize(current);
                    };
                    let second = match second {
                        Ok(value) => value,
                        Err(fault) => {
                            // SAFETY: the numeric-loop proof covers the instruction, registers, and types.
                            unsafe { flush(registers, &values, dirty) };
                            return NumericLoopOutcome::Fault {
                                resume_ip: cursor,
                                fault,
                                operator: "*",
                                left: second_source,
                                right: Some(second_source),
                            };
                        }
                    };
                    // SAFETY: the numeric-loop proof covers the instruction, registers, and types.
                    unsafe {
                        assign(registers, &mut values, &mut dirty, &mut pins, Register::new(first_destination.index() + 1),
                            second,
                        )
                    };
                }
                Instruction::FloatSquares {
                    first_destination,
                    first_source,
                    second_source,
                } => {
                    let first = values.float(first_source.index() as usize);
                    let second = values.float(second_source.index() as usize);
                    // SAFETY: the numeric-loop proof covers the instruction, registers, and types.
                    unsafe {
                        assign_float::<PREPARED_FLOATS>(registers, &mut values, &mut dirty, &mut pins, first_destination,
                            first * first,
                        );
                        assign_float::<PREPARED_FLOATS>(registers, &mut values, &mut dirty, &mut pins, Register::new(first_destination.index() + 1),
                            second * second,
                        );
                    }
                }
                Instruction::FloatSquaresSum {
                    first_destination,
                    first_source,
                    second_source,
                } => {
                    let first = values.float(first_source.index() as usize);
                    let second = values.float(second_source.index() as usize);
                    let first_square = first * first;
                    let second_square = second * second;
                    let sum = first_square + second_square;
                    // SAFETY: the numeric-loop proof covers the instruction, registers, and types.
                    unsafe {
                        assign_float::<PREPARED_FLOATS>(registers, &mut values, &mut dirty, &mut pins, Register::new(first_destination.index() + 1),
                            first_square,
                        );
                        assign_float::<PREPARED_FLOATS>(registers, &mut values, &mut dirty, &mut pins, Register::new(first_destination.index() + 2),
                            second_square,
                        );
                        assign_float::<PREPARED_FLOATS>(registers, &mut values, &mut dirty, &mut pins, first_destination,
                            sum,
                        );
                    }
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
                    let first = values.float(first_source.index() as usize);
                    let second = values.float(second_source.index() as usize);
                    let first_square = first * first;
                    let second_square = second * second;
                    let sum = first_square + second_square;
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
                    // SAFETY: the numeric-loop proof covers the instruction, registers, and types.
                    unsafe {
                        assign_float::<PREPARED_FLOATS>(registers, &mut values, &mut dirty, &mut pins, first_square_destination,
                            first_square,
                        );
                        assign_float::<PREPARED_FLOATS>(registers, &mut values, &mut dirty, &mut pins, second_square_destination,
                            second_square,
                        );
                        assign_float::<PREPARED_FLOATS>(registers, &mut values, &mut dirty, &mut pins, sum_destination,
                            sum,
                        );
                    }
                    if !float_ordered_comparison_matches(comparison, sum, constant) {
                        let relative = offset.offset();
                        jump_to!(relative_target(current, relative));
                    }
                }
                Instruction::FloatMultiplyConstant {
                    destination,
                    source,
                    constant,
                } => {
                    let source = values.float(source.index() as usize);
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
                    // SAFETY: the numeric-loop proof covers the instruction, registers, and types.
                    unsafe {
                        assign_float::<PREPARED_FLOATS>(registers, &mut values, &mut dirty, &mut pins, destination,
                            source * constant,
                        )
                    };
                }
                Instruction::FloatDifferenceAdd {
                    destination,
                    first_operand,
                    addend,
                } => {
                    let left = values.float(first_operand.index() as usize);
                    let right = values.float(first_operand.index() as usize + 1);
                    let addend = values.float(addend.index() as usize);
                    let difference = left - right;
                    // SAFETY: the numeric-loop proof covers the instruction, registers, and types.
                    unsafe {
                        assign_float::<PREPARED_FLOATS>(registers, &mut values, &mut dirty, &mut pins, destination,
                            difference + addend,
                        )
                    };
                }
                Instruction::FloatScaleProductAdd {
                    destination,
                    first_operand,
                    constant,
                } => {
                    let first = first_operand.index() as usize;
                    let addend = values.float(first);
                    let left = values.float(first + 1);
                    let right = values.float(first + 2);
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
                    // SAFETY: the numeric-loop proof covers the instruction, registers, and types.
                    unsafe {
                        assign_float::<PREPARED_FLOATS>(registers, &mut values, &mut dirty, &mut pins, destination,
                            product + addend,
                        )
                    };
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
                    let first = first_operand.index() as usize;
                    let addend = values.float(first);
                    let left = values.float(first + 1);
                    let right = values.float(first + 2);
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
                    // SAFETY: the numeric-loop proof covers the instruction, registers, and types.
                    unsafe {
                        assign_float::<PREPARED_FLOATS>(registers, &mut values, &mut dirty, &mut pins, first_destination,
                            product + addend,
                        )
                    };

                    let second = second_operand.index() as usize;
                    let left = values.float(second);
                    let right = values.float(second + 1);
                    let addend = values.float(second_addend.index() as usize);
                    let difference = left - right;
                    // SAFETY: the numeric-loop proof covers the instruction, registers, and types.
                    unsafe {
                        assign_float::<PREPARED_FLOATS>(registers, &mut values, &mut dirty, &mut pins, second_destination,
                            difference + addend,
                        )
                    };
                }
                Instruction::JumpUnless {
                    comparison,
                    left,
                    right,
                    offset,
                } => {
                    let Some(matches) = comparison_matches_numeric(
                        comparison,
                        values.get(left.index() as usize),
                        values.get(right.index() as usize),
                    ) else {
                        // SAFETY: the numeric-loop proof covers the instruction, registers, and types.
                        unsafe { flush(registers, &values, dirty) };
                        return NumericLoopOutcome::Deoptimize(current);
                    };
                    if !matches {
                        let relative = i32::from(offset.offset());
                        jump_to!(relative_target(current, relative));
                    }
                }
                Instruction::JumpUnlessConstant {
                    comparison,
                    source,
                    constant,
                    offset,
                } => {
                    let constant = match chunk.constants[constant.index() as usize] {
                        Literal::Int(value) => NumericValue::int(value),
                        Literal::Float(value) => NumericValue::float(value),
                        // SAFETY: the surrounding invariant makes this path unreachable.
                        _ => unsafe {
                            unreachable_invariant("a numeric loop contains numeric constants")
                        },
                    };
                    let Some(matches) = comparison_matches_numeric(
                        comparison,
                        values.get(source.index() as usize),
                        constant,
                    ) else {
                        // SAFETY: the numeric-loop proof covers the instruction, registers, and types.
                        unsafe { flush(registers, &values, dirty) };
                        return NumericLoopOutcome::Deoptimize(current);
                    };
                    if !matches {
                        let relative = i32::from(offset.offset());
                        jump_to!(relative_target(current, relative));
                    }
                }
                Instruction::Jump { offset } => {
                    let relative = offset.offset();
                    jump_to!(relative_target(current, relative));
                }
                Instruction::NumericRegionJump { offset } => {
                    let relative = offset.offset();
                    let target = relative_target(current, relative);
                    if !PREPARED_FLOATS && current != scan_burst_miss {
                        // SAFETY: the numeric-loop proof covers the instruction, registers, and types.
                        let burst = unsafe {
                            attempt_region_bursts(
                                chunk,
                                registers,
                                &mut values,
                                &mut dirty,
                                target,
                                current,
                            )
                        };
                        if let Some(next_cursor) = burst {
                            jump_to!(next_cursor);
                        } else {
                            scan_burst_miss = current;
                            jump_to!(target);
                        }
                    } else {
                        jump_to!(target);
                    }
                }
                Instruction::CounterLoop {
                    comparison,
                    counter,
                    limit,
                    offset,
                } => {
                    let Some(result) = step_counter(
                        comparison,
                        values.get(counter.index() as usize),
                        values.get(limit.index() as usize),
                    )
                    else {
                        // SAFETY: the numeric-loop proof covers the instruction, registers, and types.
                        unsafe { flush(registers, &values, dirty) };
                        return NumericLoopOutcome::Deoptimize(current);
                    };
                    let (value, matches) = match result {
                        Ok(result) => result,
                        Err(fault) => {
                            // SAFETY: the numeric-loop proof covers the instruction, registers, and types.
                            unsafe { flush(registers, &values, dirty) };
                            return NumericLoopOutcome::Fault {
                                resume_ip: cursor,
                                fault,
                                operator: "+",
                                left: counter,
                                right: None,
                            };
                        }
                    };
                    assign_existing_numeric(&mut values, &mut dirty, counter, value);
                    if matches {
                        jump_to!(relative_target(current, i32::from(offset.offset())));
                    } else if cursor == exit {
                        // SAFETY: the numeric-loop proof covers the instruction, registers, and types.
                        unsafe { flush(registers, &values, dirty) };
                        return NumericLoopOutcome::Completed;
                    }
                }
                Instruction::IntCounterLoop {
                    comparison,
                    counter,
                    limit,
                    offset,
                } => {
                    let counter_value = values.int(counter.index() as usize);
                    let limit_value = values.int(limit.index() as usize);
                    let next = if comparison == BytecodeComparison::LessThan {
                        counter_value + 1
                    } else {
                        let Some(next) = counter_value.checked_add(1) else {
                            // SAFETY: the numeric-loop proof covers the instruction, registers, and types.
                            unsafe { flush(registers, &values, dirty) };
                            return NumericLoopOutcome::Fault {
                                resume_ip: cursor,
                                fault: if counter_value >= 0 {
                                    Fault::Overflow
                                } else {
                                    Fault::Underflow
                                },
                                operator: "+",
                                left: counter,
                                right: None,
                            };
                        };
                        next
                    };
                    assign_existing_int(
                        &mut values,
                        &mut dirty,
                        counter,
                        next,
                    );
                    if int_ordered_comparison_matches(comparison, next, limit_value) {
                        let body_target = relative_target(current, i32::from(offset.offset()));
                        if !PREPARED_FLOATS && current != int_body_burst_miss {
                            // SAFETY: the numeric-loop proof covers the instruction, registers, and types.
                            let burst = unsafe {
                                try_int_body_burst(
                                    chunk,
                                    &mut values,
                                    &mut dirty,
                                    body_target,
                                    current,
                                    comparison,
                                    counter,
                                    limit,
                                    exit,
                                )
                            };
                            if let Some(next_cursor) = burst {
                                jump_to!(next_cursor);
                                continue;
                            }
                            int_body_burst_miss = current;
                        }
                        if !PREPARED_FLOATS && current != vec_append_burst_miss {
                            // SAFETY: the numeric-loop proof covers the instruction, registers, and types.
                            let burst = unsafe {
                                try_vec_append_burst(
                                    chunk,
                                    registers,
                                    &mut values,
                                    &mut dirty,
                                    body_target,
                                    current,
                                    comparison,
                                    counter,
                                    limit,
                                    exit,
                                )
                            };
                            if let Some(next_cursor) = burst {
                                jump_to!(next_cursor);
                                continue;
                            }
                            vec_append_burst_miss = current;
                        }
                        if !PREPARED_FLOATS && current != indexed_int_body_burst_miss {
                            // SAFETY: the numeric-loop proof covers the instruction, registers, and types.
                            let burst = unsafe {
                                try_indexed_int_body_burst(
                                    chunk,
                                    registers,
                                    &mut values,
                                    &mut dirty,
                                    &mut pins,
                                    body_target,
                                    current,
                                    comparison,
                                    counter,
                                    limit,
                                    exit,
                                )
                            };
                            if let Some(next_cursor) = burst {
                                jump_to!(next_cursor);
                                continue;
                            }
                            indexed_int_body_burst_miss = current;
                        }
                        if !PREPARED_FLOATS && current != scan_burst_miss {
                            // SAFETY: the numeric-loop proof covers the instruction, registers, and types.
                            let burst = unsafe {
                                try_scan_burst(
                                    chunk,
                                    registers,
                                    &mut values,
                                    &mut dirty,
                                    &mut pins,
                                    current,
                                    body_target,
                                    comparison,
                                    counter,
                                    limit,
                                )
                            };
                            if let Some(next_cursor) = burst {
                                jump_to!(next_cursor);
                            } else {
                                scan_burst_miss = current;
                                jump_to!(body_target);
                            }
                        } else if !PREPARED_FLOATS && current != string_slice_burst_miss {
                            // SAFETY: the numeric-loop proof covers the instruction, registers, and types.
                            let burst = unsafe {
                                try_string_slice_burst(
                                    chunk,
                                    registers,
                                    &mut values,
                                    &mut dirty,
                                    body_target,
                                    current,
                                    comparison,
                                    counter,
                                    limit,
                                    exit,
                                )
                            };
                            if let Some(next_cursor) = burst {
                                jump_to!(next_cursor);
                            } else {
                                string_slice_burst_miss = current;
                                jump_to!(body_target);
                            }
                        } else {
                            jump_to!(body_target);
                        }
                    } else if cursor == exit {
                        // SAFETY: the numeric-loop proof covers the instruction, registers, and types.
                        unsafe { flush(registers, &values, dirty) };
                        return NumericLoopOutcome::Completed;
                    }
                }
                Instruction::VecIndexGet {
                    destination,
                    container,
                    index,
                    value_mode,
                } => {
                    let index_value = values.int(index.index() as usize);
                    vec_element_read!(current, destination, container, index_value, value_mode);
                    fused_array_comparison_tail!(destination, value_mode);
                }
                Instruction::IndexGet {
                    destination,
                    container,
                    index,
                } => {
                    let index_register = index.index() as usize;
                    if values.kind(index_register) != NumericKind::Int {
                        // SAFETY: the numeric-loop proof covers the instruction, registers, and types.
                        unsafe { flush(registers, &values, dirty) };
                        return NumericLoopOutcome::Deoptimize(current);
                    }
                    let index_value = values.int(index_register);
                    vec_element_read!(
                        current,
                        destination,
                        container,
                        index_value,
                        ArrayValueMode::Generic
                    );
                }
                Instruction::VecIndexSet {
                    container,
                    index,
                    value,
                } => {
                    let index_value = values.int(index.index() as usize);
                    vec_element_write!(current, container, index_value, value);
                }
                Instruction::VecAppend { container, value } => {
                    let value = values.get(value.index() as usize);
                    if value.kind == NumericKind::Other {
                        // SAFETY: the numeric-loop proof covers the instruction, registers, and types.
                        unsafe { flush(registers, &values, dirty) };
                        return NumericLoopOutcome::Deoptimize(current);
                    }
                    let Some(vector) =
                        // SAFETY: the numeric-loop proof covers the instruction, registers, and types.
                        (unsafe { &mut *registers.add(container.index() as usize) }).as_vec_mut()
                    else {
                        // SAFETY: the surrounding invariant makes this path unreachable.
                        unsafe {
                            unreachable_invariant(
                                "a specialized vec append has a vec container",
                            )
                        }
                    };
                    vector
                        .make_mut()
                        .push_after_type_check_invalidation(value.into_value());
                    pins.invalidate(container.index() as usize);
                }
                Instruction::IndexSet {
                    container,
                    index,
                    value,
                } => {
                    let index_register = index.index() as usize;
                    if values.kind(index_register) != NumericKind::Int {
                        // SAFETY: the numeric-loop proof covers the instruction, registers, and types.
                        unsafe { flush(registers, &values, dirty) };
                        return NumericLoopOutcome::Deoptimize(current);
                    }
                    let index_value = values.int(index_register);
                    vec_element_write!(current, container, index_value, value);
                }
                Instruction::DictIndexGetIntKey {
                    destination,
                    container,
                    index,
                    value_mode,
                } => {
                    let index_value = values.int(index.index() as usize);
                    dict_element_read!(current, destination, container, index_value, value_mode);
                    fused_array_comparison_tail!(destination, value_mode);
                }
                Instruction::DictIndexSetIntKey {
                    container,
                    index,
                    value,
                } => {
                    let index_value = values.int(index.index() as usize);
                    dict_element_write!(current, container, index_value, value);
                }
                Instruction::Equal {
                    destination,
                    left,
                    right,
                } => {
                    let Some(result) = equals_numeric(
                        values.get(left.index() as usize),
                        values.get(right.index() as usize),
                    ) else {
                        // SAFETY: the numeric-loop proof covers the instruction, registers, and types.
                        unsafe { flush(registers, &values, dirty) };
                        return NumericLoopOutcome::Deoptimize(current);
                    };
                    // SAFETY: the numeric-loop proof covers the instruction, registers, and types.
                    unsafe {
                        assign(registers, &mut values, &mut dirty, &mut pins, destination,
                            NumericValue::bool(result),
                        )
                    };
                }
                Instruction::NotEqual {
                    destination,
                    left,
                    right,
                } => {
                    let Some(result) = equals_numeric(
                        values.get(left.index() as usize),
                        values.get(right.index() as usize),
                    ) else {
                        // SAFETY: the numeric-loop proof covers the instruction, registers, and types.
                        unsafe { flush(registers, &values, dirty) };
                        return NumericLoopOutcome::Deoptimize(current);
                    };
                    // SAFETY: the numeric-loop proof covers the instruction, registers, and types.
                    unsafe {
                        assign(registers, &mut values, &mut dirty, &mut pins, destination,
                            NumericValue::bool(!result),
                        )
                    };
                }
                Instruction::JumpIfFalse { condition, offset } => {
                    let index = condition.index() as usize;
                    if values.kind(index) != NumericKind::Bool {
                        // SAFETY: the numeric-loop proof covers the instruction, registers, and types.
                        unsafe { flush(registers, &values, dirty) };
                        return NumericLoopOutcome::Deoptimize(current);
                    }
                    if values.get(index).bits == 0 {
                        let relative = offset.offset();
                        jump_to!(relative_target(current, relative));
                    }
                }
                Instruction::JumpIfTrue { condition, offset } => {
                    let index = condition.index() as usize;
                    if values.kind(index) != NumericKind::Bool {
                        // SAFETY: the numeric-loop proof covers the instruction, registers, and types.
                        unsafe { flush(registers, &values, dirty) };
                        return NumericLoopOutcome::Deoptimize(current);
                    }
                    if values.get(index).bits != 0 {
                        let relative = offset.offset();
                        jump_to!(relative_target(current, relative));
                    }
                }
                Instruction::Concatenate {
                    destination,
                    left,
                    right,
                } => {
                    // SAFETY: the numeric-loop proof covers the instruction, registers, and types.
                    let appended = unsafe {
                        concatenate_append(registers, &values, destination, left, right)
                    };
                    if !appended {
                        // SAFETY: the numeric-loop proof covers the instruction, registers, and types.
                        unsafe { flush(registers, &values, dirty) };
                        return NumericLoopOutcome::Deoptimize(current);
                    }
                }
                Instruction::LoadTrue { destination } => {
                    // SAFETY: the numeric-loop proof covers the instruction, registers, and types.
                    unsafe {
                        assign(registers, &mut values, &mut dirty, &mut pins, destination,
                            NumericValue::bool(true),
                        )
                    };
                }
                Instruction::LoadFalse { destination } => {
                    // SAFETY: the numeric-loop proof covers the instruction, registers, and types.
                    unsafe {
                        assign(registers, &mut values, &mut dirty, &mut pins, destination,
                            NumericValue::bool(false),
                        )
                    };
                }
                Instruction::CheckDefined { subject, name: _ } => {
                    if values.kind(subject.index() as usize) == NumericKind::Other {
                        // SAFETY: the numeric-loop proof covers the instruction, registers, and types.
                        unsafe { flush(registers, &values, dirty) };
                        return NumericLoopOutcome::Deoptimize(current);
                    }
                }
                Instruction::IntJumpUnless {
                    comparison,
                    left,
                    right,
                    offset,
                } => {
                    let left_value = values.int(left.index() as usize);
                    let right_value = values.int(right.index() as usize);
                    if !int_comparison_matches_any(comparison, left_value, right_value) {
                        let relative = i32::from(offset.offset());
                        jump_to!(relative_target(current, relative));
                    }
                }
                Instruction::IntJumpUnlessImmediate {
                    comparison,
                    source,
                    immediate,
                    offset,
                } => {
                    let source_value = values.int(source.index() as usize);
                    if !int_comparison_matches_any(
                        comparison,
                        source_value,
                        i64::from(immediate.value()),
                    ) {
                        let relative = i32::from(offset.offset());
                        jump_to!(relative_target(current, relative));
                    }
                }
                Instruction::IntNumericLoop {
                    comparison,
                    left,
                    right,
                    offset,
                } => {
                    let left_value = values.int(left.index() as usize);
                    let right_value = values.int(right.index() as usize);
                    if !int_comparison_matches_any(comparison, left_value, right_value) {
                        jump_to!(relative_target(current, i32::from(offset.offset())));
                    } else {
                        if current != marker_burst_miss {
                            let marker_exit =
                                relative_target(current, i32::from(offset.offset()));
                            // SAFETY: the numeric-loop proof covers the instruction, registers, and types.
                            let burst = unsafe {
                                attempt_marker_bursts(
                                    chunk,
                                    registers,
                                    &mut values,
                                    &mut dirty,
                                    &mut pins,
                                    current,
                                    comparison,
                                    left,
                                    right,
                                    marker_exit,
                                )
                            };
                            if let Some(next_cursor) = burst {
                                jump_to!(next_cursor);
                            } else {
                                marker_burst_miss = current;
                            }
                        }
                    }
                }
                Instruction::NumericLoop {
                    comparison,
                    left,
                    right,
                    offset,
                } => {
                    let Some(matches) = comparison_matches_numeric(
                        comparison,
                        values.get(left.index() as usize),
                        values.get(right.index() as usize),
                    ) else {
                        // SAFETY: the numeric-loop proof covers the instruction, registers, and types.
                        unsafe { flush(registers, &values, dirty) };
                        return NumericLoopOutcome::Deoptimize(current);
                    };
                    if !matches {
                        jump_to!(relative_target(current, i32::from(offset.offset())));
                    } else {
                        if current != marker_burst_miss {
                            let marker_exit =
                                relative_target(current, i32::from(offset.offset()));
                            // SAFETY: the numeric-loop proof covers the instruction, registers, and types.
                            let burst = unsafe {
                                attempt_marker_bursts(
                                    chunk,
                                    registers,
                                    &mut values,
                                    &mut dirty,
                                    &mut pins,
                                    current,
                                    comparison,
                                    left,
                                    right,
                                    marker_exit,
                                )
                            };
                            if let Some(next_cursor) = burst {
                                jump_to!(next_cursor);
                            } else {
                                marker_burst_miss = current;
                            }
                        }
                    }
                }
                Instruction::PreparedIntNumericLoop { descriptor, offset } => {
                    let PreparedIntLoopDescriptor {
                        comparison,
                        counter,
                        limit,
                        ..
                    } = *chunk.prepared_int_loop_descriptor(descriptor);
                    let counter_value = values.int(counter.index() as usize);
                    let limit_value = values.int(limit.index() as usize);
                    if !int_comparison_matches_any(comparison, counter_value, limit_value) {
                        jump_to!(relative_target(current, i32::from(offset.offset())));
                    }
                }
                _ => {
                    // SAFETY: the numeric-loop proof covers the instruction, registers, and types.
                    unsafe { flush(registers, &values, dirty) };
                    return NumericLoopOutcome::Deoptimize(current);
                }
            });
        }
    }
}

/// # Safety
///
/// `registers` must be the live register window for `chunk` and the shadow
/// state must describe it.
#[expect(
    clippy::too_many_arguments,
    reason = "the burst borrows every piece of executor state"
)]
#[inline(never)]
unsafe fn try_fill_burst(
    chunk: &Chunk,
    registers: *mut Value,
    values: &mut NumericRegisters,
    dirty: &mut u64,
    pins: &mut Pins,
    marker: usize,
    comparison: BytecodeComparison,
    left: Register,
    right: Register,
    marker_exit: usize,
) -> Option<usize> {
    let body = marker + 1;
    // SAFETY: the numeric-loop proof covers the instruction, registers, and types.
    let first = unsafe { InstructionWord::read(chunk.code.as_ptr().add(body)) };
    if first.kind() != InstructionKind::LoadInt {
        return None;
    }
    let Instruction::LoadInt {
        destination: fill_register,
        immediate,
        // SAFETY: the numeric-loop proof covers the instruction, registers, and types.
    } = (unsafe { first.decode() })
    else {
        return None;
    };
    // SAFETY: the numeric-loop proof covers the instruction, registers, and types.
    let second = unsafe { InstructionWord::read(chunk.code.as_ptr().add(body + 1)) };
    let (container, index_register, value_register) = match second.kind() {
        InstructionKind::IndexSet => {
            let Instruction::IndexSet {
                container,
                index,
                value,
                // SAFETY: the numeric-loop proof covers the instruction, registers, and types.
            } = (unsafe { second.decode() })
            else {
                return None;
            };
            (container, index, value)
        }
        InstructionKind::VecIndexSet => {
            let Instruction::VecIndexSet {
                container,
                index,
                value,
                // SAFETY: the numeric-loop proof covers the instruction, registers, and types.
            } = (unsafe { second.decode() })
            else {
                return None;
            };
            (container, index, value)
        }
        _ => return None,
    };
    if index_register != left || value_register != fill_register {
        return None;
    }
    // SAFETY: the numeric-loop proof covers the instruction, registers, and types.
    let third = unsafe { InstructionWord::read(chunk.code.as_ptr().add(body + 2)) };
    if third.kind() != InstructionKind::IntAddAssign {
        return None;
    }
    // SAFETY: the numeric-loop proof covers the instruction, registers, and types.
    let Instruction::IntAddAssign { target, source } = (unsafe { third.decode() }) else {
        return None;
    };
    if target != left {
        return None;
    }
    // SAFETY: the numeric-loop proof covers the instruction, registers, and types.
    let fourth = unsafe { InstructionWord::read(chunk.code.as_ptr().add(body + 3)) };
    if fourth.kind() != InstructionKind::Jump {
        return None;
    }
    // SAFETY: the numeric-loop proof covers the instruction, registers, and types.
    let Instruction::Jump { offset: back } = (unsafe { fourth.decode() }) else {
        return None;
    };
    if relative_target(body + 3, back.offset()) != marker
        || values.kind(left.index() as usize) != NumericKind::Int
        || values.kind(right.index() as usize) != NumericKind::Int
        || values.kind(source.index() as usize) != NumericKind::Int
    {
        return None;
    }
    // SAFETY: the numeric-loop proof covers the instruction, registers, and types.
    let (elements, length) = (unsafe { pins.for_write(registers, container) })?;

    let fill_value = i64::from(immediate.value());
    let step = values.int(source.index() as usize);
    let limit_value = values.int(right.index() as usize);
    let mut position = values.int(left.index() as usize);
    let mut budget: u32 = BATCH_ITERATION_LIMIT;
    loop {
        if position as u64 >= length as u64 {
            assign_existing_int(values, dirty, left, position);
            return Some(body);
        }
        // SAFETY: bounded above; the assignment drops the replaced element.
        unsafe { *elements.add(position as usize) = Value::int(fill_value) };
        let Some(next) = position.checked_add(step) else {
            assign_existing_int(values, dirty, left, position);
            // SAFETY: the numeric-loop proof covers the instruction, registers, and types.
            unsafe {
                assign(
                    registers,
                    values,
                    dirty,
                    pins,
                    fill_register,
                    NumericValue::int(fill_value),
                )
            };
            return Some(body + 2);
        };
        budget -= 1;
        if budget == 0 {
            assign_existing_int(values, dirty, left, next);
            // SAFETY: the numeric-loop proof covers the instruction, registers, and types.
            unsafe {
                assign(
                    registers,
                    values,
                    dirty,
                    pins,
                    fill_register,
                    NumericValue::int(fill_value),
                )
            };
            return Some(body + 3);
        }
        if !int_comparison_matches_any(comparison, next, limit_value) {
            assign_existing_int(values, dirty, left, next);
            // SAFETY: the numeric-loop proof covers the instruction, registers, and types.
            unsafe {
                assign(
                    registers,
                    values,
                    dirty,
                    pins,
                    fill_register,
                    NumericValue::int(fill_value),
                )
            };
            return Some(marker_exit);
        }
        position = next;
    }
}

#[inline(never)]
unsafe fn load_string_constant(
    literal: &Literal,
    registers: *mut Value,
    values: &mut NumericRegisters,
    dirty: &mut u64,
    pins: &mut Pins,
    destination: Register,
) {
    let index = destination.index() as usize;
    // SAFETY: the destination register is in the live window; the write
    // drops its previous occupant.
    unsafe { *registers.add(index) = literal_value(literal) };
    values.set(index, NumericValue::OTHER);
    *dirty &= !(1u64 << index);
    pins.invalidate(index);
}

#[inline(never)]
unsafe fn move_other_register(
    registers: *mut Value,
    values: &mut NumericRegisters,
    dirty: &mut u64,
    pins: &mut Pins,
    destination: Register,
    source: Register,
) {
    let destination_index = destination.index() as usize;
    // SAFETY: other-kind shadows never shadow their live register; the
    // write drops the previous occupant.
    unsafe {
        let copied = (*registers.add(source.index() as usize)).clone_inline_scalar();
        *registers.add(destination_index) = copied;
    }
    values.set(destination_index, NumericValue::OTHER);
    *dirty &= !(1u64 << destination_index);
    pins.invalidate(destination_index);
}

#[inline(never)]
unsafe fn string_length_operation(
    registers: *mut Value,
    values: &mut NumericRegisters,
    dirty: &mut u64,
    pins: &mut Pins,
    destination: Register,
    source: Register,
) -> bool {
    let source_index = source.index() as usize;
    if values.kind(source_index) != NumericKind::Other {
        return false;
    }
    // SAFETY: other-kind shadows never shadow their live register, so the
    // read sees the current value.
    let length = match unsafe { &*registers.add(source_index) }.transparent() {
        ValueView::String(string) => string.len() as i64,
        ValueView::ShortString(string) => string.as_bytes().len() as i64,
        _ => return false,
    };
    // SAFETY: the numeric-loop proof covers the instruction, registers, and types.
    unsafe {
        assign(
            registers,
            values,
            dirty,
            pins,
            destination,
            NumericValue::int(length),
        )
    };
    true
}

#[inline(never)]
unsafe fn concatenate_append(
    registers: *mut Value,
    values: &NumericRegisters,
    destination: Register,
    left: Register,
    right: Register,
) -> bool {
    destination == left
        && left != right
        && values.kind(left.index() as usize) == NumericKind::Other
        && values.kind(right.index() as usize) == NumericKind::Other
        && {
            // SAFETY: other-kind shadows never shadow their live register,
            // so both reads see current values.
            let left_value = unsafe { &*registers.add(left.index() as usize) };
            // SAFETY: the numeric-loop proof covers the instruction, registers, and types.
            let right_value = unsafe { &*registers.add(right.index() as usize) };
            match (left_value.transparent(), right_value.transparent()) {
                // SAFETY: the numeric-loop proof covers the instruction, registers, and types.
                (ValueView::String(target), ValueView::String(extra)) => unsafe {
                    ByteStringObject::append_unique_string(target, extra)
                },
                // SAFETY: the numeric-loop proof covers the instruction, registers, and types.
                (ValueView::String(target), ValueView::ShortString(extra)) => unsafe {
                    ByteStringObject::append_unique(target, extra.as_bytes())
                },
                _ => false,
            }
        }
}

/// Routes a marker-headed burst attempt by the first body instruction's
/// kind; each burst requires a distinct head, so one word read decides.
#[expect(
    clippy::too_many_arguments,
    reason = "the burst borrows every piece of executor state"
)]
#[inline(never)]
unsafe fn attempt_marker_bursts(
    chunk: &Chunk,
    registers: *mut Value,
    values: &mut NumericRegisters,
    dirty: &mut u64,
    pins: &mut Pins,
    marker: usize,
    comparison: BytecodeComparison,
    left: Register,
    right: Register,
    marker_exit: usize,
) -> Option<usize> {
    // SAFETY: the numeric-loop proof covers the instruction, registers, and types.
    let first_kind = unsafe { InstructionWord::read(chunk.code.as_ptr().add(marker + 1)) }.kind();
    match first_kind {
        // SAFETY: the numeric-loop proof covers the instruction, registers, and types.
        InstructionKind::LoadInt => unsafe {
            try_fill_burst(
                chunk,
                registers,
                values,
                dirty,
                pins,
                marker,
                comparison,
                left,
                right,
                marker_exit,
            )
        },
        InstructionKind::DictIndexGetIntKey => {
            // SAFETY: the numeric-loop proof covers the instruction, registers, and types.
            let burst = unsafe {
                try_dict_accumulate_burst(
                    chunk,
                    registers,
                    values,
                    dirty,
                    pins,
                    marker,
                    comparison,
                    left,
                    right,
                    marker_exit,
                )
            };
            if burst.is_some() {
                return burst;
            }
            // SAFETY: the numeric-loop proof covers the instruction, registers, and types.
            unsafe {
                try_dict_copy_burst(
                    chunk,
                    registers,
                    values,
                    dirty,
                    pins,
                    marker,
                    comparison,
                    left,
                    right,
                    marker_exit,
                )
            }
        }
        // SAFETY: the numeric-loop proof covers the instruction, registers, and types.
        InstructionKind::DictIndexSetIntKey => unsafe {
            try_dict_build_burst(
                chunk,
                registers,
                values,
                dirty,
                marker,
                left,
                right,
                marker_exit,
            )
        },
        // SAFETY: the numeric-loop proof covers the instruction, registers, and types.
        InstructionKind::VecAppend => unsafe {
            try_vec_append_step_burst(
                chunk,
                registers,
                values,
                dirty,
                marker,
                comparison,
                left,
                right,
                marker_exit,
            )
        },
        _ => None,
    }
}

#[expect(
    clippy::too_many_arguments,
    reason = "the burst receives the executor state and loop shape"
)]
#[inline(never)]
unsafe fn try_vec_append_step_burst(
    chunk: &Chunk,
    registers: *mut Value,
    values: &mut NumericRegisters,
    dirty: &mut u64,
    marker: usize,
    comparison: BytecodeComparison,
    counter: Register,
    limit: Register,
    exit: usize,
) -> Option<usize> {
    let body = marker + 1;
    if exit <= body + 2 || exit > chunk.code.len() {
        return None;
    }

    let jump = exit - 1;
    let update = jump - 1;
    // SAFETY: the numeric-loop proof covers the instruction, registers, and types.
    let word = |at: usize| unsafe { InstructionWord::read(chunk.code.as_ptr().add(at)) };
    let Instruction::IntAddAssign {
        target: update_target,
        source: step,
        // SAFETY: the numeric-loop proof covers the instruction, registers, and types.
    } = (unsafe { word(update).decode() })
    else {
        return None;
    };
    // SAFETY: the numeric-loop proof covers the instruction, registers, and types.
    let Instruction::Jump { offset } = (unsafe { word(jump).decode() }) else {
        return None;
    };
    if update_target != counter
        || relative_target(jump, offset.offset()) != marker
        || values.kind(counter.index() as usize) != NumericKind::Int
        || values.kind(limit.index() as usize) != NumericKind::Int
        || values.kind(step.index() as usize) != NumericKind::Int
    {
        return None;
    }

    let mut target = Register::NONE;
    let mut append_count = 0usize;
    for at in body..update {
        // SAFETY: the numeric-loop proof covers the instruction, registers, and types.
        let instruction = unsafe { word(at).decode() };
        if let Instruction::VecAppend { container, value } = instruction {
            if values.kind(value.index() as usize) == NumericKind::Other
                || (target != Register::NONE && target != container)
            {
                return None;
            }

            target = container;
            append_count += 1;
        } else {
            if !int_burst_instruction_ready(instruction, values) {
                return None;
            }

            let destination = int_burst_destination(instruction)?;
            if destination == counter || destination == limit || destination == step {
                return None;
            }
        }
    }
    if target == Register::NONE {
        return None;
    }

    let counter_index = counter.index() as usize;
    let limit_value = values.int(limit.index() as usize);
    let step_value = values.int(step.index() as usize);
    let remaining = stepped_loop_iterations(
        comparison,
        values.int(counter_index),
        limit_value,
        step_value,
    )?;
    // SAFETY: the numeric-loop proof covers the instruction, registers, and types.
    let vector = (unsafe { &mut *registers.add(target.index() as usize) }).as_vec_mut()?;
    let vector = vector.make_mut();
    vector.invalidate_type_check();
    vector.reserve_hint(remaining.min(1 << 24).saturating_mul(append_count));

    let mut until_batch_end = BATCH_ITERATION_LIMIT;
    loop {
        for at in body..update {
            // SAFETY: the numeric-loop proof covers the instruction, registers, and types.
            let instruction = unsafe { word(at).decode() };
            if let Instruction::VecAppend { value, .. } = instruction {
                vector.push_after_type_check_invalidation(
                    values.get(value.index() as usize).into_value(),
                );
                continue;
            }

            let Some((destination, result)) = int_burst_operation(instruction, values) else {
                // SAFETY: the surrounding invariant makes this path unreachable.
                unsafe {
                    unreachable_invariant(
                        "the stepped vec append burst shape was checked before execution",
                    )
                }
            };
            let Ok(result) = result else {
                return Some(at);
            };
            assign_existing_int(values, dirty, destination, result);
        }

        until_batch_end -= 1;
        if until_batch_end == 0 {
            return Some(update);
        }

        let current = values.int(counter_index);
        let Some(next) = current.checked_add(step_value) else {
            return Some(update);
        };
        assign_existing_int(values, dirty, counter, next);
        if !int_ordered_comparison_matches(comparison, next, limit_value) {
            return Some(exit);
        }
    }
}

/// Runs the remainder of a counted loop whose body is entirely specialized
/// integer operations in a compact dispatch loop. The ordinary executor runs
/// the first iteration, which both validates the specialization and turns
/// compiler temporaries into integer shadow registers before this path starts.
#[expect(
    clippy::too_many_arguments,
    reason = "the burst receives the verified counted-loop shape"
)]
#[inline(never)]
unsafe fn try_int_body_burst(
    chunk: &Chunk,
    values: &mut NumericRegisters,
    dirty: &mut u64,
    body: usize,
    tail: usize,
    comparison: BytecodeComparison,
    counter: Register,
    limit: Register,
    exit: usize,
) -> Option<usize> {
    if body >= tail || exit != tail + 1 {
        return None;
    }

    // SAFETY: the numeric-loop proof covers the instruction, registers, and types.
    let word = |at: usize| unsafe { InstructionWord::read(chunk.code.as_ptr().add(at)) };
    let Instruction::IntCounterLoop {
        comparison: tail_comparison,
        counter: tail_counter,
        limit: tail_limit,
        offset,
        // SAFETY: the numeric-loop proof covers the instruction, registers, and types.
    } = (unsafe { word(tail).decode() })
    else {
        return None;
    };
    if tail_comparison != comparison
        || tail_counter != counter
        || tail_limit != limit
        || relative_target(tail, i32::from(offset.offset())) != body
        || !matches!(
            comparison,
            BytecodeComparison::LessThan
                | BytecodeComparison::LessThanOrEqual
                | BytecodeComparison::GreaterThan
                | BytecodeComparison::GreaterThanOrEqual
        )
        || values.kind(counter.index() as usize) != NumericKind::Int
        || values.kind(limit.index() as usize) != NumericKind::Int
    {
        return None;
    }

    for at in body..tail {
        // SAFETY: the numeric-loop proof covers the instruction, registers, and types.
        if !int_burst_instruction_ready(unsafe { word(at).decode() }, values) {
            return None;
        }
    }

    let counter_index = counter.index() as usize;
    let limit_value = values.int(limit.index() as usize);
    let mask = u64::from(BATCH_ITERATION_LIMIT - 1);
    let mut until_batch_end =
        BATCH_ITERATION_LIMIT - (values.int(counter_index) as u64 & mask) as u32;

    loop {
        for at in body..tail {
            // SAFETY: the numeric-loop proof covers the instruction, registers, and types.
            let instruction = unsafe { word(at).decode() };
            let Some((destination, result)) = int_burst_operation(instruction, values) else {
                // SAFETY: the surrounding invariant makes this path unreachable.
                unsafe {
                    unreachable_invariant("the integer burst shape was checked before execution")
                }
            };
            let Ok(result) = result else {
                return Some(at);
            };
            assign_existing_int(values, dirty, destination, result);
        }

        until_batch_end -= 1;
        if until_batch_end == 0 {
            return Some(tail);
        }

        let current = values.int(counter_index);
        let next = if comparison == BytecodeComparison::LessThan {
            current + 1
        } else {
            let Some(next) = current.checked_add(1) else {
                return Some(tail);
            };
            next
        };
        assign_existing_int(values, dirty, counter, next);
        if !int_ordered_comparison_matches(comparison, next, limit_value) {
            return Some(exit);
        }
    }
}

#[inline(always)]
fn int_burst_instruction_ready(instruction: Instruction, values: &NumericRegisters) -> bool {
    match instruction {
        Instruction::IntAdd {
            destination,
            left,
            right,
        }
        | Instruction::IntSubtract {
            destination,
            left,
            right,
        }
        | Instruction::IntMultiply {
            destination,
            left,
            right,
        }
        | Instruction::IntModulo {
            destination,
            left,
            right,
        }
        | Instruction::IntBitwiseAnd {
            destination,
            left,
            right,
        }
        | Instruction::IntBitwiseOr {
            destination,
            left,
            right,
        }
        | Instruction::IntBitwiseXor {
            destination,
            left,
            right,
        }
        | Instruction::IntShiftLeft {
            destination,
            left,
            right,
        }
        | Instruction::IntShiftRight {
            destination,
            left,
            right,
        } => {
            values.kind(destination.index() as usize) == NumericKind::Int
                && values.kind(left.index() as usize) == NumericKind::Int
                && values.kind(right.index() as usize) == NumericKind::Int
        }
        Instruction::IntBitwiseNot {
            destination,
            source,
        }
        | Instruction::IntMultiplyImmediate {
            destination,
            source,
            ..
        }
        | Instruction::IntModuloImmediate {
            destination,
            source,
            ..
        } => {
            values.kind(destination.index() as usize) == NumericKind::Int
                && values.kind(source.index() as usize) == NumericKind::Int
        }
        _ => false,
    }
}

fn int_burst_destination(instruction: Instruction) -> Option<Register> {
    match instruction {
        Instruction::IntAdd { destination, .. }
        | Instruction::IntSubtract { destination, .. }
        | Instruction::IntMultiply { destination, .. }
        | Instruction::IntModulo { destination, .. }
        | Instruction::IntMultiplyImmediate { destination, .. }
        | Instruction::IntModuloImmediate { destination, .. }
        | Instruction::IntBitwiseAnd { destination, .. }
        | Instruction::IntBitwiseOr { destination, .. }
        | Instruction::IntBitwiseXor { destination, .. }
        | Instruction::IntShiftLeft { destination, .. }
        | Instruction::IntShiftRight { destination, .. }
        | Instruction::IntBitwiseNot { destination, .. } => Some(destination),
        _ => None,
    }
}

#[inline(always)]
fn int_burst_operation(
    instruction: Instruction,
    values: &NumericRegisters,
) -> Option<(Register, Result<i64, Fault>)> {
    Some(match instruction {
        Instruction::IntAdd {
            destination,
            left,
            right,
        } => (
            destination,
            integer_add(
                values.int(left.index() as usize),
                values.int(right.index() as usize),
            ),
        ),
        Instruction::IntSubtract {
            destination,
            left,
            right,
        } => (
            destination,
            integer_subtract(
                values.int(left.index() as usize),
                values.int(right.index() as usize),
            ),
        ),
        Instruction::IntMultiply {
            destination,
            left,
            right,
        } => (
            destination,
            integer_multiply(
                values.int(left.index() as usize),
                values.int(right.index() as usize),
            ),
        ),
        Instruction::IntModulo {
            destination,
            left,
            right,
        } => (
            destination,
            integer_modulo(
                values.int(left.index() as usize),
                values.int(right.index() as usize),
            ),
        ),
        Instruction::IntMultiplyImmediate {
            destination,
            source,
            immediate,
        } => (
            destination,
            integer_multiply(
                values.int(source.index() as usize),
                i64::from(immediate.value()),
            ),
        ),
        Instruction::IntModuloImmediate {
            destination,
            source,
            immediate,
        } => (
            destination,
            integer_modulo(
                values.int(source.index() as usize),
                i64::from(immediate.value()),
            ),
        ),
        Instruction::IntBitwiseAnd {
            destination,
            left,
            right,
        } => (
            destination,
            Ok(values.int(left.index() as usize) & values.int(right.index() as usize)),
        ),
        Instruction::IntBitwiseOr {
            destination,
            left,
            right,
        } => (
            destination,
            Ok(values.int(left.index() as usize) | values.int(right.index() as usize)),
        ),
        Instruction::IntBitwiseXor {
            destination,
            left,
            right,
        } => (
            destination,
            Ok(values.int(left.index() as usize) ^ values.int(right.index() as usize)),
        ),
        Instruction::IntBitwiseNot {
            destination,
            source,
        } => (destination, Ok(!values.int(source.index() as usize))),
        Instruction::IntShiftLeft {
            destination,
            left,
            right,
        } => {
            let left = values.int(left.index() as usize);
            let right = values.int(right.index() as usize);
            (
                destination,
                if (0..=63).contains(&right) {
                    Ok(((left as u64) << right as u32) as i64)
                } else {
                    Err(Fault::ShiftRange)
                },
            )
        }
        Instruction::IntShiftRight {
            destination,
            left,
            right,
        } => {
            let left = values.int(left.index() as usize);
            let right = values.int(right.index() as usize);
            (
                destination,
                if (0..=63).contains(&right) {
                    Ok(left >> right as u32)
                } else {
                    Err(Fault::ShiftRange)
                },
            )
        }
        _ => return None,
    })
}

#[expect(
    clippy::too_many_arguments,
    reason = "the burst receives the executor state and counted-loop shape"
)]
#[inline(never)]
unsafe fn try_vec_append_burst(
    chunk: &Chunk,
    registers: *mut Value,
    values: &mut NumericRegisters,
    dirty: &mut u64,
    body: usize,
    tail: usize,
    comparison: BytecodeComparison,
    counter: Register,
    limit: Register,
    exit: usize,
) -> Option<usize> {
    if body >= tail || exit != tail + 1 {
        return None;
    }

    // SAFETY: the numeric-loop proof covers the instruction, registers, and types.
    let word = |at: usize| unsafe { InstructionWord::read(chunk.code.as_ptr().add(at)) };
    let Instruction::IntCounterLoop {
        comparison: tail_comparison,
        counter: tail_counter,
        limit: tail_limit,
        offset,
        // SAFETY: the numeric-loop proof covers the instruction, registers, and types.
    } = (unsafe { word(tail).decode() })
    else {
        return None;
    };
    if tail_comparison != comparison
        || tail_counter != counter
        || tail_limit != limit
        || relative_target(tail, i32::from(offset.offset())) != body
        || values.kind(counter.index() as usize) != NumericKind::Int
        || values.kind(limit.index() as usize) != NumericKind::Int
    {
        return None;
    }

    let mut target = Register::NONE;
    let mut append_count = 0usize;
    for at in body..tail {
        // SAFETY: the numeric-loop proof covers the instruction, registers, and types.
        let instruction = unsafe { word(at).decode() };
        if let Instruction::VecAppend { container, value } = instruction {
            if values.kind(value.index() as usize) == NumericKind::Other
                || (target != Register::NONE && target != container)
            {
                return None;
            }
            target = container;
            append_count += 1;
        } else if !int_burst_instruction_ready(instruction, values) {
            return None;
        }
    }
    if target == Register::NONE {
        return None;
    }

    // SAFETY: the numeric-loop proof covers the instruction, registers, and types.
    let vector = (unsafe { &mut *registers.add(target.index() as usize) }).as_vec_mut()?;
    let vector = vector.make_mut();
    vector.invalidate_type_check();
    let counter_index = counter.index() as usize;
    let limit_value = values.int(limit.index() as usize);
    let remaining = match comparison {
        BytecodeComparison::LessThan => limit_value.saturating_sub(values.int(counter_index)),
        BytecodeComparison::LessThanOrEqual => limit_value
            .saturating_sub(values.int(counter_index))
            .saturating_add(1),
        _ => 0,
    };
    let reservation = usize::try_from(remaining.clamp(0, 1 << 24))
        .unwrap_or(0)
        .saturating_mul(append_count);
    vector.reserve_hint(reservation);

    let mask = u64::from(BATCH_ITERATION_LIMIT - 1);
    let mut until_batch_end =
        BATCH_ITERATION_LIMIT - (values.int(counter_index) as u64 & mask) as u32;
    loop {
        for at in body..tail {
            // SAFETY: the numeric-loop proof covers the instruction, registers, and types.
            let instruction = unsafe { word(at).decode() };
            if let Instruction::VecAppend { value, .. } = instruction {
                vector.push_after_type_check_invalidation(
                    values.get(value.index() as usize).into_value(),
                );
                continue;
            }

            let Some((destination, result)) = int_burst_operation(instruction, values) else {
                // SAFETY: the surrounding invariant makes this path unreachable.
                unsafe {
                    unreachable_invariant("the vec append burst shape was checked before execution")
                }
            };
            let Ok(result) = result else {
                return Some(at);
            };
            assign_existing_int(values, dirty, destination, result);
        }

        until_batch_end -= 1;
        if until_batch_end == 0 {
            return Some(tail);
        }

        let current = values.int(counter_index);
        let next = if comparison == BytecodeComparison::LessThan {
            current + 1
        } else {
            let Some(next) = current.checked_add(1) else {
                return Some(tail);
            };
            next
        };
        assign_existing_int(values, dirty, counter, next);
        if !int_ordered_comparison_matches(comparison, next, limit_value) {
            return Some(exit);
        }
    }
}

#[expect(
    clippy::too_many_arguments,
    reason = "the burst receives the executor state and counted-loop shape"
)]
#[inline(never)]
unsafe fn try_indexed_int_body_burst(
    chunk: &Chunk,
    registers: *mut Value,
    values: &mut NumericRegisters,
    dirty: &mut u64,
    pins: &mut Pins,
    body: usize,
    tail: usize,
    comparison: BytecodeComparison,
    counter: Register,
    limit: Register,
    exit: usize,
) -> Option<usize> {
    if body + 1 >= tail || exit != tail + 1 {
        return None;
    }

    // SAFETY: the numeric-loop proof covers the instruction, registers, and types.
    let word = |at: usize| unsafe { InstructionWord::read(chunk.code.as_ptr().add(at)) };
    // SAFETY: the numeric-loop proof covers the instruction, registers, and types.
    let (element, container, index, dictionary) = match unsafe { word(body).decode() } {
        Instruction::VecIndexGet {
            destination,
            container,
            index,
            value_mode: _,
        } => (destination, container, index, false),
        Instruction::DictIndexGetIntKey {
            destination,
            container,
            index,
            value_mode: _,
        } => (destination, container, index, true),
        _ => return None,
    };
    let Instruction::IntCounterLoop {
        comparison: tail_comparison,
        counter: tail_counter,
        limit: tail_limit,
        offset,
        // SAFETY: the numeric-loop proof covers the instruction, registers, and types.
    } = (unsafe { word(tail).decode() })
    else {
        return None;
    };
    if index != counter
        || element == counter
        || element == limit
        || element == container
        || tail_comparison != comparison
        || tail_counter != counter
        || tail_limit != limit
        || relative_target(tail, i32::from(offset.offset())) != body
        || !matches!(
            comparison,
            BytecodeComparison::LessThan
                | BytecodeComparison::LessThanOrEqual
                | BytecodeComparison::GreaterThan
                | BytecodeComparison::GreaterThanOrEqual
        )
        || values.kind(counter.index() as usize) != NumericKind::Int
        || values.kind(limit.index() as usize) != NumericKind::Int
        || values.kind(element.index() as usize) != NumericKind::Int
    {
        return None;
    }

    for at in body + 1..tail {
        // SAFETY: the numeric-loop proof covers the instruction, registers, and types.
        if !int_burst_instruction_ready(unsafe { word(at).decode() }, values) {
            return None;
        }
    }

    let (elements, length) = if dictionary {
        // SAFETY: the numeric-loop proof covers the instruction, registers, and types.
        unsafe { pins.for_read_dict(registers, container) }?
    } else {
        // SAFETY: the numeric-loop proof covers the instruction, registers, and types.
        unsafe { pins.for_read(registers, container) }?
    };
    let counter_index = counter.index() as usize;
    let limit_value = values.int(limit.index() as usize);
    let mask = u64::from(BATCH_ITERATION_LIMIT - 1);
    let mut until_batch_end =
        BATCH_ITERATION_LIMIT - (values.int(counter_index) as u64 & mask) as u32;

    loop {
        let position = values.int(counter_index);
        if position as u64 >= length as u64 {
            return Some(body);
        }
        let ValueView::Int(element_value) =
            // SAFETY: the numeric-loop proof covers the instruction, registers, and types.
            (unsafe { &*elements.add(position as usize) }).transparent()
        else {
            return Some(body);
        };
        assign_existing_int(values, dirty, element, *element_value);

        for at in body + 1..tail {
            // SAFETY: the numeric-loop proof covers the instruction, registers, and types.
            let instruction = unsafe { word(at).decode() };
            let Some((destination, result)) = int_burst_operation(instruction, values) else {
                // SAFETY: the surrounding invariant makes this path unreachable.
                unsafe {
                    unreachable_invariant(
                        "the indexed integer burst shape was checked before execution",
                    )
                }
            };
            let Ok(result) = result else {
                return Some(at);
            };
            assign_existing_int(values, dirty, destination, result);
        }

        until_batch_end -= 1;
        if until_batch_end == 0 {
            return Some(tail);
        }

        let current = values.int(counter_index);
        let next = if comparison == BytecodeComparison::LessThan {
            current + 1
        } else {
            let Some(next) = current.checked_add(1) else {
                return Some(tail);
            };
            next
        };
        assign_existing_int(values, dirty, counter, next);
        if !int_ordered_comparison_matches(comparison, next, limit_value) {
            return Some(exit);
        }
    }
}

/// Attempts every backedge-entered burst for a region jump site in order.
#[inline(never)]
unsafe fn attempt_region_bursts(
    chunk: &Chunk,
    registers: *mut Value,
    values: &mut NumericRegisters,
    dirty: &mut u64,
    target: usize,
    current: usize,
) -> Option<usize> {
    // SAFETY: the numeric-loop proof covers the instruction, registers, and types.
    unsafe { try_concat_burst(chunk, registers, values, dirty, target, current) }
}

/// A packed-dict accumulation loop run directly: each step reads one
/// element from two pinned packed dicts at the counter, adds them with an
/// overflow check, stores the sum back into the first, and steps the
/// counter down while the ordered header comparison holds.
#[expect(
    clippy::too_many_arguments,
    reason = "the burst borrows every piece of executor state"
)]
#[inline(never)]
unsafe fn try_dict_accumulate_burst(
    chunk: &Chunk,
    registers: *mut Value,
    values: &mut NumericRegisters,
    dirty: &mut u64,
    pins: &mut Pins,
    marker: usize,
    comparison: BytecodeComparison,
    left: Register,
    right: Register,
    marker_exit: usize,
) -> Option<usize> {
    let body = marker + 1;
    if body + 5 >= chunk.code.len() {
        return None;
    }

    // SAFETY: the numeric-loop proof covers the instruction, registers, and types.
    let word = |at: usize| unsafe { InstructionWord::read(chunk.code.as_ptr().add(at)) };
    let Instruction::DictIndexGetIntKey {
        destination: first_value,
        container: target,
        index: first_index,
        value_mode: _,
        // SAFETY: the numeric-loop proof covers the instruction, registers, and types.
    } = (unsafe { word(body).decode() })
    else {
        return None;
    };

    let Instruction::DictIndexGetIntKey {
        destination: second_value,
        container: source,
        index: second_index,
        value_mode: _,
        // SAFETY: the numeric-loop proof covers the instruction, registers, and types.
    } = (unsafe { word(body + 1).decode() })
    else {
        return None;
    };

    // SAFETY: the numeric-loop proof covers the instruction, registers, and types.
    let (sum, add_left, add_right) = match unsafe { word(body + 2).decode() } {
        Instruction::Add {
            destination,
            left,
            right,
        }
        | Instruction::IntAdd {
            destination,
            left,
            right,
        } => (destination, left, right),
        _ => return None,
    };

    let Instruction::DictIndexSetIntKey {
        container: store_target,
        index: store_index,
        value: store_value,
        // SAFETY: the numeric-loop proof covers the instruction, registers, and types.
    } = (unsafe { word(body + 3).decode() })
    else {
        return None;
    };

    let Instruction::SubtractImmediate {
        destination: step_destination,
        source: step_source,
        immediate,
        // SAFETY: the numeric-loop proof covers the instruction, registers, and types.
    } = (unsafe { word(body + 4).decode() })
    else {
        return None;
    };

    // SAFETY: the numeric-loop proof covers the instruction, registers, and types.
    let Instruction::Jump { offset: back } = (unsafe { word(body + 5).decode() }) else {
        return None;
    };

    let step = i64::from(immediate.value());
    if first_index != left
        || second_index != left
        || add_left != first_value
        || add_right != second_value
        || store_target != target
        || store_index != left
        || store_value != sum
        || step_destination != left
        || step_source != left
        || step < 1
        || relative_target(body + 5, back.offset()) != marker
        || !matches!(
            comparison,
            BytecodeComparison::LessThan
                | BytecodeComparison::LessThanOrEqual
                | BytecodeComparison::GreaterThan
                | BytecodeComparison::GreaterThanOrEqual
        )
        || values.kind(left.index() as usize) != NumericKind::Int
        || values.kind(right.index() as usize) != NumericKind::Int
        || first_value == second_value
    {
        return None;
    }

    for temp in [first_value, second_value, sum] {
        if temp == left || temp == right || temp == target || temp == source {
            return None;
        }
    }

    // SAFETY: the numeric-loop proof covers the instruction, registers, and types.
    let (target_elements, target_length) = (unsafe { pins.for_write_dict(registers, target) })?;
    // SAFETY: the numeric-loop proof covers the instruction, registers, and types.
    let (source_elements, source_length) = (unsafe { pins.for_read_dict(registers, source) })?;

    let bound = values.int(right.index() as usize);
    let mut cursor_value = values.int(left.index() as usize);
    let reach = target_length.min(source_length);
    let mut last_first = 0i64;
    let mut last_second = 0i64;
    let mut last_sum = 0i64;
    let mut completed = false;
    let mut budget: u32 = 1 << 24;
    let finished = loop {
        if cursor_value as u64 >= reach as u64 {
            break false;
        }

        // SAFETY: the position is bounded by both pinned lengths.
        let first = unsafe { &*target_elements.add(cursor_value as usize) };
        // SAFETY: the numeric-loop proof covers the instruction, registers, and types.
        let second = unsafe { &*source_elements.add(cursor_value as usize) };
        let (ValueView::Int(first), ValueView::Int(second)) =
            (first.transparent(), second.transparent())
        else {
            break false;
        };

        let (first, second) = (*first, *second);
        let Some(total) = first.checked_add(second) else {
            break false;
        };

        // SAFETY: bounded above; the replaced element was just matched as an
        // `Int`, which owns nothing, so the raw write skips its drop.
        unsafe {
            ptr::write(
                target_elements.add(cursor_value as usize),
                Value::int(total),
            )
        };

        last_first = first;
        last_second = second;
        last_sum = total;
        completed = true;
        cursor_value -= step;
        budget -= 1;
        if budget == 0 {
            break false;
        }
        if !int_ordered_comparison_matches(comparison, cursor_value, bound) {
            break true;
        }
    };

    if !completed {
        return None;
    }

    assign_existing_int(values, dirty, left, cursor_value);
    // SAFETY: the numeric-loop proof covers the instruction, registers, and types.
    unsafe {
        assign(
            registers,
            values,
            dirty,
            pins,
            first_value,
            NumericValue::int(last_first),
        );
        assign(
            registers,
            values,
            dirty,
            pins,
            second_value,
            NumericValue::int(last_second),
        );
        assign(
            registers,
            values,
            dirty,
            pins,
            sum,
            NumericValue::int(last_sum),
        );
    }

    if finished {
        Some(marker_exit)
    } else {
        Some(marker)
    }
}

/// A counted dict build loop run directly: the body stores an integer
/// register (or the counter itself) through the counter into one dict, with
/// an immediate counter step between unrolled stores, and the counted tail
/// advances. The destination is pre-sized for the counted range.
#[expect(
    clippy::too_many_arguments,
    reason = "the burst borrows every piece of executor state"
)]
#[inline(never)]
unsafe fn try_dict_build_burst(
    chunk: &Chunk,
    registers: *mut Value,
    values: &mut NumericRegisters,
    dirty: &mut u64,
    marker: usize,
    left: Register,
    right: Register,
    marker_exit: usize,
) -> Option<usize> {
    let body = marker + 1;
    // SAFETY: the numeric-loop proof covers the instruction, registers, and types.
    let word = |at: usize| unsafe { InstructionWord::read(chunk.code.as_ptr().add(at)) };
    let Instruction::DictIndexSetIntKey {
        container: target,
        index: first_index,
        value: value_register,
        // SAFETY: the numeric-loop proof covers the instruction, registers, and types.
    } = (unsafe { word(body).decode() })
    else {
        return None;
    };
    if first_index != left {
        return None;
    }

    let mut repeats = 0usize;
    let mut step = 0i64;
    let mut at = body;
    let (tail_comparison, tail_ip) = loop {
        if at + 1 >= chunk.code.len() {
            return None;
        }
        let Instruction::DictIndexSetIntKey {
            container,
            index,
            value,
            // SAFETY: the numeric-loop proof covers the instruction, registers, and types.
        } = (unsafe { word(at).decode() })
        else {
            return None;
        };
        if container != target || index != left || value != value_register {
            return None;
        }
        repeats += 1;
        if repeats > 16 {
            return None;
        }
        // SAFETY: the numeric-loop proof covers the instruction, registers, and types.
        match unsafe { word(at + 1).decode() } {
            Instruction::AddImmediate {
                destination,
                source,
                immediate,
            } => {
                let increment = i64::from(immediate.value());
                if destination != left || source != left || (repeats > 1 && increment != step) {
                    return None;
                }
                step = increment;
                at += 2;
            }
            Instruction::IntCounterLoop {
                comparison: tail_comparison,
                counter,
                limit,
                offset,
            } => {
                if counter != left
                    || limit != right
                    || !matches!(
                        tail_comparison,
                        BytecodeComparison::LessThan
                            | BytecodeComparison::LessThanOrEqual
                            | BytecodeComparison::GreaterThan
                            | BytecodeComparison::GreaterThanOrEqual
                    )
                    || relative_target(at + 1, i32::from(offset.offset())) != body
                {
                    return None;
                }
                break (tail_comparison, at + 1);
            }
            _ => return None,
        }
    };

    if values.kind(left.index() as usize) != NumericKind::Int
        || values.kind(right.index() as usize) != NumericKind::Int
        || target == left
        || target == right
    {
        return None;
    }
    let value_template = if value_register == left {
        None
    } else {
        if values.kind(value_register.index() as usize) == NumericKind::Other {
            return None;
        }
        Some(values.get(value_register.index() as usize).into_value())
    };

    // SAFETY: the register window is live; the target register was proven a
    // dict at this site and is distinct from every register written here.
    let target_dict = (unsafe { &mut *registers.add(target.index() as usize) }).as_dict_mut()?;
    let target_dict = target_dict.make_mut();

    let limit_value = values.int(right.index() as usize);
    let mut cursor_value = values.int(left.index() as usize);
    let span = limit_value
        .abs_diff(cursor_value)
        .saturating_add(1)
        .min(1 << 20) as usize;
    target_dict.reserve_hint(span);

    let mut budget: u32 = 1 << 22;
    let finished = 'build: loop {
        for repeat in 0..repeats {
            let stored = match &value_template {
                Some(value) => value.clone(),
                None => Value::int(cursor_value),
            };
            target_dict.insert(Key::Int(cursor_value), stored);
            if repeat + 1 < repeats {
                let Some(next) = cursor_value.checked_add(step) else {
                    assign_existing_int(values, dirty, left, cursor_value);
                    return Some(body + repeat * 2 + 1);
                };
                cursor_value = next;
            }
        }
        if tail_comparison == BytecodeComparison::LessThan {
            cursor_value = cursor_value.wrapping_add(1);
        } else {
            let Some(next) = cursor_value.checked_add(1) else {
                assign_existing_int(values, dirty, left, cursor_value);
                return Some(tail_ip);
            };
            cursor_value = next;
        }
        if !int_ordered_comparison_matches(tail_comparison, cursor_value, limit_value) {
            break 'build true;
        }
        budget -= 1;
        if budget == 0 {
            break 'build false;
        }
    };

    assign_existing_int(values, dirty, left, cursor_value);
    if finished {
        Some(marker_exit)
    } else {
        Some(marker)
    }
}

/// A dict copy loop run directly: the body is one or more triplets that
/// read a pinned packed dict at the counter, insert into another dict, and
/// step the counter down. The destination is pre-sized from the counted
/// range, so the bulk build never rehashes.
#[expect(
    clippy::too_many_arguments,
    reason = "the burst borrows every piece of executor state"
)]
#[inline(never)]
unsafe fn try_dict_copy_burst(
    chunk: &Chunk,
    registers: *mut Value,
    values: &mut NumericRegisters,
    dirty: &mut u64,
    pins: &mut Pins,
    marker: usize,
    comparison: BytecodeComparison,
    left: Register,
    right: Register,
    marker_exit: usize,
) -> Option<usize> {
    let body = marker + 1;
    // SAFETY: the numeric-loop proof covers the instruction, registers, and types.
    let word = |at: usize| unsafe { InstructionWord::read(chunk.code.as_ptr().add(at)) };

    let Instruction::DictIndexGetIntKey {
        destination: temp,
        container: source,
        index: first_index,
        value_mode: _,
        // SAFETY: the numeric-loop proof covers the instruction, registers, and types.
    } = (unsafe { word(body).decode() })
    else {
        return None;
    };
    let Instruction::DictIndexSetIntKey {
        container: target, ..
    // SAFETY: the numeric-loop proof covers the instruction, registers, and types.
    } = (unsafe { word(body + 1).decode() })
    else {
        return None;
    };
    // SAFETY: the numeric-loop proof covers the instruction, registers, and types.
    let Instruction::SubtractImmediate { immediate, .. } = (unsafe { word(body + 2).decode() })
    else {
        return None;
    };
    let step = i64::from(immediate.value());
    if first_index != left
        || target == source
        || step < 1
        || temp == left
        || temp == right
        || temp == source
        || temp == target
        || !matches!(
            comparison,
            BytecodeComparison::LessThan
                | BytecodeComparison::LessThanOrEqual
                | BytecodeComparison::GreaterThan
                | BytecodeComparison::GreaterThanOrEqual
        )
        || values.kind(left.index() as usize) != NumericKind::Int
        || values.kind(right.index() as usize) != NumericKind::Int
    {
        return None;
    }

    let mut repeats = 0usize;
    let mut at = body;
    let backedge = loop {
        if at + 2 >= chunk.code.len() {
            return None;
        }
        // SAFETY: the numeric-loop proof covers the instruction, registers, and types.
        if let Instruction::Jump { offset } = unsafe { word(at).decode() } {
            break relative_target(at, offset.offset());
        }
        let Instruction::DictIndexGetIntKey {
            destination,
            container,
            index,
            value_mode: _,
            // SAFETY: the numeric-loop proof covers the instruction, registers, and types.
        } = (unsafe { word(at).decode() })
        else {
            return None;
        };
        let Instruction::DictIndexSetIntKey {
            container: store_target,
            index: store_index,
            value,
            // SAFETY: the numeric-loop proof covers the instruction, registers, and types.
        } = (unsafe { word(at + 1).decode() })
        else {
            return None;
        };
        let Instruction::SubtractImmediate {
            destination: step_destination,
            source: step_source,
            immediate: step_immediate,
            // SAFETY: the numeric-loop proof covers the instruction, registers, and types.
        } = (unsafe { word(at + 2).decode() })
        else {
            return None;
        };
        if destination != temp
            || container != source
            || index != left
            || store_target != target
            || store_index != left
            || value != temp
            || step_destination != left
            || step_source != left
            || i64::from(step_immediate.value()) != step
        {
            return None;
        }
        repeats += 1;
        at += 3;
        if repeats > 16 {
            return None;
        }
    };
    if repeats == 0 || backedge != marker {
        return None;
    }

    // SAFETY: the numeric-loop proof covers the instruction, registers, and types.
    let (source_elements, source_length) = (unsafe { pins.for_read_dict(registers, source) })?;
    // SAFETY: the register window is live; the target register was proven a
    // dict at this site and is distinct from every register written here.
    let target_dict = (unsafe { &mut *registers.add(target.index() as usize) }).as_dict_mut()?;
    let target_dict = target_dict.make_mut();

    let bound = values.int(right.index() as usize);
    let mut cursor_value = values.int(left.index() as usize);
    if cursor_value >= 0 {
        let span = match comparison {
            BytecodeComparison::GreaterThan => cursor_value.saturating_sub(bound),
            BytecodeComparison::GreaterThanOrEqual => {
                cursor_value.saturating_sub(bound).saturating_add(1)
            }
            _ => cursor_value.saturating_add(1),
        };
        let reservation = usize::try_from(span.clamp(0, 1 << 20)).unwrap_or(0);
        target_dict.reserve_for_build(reservation);
    }

    let mut last_read = -1i64;
    let mut budget: u32 = 1 << 22;
    let mut resume = None;
    let finished = 'copy: loop {
        for repeat in 0..repeats {
            if cursor_value as u64 >= source_length as u64 || budget == 0 {
                resume = Some(body + repeat * 3);
                break 'copy false;
            }
            // SAFETY: the position is bounded by the pinned length; the
            // source dict is a distinct object from the separated target.
            let element = unsafe { (*source_elements.add(cursor_value as usize)).clone() };
            target_dict.insert(Key::Int(cursor_value), element);
            last_read = cursor_value;
            cursor_value -= step;
            budget -= 1;
        }
        if !int_ordered_comparison_matches(comparison, cursor_value, bound) {
            break true;
        }
    };

    if last_read < 0 {
        return None;
    }
    assign_existing_int(values, dirty, left, cursor_value);
    // SAFETY: the last read position stayed bounded by the pinned length.
    let element = unsafe { &*source_elements.add(last_read as usize) };
    let temp_index = temp.index() as usize;
    match element.transparent() {
        ValueView::Int(element) => {
            let value = NumericValue::int(*element);
            // SAFETY: the numeric-loop proof covers the instruction, registers, and types.
            unsafe { assign(registers, values, dirty, pins, temp, value) };
        }
        ValueView::Float(element) => {
            let value = NumericValue::float(*element);
            // SAFETY: the numeric-loop proof covers the instruction, registers, and types.
            unsafe { assign(registers, values, dirty, pins, temp, value) };
        }
        ValueView::Bool(element) => {
            let value = NumericValue::bool(*element);
            // SAFETY: the numeric-loop proof covers the instruction, registers, and types.
            unsafe { assign(registers, values, dirty, pins, temp, value) };
        }
        _ => {
            // SAFETY: the temp register is in the live window; the write
            // drops its previous occupant.
            unsafe { *registers.add(temp_index) = element.clone() };
            values.set(temp_index, NumericValue::OTHER);
            *dirty &= !(1u64 << temp_index);
            pins.invalidate(temp_index);
        }
    }
    if finished {
        Some(marker_exit)
    } else {
        Some(resume.unwrap_or(marker))
    }
}

/// Runs a proven integer scan as one loop.
///
/// # Safety
///
/// `registers` and the shadow must match `chunk`.
#[expect(
    clippy::too_many_arguments,
    reason = "the burst borrows every piece of executor state"
)]
#[inline(never)]
unsafe fn try_scan_burst(
    chunk: &Chunk,
    registers: *mut Value,
    values: &mut NumericRegisters,
    dirty: &mut u64,
    pins: &mut Pins,
    counter_ip: usize,
    body: usize,
    comparison: BytecodeComparison,
    counter: Register,
    limit: Register,
) -> Option<usize> {
    if comparison != BytecodeComparison::LessThan {
        return None;
    }
    // SAFETY: the numeric-loop proof covers the instruction, registers, and types.
    let first = unsafe { InstructionWord::read(chunk.code.as_ptr().add(body)) };
    let (element_register, container, index_register) = match first.kind() {
        InstructionKind::IndexGet => {
            let Instruction::IndexGet {
                destination,
                container,
                index,
                // SAFETY: the numeric-loop proof covers the instruction, registers, and types.
            } = (unsafe { first.decode() })
            else {
                return None;
            };
            (destination, container, index)
        }
        InstructionKind::VecIndexGet => {
            let Instruction::VecIndexGet {
                destination,
                container,
                index,
                ..
            // SAFETY: the numeric-loop proof covers the instruction, registers, and types.
            } = (unsafe { first.decode() })
            else {
                return None;
            };
            (destination, container, index)
        }
        _ => return None,
    };
    if index_register != counter {
        return None;
    }
    // SAFETY: the numeric-loop proof covers the instruction, registers, and types.
    let second = unsafe { InstructionWord::read(chunk.code.as_ptr().add(body + 1)) };
    if second.kind() != InstructionKind::LoadInt {
        return None;
    }
    let Instruction::LoadInt {
        destination: threshold_register,
        immediate,
        // SAFETY: the numeric-loop proof covers the instruction, registers, and types.
    } = (unsafe { second.decode() })
    else {
        return None;
    };
    // SAFETY: the numeric-loop proof covers the instruction, registers, and types.
    let third = unsafe { InstructionWord::read(chunk.code.as_ptr().add(body + 2)) };
    if third.kind() != InstructionKind::JumpUnless {
        return None;
    }
    let Instruction::JumpUnless {
        comparison: element_comparison,
        left: compared,
        right: against,
        offset: skip,
        // SAFETY: the numeric-loop proof covers the instruction, registers, and types.
    } = (unsafe { third.decode() })
    else {
        return None;
    };
    if compared != element_register
        || against != threshold_register
        || relative_target(body + 2, i32::from(skip.offset())) != counter_ip
    {
        return None;
    }
    // SAFETY: the numeric-loop proof covers the instruction, registers, and types.
    let (elements, length) = (unsafe { pins.for_read(registers, container) })?;

    let threshold = i64::from(immediate.value());
    let limit_value = values.int(limit.index() as usize);
    let mut index_value = values.int(counter.index() as usize);
    loop {
        if index_value as u64 >= length as u64 {
            assign_existing_int(values, dirty, counter, index_value);
            return Some(body);
        }
        // SAFETY: bounded above.
        let element = unsafe { &*elements.add(index_value as usize) };
        let ValueView::Int(element_value) = element.transparent() else {
            assign_existing_int(values, dirty, counter, index_value);
            return Some(body);
        };
        let element_value = *element_value;
        if int_comparison_matches_any(element_comparison, element_value, threshold) {
            assign_existing_int(values, dirty, counter, index_value);
            // SAFETY: the numeric-loop proof covers the instruction, registers, and types.
            unsafe {
                assign(
                    registers,
                    values,
                    dirty,
                    pins,
                    element_register,
                    NumericValue::int(element_value),
                )
            };
            // SAFETY: the numeric-loop proof covers the instruction, registers, and types.
            unsafe {
                assign(
                    registers,
                    values,
                    dirty,
                    pins,
                    threshold_register,
                    NumericValue::int(threshold),
                )
            };
            return Some(body + 3);
        }
        let next = index_value + 1;
        if next as u64 & u64::from(BATCH_ITERATION_LIMIT - 1) == 0 {
            assign_existing_int(values, dirty, counter, index_value);
            // SAFETY: the numeric-loop proof covers the instruction, registers, and types.
            unsafe {
                assign(
                    registers,
                    values,
                    dirty,
                    pins,
                    element_register,
                    NumericValue::int(element_value),
                )
            };
            // SAFETY: the numeric-loop proof covers the instruction, registers, and types.
            unsafe {
                assign(
                    registers,
                    values,
                    dirty,
                    pins,
                    threshold_register,
                    NumericValue::int(threshold),
                )
            };
            return Some(counter_ip);
        }
        if next >= limit_value {
            assign_existing_int(values, dirty, counter, next);
            // SAFETY: the numeric-loop proof covers the instruction, registers, and types.
            unsafe {
                assign(
                    registers,
                    values,
                    dirty,
                    pins,
                    element_register,
                    NumericValue::int(element_value),
                )
            };
            // SAFETY: the numeric-loop proof covers the instruction, registers, and types.
            unsafe {
                assign(
                    registers,
                    values,
                    dirty,
                    pins,
                    threshold_register,
                    NumericValue::int(threshold),
                )
            };
            return Some(counter_ip + 1);
        }
        index_value = next;
    }
}

#[expect(
    clippy::too_many_arguments,
    reason = "the burst receives the verified counted-loop shape"
)]
#[inline(never)]
unsafe fn try_string_slice_burst(
    chunk: &Chunk,
    registers: *mut Value,
    values: &mut NumericRegisters,
    dirty: &mut u64,
    body: usize,
    tail: usize,
    comparison: BytecodeComparison,
    counter: Register,
    limit: Register,
    exit: usize,
) -> Option<usize> {
    if comparison != BytecodeComparison::LessThan || body + 2 != tail {
        return None;
    }

    // SAFETY: the numeric-loop proof covers the instruction, registers, and types.
    let word = |at: usize| unsafe { InstructionWord::read(chunk.code.as_ptr().add(at)) };
    let Instruction::IndexGet {
        destination: element,
        container: source,
        index,
        // SAFETY: the numeric-loop proof covers the instruction, registers, and types.
    } = (unsafe { word(body).decode() })
    else {
        return None;
    };
    if index != counter {
        return None;
    }

    let Instruction::Concatenate {
        destination: accumulator,
        left,
        right,
        // SAFETY: the numeric-loop proof covers the instruction, registers, and types.
    } = (unsafe { word(body + 1).decode() })
    else {
        return None;
    };
    if accumulator != left || right != element || accumulator == source {
        return None;
    }

    let Instruction::IntCounterLoop {
        comparison: tail_comparison,
        counter: tail_counter,
        limit: tail_limit,
        offset,
        // SAFETY: the numeric-loop proof covers the instruction, registers, and types.
    } = (unsafe { word(tail).decode() })
    else {
        return None;
    };
    if tail_comparison != comparison
        || tail_counter != counter
        || tail_limit != limit
        || relative_target(tail, i32::from(offset.offset())) != body
        || values.kind(counter.index() as usize) != NumericKind::Int
        || values.kind(limit.index() as usize) != NumericKind::Int
        || values.kind(accumulator.index() as usize) != NumericKind::Other
    {
        return None;
    }

    // SAFETY: the numeric-loop proof covers the instruction, registers, and types.
    let source_value = unsafe { &*registers.add(source.index() as usize) };
    let source_bytes = match source_value.transparent() {
        ValueView::String(string) => string.flatten(),
        ValueView::ShortString(string) => string.as_bytes(),
        _ => return None,
    };
    let ValueView::String(target) =
        // SAFETY: the numeric-loop proof covers the instruction, registers, and types.
        (unsafe { &*registers.add(accumulator.index() as usize) }).transparent()
    else {
        return None;
    };
    if !target.is_unique() {
        return None;
    }

    let start = values.int(counter.index() as usize);
    let end = values.int(limit.index() as usize);
    let Ok(start) = usize::try_from(start) else {
        return None;
    };
    let Ok(end) = usize::try_from(end) else {
        return None;
    };
    if start > end || end > source_bytes.len() {
        return None;
    }

    let remaining = end - start;
    if remaining == 0 {
        return Some(exit);
    }
    let count = remaining.min(BATCH_ITERATION_LIMIT as usize);
    // SAFETY: the numeric-loop proof covers the instruction, registers, and types.
    if !unsafe { ByteStringObject::append_unique(target, &source_bytes[start..start + count]) } {
        return None;
    }

    if count == remaining {
        assign_existing_int(values, dirty, counter, end as i64);
        Some(exit)
    } else {
        assign_existing_int(values, dirty, counter, (start + count - 1) as i64);
        Some(tail)
    }
}

/// # Safety
///
/// `registers` must be the live register window for `chunk` and the shadow
/// state must describe it.
#[inline(never)]
unsafe fn try_concat_burst(
    chunk: &Chunk,
    registers: *mut Value,
    values: &mut NumericRegisters,
    dirty: &mut u64,
    header: usize,
    tail: usize,
) -> Option<usize> {
    if tail != header + 4 {
        return None;
    }
    // SAFETY: the numeric-loop proof covers the instruction, registers, and types.
    let word = |at: usize| unsafe { InstructionWord::read(chunk.code.as_ptr().add(at)) };
    if word(header).kind() != InstructionKind::Move {
        return None;
    }
    let Instruction::Move {
        destination: probe,
        source: counter,
        // SAFETY: the numeric-loop proof covers the instruction, registers, and types.
    } = (unsafe { word(header).decode() })
    else {
        return None;
    };
    if word(header + 1).kind() != InstructionKind::SubtractImmediate {
        return None;
    }
    let Instruction::SubtractImmediate {
        destination: stepped,
        source: step_source,
        immediate,
        // SAFETY: the numeric-loop proof covers the instruction, registers, and types.
    } = (unsafe { word(header + 1).decode() })
    else {
        return None;
    };
    if stepped != counter || step_source != counter {
        return None;
    }
    if word(header + 2).kind() != InstructionKind::IntJumpUnlessImmediate {
        return None;
    }
    let Instruction::IntJumpUnlessImmediate {
        comparison,
        source: guarded,
        immediate: threshold,
        offset,
        // SAFETY: the numeric-loop proof covers the instruction, registers, and types.
    } = (unsafe { word(header + 2).decode() })
    else {
        return None;
    };
    if guarded != probe || relative_target(header + 2, i32::from(offset.offset())) != tail + 1 {
        return None;
    }
    if word(header + 3).kind() != InstructionKind::Concatenate {
        return None;
    }
    let Instruction::Concatenate {
        destination: accumulator,
        left,
        right,
        // SAFETY: the numeric-loop proof covers the instruction, registers, and types.
    } = (unsafe { word(header + 3).decode() })
    else {
        return None;
    };
    if accumulator != left
        || right == left
        || right == counter
        || right == probe
        || accumulator == counter
        || accumulator == probe
        || values.kind(counter.index() as usize) != NumericKind::Int
        || values.kind(probe.index() as usize) != NumericKind::Int
        || values.kind(accumulator.index() as usize) != NumericKind::Other
        || values.kind(right.index() as usize) != NumericKind::Other
    {
        return None;
    }

    let step = i64::from(immediate.value());
    let mut count = values.int(counter.index() as usize);
    let mut probe_value;
    let mut budget: u32 = BATCH_ITERATION_LIMIT;
    loop {
        probe_value = count;
        let Some(next) = count.checked_sub(step) else {
            assign_existing_int(values, dirty, counter, count);
            assign_existing_int(values, dirty, probe, probe_value);
            return Some(header + 1);
        };
        count = next;
        if !int_comparison_matches_any(comparison, probe_value, i64::from(threshold.value())) {
            assign_existing_int(values, dirty, counter, count);
            assign_existing_int(values, dirty, probe, probe_value);
            return Some(tail + 1);
        }
        let appended = {
            // SAFETY: other-kind shadows never shadow their live register.
            let target = unsafe { &*registers.add(accumulator.index() as usize) };
            // SAFETY: the numeric-loop proof covers the instruction, registers, and types.
            let extra = unsafe { &*registers.add(right.index() as usize) };
            match (target.transparent(), extra.transparent()) {
                // SAFETY: the numeric-loop proof covers the instruction, registers, and types.
                (ValueView::String(target), ValueView::String(extra)) => unsafe {
                    ByteStringObject::append_unique(target, extra.flatten())
                },
                // SAFETY: the numeric-loop proof covers the instruction, registers, and types.
                (ValueView::String(target), ValueView::ShortString(extra)) => unsafe {
                    ByteStringObject::append_unique(target, extra.as_bytes())
                },
                _ => false,
            }
        };
        if !appended {
            assign_existing_int(values, dirty, counter, count);
            assign_existing_int(values, dirty, probe, probe_value);
            return Some(header + 3);
        }
        budget -= 1;
        if budget == 0 {
            assign_existing_int(values, dirty, counter, count);
            assign_existing_int(values, dirty, probe, probe_value);
            return Some(tail);
        }
    }
}

#[inline(always)]
unsafe fn assign_array_element(
    registers: *mut Value,
    values: &mut NumericRegisters,
    dirty: &mut u64,
    pins: &mut Pins,
    destination: Register,
    element: &Value,
) {
    let value = match element.transparent() {
        ValueView::Int(element) => NumericValue::int(*element),
        ValueView::Float(element) => NumericValue::float(*element),
        ValueView::Bool(element) => NumericValue::bool(*element),
        _ => {
            let index = destination.index() as usize;
            // SAFETY: the numeric-loop proof covers the instruction, registers, and types.
            unsafe { *registers.add(index) = element.clone() };
            values.set(index, NumericValue::OTHER);
            *dirty &= !(1u64 << index);
            pins.invalidate(index);
            return;
        }
    };
    // SAFETY: the numeric-loop proof covers the instruction, registers, and types.
    unsafe { assign(registers, values, dirty, pins, destination, value) };
}

#[inline(always)]
unsafe fn assign(
    registers: *mut Value,
    values: &mut NumericRegisters,
    dirty: &mut u64,
    pins: &mut Pins,
    destination: Register,
    value: NumericValue,
) {
    let index = destination.index() as usize;
    if values.kind(index) == value.kind {
        values.set_bits(index, value.bits);
        return;
    }

    // SAFETY: the numeric-loop proof covers the instruction, registers, and types.
    unsafe { assign_changed_kind(registers, values, dirty, pins, index, value) };
}

#[cold]
#[inline(never)]
unsafe fn assign_changed_kind(
    registers: *mut Value,
    values: &mut NumericRegisters,
    dirty: &mut u64,
    pins: &mut Pins,
    index: usize,
    value: NumericValue,
) {
    if values.kind(index) == NumericKind::Other {
        // SAFETY: the numeric-loop proof covers the instruction, registers, and types.
        unsafe { *registers.add(index) = value.into_value() };
        pins.invalidate(index);
    }
    values.set(index, value);
    *dirty |= 1u64 << index;
}

#[inline(always)]
unsafe fn assign_float<const PREPARED_FLOATS: bool>(
    registers: *mut Value,
    values: &mut NumericRegisters,
    dirty: &mut u64,
    pins: &mut Pins,
    destination: Register,
    value: f64,
) {
    let index = destination.index() as usize;
    if PREPARED_FLOATS {
        debug_assert!(values.kind(index) == NumericKind::Float);
    } else if values.kind(index) != NumericKind::Float {
        // SAFETY: the numeric-loop proof covers the instruction, registers, and types.
        unsafe {
            assign_changed_kind(
                registers,
                values,
                dirty,
                pins,
                index,
                NumericValue::float(value),
            )
        };
        return;
    }
    values.set_bits(index, value.to_bits());
}

struct Pins {
    pointers: [MaybeUninit<(*mut Value, usize)>; NUMERIC_LOOP_REGISTER_LIMIT as usize],
    pinned: u64,
    writable: u64,
}

impl Pins {
    fn new() -> Pins {
        Pins {
            pointers: [MaybeUninit::uninit(); NUMERIC_LOOP_REGISTER_LIMIT as usize],
            pinned: 0,
            writable: 0,
        }
    }

    #[inline(always)]
    fn invalidate(&mut self, index: usize) {
        let clear = !(1u64 << index);
        self.pinned &= clear;
        self.writable &= clear;
    }

    /// # Safety
    ///
    /// `registers` must be the live register window and the pinned register
    /// must not have been overwritten since it was pinned.
    #[inline(always)]
    unsafe fn for_read(
        &mut self,
        registers: *mut Value,
        register: Register,
    ) -> Option<(*mut Value, usize)> {
        let index = register.index() as usize;
        let bit = 1u64 << index;
        if self.pinned & bit != 0 {
            // SAFETY: the surrounding invariant keeps this index in bounds.
            return Some(unsafe { self.pointers.get_unchecked(index).assume_init() });
        }
        // SAFETY: the register window is live and pinned buffers are
        // invalidated whenever their register is overwritten.
        match unsafe { &*registers.add(index) }.transparent() {
            ValueView::Vec(vec) => {
                let slice = vec.as_slice();
                let pin = (slice.as_ptr().cast_mut(), slice.len());
                self.pointers[index].write(pin);
                self.pinned |= bit;
                Some(pin)
            }
            _ => None,
        }
    }

    /// # Safety
    ///
    /// `registers` must be the live register window and the pinned register
    /// must not have been overwritten since it was pinned.
    #[inline(always)]
    unsafe fn for_write(
        &mut self,
        registers: *mut Value,
        register: Register,
    ) -> Option<(*mut Value, usize)> {
        let index = register.index() as usize;
        let bit = 1u64 << index;
        if self.pinned & self.writable & bit != 0 {
            // SAFETY: the surrounding invariant keeps this index in bounds.
            return Some(unsafe { self.pointers.get_unchecked(index).assume_init() });
        }
        // SAFETY: the register window is live and pinned buffers are
        // invalidated whenever their register is overwritten.
        match unsafe { &mut *registers.add(index) }.as_vec_mut() {
            Some(vec) => {
                let slice = vec.make_mut().as_mut_slice();
                let pin = (slice.as_mut_ptr(), slice.len());
                self.pointers[index].write(pin);
                self.pinned |= bit;
                self.writable |= bit;
                Some(pin)
            }
            _ => None,
        }
    }

    /// # Safety
    ///
    /// `registers` must be the live register window and the pinned register
    /// must not have been overwritten since it was pinned.
    #[inline(always)]
    unsafe fn for_read_dict(
        &mut self,
        registers: *mut Value,
        register: Register,
    ) -> Option<(*mut Value, usize)> {
        let index = register.index() as usize;
        let bit = 1u64 << index;
        if self.pinned & bit != 0 {
            // SAFETY: the surrounding invariant keeps this index in bounds.
            return Some(unsafe { self.pointers.get_unchecked(index).assume_init() });
        }
        // SAFETY: the register window is live and pinned buffers are
        // invalidated whenever their register is overwritten.
        match unsafe { &*registers.add(index) }.transparent() {
            ValueView::Dict(dict) => {
                let values = dict.packed_values()?;
                let pin = (values.as_ptr().cast_mut(), values.len());
                self.pointers[index].write(pin);
                self.pinned |= bit;
                Some(pin)
            }
            _ => None,
        }
    }

    /// # Safety
    ///
    /// `registers` must be the live register window and the pinned register
    /// must not have been overwritten since it was pinned.
    #[inline(always)]
    unsafe fn for_write_dict(
        &mut self,
        registers: *mut Value,
        register: Register,
    ) -> Option<(*mut Value, usize)> {
        let index = register.index() as usize;
        let bit = 1u64 << index;
        if self.pinned & self.writable & bit != 0 {
            // SAFETY: the surrounding invariant keeps this index in bounds.
            return Some(unsafe { self.pointers.get_unchecked(index).assume_init() });
        }
        // SAFETY: the register window is live and pinned buffers are
        // invalidated whenever their register is overwritten.
        match unsafe { &mut *registers.add(index) }.as_dict_mut() {
            Some(dict) => {
                let values = dict.make_mut().packed_values_for_pin()?;
                let pin = (values.as_mut_ptr(), values.len());
                self.pointers[index].write(pin);
                self.pinned |= bit;
                self.writable |= bit;
                Some(pin)
            }
            _ => None,
        }
    }
}

#[inline(always)]
fn assign_existing_numeric(
    values: &mut NumericRegisters,
    dirty: &mut u64,
    destination: Register,
    value: NumericValue,
) {
    let index = destination.index() as usize;
    debug_assert!(values.kind(index) == value.kind);
    debug_assert!(value.kind != NumericKind::Other);
    debug_assert!(*dirty & (1u64 << index) != 0);
    let _ = dirty;
    values.set_bits(index, value.bits);
}

#[inline(always)]
fn assign_existing_int(
    values: &mut NumericRegisters,
    dirty: &mut u64,
    destination: Register,
    value: i64,
) {
    let index = destination.index() as usize;
    debug_assert!(values.kind(index) == NumericKind::Int);
    debug_assert!(*dirty & (1u64 << index) != 0);
    let _ = dirty;
    values.set_bits(index, value as u64);
}

unsafe fn flush(registers: *mut Value, values: &NumericRegisters, mut dirty: u64) {
    while dirty != 0 {
        let index = dirty.trailing_zeros() as usize;
        // SAFETY: the numeric-loop proof covers the instruction, registers, and types.
        unsafe { registers.add(index).write(values.get(index).into_value()) };
        dirty &= dirty - 1;
    }
}
