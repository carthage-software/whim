//! Cheap discovery of the analyses and rewrites a chunk can use.

use whim_bytecode::chunk::Chunk;
use whim_bytecode::instruction::Instruction;

use crate::OptimizationConfiguration;

/// The type-flow consumers that have at least one candidate in a chunk.
#[derive(Clone, Copy, Default, PartialEq, Eq)]
pub(crate) struct CandidateSet(u16);

impl CandidateSet {
    pub(crate) const ARITHMETIC: Self = Self(1 << 0);
    pub(crate) const CALL: Self = Self(1 << 1);
    pub(crate) const COLLECTION: Self = Self(1 << 2);
    pub(crate) const COMPARISON: Self = Self(1 << 3);
    pub(crate) const CONSTANT: Self = Self(1 << 4);
    pub(crate) const COUNTER_LOOP: Self = Self(1 << 5);
    pub(crate) const DEAD_STORE: Self = Self(1 << 6);
    pub(crate) const DISCARDED_RESULT: Self = Self(1 << 7);
    pub(crate) const OWNERSHIP: Self = Self(1 << 8);
    pub(crate) const PROPERTY: Self = Self(1 << 9);
    pub(crate) const TYPE_CHECK: Self = Self(1 << 10);
    pub(crate) const EARLY_OPERATION: Self = Self(1 << 11);

    pub(crate) fn of(chunk: &Chunk, configuration: OptimizationConfiguration) -> Self {
        let mut candidates = Self::default();
        for instruction in &chunk.code {
            candidates.insert(instruction_candidates(*instruction, configuration));
        }

        candidates
    }

    pub(crate) const fn contains(self, candidate: Self) -> bool {
        self.0 & candidate.0 != 0
    }

    pub(crate) const fn is_empty(self) -> bool {
        self.0 == 0
    }

    pub(crate) const fn needs_array_elements(self) -> bool {
        self.contains(Self::CALL)
            || self.contains(Self::COLLECTION)
            || self.contains(Self::PROPERTY)
            || self.contains(Self::TYPE_CHECK)
    }

    pub(crate) const fn needs_constant_cache(self) -> bool {
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
                | Instruction::IndexGetOrNull { .. }
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
            Instruction::Not { .. }
                | Instruction::Equal { .. }
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
            Instruction::PropertyGetOrNull { .. }
                | Instruction::PropertyGet { .. }
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
                | Instruction::JumpIfNull { .. }
                | Instruction::JumpIfNotNull { .. }
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
            | Instruction::JumpIfFalse { .. }
            | Instruction::JumpIfTrue { .. }
            | Instruction::JumpIfNull { .. }
            | Instruction::JumpIfNotNull { .. }
            | Instruction::JumpUnless { .. }
            | Instruction::IntJumpUnless { .. }
            | Instruction::StringJumpUnless { .. }
            | Instruction::JumpUnlessConstant { .. }
            | Instruction::IntJumpUnlessImmediate { .. }
            | Instruction::BoolPatternBranch { .. }
            | Instruction::SwitchInt { .. }
            | Instruction::SwitchString { .. }
            | Instruction::SwitchBool { .. }
            | Instruction::SwitchFloat { .. }
            | Instruction::SwitchPattern { .. }
            | Instruction::SwitchTuplePattern { .. }
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
    use whim_bytecode::instruction::Instruction;
    use whim_bytecode::instruction::operands::Register;

    use crate::OptimizationConfiguration;
    use crate::candidates::CandidateSet;
    use crate::candidates::instruction_candidates;

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
