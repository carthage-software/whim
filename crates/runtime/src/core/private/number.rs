//! Number parsing primitives.

use whim_macros::whim_function;
use whim_value::Value;

use crate::builtin::arguments::Arguments;

#[whim_function("Whim\\Float\\try_parse(string $value): null|float", must_use)]
pub(crate) fn try_parse_float(arguments: Arguments<'_>) -> Value {
    lexical_core::parse::<f64>(arguments.bytes(0))
        .ok()
        .map_or_else(Value::null, Value::float)
}

#[whim_function("Whim\\Int\\try_parse(string $value): null|int", must_use)]
pub(crate) fn try_parse_int(arguments: Arguments<'_>) -> Value {
    lexical_core::parse::<i64>(arguments.bytes(0))
        .ok()
        .map_or_else(Value::null, Value::int)
}
