//! Retargeting an instruction's destination register.

use whim_bytecode::instruction::Instruction;
use whim_bytecode::instruction::operands::ArrayValueMode;
use whim_bytecode::instruction::operands::Register;

pub(crate) fn with_destination(
    mut instruction: Instruction,
    destination: Register,
    expected: Register,
) -> Option<Instruction> {
    if let Instruction::Add {
        kind: Some(kind),
        destination: current,
        left,
        right,
    } = instruction
    {
        if current != expected {
            return None;
        }

        return Some(if destination == left {
            Instruction::IntegerAddAssign {
                kind,
                target: destination,
                source: right,
            }
        } else if destination == right {
            Instruction::IntegerAddAssign {
                kind,
                target: destination,
                source: left,
            }
        } else {
            Instruction::Add {
                kind: Some(kind),
                destination,
                left,
                right,
            }
        });
    }

    let current = match &mut instruction {
        Instruction::VecIndexGet {
            destination,
            value_mode,
            ..
        }
        | Instruction::DictIndexGetIntKey {
            destination,
            value_mode,
            ..
        }
        | Instruction::DictIndexGetUintKey {
            destination,
            value_mode,
            ..
        }
        | Instruction::DictIndexGetStringKey {
            destination,
            value_mode,
            ..
        } if *value_mode != ArrayValueMode::Generic => destination,
        Instruction::AddImmediate { destination, .. }
        | Instruction::SubtractImmediate { destination, .. }
        | Instruction::Step { destination, .. }
        | Instruction::IndexGetOrNull { destination, .. }
        | Instruction::VecIndexGetOrNull { destination, .. }
        | Instruction::DictIndexGetIntegerKeyOrNull { destination, .. }
        | Instruction::DictIndexGetStringKeyOrNull { destination, .. }
        | Instruction::StringIndexGetOrNull { destination, .. }
        | Instruction::PropertyGetOrNull { destination, .. }
        | Instruction::PropertyGetOrNullUnchecked { destination, .. }
        | Instruction::StaticPropertyGetOrNull { destination, .. }
        | Instruction::Move { destination, .. }
        | Instruction::LoadConstant { destination, .. }
        | Instruction::LoadNull { destination }
        | Instruction::LoadTrue { destination }
        | Instruction::LoadFalse { destination }
        | Instruction::LoadInteger { destination, .. }
        | Instruction::Add { destination, .. }
        | Instruction::Subtract { destination, .. }
        | Instruction::Multiply { destination, .. }
        | Instruction::Modulo { destination, .. }
        | Instruction::FloatAdd { destination, .. }
        | Instruction::FloatSubtract { destination, .. }
        | Instruction::FloatMultiply { destination, .. }
        | Instruction::Divide { destination, .. }
        | Instruction::Power { destination, .. }
        | Instruction::Negate { destination, .. }
        | Instruction::UnaryPlus { destination, .. }
        | Instruction::IntegerMultiplyImmediate { destination, .. }
        | Instruction::IntegerModuloImmediate { destination, .. }
        | Instruction::FloatMultiplyConstant { destination, .. }
        | Instruction::FloatDifferenceAdd { destination, .. }
        | Instruction::FloatScaleProductAdd { destination, .. }
        | Instruction::Concatenate { destination, .. }
        | Instruction::ConcatenateRightConstant { destination, .. }
        | Instruction::ConcatenateLeftConstant { destination, .. }
        | Instruction::BitwiseAnd { destination, .. }
        | Instruction::BitwiseOr { destination, .. }
        | Instruction::BitwiseXor { destination, .. }
        | Instruction::BitwiseNot { destination, .. }
        | Instruction::ShiftLeft { destination, .. }
        | Instruction::ShiftRight { destination, .. }
        | Instruction::Equal { destination, .. }
        | Instruction::NotEqual { destination, .. }
        | Instruction::LessThan { destination, .. }
        | Instruction::LessThanOrEqual { destination, .. }
        | Instruction::GreaterThan { destination, .. }
        | Instruction::GreaterThanOrEqual { destination, .. }
        | Instruction::Compare { destination, .. }
        | Instruction::Not { destination, .. }
        | Instruction::NewArray { destination, .. }
        | Instruction::IndexGet { destination, .. }
        | Instruction::IndexGetPath { destination, .. }
        | Instruction::StringIndexGet { destination, .. }
        | Instruction::StringByteEqual { destination, .. }
        | Instruction::StringByteNotEqual { destination, .. }
        | Instruction::StringByteLessThan { destination, .. }
        | Instruction::StringByteLessThanOrEqual { destination, .. }
        | Instruction::StringByteGreaterThan { destination, .. }
        | Instruction::StringByteGreaterThanOrEqual { destination, .. }
        | Instruction::Rest { destination, .. }
        | Instruction::Length { destination, .. }
        | Instruction::StringLength { destination, .. }
        | Instruction::Remove { destination, .. }
        | Instruction::SwapRemove { destination, .. }
        | Instruction::RemoveFirst { destination, .. }
        | Instruction::RemoveLast { destination, .. }
        | Instruction::ElementGet { destination, .. }
        | Instruction::NewStatic { destination, .. }
        | Instruction::NewDynamic { destination, .. }
        | Instruction::NewTyped { destination, .. }
        | Instruction::PropertyGet { destination, .. }
        | Instruction::PropertyGetUnchecked { destination, .. }
        | Instruction::CloneObject { destination, .. }
        | Instruction::StaticPropertyGet { destination, .. }
        | Instruction::ConstantGet { destination, .. }
        | Instruction::ClassConstantGet { destination, .. }
        | Instruction::CallValue { destination, .. }
        | Instruction::CallValueUnchecked { destination, .. }
        | Instruction::CallNamed { destination, .. }
        | Instruction::CallNamedUnchecked { destination, .. }
        | Instruction::CallNamedConstantUnchecked { destination, .. }
        | Instruction::CallMethod { destination, .. }
        | Instruction::CallMethodUnchecked { destination, .. }
        | Instruction::CallNamedDirect { destination, .. }
        | Instruction::CallMethodDirect { destination, .. }
        | Instruction::CallStatic { destination, .. }
        | Instruction::CallWithNames { destination, .. }
        | Instruction::MakeClosure { destination, .. }
        | Instruction::MakeBound { destination, .. }
        | Instruction::Is { destination, .. }
        | Instruction::AsCheck { destination, .. }
        | Instruction::AsOrNull { destination, .. }
        | Instruction::Require { destination, .. } => destination,
        _ => return None,
    };

    if *current != expected {
        return None;
    }

    *current = destination;
    Some(instruction)
}
