//! Self-inlining of recursive bodies and the jumping replacement builder.

use crate::optimizer::passes::inline_leaf_calls::Atom;
use crate::optimizer::passes::inline_leaf_calls::CALLEE_INSTRUCTION_LIMIT;
use crate::optimizer::passes::inline_leaf_calls::CALLER_CODE_LIMIT;
use crate::optimizer::passes::inline_leaf_calls::Chunk;
use crate::optimizer::passes::inline_leaf_calls::CompiledFunction;
use crate::optimizer::passes::inline_leaf_calls::IcDescriptor;
use crate::optimizer::passes::inline_leaf_calls::IcSlot;
use crate::optimizer::passes::inline_leaf_calls::Instruction;
use crate::optimizer::passes::inline_leaf_calls::JumpOffset;
use crate::optimizer::passes::inline_leaf_calls::OptimizationStatistics;
use crate::optimizer::passes::inline_leaf_calls::PropertyValueMode;
use crate::optimizer::passes::inline_leaf_calls::REGISTER_LIMIT;
use crate::optimizer::passes::inline_leaf_calls::Register;
use crate::optimizer::passes::inline_leaf_calls::ShortJumpOffset;
use crate::optimizer::passes::inline_leaf_calls::Span;
use crate::optimizer::passes::inline_leaf_calls::TypeDescriptor;
use crate::optimizer::passes::inline_leaf_calls::effect_on;
use crate::optimizer::passes::inline_leaf_calls::leaf::owned_register_mask;
use crate::optimizer::passes::inline_leaf_calls::leaf::release_callee_registers;
use crate::optimizer::passes::inline_leaf_calls::leaf::remap_instruction;
use crate::optimizer::passes::inline_leaf_calls::leaf::straight_line_body_instruction;
use crate::optimizer::passes::inline_leaf_calls::splice_replace;
use crate::optimizer::passes::inline_leaf_calls::substitute;

/// Splices a pristine snapshot of a recursive function's own body into its
/// self-call sites, halving the call depth of tight recursion.
pub(super) fn self_inline_function(
    function: &mut CompiledFunction,
    statistics: &mut OptimizationStatistics,
) -> bool {
    let Ok(parameters) = u16::try_from(function.parameters.len()) else {
        return false;
    };

    if !function.type_parameters.is_empty()
        || function.captures_this
        || function
            .parameters
            .iter()
            .any(|parameter| parameter.has_default)
    {
        return false;
    }

    let snapshot = function.chunk.clone();
    if snapshot.code.is_empty()
        || snapshot.code.len() > CALLEE_INSTRUCTION_LIMIT
        || !snapshot.catch_table.is_empty()
        || !snapshot.switch_tables.is_empty()
    {
        return false;
    }

    let mut terminal = snapshot.code.len() - 1;
    if terminal > 0 && matches!(snapshot.code[terminal], Instruction::ReturnNull) {
        terminal -= 1;
    }

    if !matches!(
        snapshot.code[terminal],
        Instruction::ReturnScalarUnchecked { .. }
            | Instruction::ReturnReferenceUnchecked { .. }
            | Instruction::ReturnUnchecked { .. }
            | Instruction::ReturnIntUnchecked { .. }
            | Instruction::ReturnNullUnchecked
    ) {
        return false;
    }

    for (index, instruction) in snapshot.code[..terminal].iter().copied().enumerate() {
        let allowed = straight_line_body_instruction(instruction)
            || matches!(
                instruction,
                Instruction::CallSelfUnchecked { first_argument, .. }
                    if first_argument.index() >= parameters
            )
            || matches!(
                instruction,
                Instruction::PropertyGet { .. }
                    | Instruction::CheckDefined { .. }
                    | Instruction::ReturnScalarUnchecked { .. }
                    | Instruction::ReturnReferenceUnchecked { .. }
                    | Instruction::ReturnUnchecked { .. }
                    | Instruction::ReturnIntUnchecked { .. }
                    | Instruction::ReturnNullUnchecked
            )
            || body_jump_targets_are_forward(instruction, index, terminal);

        if !allowed {
            return false;
        }

        for parameter in 0..parameters {
            if effect_on(&snapshot, instruction, Register::new(parameter)).writes() {
                return false;
            }
        }
    }

    let mut changed = false;
    let mut index = function.chunk.code.len();
    while index > 0 {
        index -= 1;
        if function.chunk.code.len() >= CALLER_CODE_LIMIT {
            break;
        }

        let Instruction::CallSelfUnchecked {
            argument_count,
            destination,
            first_argument,
        } = function.chunk.code[index]
        else {
            continue;
        };

        if usize::from(argument_count.value()) != usize::from(parameters) {
            continue;
        }

        let owned = owned_register_mask(
            &function.chunk,
            index,
            first_argument,
            function,
            false,
            parameters,
        );
        if let Some(replacement) = build_jumping_replacement(
            &mut function.chunk,
            &snapshot,
            terminal,
            parameters,
            destination,
            first_argument,
            owned,
        ) {
            splice_replace(&mut function.chunk, index, &replacement);
            statistics.calls_inlined += 1;
            changed = true;
        }
    }

    changed
}

/// The forward jump target of a body instruction, when it is one of the
/// relative control-flow forms an inlined body may keep.
pub(super) fn body_jump_target(instruction: Instruction, index: usize) -> Option<usize> {
    let offset = match instruction {
        Instruction::Jump { offset }
        | Instruction::JumpIfFalse { offset, .. }
        | Instruction::JumpIfTrue { offset, .. }
        | Instruction::JumpIfNull { offset, .. }
        | Instruction::JumpIfNotNull { offset, .. } => offset.offset(),
        Instruction::JumpUnless { offset, .. }
        | Instruction::IntJumpUnless { offset, .. }
        | Instruction::StringJumpUnless { offset, .. }
        | Instruction::StringByteJumpUnlessEqual { offset, .. }
        | Instruction::StringByteJumpUnlessNotEqual { offset, .. }
        | Instruction::IntJumpUnlessImmediate { offset, .. }
        | Instruction::IntRangeJumpIf { offset, .. }
        | Instruction::IntRangeJumpUnless { offset, .. } => i32::from(offset.offset()),
        _ => return None,
    };

    usize::try_from(index as i64 + i64::from(offset)).ok()
}

pub(super) fn body_jump_targets_are_forward(
    instruction: Instruction,
    index: usize,
    terminal: usize,
) -> bool {
    if let Instruction::BoolPatternBranch {
        false_offset,
        default_offset,
        ..
    } = instruction
    {
        return [false_offset, default_offset].into_iter().all(|offset| {
            usize::try_from(index as i64 + i64::from(offset.offset()))
                .is_ok_and(|target| target <= terminal && target > index)
        });
    }

    body_jump_target(instruction, index).is_some_and(|target| target <= terminal && target > index)
}

pub(super) fn build_jumping_replacement(
    chunk: &mut Chunk,
    snapshot: &Chunk,
    terminal: usize,
    parameters: u16,
    destination: Register,
    first_argument: Register,
    owned: u64,
) -> Option<Vec<(Instruction, Span)>> {
    build_jumping_replacement_capped(
        chunk,
        snapshot,
        terminal,
        parameters,
        destination,
        first_argument,
        REGISTER_LIMIT,
        owned,
    )
}

#[expect(clippy::too_many_arguments, reason = "internal builder")]
pub(super) fn build_jumping_replacement_capped(
    chunk: &mut Chunk,
    snapshot: &Chunk,
    terminal: usize,
    parameters: u16,
    destination: Register,
    first_argument: Register,
    register_cap: u16,
    owned: u64,
) -> Option<Vec<(Instruction, Span)>> {
    build_jumping_replacement_bound(
        chunk,
        snapshot,
        terminal,
        parameters,
        destination,
        first_argument,
        register_cap,
        None,
        owned,
    )
}

#[expect(clippy::too_many_arguments, reason = "internal builder")]
pub(super) fn build_jumping_replacement_bound(
    chunk: &mut Chunk,
    snapshot: &Chunk,
    terminal: usize,
    parameters: u16,
    destination: Register,
    first_argument: Register,
    register_cap: u16,
    bindings: Option<&[(Atom, TypeDescriptor)]>,
    owned: u64,
) -> Option<Vec<(Instruction, Span)>> {
    let local_base = chunk.register_count;
    let local_count = snapshot.register_count.saturating_sub(parameters);
    let new_register_count = local_base.checked_add(local_count)?.checked_add(1)?;
    if new_register_count > register_cap {
        return None;
    }

    let remap = |register: Register| -> Register {
        if register.index() < parameters {
            Register::new(first_argument.index() + register.index())
        } else {
            Register::new(local_base + 1 + (register.index() - parameters))
        }
    };

    let mut sizes = Vec::with_capacity(terminal + 1);
    for (index, instruction) in snapshot.code[..=terminal].iter().copied().enumerate() {
        let is_return = matches!(
            instruction,
            Instruction::ReturnScalarUnchecked { .. }
                | Instruction::ReturnReferenceUnchecked { .. }
                | Instruction::ReturnUnchecked { .. }
                | Instruction::ReturnIntUnchecked { .. }
                | Instruction::ReturnNullUnchecked
        );

        sizes.push(if is_return && index != terminal { 2 } else { 1 });
    }

    let mut new_positions = Vec::with_capacity(terminal + 2);
    let mut position = 0usize;
    for size in &sizes {
        new_positions.push(position);
        position += size;
    }

    new_positions.push(position);
    let end = position;

    let mut replacement = Vec::with_capacity(end);
    for (index, instruction) in snapshot.code[..=terminal].iter().copied().enumerate() {
        let span = snapshot.spans[index];
        let value_load = match instruction {
            Instruction::ReturnScalarUnchecked { source }
            | Instruction::ReturnReferenceUnchecked { source }
            | Instruction::ReturnUnchecked { source } => Some(Instruction::Move {
                destination,
                source: remap(source),
            }),
            Instruction::ReturnIntUnchecked { immediate } => Some(Instruction::LoadInt {
                destination,
                immediate,
            }),
            Instruction::ReturnNullUnchecked => Some(Instruction::LoadNull { destination }),
            _ => None,
        };

        if let Some(value_load) = value_load {
            replacement.push((value_load, span));
            if index != terminal {
                let from = new_positions[index] + 1;
                let relative = i32::try_from(end as i64 - from as i64).ok()?;
                replacement.push((
                    Instruction::Jump {
                        offset: JumpOffset::new(relative),
                    },
                    span,
                ));
            }

            continue;
        }

        if let Instruction::BoolPatternBranch {
            subject,
            false_offset,
            default_offset,
        } = instruction
        {
            let from = new_positions[index];
            let target = usize::try_from(index as i64 + i64::from(false_offset.offset())).ok()?;
            let false_relative = new_positions[target] as i64 - from as i64;
            let target = usize::try_from(index as i64 + i64::from(default_offset.offset())).ok()?;
            let default_relative = new_positions[target] as i64 - from as i64;
            let rebased = Instruction::BoolPatternBranch {
                subject,
                false_offset: ShortJumpOffset::new(i16::try_from(false_relative).ok()?),
                default_offset: ShortJumpOffset::new(i16::try_from(default_relative).ok()?),
            };
            let remapped = remap_instruction(chunk, snapshot, rebased, &remap)?;
            replacement.push((remapped, span));
            continue;
        }

        if let Some(target) = body_jump_target(instruction, index) {
            let from = new_positions[index];
            let relative = new_positions[target] as i64 - from as i64;
            let rebased = rebase_body_jump(instruction, relative)?;
            let remapped = remap_instruction(chunk, snapshot, rebased, &remap)?;
            replacement.push((remapped, span));
            continue;
        }

        let remapped = match instruction {
            Instruction::PropertySetUnchecked {
                object,
                value,
                slot,
                value_mode,
            } => Instruction::PropertySetUnchecked {
                object: remap(object),
                value: remap(value),
                slot,
                value_mode: if value.index() < parameters {
                    match value_mode {
                        PropertyValueMode::Move | PropertyValueMode::MoveAndClear => {
                            PropertyValueMode::Clone
                        }
                        PropertyValueMode::FreshMove | PropertyValueMode::FreshMoveAndClear => {
                            PropertyValueMode::FreshClone
                        }
                        mode => mode,
                    }
                } else {
                    value_mode
                },
            },
            Instruction::NewStatic {
                destination: inner_destination,
                cache,
            } => Instruction::NewStatic {
                destination: remap(inner_destination),
                cache: remap_cache(chunk, snapshot, cache, bindings?)?,
            },
            Instruction::CallMethodUnchecked {
                argument_count,
                destination: inner_destination,
                first_argument: inner_first,
                cache,
            } => Instruction::CallMethodUnchecked {
                argument_count,
                destination: remap(inner_destination),
                first_argument: remap(inner_first),
                cache: remap_cache(chunk, snapshot, cache, bindings?)?,
            },
            other => remap_instruction(chunk, snapshot, other, &remap)?,
        };

        replacement.push((remapped, span));
    }

    release_callee_registers(
        &mut replacement,
        owned,
        snapshot.register_count,
        destination,
        &remap,
        snapshot.spans[terminal],
    )?;
    chunk.register_count = new_register_count;
    Some(replacement)
}

/// Clones a callee inline-cache descriptor into the caller's table,
/// substituting the callee's type parameters with the call site's concrete
/// arguments so the descriptor resolves without the callee's environment.
fn remap_cache(
    chunk: &mut Chunk,
    snapshot: &Chunk,
    cache: IcSlot,
    bindings: &[(Atom, TypeDescriptor)],
) -> Option<IcSlot> {
    let descriptor = snapshot.ic_descriptors.get(usize::from(cache.index()))?;
    let cloned = match descriptor {
        IcDescriptor::Member {
            name,
            type_arguments,
        } => IcDescriptor::Member {
            name: name.clone(),
            type_arguments: type_arguments.as_ref().map(|arguments| {
                arguments
                    .iter()
                    .map(|argument| substitute(argument, bindings, 0))
                    .collect()
            }),
        },
        IcDescriptor::ClassMember {
            class,
            member,
            type_arguments,
        } => IcDescriptor::ClassMember {
            class: class.clone(),
            member: member.clone(),
            type_arguments: type_arguments.clone(),
        },
    };

    let slot = u16::try_from(chunk.ic_descriptors.len()).ok()?;
    chunk.ic_descriptors.push(cloned);
    Some(IcSlot::new(slot))
}

/// Reissues a body jump with a rebased relative offset.
fn rebase_body_jump(instruction: Instruction, relative: i64) -> Option<Instruction> {
    let mut instruction = instruction;
    match &mut instruction {
        Instruction::Jump { offset }
        | Instruction::JumpIfFalse { offset, .. }
        | Instruction::JumpIfTrue { offset, .. }
        | Instruction::JumpIfNull { offset, .. }
        | Instruction::JumpIfNotNull { offset, .. } => {
            *offset = JumpOffset::new(i32::try_from(relative).ok()?);
        }
        Instruction::JumpUnless { offset, .. }
        | Instruction::IntJumpUnless { offset, .. }
        | Instruction::StringJumpUnless { offset, .. }
        | Instruction::StringByteJumpUnlessEqual { offset, .. }
        | Instruction::StringByteJumpUnlessNotEqual { offset, .. }
        | Instruction::IntJumpUnlessImmediate { offset, .. }
        | Instruction::IntRangeJumpIf { offset, .. }
        | Instruction::IntRangeJumpUnless { offset, .. } => {
            *offset = ShortJumpOffset::new(i16::try_from(relative).ok()?);
        }
        _ => return None,
    }

    Some(instruction)
}
