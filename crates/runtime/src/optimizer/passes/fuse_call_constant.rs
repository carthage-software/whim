//! Literal arguments embedded directly in exact named-function calls.

use crate::bytecode::chunk::Chunk;
use crate::bytecode::chunk::descriptors::IcDescriptor;
use crate::bytecode::chunk::descriptors::Literal;
use crate::bytecode::instruction::Instruction;
use crate::bytecode::instruction::operands::ConstantIndex;
use crate::bytecode::instruction::operands::IcSlot;
use crate::bytecode::instruction::operands::Register;
use crate::bytecode::unit::CompiledFunction;
use crate::bytecode::unit::CompiledUnit;
use crate::optimizer::OptimizationConfiguration;
use crate::optimizer::OptimizationStatistics;
use crate::optimizer::cfg::control_flow_targets;
use crate::optimizer::liveness::effect::overwrites_register;
use crate::optimizer::passes::compact_removed_instructions;
use crate::optimizer::passes::for_each_mutable_chunk;
use crate::value::atom::Atom;

pub(in crate::optimizer) fn optimize_unit(
    unit: &mut CompiledUnit,
    configuration: OptimizationConfiguration,
    statistics: &mut OptimizationStatistics,
) {
    if !configuration.fuse_call_constant {
        return;
    }

    let functions = unit
        .functions
        .iter()
        .map(|function| BorrowableFunction {
            name: function.name.clone(),
            first_parameter: first_parameter_is_borrowable(function),
        })
        .collect::<Vec<_>>();

    for_each_mutable_chunk(unit, configuration, |chunk| {
        optimize_chunk(chunk, &functions, statistics);
    });
}

struct BorrowableFunction {
    name: Atom,
    first_parameter: bool,
}

fn optimize_chunk(
    chunk: &mut Chunk,
    functions: &[BorrowableFunction],
    statistics: &mut OptimizationStatistics,
) {
    if chunk.code.len() < 2 {
        return;
    }

    let targets = control_flow_targets(chunk);
    let mut remove = vec![false; chunk.code.len()];

    for call_index in 1..chunk.code.len() {
        let Instruction::CallNamedUnchecked {
            argument_count,
            destination,
            first_argument,
            cache,
        } = chunk.code[call_index]
        else {
            continue;
        };

        if argument_count.value() != 1 || targets.contains(&call_index) {
            continue;
        }

        if matches!(
            chunk.ic_descriptors.get(usize::from(cache.index())),
            Some(IcDescriptor::Member {
                type_arguments: Some(_),
                ..
            })
        ) {
            continue;
        }

        let load_index = call_index - 1;
        if remove[load_index] {
            continue;
        }

        let Instruction::LoadConstant {
            destination: argument,
            constant,
        } = chunk.code[load_index]
        else {
            continue;
        };

        if argument != first_argument {
            continue;
        }

        remove[load_index] = true;
        chunk.code[call_index] = Instruction::CallNamedConstantUnchecked {
            destination,
            constant,
            cache,
            borrowed: borrows_constant(chunk, constant, cache, functions),
        };
    }

    compact_removed_instructions(chunk, &remove, statistics);
}

fn borrows_constant(
    chunk: &Chunk,
    constant: ConstantIndex,
    cache: IcSlot,
    functions: &[BorrowableFunction],
) -> bool {
    if !matches!(
        chunk.constants.get(constant.index() as usize),
        Some(Literal::String(_))
    ) {
        return false;
    }

    let Some(IcDescriptor::Member { name, .. }) = chunk.ic_descriptors.get(cache.index() as usize)
    else {
        return false;
    };

    functions
        .iter()
        .find(|function| function.name == *name)
        .is_some_and(|function| function.first_parameter)
}

fn first_parameter_is_borrowable(function: &CompiledFunction) -> bool {
    !function.parameters.is_empty()
        && function
            .chunk
            .catch_table
            .iter()
            .all(|entry| entry.binding != Some(Register::new(0)))
        && function.chunk.code.iter().all(|instruction| {
            !consumes_register(*instruction, Register::new(0))
                && !overwrites_register(&function.chunk, *instruction, Register::new(0))
        })
}

fn consumes_register(instruction: Instruction, register: Register) -> bool {
    match instruction {
        Instruction::Return { source }
        | Instruction::ReturnUnchecked { source }
        | Instruction::ReturnReferenceUnchecked { source }
        | Instruction::ReturnScalarUnchecked { source } => source == register,
        Instruction::ReturnPairUnchecked { first, second } => {
            first == register || second == register
        }
        Instruction::CallValue {
            argument_count,
            first_argument,
            ..
        }
        | Instruction::CallNamed {
            argument_count,
            first_argument,
            ..
        }
        | Instruction::CallNamedUnchecked {
            argument_count,
            first_argument,
            ..
        }
        | Instruction::CallMethod {
            argument_count,
            first_argument,
            ..
        }
        | Instruction::CallMethodUnchecked {
            argument_count,
            first_argument,
            ..
        }
        | Instruction::CallStatic {
            argument_count,
            first_argument,
            ..
        }
        | Instruction::CallSelfUnchecked {
            argument_count,
            first_argument,
            ..
        } => window_contains(first_argument, argument_count.value(), register),
        _ => false,
    }
}

fn window_contains(first: Register, count: u8, register: Register) -> bool {
    register_in_window(first, u16::from(count), register)
}

fn register_in_window(first: Register, count: u16, register: Register) -> bool {
    let first = first.index();
    let register = register.index();
    register >= first && register - first < count
}
