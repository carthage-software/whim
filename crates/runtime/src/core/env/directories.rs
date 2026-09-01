//! The current, home, and temporary directories of the running process.

use std::env;

use whim_macros::whim_function;

use crate::builtin::Context;
use crate::builtin::arguments::Arguments;
use crate::path::path_bytes;
use crate::path::path_from_bytes;
use crate::value::Value;

#[whim_function("Whim\\Env\\current_directory(): null|string")]
fn current_directory(scope: &Context<'_, '_, '_>) -> Value {
    match env::current_dir() {
        Ok(directory) => scope.string(&path_bytes(&directory)),
        Err(_) => Value::null(),
    }
}

#[whim_function("Whim\\Env\\set_current_directory(string $directory): bool")]
fn set_current_directory(arguments: Arguments<'_>) -> Value {
    let directory = arguments.bytes(0);
    let changed = env::set_current_dir(path_from_bytes(directory)).is_ok();
    Value::bool(changed)
}

#[whim_function("Whim\\Env\\home_directory(): null|string")]
fn home_directory(scope: &Context<'_, '_, '_>) -> Value {
    match env::home_dir() {
        Some(directory) => scope.string(&path_bytes(&directory)),
        None => Value::null(),
    }
}

#[whim_function("Whim\\Env\\temporary_directory(): null|string")]
fn temporary_directory(scope: &Context<'_, '_, '_>) -> Value {
    let directory = env::temp_dir();
    if directory.as_os_str().is_empty() {
        return Value::null();
    }

    scope.string(&path_bytes(&directory))
}
