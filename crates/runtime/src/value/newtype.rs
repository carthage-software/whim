//! Nominal metadata carried by newtype values.

use crate::value::object::TypeEnvironmentId;

/// The index of a declared newtype in the engine's append-only store.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) struct NewtypeId(pub u32);

/// The index of one interned newtype chain in the engine.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) struct NewtypeValueId(pub u32);

/// The nominal layers attached to one runtime value.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) struct NewtypeValueDescriptor {
    pub(crate) declaration: NewtypeId,
    pub(crate) type_environment: TypeEnvironmentId,
    pub(crate) parent: Option<NewtypeValueId>,
}
