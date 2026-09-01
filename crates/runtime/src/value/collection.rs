//! Cached structural checks for mutable collections.

use std::cell::Cell;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(transparent)]
pub(crate) struct CollectionTypeCheckId(u32);

impl CollectionTypeCheckId {
    pub(crate) const fn new(value: u32) -> Self {
        Self(value)
    }

    const fn get(self) -> u32 {
        self.0
    }
}

/// The result of one cached structural type check.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum CollectionTypeCheck {
    Unknown,
    Clean(CollectionTypeCheckId),
    Dirty {
        id: CollectionTypeCheckId,
        slot: u32,
    },
}

/// Caches structural type checks and their mutation state.
pub(crate) struct CollectionTypeCheckCache {
    state: Cell<CollectionTypeCheckState>,
}

#[derive(Clone, Copy)]
struct CollectionTypeCheckState {
    primary: u32,
    secondary: u32,
    slot: u32,
    kind: CollectionTypeCheckStateKind,
}

#[derive(Clone, Copy)]
#[repr(u32)]
enum CollectionTypeCheckStateKind {
    Unknown,
    CleanOne,
    CleanTwo,
    Dirty,
}

#[expect(
    clippy::inline_always,
    reason = "these accessors run inside optimized collection checks"
)]
impl CollectionTypeCheckCache {
    pub(crate) const fn new() -> Self {
        Self {
            state: Cell::new(CollectionTypeCheckState {
                primary: 0,
                secondary: 0,
                slot: 0,
                kind: CollectionTypeCheckStateKind::Unknown,
            }),
        }
    }

    #[must_use]
    #[inline(always)]
    pub(crate) const fn get(&self, id: CollectionTypeCheckId) -> CollectionTypeCheck {
        let state = self.state.get();
        match state.kind {
            CollectionTypeCheckStateKind::CleanOne if state.primary == id.get() => {
                CollectionTypeCheck::Clean(id)
            }
            CollectionTypeCheckStateKind::CleanTwo
                if state.primary == id.get() || state.secondary == id.get() =>
            {
                CollectionTypeCheck::Clean(id)
            }
            CollectionTypeCheckStateKind::Dirty if state.primary == id.get() => {
                CollectionTypeCheck::Dirty {
                    id,
                    slot: state.slot,
                }
            }
            CollectionTypeCheckStateKind::Unknown
            | CollectionTypeCheckStateKind::CleanOne
            | CollectionTypeCheckStateKind::CleanTwo
            | CollectionTypeCheckStateKind::Dirty => CollectionTypeCheck::Unknown,
        }
    }

    #[inline(always)]
    pub(crate) fn mark_checked(&self, id: CollectionTypeCheckId) {
        let mut state = self.state.get();
        match state.kind {
            CollectionTypeCheckStateKind::Unknown | CollectionTypeCheckStateKind::Dirty => {
                state.primary = id.get();
                state.secondary = 0;
                state.slot = 0;
                state.kind = CollectionTypeCheckStateKind::CleanOne;
            }
            CollectionTypeCheckStateKind::CleanOne if state.primary != id.get() => {
                state.secondary = id.get();
                state.kind = CollectionTypeCheckStateKind::CleanTwo;
            }
            CollectionTypeCheckStateKind::CleanTwo
                if state.primary != id.get() && state.secondary != id.get() =>
            {
                state.primary = id.get();
            }
            CollectionTypeCheckStateKind::CleanOne | CollectionTypeCheckStateKind::CleanTwo => {}
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
            CollectionTypeCheckStateKind::Unknown => return,
            CollectionTypeCheckStateKind::Dirty => {
                self.invalidate();
                return;
            }
            CollectionTypeCheckStateKind::CleanOne | CollectionTypeCheckStateKind::CleanTwo => {
                state.secondary = 0;
                state.slot = slot;
                state.kind = CollectionTypeCheckStateKind::Dirty;
            }
        }
        self.state.set(state);
    }

    #[inline(always)]
    pub(crate) fn note_removal(&self) {
        if matches!(self.state.get().kind, CollectionTypeCheckStateKind::Dirty) {
            self.invalidate();
        }
    }

    #[inline(always)]
    pub(crate) fn invalidate(&self) {
        self.state.set(Self::new().state.get());
    }
}

impl Clone for CollectionTypeCheckCache {
    fn clone(&self) -> Self {
        Self {
            state: Cell::new(self.state.get()),
        }
    }
}

#[cfg(test)]
mod tests {
    use std::mem::size_of;

    use crate::value::collection::CollectionTypeCheck;
    use crate::value::collection::CollectionTypeCheckCache;
    use crate::value::collection::CollectionTypeCheckId;

    #[test]
    fn two_entries_fit_in_sixteen_bytes() {
        assert_eq!(size_of::<CollectionTypeCheckCache>(), 16);
    }

    #[test]
    fn ids_use_every_bit_without_colliding_with_cache_state() {
        let cache = CollectionTypeCheckCache::new();
        let low = CollectionTypeCheckId::new(1);
        let high = CollectionTypeCheckId::new(u32::MAX);

        cache.mark_checked(low);
        cache.mark_checked(high);

        assert_eq!(cache.get(low), CollectionTypeCheck::Clean(low));
        assert_eq!(cache.get(high), CollectionTypeCheck::Clean(high));
    }

    #[test]
    fn removal_preserves_clean_checks_but_discards_dirty_checks() {
        let cache = CollectionTypeCheckCache::new();
        let id = CollectionTypeCheckId::new(7);

        cache.mark_checked(id);
        cache.note_removal();
        assert_eq!(cache.get(id), CollectionTypeCheck::Clean(id));

        cache.note_mutation(3);
        cache.note_removal();
        assert_eq!(cache.get(id), CollectionTypeCheck::Unknown);
    }
}
