//! Conservative ownership metadata for fast frame teardown.

use crate::bytecode::REFERENCE_REGISTER_LIMIT;
use crate::bytecode::chunk::Chunk;
use crate::bytecode::chunk::descriptors::Literal;
use crate::bytecode::instruction::Instruction;
use crate::bytecode::instruction::operands::ArrayValueMode;
use crate::bytecode::instruction::operands::Register;

/// Bitmask of registers that may own a reference-counted value; a full mask
/// means use ordinary teardown for wider frames.
pub(crate) fn mask(chunk: &Chunk) -> u64 {
    mask_with_classification(chunk, |_| true)
}

#[expect(
    clippy::too_many_lines,
    reason = "the exhaustive instruction classification stays visible in one match"
)]
pub(crate) fn mask_with_classification(
    chunk: &Chunk,
    mut result_may_reference: impl FnMut(Instruction) -> bool,
) -> u64 {
    if chunk.register_count > REFERENCE_REGISTER_LIMIT {
        return u64::MAX;
    }

    let mut mask = 0u64;
    for entry in &chunk.catch_table {
        if let Some(binding) = entry.binding {
            mask |= 1u64 << binding.index();
        }
    }

    for instruction in &chunk.code {
        let destination = match *instruction {
            Instruction::LoadConstant {
                destination,
                constant,
            } => matches!(
                chunk.constants[constant.index() as usize],
                Literal::String(_)
            )
            .then_some(destination),
            Instruction::Move { destination, .. }
            | Instruction::AsCheck { destination, .. }
            | Instruction::AsOrNull { destination, .. }
            | Instruction::Concatenate { destination, .. }
            | Instruction::ConcatenateConstant { destination, .. }
            | Instruction::NewVec { destination, .. }
            | Instruction::NewFilledVec { destination, .. }
            | Instruction::NewDict { destination, .. }
            | Instruction::NewTuple { destination, .. }
            | Instruction::IndexGet { destination, .. }
            | Instruction::Rest { destination, .. }
            | Instruction::Remove { destination, .. }
            | Instruction::SwapRemove { destination, .. }
            | Instruction::RemoveFirst { destination, .. }
            | Instruction::RemoveLast { destination, .. }
            | Instruction::PropertyRemove { destination, .. }
            | Instruction::PropertyRemoveUnchecked { destination, .. }
            | Instruction::ElementGet { destination, .. }
            | Instruction::NewStatic { destination, .. }
            | Instruction::InitializeProperties {
                object: destination,
                ..
            }
            | Instruction::NewDynamic { destination, .. }
            | Instruction::NewTyped { destination, .. }
            | Instruction::PropertyGet { destination, .. }
            | Instruction::CloneObject { destination, .. }
            | Instruction::StaticPropertyGet { destination, .. }
            | Instruction::ConstantGet { destination, .. }
            | Instruction::ClassConstantGet { destination, .. }
            | Instruction::CallValue { destination, .. }
            | Instruction::CallValueUnchecked { destination, .. }
            | Instruction::CallValueDiscarded { destination, .. }
            | Instruction::CallNamed { destination, .. }
            | Instruction::CallNamedDiscarded { destination, .. }
            | Instruction::CallMethod { destination, .. }
            | Instruction::CallMethodDiscarded { destination, .. }
            | Instruction::CallMethodUnchecked { destination, .. }
            | Instruction::CallMethodDirect { destination, .. }
            | Instruction::CallStatic { destination, .. }
            | Instruction::CallStaticDiscarded { destination, .. }
            | Instruction::CallWithNames { destination, .. }
            | Instruction::CallWithNamesDiscarded { destination, .. }
            | Instruction::MakeClosure { destination, .. }
            | Instruction::MakeBound { destination, .. }
            | Instruction::Require { destination, .. }
            | Instruction::ForeachInit {
                iterator: destination,
                ..
            }
            | Instruction::DictIndexGetStringKey { destination, .. } => Some(destination),
            classified @ (Instruction::PropertyGetUnchecked { destination, .. }
            | Instruction::CallNamedUnchecked { destination, .. }
            | Instruction::CallNamedConstantUnchecked { destination, .. }
            | Instruction::CallSelfUnchecked { destination, .. }) => {
                result_may_reference(classified).then_some(destination)
            }
            Instruction::ForeachNext {
                key_destination,
                value_destination,
                ..
            } => {
                if key_destination != Register::NONE {
                    mask |= 1u64 << key_destination.index();
                }
                Some(value_destination)
            }
            Instruction::VecForeachNext {
                key_destination,
                value_destination,
                value_mode,
                ..
            }
            | Instruction::DictForeachNext {
                key_destination,
                value_destination,
                value_mode,
                ..
            } => {
                if key_destination != Register::NONE {
                    mask |= 1u64 << key_destination.index();
                }

                (value_mode == ArrayValueMode::Generic).then_some(value_destination)
            }
            Instruction::VecIndexGet {
                destination,
                value_mode,
                ..
            }
            | Instruction::DictIndexGetIntKey {
                destination,
                value_mode,
                ..
            } => (value_mode == ArrayValueMode::Generic).then_some(destination),
            Instruction::MoveOwned { .. }
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
            | Instruction::BitwiseAnd { .. }
            | Instruction::BitwiseOr { .. }
            | Instruction::BitwiseXor { .. }
            | Instruction::BitwiseNot { .. }
            | Instruction::ShiftLeft { .. }
            | Instruction::ShiftRight { .. }
            | Instruction::Equal { .. }
            | Instruction::StringByteEqual { .. }
            | Instruction::StringByteNotEqual { .. }
            | Instruction::StringByteLessThan { .. }
            | Instruction::StringByteLessThanOrEqual { .. }
            | Instruction::StringByteGreaterThan { .. }
            | Instruction::StringByteGreaterThanOrEqual { .. }
            | Instruction::NotEqual { .. }
            | Instruction::LessThan { .. }
            | Instruction::LessThanOrEqual { .. }
            | Instruction::GreaterThan { .. }
            | Instruction::GreaterThanOrEqual { .. }
            | Instruction::Compare { .. }
            | Instruction::Not { .. }
            | Instruction::Jump { .. }
            | Instruction::JumpIfFalse { .. }
            | Instruction::JumpIfTrue { .. }
            | Instruction::JumpIfNull { .. }
            | Instruction::JumpIfNotNull { .. }
            | Instruction::SwitchInt { .. }
            | Instruction::SwitchString { .. }
            | Instruction::SwitchBool { .. }
            | Instruction::SwitchFloat { .. }
            | Instruction::SwitchPattern { .. }
            | Instruction::SwitchTuplePattern { .. }
            | Instruction::IntRangeJumpIf { .. }
            | Instruction::IntRangeJumpUnless { .. }
            | Instruction::BoolPatternBranch { .. }
            | Instruction::CheckDefined { .. }
            | Instruction::IndexSet { .. }
            | Instruction::Append { .. }
            | Instruction::Spread { .. }
            | Instruction::Length { .. }
            | Instruction::CheckDestructure { .. }
            | Instruction::PropertySet { .. }
            | Instruction::PropertyInitRaw { .. }
            | Instruction::StaticPropertySet { .. }
            | Instruction::Return { .. }
            | Instruction::ReturnNull
            | Instruction::Is { .. }
            | Instruction::Throw { .. }
            | Instruction::Rethrow
            | Instruction::ThrowUnhandledMatch { .. }
            | Instruction::Write { .. }
            | Instruction::WriteLine { .. }
            | Instruction::WriteError { .. }
            | Instruction::WriteErrorLine { .. }
            | Instruction::Debug { .. }
            | Instruction::Assert { .. }
            | Instruction::Exit { .. }
            | Instruction::Panic { .. }
            | Instruction::FillDefault { .. }
            | Instruction::JumpUnless { .. }
            | Instruction::IncrementJump { .. }
            | Instruction::Squares { .. }
            | Instruction::CounterLoop { .. }
            | Instruction::NumericLoop { .. }
            | Instruction::PropertyIndexUpdate { .. }
            | Instruction::PropertyIndexSet { .. }
            | Instruction::PropertyStep { .. }
            | Instruction::PropertyAdd { .. }
            | Instruction::ReturnUnchecked { .. }
            | Instruction::ReturnNullUnchecked
            | Instruction::FloatAdd { .. }
            | Instruction::FloatSubtract { .. }
            | Instruction::FloatMultiply { .. }
            | Instruction::FloatSquares { .. }
            | Instruction::FloatMultiplyConstant { .. }
            | Instruction::JumpUnlessConstant { .. }
            | Instruction::PropertyFillIntRange { .. }
            | Instruction::FloatSquaresSum { .. }
            | Instruction::FloatDifferenceAdd { .. }
            | Instruction::FloatScaleProductAdd { .. }
            | Instruction::FloatSquaresSumBranch { .. }
            | Instruction::IntCounterLoop { .. }
            | Instruction::IntNumericLoop { .. }
            | Instruction::PreparedIntNumericLoop { .. }
            | Instruction::IntStepLoop { .. }
            | Instruction::FloatPairUpdate { .. }
            | Instruction::IntJumpUnless { .. }
            | Instruction::StringJumpUnless { .. }
            | Instruction::StringByteJumpUnlessEqual { .. }
            | Instruction::StringByteJumpUnlessNotEqual { .. }
            | Instruction::PropertySetUnchecked { .. }
            | Instruction::PropertyIndexUpdateUnchecked { .. }
            | Instruction::PropertyIndexSetUnchecked { .. }
            | Instruction::PropertyStepUnchecked { .. }
            | Instruction::PropertyAddUnchecked { .. }
            | Instruction::IntAdd { .. }
            | Instruction::IntSubtract { .. }
            | Instruction::IntMultiply { .. }
            | Instruction::IntModulo { .. }
            | Instruction::IntMultiplyImmediate { .. }
            | Instruction::IntModuloImmediate { .. }
            | Instruction::VecIndexSet { .. }
            | Instruction::VecAppend { .. }
            | Instruction::DictIndexSetIntKey { .. }
            | Instruction::DictIndexSetStringKey { .. }
            | Instruction::DictIndexSet { .. }
            | Instruction::ReserveArray { .. }
            | Instruction::Contains { .. }
            | Instruction::ContainsKey { .. }
            | Instruction::StringIndexGet { .. }
            | Instruction::IndexAddAssign { .. }
            | Instruction::NumericRegionJump { .. }
            | Instruction::StringLength { .. }
            | Instruction::IntAddAssign { .. }
            | Instruction::IntJumpUnlessImmediate { .. }
            | Instruction::ReturnIntUnchecked { .. }
            | Instruction::ReturnReferenceUnchecked { .. }
            | Instruction::ReturnPairUnchecked { .. }
            | Instruction::ReturnScalarUnchecked { .. }
            | Instruction::IntBitwiseAnd { .. }
            | Instruction::IntBitwiseOr { .. }
            | Instruction::IntBitwiseXor { .. }
            | Instruction::IntBitwiseNot { .. }
            | Instruction::IntShiftLeft { .. }
            | Instruction::IntShiftRight { .. }
            | Instruction::DrainFinalizers
            | Instruction::Clear { .. }
            | Instruction::CheckSoleReference { .. }
            | Instruction::CheckDiscardedResult { .. } => None,
        };

        if let Some(destination) = destination {
            mask |= 1u64 << destination.index();
        }
    }

    loop {
        let previous = mask;
        for instruction in &chunk.code {
            let Instruction::MoveOwned {
                destination,
                source,
            } = *instruction
            else {
                continue;
            };
            if mask & (1u64 << source.index()) != 0 {
                mask |= 1u64 << destination.index();
            }
        }
        if mask == previous {
            break;
        }
    }

    mask
}
