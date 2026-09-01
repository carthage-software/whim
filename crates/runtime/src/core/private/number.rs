//! Number parsing primitives.

use whim_macros::whim_function;

use crate::builtin::arguments::Arguments;
use crate::value::Value;

#[whim_function("Whim\\_Private\\float_try_parse(string $value): null|float", must_use)]
pub(crate) fn try_parse_float(arguments: Arguments<'_>) -> Value {
    lexical_core::parse::<f64>(arguments.bytes(0))
        .ok()
        .map_or_else(Value::null, Value::float)
}

#[whim_function("Whim\\_Private\\int_try_parse(string $value): null|int", must_use)]
pub(crate) fn try_parse_int(arguments: Arguments<'_>) -> Value {
    lexical_core::parse::<i64>(arguments.bytes(0))
        .ok()
        .map_or_else(Value::null, Value::int)
}
