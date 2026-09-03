//! Coalescing of call results and interference-graph recoloring.

use std::mem;

use crate::optimizer::operands::Access;
use crate::optimizer::operands::Operand;
use crate::optimizer::operands::implicit_reads;
use crate::optimizer::operands::instruction_bytes;
use crate::optimizer::operands::operands;
use crate::optimizer::operands::register_at;
use crate::optimizer::operands::write_may_alias_inputs;
use crate::optimizer::passes::reuse_temporaries::Chunk;
use crate::optimizer::passes::reuse_temporaries::Instruction;
use crate::optimizer::passes::reuse_temporaries::OptimizationStatistics;
use crate::optimizer::passes::reuse_temporaries::Register;
use crate::optimizer::passes::reuse_temporaries::Reverse;
use crate::optimizer::passes::reuse_temporaries::control_flow_targets;
use crate::optimizer::passes::reuse_temporaries::normalize_empty_window_starts;
use crate::optimizer::passes::reuse_temporaries::pinned_high_water;
use crate::optimizer::passes::reuse_temporaries::pinned_window_registers;
use crate::optimizer::passes::reuse_temporaries::register_is_dead_after;
use crate::optimizer::passes::reuse_temporaries::remap_registers;
use crate::optimizer::passes::reuse_temporaries::successors;
use crate::unwrap_option_invariant;

/// Lets a one-argument exact call return into its consumed argument register
/// when the immediately following instruction replaces that same register
/// with a result derived from the call. The call already transfers the
/// argument into the callee, so its caller slot is free until the return.
pub(super) fn coalesce_single_argument_call_results(chunk: &mut Chunk) {
    if chunk.code.len() < 2 || !chunk.catch_table.is_empty() {
        return;
    }
    let targets = control_flow_targets(chunk);
    for index in 0..chunk.code.len() - 1 {
        if targets.contains(&(index + 1)) {
            continue;
        }

        let (destination, argument) = match chunk.code[index] {
            Instruction::CallNamedUnchecked {
                argument_count,
                destination,
                first_argument,
                ..
            }
            | Instruction::CallSelfUnchecked {
                argument_count,
                destination,
                first_argument,
            } if argument_count.value() == 1 => (destination, first_argument),
            _ => continue,
        };

        if destination == argument
            || !register_is_dead_after(chunk, destination, index + 2)
            || !register_is_dead_after(chunk, argument, index + 1)
        {
            continue;
        }

        let next = chunk.code[index + 1];
        let Some(next_operands) = operands(next.kind()) else {
            continue;
        };

        if !write_may_alias_inputs(next.kind()) {
            continue;
        }

        let bytes = instruction_bytes(next);
        let writes_argument = next_operands.iter().any(|operand| {
            operand.access == Access::Write && register_at(bytes, operand.offset) == argument
        });

        let reads_argument = next_operands.iter().any(|operand| {
            operand.access == Access::Read && register_at(bytes, operand.offset) == argument
        });

        let reads_destination = next_operands.iter().any(|operand| {
            operand.access == Access::Read && register_at(bytes, operand.offset) == destination
        });

        if !writes_argument || reads_argument || !reads_destination {
            continue;
        }

        chunk.code[index] = match chunk.code[index] {
            Instruction::CallNamedUnchecked {
                argument_count,
                destination: _,
                first_argument,
                cache,
            } => Instruction::CallNamedUnchecked {
                argument_count,
                destination: argument,
                first_argument,
                cache,
            },
            Instruction::CallSelfUnchecked {
                argument_count,
                destination: _,
                first_argument,
            } => Instruction::CallSelfUnchecked {
                argument_count,
                destination: argument,
                first_argument,
            },
            _ => unreachable!("the call shape was selected above"),
        };

        chunk.code[index + 1] = replace_read_register(next, next_operands, destination, argument);
    }
}

fn replace_read_register(
    instruction: Instruction,
    operands: &[Operand],
    from: Register,
    to: Register,
) -> Instruction {
    let mut bytes = instruction_bytes(instruction);
    for operand in operands {
        if operand.access == Access::Read && register_at(bytes, operand.offset) == from {
            let replacement = to.index().to_le_bytes();
            bytes[operand.offset] = replacement[0];
            bytes[operand.offset + 1] = replacement[1];
        }
    }

    // SAFETY: verified bytecode and this pass's guards prove the index.
    unsafe { mem::transmute::<[u8; 8], Instruction>(bytes) }
}

/// Recolors complete temporary live ranges after the cheap interval pass.
pub(super) fn recolor_interference_graph(
    chunk: &mut Chunk,
    statistics: &mut OptimizationStatistics,
) {
    let register_count = usize::from(chunk.register_count);
    let local_count = usize::from(chunk.local_register_count);
    if register_count <= local_count || !chunk.catch_table.is_empty() {
        return;
    }

    let pinned = pinned_window_registers(chunk);
    if pinned_high_water(&pinned, chunk.local_register_count) >= chunk.register_count {
        return;
    }

    let mut uses = Vec::with_capacity(chunk.code.len());
    let mut definitions = Vec::with_capacity(chunk.code.len());
    for instruction in chunk.code.iter().copied() {
        let Some(operands) = operands(instruction.kind()) else {
            return;
        };

        let bytes = instruction_bytes(instruction);
        let mut instruction_uses = Vec::new();
        let mut instruction_definitions = Vec::new();
        for operand in operands {
            if operand.offset == 4
                && matches!(
                    instruction,
                    Instruction::CallNamedUnchecked { argument_count, .. }
                        | Instruction::CallSelfUnchecked { argument_count, .. }
                        if argument_count.value() == 0
                )
            {
                continue;
            }
            let register = register_at(bytes, operand.offset);
            if register == Register::NONE {
                continue;
            }

            let target = match operand.access {
                Access::Read => &mut instruction_uses,
                Access::Write => &mut instruction_definitions,
            };

            if !target.contains(&register.index()) {
                target.push(register.index());
            }
        }

        if let Some((first, count)) = implicit_reads(instruction) {
            for register in first.index()..first.index() + count as u16 {
                if !instruction_uses.contains(&register) {
                    instruction_uses.push(register);
                }
            }
        }

        uses.push(instruction_uses);
        definitions.push(instruction_definitions);
    }

    let word_count = register_count.div_ceil(u64::BITS as usize);
    let mut live_in = vec![0_u64; chunk.code.len() * word_count];
    let mut successors_buffer = Vec::new();
    let mut incoming = vec![0_u64; word_count];
    loop {
        let mut changed = false;
        for index in (0..chunk.code.len()).rev() {
            successors_buffer.clear();
            successors(chunk, index, &mut successors_buffer);
            push_catch_successors(chunk, index, &mut successors_buffer);
            incoming.fill(0);
            for &successor in &successors_buffer {
                if successor < chunk.code.len() {
                    union_bits(&mut incoming, bit_row(&live_in, successor, word_count));
                }
            }

            for definition in &definitions[index] {
                remove_bit(&mut incoming, usize::from(*definition));
            }

            for used in &uses[index] {
                insert_bit(&mut incoming, usize::from(*used));
            }
            let current = bit_row_mut(&mut live_in, index, word_count);
            if incoming != current {
                current.copy_from_slice(&incoming);
                changed = true;
            }
        }

        if !changed {
            break;
        }
    }

    let mut interference = vec![0_u64; register_count * word_count];
    let mut live_out = vec![0_u64; word_count];
    for index in 0..chunk.code.len() {
        successors_buffer.clear();
        successors(chunk, index, &mut successors_buffer);
        push_catch_successors(chunk, index, &mut successors_buffer);
        live_out.fill(0);
        for &successor in &successors_buffer {
            if successor < chunk.code.len() {
                union_bits(&mut live_out, bit_row(&live_in, successor, word_count));
            }
        }

        for &definition in &definitions[index] {
            if usize::from(definition) < local_count {
                continue;
            }

            for live in local_count..register_count {
                if contains_bit(&live_out, live) {
                    add_interference(
                        &mut interference,
                        word_count,
                        definition,
                        live as u16,
                        local_count,
                    );
                }
            }

            if !write_may_alias_inputs(chunk.code[index].kind()) {
                for &used in &uses[index] {
                    add_interference(&mut interference, word_count, definition, used, local_count);
                }
            }

            for &other in &definitions[index] {
                add_interference(
                    &mut interference,
                    word_count,
                    definition,
                    other,
                    local_count,
                );
            }
        }
    }

    let mut order = (local_count..register_count)
        .filter(|register| !pinned.contains(&(*register as u16)))
        .collect::<Vec<_>>();

    order.sort_unstable_by_key(|register| {
        Reverse(
            bit_row(&interference, *register, word_count)
                .iter()
                .map(|word| word.count_ones())
                .sum::<u32>(),
        )
    });
    let mut mapping = vec![Register::NONE; register_count];
    for (register, target) in mapping.iter_mut().enumerate().take(local_count) {
        *target = Register::new(register as u16);
    }

    for register in &pinned {
        mapping[usize::from(*register)] = Register::new(*register);
    }

    let mut high_water = pinned_high_water(&pinned, chunk.local_register_count);
    let mut occupied = vec![false; register_count];
    for register in order {
        occupied.fill(false);
        let neighbors = bit_row(&interference, register, word_count);
        for (neighbor, mapped) in mapping
            .iter()
            .enumerate()
            .take(register_count)
            .skip(local_count)
        {
            if !contains_bit(neighbors, neighbor) {
                continue;
            }

            if *mapped != Register::NONE {
                occupied[usize::from(mapped.index())] = true;
            }
        }
        let physical = (local_count..register_count).find(|candidate| !occupied[*candidate]);
        // SAFETY: the surrounding invariant proves this option contains a value.
        let physical = unsafe {
            unwrap_option_invariant(
                physical,
                "recoloring never needs more registers than the original chunk",
            ) as u16
        };

        mapping[register] = Register::new(physical);
        high_water = high_water.max(physical + 1);
    }

    if high_water >= chunk.register_count {
        return;
    }

    for instruction in &mut chunk.code {
        *instruction = remap_registers(*instruction, &mapping);
    }

    statistics.registers_removed += usize::from(chunk.register_count - high_water);
    chunk.register_count = high_water;
    normalize_empty_window_starts(chunk);
}

fn push_catch_successors(chunk: &Chunk, index: usize, successors: &mut Vec<usize>) {
    for entry in &chunk.catch_table {
        if index >= entry.start as usize && index < entry.end as usize {
            successors.push(entry.handler as usize);
        }
    }
}

fn bit_row(bits: &[u64], row: usize, word_count: usize) -> &[u64] {
    let start = row * word_count;
    &bits[start..start + word_count]
}

fn bit_row_mut(bits: &mut [u64], row: usize, word_count: usize) -> &mut [u64] {
    let start = row * word_count;
    &mut bits[start..start + word_count]
}

fn union_bits(target: &mut [u64], source: &[u64]) {
    for (target, source) in target.iter_mut().zip(source) {
        *target |= source;
    }
}

fn contains_bit(bits: &[u64], register: usize) -> bool {
    bits[register / u64::BITS as usize] & (1 << (register % u64::BITS as usize)) != 0
}

fn insert_bit(bits: &mut [u64], register: usize) {
    bits[register / u64::BITS as usize] |= 1 << (register % u64::BITS as usize);
}

fn remove_bit(bits: &mut [u64], register: usize) {
    bits[register / u64::BITS as usize] &= !(1 << (register % u64::BITS as usize));
}

fn add_interference(
    interference: &mut [u64],
    word_count: usize,
    left: u16,
    right: u16,
    local_count: usize,
) {
    if left == right || usize::from(right) < local_count {
        return;
    }

    insert_bit(
        bit_row_mut(interference, usize::from(left), word_count),
        usize::from(right),
    );
    insert_bit(
        bit_row_mut(interference, usize::from(right), word_count),
        usize::from(left),
    );
}
