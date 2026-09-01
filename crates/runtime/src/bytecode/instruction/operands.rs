//! The operand encodings packed into the eight-byte instruction word.

use serde::Deserialize;
use serde::Serialize;

macro_rules! integer_operand {
    ($(#[$attribute:meta])* $name:ident($integer:ty) => $accessor:ident) => {
        $(#[$attribute])*
        #[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
        pub(crate) struct $name([u8; size_of::<$integer>()]);

        impl $name {
            #[must_use]
            pub(crate) const fn new(value: $integer) -> Self {
                Self(value.to_le_bytes())
            }

            #[must_use]
            pub(crate) const fn $accessor(self) -> $integer {
                <$integer>::from_le_bytes(self.0)
            }
        }
    };
}

integer_operand!(
    Register(u16) => index
);

impl Register {
    pub(crate) const NONE: Self = Self::new(u16::MAX);
}

integer_operand!(
    ConstantIndex(u16) => index
);
integer_operand!(
    DescriptorIndex(u16) => index
);
integer_operand!(
    CallDescriptorIndex(u16) => index
);
integer_operand!(
    SwitchTableIndex(u16) => index
);
integer_operand!(
    /// An index into the chunk's inline-cache descriptors: one runtime cache
    /// slot per site, whose descriptor carries the names the site resolves.
    IcSlot(u16) => index
);
integer_operand!(
    PropertySlot(u16) => index
);
integer_operand!(
    PresetDescriptorIndex(u16) => index
);
integer_operand!(
    PreparedIntLoopDescriptorIndex(u16) => index
);
integer_operand!(
    IntStepLoopDescriptorIndex(u16) => index
);
integer_operand!(
    FloatSquaresSumBranchDescriptorIndex(u16) => index
);
integer_operand!(
    FloatPairUpdateDescriptorIndex(u16) => index
);
integer_operand!(
    PropertyInitializationDescriptorIndex(u16) => index
);
integer_operand!(
    /// A jump offset in instruction indices, relative to the index of the
    /// jumping instruction itself.
    JumpOffset(i32) => offset
);
integer_operand!(
    /// A signed 16-bit relative jump used by packed superinstructions.
    ShortJumpOffset(i16) => offset
);

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[repr(u8)]
pub(crate) enum Comparison {
    Equal,
    NotEqual,
    LessThan,
    LessThanOrEqual,
    GreaterThan,
    GreaterThanOrEqual,
}

impl Comparison {
    #[must_use]
    pub(crate) const fn operator(self) -> &'static str {
        match self {
            Self::Equal => "==",
            Self::NotEqual => "!=",
            Self::LessThan => "<",
            Self::LessThanOrEqual => "<=",
            Self::GreaterThan => ">",
            Self::GreaterThanOrEqual => ">=",
        }
    }

    /// The equivalent comparison after exchanging its operands.
    #[must_use]
    pub(crate) const fn reversed(self) -> Self {
        match self {
            Self::Equal => Self::Equal,
            Self::NotEqual => Self::NotEqual,
            Self::LessThan => Self::GreaterThan,
            Self::LessThanOrEqual => Self::GreaterThanOrEqual,
            Self::GreaterThan => Self::LessThan,
            Self::GreaterThanOrEqual => Self::LessThanOrEqual,
        }
    }

    #[must_use]
    pub(crate) const fn negated(self) -> Self {
        match self {
            Self::Equal => Self::NotEqual,
            Self::NotEqual => Self::Equal,
            Self::LessThan => Self::GreaterThanOrEqual,
            Self::LessThanOrEqual => Self::GreaterThan,
            Self::GreaterThan => Self::LessThanOrEqual,
            Self::GreaterThanOrEqual => Self::LessThan,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[repr(u8)]
pub(crate) enum IndexAddMode {
    Generic,
    DictAnyKeyIntValue,
    DictStringKeyIntValue,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[repr(u8)]
pub(crate) enum AsMode {
    /// Preserve the value after checking that it already satisfies the type.
    Boundary,
    Cast,
}

integer_operand!(
    ImmediateInt(i16) => value
);

/// An element or argument count, occupying the byte after the tag.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct Count(u8);

impl Count {
    #[must_use]
    pub(crate) const fn new(count: u8) -> Self {
        Self(count)
    }

    #[must_use]
    pub(crate) const fn value(self) -> u8 {
        self.0
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[repr(u8)]
pub(crate) enum CollectionValueMode {
    /// The element may be any value.
    Generic,
    Int,
    Float,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[repr(u8)]
pub(crate) enum PropertyReadMode {
    Clone,
    Take,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[repr(u8)]
pub(crate) enum PropertyIndexUpdateMode {
    Increment,
    Remove,
    Append,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[repr(u8)]
pub(crate) enum PropertyRemoveMode {
    Key,
    First,
    Last,
    Swap,
}

impl PropertyRemoveMode {
    #[must_use]
    pub(crate) const fn uses_operand(self) -> bool {
        matches!(self, Self::Key | Self::Swap)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[repr(u8)]
pub(crate) enum PropertyValueMode {
    Clone,
    /// Move the value, retaining the physical register's teardown bit because
    /// a later temporary reuses it.
    Move,
    MoveAndClear,
    /// Clone a value into a fresh instance that user code cannot yet observe.
    FreshClone,
    /// Move a value into a fresh instance that user code cannot yet observe,
    /// retaining the physical register's teardown bit.
    FreshMove,
    /// Move a value into a fresh instance that user code cannot yet observe
    /// and clear its teardown bit.
    FreshMoveAndClear,
}

impl PropertyValueMode {
    #[must_use]
    pub(crate) const fn moves(self) -> bool {
        matches!(
            self,
            Self::Move | Self::MoveAndClear | Self::FreshMove | Self::FreshMoveAndClear
        )
    }

    #[must_use]
    pub(crate) const fn clears_reference_mask(self) -> bool {
        matches!(self, Self::MoveAndClear | Self::FreshMoveAndClear)
    }

    /// Whether user code cannot yet observe the receiver.
    #[must_use]
    pub(crate) const fn fresh_receiver(self) -> bool {
        matches!(
            self,
            Self::FreshClone | Self::FreshMove | Self::FreshMoveAndClear
        )
    }
}
