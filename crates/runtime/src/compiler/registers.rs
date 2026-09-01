//! The register allocator: a bump allocator with floors.

use crate::bytecode::instruction::operands::Register;

use crate::unreachable_invariant;

pub(in crate::compiler) const REGISTER_CAPACITY: usize = u16::MAX as usize;

pub(in crate::compiler) struct Registers {
    next: u16,
    /// The lowest register temporaries may use: everything below holds
    /// named locals.
    floor: u16,
    high_water: u16,
}

impl Registers {
    pub(in crate::compiler) const fn new() -> Self {
        Self {
            next: 0,
            floor: 0,
            high_water: 0,
        }
    }

    /// Reserves the next register for a named local, before any temporary
    /// exists; [`None`] when the register space is exhausted.
    pub(in crate::compiler) fn reserve_local(&mut self) -> Option<Register> {
        if self.next != self.floor {
            // SAFETY: named locals are allocated before expression temporaries.
            unsafe { unreachable_invariant("locals are reserved before any temporary") };
        }

        let register = self.allocate()?;
        self.floor = self.next;
        Some(register)
    }

    pub(in crate::compiler) fn allocate(&mut self) -> Option<Register> {
        let index = self.next;
        self.next = index.checked_add(1)?;
        if self.next > self.high_water {
            self.high_water = self.next;
        }

        Some(Register::new(index))
    }

    pub(in crate::compiler) const fn mark(&self) -> u16 {
        self.next
    }

    pub(in crate::compiler) const fn temporary_floor(&self) -> u16 {
        self.floor
    }

    pub(in crate::compiler) fn release_to(&mut self, mark: u16) {
        if mark < self.floor || mark > self.next {
            // SAFETY: callers release only marks returned by this allocator.
            unsafe { unreachable_invariant("a register mark is released out of stack order") };
        }

        self.next = mark;
    }

    pub(in crate::compiler) const fn release_temporaries(&mut self) {
        self.next = self.floor;
    }

    pub(in crate::compiler) const fn pin_temporaries(&mut self) -> u16 {
        let saved = self.floor;
        self.floor = self.next;
        saved
    }

    pub(in crate::compiler) fn unpin_temporaries(&mut self, saved: u16) {
        if saved > self.floor {
            // SAFETY: callers restore only floors returned by `pin_temporaries`.
            unsafe { unreachable_invariant("a register pin is released out of stack order") };
        }

        self.floor = saved;
    }

    pub(in crate::compiler) const fn count(&self) -> u16 {
        self.high_water
    }

    pub(in crate::compiler) const fn local_count(&self) -> u16 {
        self.floor
    }
}
