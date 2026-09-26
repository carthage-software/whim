//! The 8-byte instruction and its operand encodings.
//!
//! The first byte is the tag. The rest holds little-endian operands without
//! padding. Register windows are contiguous, and jump offsets are relative to
//! the instruction that holds them. [`Register::NONE`] marks a missing operand.

use serde::Deserialize;
use serde::Serialize;

#[doc(hidden)]
pub const NUMERIC_LOOP_REGISTER_LIMIT: u16 = 64;

pub const MAIN_FRAME_REGISTER_HEADROOM: u16 = 24;

pub mod operands;

use crate::instruction::operands::ArrayKind;
use crate::instruction::operands::ArrayValueMode;
use crate::instruction::operands::AsMode;
use crate::instruction::operands::CallDescriptorIndex;
use crate::instruction::operands::Comparison;
use crate::instruction::operands::ConstantIndex;
use crate::instruction::operands::Count;
use crate::instruction::operands::DescriptorIndex;
use crate::instruction::operands::FloatPairUpdateDescriptorIndex;
use crate::instruction::operands::FloatSquaresSumBranchDescriptorIndex;
use crate::instruction::operands::IcSlot;
use crate::instruction::operands::ImmediateInt;
use crate::instruction::operands::ImmediateInteger;
use crate::instruction::operands::ImmediateUint;
use crate::instruction::operands::IndexAddMode;
use crate::instruction::operands::IntStepLoopDescriptorIndex;
use crate::instruction::operands::IntegerKind;
use crate::instruction::operands::JumpOffset;
use crate::instruction::operands::NearJumpOffset;
use crate::instruction::operands::PreparedIntLoopDescriptorIndex;
use crate::instruction::operands::PresetDescriptorIndex;
use crate::instruction::operands::PropertyIndexUpdateMode;
use crate::instruction::operands::PropertyInitializationDescriptorIndex;
use crate::instruction::operands::PropertyReadMode;
use crate::instruction::operands::PropertyRemoveMode;
use crate::instruction::operands::PropertySlot;
use crate::instruction::operands::PropertyStepMode;
use crate::instruction::operands::PropertyValueMode;
use crate::instruction::operands::Register;
use crate::instruction::operands::ShortJumpOffset;
use crate::instruction::operands::SwitchTableIndex;

macro_rules! instruction_set {
    ($declaration:ident) => {
        $declaration! {
            Move { destination: Register, source: Register } = 0,
            MoveOwned { destination: Register, source: Register } = 1,
            Clear { target: Register } = 2,
            LoadConstant { destination: Register, constant: ConstantIndex } = 3,
            LoadNull { destination: Register } = 4,
            LoadTrue { destination: Register } = 5,
            LoadFalse { destination: Register } = 6,
            LoadInteger { destination: Register, immediate: ImmediateInteger, kind: IntegerKind } = 7,
            ConstantGet { destination: Register, cache: IcSlot } = 8,
            ClassConstantGet { destination: Register, cache: IcSlot } = 9,
            Add { destination: Register, left: Register, right: Register, kind: Option<IntegerKind> } = 10,
            AddImmediate { destination: Register, source: Register, immediate: ImmediateInteger, kind: Option<IntegerKind> } = 11,
            IntegerAddAssign { target: Register, source: Register, kind: IntegerKind } = 12,
            Subtract { destination: Register, left: Register, right: Register, kind: Option<IntegerKind> } = 13,
            SubtractImmediate { destination: Register, source: Register, immediate: ImmediateInteger, kind: Option<IntegerKind> } = 14,
            Multiply { destination: Register, left: Register, right: Register, kind: Option<IntegerKind> } = 15,
            IntegerMultiplyImmediate { destination: Register, source: Register, immediate: ImmediateInteger, kind: IntegerKind } = 16,
            Divide { destination: Register, left: Register, right: Register } = 17,
            Modulo { destination: Register, left: Register, right: Register, kind: Option<IntegerKind> } = 18,
            IntegerModuloImmediate { destination: Register, source: Register, immediate: ImmediateInteger, kind: IntegerKind } = 19,
            Power { destination: Register, left: Register, right: Register } = 20,
            Negate { destination: Register, source: Register } = 21,
            UnaryPlus { destination: Register, source: Register } = 22,
            Step { destination: Register, source: Register, immediate: ImmediateInt, kind: Option<IntegerKind> } = 23,
            BitwiseAnd { destination: Register, left: Register, right: Register, kind: Option<IntegerKind> } = 24,
            BitwiseOr { destination: Register, left: Register, right: Register, kind: Option<IntegerKind> } = 25,
            BitwiseXor { destination: Register, left: Register, right: Register, kind: Option<IntegerKind> } = 26,
            BitwiseNot { destination: Register, source: Register, kind: Option<IntegerKind> } = 27,
            ShiftLeft { destination: Register, left: Register, right: Register, kind: Option<IntegerKind> } = 28,
            ShiftRight { destination: Register, left: Register, right: Register, kind: Option<IntegerKind> } = 29,
            FloatAdd { destination: Register, left: Register, right: Register } = 30,
            FloatSubtract { destination: Register, left: Register, right: Register } = 31,
            FloatMultiply { destination: Register, left: Register, right: Register } = 32,
            FloatMultiplyConstant { destination: Register, source: Register, constant: ConstantIndex } = 33,
            Squares { first_destination: Register, first_source: Register, second_source: Register } = 34,
            FloatSquares { first_destination: Register, first_source: Register, second_source: Register } = 35,
            FloatSquaresSum { first_destination: Register, first_source: Register, second_source: Register } = 36,
            FloatSquaresSumBranch { descriptor: FloatSquaresSumBranchDescriptorIndex, offset: JumpOffset } = 37,
            FloatDifferenceAdd { destination: Register, first_operand: Register, addend: Register } = 38,
            FloatScaleProductAdd { destination: Register, first_operand: Register, constant: ConstantIndex } = 39,
            FloatPairUpdate { descriptor: FloatPairUpdateDescriptorIndex } = 40,
            Equal { destination: Register, left: Register, right: Register } = 41,
            NotEqual { destination: Register, left: Register, right: Register } = 42,
            LessThan { destination: Register, left: Register, right: Register } = 43,
            LessThanOrEqual { destination: Register, left: Register, right: Register } = 44,
            GreaterThan { destination: Register, left: Register, right: Register } = 45,
            GreaterThanOrEqual { destination: Register, left: Register, right: Register } = 46,
            Compare { destination: Register, left: Register, right: Register } = 47,
            Not { destination: Register, source: Register } = 48,
            Concatenate { destination: Register, left: Register, right: Register } = 49,
            ConcatenateLeftConstant { destination: Register, source: Register, constant: ConstantIndex } = 50,
            ConcatenateRightConstant { destination: Register, source: Register, constant: ConstantIndex } = 51,
            StringLength { destination: Register, source: Register } = 52,
            StringIndexGet { destination: Register, container: Register, index: Register } = 53,
            StringIndexGetOrNull { destination: Register, container: Register, index: Register } = 54,
            StringIndexCoalesce { destination: Register, container: Register, index: Register, offset: NearJumpOffset } = 55,
            StringByteEqual { destination: Register, container: Register, index: Register, byte: u8 } = 56,
            StringByteNotEqual { destination: Register, container: Register, index: Register, byte: u8 } = 57,
            StringByteLessThan { destination: Register, container: Register, index: Register, byte: u8 } = 58,
            StringByteLessThanOrEqual { destination: Register, container: Register, index: Register, byte: u8 } = 59,
            StringByteGreaterThan { destination: Register, container: Register, index: Register, byte: u8 } = 60,
            StringByteGreaterThanOrEqual { destination: Register, container: Register, index: Register, byte: u8 } = 61,
            StringByteJumpUnlessEqual { container: Register, index: Register, byte: u8, offset: ShortJumpOffset } = 62,
            StringByteJumpUnlessNotEqual { container: Register, index: Register, byte: u8, offset: ShortJumpOffset } = 63,
            Jump { offset: JumpOffset } = 64,
            JumpIfFalse { condition: Register, offset: JumpOffset } = 65,
            JumpIfTrue { condition: Register, offset: JumpOffset } = 66,
            JumpIfNull { subject: Register, offset: JumpOffset } = 67,
            JumpIfNotNull { subject: Register, offset: JumpOffset } = 68,
            JumpUnless { comparison: Comparison, left: Register, right: Register, offset: ShortJumpOffset } = 69,
            IntJumpUnless { comparison: Comparison, left: Register, right: Register, offset: ShortJumpOffset } = 70,
            UintJumpUnless { comparison: Comparison, left: Register, right: Register, offset: ShortJumpOffset } = 71,
            StringJumpUnless { comparison: Comparison, left: Register, right: Register, offset: ShortJumpOffset } = 72,
            JumpUnlessConstant { comparison: Comparison, source: Register, constant: ConstantIndex, offset: ShortJumpOffset } = 73,
            IntJumpUnlessImmediate { comparison: Comparison, source: Register, immediate: ImmediateInt, offset: ShortJumpOffset } = 74,
            UintJumpUnlessImmediate { comparison: Comparison, source: Register, immediate: ImmediateUint, offset: ShortJumpOffset } = 75,
            Coalesce { destination: Register, source: Register, offset: ShortJumpOffset } = 76,
            FillDefault { target: Register, offset: JumpOffset } = 77,
            SwitchBool { subject: Register, table: SwitchTableIndex } = 78,
            SwitchInt { subject: Register, table: SwitchTableIndex } = 79,
            SwitchFloat { subject: Register, table: SwitchTableIndex } = 80,
            SwitchString { subject: Register, table: SwitchTableIndex } = 81,
            SwitchPattern { subject: Register, table: SwitchTableIndex } = 82,
            SwitchTuplePattern { first_element: Register, element_count: Count, table: SwitchTableIndex } = 83,
            BoolPatternBranch { subject: Register, false_offset: ShortJumpOffset, default_offset: ShortJumpOffset } = 84,
            IntegerRangeJumpIf { subject: Register, descriptor: DescriptorIndex, offset: ShortJumpOffset, kind: IntegerKind } = 85,
            IntegerRangeJumpUnless { subject: Register, descriptor: DescriptorIndex, offset: ShortJumpOffset, kind: IntegerKind } = 86,
            IncrementJump { target: Register, immediate: ImmediateInt, offset: ShortJumpOffset } = 87,
            CounterLoop { comparison: Comparison, counter: Register, limit: Register, offset: ShortJumpOffset } = 88,
            IntCounterLoop { comparison: Comparison, counter: Register, limit: Register, offset: ShortJumpOffset } = 89,
            UintCounterLoop { comparison: Comparison, counter: Register, limit: Register, offset: ShortJumpOffset } = 90,
            NumericLoop { comparison: Comparison, left: Register, right: Register, offset: ShortJumpOffset } = 91,
            IntNumericLoop { comparison: Comparison, left: Register, right: Register, offset: ShortJumpOffset } = 92,
            PreparedIntNumericLoop { descriptor: PreparedIntLoopDescriptorIndex, offset: ShortJumpOffset } = 93,
            IntStepLoop { descriptor: IntStepLoopDescriptorIndex, offset: ShortJumpOffset } = 94,
            NumericRegionJump { offset: JumpOffset } = 95,
            NewArray { count: Count, destination: Register, first_element: Register, kind: ArrayKind } = 96,
            NewFilledVec { destination: Register, value: Register, size: Register } = 97,
            ReserveArray { container: Register, additional: Register } = 98,
            Length { destination: Register, source: Register } = 99,
            ElementGet { destination: Register, subject: Register, index: ImmediateInt } = 100,
            IndexGet { destination: Register, container: Register, index: Register } = 101,
            VecIndexGet { destination: Register, container: Register, index: Register, value_mode: ArrayValueMode } = 102,
            DictIndexGetIntKey { destination: Register, container: Register, index: Register, value_mode: ArrayValueMode } = 103,
            DictIndexGetUintKey { destination: Register, container: Register, index: Register, value_mode: ArrayValueMode } = 104,
            DictIndexGetStringKey { destination: Register, container: Register, index: Register, value_mode: ArrayValueMode } = 105,
            IndexGetOrNull { destination: Register, container: Register, index: Register } = 106,
            VecIndexGetOrNull { destination: Register, container: Register, index: Register } = 107,
            DictIndexGetIntegerKeyOrNull { destination: Register, container: Register, index: Register, kind: IntegerKind } = 108,
            DictIndexGetStringKeyOrNull { destination: Register, container: Register, index: Register } = 109,
            IndexCoalesce { destination: Register, container: Register, index: Register, offset: NearJumpOffset } = 110,
            VecIndexCoalesce { destination: Register, container: Register, index: Register, offset: NearJumpOffset } = 111,
            DictIndexCoalesceIntKey { destination: Register, container: Register, index: Register, offset: NearJumpOffset } = 112,
            DictIndexCoalesceUintKey { destination: Register, container: Register, index: Register, offset: NearJumpOffset } = 113,
            DictIndexCoalesceStringKey { destination: Register, container: Register, index: Register, offset: NearJumpOffset } = 114,
            IndexSet { container: Register, index: Register, value: Register } = 115,
            VecIndexSet { container: Register, index: Register, value: Register } = 116,
            DictIndexSet { container: Register, index: Register, value: Register } = 117,
            DictIndexSetIntegerKey { container: Register, index: Register, value: Register, kind: IntegerKind } = 118,
            DictIndexSetStringKey { container: Register, index: Register, value: Register } = 119,
            IndexAddAssign { container: Register, index: Register, value: Register, mode: IndexAddMode } = 120,
            Append { container: Register, value: Register } = 121,
            VecAppend { container: Register, value: Register } = 122,
            Spread { container: Register, value: Register } = 123,
            Rest { destination: Register, subject: Register, from: ImmediateInt } = 124,
            Contains { destination: Register, array: Register, value: Register } = 125,
            ContainsKey { destination: Register, array: Register, key: Register } = 126,
            Remove { destination: Register, container: Register, key: Register } = 127,
            RemoveFirst { destination: Register, container: Register } = 128,
            RemoveLast { destination: Register, container: Register } = 129,
            SwapRemove { destination: Register, container: Register, index: Register } = 130,
            CheckDestructure { subject: Register, required: ImmediateInt, arity: ImmediateInt, rest: bool } = 131,
            ForeachInit { iterator: Register, subject: Register, reserve: Register } = 132,
            ForeachNext { iterator: Register, key_destination: Register, value_destination: Register } = 133,
            VecForeachNext { iterator: Register, key_destination: Register, value_destination: Register, value_mode: ArrayValueMode } = 134,
            DictForeachNext { iterator: Register, key_destination: Register, value_destination: Register, value_mode: ArrayValueMode } = 135,
            NewStatic { destination: Register, cache: IcSlot } = 136,
            NewDynamic { destination: Register, class_name: Register } = 137,
            NewTyped { destination: Register, descriptor: DescriptorIndex } = 138,
            CloneObject { destination: Register, source: Register } = 139,
            InitializeProperties { object: Register, cache: IcSlot, descriptor: PropertyInitializationDescriptorIndex } = 140,
            PropertyGet { destination: Register, object: Register, cache: IcSlot } = 141,
            PropertyGetUnchecked { destination: Register, object: Register, slot: PropertySlot, value_mode: PropertyReadMode } = 142,
            PropertyGetOrNull { destination: Register, object: Register, cache: IcSlot } = 143,
            PropertyGetOrNullUnchecked { destination: Register, object: Register, slot: PropertySlot } = 144,
            PropertyCoalesce { destination: Register, object: Register, cache: IcSlot, offset: NearJumpOffset } = 145,
            PropertyCoalesceUnchecked { destination: Register, object: Register, slot: PropertySlot, offset: NearJumpOffset } = 146,
            PropertySet { object: Register, value: Register, cache: IcSlot } = 147,
            PropertySetUnchecked { object: Register, value: Register, slot: PropertySlot, value_mode: PropertyValueMode } = 148,
            PropertyInitRaw { object: Register, value: Register, cache: IcSlot } = 149,
            PropertyIndexSet { object: Register, first_operand: Register, cache: IcSlot } = 150,
            PropertyIndexSetUnchecked { object: Register, first_operand: Register, slot: PropertySlot } = 151,
            PropertyIndexUpdate { object: Register, operand: Register, cache: IcSlot, mode: PropertyIndexUpdateMode } = 152,
            PropertyIndexUpdateUnchecked { object: Register, operand: Register, slot: PropertySlot, mode: PropertyIndexUpdateMode } = 153,
            PropertyStep { object: Register, cache: IcSlot, immediate: ImmediateInt, mode: PropertyStepMode } = 154,
            PropertyStepUnchecked { object: Register, slot: PropertySlot, immediate: ImmediateInt, mode: PropertyStepMode } = 155,
            PropertyAdd { object: Register, source: Register, cache: IcSlot } = 156,
            PropertyAddUnchecked { object: Register, source: Register, slot: PropertySlot } = 157,
            PropertyFillIntRange { object: Register, first_operand: Register, cache: IcSlot } = 158,
            PropertyRemove { object: Register, destination: Register, cache: IcSlot, mode: PropertyRemoveMode } = 159,
            PropertyRemoveUnchecked { object: Register, destination: Register, slot: PropertySlot, mode: PropertyRemoveMode } = 160,
            StaticPropertyGet { destination: Register, cache: IcSlot } = 161,
            StaticPropertyGetOrNull { destination: Register, cache: IcSlot } = 162,
            StaticPropertyCoalesce { destination: Register, cache: IcSlot, offset: ShortJumpOffset } = 163,
            StaticPropertySet { cache: IcSlot, value: Register } = 164,
            MakeClosure { capture_count: Count, destination: Register, prototype: ConstantIndex, first_capture: Register } = 165,
            MakeBound { destination: Register, callee: Register, descriptor: PresetDescriptorIndex } = 166,
            CallValue { argument_count: Count, destination: Register, callee: Register, first_argument: Register } = 167,
            CallValueUnchecked { argument_count: Count, destination: Register, callee: Register, first_argument: Register } = 168,
            CallValueDiscarded { argument_count: Count, destination: Register, callee: Register, first_argument: Register } = 169,
            CallNamed { argument_count: Count, destination: Register, first_argument: Register, cache: IcSlot } = 170,
            CallNamedUnchecked { argument_count: Count, destination: Register, first_argument: Register, cache: IcSlot } = 171,
            CallNamedDirect { argument_count: Count, destination: Register, first_argument: Register, cache: IcSlot } = 172,
            CallNamedConstantUnchecked { destination: Register, constant: ConstantIndex, cache: IcSlot, borrowed: bool } = 173,
            CallNamedDiscarded { argument_count: Count, destination: Register, first_argument: Register, cache: IcSlot } = 174,
            CallMethod { argument_count: Count, destination: Register, first_argument: Register, cache: IcSlot } = 175,
            CallMethodUnchecked { argument_count: Count, destination: Register, first_argument: Register, cache: IcSlot } = 176,
            CallMethodDirect { argument_count: Count, destination: Register, first_argument: Register, cache: IcSlot } = 177,
            CallMethodDiscarded { argument_count: Count, destination: Register, first_argument: Register, cache: IcSlot } = 178,
            CallStatic { argument_count: Count, destination: Register, first_argument: Register, cache: IcSlot } = 179,
            CallStaticDiscarded { argument_count: Count, destination: Register, first_argument: Register, cache: IcSlot } = 180,
            CallSelfUnchecked { argument_count: Count, destination: Register, first_argument: Register } = 181,
            CallWithNames { destination: Register, callee: Register, descriptor: CallDescriptorIndex } = 182,
            CallWithNamesDiscarded { destination: Register, callee: Register, descriptor: CallDescriptorIndex } = 183,
            CheckDiscardedResult { source: Register } = 184,
            Return { source: Register } = 185,
            ReturnUnchecked { source: Register } = 186,
            ReturnReferenceUnchecked { source: Register } = 187,
            ReturnScalarUnchecked { source: Register } = 188,
            ReturnIntegerUnchecked { immediate: ImmediateInteger, kind: IntegerKind } = 189,
            ReturnPairUnchecked { first: Register, second: Register } = 190,
            ReturnNull = 191,
            ReturnNullUnchecked = 192,
            Is { destination: Register, source: Register, descriptor: DescriptorIndex } = 193,
            AsCheck { destination: Register, source: Register, descriptor: DescriptorIndex, mode: AsMode } = 194,
            AsOrNull { destination: Register, source: Register, descriptor: DescriptorIndex } = 195,
            CheckDefined { subject: Register, name: ConstantIndex } = 196,
            CheckSoleReference { source: Register, message: ConstantIndex, chain_previous: bool } = 197,
            CheckWhereConstraints = 198,
            Throw { source: Register } = 199,
            Rethrow = 200,
            ThrowUnhandledMatch { subject: Register } = 201,
            Panic { message: Register } = 202,
            Assert { operand_count: Count, first_value: Register, message: Register, text: ConstantIndex } = 203,
            Exit { code: Register } = 204,
            Write { count: Count, register: Register, new_line: bool, stderr: bool } = 205,
            Debug { value_count: Count, first_value: Register } = 206,
            Require { once: bool, destination: Register, path: Register } = 207,
            DrainFinalizers = 208,
        }
    };
}

macro_rules! define_instruction {
    ($($variant:tt)*) => {
        #[expect(
            clippy::unsafe_derive_deserialize,
            reason = "verification precedes every unsafe instruction decode"
        )]
        #[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
        #[repr(u8)]
        pub enum Instruction {
            $($variant)*
        }
    };
}

/// An encoded operand whose value refers outside its instruction word.
#[derive(Clone, Copy)]
pub(crate) enum InstructionOperand {
    Register(Register),
    OptionalRegister(Register),
    Constant(ConstantIndex),
    FloatConstant(ConstantIndex),
    Cache(IcSlot),
    Jump(JumpOffset),
    RelativeTarget(ShortJumpOffset),
    SwitchTable(SwitchTableIndex),
    TypeDescriptor(DescriptorIndex),
    CallDescriptor(CallDescriptorIndex),
    PresetDescriptor(PresetDescriptorIndex),
    FloatPairUpdateDescriptor(FloatPairUpdateDescriptorIndex),
    FloatSquaresSumBranchDescriptor(FloatSquaresSumBranchDescriptorIndex),
    IntStepLoopDescriptor(IntStepLoopDescriptorIndex),
    PreparedIntLoopDescriptor(PreparedIntLoopDescriptorIndex),
    PropertyInitializationDescriptor(PropertyInitializationDescriptorIndex),
}

pub trait InstructionSideTableMapper {
    type Error;

    /// # Errors
    ///
    /// Returns an error if the constant index cannot be mapped.
    fn constant(&mut self, value: ConstantIndex) -> Result<ConstantIndex, Self::Error>;

    /// # Errors
    ///
    /// Returns an error if the cache slot cannot be mapped.
    fn cache(&mut self, value: IcSlot) -> Result<IcSlot, Self::Error>;

    /// # Errors
    ///
    /// Returns an error if the switch table index cannot be mapped.
    fn switch(&mut self, value: SwitchTableIndex) -> Result<SwitchTableIndex, Self::Error>;

    /// # Errors
    ///
    /// Returns an error if the type descriptor index cannot be mapped.
    fn descriptor(&mut self, value: DescriptorIndex) -> Result<DescriptorIndex, Self::Error>;

    /// # Errors
    ///
    /// Returns an error if the call descriptor index cannot be mapped.
    fn call(&mut self, value: CallDescriptorIndex) -> Result<CallDescriptorIndex, Self::Error>;

    /// # Errors
    ///
    /// Returns an error if the preset descriptor index cannot be mapped.
    fn preset(
        &mut self,
        value: PresetDescriptorIndex,
    ) -> Result<PresetDescriptorIndex, Self::Error>;
    /// # Errors
    ///
    /// Returns an error if the float pair update index cannot be mapped.
    fn float_pair_update(
        &mut self,
        value: FloatPairUpdateDescriptorIndex,
    ) -> Result<FloatPairUpdateDescriptorIndex, Self::Error>;
    /// # Errors
    ///
    /// Returns an error if the float squares sum branch index cannot be mapped.
    fn float_squares_sum_branch(
        &mut self,
        value: FloatSquaresSumBranchDescriptorIndex,
    ) -> Result<FloatSquaresSumBranchDescriptorIndex, Self::Error>;
    /// # Errors
    ///
    /// Returns an error if the integer step loop index cannot be mapped.
    fn int_step_loop(
        &mut self,
        value: IntStepLoopDescriptorIndex,
    ) -> Result<IntStepLoopDescriptorIndex, Self::Error>;
    /// # Errors
    ///
    /// Returns an error if the prepared integer loop index cannot be mapped.
    fn prepared_int_loop(
        &mut self,
        value: PreparedIntLoopDescriptorIndex,
    ) -> Result<PreparedIntLoopDescriptorIndex, Self::Error>;
    /// # Errors
    ///
    /// Returns an error if the property initialization index cannot be mapped.
    fn property_initialization(
        &mut self,
        value: PropertyInitializationDescriptorIndex,
    ) -> Result<PropertyInitializationDescriptorIndex, Self::Error>;
}

macro_rules! visit_instruction_operand {
    ($visit:ident, Write, register, Register, $value:expr) => {
        let _ = $value;
    };
    ($visit:ident, $variant:ident, first_argument, Register, $value:expr) => {
        let _ = $value;
    };
    ($visit:ident, $variant:ident, first_operand, Register, $value:expr) => {
        let _ = $value;
    };
    ($visit:ident, $variant:ident, first_destination, Register, $value:expr) => {
        let _ = $value;
    };
    ($visit:ident, $variant:ident, first_element, Register, $value:expr) => {
        let _ = $value;
    };
    ($visit:ident, $variant:ident, first_pair, Register, $value:expr) => {
        let _ = $value;
    };
    ($visit:ident, $variant:ident, first_capture, Register, $value:expr) => {
        let _ = $value;
    };
    ($visit:ident, $variant:ident, first_value, Register, $value:expr) => {
        let _ = $value;
    };
    ($visit:ident, Assert, message, Register, $value:expr) => {
        $visit(InstructionOperand::OptionalRegister($value))?;
    };
    ($visit:ident, Exit, code, Register, $value:expr) => {
        $visit(InstructionOperand::OptionalRegister($value))?;
    };
    ($visit:ident, ForeachInit, reserve, Register, $value:expr) => {
        $visit(InstructionOperand::OptionalRegister($value))?;
    };
    ($visit:ident, ForeachNext, key_destination, Register, $value:expr) => {
        $visit(InstructionOperand::OptionalRegister($value))?;
    };
    ($visit:ident, VecForeachNext, key_destination, Register, $value:expr) => {
        $visit(InstructionOperand::OptionalRegister($value))?;
    };
    ($visit:ident, DictForeachNext, key_destination, Register, $value:expr) => {
        $visit(InstructionOperand::OptionalRegister($value))?;
    };
    ($visit:ident, $variant:ident, $field:ident, Register, $value:expr) => {
        $visit(InstructionOperand::Register($value))?;
    };
    ($visit:ident, FloatMultiplyConstant, constant, ConstantIndex, $value:expr) => {
        $visit(InstructionOperand::FloatConstant($value))?;
    };
    ($visit:ident, FloatScaleProductAdd, constant, ConstantIndex, $value:expr) => {
        $visit(InstructionOperand::FloatConstant($value))?;
    };
    ($visit:ident, $variant:ident, $field:ident, ConstantIndex, $value:expr) => {
        $visit(InstructionOperand::Constant($value))?;
    };
    ($visit:ident, $variant:ident, $field:ident, IcSlot, $value:expr) => {
        $visit(InstructionOperand::Cache($value))?;
    };
    ($visit:ident, $variant:ident, $field:ident, JumpOffset, $value:expr) => {
        $visit(InstructionOperand::Jump($value))?;
    };
    ($visit:ident, $variant:ident, $field:ident, ShortJumpOffset, $value:expr) => {
        $visit(InstructionOperand::RelativeTarget($value))?;
    };
    ($visit:ident, $variant:ident, $field:ident, NearJumpOffset, $value:expr) => {
        $visit(InstructionOperand::Jump(JumpOffset::new(i32::from(
            $value.offset(),
        ))))?;
    };
    ($visit:ident, $variant:ident, $field:ident, SwitchTableIndex, $value:expr) => {
        $visit(InstructionOperand::SwitchTable($value))?;
    };
    ($visit:ident, $variant:ident, $field:ident, DescriptorIndex, $value:expr) => {
        $visit(InstructionOperand::TypeDescriptor($value))?;
    };
    ($visit:ident, $variant:ident, $field:ident, CallDescriptorIndex, $value:expr) => {
        $visit(InstructionOperand::CallDescriptor($value))?;
    };
    ($visit:ident, $variant:ident, $field:ident, PresetDescriptorIndex, $value:expr) => {
        $visit(InstructionOperand::PresetDescriptor($value))?;
    };
    ($visit:ident, $variant:ident, $field:ident, FloatPairUpdateDescriptorIndex, $value:expr) => {
        $visit(InstructionOperand::FloatPairUpdateDescriptor($value))?;
    };
    ($visit:ident, $variant:ident, $field:ident, FloatSquaresSumBranchDescriptorIndex, $value:expr) => {
        $visit(InstructionOperand::FloatSquaresSumBranchDescriptor($value))?;
    };
    ($visit:ident, $variant:ident, $field:ident, IntStepLoopDescriptorIndex, $value:expr) => {
        $visit(InstructionOperand::IntStepLoopDescriptor($value))?;
    };
    ($visit:ident, $variant:ident, $field:ident, PreparedIntLoopDescriptorIndex, $value:expr) => {
        $visit(InstructionOperand::PreparedIntLoopDescriptor($value))?;
    };
    ($visit:ident, $variant:ident, $field:ident, PropertyInitializationDescriptorIndex, $value:expr) => {
        $visit(InstructionOperand::PropertyInitializationDescriptor($value))?;
    };
    ($visit:ident, $variant:ident, $field:ident, $type:ty, $value:expr) => {
        let _ = $value;
    };
}

macro_rules! define_operand_visit {
    ($($(#[$attribute:meta])* $name:ident $({$($(#[$field_attribute:meta])* $field:ident: $type:ident $(<$argument:ty>)?),* $(,)?})? = $tag:literal,)*) => {
        impl Instruction {
            /// Visits every operand that refers to a register or side table.
            pub(crate) fn try_visit_operands<E>(
                self,
                mut visit: impl FnMut(InstructionOperand) -> Result<(), E>,
            ) -> Result<(), E> {
                match self {
                    $(
                        Instruction::$name $({ $($field),* })? => {
                            $($(visit_instruction_operand!(visit, $name, $field, $type $(<$argument>)?, $field);)*)?
                        }
                    )*
                }
                Ok(())
            }
        }
    };
}

macro_rules! map_instruction_side_table {
    ($mapper:ident, $field:ident, ConstantIndex) => {
        *$field = $mapper.constant(*$field)?;
    };
    ($mapper:ident, $field:ident, IcSlot) => {
        *$field = $mapper.cache(*$field)?;
    };
    ($mapper:ident, $field:ident, SwitchTableIndex) => {
        *$field = $mapper.switch(*$field)?;
    };
    ($mapper:ident, $field:ident, DescriptorIndex) => {
        *$field = $mapper.descriptor(*$field)?;
    };
    ($mapper:ident, $field:ident, CallDescriptorIndex) => {
        *$field = $mapper.call(*$field)?;
    };
    ($mapper:ident, $field:ident, PresetDescriptorIndex) => {
        *$field = $mapper.preset(*$field)?;
    };
    ($mapper:ident, $field:ident, FloatPairUpdateDescriptorIndex) => {
        *$field = $mapper.float_pair_update(*$field)?;
    };
    ($mapper:ident, $field:ident, FloatSquaresSumBranchDescriptorIndex) => {
        *$field = $mapper.float_squares_sum_branch(*$field)?;
    };
    ($mapper:ident, $field:ident, IntStepLoopDescriptorIndex) => {
        *$field = $mapper.int_step_loop(*$field)?;
    };
    ($mapper:ident, $field:ident, PreparedIntLoopDescriptorIndex) => {
        *$field = $mapper.prepared_int_loop(*$field)?;
    };
    ($mapper:ident, $field:ident, PropertyInitializationDescriptorIndex) => {
        *$field = $mapper.property_initialization(*$field)?;
    };
    ($mapper:ident, $field:ident, $type:ty) => {
        let _ = $field;
    };
}

macro_rules! define_side_table_map {
    ($($(#[$attribute:meta])* $name:ident $({$($(#[$field_attribute:meta])* $field:ident: $type:ident $(<$argument:ty>)?),* $(,)?})? = $tag:literal,)*) => {
        impl Instruction {
            /// # Errors
            ///
            /// Returns the first mapper error. Earlier operands remain mapped.
            pub fn try_map_side_tables<M: InstructionSideTableMapper>(
                &mut self,
                mapper: &mut M,
            ) -> Result<(), M::Error> {
                match self {
                    $(
                        Instruction::$name $({ $($field),* })? => {
                            $($(map_instruction_side_table!(mapper, $field, $type $(<$argument>)?);)*)?
                        }
                    )*
                }
                Ok(())
            }
        }
    };
}

pub mod word;

instruction_set!(define_instruction);
instruction_set!(define_operand_visit);
instruction_set!(define_side_table_map);

const _: () = assert!(size_of::<Instruction>() == 8);
const _: () = assert!(align_of::<Instruction>() == 1);
