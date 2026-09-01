//! The program arguments, the running binary, and the executing script.

use std::env;

use whim_macros::whim_function;

use crate::builtin::Context;
use crate::path::path_bytes;
use crate::value::Value;

#[whim_function("Whim\\Env\\get_arguments(): vec<string>")]
fn get_arguments(scope: &Context<'_, '_, '_>) -> Value {
    let arguments: Vec<Value> = scope
        .vm
        .engine
        .arguments
        .iter()
        .map(|argument| scope.string(argument))
        .collect();

    scope.vec(arguments)
}

#[whim_function("Whim\\Env\\current_binary(): string")]
fn current_binary(scope: &Context<'_, '_, '_>) -> Value {
    match env::current_exe() {
        Ok(binary) => scope.string(&path_bytes(&binary)),
        Err(_) => scope.string(b""),
    }
}

#[whim_function("Whim\\Env\\current_script(): null|string")]
fn current_script(scope: &Context<'_, '_, '_>) -> Value {
    match scope.vm.engine.script.as_deref() {
        Some(script) => scope.string(script),
        None => Value::null(),
    }
}
