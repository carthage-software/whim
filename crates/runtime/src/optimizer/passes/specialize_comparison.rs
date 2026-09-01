//! Integer specialization of comparison branches.

use crate::bytecode::chunk::Chunk;
use crate::bytecode::chunk::descriptors::Literal;
use crate::bytecode::chunk::descriptors::TypeDescriptor;
use crate::bytecode::instruction::Instruction;
use crate::bytecode::instruction::operands::Comparison;
use crate::bytecode::instruction::operands::JumpOffset;
use crate::bytecode::instruction::operands::Register;
use crate::bytecode::instruction::operands::ShortJumpOffset;
use crate::bytecode::unit::CompiledParameter;
use crate::bytecode::unit::CompiledTypeParameter;
use crate::bytecode::unit::CompiledUnit;
use crate::optimizer::OptimizationConfiguration;
use crate::optimizer::OptimizationStatistics;
use crate::optimizer::analysis::Analysis;
use crate::optimizer::analysis::AnalyzedChunk;
use crate::optimizer::candidates::CandidateSet;
use crate::optimizer::cfg::control_flow_targets;
use crate::optimizer::cfg::relative_target;
use crate::optimizer::cfg::successors;
use crate::optimizer::liveness::effect::effect_on;
use crate::optimizer::liveness::effect::overwrites_register;
use crate::optimizer::liveness::register_is_dead_after;
use crate::optimizer::passes::compact_removed_instructions;
use crate::optimizer::passes::for_each_mutable_chunk;
use crate::optimizer::passes::fuse_comparison;
use crate::optimizer::passes::plan_type_specializations;
use crate::optimizer::rewrite::plan::RewritePlan;
use crate::optimizer::type_flow::ConstantValue;
use crate::optimizer::type_flow::TypeFlow;
use crate::value::atom::Atom;
use crate::value::heap::Heap;

pub(in crate::optimizer) fn optimize_unit(
    plan: &mut RewritePlan,
    analysis: &Analysis<'_>,
    configuration: OptimizationConfiguration,
    statistics: &mut OptimizationStatistics,
) {
    if !configuration.specialize_comparison {
        return;
    }

    statistics.operations_specialized += plan_type_specializations(
        plan,
        analysis,
        CandidateSet::COMPARISON,
        specialized_instruction,
    );
}

pub(in crate::optimizer) fn prepare_unit(
    unit: &mut CompiledUnit,
    configuration: OptimizationConfiguration,
    statistics: &mut OptimizationStatistics,
) {
    if !configuration.specialize_comparison {
        return;
    }

    for_each_mutable_chunk(unit, configuration, |chunk| {
        prepare_chunk(chunk, configuration, statistics);
    });
}

pub(in crate::optimizer) fn remove_boolean_literal_branches_unit(
    analysis: &Analysis<'_>,
    plan: &mut RewritePlan,
    configuration: OptimizationConfiguration,
    statistics: &mut OptimizationStatistics,
) -> bool {
    if !configuration.specialize_comparison {
        return false;
    }

    let mut changed = false;
    for analyzed in analysis.chunks() {
        if !analyzed.candidates.contains(CandidateSet::COMPARISON) {
            continue;
        }

        changed |= plan_boolean_literal_branches(analyzed, plan, statistics);
    }
    changed
}

pub(in crate::optimizer) fn specialize_reused_string_bytes_unit(
    analysis: &Analysis<'_>,
    plan: &mut RewritePlan,
    configuration: OptimizationConfiguration,
    statistics: &mut OptimizationStatistics,
) -> bool {
    if !configuration.specialize_comparison {
        return false;
    }

    let mut changed = false;
    for analyzed in analysis.chunks() {
        if !analyzed.candidates.contains(CandidateSet::COMPARISON) {
            continue;
        }

        changed |= plan_reused_string_bytes(analyzed, plan, statistics);
    }
    changed
}

fn prepare_chunk(
    chunk: &mut Chunk,
    configuration: OptimizationConfiguration,
    statistics: &mut OptimizationStatistics,
) {
    fuse_comparison::optimize_chunk(chunk, configuration, statistics);
    fuse_string_byte_branches(chunk, statistics);
}

pub(in crate::optimizer) fn optimize_chunk(
    chunk: &mut Chunk,
    allocator: &Heap,
    configuration: OptimizationConfiguration,
    statistics: &mut OptimizationStatistics,
) {
    optimize_chunk_with_context(
        chunk,
        &[],
        false,
        None,
        &[],
        allocator,
        configuration,
        statistics,
    );
}

#[expect(
    clippy::too_many_arguments,
    reason = "the pass receives borrowed callable context without allocating an aggregate"
)]
fn optimize_chunk_with_context(
    chunk: &mut Chunk,
    parameters: &[CompiledParameter],
    has_receiver: bool,
    class_name: Option<&Atom>,
    class_type_parameters: &[CompiledTypeParameter],
    allocator: &Heap,
    configuration: OptimizationConfiguration,
    statistics: &mut OptimizationStatistics,
) {
    if !configuration.specialize_comparison || chunk.code.is_empty() {
        return;
    }

    let image = chunk.clone();
    let flow = TypeFlow::analyze(
        &image,
        parameters,
        has_receiver,
        class_name,
        class_type_parameters,
        allocator,
    );
    fuse_boolean_literal_branches(chunk, &flow, statistics);
    specialize_bool_pattern_branches(chunk, &flow, statistics);
    drop(flow);
    prepare_chunk(chunk, configuration, statistics);

    let mut replacements = vec![None; chunk.code.len()];
    let image = chunk.clone();
    let flow = TypeFlow::analyze(
        &image,
        parameters,
        has_receiver,
        class_name,
        class_type_parameters,
        allocator,
    );

    for (index, instruction) in chunk.code.iter().copied().enumerate() {
        replacements[index] = specialized_instruction(&flow, index, instruction);
    }

    for (instruction, replacement) in chunk.code.iter_mut().zip(replacements) {
        if let Some(replacement) = replacement {
            *instruction = replacement;
            statistics.operations_specialized += 1;
        }
    }

    let image = chunk.clone();
    let flow = TypeFlow::analyze(
        &image,
        parameters,
        has_receiver,
        class_name,
        class_type_parameters,
        allocator,
    );
    specialize_reused_string_bytes(chunk, &flow, statistics);
}

fn specialize_bool_pattern_branches(
    chunk: &mut Chunk,
    flow: &TypeFlow<'_>,
    statistics: &mut OptimizationStatistics,
) {
    for (index, instruction) in chunk.code.iter_mut().enumerate() {
        let Instruction::BoolPatternBranch {
            subject,
            false_offset,
            ..
        } = *instruction
        else {
            continue;
        };
        if !flow.proves(index, subject, &TypeDescriptor::Bool) {
            continue;
        }
        *instruction = Instruction::JumpIfFalse {
            condition: subject,
            offset: JumpOffset::new(i32::from(false_offset.offset())),
        };
        statistics.operations_specialized += 1;
    }
}

pub(in crate::optimizer) fn specialized_instruction(
    flow: &TypeFlow<'_>,
    index: usize,
    instruction: Instruction,
) -> Option<Instruction> {
    if let Instruction::BoolPatternBranch {
        subject,
        false_offset,
        ..
    } = instruction
        && flow.proves(index, subject, &TypeDescriptor::Bool)
    {
        return Some(Instruction::JumpIfFalse {
            condition: subject,
            offset: JumpOffset::new(i32::from(false_offset.offset())),
        });
    }

    specialize_with(
        instruction,
        |register| flow.proves(index, register, &TypeDescriptor::Int),
        |register| flow.proves(index, register, &TypeDescriptor::String),
    )
}

pub(super) fn specialize_with(
    instruction: Instruction,
    is_int: impl Fn(Register) -> bool,
    is_string: impl Fn(Register) -> bool,
) -> Option<Instruction> {
    match instruction {
        Instruction::JumpUnless {
            comparison,
            left,
            right,
            offset,
        } if is_int(left) && is_int(right) => Some(Instruction::IntJumpUnless {
            comparison,
            left,
            right,
            offset,
        }),
        Instruction::JumpUnless {
            comparison,
            left,
            right,
            offset,
        } if is_string(left) && is_string(right) => Some(Instruction::StringJumpUnless {
            comparison,
            left,
            right,
            offset,
        }),
        Instruction::NumericLoop {
            comparison,
            left,
            right,
            offset,
        } if is_int(left) && is_int(right) => Some(Instruction::IntNumericLoop {
            comparison,
            left,
            right,
            offset,
        }),
        _ => None,
    }
}

fn plan_reused_string_bytes(
    analyzed: &AnalyzedChunk<'_>,
    plan: &mut RewritePlan,
    statistics: &mut OptimizationStatistics,
) -> bool {
    let chunk = analyzed.chunk;
    let flow = &analyzed.flow;
    let Some((replacements, remove)) = reused_string_byte_changes(chunk, flow) else {
        return false;
    };

    let mut specialized = 0;
    for (position, replacement) in replacements.into_iter().enumerate() {
        if let Some(replacement) = replacement {
            specialized += usize::from(plan.replace(analyzed, position, replacement));
        }
    }
    if specialized == 0 {
        return false;
    }
    let mut removed = 0;
    for (position, remove) in remove.into_iter().enumerate() {
        if remove {
            removed += usize::from(plan.remove(analyzed, position));
        }
    }
    statistics.instructions_removed += removed;
    statistics.operations_specialized += specialized;
    removed != 0
}

fn specialize_reused_string_bytes(
    chunk: &mut Chunk,
    flow: &TypeFlow<'_>,
    statistics: &mut OptimizationStatistics,
) -> bool {
    let Some((replacements, remove)) = reused_string_byte_changes(chunk, flow) else {
        return false;
    };

    let specialized = replacements
        .iter()
        .filter(|replacement| replacement.is_some())
        .count();
    if specialized == 0 {
        return false;
    }
    for (instruction, replacement) in chunk.code.iter_mut().zip(replacements) {
        if let Some(replacement) = replacement {
            *instruction = replacement;
        }
    }

    compact_removed_instructions(chunk, &remove, statistics);
    statistics.operations_specialized += specialized;
    true
}

fn reused_string_byte_changes(
    chunk: &Chunk,
    flow: &TypeFlow<'_>,
) -> Option<(Vec<Option<Instruction>>, Vec<bool>)> {
    if chunk.code.len() < 2 || !chunk.catch_table.is_empty() {
        return None;
    }

    let mut replacements = vec![None; chunk.code.len()];
    let mut remove = vec![false; chunk.code.len()];
    for definition in 0..remove.len() {
        let Instruction::StringIndexGet {
            destination: character,
            container,
            index,
        } = chunk.code[definition]
        else {
            continue;
        };

        let Some(ReusableStringByteUses {
            replacements: changes,
            clears,
        }) = reusable_string_byte_uses(chunk, flow, definition, character, container, index)
        else {
            continue;
        };
        if changes.is_empty() {
            continue;
        }

        remove[definition] = true;
        for position in clears {
            remove[position] = true;
        }
        for (position, replacement) in changes {
            if let Some(load) = redundant_byte_literal_load(chunk, flow, position, character) {
                remove[load] = true;
            }
            replacements[position] = Some(replacement);
        }
    }

    replacements
        .iter()
        .any(Option::is_some)
        .then_some((replacements, remove))
}

fn redundant_byte_literal_load(
    chunk: &Chunk,
    flow: &TypeFlow<'_>,
    position: usize,
    character: Register,
) -> Option<usize> {
    let load = position.checked_sub(1)?;
    let Instruction::LoadConstant {
        destination: literal,
        ..
    } = chunk.code[load]
    else {
        return None;
    };
    let other = match chunk.code[position] {
        Instruction::StringJumpUnless {
            left,
            right,
            offset,
            ..
        } => {
            let other = compared_register(character, left, right)?;
            let target = relative_target(position, i32::from(offset.offset()));
            if !register_is_dead_after(chunk, other, position + 1)
                || !register_is_dead_after(chunk, other, target)
            {
                return None;
            }
            other
        }
        instruction => {
            let (_, _, left, right) = comparison_parts(instruction)?;
            let other = compared_register(character, left, right)?;
            if !register_is_dead_after(chunk, other, position + 1) {
                return None;
            }
            other
        }
    };
    if literal != other || flow.register_may_release_observably(load, literal) {
        return None;
    }

    Some(load)
}

struct ReusableStringByteUses {
    replacements: Vec<(usize, Instruction)>,
    clears: Vec<usize>,
}

fn reusable_string_byte_uses(
    chunk: &Chunk,
    flow: &TypeFlow<'_>,
    definition: usize,
    character: Register,
    container: Register,
    string_index: Register,
) -> Option<ReusableStringByteUses> {
    let mut changes = Vec::new();
    let mut clears = Vec::new();
    let mut work = vec![(definition + 1, true)];
    let mut seen = vec![[false; 2]; chunk.code.len()];
    while let Some((position, stable)) = work.pop() {
        if position == chunk.code.len() || seen[position][usize::from(stable)] {
            continue;
        }
        seen[position][usize::from(stable)] = true;

        let instruction = chunk.code[position];
        if matches!(instruction, Instruction::Clear { target } if target == character) {
            clears.push(position);
            continue;
        }
        if stable
            && matches!(
                instruction,
                Instruction::Clear { target } if target == container
            )
        {
            clears.push(position);
            let mut next = Vec::new();
            successors(chunk, position, &mut next);
            work.extend(next.into_iter().map(|next| (next, true)));
            continue;
        }

        let character_effect = effect_on(chunk, instruction, character);
        if overwrites_register(chunk, instruction, character) {
            if character_effect.reads() {
                return None;
            }
            continue;
        }

        if character_effect.reads() {
            if !stable {
                return None;
            }
            let replacement = string_byte_use(
                flow,
                position,
                instruction,
                character,
                container,
                string_index,
            )?;
            changes.push((position, replacement));
        }

        let sources_stable = stable
            && !overwrites_register(chunk, instruction, container)
            && !overwrites_register(chunk, instruction, string_index);
        let mut next = Vec::new();
        successors(chunk, position, &mut next);
        work.extend(next.into_iter().map(|next| (next, sources_stable)));
    }

    Some(ReusableStringByteUses {
        replacements: changes,
        clears,
    })
}

fn string_byte_use(
    flow: &TypeFlow<'_>,
    position: usize,
    instruction: Instruction,
    character: Register,
    container: Register,
    string_index: Register,
) -> Option<Instruction> {
    if let Instruction::StringJumpUnless {
        comparison,
        left,
        right,
        offset,
    } = instruction
    {
        let other = compared_register(character, left, right)?;
        let byte = constant_byte(flow, position, other)?;
        return match comparison {
            Comparison::Equal => Some(Instruction::StringByteJumpUnlessEqual {
                container,
                index: string_index,
                byte,
                offset,
            }),
            Comparison::NotEqual => Some(Instruction::StringByteJumpUnlessNotEqual {
                container,
                index: string_index,
                byte,
                offset,
            }),
            _ => None,
        };
    }

    let (comparison, destination, left, right) = comparison_parts(instruction)?;
    if destination == character {
        return None;
    }
    let (other, comparison) = if left == character && right != character {
        (right, comparison)
    } else if right == character && left != character {
        (left, comparison.reversed())
    } else {
        return None;
    };
    let byte = constant_byte(flow, position, other)?;
    Some(string_byte_comparison(
        comparison,
        destination,
        container,
        string_index,
        byte,
    ))
}

fn comparison_parts(
    instruction: Instruction,
) -> Option<(Comparison, Register, Register, Register)> {
    match instruction {
        Instruction::Equal {
            destination,
            left,
            right,
        } => Some((Comparison::Equal, destination, left, right)),
        Instruction::NotEqual {
            destination,
            left,
            right,
        } => Some((Comparison::NotEqual, destination, left, right)),
        Instruction::LessThan {
            destination,
            left,
            right,
        } => Some((Comparison::LessThan, destination, left, right)),
        Instruction::LessThanOrEqual {
            destination,
            left,
            right,
        } => Some((Comparison::LessThanOrEqual, destination, left, right)),
        Instruction::GreaterThan {
            destination,
            left,
            right,
        } => Some((Comparison::GreaterThan, destination, left, right)),
        Instruction::GreaterThanOrEqual {
            destination,
            left,
            right,
        } => Some((Comparison::GreaterThanOrEqual, destination, left, right)),
        _ => None,
    }
}

fn compared_register(character: Register, left: Register, right: Register) -> Option<Register> {
    if left == character && right != character {
        Some(right)
    } else if right == character && left != character {
        Some(left)
    } else {
        None
    }
}

fn constant_byte(flow: &TypeFlow<'_>, position: usize, register: Register) -> Option<u8> {
    let ConstantValue::String(value) = flow.constant_value(position, register)? else {
        return None;
    };
    let [byte] = value.as_bytes() else {
        return None;
    };
    Some(*byte)
}

fn string_byte_comparison(
    comparison: Comparison,
    destination: Register,
    container: Register,
    index: Register,
    byte: u8,
) -> Instruction {
    match comparison {
        Comparison::Equal => Instruction::StringByteEqual {
            destination,
            container,
            index,
            byte,
        },
        Comparison::NotEqual => Instruction::StringByteNotEqual {
            destination,
            container,
            index,
            byte,
        },
        Comparison::LessThan => Instruction::StringByteLessThan {
            destination,
            container,
            index,
            byte,
        },
        Comparison::LessThanOrEqual => Instruction::StringByteLessThanOrEqual {
            destination,
            container,
            index,
            byte,
        },
        Comparison::GreaterThan => Instruction::StringByteGreaterThan {
            destination,
            container,
            index,
            byte,
        },
        Comparison::GreaterThanOrEqual => Instruction::StringByteGreaterThanOrEqual {
            destination,
            container,
            index,
            byte,
        },
    }
}

fn fuse_string_byte_branches(chunk: &mut Chunk, statistics: &mut OptimizationStatistics) {
    if chunk.code.len() < 3 {
        return;
    }

    let targets = control_flow_targets(chunk);
    let mut remove = vec![false; chunk.code.len()];
    for index in 0..chunk.code.len() - 2 {
        let Instruction::StringIndexGet {
            destination: character,
            container,
            index: string_index,
        } = chunk.code[index]
        else {
            continue;
        };
        let Instruction::LoadConstant {
            destination: literal,
            constant,
        } = chunk.code[index + 1]
        else {
            continue;
        };
        let Instruction::StringJumpUnless {
            comparison,
            left,
            right,
            offset,
        } = chunk.code[index + 2]
        else {
            continue;
        };
        if targets.contains(&(index + 1)) || targets.contains(&(index + 2)) {
            continue;
        }

        let compared =
            (left == character && right == literal) || (left == literal && right == character);
        if !compared || character == literal {
            continue;
        }

        let Literal::String(value) = &chunk.constants[usize::from(constant.index())] else {
            continue;
        };
        let [byte] = value.as_bytes() else {
            continue;
        };

        let target = relative_target(index + 2, i32::from(offset.offset()));
        if !register_is_dead_after(chunk, character, index + 3)
            || !register_is_dead_after(chunk, character, target)
            || !register_is_dead_after(chunk, literal, index + 3)
            || !register_is_dead_after(chunk, literal, target)
        {
            continue;
        }

        let relative = target as i64 - index as i64;
        let Ok(relative) = i16::try_from(relative) else {
            continue;
        };
        chunk.code[index] = match comparison {
            Comparison::Equal => Instruction::StringByteJumpUnlessEqual {
                container,
                index: string_index,
                byte: *byte,
                offset: ShortJumpOffset::new(relative),
            },
            Comparison::NotEqual => Instruction::StringByteJumpUnlessNotEqual {
                container,
                index: string_index,
                byte: *byte,
                offset: ShortJumpOffset::new(relative),
            },
            _ => continue,
        };
        remove[index + 1] = true;
        remove[index + 2] = true;
    }

    let removed = compact_removed_instructions(chunk, &remove, statistics);
    if removed == 0 {
        return;
    }

    statistics.operations_specialized += removed / 2;
}

fn plan_boolean_literal_branches(
    analyzed: &AnalyzedChunk<'_>,
    plan: &mut RewritePlan,
    statistics: &mut OptimizationStatistics,
) -> bool {
    let chunk = analyzed.chunk;
    let flow = &analyzed.flow;
    let Some((replacements, remove)) = boolean_literal_branch_changes(chunk, flow) else {
        return false;
    };

    let mut removed = 0;
    for (position, replacement) in replacements.into_iter().enumerate() {
        let Some(replacement) = replacement else {
            continue;
        };
        if plan.replace(analyzed, position, replacement)
            && remove[position - 1]
            && plan.remove(analyzed, position - 1)
        {
            removed += 1;
        }
    }
    statistics.instructions_removed += removed;
    removed != 0
}

fn fuse_boolean_literal_branches(
    chunk: &mut Chunk,
    flow: &TypeFlow<'_>,
    statistics: &mut OptimizationStatistics,
) -> bool {
    let Some((replacements, remove)) = boolean_literal_branch_changes(chunk, flow) else {
        return false;
    };

    for (instruction, replacement) in chunk.code.iter_mut().zip(replacements) {
        if let Some(replacement) = replacement {
            *instruction = replacement;
        }
    }

    compact_removed_instructions(chunk, &remove, statistics) != 0
}

fn boolean_literal_branch_changes(
    chunk: &Chunk,
    flow: &TypeFlow<'_>,
) -> Option<(Vec<Option<Instruction>>, Vec<bool>)> {
    if chunk.code.len() < 2 {
        return None;
    }

    let targets = control_flow_targets(chunk);
    let mut remove = vec![false; chunk.code.len()];
    let mut replacements = vec![None; chunk.code.len()];
    for index in 0..chunk.code.len() - 1 {
        let (temporary, literal) = match chunk.code[index] {
            Instruction::LoadTrue { destination } => (destination, true),
            Instruction::LoadFalse { destination } => (destination, false),
            _ => continue,
        };
        let Instruction::JumpUnless {
            comparison,
            left,
            right,
            offset,
        } = chunk.code[index + 1]
        else {
            continue;
        };
        if !matches!(comparison, Comparison::Equal | Comparison::NotEqual)
            || targets.contains(&index)
            || targets.contains(&(index + 1))
        {
            continue;
        }
        let subject = if left == temporary && right != temporary {
            right
        } else if right == temporary && left != temporary {
            left
        } else {
            continue;
        };
        let target = relative_target(index + 1, i32::from(offset.offset()));
        if !flow.proves(index + 1, subject, &TypeDescriptor::Bool)
            || !register_is_dead_after(chunk, temporary, index + 2)
            || !register_is_dead_after(chunk, temporary, target)
        {
            continue;
        }

        let jumps_when_true = (comparison == Comparison::NotEqual) == literal;
        let offset = JumpOffset::new(i32::from(offset.offset()));
        replacements[index + 1] = Some(if jumps_when_true {
            Instruction::JumpIfTrue {
                condition: subject,
                offset,
            }
        } else {
            Instruction::JumpIfFalse {
                condition: subject,
                offset,
            }
        });
        remove[index] = true;
    }

    remove
        .iter()
        .any(|removed| *removed)
        .then_some((replacements, remove))
}
