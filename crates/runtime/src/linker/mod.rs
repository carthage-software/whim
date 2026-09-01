//! Linking a class: its layout, its method table, and its contracts.

#![deny(clippy::nursery, clippy::pedantic)]
#![expect(
    clippy::redundant_pub_crate,
    reason = "linking rules are shared with the engine and optimizer"
)]

use crate::bytecode::unit::Visibility;
use crate::classes::MethodEntry;
use crate::classes::PropertyInfo;
use crate::classes::RuntimeClass;
use crate::value::atom::Atom;
use crate::value::object::ClassId;

mod classes;
mod contracts;
pub(crate) mod descriptors;
mod externals;
mod generics;

/// Where a declared instance property lands in its inherited slot layout.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) enum SlotPlacement {
    Inherited(u32),
    Appended,
}

/// Applies the instance property layout rule.
pub(crate) fn slot_placement(
    inherited: Option<(u32, Visibility)>,
    visibility: Visibility,
) -> SlotPlacement {
    match inherited {
        Some((slot, inherited_visibility))
            if visibility != Visibility::Private && inherited_visibility != Visibility::Private =>
        {
            SlotPlacement::Inherited(slot)
        }
        _ => SlotPlacement::Appended,
    }
}

/// How open a visibility is; an override may move up this order, never down.
const fn visibility_rank(visibility: Visibility) -> u8 {
    match visibility {
        Visibility::Private => 0,
        Visibility::Protected => 1,
        Visibility::Public => 2,
    }
}

pub(in crate::linker) const fn visibility_name(visibility: Visibility) -> &'static str {
    match visibility {
        Visibility::Private => "private",
        Visibility::Protected => "protected",
        Visibility::Public => "public",
    }
}

/// One interface's name and the members it requires, snapshotted so the
/// linker can report on them without holding the class table borrowed.
struct InterfaceRequirements {
    name: String,
    methods: Vec<(Atom, MethodEntry)>,
    properties: Vec<PropertyInfo>,
}

#[derive(Clone, Copy)]
enum Replaced<'interface> {
    Inherited,
    Required(&'interface str),
}

#[derive(Clone, Copy)]
struct OverrideCheck<'a> {
    current: &'a RuntimeClass,
    current_id: ClassId,
    method_name: &'a Atom,
    replacement: &'a MethodEntry,
    replaced: &'a MethodEntry,
    name_text: &'a str,
    source: Replaced<'a>,
    enforce_constructor: bool,
    path: &'a Atom,
}

impl Replaced<'_> {
    const fn describe(self) -> &'static str {
        match self {
            Replaced::Inherited => "the inherited method",
            Replaced::Required(_) => "the required method",
        }
    }

    fn role(self, method_text: &str) -> String {
        match self {
            Replaced::Inherited => "the method it overrides".to_string(),
            Replaced::Required(interface) => format!("{interface}::{method_text}"),
        }
    }
}
