//! Reading and mutating the process environment variables.

use std::env;
use std::path::Path;

use whim_macros::whim_function;
use whim_sys::path::os_string_from_bytes;
use whim_sys::path::path_bytes;
use whim_value::Value;

use crate::builtin::Context;
use crate::builtin::arguments::Arguments;

#[whim_function("Whim\\Env\\get_variable(string $name): null|string")]
fn get_variable(scope: &Context<'_, '_, '_>, arguments: Arguments<'_>) -> Value {
    let name = arguments.bytes(0);
    if name.is_empty() || name.contains(&b'=') || name.contains(&b'\0') {
        return Value::null();
    }

    let Ok(name) = os_string_from_bytes(name) else {
        return Value::null();
    };

    match env::var_os(name) {
        Some(value) => scope.string(&path_bytes(Path::new(&value))),
        None => Value::null(),
    }
}

#[whim_function("Whim\\Env\\set_variable(string $name, string $value): bool")]
fn set_variable(arguments: Arguments<'_>) -> Value {
    let name = arguments.bytes(0);
    let value = arguments.bytes(1);

    if name.is_empty() || name.contains(&b'=') || name.contains(&b'\0') {
        return Value::bool(false);
    }

    if value.contains(&b'\0') {
        return Value::bool(false);
    }

    let (Ok(name), Ok(value)) = (os_string_from_bytes(name), os_string_from_bytes(value)) else {
        return Value::bool(false);
    };

    // SAFETY: Whim serializes user code, and its native workers do not access the environment.
    unsafe { env::set_var(name, value) };
    Value::bool(true)
}

#[whim_function("Whim\\Env\\remove_variable(string $name): bool")]
fn remove_variable(arguments: Arguments<'_>) -> Value {
    let name = arguments.bytes(0);

    if name.is_empty() || name.contains(&b'=') || name.contains(&b'\0') {
        return Value::bool(false);
    }

    let Ok(name) = os_string_from_bytes(name) else {
        return Value::bool(false);
    };

    let existed = env::var_os(&name).is_some();
    // SAFETY: Whim serializes user code, and its native workers do not access the environment.
    unsafe { env::remove_var(name) };
    Value::bool(existed)
}

#[whim_function("Whim\\Env\\get_variables(): dict<string, string>")]
fn get_variables(scope: &Context<'_, '_, '_>) -> Value {
    let pairs: Vec<(Value, Value)> = env::vars_os()
        .map(|(name, value)| {
            (
                scope.string(&path_bytes(Path::new(&name))),
                scope.string(&path_bytes(Path::new(&value))),
            )
        })
        .collect();

    scope.dict(pairs)
}
