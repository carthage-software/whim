//! Cached structural checks for mutable arrays.

use std::cell::Cell;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(transparent)]
pub(crate) struct ArrayTypeCheckId(u32);

impl ArrayTypeCheckId {
    pub(crate) const fn new(value: u32) -> Self {
        Self(value)
    }

    const fn get(self) -> u32 {
        self.0
    }
}

/// The result of one cached structural type check.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ArrayTypeCheck {
    Unknown,
    Clean(ArrayTypeCheckId),
    Dirty { id: ArrayTypeCheckId, slot: u32 },
}

/// Caches structural type checks and their mutation state.
pub(crate) struct ArrayTypeCheckCache {
    state: Cell<ArrayTypeCheckState>,
}

#[derive(Clone, Copy)]
struct ArrayTypeCheckState {
    primary: u32,
    secondary: u32,
    slot: u32,
    kind: ArrayTypeCheckStateKind,
}

#[derive(Clone, Copy)]
#[repr(u32)]
enum ArrayTypeCheckStateKind {
    Unknown,
    CleanOne,
    CleanTwo,
    Dirty,
}

#[expect(
    clippy::inline_always,
    reason = "these accessors run inside optimized array checks"
)]
impl ArrayTypeCheckCache {
    pub(crate) const fn new() -> Self {
        Self {
            state: Cell::new(ArrayTypeCheckState {
                primary: 0,
                secondary: 0,
                slot: 0,
                kind: ArrayTypeCheckStateKind::Unknown,
            }),
        }
    }

    #[must_use]
    #[inline(always)]
    pub(crate) const fn get(&self, id: ArrayTypeCheckId) -> ArrayTypeCheck {
        let state = self.state.get();
        match state.kind {
            ArrayTypeCheckStateKind::CleanOne if state.primary == id.get() => {
                ArrayTypeCheck::Clean(id)
            }
            ArrayTypeCheckStateKind::CleanTwo
                if state.primary == id.get() || state.secondary == id.get() =>
            {
                ArrayTypeCheck::Clean(id)
            }
            ArrayTypeCheckStateKind::Dirty if state.primary == id.get() => ArrayTypeCheck::Dirty {
                id,
                slot: state.slot,
            },
            ArrayTypeCheckStateKind::Unknown
            | ArrayTypeCheckStateKind::CleanOne
            | ArrayTypeCheckStateKind::CleanTwo
            | ArrayTypeCheckStateKind::Dirty => ArrayTypeCheck::Unknown,
        }
    }

    #[inline(always)]
    pub(crate) fn mark_checked(&self, id: ArrayTypeCheckId) {
        let mut state = self.state.get();
        match state.kind {
            ArrayTypeCheckStateKind::Unknown | ArrayTypeCheckStateKind::Dirty => {
                state.primary = id.get();
                state.secondary = 0;
                state.slot = 0;
                state.kind = ArrayTypeCheckStateKind::CleanOne;
            }
            ArrayTypeCheckStateKind::CleanOne if state.primary != id.get() => {
                state.secondary = id.get();
                state.kind = ArrayTypeCheckStateKind::CleanTwo;
            }
            ArrayTypeCheckStateKind::CleanTwo
                if state.primary != id.get() && state.secondary != id.get() =>
            {
                state.primary = id.get();
            }
            ArrayTypeCheckStateKind::CleanOne | ArrayTypeCheckStateKind::CleanTwo => {}
        }
        self.state.set(state);
    }

    #[inline(always)]
    pub(crate) fn note_mutation(&self, slot: usize) {
        let Ok(slot) = u32::try_from(slot) else {
            self.invalidate();
            return;
        };
        let mut state = self.state.get();
        match state.kind {
            ArrayTypeCheckStateKind::Unknown => return,
            ArrayTypeCheckStateKind::Dirty => {
                self.invalidate();
                return;
            }
            ArrayTypeCheckStateKind::CleanOne | ArrayTypeCheckStateKind::CleanTwo => {
                state.secondary = 0;
                state.slot = slot;
                state.kind = ArrayTypeCheckStateKind::Dirty;
            }
        }
        self.state.set(state);
    }

    #[inline(always)]
    pub(crate) fn note_removal(&self) {
        if matches!(self.state.get().kind, ArrayTypeCheckStateKind::Dirty) {
            self.invalidate();
        }
    }

    #[inline(always)]
    pub(crate) fn invalidate(&self) {
        self.state.set(Self::new().state.get());
    }
}

impl Clone for ArrayTypeCheckCache {
    fn clone(&self) -> Self {
        Self {
            state: Cell::new(self.state.get()),
        }
    }
}

#[cfg(test)]
mod tests {
    use std::mem::size_of;

    use crate::value::array::ArrayTypeCheck;
    use crate::value::array::ArrayTypeCheckCache;
    use crate::value::array::ArrayTypeCheckId;

    #[test]
    fn two_entries_fit_in_sixteen_bytes() {
        assert_eq!(size_of::<ArrayTypeCheckCache>(), 16);
    }

    #[test]
    fn ids_use_every_bit_without_colliding_with_cache_state() {
        let cache = ArrayTypeCheckCache::new();
        let low = ArrayTypeCheckId::new(1);
        let high = ArrayTypeCheckId::new(u32::MAX);

        cache.mark_checked(low);
        cache.mark_checked(high);

        assert_eq!(cache.get(low), ArrayTypeCheck::Clean(low));
        assert_eq!(cache.get(high), ArrayTypeCheck::Clean(high));
    }

    #[test]
    fn removal_preserves_clean_checks_but_discards_dirty_checks() {
        let cache = ArrayTypeCheckCache::new();
        let id = ArrayTypeCheckId::new(7);

        cache.mark_checked(id);
        cache.note_removal();
        assert_eq!(cache.get(id), ArrayTypeCheck::Clean(id));

        cache.note_mutation(3);
        cache.note_removal();
        assert_eq!(cache.get(id), ArrayTypeCheck::Unknown);
    }
}
