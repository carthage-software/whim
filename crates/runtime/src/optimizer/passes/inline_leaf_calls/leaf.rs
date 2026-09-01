//! Leaf-callee selection and straight-line inlining into caller chunks.

use hashbrown::HashSet;

use crate::bytecode::REFERENCE_REGISTER_LIMIT;
use crate::bytecode::chunk::descriptors::Literal;
use crate::bytecode::chunk::descriptors::TypeDescriptor;
use crate::optimizer::cfg::branches_or_terminates;
use crate::optimizer::cfg::control_flow_targets;
use crate::optimizer::liveness::register_is_dead_after;
use crate::optimizer::passes::inline_leaf_calls::CALLEE_INSTRUCTION_LIMIT;
use crate::optimizer::passes::inline_leaf_calls::CALLER_CODE_LIMIT;
use crate::optimizer::passes::inline_leaf_calls::Chunk;
use crate::optimizer::passes::inline_leaf_calls::CompiledFunction;
use crate::optimizer::passes::inline_leaf_calls::ConstantIndex;
use crate::optimizer::passes::inline_leaf_calls::IcDescriptor;
use crate::optimizer::passes::inline_leaf_calls::InlineCandidates;
use crate::optimizer::passes::inline_leaf_calls::Instruction;
use crate::optimizer::passes::inline_leaf_calls::OptimizationStatistics;
use crate::optimizer::passes::inline_leaf_calls::REGISTER_LIMIT;
use crate::optimizer::passes::inline_leaf_calls::Register;
use crate::optimizer::passes::inline_leaf_calls::Span;
use crate::optimizer::passes::inline_leaf_calls::effect_on;
use crate::optimizer::passes::inline_leaf_calls::is_always_inline;
use crate::optimizer::passes::inline_leaf_calls::is_external;
use crate::optimizer::passes::inline_leaf_calls::is_never_inline;
use crate::optimizer::passes::inline_leaf_calls::jumping::build_jumping_replacement_capped;
use crate::optimizer::passes::inline_leaf_calls::methods::method_body_inlinable;
use crate::optimizer::passes::inline_leaf_calls::methods::unchecked_terminal;
use crate::optimizer::passes::inline_leaf_calls::splice_replace_many;

pub(super) struct LeafCallee {
    parameters: u16,
    registers: u16,
    terminal: usize,
}

pub(super) fn leaf_callee(function: &CompiledFunction) -> Option<LeafCallee> {
    if is_external(&function.attributes)
        || is_never_inline(&function.attributes)
        || !function.type_parameters.is_empty()
        || function.captures_this
    {
        return None;
    }

    let chunk = &function.chunk;
    if chunk.code.is_empty()
        || (!is_always_inline(&function.attributes) && chunk.code.len() > CALLEE_INSTRUCTION_LIMIT)
        || !chunk.catch_table.is_empty()
        || !chunk.switch_tables.is_empty()
    {
        return None;
    }

    let parameters = u16::try_from(function.parameters.len()).ok()?;
    if function
        .parameters
        .iter()
        .any(|parameter| parameter.has_default)
    {
        return None;
    }

    let mut terminal = chunk.code.len() - 1;
    if terminal > 0 && matches!(chunk.code[terminal], Instruction::ReturnNull) {
        terminal -= 1;
    }

    match chunk.code[terminal] {
        Instruction::ReturnScalarUnchecked { .. }
        | Instruction::ReturnReferenceUnchecked { .. }
        | Instruction::ReturnUnchecked { .. }
        | Instruction::ReturnIntUnchecked { .. }
        | Instruction::ReturnNullUnchecked => {}
        _ => return None,
    }

    let force = is_always_inline(&function.attributes);
    for instruction in chunk.code[..terminal].iter().copied() {
        let allowed = straight_line_body_instruction(instruction)
            || force && matches!(instruction, Instruction::CallNamedUnchecked { .. });
        if !allowed {
            return None;
        }
        for parameter in 0..parameters {
            if effect_on(chunk, instruction, Register::new(parameter)).writes() {
                return None;
            }
        }
    }

    Some(LeafCallee {
        parameters,
        registers: chunk.register_count,
        terminal,
    })
}

/// Whether an instruction may appear in an inlined body: pure straight-line
/// data flow with no jumps, calls, caches, or heap-shape side effects.
pub(super) fn straight_line_body_instruction(instruction: Instruction) -> bool {
    matches!(
        instruction,
        Instruction::Move { .. }
            | Instruction::LoadConstant { .. }
            | Instruction::LoadNull { .. }
            | Instruction::LoadTrue { .. }
            | Instruction::LoadFalse { .. }
            | Instruction::LoadInt { .. }
            | Instruction::Add { .. }
            | Instruction::Subtract { .. }
            | Instruction::Multiply { .. }
            | Instruction::Divide { .. }
            | Instruction::Modulo { .. }
            | Instruction::Power { .. }
            | Instruction::Negate { .. }
            | Instruction::UnaryPlus { .. }
            | Instruction::AddImmediate { .. }
            | Instruction::SubtractImmediate { .. }
            | Instruction::Concatenate { .. }
            | Instruction::BitwiseAnd { .. }
            | Instruction::BitwiseOr { .. }
            | Instruction::BitwiseXor { .. }
            | Instruction::BitwiseNot { .. }
            | Instruction::ShiftLeft { .. }
            | Instruction::ShiftRight { .. }
            | Instruction::Equal { .. }
            | Instruction::NotEqual { .. }
            | Instruction::LessThan { .. }
            | Instruction::LessThanOrEqual { .. }
            | Instruction::GreaterThan { .. }
            | Instruction::GreaterThanOrEqual { .. }
            | Instruction::Not { .. }
            | Instruction::IntAdd { .. }
            | Instruction::IntSubtract { .. }
            | Instruction::IntMultiply { .. }
            | Instruction::IntModulo { .. }
            | Instruction::IntMultiplyImmediate { .. }
            | Instruction::IntModuloImmediate { .. }
            | Instruction::IntBitwiseAnd { .. }
            | Instruction::IntBitwiseOr { .. }
            | Instruction::IntBitwiseXor { .. }
            | Instruction::IntBitwiseNot { .. }
            | Instruction::IntAddAssign { .. }
            | Instruction::IntShiftLeft { .. }
            | Instruction::IntShiftRight { .. }
            | Instruction::FloatAdd { .. }
            | Instruction::FloatSubtract { .. }
            | Instruction::FloatMultiply { .. }
            | Instruction::FloatMultiplyConstant { .. }
            | Instruction::StringLength { .. }
            | Instruction::Length { .. }
            | Instruction::IndexGet { .. }
            | Instruction::StringIndexGet { .. }
    )
}

pub(super) fn inline_into_chunk(
    chunk: &mut Chunk,
    functions: &[CompiledFunction],
    candidates: &InlineCandidates<'_>,
    caller_position: Option<usize>,
    statistics: &mut OptimizationStatistics,
) -> bool {
    inline_into_chunk_from(
        chunk,
        functions,
        candidates,
        caller_position,
        0,
        REGISTER_LIMIT,
        statistics,
    )
}

pub(in crate::optimizer) fn inline_live_tail(
    chunk: &mut Chunk,
    functions: &[CompiledFunction],
    floor: usize,
    register_cap: u16,
    statistics: &mut OptimizationStatistics,
) -> bool {
    let candidates = InlineCandidates::new(&[], functions.iter().collect());
    inline_into_chunk_from(
        chunk,
        &[],
        &candidates,
        None,
        floor,
        register_cap,
        statistics,
    )
}

fn inline_into_chunk_from(
    chunk: &mut Chunk,
    functions: &[CompiledFunction],
    candidates: &InlineCandidates<'_>,
    caller_position: Option<usize>,
    floor: usize,
    register_cap: u16,
    statistics: &mut OptimizationStatistics,
) -> bool {
    let targets = control_flow_targets(chunk);
    let mut projected_length = chunk.code.len();
    let mut replacements = Vec::new();
    let mut index = chunk.code.len();
    while index > floor {
        index -= 1;
        if projected_length >= CALLER_CODE_LIMIT {
            break;
        }

        if let Instruction::CallValueUnchecked {
            argument_count,
            destination,
            callee: closure,
            first_argument,
        } = chunk.code[index]
        {
            let Some((producer, position)) =
                immediate_closure(chunk, index, closure, candidates, &targets)
            else {
                continue;
            };
            if caller_position == Some(position) {
                continue;
            }
            let callee_function = &functions[position];
            let Some(callee) = &candidates.local[position] else {
                continue;
            };
            if usize::from(argument_count.value()) != usize::from(callee.parameters) {
                continue;
            }
            let Some(replacement) = build_replacement(
                chunk,
                index,
                callee_function,
                callee,
                destination,
                Some((first_argument, usize::from(argument_count.value()))),
                None,
                chunk.spans[index],
            ) else {
                continue;
            };

            chunk.code[producer] = Instruction::LoadNull {
                destination: closure,
            };
            projected_length += replacement.len() - 1;
            replacements.push((index, replacement));
            statistics.calls_inlined += 1;
            continue;
        }
        let (destination, cache, arguments, constant_argument) = match chunk.code[index] {
            Instruction::CallNamedUnchecked {
                argument_count,
                destination,
                first_argument,
                cache,
            } => (
                destination,
                cache,
                Some((first_argument, usize::from(argument_count.value()))),
                None,
            ),
            Instruction::CallNamedConstantUnchecked {
                destination,
                constant,
                cache,
                ..
            } => (destination, cache, None, Some(constant)),
            _ => continue,
        };
        let IcDescriptor::Member { name, .. } = &chunk.ic_descriptors[usize::from(cache.index())]
        else {
            continue;
        };
        let resolved = candidates
            .local_position(name)
            .map(|position| {
                (
                    &functions[position],
                    &candidates.local[position],
                    Some(position),
                )
            })
            .or_else(|| {
                let position = candidates.donor_position(name)?;
                Some((
                    candidates.donors[position],
                    &candidates.donor_callees[position],
                    None,
                ))
            });

        let Some((callee_function, callee, position)) = resolved else {
            continue;
        };

        if position.is_some() && caller_position == position {
            continue;
        }

        if is_external(&callee_function.attributes) || is_never_inline(&callee_function.attributes)
        {
            continue;
        }
        let Some(callee) = callee else {
            // Branching bodies still inline through the jump-rebasing
            // builder when the arguments arrive in a register window.
            let Some((first_argument, count)) = arguments else {
                continue;
            };
            let Ok(parameters) = u16::try_from(callee_function.parameters.len()) else {
                continue;
            };
            if count != usize::from(parameters)
                || !callee_function.type_parameters.is_empty()
                || callee_function.captures_this
                || callee_function
                    .parameters
                    .iter()
                    .any(|parameter| parameter.has_default)
                || !method_body_inlinable(
                    &callee_function.chunk,
                    parameters,
                    is_always_inline(&callee_function.attributes),
                )
            {
                continue;
            }
            let Some(terminal) = unchecked_terminal(&callee_function.chunk) else {
                continue;
            };
            let snapshot = callee_function.chunk.clone();
            if let Some(replacement) = build_jumping_replacement_capped(
                chunk,
                &snapshot,
                terminal,
                parameters,
                destination,
                first_argument,
                register_cap,
                owned_register_mask(
                    chunk,
                    index,
                    first_argument,
                    callee_function,
                    false,
                    parameters,
                ),
            ) {
                projected_length += replacement.len() - 1;
                replacements.push((index, replacement));
                statistics.calls_inlined += 1;
            }
            continue;
        };
        let argc = match (&arguments, &constant_argument) {
            (Some((_, count)), _) => *count,
            (None, Some(_)) => 1,
            _ => continue,
        };
        if argc != usize::from(callee.parameters) {
            continue;
        }

        if let Some(replacement) = build_replacement(
            chunk,
            index,
            callee_function,
            callee,
            destination,
            arguments,
            constant_argument,
            chunk.spans[index],
        ) {
            projected_length += replacement.len() - 1;
            replacements.push((index, replacement));
            statistics.calls_inlined += 1;
        }
    }

    replacements.reverse();
    let changed = !replacements.is_empty();
    splice_replace_many(chunk, &replacements);
    changed
}

fn immediate_closure(
    chunk: &Chunk,
    call: usize,
    closure: Register,
    candidates: &InlineCandidates<'_>,
    targets: &HashSet<usize>,
) -> Option<(usize, usize)> {
    if !register_is_dead_after(chunk, closure, call + 1) {
        return None;
    }
    for at in (0..call).rev() {
        if targets.contains(&(at + 1)) || branches_or_terminates(chunk.code[at]) {
            return None;
        }
        let effect = effect_on(chunk, chunk.code[at], closure);
        if effect.reads() {
            return None;
        }
        if !effect.writes() {
            continue;
        }
        let Instruction::MakeClosure {
            capture_count,
            destination,
            prototype,
            ..
        } = chunk.code[at]
        else {
            return None;
        };
        if destination != closure || capture_count.value() != 0 {
            return None;
        }
        let Literal::String(name) = &chunk.constants[usize::from(prototype.index())] else {
            return None;
        };
        let position = candidates.local_position(name)?;
        return Some((at, position));
    }
    None
}

#[expect(clippy::too_many_arguments, reason = "internal builder")]
fn build_replacement(
    chunk: &mut Chunk,
    index: usize,
    callee_function: &CompiledFunction,
    callee: &LeafCallee,
    destination: Register,
    arguments: Option<(Register, usize)>,
    constant_argument: Option<ConstantIndex>,
    call_span: Span,
) -> Option<Vec<(Instruction, Span)>> {
    let callee_chunk = &callee_function.chunk;
    let local_base = chunk.register_count;
    let local_count = callee.registers.saturating_sub(callee.parameters);
    let new_register_count = local_base.checked_add(local_count)?.checked_add(1)?;
    if new_register_count > REGISTER_LIMIT {
        return None;
    }

    let parameter_base = match arguments {
        Some((first_argument, _)) => first_argument,
        None => Register::new(local_base),
    };
    let remap = |register: Register| -> Register {
        if register.index() < callee.parameters {
            Register::new(parameter_base.index() + register.index())
        } else {
            Register::new(local_base + 1 + (register.index() - callee.parameters))
        }
    };

    let mut replacement = Vec::with_capacity(callee_chunk.code.len() + 1);
    if let Some(constant) = constant_argument {
        let literal = chunk.constants[usize::from(constant.index())].clone();
        let remapped = chunk.add_constant(literal).ok()?;
        replacement.push((
            Instruction::LoadConstant {
                destination: parameter_base,
                constant: remapped,
            },
            call_span,
        ));
    }

    let terminal = callee.terminal;
    for (index, instruction) in callee_chunk.code[..=terminal].iter().copied().enumerate() {
        let span = callee_chunk.spans[index];
        if index == terminal {
            let morphed = match instruction {
                Instruction::ReturnScalarUnchecked { source }
                | Instruction::ReturnReferenceUnchecked { source }
                | Instruction::ReturnUnchecked { source } => Instruction::Move {
                    destination,
                    source: remap(source),
                },
                Instruction::ReturnIntUnchecked { immediate } => Instruction::LoadInt {
                    destination,
                    immediate,
                },
                Instruction::ReturnNullUnchecked => Instruction::LoadNull { destination },
                _ => return None,
            };
            replacement.push((morphed, span));
            continue;
        }
        let remapped = remap_instruction(chunk, callee_chunk, instruction, &remap)?;
        replacement.push((remapped, span));
    }

    release_callee_registers(
        &mut replacement,
        owned_register_mask(
            chunk,
            index,
            parameter_base,
            callee_function,
            false,
            callee.parameters,
        ),
        callee.registers,
        destination,
        &remap,
        call_span,
    )?;
    chunk.register_count = new_register_count;
    Some(replacement)
}

/// The registers a call frame owns: its receiver, its reference-typed
/// arguments, and every local its body may leave holding a reference.
pub(super) fn owned_register_mask(
    caller: &Chunk,
    index: usize,
    first_argument: Register,
    function: &CompiledFunction,
    has_receiver: bool,
    parameters: u16,
) -> u64 {
    let mut mask = function.chunk.reference_register_mask;
    if has_receiver {
        mask |= 1;
    }

    let base = usize::from(parameters).saturating_sub(function.parameters.len());
    for (position, parameter) in function
        .parameters
        .iter()
        .take(usize::from(REFERENCE_REGISTER_LIMIT))
        .enumerate()
    {
        if parameter
            .declared_type
            .as_ref()
            .is_none_or(TypeDescriptor::may_hold_reference)
        {
            mask |= 1u64 << (base + position);
        }
    }

    for position in 0..u32::from(parameters).min(u32::from(REFERENCE_REGISTER_LIMIT)) {
        let window = Register::new(first_argument.index() + position as u16);
        if !register_is_dead_after(caller, window, index + 1) {
            mask &= !(1u64 << position);
        }
    }

    mask
}

pub(super) fn release_callee_registers(
    replacement: &mut Vec<(Instruction, Span)>,
    mask: u64,
    registers: u16,
    destination: Register,
    remap: &impl Fn(Register) -> Register,
    span: Span,
) -> Option<()> {
    if mask == 0 {
        return Some(());
    }

    if registers > REFERENCE_REGISTER_LIMIT {
        return None;
    }

    for register in 0..registers {
        if mask & (1u64 << register) == 0 {
            continue;
        }

        let target = remap(Register::new(register));
        if target == destination {
            continue;
        }

        replacement.push((Instruction::Clear { target }, span));
    }

    Some(())
}

pub(super) fn remap_instruction(
    chunk: &mut Chunk,
    callee_chunk: &Chunk,
    instruction: Instruction,
    remap: &impl Fn(Register) -> Register,
) -> Option<Instruction> {
    let remap_constant = |chunk: &mut Chunk, constant: ConstantIndex| -> Option<ConstantIndex> {
        let literal = callee_chunk.constants[usize::from(constant.index())].clone();
        chunk.add_constant(literal).ok()
    };
    let mut instruction = instruction;

    match &mut instruction {
        Instruction::LoadConstant {
            destination,
            constant,
        } => {
            *destination = remap(*destination);
            *constant = remap_constant(chunk, *constant)?;
        }
        Instruction::FloatMultiplyConstant {
            destination,
            source,
            constant,
        } => {
            *destination = remap(*destination);
            *source = remap(*source);
            *constant = remap_constant(chunk, *constant)?;
        }
        Instruction::CallNamedUnchecked {
            destination,
            first_argument,
            cache,
            ..
        } => {
            *destination = remap(*destination);
            *first_argument = remap(*first_argument);
            *cache = chunk
                .add_ic_descriptor(callee_chunk.ic_descriptors[usize::from(cache.index())].clone())
                .ok()?;
        }
        Instruction::Move {
            destination,
            source,
        }
        | Instruction::Negate {
            destination,
            source,
        }
        | Instruction::UnaryPlus {
            destination,
            source,
        }
        | Instruction::BitwiseNot {
            destination,
            source,
        }
        | Instruction::IntBitwiseNot {
            destination,
            source,
        }
        | Instruction::Not {
            destination,
            source,
        }
        | Instruction::StringLength {
            destination,
            source,
        }
        | Instruction::Length {
            destination,
            source,
        }
        | Instruction::AddImmediate {
            destination,
            source,
            ..
        }
        | Instruction::SubtractImmediate {
            destination,
            source,
            ..
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
            *destination = remap(*destination);
            *source = remap(*source);
        }
        Instruction::LoadNull { destination }
        | Instruction::LoadTrue { destination }
        | Instruction::LoadFalse { destination }
        | Instruction::LoadInt { destination, .. } => {
            *destination = remap(*destination);
        }
        Instruction::IntAddAssign { target, source } => {
            *target = remap(*target);
            *source = remap(*source);
        }
        Instruction::Add {
            destination,
            left,
            right,
        }
        | Instruction::Subtract {
            destination,
            left,
            right,
        }
        | Instruction::Multiply {
            destination,
            left,
            right,
        }
        | Instruction::Divide {
            destination,
            left,
            right,
        }
        | Instruction::Modulo {
            destination,
            left,
            right,
        }
        | Instruction::Power {
            destination,
            left,
            right,
        }
        | Instruction::Concatenate {
            destination,
            left,
            right,
        }
        | Instruction::BitwiseAnd {
            destination,
            left,
            right,
        }
        | Instruction::IntBitwiseAnd {
            destination,
            left,
            right,
        }
        | Instruction::BitwiseOr {
            destination,
            left,
            right,
        }
        | Instruction::IntBitwiseOr {
            destination,
            left,
            right,
        }
        | Instruction::BitwiseXor {
            destination,
            left,
            right,
        }
        | Instruction::IntBitwiseXor {
            destination,
            left,
            right,
        }
        | Instruction::ShiftLeft {
            destination,
            left,
            right,
        }
        | Instruction::IntShiftLeft {
            destination,
            left,
            right,
        }
        | Instruction::ShiftRight {
            destination,
            left,
            right,
        }
        | Instruction::IntShiftRight {
            destination,
            left,
            right,
        }
        | Instruction::Equal {
            destination,
            left,
            right,
        }
        | Instruction::NotEqual {
            destination,
            left,
            right,
        }
        | Instruction::LessThan {
            destination,
            left,
            right,
        }
        | Instruction::LessThanOrEqual {
            destination,
            left,
            right,
        }
        | Instruction::GreaterThan {
            destination,
            left,
            right,
        }
        | Instruction::GreaterThanOrEqual {
            destination,
            left,
            right,
        }
        | Instruction::IntAdd {
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
        | Instruction::FloatAdd {
            destination,
            left,
            right,
        }
        | Instruction::FloatSubtract {
            destination,
            left,
            right,
        }
        | Instruction::FloatMultiply {
            destination,
            left,
            right,
        } => {
            *destination = remap(*destination);
            *left = remap(*left);
            *right = remap(*right);
        }
        Instruction::IndexGet {
            destination,
            container,
            index,
        }
        | Instruction::StringIndexGet {
            destination,
            container,
            index,
        } => {
            *destination = remap(*destination);
            *container = remap(*container);
            *index = remap(*index);
        }
        Instruction::Jump { .. } => {}
        Instruction::JumpIfFalse { condition, .. } | Instruction::JumpIfTrue { condition, .. } => {
            *condition = remap(*condition);
        }
        Instruction::JumpIfNull { subject, .. }
        | Instruction::JumpIfNotNull { subject, .. }
        | Instruction::BoolPatternBranch { subject, .. }
        | Instruction::CheckDefined { subject, .. } => {
            *subject = remap(*subject);
        }
        Instruction::JumpUnless { left, right, .. }
        | Instruction::IntJumpUnless { left, right, .. }
        | Instruction::StringJumpUnless { left, right, .. } => {
            *left = remap(*left);
            *right = remap(*right);
        }
        Instruction::StringByteJumpUnlessEqual {
            container, index, ..
        }
        | Instruction::StringByteJumpUnlessNotEqual {
            container, index, ..
        } => {
            *container = remap(*container);
            *index = remap(*index);
        }
        Instruction::IntJumpUnlessImmediate { source, .. } => {
            *source = remap(*source);
        }
        Instruction::IntRangeJumpIf {
            subject,
            descriptor,
            ..
        }
        | Instruction::IntRangeJumpUnless {
            subject,
            descriptor,
            ..
        } => {
            *subject = remap(*subject);
            *descriptor = chunk
                .add_type_descriptor(
                    callee_chunk.type_descriptors[usize::from(descriptor.index())].clone(),
                )
                .ok()?;
        }
        Instruction::CallSelfUnchecked {
            destination,
            first_argument,
            ..
        } => {
            *destination = remap(*destination);
            *first_argument = remap(*first_argument);
        }
        Instruction::PropertyGet {
            destination,
            object,
            ..
        }
        | Instruction::PropertyGetUnchecked {
            destination,
            object,
            ..
        } => {
            *destination = remap(*destination);
            *object = remap(*object);
        }
        _ => return None,
    }

    Some(instruction)
}
