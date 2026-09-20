use whim_bytecode::chunk::descriptors::SwitchTable;
use whim_bytecode::chunk::descriptors::check_trivial_descriptor;
use whim_bytecode::chunk::descriptors::string_switch_lookup;
use whim_bytecode::instruction::Instruction;
use whim_bytecode::instruction::operands::Count;
use whim_bytecode::instruction::operands::Register;
use whim_bytecode::rewrite::relative_target;
use whim_value::Value;
use whim_value::tuple::TupleObject;

use crate::type_flow::ConstantValue;
use crate::type_flow::TypeFlow;
use crate::type_flow::constants::constant_comparison;
use crate::type_flow::constants::constant_from_literal;
use crate::type_flow::instruction_index;

impl TypeFlow<'_> {
    pub(crate) fn constant_branch_offset(&self, index: usize) -> Option<i32> {
        let instruction = self.chunk.code[index];
        let value = |register| self.constant_value(index, register);
        let (taken, offset) = match instruction {
            Instruction::JumpIfFalse { condition, offset }
            | Instruction::JumpIfTrue { condition, offset } => {
                let ConstantValue::Bool(value) = value(condition)? else {
                    return None;
                };

                (
                    value == matches!(instruction, Instruction::JumpIfTrue { .. }),
                    offset.offset(),
                )
            }
            Instruction::JumpIfNull { subject, offset }
            | Instruction::JumpIfNotNull { subject, offset } => (
                matches!(value(subject)?, ConstantValue::Null)
                    == matches!(instruction, Instruction::JumpIfNull { .. }),
                offset.offset(),
            ),
            Instruction::JumpUnless {
                comparison,
                left,
                right,
                offset,
            }
            | Instruction::IntJumpUnless {
                comparison,
                left,
                right,
                offset,
            }
            | Instruction::StringJumpUnless {
                comparison,
                left,
                right,
                offset,
            } => (
                !constant_comparison(comparison, &value(left)?, &value(right)?)?,
                i32::from(offset.offset()),
            ),
            Instruction::JumpUnlessConstant {
                comparison,
                source,
                constant,
                offset,
            } => (
                !constant_comparison(
                    comparison,
                    &value(source)?,
                    &constant_from_literal(&self.chunk.constants[usize::from(constant.index())]),
                )?,
                i32::from(offset.offset()),
            ),
            Instruction::IntJumpUnlessImmediate {
                comparison,
                source,
                immediate,
                offset,
            } => (
                !constant_comparison(
                    comparison,
                    &value(source)?,
                    &ConstantValue::Int(i64::from(immediate.value())),
                )?,
                i32::from(offset.offset()),
            ),
            Instruction::BoolPatternBranch {
                subject,
                false_offset,
                default_offset,
            } => {
                return Some(match value(subject)? {
                    ConstantValue::Bool(true) => 1,
                    ConstantValue::Bool(false) => i32::from(false_offset.offset()),
                    _ => i32::from(default_offset.offset()),
                });
            }
            Instruction::SwitchInt { subject, table }
            | Instruction::SwitchString { subject, table }
            | Instruction::SwitchBool { subject, table }
            | Instruction::SwitchFloat { subject, table } => {
                return switch_offset(
                    &self.chunk.switch_tables[usize::from(table.index())],
                    &value(subject)?,
                );
            }
            Instruction::SwitchPattern { subject, table } => {
                return pattern_offset(
                    &self.chunk.switch_tables[usize::from(table.index())],
                    &self.constant_pattern_value(index, subject)?,
                );
            }
            Instruction::SwitchTuplePattern {
                first_element,
                element_count,
                table,
            } => {
                return pattern_offset(
                    &self.chunk.switch_tables[usize::from(table.index())],
                    &self.constant_tuple_value(index, first_element, element_count)?,
                );
            }
            _ => return None,
        };

        if !taken
            && offset > 1
            && let Some(
                Instruction::CounterLoop {
                    offset: back_edge, ..
                }
                | Instruction::IntCounterLoop {
                    offset: back_edge, ..
                },
            ) = self.chunk.code.get(relative_target(index, offset) - 1)
            && relative_target(
                relative_target(index, offset) - 1,
                i32::from(back_edge.offset()),
            ) == index + 1
        {
            return None;
        }

        Some(if taken { offset } else { 1 })
    }

    fn constant_pattern_value(&self, index: usize, register: Register) -> Option<Value> {
        if let Some(value) = self.constant_value(index, register) {
            return Some(runtime_value(value));
        }
        let producer = instruction_index(self.fact(index, register).origin)?;
        let Instruction::NewTuple {
            first_element,
            element_count,
            ..
        } = self.chunk.code[producer]
        else {
            return None;
        };
        self.constant_tuple_value(producer, first_element, element_count)
    }

    fn constant_tuple_value(
        &self,
        index: usize,
        first_element: Register,
        element_count: Count,
    ) -> Option<Value> {
        let elements = (0..element_count.value())
            .map(|offset| {
                self.constant_value(
                    index,
                    Register::new(first_element.index() + u16::from(offset)),
                )
                .map(runtime_value)
            })
            .collect::<Option<Vec<_>>>()?;
        Some(Value::tuple(TupleObject::with_elements(
            self.allocator,
            elements,
        )))
    }
}

fn runtime_value(value: ConstantValue) -> Value {
    match value {
        ConstantValue::Null => Value::null(),
        ConstantValue::Bool(value) => Value::bool(value),
        ConstantValue::Int(value) => Value::int(value),
        ConstantValue::Float(value) => Value::float(value),
        ConstantValue::String(value) => Value::string(value.to_handle()),
    }
}

fn pattern_offset(table: &SwitchTable, value: &Value) -> Option<i32> {
    let SwitchTable::Pattern {
        descriptors,
        targets,
        default,
    } = table
    else {
        return None;
    };
    for (descriptor, target) in descriptors.iter().zip(targets) {
        if check_trivial_descriptor(descriptor, value)? {
            return Some(*target);
        }
    }
    Some(*default)
}

fn switch_offset(table: &SwitchTable, value: &ConstantValue) -> Option<i32> {
    Some(match table {
        SwitchTable::Int {
            base,
            targets,
            default,
        } => {
            let ConstantValue::Int(value) = value else {
                return Some(*default);
            };

            value
                .checked_sub(*base)
                .and_then(|index| usize::try_from(index).ok())
                .and_then(|index| targets.get(index))
                .copied()
                .unwrap_or(*default)
        }
        SwitchTable::String {
            arms,
            buckets,
            default,
        } => {
            let ConstantValue::String(value) = value else {
                return Some(*default);
            };

            string_switch_lookup(arms, buckets, value.as_bytes())
                .map_or(*default, |index| arms[index].1)
        }
        SwitchTable::StringByte {
            base,
            targets,
            default,
        } => {
            let ConstantValue::String(value) = value else {
                return Some(*default);
            };

            let [byte] = value.as_bytes() else {
                return Some(*default);
            };

            byte.checked_sub(*base)
                .and_then(|index| targets.get(usize::from(index)))
                .copied()
                .unwrap_or(*default)
        }
        SwitchTable::Bool { targets, default } => match value {
            ConstantValue::Bool(value) => targets[usize::from(*value)],
            _ => *default,
        },
        SwitchTable::Float {
            values,
            targets,
            default,
        } => {
            let ConstantValue::Float(value) = value else {
                return Some(*default);
            };

            values
                .iter()
                .zip(targets)
                .find_map(|(candidate, target)| (candidate == value).then_some(*target))
                .unwrap_or(*default)
        }
        _ => return None,
    })
}
