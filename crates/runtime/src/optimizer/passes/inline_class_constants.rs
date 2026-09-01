//! Inlining of public literal class constants declared in the same unit.

use crate::bytecode::chunk::Chunk;
use crate::bytecode::chunk::descriptors::IcDescriptor;
use crate::bytecode::chunk::descriptors::Literal;
use crate::bytecode::instruction::Instruction;
use crate::bytecode::instruction::operands::ImmediateInt;
use crate::bytecode::instruction::operands::Register;
use crate::bytecode::unit::CompiledAttribute;
use crate::bytecode::unit::CompiledUnit;
use crate::bytecode::unit::ConstantInitializer;
use crate::bytecode::unit::Visibility;
use crate::optimizer::OptimizationConfiguration;
use crate::optimizer::OptimizationStatistics;
use crate::value::atom::Atom;

struct LiteralClassConstant {
    class: Atom,
    member: Atom,
    literal: Literal,
}

pub(in crate::optimizer) fn optimize_unit(
    unit: &mut CompiledUnit,
    configuration: OptimizationConfiguration,
    statistics: &mut OptimizationStatistics,
) {
    if !configuration.inline_class_constants {
        return;
    }

    let mut constants = Vec::new();
    for class in &unit.classes {
        for constant in &class.constants {
            if constant.visibility != Visibility::Public {
                continue;
            }

            let ConstantInitializer::Literal(literal) = &constant.initializer else {
                continue;
            };

            constants.push(LiteralClassConstant {
                class: class.name.clone(),
                member: constant.name.clone(),
                literal: literal.clone(),
            });
        }
    }

    if constants.is_empty() {
        return;
    }

    optimize_chunk(&mut unit.main, &constants, statistics);
    let function_floor = configuration.function_floor(unit.functions.len());
    for function in &mut unit.functions[function_floor..] {
        optimize_attributes(&mut function.attributes, &constants, statistics);
        for parameter in &mut function.parameters {
            optimize_attributes(&mut parameter.attributes, &constants, statistics);
        }

        optimize_chunk(&mut function.chunk, &constants, statistics);
    }

    let class_floor = configuration.class_floor(unit.classes.len());
    for class in &mut unit.classes[class_floor..] {
        optimize_attributes(&mut class.attributes, &constants, statistics);
        for constant in &mut class.constants {
            optimize_initializer(&mut constant.initializer, &constants, statistics);
            optimize_attributes(&mut constant.attributes, &constants, statistics);
        }

        for property in &mut class.properties {
            if let Some(default) = &mut property.default {
                optimize_initializer(default, &constants, statistics);
            }

            optimize_attributes(&mut property.attributes, &constants, statistics);
        }

        for method in &mut class.methods {
            optimize_attributes(&mut method.function.attributes, &constants, statistics);
            for parameter in &mut method.function.parameters {
                optimize_attributes(&mut parameter.attributes, &constants, statistics);
            }

            optimize_chunk(&mut method.function.chunk, &constants, statistics);
        }

        for case in &mut class.cases {
            if let Some(value) = &mut case.value {
                optimize_initializer(value, &constants, statistics);
            }
        }
    }

    let constant_floor = configuration.constant_floor(unit.constants.len());
    for constant in &mut unit.constants[constant_floor..] {
        optimize_initializer(&mut constant.initializer, &constants, statistics);
    }
}

fn optimize_attributes(
    attributes: &mut [CompiledAttribute],
    constants: &[LiteralClassConstant],
    statistics: &mut OptimizationStatistics,
) {
    for attribute in attributes {
        for argument in &mut attribute.arguments {
            optimize_initializer(argument, constants, statistics);
        }
        for (_, argument) in &mut attribute.named_arguments {
            optimize_initializer(argument, constants, statistics);
        }
    }
}

fn optimize_initializer(
    initializer: &mut ConstantInitializer,
    constants: &[LiteralClassConstant],
    statistics: &mut OptimizationStatistics,
) {
    if let ConstantInitializer::Thunk(chunk) = initializer {
        optimize_chunk(chunk, constants, statistics);
    }
}

fn optimize_chunk(
    chunk: &mut Chunk,
    constants: &[LiteralClassConstant],
    statistics: &mut OptimizationStatistics,
) {
    for index in 0..chunk.code.len() {
        let Instruction::ClassConstantGet { destination, cache } = chunk.code[index] else {
            continue;
        };
        let IcDescriptor::ClassMember { class, member, .. } =
            &chunk.ic_descriptors[usize::from(cache.index())]
        else {
            continue;
        };
        let Some(literal) = constants.iter().find_map(|constant| {
            (constant.class.as_bytes() == class.as_bytes()
                && constant.member.as_bytes() == member.as_bytes())
            .then(|| constant.literal.clone())
        }) else {
            continue;
        };
        let Some(instruction) = literal_instruction(chunk, destination, literal) else {
            continue;
        };
        chunk.code[index] = instruction;
        statistics.constants_folded += 1;
    }
}

fn literal_instruction(
    chunk: &mut Chunk,
    destination: Register,
    literal: Literal,
) -> Option<Instruction> {
    match literal {
        Literal::Null => Some(Instruction::LoadNull { destination }),
        Literal::Bool(true) => Some(Instruction::LoadTrue { destination }),
        Literal::Bool(false) => Some(Instruction::LoadFalse { destination }),
        Literal::Int(value) => {
            if let Ok(immediate) = i16::try_from(value) {
                Some(Instruction::LoadInt {
                    destination,
                    immediate: ImmediateInt::new(immediate),
                })
            } else {
                let constant = chunk.add_constant(Literal::Int(value)).ok()?;
                Some(Instruction::LoadConstant {
                    destination,
                    constant,
                })
            }
        }
        literal @ (Literal::Float(_) | Literal::String(_)) => {
            let constant = chunk.add_constant(literal).ok()?;
            Some(Instruction::LoadConstant {
                destination,
                constant,
            })
        }
    }
}
