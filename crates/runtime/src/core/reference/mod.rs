//! `Whim\Reference\Weak` and `Whim\Reference\WeakMap` over the runtime's
//! weak types, as traced built-in state classes.

#![expect(
    clippy::option_if_let_else,
    reason = "explicit weak-reference misses are clearer than closure adapters"
)]

pub(crate) mod weak;
pub(crate) mod weak_map;
