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
            Add { destination: Register, left: Register, right: Register } = 10,
            IntegerAdd { destination: Register, left: Register, right: Register, kind: IntegerKind } = 11,
            AddImmediate { destination: Register, source: Register, immediate: ImmediateInt } = 12,
            IntegerAddImmediate { destination: Register, source: Register, immediate: ImmediateInteger, kind: IntegerKind } = 13,
            IntegerAddAssign { target: Register, source: Register, kind: IntegerKind } = 14,
            Subtract { destination: Register, left: Register, right: Register } = 15,
            IntegerSubtract { destination: Register, left: Register, right: Register, kind: IntegerKind } = 16,
            SubtractImmediate { destination: Register, source: Register, immediate: ImmediateInt } = 17,
            IntegerSubtractImmediate { destination: Register, source: Register, immediate: ImmediateInteger, kind: IntegerKind } = 18,
            Multiply { destination: Register, left: Register, right: Register } = 19,
            IntegerMultiply { destination: Register, left: Register, right: Register, kind: IntegerKind } = 20,
            IntegerMultiplyImmediate { destination: Register, source: Register, immediate: ImmediateInteger, kind: IntegerKind } = 21,
            Divide { destination: Register, left: Register, right: Register } = 22,
            Modulo { destination: Register, left: Register, right: Register } = 23,
            IntegerModulo { destination: Register, left: Register, right: Register, kind: IntegerKind } = 24,
            IntegerModuloImmediate { destination: Register, source: Register, immediate: ImmediateInteger, kind: IntegerKind } = 25,
            Power { destination: Register, left: Register, right: Register } = 26,
            Negate { destination: Register, source: Register } = 27,
            UnaryPlus { destination: Register, source: Register } = 28,
            Step { destination: Register, source: Register, immediate: ImmediateInt } = 29,
            IntegerStep { destination: Register, source: Register, immediate: ImmediateInt, kind: IntegerKind } = 30,
            BitwiseAnd { destination: Register, left: Register, right: Register } = 31,
            IntegerBitwiseAnd { destination: Register, left: Register, right: Register, kind: IntegerKind } = 32,
            BitwiseOr { destination: Register, left: Register, right: Register } = 33,
            IntegerBitwiseOr { destination: Register, left: Register, right: Register, kind: IntegerKind } = 34,
            BitwiseXor { destination: Register, left: Register, right: Register } = 35,
            IntegerBitwiseXor { destination: Register, left: Register, right: Register, kind: IntegerKind } = 36,
            BitwiseNot { destination: Register, source: Register } = 37,
            IntegerBitwiseNot { destination: Register, source: Register, kind: IntegerKind } = 38,
            ShiftLeft { destination: Register, left: Register, right: Register } = 39,
            IntegerShiftLeft { destination: Register, left: Register, right: Register, kind: IntegerKind } = 40,
            ShiftRight { destination: Register, left: Register, right: Register } = 41,
            IntegerShiftRight { destination: Register, left: Register, right: Register, kind: IntegerKind } = 42,
            FloatAdd { destination: Register, left: Register, right: Register } = 43,
            FloatSubtract { destination: Register, left: Register, right: Register } = 44,
            FloatMultiply { destination: Register, left: Register, right: Register } = 45,
            FloatMultiplyConstant { destination: Register, source: Register, constant: ConstantIndex } = 46,
            Squares { first_destination: Register, first_source: Register, second_source: Register } = 47,
            FloatSquares { first_destination: Register, first_source: Register, second_source: Register } = 48,
            FloatSquaresSum { first_destination: Register, first_source: Register, second_source: Register } = 49,
            FloatSquaresSumBranch { descriptor: FloatSquaresSumBranchDescriptorIndex, offset: JumpOffset } = 50,
            FloatDifferenceAdd { destination: Register, first_operand: Register, addend: Register } = 51,
            FloatScaleProductAdd { destination: Register, first_operand: Register, constant: ConstantIndex } = 52,
            FloatPairUpdate { descriptor: FloatPairUpdateDescriptorIndex } = 53,
            Equal { destination: Register, left: Register, right: Register } = 54,
            NotEqual { destination: Register, left: Register, right: Register } = 55,
            LessThan { destination: Register, left: Register, right: Register } = 56,
            LessThanOrEqual { destination: Register, left: Register, right: Register } = 57,
            GreaterThan { destination: Register, left: Register, right: Register } = 58,
            GreaterThanOrEqual { destination: Register, left: Register, right: Register } = 59,
            Compare { destination: Register, left: Register, right: Register } = 60,
            Not { destination: Register, source: Register } = 61,
            Concatenate { destination: Register, left: Register, right: Register } = 62,
            ConcatenateLeftConstant { destination: Register, source: Register, constant: ConstantIndex } = 63,
            ConcatenateRightConstant { destination: Register, source: Register, constant: ConstantIndex } = 64,
            StringLength { destination: Register, source: Register } = 65,
            StringIndexGet { destination: Register, container: Register, index: Register } = 66,
            StringIndexGetOrNull { destination: Register, container: Register, index: Register } = 67,
            StringIndexCoalesce { destination: Register, container: Register, index: Register, offset: NearJumpOffset } = 68,
            StringByteEqual { destination: Register, container: Register, index: Register, byte: u8 } = 69,
            StringByteNotEqual { destination: Register, container: Register, index: Register, byte: u8 } = 70,
            StringByteLessThan { destination: Register, container: Register, index: Register, byte: u8 } = 71,
            StringByteLessThanOrEqual { destination: Register, container: Register, index: Register, byte: u8 } = 72,
            StringByteGreaterThan { destination: Register, container: Register, index: Register, byte: u8 } = 73,
            StringByteGreaterThanOrEqual { destination: Register, container: Register, index: Register, byte: u8 } = 74,
            StringByteJumpUnlessEqual { container: Register, index: Register, byte: u8, offset: ShortJumpOffset } = 75,
            StringByteJumpUnlessNotEqual { container: Register, index: Register, byte: u8, offset: ShortJumpOffset } = 76,
            Jump { offset: JumpOffset } = 77,
            JumpIfFalse { condition: Register, offset: JumpOffset } = 78,
            JumpIfTrue { condition: Register, offset: JumpOffset } = 79,
            JumpIfNull { subject: Register, offset: JumpOffset } = 80,
            JumpIfNotNull { subject: Register, offset: JumpOffset } = 81,
            JumpUnless { comparison: Comparison, left: Register, right: Register, offset: ShortJumpOffset } = 82,
            IntJumpUnless { comparison: Comparison, left: Register, right: Register, offset: ShortJumpOffset } = 83,
            UintJumpUnless { comparison: Comparison, left: Register, right: Register, offset: ShortJumpOffset } = 84,
            StringJumpUnless { comparison: Comparison, left: Register, right: Register, offset: ShortJumpOffset } = 85,
            JumpUnlessConstant { comparison: Comparison, source: Register, constant: ConstantIndex, offset: ShortJumpOffset } = 86,
            IntJumpUnlessImmediate { comparison: Comparison, source: Register, immediate: ImmediateInt, offset: ShortJumpOffset } = 87,
            UintJumpUnlessImmediate { comparison: Comparison, source: Register, immediate: ImmediateUint, offset: ShortJumpOffset } = 88,
            Coalesce { destination: Register, source: Register, offset: ShortJumpOffset } = 89,
            FillDefault { target: Register, offset: JumpOffset } = 90,
            SwitchBool { subject: Register, table: SwitchTableIndex } = 91,
            SwitchInt { subject: Register, table: SwitchTableIndex } = 92,
            SwitchFloat { subject: Register, table: SwitchTableIndex } = 93,
            SwitchString { subject: Register, table: SwitchTableIndex } = 94,
            SwitchPattern { subject: Register, table: SwitchTableIndex } = 95,
            SwitchTuplePattern { first_element: Register, element_count: Count, table: SwitchTableIndex } = 96,
            BoolPatternBranch { subject: Register, false_offset: ShortJumpOffset, default_offset: ShortJumpOffset } = 97,
            IntegerRangeJumpIf { subject: Register, descriptor: DescriptorIndex, offset: ShortJumpOffset, kind: IntegerKind } = 98,
            IntegerRangeJumpUnless { subject: Register, descriptor: DescriptorIndex, offset: ShortJumpOffset, kind: IntegerKind } = 99,
            IncrementJump { target: Register, immediate: ImmediateInt, offset: ShortJumpOffset } = 100,
            CounterLoop { comparison: Comparison, counter: Register, limit: Register, offset: ShortJumpOffset } = 101,
            IntCounterLoop { comparison: Comparison, counter: Register, limit: Register, offset: ShortJumpOffset } = 102,
            UintCounterLoop { comparison: Comparison, counter: Register, limit: Register, offset: ShortJumpOffset } = 103,
            NumericLoop { comparison: Comparison, left: Register, right: Register, offset: ShortJumpOffset } = 104,
            IntNumericLoop { comparison: Comparison, left: Register, right: Register, offset: ShortJumpOffset } = 105,
            PreparedIntNumericLoop { descriptor: PreparedIntLoopDescriptorIndex, offset: ShortJumpOffset } = 106,
            IntStepLoop { descriptor: IntStepLoopDescriptorIndex, offset: ShortJumpOffset } = 107,
            NumericRegionJump { offset: JumpOffset } = 108,
            NewVec { element_count: Count, destination: Register, first_element: Register } = 109,
            NewDict { pair_count: Count, destination: Register, first_pair: Register } = 110,
            NewTuple { element_count: Count, destination: Register, first_element: Register } = 111,
            NewFilledVec { destination: Register, value: Register, size: Register } = 112,
            ReserveArray { container: Register, additional: Register } = 113,
            Length { destination: Register, source: Register } = 114,
            ElementGet { destination: Register, subject: Register, index: ImmediateInt } = 115,
            IndexGet { destination: Register, container: Register, index: Register } = 116,
            VecIndexGet { destination: Register, container: Register, index: Register, value_mode: ArrayValueMode } = 117,
            DictIndexGetIntKey { destination: Register, container: Register, index: Register, value_mode: ArrayValueMode } = 118,
            DictIndexGetUintKey { destination: Register, container: Register, index: Register, value_mode: ArrayValueMode } = 119,
            DictIndexGetStringKey { destination: Register, container: Register, index: Register, value_mode: ArrayValueMode } = 120,
            IndexGetOrNull { destination: Register, container: Register, index: Register } = 121,
            VecIndexGetOrNull { destination: Register, container: Register, index: Register } = 122,
            DictIndexGetIntegerKeyOrNull { destination: Register, container: Register, index: Register, kind: IntegerKind } = 123,
            DictIndexGetStringKeyOrNull { destination: Register, container: Register, index: Register } = 124,
            IndexCoalesce { destination: Register, container: Register, index: Register, offset: NearJumpOffset } = 125,
            VecIndexCoalesce { destination: Register, container: Register, index: Register, offset: NearJumpOffset } = 126,
            DictIndexCoalesceIntKey { destination: Register, container: Register, index: Register, offset: NearJumpOffset } = 127,
            DictIndexCoalesceUintKey { destination: Register, container: Register, index: Register, offset: NearJumpOffset } = 128,
            DictIndexCoalesceStringKey { destination: Register, container: Register, index: Register, offset: NearJumpOffset } = 129,
            IndexSet { container: Register, index: Register, value: Register } = 130,
            VecIndexSet { container: Register, index: Register, value: Register } = 131,
            DictIndexSet { container: Register, index: Register, value: Register } = 132,
            DictIndexSetIntegerKey { container: Register, index: Register, value: Register, kind: IntegerKind } = 133,
            DictIndexSetStringKey { container: Register, index: Register, value: Register } = 134,
            IndexAddAssign { container: Register, index: Register, value: Register, mode: IndexAddMode } = 135,
            Append { container: Register, value: Register } = 136,
            VecAppend { container: Register, value: Register } = 137,
            Spread { container: Register, value: Register } = 138,
            Rest { destination: Register, subject: Register, from: ImmediateInt } = 139,
            Contains { destination: Register, array: Register, value: Register } = 140,
            ContainsKey { destination: Register, array: Register, key: Register } = 141,
            Remove { destination: Register, container: Register, key: Register } = 142,
            RemoveFirst { destination: Register, container: Register } = 143,
            RemoveLast { destination: Register, container: Register } = 144,
            SwapRemove { destination: Register, container: Register, index: Register } = 145,
            CheckDestructure { subject: Register, required: ImmediateInt, arity: ImmediateInt, rest: bool } = 146,
            ForeachInit { iterator: Register, subject: Register, reserve: Register } = 147,
            ForeachNext { iterator: Register, key_destination: Register, value_destination: Register } = 148,
            VecForeachNext { iterator: Register, key_destination: Register, value_destination: Register, value_mode: ArrayValueMode } = 149,
            DictForeachNext { iterator: Register, key_destination: Register, value_destination: Register, value_mode: ArrayValueMode } = 150,
            NewStatic { destination: Register, cache: IcSlot } = 151,
            NewDynamic { destination: Register, class_name: Register } = 152,
            NewTyped { destination: Register, descriptor: DescriptorIndex } = 153,
            CloneObject { destination: Register, source: Register } = 154,
            InitializeProperties { object: Register, cache: IcSlot, descriptor: PropertyInitializationDescriptorIndex } = 155,
            PropertyGet { destination: Register, object: Register, cache: IcSlot } = 156,
            PropertyGetUnchecked { destination: Register, object: Register, slot: PropertySlot, value_mode: PropertyReadMode } = 157,
            PropertyGetOrNull { destination: Register, object: Register, cache: IcSlot } = 158,
            PropertyGetOrNullUnchecked { destination: Register, object: Register, slot: PropertySlot } = 159,
            PropertyCoalesce { destination: Register, object: Register, cache: IcSlot, offset: NearJumpOffset } = 160,
            PropertyCoalesceUnchecked { destination: Register, object: Register, slot: PropertySlot, offset: NearJumpOffset } = 161,
            PropertySet { object: Register, value: Register, cache: IcSlot } = 162,
            PropertySetUnchecked { object: Register, value: Register, slot: PropertySlot, value_mode: PropertyValueMode } = 163,
            PropertyInitRaw { object: Register, value: Register, cache: IcSlot } = 164,
            PropertyIndexSet { object: Register, first_operand: Register, cache: IcSlot } = 165,
            PropertyIndexSetUnchecked { object: Register, first_operand: Register, slot: PropertySlot } = 166,
            PropertyIndexUpdate { object: Register, operand: Register, cache: IcSlot, mode: PropertyIndexUpdateMode } = 167,
            PropertyIndexUpdateUnchecked { object: Register, operand: Register, slot: PropertySlot, mode: PropertyIndexUpdateMode } = 168,
            PropertyStep { object: Register, cache: IcSlot, immediate: ImmediateInt, mode: PropertyStepMode } = 169,
            PropertyStepUnchecked { object: Register, slot: PropertySlot, immediate: ImmediateInt, mode: PropertyStepMode } = 170,
            PropertyAdd { object: Register, source: Register, cache: IcSlot } = 171,
            PropertyAddUnchecked { object: Register, source: Register, slot: PropertySlot } = 172,
            PropertyFillIntRange { object: Register, first_operand: Register, cache: IcSlot } = 173,
            PropertyRemove { object: Register, destination: Register, cache: IcSlot, mode: PropertyRemoveMode } = 174,
            PropertyRemoveUnchecked { object: Register, destination: Register, slot: PropertySlot, mode: PropertyRemoveMode } = 175,
            StaticPropertyGet { destination: Register, cache: IcSlot } = 176,
            StaticPropertyGetOrNull { destination: Register, cache: IcSlot } = 177,
            StaticPropertyCoalesce { destination: Register, cache: IcSlot, offset: ShortJumpOffset } = 178,
            StaticPropertySet { cache: IcSlot, value: Register } = 179,
            MakeClosure { capture_count: Count, destination: Register, prototype: ConstantIndex, first_capture: Register } = 180,
            MakeBound { destination: Register, callee: Register, descriptor: PresetDescriptorIndex } = 181,
            CallValue { argument_count: Count, destination: Register, callee: Register, first_argument: Register } = 182,
            CallValueUnchecked { argument_count: Count, destination: Register, callee: Register, first_argument: Register } = 183,
            CallValueDiscarded { argument_count: Count, destination: Register, callee: Register, first_argument: Register } = 184,
            CallNamed { argument_count: Count, destination: Register, first_argument: Register, cache: IcSlot } = 185,
            CallNamedUnchecked { argument_count: Count, destination: Register, first_argument: Register, cache: IcSlot } = 186,
            CallNamedDirect { argument_count: Count, destination: Register, first_argument: Register, cache: IcSlot } = 187,
            CallNamedConstantUnchecked { destination: Register, constant: ConstantIndex, cache: IcSlot, borrowed: bool } = 188,
            CallNamedDiscarded { argument_count: Count, destination: Register, first_argument: Register, cache: IcSlot } = 189,
            CallMethod { argument_count: Count, destination: Register, first_argument: Register, cache: IcSlot } = 190,
            CallMethodUnchecked { argument_count: Count, destination: Register, first_argument: Register, cache: IcSlot } = 191,
            CallMethodDirect { argument_count: Count, destination: Register, first_argument: Register, cache: IcSlot } = 192,
            CallMethodDiscarded { argument_count: Count, destination: Register, first_argument: Register, cache: IcSlot } = 193,
            CallStatic { argument_count: Count, destination: Register, first_argument: Register, cache: IcSlot } = 194,
            CallStaticDiscarded { argument_count: Count, destination: Register, first_argument: Register, cache: IcSlot } = 195,
            CallSelfUnchecked { argument_count: Count, destination: Register, first_argument: Register } = 196,
            CallWithNames { destination: Register, callee: Register, descriptor: CallDescriptorIndex } = 197,
            CallWithNamesDiscarded { destination: Register, callee: Register, descriptor: CallDescriptorIndex } = 198,
            CheckDiscardedResult { source: Register } = 199,
            Return { source: Register } = 200,
            ReturnUnchecked { source: Register } = 201,
            ReturnReferenceUnchecked { source: Register } = 202,
            ReturnScalarUnchecked { source: Register } = 203,
            ReturnIntegerUnchecked { immediate: ImmediateInteger, kind: IntegerKind } = 204,
            ReturnPairUnchecked { first: Register, second: Register } = 205,
            ReturnNull = 206,
            ReturnNullUnchecked = 207,
            Is { destination: Register, source: Register, descriptor: DescriptorIndex } = 208,
            AsCheck { destination: Register, source: Register, descriptor: DescriptorIndex, mode: AsMode } = 209,
            AsOrNull { destination: Register, source: Register, descriptor: DescriptorIndex } = 210,
            CheckDefined { subject: Register, name: ConstantIndex } = 211,
            CheckSoleReference { source: Register, message: ConstantIndex, chain_previous: bool } = 212,
            CheckWhereConstraints = 213,
            Throw { source: Register } = 214,
            Rethrow = 215,
            ThrowUnhandledMatch { subject: Register } = 216,
            Panic { message: Register } = 217,
            Assert { operand_count: Count, first_value: Register, message: Register, text: ConstantIndex } = 218,
            Exit { code: Register } = 219,
            Write { value_count: Count, first_value: Register } = 220,
            WriteLine { value_count: Count, first_value: Register } = 221,
            WriteError { value_count: Count, first_value: Register } = 222,
            WriteErrorLine { value_count: Count, first_value: Register } = 223,
            Debug { value_count: Count, first_value: Register } = 224,
            Require { once: bool, destination: Register, path: Register } = 225,
            DrainFinalizers = 226,
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
    ($visit:ident, $variant:ident, $field:ident, $type:ident, $value:expr) => {
        let _ = $value;
    };
}

macro_rules! define_operand_visit {
    ($($(#[$attribute:meta])* $name:ident $({$($(#[$field_attribute:meta])* $field:ident: $type:ident),* $(,)?})? = $tag:literal,)*) => {
        impl Instruction {
            /// Visits every operand that refers to a register or side table.
            pub(crate) fn try_visit_operands<E>(
                self,
                mut visit: impl FnMut(InstructionOperand) -> Result<(), E>,
            ) -> Result<(), E> {
                match self {
                    $(
                        Instruction::$name $({ $($field),* })? => {
                            $($(visit_instruction_operand!(visit, $name, $field, $type, $field);)*)?
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
    ($mapper:ident, $field:ident, $type:ident) => {
        let _ = $field;
    };
}

macro_rules! define_side_table_map {
    ($($(#[$attribute:meta])* $name:ident $({$($(#[$field_attribute:meta])* $field:ident: $type:ident),* $(,)?})? = $tag:literal,)*) => {
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
                            $($(map_instruction_side_table!(mapper, $field, $type);)*)?
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
