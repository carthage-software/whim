//! Capacity reservation for fresh arrays populated by counted loops.

use whim_span::Span;

use crate::bytecode::chunk::Chunk;
use crate::bytecode::instruction::Instruction;
use crate::bytecode::instruction::operands::Comparison;
use crate::bytecode::instruction::operands::Register;
use crate::optimizer::OptimizationConfiguration;
use crate::optimizer::cfg::relative_target;
use crate::optimizer::liveness::effect::effect_on;
use crate::optimizer::rewrite::splice::insert_straight_line_before_many;
use crate::optimizer::rewrite::splice::straight_line_insertion_boundaries;

pub(in crate::optimizer) fn optimize_chunk(
    chunk: &mut Chunk,
    configuration: OptimizationConfiguration,
) {
    if !configuration.reserve_counted_arrays || chunk.code.len() < 4 {
        return;
    }

    let mut boundaries = None;
    let mut insertions = Vec::new();
    for tail in 0..chunk.code.len() {
        let Instruction::IntCounterLoop {
            comparison,
            counter,
            limit,
            offset,
        } = chunk.code[tail]
        else {
            continue;
        };
        if !matches!(
            comparison,
            Comparison::LessThan | Comparison::LessThanOrEqual
        ) {
            continue;
        }

        let body = relative_target(tail, i32::from(offset.offset()));
        let Some(header) = body.checked_sub(1) else {
            continue;
        };
        let Instruction::IntJumpUnless {
            comparison: header_comparison,
            left,
            right,
            offset: header_offset,
        } = chunk.code[header]
        else {
            continue;
        };
        if header_comparison != comparison
            || left != counter
            || right != limit
            || relative_target(header, i32::from(header_offset.offset())) != tail + 1
        {
            continue;
        }

        let mut containers = Vec::new();
        for instruction in &chunk.code[body..tail] {
            let container = match *instruction {
                Instruction::VecAppend { container, .. }
                | Instruction::VecIndexSet { container, .. }
                | Instruction::DictIndexSetIntKey { container, .. }
                | Instruction::DictIndexSetStringKey { container, .. }
                | Instruction::DictIndexSet { container, .. } => container,
                _ => continue,
            };
            if !containers.contains(&container)
                && fresh_array_before(chunk, container, header)
                && chunk.code[header..=tail]
                    .iter()
                    .all(|instruction| !effect_on(chunk, *instruction, container).writes())
            {
                containers.push(container);
            }
        }

        if !containers.is_empty()
            && boundaries.get_or_insert_with(|| straight_line_insertion_boundaries(chunk))[header]
        {
            insertions.push((header, limit, containers));
        }
    }

    let insertions = insertions
        .into_iter()
        .map(|(header, limit, containers)| {
            let span = chunk.spans.get(header).copied().unwrap_or_else(Span::zero);
            let instructions = containers
                .into_iter()
                .map(|container| {
                    (
                        Instruction::ReserveArray {
                            container,
                            additional: limit,
                        },
                        span,
                    )
                })
                .collect::<Vec<_>>();
            (header, instructions)
        })
        .collect::<Vec<_>>();
    let inserted = insert_straight_line_before_many(chunk, &insertions);
    debug_assert!(
        inserted,
        "insertion boundaries were checked before rewriting"
    );
}

fn fresh_array_before(chunk: &Chunk, container: Register, header: usize) -> bool {
    for index in (0..header).rev() {
        let effect = effect_on(chunk, chunk.code[index], container);
        if effect.reads() {
            return false;
        }
        if effect.writes() {
            return match chunk.code[index] {
                Instruction::NewVec {
                    element_count,
                    destination,
                    ..
                }
                | Instruction::NewDict {
                    pair_count: element_count,
                    destination,
                    ..
                } => destination == container && element_count.value() == 0,
                _ => false,
            };
        }
    }

    false
}
