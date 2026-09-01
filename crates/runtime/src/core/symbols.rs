//! Symbol operations required by the Rust-backed core.

use crate::symbols::SymbolKind;
use crate::value::atom::Atom;
use crate::value::heap::Heap;

pub(crate) fn strip_leading_backslash(heap: &Heap, name: Atom) -> Atom {
    let bytes = name.as_bytes();
    match bytes.first() {
        Some(b'\\') => heap.intern(&bytes[1..]),
        _ => name,
    }
}

/// The kind atoms, in the documented `exists` probe order.
pub(crate) const CHECKED_KIND_ORDER: [SymbolKind; 7] = [
    SymbolKind::Class,
    SymbolKind::Interface,
    SymbolKind::Enum,
    SymbolKind::TypeAlias,
    SymbolKind::Newtype,
    SymbolKind::Function,
    SymbolKind::Constant,
];

pub(crate) const CLASS_LIKE_KIND_ORDER: [SymbolKind; 3] =
    [SymbolKind::Class, SymbolKind::Interface, SymbolKind::Enum];
