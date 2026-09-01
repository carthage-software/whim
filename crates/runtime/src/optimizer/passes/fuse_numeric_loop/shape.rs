use crate::optimizer::passes::fuse_numeric_loop::BytecodeComparison;
use crate::optimizer::passes::fuse_numeric_loop::Chunk;
use crate::optimizer::passes::fuse_numeric_loop::Instruction;
use crate::optimizer::passes::fuse_numeric_loop::Literal;
use crate::optimizer::passes::fuse_numeric_loop::Register;
use crate::optimizer::passes::fuse_numeric_loop::effect_on;
use crate::optimizer::passes::fuse_numeric_loop::relative_target;
use crate::optimizer::passes::fuse_numeric_loop::successors;

pub(super) fn region_profits(chunk: &Chunk, header: usize, tail: usize) -> bool {
    let mut integer_operations = 0;

    for index in header + 1..=tail {
        match chunk.code[index] {
            Instruction::FloatAdd { .. }
            | Instruction::FloatSubtract { .. }
            | Instruction::FloatMultiply { .. }
            | Instruction::FloatMultiplyConstant { .. }
            | Instruction::FloatDifferenceAdd { .. }
            | Instruction::FloatScaleProductAdd { .. }
            | Instruction::FloatPairUpdate { .. }
            | Instruction::Squares { .. }
            | Instruction::FloatSquares { .. }
            | Instruction::FloatSquaresSum { .. }
            | Instruction::FloatSquaresSumBranch { .. }
            | Instruction::IndexGet { .. }
            | Instruction::IndexSet { .. }
            | Instruction::VecIndexGet { .. }
            | Instruction::VecIndexSet { .. }
            | Instruction::VecAppend { .. }
            | Instruction::DictIndexGetIntKey { .. }
            | Instruction::DictIndexSetIntKey { .. } => return true,
            Instruction::IntAdd { .. }
            | Instruction::IntSubtract { .. }
            | Instruction::IntMultiply { .. }
            | Instruction::IntModulo { .. }
            | Instruction::IntMultiplyImmediate { .. }
            | Instruction::IntModuloImmediate { .. }
            | Instruction::IntBitwiseAnd { .. }
            | Instruction::IntBitwiseOr { .. }
            | Instruction::IntBitwiseXor { .. }
            | Instruction::IntShiftLeft { .. }
            | Instruction::IntShiftRight { .. } => {
                integer_operations += 1;
                if integer_operations >= 3 {
                    return true;
                }
            }
            Instruction::Concatenate {
                destination, left, ..
            } if destination == left => return true,
            Instruction::AddImmediate { .. } | Instruction::IntAddAssign { .. }
                if index < tail
                    && matches!(chunk.code[index + 1], Instruction::IntCounterLoop { .. }) =>
            {
                return true;
            }
            _ => {}
        }
    }

    false
}

pub(super) fn writes_pinned_container(chunk: &Chunk, header: usize, tail: usize) -> bool {
    let mut containers = 0u64;
    for instruction in chunk.code[header..=tail].iter().copied() {
        match instruction {
            Instruction::IndexGet { container, .. }
            | Instruction::IndexSet { container, .. }
            | Instruction::VecIndexGet { container, .. }
            | Instruction::VecIndexSet { container, .. }
            | Instruction::DictIndexGetIntKey { container, .. }
            | Instruction::DictIndexSetIntKey { container, .. } => {
                containers |= 1u64 << u32::from(container.index());
            }
            _ => {}
        }
    }

    if containers == 0 {
        return false;
    }

    for instruction in chunk.code[header..=tail].iter().copied() {
        if matches!(
            instruction,
            Instruction::IndexGet { .. }
                | Instruction::VecIndexGet { .. }
                | Instruction::DictIndexGetIntKey { .. }
        ) {
            continue;
        }

        let mut remaining = containers;
        while remaining != 0 {
            let register = Register::new(remaining.trailing_zeros() as u16);
            if effect_on(chunk, instruction, register).writes() {
                return true;
            }

            remaining &= remaining - 1;
        }
    }

    false
}

pub(super) fn float_write_mask(chunk: &Chunk, start: usize, end: usize) -> u64 {
    let mut registers = 0u64;

    for instruction in chunk.code[start..end].iter().copied() {
        registers |= float_destinations(chunk, instruction);
    }

    let mut candidates = registers;
    while candidates != 0 {
        let index = candidates.trailing_zeros() as u16;
        let register = Register::new(index);
        let bit = 1u64 << u32::from(index);
        for instruction in chunk.code[start..end].iter().copied() {
            if float_destinations(chunk, instruction) & bit == 0
                && effect_on(chunk, instruction, register).writes()
            {
                return 0;
            }
        }

        candidates &= candidates - 1;
    }

    registers
}

fn float_destinations(chunk: &Chunk, instruction: Instruction) -> u64 {
    let one = |register: Register| 1u64 << u32::from(register.index());
    match instruction {
        Instruction::FloatAdd { destination, .. }
        | Instruction::FloatSubtract { destination, .. }
        | Instruction::FloatMultiply { destination, .. }
        | Instruction::FloatMultiplyConstant { destination, .. }
        | Instruction::FloatDifferenceAdd { destination, .. }
        | Instruction::FloatScaleProductAdd { destination, .. } => one(destination),
        Instruction::FloatSquares {
            first_destination, ..
        } => one(first_destination) | one(Register::new(first_destination.index() + 1)),
        Instruction::FloatSquaresSum {
            first_destination, ..
        } => {
            one(first_destination)
                | one(Register::new(first_destination.index() + 1))
                | one(Register::new(first_destination.index() + 2))
        }
        Instruction::FloatSquaresSumBranch { descriptor, .. } => {
            let descriptor = chunk.float_squares_sum_branch_descriptor(descriptor);

            one(descriptor.sum_destination)
                | one(descriptor.first_square_destination)
                | one(descriptor.second_square_destination)
        }
        Instruction::FloatPairUpdate { descriptor } => {
            let descriptor = chunk.float_pair_update_descriptor(descriptor);
            one(descriptor.first_destination) | one(descriptor.second_destination)
        }
        _ => 0,
    }
}

fn dict_writes_covered(chunk: &Chunk, header: usize, tail: usize) -> bool {
    let mut written = 0u64;
    let mut read = 0u64;
    for instruction in chunk.code[header + 1..=tail].iter().copied() {
        match instruction {
            Instruction::DictIndexSetIntKey { container, .. } => {
                written |= 1u64 << u32::from(container.index());
            }
            Instruction::DictIndexGetIntKey { container, .. } => {
                read |= 1u64 << u32::from(container.index());
            }
            _ => {}
        }
    }

    written & !read == 0
        || dict_copy_shape(chunk, header, tail)
        || dict_build_shape(chunk, header, tail)
}

fn dict_build_shape(chunk: &Chunk, header: usize, tail: usize) -> bool {
    let Instruction::IntCounterLoop { counter, .. } = chunk.code[tail] else {
        return false;
    };

    let body = &chunk.code[header + 1..tail];
    if body.is_empty() || body.len().is_multiple_of(2) {
        return false;
    }

    let Instruction::DictIndexSetIntKey {
        container, value, ..
    } = body[0]
    else {
        return false;
    };

    let mut step = None;
    for (position, instruction) in body.iter().copied().enumerate() {
        if position % 2 == 0 {
            let Instruction::DictIndexSetIntKey {
                container: store_container,
                index,
                value: store_value,
            } = instruction
            else {
                return false;
            };

            if store_container != container || index != counter || store_value != value {
                return false;
            }
        } else {
            let Instruction::AddImmediate {
                destination,
                source,
                immediate,
            } = instruction
            else {
                return false;
            };

            if destination != counter
                || source != counter
                || step
                    .replace(immediate)
                    .is_some_and(|seen| seen != immediate)
            {
                return false;
            }
        }
    }

    true
}

fn dict_copy_shape(chunk: &Chunk, header: usize, tail: usize) -> bool {
    if !matches!(chunk.code[tail], Instruction::Jump { .. }) {
        return false;
    }

    let body = &chunk.code[header + 1..tail];
    if body.len() < 3 || !body.len().is_multiple_of(3) {
        return false;
    }

    let Instruction::DictIndexGetIntKey {
        destination: temp,
        container: source,
        index: counter,
        ..
    } = body[0]
    else {
        return false;
    };

    for triplet in body.as_chunks::<3>().0 {
        let Instruction::DictIndexGetIntKey {
            destination,
            container,
            index,
            ..
        } = triplet[0]
        else {
            return false;
        };

        let Instruction::DictIndexSetIntKey {
            container: target,
            index: store_index,
            value,
        } = triplet[1]
        else {
            return false;
        };

        let Instruction::SubtractImmediate {
            destination: step_destination,
            source: step_source,
            ..
        } = triplet[2]
        else {
            return false;
        };

        if destination != temp
            || container != source
            || index != counter
            || store_index != counter
            || value != temp
            || target == source
            || step_destination != counter
            || step_source != counter
            || temp == counter
            || temp == source
            || temp == target
        {
            return false;
        }
    }

    true
}

pub(super) fn ordered(comparison: BytecodeComparison) -> bool {
    matches!(
        comparison,
        BytecodeComparison::LessThan
            | BytecodeComparison::LessThanOrEqual
            | BytecodeComparison::GreaterThan
            | BytecodeComparison::GreaterThanOrEqual
    )
}

pub(super) fn closed_numeric_body(chunk: &Chunk, header: usize, tail: usize, exit: usize) -> bool {
    if chunk.catch_table.iter().any(|entry| {
        let handler = entry.handler as usize;
        handler > header && handler <= tail
    }) {
        return false;
    }

    if !dict_writes_covered(chunk, header, tail) {
        return false;
    }

    for index in header + 1..=tail {
        match chunk.code[index] {
            Instruction::LoadConstant { constant, .. } => {
                if !matches!(
                    chunk.constants[constant.index() as usize],
                    Literal::Int(_) | Literal::Float(_) | Literal::String(_)
                ) {
                    return false;
                }
            }
            Instruction::StringLength { .. }
            | Instruction::LoadInt { .. }
            | Instruction::LoadTrue { .. }
            | Instruction::LoadFalse { .. }
            | Instruction::Move { .. }
            | Instruction::MoveOwned { .. }
            | Instruction::Add { .. }
            | Instruction::Subtract { .. }
            | Instruction::Multiply { .. }
            | Instruction::IntAdd { .. }
            | Instruction::IntSubtract { .. }
            | Instruction::IntMultiply { .. }
            | Instruction::IntModulo { .. }
            | Instruction::IntMultiplyImmediate { .. }
            | Instruction::IntModuloImmediate { .. }
            | Instruction::IntBitwiseAnd { .. }
            | Instruction::IntBitwiseOr { .. }
            | Instruction::IntBitwiseXor { .. }
            | Instruction::IntShiftLeft { .. }
            | Instruction::IntShiftRight { .. }
            | Instruction::IntAddAssign { .. }
            | Instruction::FloatAdd { .. }
            | Instruction::FloatSubtract { .. }
            | Instruction::FloatMultiply { .. }
            | Instruction::FloatMultiplyConstant { .. }
            | Instruction::FloatDifferenceAdd { .. }
            | Instruction::FloatScaleProductAdd { .. }
            | Instruction::FloatPairUpdate { .. }
            | Instruction::AddImmediate { .. }
            | Instruction::SubtractImmediate { .. }
            | Instruction::Squares { .. }
            | Instruction::FloatSquares { .. }
            | Instruction::FloatSquaresSum { .. }
            | Instruction::LessThan { .. }
            | Instruction::LessThanOrEqual { .. }
            | Instruction::GreaterThan { .. }
            | Instruction::GreaterThanOrEqual { .. }
            | Instruction::Equal { .. }
            | Instruction::NotEqual { .. }
            | Instruction::CheckDefined { .. }
            | Instruction::ShiftLeft { .. }
            | Instruction::ShiftRight { .. }
            | Instruction::BitwiseAnd { .. }
            | Instruction::BitwiseOr { .. }
            | Instruction::BitwiseXor { .. }
            | Instruction::IndexGet { .. }
            | Instruction::IndexSet { .. }
            | Instruction::VecIndexGet { .. }
            | Instruction::VecIndexSet { .. }
            | Instruction::VecAppend { .. }
            | Instruction::DictIndexGetIntKey { .. }
            | Instruction::DictIndexSetIntKey { .. }
            | Instruction::Concatenate { .. }
            | Instruction::Return { .. }
            | Instruction::ReturnUnchecked { .. }
            | Instruction::ReturnReferenceUnchecked { .. }
            | Instruction::ReturnPairUnchecked { .. }
            | Instruction::ReturnScalarUnchecked { .. }
            | Instruction::ReturnIntUnchecked { .. }
            | Instruction::ReturnNull
            | Instruction::ReturnNullUnchecked => {}
            Instruction::JumpIfFalse { offset, .. }
            | Instruction::JumpIfTrue { offset, .. }
            | Instruction::Jump { offset }
            | Instruction::NumericRegionJump { offset } => {
                let target = relative_target(index, offset.offset());
                if target < header || target > exit {
                    return false;
                }
            }
            Instruction::JumpUnless {
                comparison, offset, ..
            }
            | Instruction::NumericLoop {
                comparison, offset, ..
            } => {
                let target = relative_target(index, i32::from(offset.offset()));
                if !ordered(comparison) || target < header || target > exit {
                    return false;
                }
            }
            Instruction::IntJumpUnless { offset, .. }
            | Instruction::IntJumpUnlessImmediate { offset, .. }
            | Instruction::IntNumericLoop { offset, .. } => {
                let target = relative_target(index, i32::from(offset.offset()));
                if target < header || target > exit {
                    return false;
                }
            }
            Instruction::PreparedIntNumericLoop { descriptor, offset } => {
                let _ = chunk.prepared_int_loop_descriptor(descriptor);

                let target = relative_target(index, i32::from(offset.offset()));
                if target < header || target > exit {
                    return false;
                }
            }
            Instruction::FloatSquaresSumBranch { descriptor, offset } => {
                let descriptor = chunk.float_squares_sum_branch_descriptor(descriptor);

                let target = relative_target(index, offset.offset());
                if !ordered(descriptor.comparison)
                    || !matches!(
                        chunk.constants[usize::from(descriptor.constant.index())],
                        Literal::Float(_)
                    )
                    || target < header + 1
                    || target > exit
                {
                    return false;
                }
            }
            Instruction::JumpUnlessConstant {
                comparison,
                constant,
                offset,
                ..
            } => {
                let target = relative_target(index, i32::from(offset.offset()));
                if !ordered(comparison)
                    || !matches!(
                        chunk.constants[constant.index() as usize],
                        Literal::Int(_) | Literal::Float(_)
                    )
                    || target < header + 1
                    || target > exit
                {
                    return false;
                }
            }
            Instruction::CounterLoop { offset, .. }
            | Instruction::IntCounterLoop { offset, .. }
                if index == tail =>
            {
                if relative_target(index, i32::from(offset.offset())) != header + 1 {
                    return false;
                }
            }
            Instruction::CounterLoop { offset, .. }
            | Instruction::IntCounterLoop { offset, .. } => {
                let target = relative_target(index, i32::from(offset.offset()));
                if target <= header || target > index {
                    return false;
                }
            }
            _ => return false,
        }
    }

    true
}

pub(super) fn has_external_entry(chunk: &Chunk, header: usize, tail: usize) -> bool {
    let mut targets = Vec::new();
    for index in 0..chunk.code.len() {
        if index >= header && index <= tail {
            continue;
        }

        targets.clear();
        successors(chunk, index, &mut targets);
        if targets
            .iter()
            .any(|target| *target > header && *target <= tail)
        {
            return true;
        }
    }

    false
}
