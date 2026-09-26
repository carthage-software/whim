//! The instruction tag enumeration and the packed instruction word.

use std::fmt;
use std::mem;
use std::mem::MaybeUninit;
use std::ptr;

use crate::instruction::Instruction;

macro_rules! define_instruction_kind {
    ($($(#[$attribute:meta])* $name:ident $({$($(#[$field_attribute:meta])* $field:ident: $type:ty),* $(,)?})? = $tag:literal,)*) => {
        #[derive(Debug, Clone, Copy, PartialEq, Eq)]
        #[repr(u8)]
        pub enum InstructionKind {
            $($name = $tag,)*
        }
    };
}

instruction_set!(define_instruction_kind);

impl Instruction {
    #[must_use]
    pub fn kind(&self) -> InstructionKind {
        // SAFETY: `Instruction` stores its `repr(u8)` tag in the first byte.
        let tag = unsafe { *ptr::from_ref(self).cast::<u8>() };
        // SAFETY: every live instruction has a valid tag.
        unsafe { mem::transmute::<u8, InstructionKind>(tag) }
    }
}

#[derive(Clone, Copy)]
#[repr(transparent)]
pub struct InstructionWord(MaybeUninit<u64>);

impl fmt::Debug for InstructionWord {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        // SAFETY: every word preserves the tag and fields of a live instruction.
        let instruction = unsafe { self.decode() };
        formatter
            .debug_tuple("InstructionWord")
            .field(&instruction)
            .finish()
    }
}

impl PartialEq for InstructionWord {
    fn eq(&self, other: &Self) -> bool {
        // SAFETY: both words preserve the tags and fields of live instructions.
        unsafe { self.decode() == other.decode() }
    }
}

impl Eq for InstructionWord {}

impl InstructionWord {
    /// Fetches one packed instruction from verified bytecode.
    ///
    /// # Safety
    ///
    /// `instruction` must point to a live [`Instruction`].
    #[must_use]
    pub const unsafe fn read(instruction: *const Instruction) -> Self {
        // SAFETY: MaybeUninit permits instruction padding; the source has alignment one.
        Self(unsafe { instruction.cast::<MaybeUninit<u64>>().read_unaligned() })
    }

    #[must_use]
    pub const fn kind(self) -> InstructionKind {
        // SAFETY: only the initialized repr(u8) tag is read.
        let tag = unsafe { self.0.as_ptr().cast::<u8>().read() };
        // SAFETY: every word retains a valid instruction tag.
        unsafe { mem::transmute::<u8, InstructionKind>(tag) }
    }

    /// # Safety
    ///
    /// The word must retain the tag and operands of a live [`Instruction`].
    #[must_use]
    pub const unsafe fn decode(self) -> Instruction {
        // SAFETY: active fields remain initialized; Instruction permits the unused padding.
        unsafe { mem::transmute::<MaybeUninit<u64>, Instruction>(self.0) }
    }
}
