//! Destructor presence for optimizations that may change observable lifetimes.

use crate::bytecode::unit::CompiledUnit;

const DESTRUCTOR: &[u8] = b"__destruct";

pub(in crate::optimizer) fn has_destructor(unit: &CompiledUnit) -> bool {
    unit.classes.iter().any(|class| {
        class
            .methods
            .iter()
            .any(|method| method.name.as_bytes() == DESTRUCTOR)
    })
}
