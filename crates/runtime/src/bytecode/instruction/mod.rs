//! The 8-byte instruction and its operand encodings.
//!
//! The first byte is the tag. The rest holds little-endian operands without
//! padding. Register windows are contiguous, and jump offsets are relative to
//! the instruction that holds them. [`Register::NONE`] marks a missing operand.

use serde::Deserialize;
use serde::Serialize;

#[doc(hidden)]
pub(crate) const NUMERIC_LOOP_REGISTER_LIMIT: u16 = 64;

pub(crate) const MAIN_FRAME_REGISTER_HEADROOM: u16 = 24;

pub(crate) mod operands;

use crate::bytecode::instruction::operands::ArrayValueMode;
use crate::bytecode::instruction::operands::AsMode;
use crate::bytecode::instruction::operands::CallDescriptorIndex;
use crate::bytecode::instruction::operands::Comparison;
use crate::bytecode::instruction::operands::ConstantIndex;
use crate::bytecode::instruction::operands::Count;
use crate::bytecode::instruction::operands::DescriptorIndex;
use crate::bytecode::instruction::operands::FloatPairUpdateDescriptorIndex;
use crate::bytecode::instruction::operands::FloatSquaresSumBranchDescriptorIndex;
use crate::bytecode::instruction::operands::IcSlot;
use crate::bytecode::instruction::operands::ImmediateInt;
use crate::bytecode::instruction::operands::IndexAddMode;
use crate::bytecode::instruction::operands::IntStepLoopDescriptorIndex;
use crate::bytecode::instruction::operands::JumpOffset;
use crate::bytecode::instruction::operands::PreparedIntLoopDescriptorIndex;
use crate::bytecode::instruction::operands::PresetDescriptorIndex;
use crate::bytecode::instruction::operands::PropertyIndexUpdateMode;
use crate::bytecode::instruction::operands::PropertyInitializationDescriptorIndex;
use crate::bytecode::instruction::operands::PropertyReadMode;
use crate::bytecode::instruction::operands::PropertyRemoveMode;
use crate::bytecode::instruction::operands::PropertySlot;
use crate::bytecode::instruction::operands::PropertyValueMode;
use crate::bytecode::instruction::operands::Register;
use crate::bytecode::instruction::operands::ShortJumpOffset;
use crate::bytecode::instruction::operands::SwitchTableIndex;

macro_rules! instruction_set {
    ($declaration:ident) => {
        $declaration! {
            Move { destination: Register, source: Register, } = 0,
            LoadConstant { destination: Register, constant: ConstantIndex, } = 1,
            LoadNull { destination: Register, } = 2,
            LoadTrue { destination: Register, } = 3,
            LoadFalse { destination: Register, } = 4,
            LoadInt { destination: Register, immediate: ImmediateInt, } = 5,
            /// `destination = left + right`; numeric operands only.
            Add { destination: Register, left: Register, right: Register, } = 6,
            /// `destination = left - right`; numeric operands only.
            Subtract { destination: Register, left: Register, right: Register, } = 7,
            /// `destination = left * right`; numeric operands only.
            Multiply { destination: Register, left: Register, right: Register, } = 8,
            /// `destination = left / right`; the result is always a float.
            Divide { destination: Register, left: Register, right: Register, } = 9,
            /// `destination = left % right`; integer operands only.
            Modulo { destination: Register, left: Register, right: Register, } = 10,
            /// `destination = left ** right`; numeric operands only.
            Power { destination: Register, left: Register, right: Register, } = 11,
            /// `destination = -source`; numeric operand only.
            Negate { destination: Register, source: Register, } = 12,
            /// `destination = +source`; numeric operand only.
            UnaryPlus { destination: Register, source: Register, } = 13,
            AddImmediate { destination: Register, source: Register, immediate: ImmediateInt, } = 14,
            SubtractImmediate { destination: Register, source: Register, immediate: ImmediateInt, } = 15,
            /// `destination = left . right`; strings and numbers only.
            Concatenate { destination: Register, left: Register, right: Register, } = 16,
            /// `destination = left & right`; integer operands only.
            BitwiseAnd { destination: Register, left: Register, right: Register, } = 17,
            /// `destination = left | right`; integer operands only.
            BitwiseOr { destination: Register, left: Register, right: Register, } = 18,
            /// `destination = left ^ right`; integer operands only.
            BitwiseXor { destination: Register, left: Register, right: Register, } = 19,
            /// `destination = ~source`; integer operand only.
            BitwiseNot { destination: Register, source: Register, } = 20,
            /// `destination = left << right`; integer operands only.
            ShiftLeft { destination: Register, left: Register, right: Register, } = 21,
            /// `destination = left >> right`; integer operands only.
            ShiftRight { destination: Register, left: Register, right: Register, } = 22,
            /// `destination = left == right`; total, never throws.
            Equal { destination: Register, left: Register, right: Register, } = 23,
            /// `destination = left != right`; total, never throws.
            NotEqual { destination: Register, left: Register, right: Register, } = 24,
            /// `destination = left < right`; partial ordering, throws on
            /// incomparable operands.
            LessThan { destination: Register, left: Register, right: Register, } = 25,
            /// `destination = left <= right`; partial ordering, throws on
            /// incomparable operands.
            LessThanOrEqual { destination: Register, left: Register, right: Register, } = 26,
            /// `destination = left > right`; partial ordering, throws on
            /// incomparable operands.
            GreaterThan { destination: Register, left: Register, right: Register, } = 27,
            /// `destination = left >= right`; partial ordering, throws on
            /// incomparable operands.
            GreaterThanOrEqual { destination: Register, left: Register, right: Register, } = 28,
            /// `destination = left <=> right`; throws on incomparable operands and
            /// on NaN.
            Compare { destination: Register, left: Register, right: Register, } = 29,
            Not { destination: Register, source: Register, } = 30,
            Jump { offset: JumpOffset, } = 31,
            /// Jumps when `condition` is `false`; a non-boolean condition throws.
            JumpIfFalse { condition: Register, offset: JumpOffset, } = 32,
            /// Jumps when `condition` is `true`; a non-boolean condition throws.
            JumpIfTrue { condition: Register, offset: JumpOffset, } = 33,
            JumpIfNull { subject: Register, offset: JumpOffset, } = 34,
            JumpIfNotNull { subject: Register, offset: JumpOffset, } = 35,
            SwitchInt { subject: Register, table: SwitchTableIndex, } = 36,
            SwitchString { subject: Register, table: SwitchTableIndex, } = 37,
            /// Throws `UndefinedVariableError` named by the constant at `name` when
            /// `subject` is still uninitialized.
            CheckDefined { subject: Register, name: ConstantIndex, } = 38,
            NewVec { element_count: Count, destination: Register, first_element: Register, } = 39,
            NewDict { pair_count: Count, destination: Register, first_pair: Register, } = 40,
            NewTuple { element_count: Count, destination: Register, first_element: Register, } = 41,
            /// `destination = container[index]`; throws on a bad index or key.
            IndexGet { destination: Register, container: Register, index: Register, } = 42,
            IndexSet { container: Register, index: Register, value: Register, } = 43,
            Append { container: Register, value: Register, } = 44,
            /// Spreads every element of `value` into `container`: a vec literal's
            /// `...`, taking a vec or tuple, and a dict literal's, taking a vec,
            /// tuple, or dict. A source the container does not accept throws.
            Spread { container: Register, value: Register, } = 87,
            /// Collects `subject`'s elements from `from` onward into a fresh vec in
            /// `destination`: a destructuring pattern's `...$r`. The result is always
            /// a vec, whatever the subject was, and is empty when nothing is left.
            Rest { destination: Register, subject: Register, from: ImmediateInt, } = 88,
            Length { destination: Register, source: Register, } = 45,
            /// `destination = remove!(container, key)`; throws on a missing key.
            Remove { destination: Register, container: Register, key: Register, } = 46,
            /// `destination = remove_first!(container)`; throws on an empty vec.
            RemoveFirst { destination: Register, container: Register, } = 47,
            /// `destination = remove_last!(container)`; throws on an empty vec.
            RemoveLast { destination: Register, container: Register, } = 48,
            /// Throws `TypeError` unless `subject` is a vec or tuple with at least
            /// `required` elements and, unless a `...` rest accepts surplus elements,
            /// no more than `arity`. The destructuring guard.
            CheckDestructure { subject: Register, required: ImmediateInt, arity: ImmediateInt, rest: bool, } = 49,
            /// Reads a proven in-range element from a vec or tuple.
            ElementGet { destination: Register, subject: Register, index: ImmediateInt, } = 50,
            /// Instantiates the class named by the cache descriptor at `cache` into
            /// `destination`; the slot caches the resolved class.
            NewStatic { destination: Register, cache: IcSlot, } = 51,
            NewDynamic { destination: Register, class_name: Register, } = 52,
            NewTyped { destination: Register, descriptor: DescriptorIndex, } = 90,
            /// `destination = object->property`, the property named by the cache
            /// descriptor at `cache`; the slot caches the resolved slot index.
            PropertyGet { destination: Register, object: Register, cache: IcSlot, } = 53,
            /// `object->property = value`, the property named by the cache
            /// descriptor at `cache`; enforces visibility and readonly.
            PropertySet { object: Register, value: Register, cache: IcSlot, } = 54,
            /// Initializes a constructor-promoted property or applies a checked
            /// `clone!` override; named by the descriptor at `cache`.
            PropertyInitRaw { object: Register, value: Register, cache: IcSlot, } = 55,
            CloneObject { destination: Register, source: Register, } = 56,
            /// Reads the static property named by the cache descriptor at `cache`
            /// into `destination`.
            StaticPropertyGet { destination: Register, cache: IcSlot, } = 57,
            /// Writes `value` into the static property named by the cache descriptor
            /// at `cache`.
            StaticPropertySet { cache: IcSlot, value: Register, } = 58,
            /// Reads the constant named by the cache descriptor at `cache` into
            /// `destination`.
            ConstantGet { destination: Register, cache: IcSlot, } = 59,
            /// Reads the class constant named by the cache descriptor at `cache`
            /// into `destination`.
            ClassConstantGet { destination: Register, cache: IcSlot, } = 60,
            CallValue { argument_count: Count, destination: Register, callee: Register, first_argument: Register, } = 61,
            /// Calls the function named by the cache descriptor at `cache` with
            /// `argument_count` arguments starting at `first_argument`.
            CallNamed { argument_count: Count, destination: Register, first_argument: Register, cache: IcSlot, } = 62,
            /// Calls the method named by the cache descriptor at `cache`; the
            /// receiver sits at `first_argument` and is included in the count.
            CallMethod { argument_count: Count, destination: Register, first_argument: Register, cache: IcSlot, } = 63,
            /// Calls the static method named by the cache descriptor at `cache`
            /// (class and method) with `argument_count` arguments starting at
            /// `first_argument`.
            CallStatic { argument_count: Count, destination: Register, first_argument: Register, cache: IcSlot, } = 64,
            /// Calls the callable value in `callee` with the argument shape from the
            /// call descriptor at `descriptor`; the argument window starts at
            /// `callee + 1`, positionals first, then the named values in descriptor
            /// order.
            CallWithNames { destination: Register, callee: Register, descriptor: CallDescriptorIndex, } = 65,
            Return { source: Register, } = 66,
            ReturnNull = 67,
            MakeClosure { capture_count: Count, destination: Register, prototype: ConstantIndex, first_capture: Register, } = 68,
            /// Builds a bound or partially applied callable from the value in
            /// `callee`, shaped by the preset descriptor at `descriptor`: the given
            /// values follow at `callee + 1` in slot order, holes stay open. An
            /// empty descriptor is a first-class callable, a pure binding.
            MakeBound { destination: Register, callee: Register, descriptor: PresetDescriptorIndex, } = 69,
            Is { destination: Register, source: Register, descriptor: DescriptorIndex, } = 70,
            /// `destination = source as T`; throws `TypeError` when the value does
            /// not conform to the type descriptor at `descriptor`.
            AsCheck { destination: Register, source: Register, descriptor: DescriptorIndex, mode: AsMode, } = 71,
            AsOrNull { destination: Register, source: Register, descriptor: DescriptorIndex, } = 72,
            Throw { source: Register, } = 73,
            Rethrow = 74,
            /// Throws `UnhandledMatchError` for the unmatched `subject`.
            ThrowUnhandledMatch { subject: Register, } = 75,
            ForeachInit { iterator: Register, subject: Register, reserve: Register, } = 76,
            ForeachNext { iterator: Register, key_destination: Register, value_destination: Register, } = 77,
            Write { value_count: Count, first_value: Register, } = 78,
            WriteLine { value_count: Count, first_value: Register, } = 79,
            WriteError { value_count: Count, first_value: Register, } = 80,
            WriteErrorLine { value_count: Count, first_value: Register, } = 81,
            Debug { value_count: Count, first_value: Register, } = 82,
            /// Throws `AssertionError` when the condition at `first_value` is
            /// `false`. The next `operand_count` registers retain values used only
            /// for a failure diagnostic, `message` is optional, and `text` names the
            /// condition's source text.
            Assert { operand_count: Count, first_value: Register, message: Register, text: ConstantIndex, } = 83,
            Exit { code: Register, } = 84,
            /// Loads and runs the file whose path string is in `path`, storing its
            /// return value in `destination`; `once` makes a repeated load yield
            /// `null` without re-running.
            Require { once: bool, destination: Register, path: Register, } = 85,
            /// Jumps by `offset` when `target` already holds a value; when `target`
            /// holds the uninitialized sentinel, control falls through into the
            /// default's evaluation, which must end by writing `target`.
            FillDefault { target: Register, offset: JumpOffset, } = 86,
            /// Compares `left` and `right`, jumping when the comparison is false.
            /// Used only when the offset fits [`ShortJumpOffset`]; otherwise codegen
            /// keeps the ordinary compare-then-jump pair.
            JumpUnless { comparison: Comparison, left: Register, right: Register, offset: ShortJumpOffset, } = 91,
            /// Adds `immediate` to `target` in place, then jumps unconditionally.
            /// Emitted only when the jump fits [`ShortJumpOffset`] and the arithmetic
            /// source and destination are the same register.
            IncrementJump { target: Register, immediate: ImmediateInt, offset: ShortJumpOffset, } = 92,
            Squares { first_destination: Register, first_source: Register, second_source: Register, } = 93,
            CounterLoop { comparison: Comparison, counter: Register, limit: Register, offset: ShortJumpOffset, } = 94,
            /// Executes a closed, side-effect-free numeric counted loop with unboxed
            /// scalar registers. The body remains in the chunk immediately after this
            /// instruction so the VM can deoptimize to ordinary dispatch at any body
            /// instruction whose dynamic operands are not numeric.
            NumericLoop { comparison: Comparison, left: Register, right: Register, offset: ShortJumpOffset, } = 95,
            /// Updates an array held by an object property in place.
            PropertyIndexUpdate { object: Register, operand: Register, cache: IcSlot, mode: PropertyIndexUpdateMode, } = 96,
            PropertyStep { object: Register, cache: IcSlot, immediate: ImmediateInt, } = 97,
            PropertyAdd { object: Register, source: Register, cache: IcSlot, } = 98,
            /// Returns `source` after the optimizer proved it satisfies the declared
            /// return type on every path reaching this instruction.
            ReturnUnchecked { source: Register, } = 99,
            /// Returns `null` after the optimizer proved it satisfies the declared
            /// return type.
            ReturnNullUnchecked = 100,
            /// Adds two values proven to be floats without repeating their type checks.
            FloatAdd { destination: Register, left: Register, right: Register, } = 101,
            /// Subtracts two values proven to be floats without repeating their type checks.
            FloatSubtract { destination: Register, left: Register, right: Register, } = 102,
            /// Multiplies two values proven to be floats without repeating their type checks.
            FloatMultiply { destination: Register, left: Register, right: Register, } = 103,
            FloatSquares { first_destination: Register, first_source: Register, second_source: Register, } = 104,
            FloatMultiplyConstant { destination: Register, source: Register, constant: ConstantIndex, } = 105,
            JumpUnlessConstant { comparison: Comparison, source: Register, constant: ConstantIndex, offset: ShortJumpOffset, } = 106,
            PropertyFillIntRange { object: Register, first_operand: Register, cache: IcSlot, } = 107,
            FloatSquaresSum { first_destination: Register, first_source: Register, second_source: Register, } = 108,
            /// Computes `(left - right) + addend` with a rounding step after the
            /// subtraction. `first_operand` names the adjacent `left` and `right`.
            FloatDifferenceAdd { destination: Register, first_operand: Register, addend: Register, } = 109,
            FloatScaleProductAdd { destination: Register, first_operand: Register, constant: ConstantIndex, } = 110,
            FloatSquaresSumBranch { descriptor: FloatSquaresSumBranchDescriptorIndex, offset: JumpOffset, } = 111,
            /// Integer-only form of [`Instruction::CounterLoop`] selected when type
            /// flow proves both the counter and its limit are integers.
            IntCounterLoop { comparison: Comparison, counter: Register, limit: Register, offset: ShortJumpOffset, } = 112,
            /// Integer-header form of [`Instruction::NumericLoop`]. Its initial
            /// comparison and counted back edge are both statically integer-only.
            IntNumericLoop { comparison: Comparison, left: Register, right: Register, offset: ShortJumpOffset, } = 113,
            PreparedIntNumericLoop { descriptor: PreparedIntLoopDescriptorIndex, offset: ShortJumpOffset, } = 114,
            /// Calls an exact final-class, non-generic method after whole-unit type
            /// flow proved the receiver, arity, and every supplied argument.
            CallMethodUnchecked { argument_count: Count, destination: Register, first_argument: Register, cache: IcSlot, } = 115,
            FloatPairUpdate { descriptor: FloatPairUpdateDescriptorIndex, } = 116,
            IntJumpUnless { comparison: Comparison, left: Register, right: Register, offset: ShortJumpOffset, } = 117,
            /// Writes a property after whole-unit type flow proved the receiver's
            /// exact property, its mutability, and the stored value's declared type.
            PropertySetUnchecked { object: Register, value: Register, slot: PropertySlot, value_mode: PropertyValueMode, } = 118,
            /// Updates a proven mutable array property in place.
            PropertyIndexUpdateUnchecked { object: Register, operand: Register, slot: PropertySlot, mode: PropertyIndexUpdateMode, } = 119,
            /// Steps a numeric property after whole-unit type flow proved that the
            /// property is mutable and the result retains its declared type.
            PropertyStepUnchecked { object: Register, slot: PropertySlot, immediate: ImmediateInt, } = 120,
            /// Adds into a numeric property after whole-unit type flow proved that
            /// the property is mutable and the result retains its declared type.
            PropertyAddUnchecked { object: Register, source: Register, slot: PropertySlot, } = 121,
            PropertyGetUnchecked { destination: Register, object: Register, slot: PropertySlot, value_mode: PropertyReadMode, } = 122,
            IntAdd { destination: Register, left: Register, right: Register, } = 123,
            IntSubtract { destination: Register, left: Register, right: Register, } = 124,
            IntMultiply { destination: Register, left: Register, right: Register, } = 125,
            IntModulo { destination: Register, left: Register, right: Register, } = 126,
            VecIndexGet { destination: Register, container: Register, index: Register, value_mode: ArrayValueMode, } = 127,
            VecIndexSet { container: Register, index: Register, value: Register, } = 128,
            VecAppend { container: Register, value: Register, } = 129,
            DictIndexGetIntKey { destination: Register, container: Register, index: Register, value_mode: ArrayValueMode, } = 130,
            DictIndexSetIntKey { container: Register, index: Register, value: Register, } = 131,
            DictIndexGetStringKey { destination: Register, container: Register, index: Register, value_mode: ArrayValueMode, } = 132,
            DictIndexSetStringKey { container: Register, index: Register, value: Register, } = 133,
            IndexAddAssign { container: Register, index: Register, value: Register, mode: IndexAddMode, } = 146,
            NumericRegionJump { offset: JumpOffset, } = 147,
            CallNamedUnchecked { argument_count: Count, destination: Register, first_argument: Register, cache: IcSlot, } = 134,
            CallSelfUnchecked { argument_count: Count, destination: Register, first_argument: Register, } = 135,
            /// Calls an exact instance method directly from a proven caller-register
            /// window, borrowing the receiver for the duration of the frame.
            CallMethodDirect { argument_count: Count, destination: Register, first_argument: Register, cache: IcSlot, } = 136,
            VecForeachNext { iterator: Register, key_destination: Register, value_destination: Register, value_mode: ArrayValueMode, } = 137,
            DictForeachNext { iterator: Register, key_destination: Register, value_destination: Register, value_mode: ArrayValueMode, } = 138,
            StringLength { destination: Register, source: Register, } = 139,
            IntAddAssign { target: Register, source: Register, } = 140,
            CallNamedConstantUnchecked {
                destination: Register,
                constant: ConstantIndex,
                cache: IcSlot,
                /// The callee only borrows the literal parameter; its constant-pool
                /// atom keeps a string alive for the complete frame.
                borrowed: bool,
            } = 141,
            IntJumpUnlessImmediate { comparison: Comparison, source: Register, immediate: ImmediateInt, offset: ShortJumpOffset, } = 142,
            /// Returns an immediate integer after the optimizer proved it satisfies
            /// the declared return type.
            ReturnIntUnchecked { immediate: ImmediateInt, } = 143,
            /// Returns a reference-counted register after the optimizer proved both
            /// its type and ownership category.
            ReturnReferenceUnchecked { source: Register, } = 144,
            /// Returns a scalar register after the optimizer proved both its type and
            /// ownership category.
            ReturnScalarUnchecked { source: Register, } = 145,
            MoveOwned { destination: Register, source: Register, } = 149,
            IntBitwiseAnd { destination: Register, left: Register, right: Register, } = 150,
            IntBitwiseOr { destination: Register, left: Register, right: Register, } = 151,
            IntBitwiseXor { destination: Register, left: Register, right: Register, } = 152,
            IntBitwiseNot { destination: Register, source: Register, } = 153,
            IntShiftLeft { destination: Register, left: Register, right: Register, } = 154,
            IntShiftRight { destination: Register, left: Register, right: Register, } = 155,
            DrainFinalizers = 156,
            Clear { target: Register, } = 157,
            /// Throws when another strong reference keeps `source` alive.
            CheckSoleReference { source: Register, message: ConstantIndex, chain_previous: bool, } = 158,
            /// Calls a callable value whose result must either be consumed or
            /// explicitly discarded.
            CallValueDiscarded { argument_count: Count, destination: Register, callee: Register, first_argument: Register, } = 159,
            CallNamedDiscarded { argument_count: Count, destination: Register, first_argument: Register, cache: IcSlot, } = 160,
            CallMethodDiscarded { argument_count: Count, destination: Register, first_argument: Register, cache: IcSlot, } = 161,
            CallStaticDiscarded { argument_count: Count, destination: Register, first_argument: Register, cache: IcSlot, } = 162,
            CallWithNamesDiscarded { destination: Register, callee: Register, descriptor: CallDescriptorIndex, } = 163,
            CheckDiscardedResult { source: Register, } = 164,
            IntStepLoop { descriptor: IntStepLoopDescriptorIndex, offset: ShortJumpOffset, } = 165,
            /// Calls a callable value after type flow proved its arity and argument
            /// types against the callable's declared signature.
            CallValueUnchecked { argument_count: Count, destination: Register, callee: Register, first_argument: Register, } = 166,
            /// Returns a two-element tuple from two registers after the optimizer
            /// proved the declared return type. Iterator continuations may consume
            /// the pair directly without materializing the tuple.
            ReturnPairUnchecked { first: Register, second: Register, } = 167,
            IntMultiplyImmediate { destination: Register, source: Register, immediate: ImmediateInt, } = 168,
            IntModuloImmediate { destination: Register, source: Register, immediate: ImmediateInt, } = 169,
            DictIndexSet { container: Register, index: Register, value: Register, } = 170,
            /// Reserves capacity in a proven fresh array before a counted fill
            /// loop. Non-positive and excessively large hints are ignored or capped
            /// by the VM without changing array semantics.
            ReserveArray { container: Register, additional: Register, } = 171,
            Contains { destination: Register, array: Register, value: Register, } = 172,
            ContainsKey { destination: Register, array: Register, key: Register, } = 173,
            NewFilledVec { destination: Register, value: Register, size: Register, } = 174,
            StringIndexGet { destination: Register, container: Register, index: Register, } = 175,
            StringJumpUnless { comparison: Comparison, left: Register, right: Register, offset: ShortJumpOffset, } = 176,
            /// Reads one indexed byte from a proven string and jumps unless it equals
            /// the immediate byte.
            StringByteJumpUnlessEqual { container: Register, index: Register, byte: u8, offset: ShortJumpOffset, } = 177,
            /// Reads one indexed byte from a proven string and jumps unless it differs
            /// from the immediate byte.
            StringByteJumpUnlessNotEqual { container: Register, index: Register, byte: u8, offset: ShortJumpOffset, } = 178,
            StringByteEqual { destination: Register, container: Register, index: Register, byte: u8, } = 179,
            StringByteNotEqual { destination: Register, container: Register, index: Register, byte: u8, } = 180,
            StringByteLessThan { destination: Register, container: Register, index: Register, byte: u8, } = 181,
            StringByteLessThanOrEqual { destination: Register, container: Register, index: Register, byte: u8, } = 182,
            StringByteGreaterThan { destination: Register, container: Register, index: Register, byte: u8, } = 183,
            StringByteGreaterThanOrEqual { destination: Register, container: Register, index: Register, byte: u8, } = 184,
            /// Initializes several proven slots of one fresh object.
            InitializeProperties { object: Register, cache: IcSlot, descriptor: PropertyInitializationDescriptorIndex, } = 185,
            /// Sets an element of an array property in place.
            PropertyIndexSet { object: Register, first_operand: Register, cache: IcSlot, } = 186,
            /// Sets an element after type flow proves the property access and value.
            PropertyIndexSetUnchecked { object: Register, first_operand: Register, slot: PropertySlot, } = 187,
            /// Removes from a array property and returns the removed value.
            /// For `Key`, the key follows `destination` in the register window.
            PropertyRemove { object: Register, destination: Register, cache: IcSlot, mode: PropertyRemoveMode, } = 188,
            /// The proven-property form of [`Instruction::PropertyRemove`].
            PropertyRemoveUnchecked { object: Register, destination: Register, slot: PropertySlot, mode: PropertyRemoveMode, } = 189,
            SwitchPattern { subject: Register, table: SwitchTableIndex, } = 190,
            SwitchTuplePattern { first_element: Register, element_count: Count, table: SwitchTableIndex, } = 191,
            SwitchBool { subject: Register, table: SwitchTableIndex, } = 192,
            SwitchFloat { subject: Register, table: SwitchTableIndex, } = 193,
            IntRangeJumpIf { subject: Register, descriptor: DescriptorIndex, offset: ShortJumpOffset, } = 194,
            IntRangeJumpUnless { subject: Register, descriptor: DescriptorIndex, offset: ShortJumpOffset, } = 195,
            BoolPatternBranch { subject: Register, false_offset: ShortJumpOffset, default_offset: ShortJumpOffset, } = 196,
            /// Stops the process with status 255 after printing the string constant
            /// at `message` and the current stack trace.
            Panic { message: ConstantIndex, } = 197,
            /// `destination = swap_remove!(container, index)`; does not preserve order.
            SwapRemove { destination: Register, container: Register, index: Register, } = 198,
            /// `destination = source . constants[constant]`; the constant is a string.
            ConcatenateRightConstant { destination: Register, source: Register, constant: ConstantIndex, } = 199,
            /// `destination = constants[constant] . source`; the constant is a string.
            ConcatenateLeftConstant { destination: Register, source: Register, constant: ConstantIndex, } = 200,
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
        pub(crate) enum Instruction {
            $($variant)*
        }
    };
}

/// An encoded operand whose value refers outside its instruction word.
#[derive(Clone, Copy)]
pub(in crate::bytecode) enum InstructionOperand {
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

pub(crate) trait InstructionSideTableMapper {
    type Error;

    fn constant(&mut self, value: ConstantIndex) -> Result<ConstantIndex, Self::Error>;
    fn cache(&mut self, value: IcSlot) -> Result<IcSlot, Self::Error>;
    fn switch(&mut self, value: SwitchTableIndex) -> Result<SwitchTableIndex, Self::Error>;
    fn descriptor(&mut self, value: DescriptorIndex) -> Result<DescriptorIndex, Self::Error>;
    fn call(&mut self, value: CallDescriptorIndex) -> Result<CallDescriptorIndex, Self::Error>;
    fn preset(
        &mut self,
        value: PresetDescriptorIndex,
    ) -> Result<PresetDescriptorIndex, Self::Error>;
    fn float_pair_update(
        &mut self,
        value: FloatPairUpdateDescriptorIndex,
    ) -> Result<FloatPairUpdateDescriptorIndex, Self::Error>;
    fn float_squares_sum_branch(
        &mut self,
        value: FloatSquaresSumBranchDescriptorIndex,
    ) -> Result<FloatSquaresSumBranchDescriptorIndex, Self::Error>;
    fn int_step_loop(
        &mut self,
        value: IntStepLoopDescriptorIndex,
    ) -> Result<IntStepLoopDescriptorIndex, Self::Error>;
    fn prepared_int_loop(
        &mut self,
        value: PreparedIntLoopDescriptorIndex,
    ) -> Result<PreparedIntLoopDescriptorIndex, Self::Error>;
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
            pub(in crate::bytecode) fn try_visit_operands<E>(
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
            pub(crate) fn try_map_side_tables<M: InstructionSideTableMapper>(
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

pub(crate) mod word;

instruction_set!(define_instruction);
instruction_set!(define_operand_visit);
instruction_set!(define_side_table_map);

const _: () = assert!(size_of::<Instruction>() == 8);
const _: () = assert!(align_of::<Instruction>() == 1);
