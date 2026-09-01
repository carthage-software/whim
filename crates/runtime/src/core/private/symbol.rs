//! The symbol boundary: kind numbers, the one autoload callback, and the probe.

use core::str;

use whim_macros::whim_constant;
use whim_macros::whim_function;

use crate::builtin::Context;
use crate::builtin::arguments::Arguments;
use crate::builtin::throw::Throw;
use crate::core::symbols::strip_leading_backslash;
use crate::symbols::SymbolKind;
use crate::value::Value;
use crate::value::atom::Atom;

#[whim_constant("Whim\\_Private\\SYMBOL_KIND_CLASS", "int")]
pub(crate) const SYMBOL_KIND_CLASS: i64 = SymbolKind::Class as i64;

#[whim_constant("Whim\\_Private\\SYMBOL_KIND_INTERFACE", "int")]
pub(crate) const SYMBOL_KIND_INTERFACE: i64 = SymbolKind::Interface as i64;

#[whim_constant("Whim\\_Private\\SYMBOL_KIND_ENUM", "int")]
pub(crate) const SYMBOL_KIND_ENUM: i64 = SymbolKind::Enum as i64;

#[whim_constant("Whim\\_Private\\SYMBOL_KIND_TYPE_ALIAS", "int")]
pub(crate) const SYMBOL_KIND_TYPE_ALIAS: i64 = SymbolKind::TypeAlias as i64;

#[whim_constant("Whim\\_Private\\SYMBOL_KIND_NEWTYPE", "int")]
pub(crate) const SYMBOL_KIND_NEWTYPE: i64 = SymbolKind::Newtype as i64;

#[whim_constant("Whim\\_Private\\SYMBOL_KIND_FUNCTION", "int")]
pub(crate) const SYMBOL_KIND_FUNCTION: i64 = SymbolKind::Function as i64;

#[whim_constant("Whim\\_Private\\SYMBOL_KIND_CONSTANT", "int")]
pub(crate) const SYMBOL_KIND_CONSTANT: i64 = SymbolKind::Constant as i64;

#[inline]
const fn symbol_kind_from_number(number: i64) -> Option<SymbolKind> {
    match number {
        SYMBOL_KIND_CLASS => Some(SymbolKind::Class),
        SYMBOL_KIND_INTERFACE => Some(SymbolKind::Interface),
        SYMBOL_KIND_ENUM => Some(SymbolKind::Enum),
        SYMBOL_KIND_TYPE_ALIAS => Some(SymbolKind::TypeAlias),
        SYMBOL_KIND_NEWTYPE => Some(SymbolKind::Newtype),
        SYMBOL_KIND_FUNCTION => Some(SymbolKind::Function),
        SYMBOL_KIND_CONSTANT => Some(SymbolKind::Constant),
        _ => None,
    }
}

#[whim_function(
    "Whim\\_Private\\register_symbol_autoloader((fn(int, string): void) $autoloader): void"
)]
fn register_symbol_autoloader(
    context: &mut Context<'_, '_, '_>,
    arguments: Arguments<'_>,
) -> Result<Value, Throw> {
    let autoloader = arguments.local(0);
    if !context.vm.install_autoloader(autoloader) {
        let class = context.vm.intern(b"Whim\\Unwind\\Error");
        return Err(context.vm.throw(
            class,
            "an autoloader callback is already registered, and the engine keeps only one",
            0,
        ));
    }

    Ok(Value::null())
}

#[whim_function("Whim\\_Private\\run_symbol_autoload(int $kind, string $name): void")]
fn run_symbol_autoload(
    context: &mut Context<'_, '_, '_>,
    arguments: Arguments<'_>,
) -> Result<Value, Throw> {
    let kind = arguments.int(0);
    let Some(kind) = symbol_kind_from_number(kind) else {
        let class = context.vm.intern(b"Whim\\Unwind\\TypeError");
        return Err(context
            .vm
            .throw(class, "argument 1 ($kind) must name a symbol kind", 0));
    };

    let name = symbol_name(
        context,
        arguments.bytes(1),
        "argument 2 ($name) must be a valid UTF-8 string",
    )?;

    context.vm.run_autoload(kind, name)?;

    Ok(Value::null())
}

#[whim_function("Whim\\_Private\\get_symbol_kind(string $name): null|int")]
fn get_symbol_kind(
    context: &mut Context<'_, '_, '_>,
    arguments: Arguments<'_>,
) -> Result<Value, Throw> {
    let name = symbol_name(
        context,
        arguments.bytes(0),
        "argument 1 ($name) must be a valid UTF-8 string",
    )?;

    let Some(kind) = context
        .vm
        .symbol_kind_of(strip_leading_backslash(context.vm.heap(), name))
    else {
        return Ok(Value::null());
    };

    Ok(Value::int(kind as i64))
}

fn symbol_name(
    context: &mut Context<'_, '_, '_>,
    bytes: &[u8],
    error: &'static str,
) -> Result<Atom, Throw> {
    if str::from_utf8(bytes).is_err() {
        let class = context.vm.intern(b"Whim\\Unwind\\TypeError");
        return Err(context.vm.throw(class, error, 0));
    }

    Ok(context.vm.intern(bytes))
}
