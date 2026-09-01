//! The instruction tag enumeration and the packed instruction word.

use std::mem;
use std::ptr;

use crate::bytecode::instruction::Instruction;

macro_rules! define_instruction_kind {
    ($($(#[$attribute:meta])* $name:ident $({$($(#[$field_attribute:meta])* $field:ident: $type:ty),* $(,)?})? = $tag:literal,)*) => {
        #[derive(Debug, Clone, Copy, PartialEq, Eq)]
        #[repr(u8)]
        #[expect(
            dead_code,
            reason = "variants are materialized through instruction-tag transmutation"
        )]
        pub(crate) enum InstructionKind {
            $($name = $tag,)*
        }
    };
}

instruction_set!(define_instruction_kind);

impl Instruction {
    #[must_use]
    pub(crate) fn kind(&self) -> InstructionKind {
        // SAFETY: `Instruction` stores its `repr(u8)` tag in the first byte.
        let tag = unsafe { *ptr::from_ref(self).cast::<u8>() };
        // SAFETY: every live instruction has a valid tag.
        unsafe { mem::transmute::<u8, InstructionKind>(tag) }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(transparent)]
pub(crate) struct InstructionWord(u64);

impl InstructionWord {
    /// Fetches one packed instruction from verified bytecode.
    ///
    /// # Safety
    ///
    /// `instruction` must point to a live [`Instruction`].
    #[must_use]
    pub(crate) const unsafe fn read(instruction: *const Instruction) -> Self {
        // SAFETY: the caller provides a live, fully initialized instruction.
        let bytes = unsafe { instruction.cast::<[u8; 8]>().read_unaligned() };
        Self(u64::from_le_bytes(bytes))
    }

    #[must_use]
    pub(crate) fn kind(self) -> InstructionKind {
        // SAFETY: this word came from a live instruction with a valid tag.
        unsafe { mem::transmute::<u8, InstructionKind>(self.0.to_le_bytes()[0]) }
    }

    /// # Safety
    ///
    /// The caller must have selected the variant returned by [`Self::kind`].
    #[must_use]
    pub(crate) unsafe fn decode(self) -> Instruction {
        // SAFETY: the selected tag matches the preserved instruction bytes.
        unsafe { mem::transmute::<[u8; 8], Instruction>(self.0.to_le_bytes()) }
    }
}
