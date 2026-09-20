//! Linking a class: its layout, its method table, and its contracts.

#![deny(clippy::nursery, clippy::pedantic)]
#![expect(
    clippy::redundant_pub_crate,
    reason = "linking rules are shared with the engine"
)]

use whim_value::atom::Atom;
use whim_value::object::ClassId;

use crate::classes::MethodEntry;
use crate::classes::PropertyInfo;
use crate::classes::RuntimeClass;

mod classes;
mod contracts;
pub(crate) mod descriptors;
mod externals;
mod generics;

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
