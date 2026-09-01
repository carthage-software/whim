//! Layout of straight-line cold branch bodies behind their hot continuation.

use std::mem;

use crate::bytecode::chunk::Chunk;
use crate::bytecode::chunk::descriptors::IcDescriptor;
use crate::bytecode::instruction::Instruction;
use crate::bytecode::instruction::operands::Comparison;
use crate::bytecode::instruction::operands::IcSlot;
use crate::bytecode::instruction::operands::JumpOffset;
use crate::bytecode::instruction::operands::ShortJumpOffset;
use crate::bytecode::rewrite::rebase_targets;
use crate::bytecode::unit::COLD_ATTRIBUTE;
use crate::bytecode::unit::CompiledUnit;
use crate::bytecode::unit::has_attribute;
use crate::optimizer::OptimizationConfiguration;
use crate::optimizer::OptimizationStatistics;
use crate::optimizer::cfg::control_flow_targets;
use crate::optimizer::cfg::is_block_boundary;
use crate::optimizer::cfg::relative_target;
use crate::optimizer::passes::for_each_mutable_chunk;
use crate::unwrap_result_invariant;
use crate::value::atom::Atom;

struct ColdCallables {
    functions: Vec<Atom>,
    methods: Vec<Atom>,
    static_methods: Vec<(Atom, Atom)>,
}

pub(in crate::optimizer) fn optimize_unit(
    unit: &mut CompiledUnit,
    configuration: OptimizationConfiguration,
    statistics: &mut OptimizationStatistics,
) {
    if !configuration.cold_block_layout {
        return;
    }

    let callables = ColdCallables {
        functions: unit
            .functions
            .iter()
            .filter(|function| has_attribute(&function.attributes, COLD_ATTRIBUTE))
            .map(|function| function.name.clone())
            .collect(),
        methods: unit
            .classes
            .iter()
            .flat_map(|class| &class.methods)
            .filter(|method| {
                !method.is_static && has_attribute(&method.function.attributes, COLD_ATTRIBUTE)
            })
            .map(|method| method.name.clone())
            .collect(),
        static_methods: unit
            .classes
            .iter()
            .flat_map(|class| {
                class
                    .methods
                    .iter()
                    .filter(|method| {
                        method.is_static
                            && has_attribute(&method.function.attributes, COLD_ATTRIBUTE)
                    })
                    .map(|method| (class.name.clone(), method.name.clone()))
            })
            .collect(),
    };

    if callables.functions.is_empty()
        && callables.methods.is_empty()
        && callables.static_methods.is_empty()
    {
        return;
    }

    for_each_mutable_chunk(unit, configuration, |chunk| {
        optimize_chunk(chunk, &callables, statistics);
    });
}

fn optimize_chunk(
    chunk: &mut Chunk,
    callables: &ColdCallables,
    statistics: &mut OptimizationStatistics,
) {
    if !chunk.catch_table.is_empty()
        || chunk.code.len() < 4
        || chunk.code.len() >= i16::MAX as usize
        || chunk.code.iter().any(|instruction| {
            matches!(
                instruction,
                Instruction::SwitchInt { .. }
                    | Instruction::SwitchString { .. }
                    | Instruction::SwitchBool { .. }
                    | Instruction::SwitchFloat { .. }
                    | Instruction::SwitchPattern { .. }
                    | Instruction::SwitchTuplePattern { .. }
            )
        })
    {
        return;
    }

    let mut relocated = 0;
    while let Some(candidate) = find_candidate(chunk, callables) {
        relocate(chunk, candidate);
        statistics.cold_blocks_relocated += 1;
        relocated += 1;
        if relocated == chunk.code.len() {
            break;
        }
    }
}

#[derive(Clone, Copy)]
struct Candidate {
    branch: usize,
    hot_start: usize,
    inverted: Instruction,
    returns_to_hot: bool,
}

fn find_candidate(chunk: &Chunk, callables: &ColdCallables) -> Option<Candidate> {
    if !hard_terminator(*chunk.code.last()?) {
        return None;
    }

    let targets = control_flow_targets(chunk);
    for branch in 0..chunk.code.len() - 1 {
        let Some((hot_start, inverted)) = invert_forward_branch(chunk.code[branch], branch) else {
            continue;
        };
        let cold_start = branch + 1;
        if hot_start <= cold_start
            || hot_start >= chunk.code.len()
            || targets
                .iter()
                .any(|target| *target >= cold_start && *target < hot_start && *target != cold_start)
        {
            continue;
        }

        let mut calls_cold = false;
        let mut returns_to_hot = true;
        let mut structurally_safe = true;
        for index in cold_start..hot_start {
            let instruction = chunk.code[index];
            calls_cold |= is_cold_call(chunk, instruction, callables);
            if hard_terminator(instruction) {
                returns_to_hot = false;
                break;
            }
            if is_block_boundary(instruction) {
                structurally_safe = false;
                break;
            }
        }

        if calls_cold && structurally_safe {
            return Some(Candidate {
                branch,
                hot_start,
                inverted,
                returns_to_hot,
            });
        }
    }

    None
}

fn invert_forward_branch(instruction: Instruction, index: usize) -> Option<(usize, Instruction)> {
    match instruction {
        Instruction::JumpIfFalse { condition, offset } => Some((
            relative_target(index, offset.offset()),
            Instruction::JumpIfTrue {
                condition,
                offset: JumpOffset::new(1),
            },
        )),
        Instruction::JumpIfTrue { condition, offset } => Some((
            relative_target(index, offset.offset()),
            Instruction::JumpIfFalse {
                condition,
                offset: JumpOffset::new(1),
            },
        )),
        Instruction::JumpIfNull { subject, offset } => Some((
            relative_target(index, offset.offset()),
            Instruction::JumpIfNotNull {
                subject,
                offset: JumpOffset::new(1),
            },
        )),
        Instruction::JumpIfNotNull { subject, offset } => Some((
            relative_target(index, offset.offset()),
            Instruction::JumpIfNull {
                subject,
                offset: JumpOffset::new(1),
            },
        )),
        Instruction::JumpUnless {
            comparison,
            left,
            right,
            offset,
        } => Some((
            relative_target(index, i32::from(offset.offset())),
            Instruction::JumpUnless {
                comparison: equality_complement(comparison)?,
                left,
                right,
                offset: ShortJumpOffset::new(1),
            },
        )),
        Instruction::IntJumpUnless {
            comparison,
            left,
            right,
            offset,
        } => Some((
            relative_target(index, i32::from(offset.offset())),
            Instruction::IntJumpUnless {
                comparison: comparison.negated(),
                left,
                right,
                offset: ShortJumpOffset::new(1),
            },
        )),
        Instruction::StringJumpUnless {
            comparison,
            left,
            right,
            offset,
        } => Some((
            relative_target(index, i32::from(offset.offset())),
            Instruction::StringJumpUnless {
                comparison: comparison.negated(),
                left,
                right,
                offset: ShortJumpOffset::new(1),
            },
        )),
        Instruction::StringByteJumpUnlessEqual {
            container,
            index: string_index,
            byte,
            offset,
        } => Some((
            relative_target(index, i32::from(offset.offset())),
            Instruction::StringByteJumpUnlessNotEqual {
                container,
                index: string_index,
                byte,
                offset: ShortJumpOffset::new(1),
            },
        )),
        Instruction::StringByteJumpUnlessNotEqual {
            container,
            index: string_index,
            byte,
            offset,
        } => Some((
            relative_target(index, i32::from(offset.offset())),
            Instruction::StringByteJumpUnlessEqual {
                container,
                index: string_index,
                byte,
                offset: ShortJumpOffset::new(1),
            },
        )),
        Instruction::IntJumpUnlessImmediate {
            comparison,
            source,
            immediate,
            offset,
        } => Some((
            relative_target(index, i32::from(offset.offset())),
            Instruction::IntJumpUnlessImmediate {
                comparison: comparison.negated(),
                source,
                immediate,
                offset: ShortJumpOffset::new(1),
            },
        )),
        Instruction::JumpUnlessConstant {
            comparison,
            source,
            constant,
            offset,
        } => Some((
            relative_target(index, i32::from(offset.offset())),
            Instruction::JumpUnlessConstant {
                comparison: equality_complement(comparison)?,
                source,
                constant,
                offset: ShortJumpOffset::new(1),
            },
        )),
        _ => None,
    }
}

fn equality_complement(comparison: Comparison) -> Option<Comparison> {
    match comparison {
        Comparison::Equal => Some(Comparison::NotEqual),
        Comparison::NotEqual => Some(Comparison::Equal),
        _ => None,
    }
}

fn is_cold_call(chunk: &Chunk, instruction: Instruction, callables: &ColdCallables) -> bool {
    let named = match instruction {
        Instruction::CallNamed { cache, .. }
        | Instruction::CallNamedUnchecked { cache, .. }
        | Instruction::CallNamedConstantUnchecked { cache, .. }
        | Instruction::CallNamedDiscarded { cache, .. } => Some(cache),
        _ => None,
    };
    if let Some(cache) = named {
        return descriptor_member(chunk, cache)
            .is_some_and(|name| contains_atom(&callables.functions, name));
    }

    let method = match instruction {
        Instruction::CallMethod { cache, .. }
        | Instruction::CallMethodUnchecked { cache, .. }
        | Instruction::CallMethodDirect { cache, .. }
        | Instruction::CallMethodDiscarded { cache, .. } => Some(cache),
        _ => None,
    };
    if let Some(cache) = method {
        return descriptor_member(chunk, cache)
            .is_some_and(|name| contains_atom(&callables.methods, name));
    }

    let static_method = match instruction {
        Instruction::CallStatic { cache, .. } | Instruction::CallStaticDiscarded { cache, .. } => {
            Some(cache)
        }
        _ => None,
    };
    static_method
        .and_then(|cache| descriptor_class_member(chunk, cache))
        .is_some_and(|(class, method)| {
            callables
                .static_methods
                .iter()
                .any(|(candidate_class, candidate_method)| {
                    candidate_class == class && candidate_method == method
                })
        })
}

fn descriptor_member(chunk: &Chunk, cache: IcSlot) -> Option<&Atom> {
    match &chunk.ic_descriptors[usize::from(cache.index())] {
        IcDescriptor::Member { name, .. } => Some(name),
        IcDescriptor::ClassMember { .. } => None,
    }
}

fn descriptor_class_member(chunk: &Chunk, cache: IcSlot) -> Option<(&Atom, &Atom)> {
    match &chunk.ic_descriptors[usize::from(cache.index())] {
        IcDescriptor::ClassMember { class, member, .. } => Some((class, member)),
        IcDescriptor::Member { .. } => None,
    }
}

fn contains_atom(atoms: &[Atom], wanted: &Atom) -> bool {
    atoms.iter().any(|candidate| candidate == wanted)
}

fn hard_terminator(instruction: Instruction) -> bool {
    matches!(
        instruction,
        Instruction::Return { .. }
            | Instruction::ReturnUnchecked { .. }
            | Instruction::ReturnReferenceUnchecked { .. }
            | Instruction::ReturnPairUnchecked { .. }
            | Instruction::ReturnScalarUnchecked { .. }
            | Instruction::ReturnNull
            | Instruction::ReturnNullUnchecked
            | Instruction::ReturnIntUnchecked { .. }
            | Instruction::Throw { .. }
            | Instruction::Rethrow
            | Instruction::ThrowUnhandledMatch { .. }
            | Instruction::Exit { .. }
            | Instruction::Panic { .. }
    )
}

fn relocate(chunk: &mut Chunk, candidate: Candidate) {
    let old_code = mem::take(&mut chunk.code);
    let old_spans = mem::take(&mut chunk.spans);
    let new_length = old_code.len() + usize::from(candidate.returns_to_hot);
    let mut order = Vec::with_capacity(old_code.len());
    order.extend(0..=candidate.branch);
    order.extend(candidate.hot_start..old_code.len());
    order.extend(candidate.branch + 1..candidate.hot_start);

    let mut old_to_new = vec![0; old_code.len() + 1];
    for (new_index, old_index) in order.iter().copied().enumerate() {
        old_to_new[old_index] = new_index;
    }
    old_to_new[old_code.len()] = new_length;

    chunk.code.reserve(new_length);
    chunk.spans.reserve(new_length);
    for (new_index, old_index) in order.into_iter().enumerate() {
        let mut instruction = if old_index == candidate.branch {
            candidate.inverted
        } else {
            old_code[old_index]
        };
        rebase_targets(chunk, &mut instruction, old_index, new_index, &old_to_new);
        chunk.code.push(instruction);
        chunk.spans.push(old_spans[old_index]);
    }

    if candidate.returns_to_hot {
        let source = chunk.code.len();
        let target = old_to_new[candidate.hot_start];
        // SAFETY: the surrounding invariant proves this result is successful.
        let offset = unsafe {
            unwrap_result_invariant(
                i32::try_from(target as i64 - source as i64),
                "a chunk's instruction indices fit a signed jump offset",
            )
        };
        chunk.code.push(Instruction::Jump {
            offset: JumpOffset::new(offset),
        });
        chunk.spans.push(old_spans[candidate.branch]);
    }
}
