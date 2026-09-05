//! Cheap discovery of the analyses and rewrites a chunk can use.

use crate::bytecode::chunk::Chunk;
use crate::bytecode::instruction::Instruction;
use crate::optimizer::OptimizationConfiguration;

/// The type-flow consumers that have at least one candidate in a chunk.
#[derive(Clone, Copy, Default, PartialEq, Eq)]
pub(in crate::optimizer) struct CandidateSet(u16);

impl CandidateSet {
    pub(in crate::optimizer) const ARITHMETIC: Self = Self(1 << 0);
    pub(in crate::optimizer) const CALL: Self = Self(1 << 1);
    pub(in crate::optimizer) const COLLECTION: Self = Self(1 << 2);
    pub(in crate::optimizer) const COMPARISON: Self = Self(1 << 3);
    pub(in crate::optimizer) const CONSTANT: Self = Self(1 << 4);
    pub(in crate::optimizer) const COUNTER_LOOP: Self = Self(1 << 5);
    pub(in crate::optimizer) const DEAD_STORE: Self = Self(1 << 6);
    pub(in crate::optimizer) const DISCARDED_RESULT: Self = Self(1 << 7);
    pub(in crate::optimizer) const OWNERSHIP: Self = Self(1 << 8);
    pub(in crate::optimizer) const PROPERTY: Self = Self(1 << 9);
    pub(in crate::optimizer) const TYPE_CHECK: Self = Self(1 << 10);
    pub(in crate::optimizer) const EARLY_OPERATION: Self = Self(1 << 11);

    pub(in crate::optimizer) fn of(
        chunk: &Chunk,
        configuration: OptimizationConfiguration,
    ) -> Self {
        let mut candidates = Self::default();
        for instruction in &chunk.code {
            candidates.insert(instruction_candidates(*instruction, configuration));
        }

        candidates
    }

    pub(in crate::optimizer) const fn contains(self, candidate: Self) -> bool {
        self.0 & candidate.0 != 0
    }

    pub(in crate::optimizer) const fn is_empty(self) -> bool {
        self.0 == 0
    }

    pub(in crate::optimizer) const fn needs_array_elements(self) -> bool {
        self.contains(Self::CALL)
            || self.contains(Self::COLLECTION)
            || self.contains(Self::PROPERTY)
            || self.contains(Self::TYPE_CHECK)
    }

    pub(in crate::optimizer) const fn needs_constant_cache(self) -> bool {
        self.contains(Self::CONSTANT)
            || self.contains(Self::ARITHMETIC)
            || self.contains(Self::COMPARISON)
            || self.contains(Self::COUNTER_LOOP)
    }

    const fn insert(&mut self, candidates: Self) {
        self.0 |= candidates.0;
    }
}

fn instruction_candidates(
    instruction: Instruction,
    configuration: OptimizationConfiguration,
) -> CandidateSet {
    let mut candidates = CandidateSet::default();

    if (configuration.specialize_arithmetic
        || configuration.strength_reduction
        || configuration.const_fold)
        && matches!(
            instruction,
            Instruction::Add { .. }
                | Instruction::Subtract { .. }
                | Instruction::Multiply { .. }
                | Instruction::Divide { .. }
                | Instruction::Modulo { .. }
                | Instruction::Power { .. }
                | Instruction::Negate { .. }
                | Instruction::UnaryPlus { .. }
                | Instruction::BitwiseAnd { .. }
                | Instruction::BitwiseOr { .. }
                | Instruction::BitwiseXor { .. }
                | Instruction::BitwiseNot { .. }
                | Instruction::ShiftLeft { .. }
                | Instruction::ShiftRight { .. }
                | Instruction::IntAdd { .. }
                | Instruction::IntSubtract { .. }
        )
    {
        candidates.insert(CandidateSet::ARITHMETIC);
    }

    if configuration.strength_reduction
        && matches!(
            instruction,
            Instruction::AddImmediate { .. }
                | Instruction::SubtractImmediate { .. }
                | Instruction::IntMultiplyImmediate { .. }
        )
    {
        candidates.insert(CandidateSet::ARITHMETIC);
    }

    if configuration.specialize_arithmetic
        && matches!(
            instruction,
            Instruction::Add { .. }
                | Instruction::Subtract { .. }
                | Instruction::Multiply { .. }
                | Instruction::Modulo { .. }
                | Instruction::BitwiseAnd { .. }
                | Instruction::BitwiseOr { .. }
                | Instruction::BitwiseXor { .. }
                | Instruction::BitwiseNot { .. }
                | Instruction::ShiftLeft { .. }
                | Instruction::ShiftRight { .. }
        )
    {
        candidates.insert(CandidateSet::EARLY_OPERATION);
    }

    if configuration.elide_parameter_checks
        && matches!(
            instruction,
            Instruction::CallValue { .. }
                | Instruction::CallNamed { .. }
                | Instruction::CallMethod { .. }
        )
    {
        candidates.insert(CandidateSet::CALL);
    }

    if configuration.specialize_arrays
        && matches!(
            instruction,
            Instruction::Length { .. }
                | Instruction::IndexGet { .. }
                | Instruction::IndexSet { .. }
                | Instruction::IndexAddAssign { .. }
                | Instruction::Append { .. }
                | Instruction::DictIndexSet { .. }
                | Instruction::DictIndexGetIntKey { .. }
                | Instruction::DictIndexGetStringKey { .. }
                | Instruction::ForeachInit { .. }
                | Instruction::ForeachNext { .. }
        )
    {
        candidates.insert(CandidateSet::COLLECTION);
    }

    if configuration.specialize_arrays
        && matches!(
            instruction,
            Instruction::Length { .. }
                | Instruction::IndexGet { .. }
                | Instruction::IndexSet { .. }
                | Instruction::DictIndexSet { .. }
                | Instruction::Append { .. }
        )
    {
        candidates.insert(CandidateSet::EARLY_OPERATION);
    }

    if configuration.specialize_comparison
        && matches!(
            instruction,
            Instruction::Equal { .. }
                | Instruction::NotEqual { .. }
                | Instruction::LessThan { .. }
                | Instruction::LessThanOrEqual { .. }
                | Instruction::GreaterThan { .. }
                | Instruction::GreaterThanOrEqual { .. }
                | Instruction::JumpUnless { .. }
                | Instruction::BoolPatternBranch { .. }
                | Instruction::NumericLoop { .. }
                | Instruction::StringIndexGet { .. }
                | Instruction::StringJumpUnless { .. }
                | Instruction::Is { .. }
                | Instruction::SwitchPattern { .. }
                | Instruction::IntRangeJumpUnless { .. }
        )
    {
        candidates.insert(CandidateSet::COMPARISON);
    }

    if configuration.const_fold && constant_candidate(instruction) {
        candidates.insert(CandidateSet::CONSTANT);
    }

    if configuration.specialize_counter_loop
        && matches!(instruction, Instruction::CounterLoop { .. })
    {
        candidates.insert(CandidateSet::COUNTER_LOOP);
    }

    if configuration.dead_store && instruction_may_be_a_dead_store(instruction) {
        candidates.insert(CandidateSet::DEAD_STORE);
    }

    if configuration.elide_discarded_checks
        && matches!(
            instruction,
            Instruction::CallValueDiscarded { .. }
                | Instruction::CallNamedDiscarded { .. }
                | Instruction::CallMethodDiscarded { .. }
                | Instruction::CallStaticDiscarded { .. }
                | Instruction::CallWithNamesDiscarded { .. }
        )
    {
        candidates.insert(CandidateSet::DISCARDED_RESULT);
    }

    if configuration.ownership_moves
        && matches!(
            instruction,
            Instruction::Move { .. } | Instruction::PropertySetUnchecked { .. }
        )
    {
        candidates.insert(CandidateSet::OWNERSHIP);
    }

    if (configuration.elide_property_checks || configuration.specialize_property_get)
        && matches!(
            instruction,
            Instruction::PropertyGet { .. }
                | Instruction::PropertySet { .. }
                | Instruction::PropertyInitRaw { .. }
                | Instruction::PropertyIndexSet { .. }
                | Instruction::PropertyIndexUpdate { .. }
                | Instruction::PropertyRemove { .. }
                | Instruction::PropertyStep { .. }
                | Instruction::PropertyAdd { .. }
        )
    {
        candidates.insert(CandidateSet::PROPERTY);
    }

    if configuration.elide_type_checks
        && matches!(
            instruction,
            Instruction::CheckDestructure { .. }
                | Instruction::Return { .. }
                | Instruction::ReturnNull
        )
    {
        candidates.insert(CandidateSet::TYPE_CHECK);
    }

    candidates
}

fn constant_candidate(instruction: Instruction) -> bool {
    matches!(
        instruction,
        Instruction::Move { .. }
            | Instruction::MoveOwned { .. }
            | Instruction::NewVec { .. }
            | Instruction::NewDict { .. }
            | Instruction::NewTuple { .. }
            | Instruction::Add { .. }
            | Instruction::Subtract { .. }
            | Instruction::Multiply { .. }
            | Instruction::Divide { .. }
            | Instruction::Modulo { .. }
            | Instruction::Power { .. }
            | Instruction::Negate { .. }
            | Instruction::UnaryPlus { .. }
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
            | Instruction::Compare { .. }
            | Instruction::Not { .. }
            | Instruction::Concatenate { .. }
            | Instruction::ConcatenateRightConstant { .. }
            | Instruction::ConcatenateLeftConstant { .. }
            | Instruction::Length { .. }
            | Instruction::StringLength { .. }
            | Instruction::IndexGet { .. }
            | Instruction::ElementGet { .. }
            | Instruction::IntAdd { .. }
            | Instruction::IntSubtract { .. }
            | Instruction::IntMultiply { .. }
            | Instruction::IntModulo { .. }
            | Instruction::IntBitwiseAnd { .. }
            | Instruction::IntBitwiseOr { .. }
            | Instruction::IntBitwiseXor { .. }
            | Instruction::IntBitwiseNot { .. }
            | Instruction::IntShiftLeft { .. }
            | Instruction::IntShiftRight { .. }
            | Instruction::FloatAdd { .. }
            | Instruction::FloatSubtract { .. }
            | Instruction::FloatMultiply { .. }
            | Instruction::AddImmediate { .. }
            | Instruction::SubtractImmediate { .. }
            | Instruction::IntMultiplyImmediate { .. }
            | Instruction::IntModuloImmediate { .. }
    )
}

fn instruction_may_be_a_dead_store(instruction: Instruction) -> bool {
    matches!(
        instruction,
        Instruction::Move { .. }
            | Instruction::MoveOwned { .. }
            | Instruction::LoadConstant { .. }
            | Instruction::LoadNull { .. }
            | Instruction::LoadTrue { .. }
            | Instruction::LoadFalse { .. }
            | Instruction::LoadInt { .. }
            | Instruction::Clear { .. }
            | Instruction::VecIndexGet { .. }
            | Instruction::DictIndexGetIntKey { .. }
            | Instruction::DictIndexGetStringKey { .. }
    )
}

#[cfg(test)]
mod tests {
    use crate::bytecode::instruction::Instruction;
    use crate::bytecode::instruction::operands::Register;
    use crate::optimizer::OptimizationConfiguration;
    use crate::optimizer::candidates::CandidateSet;
    use crate::optimizer::candidates::instruction_candidates;

    #[test]
    fn arithmetic_candidates_request_only_their_needed_domains() {
        let candidates = instruction_candidates(
            Instruction::Add {
                destination: Register::new(2),
                left: Register::new(0),
                right: Register::new(1),
            },
            OptimizationConfiguration::default(),
        );

        assert!(candidates.contains(CandidateSet::ARITHMETIC));
        assert!(candidates.contains(CandidateSet::CONSTANT));
        assert!(candidates.needs_constant_cache());
        assert!(!candidates.needs_array_elements());
    }

    #[test]
    fn finished_null_returns_need_no_analysis() {
        let candidates = instruction_candidates(
            Instruction::ReturnNullUnchecked,
            OptimizationConfiguration::default(),
        );

        assert!(candidates.is_empty());
    }
}
