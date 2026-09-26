//! Number parsing primitives.

use whim_macros::whim_function;
use whim_value::Value;

use crate::builtin::Context;
use crate::builtin::arguments::Arguments;
use crate::builtin::throw::Throw;

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

#[whim_function("Whim\\UInt\\try_parse(string $value): null|uint", must_use)]
pub(crate) fn try_parse_uint(arguments: Arguments<'_>) -> Value {
    lexical_core::parse::<u64>(arguments.bytes(0))
        .ok()
        .map_or_else(Value::null, Value::uint)
}

#[whim_function("Whim\\UInt\\div(uint $numerator, uint $denominator): uint", must_use)]
pub(crate) fn div_uint(
    context: &mut Context<'_, '_, '_>,
    arguments: Arguments<'_>,
) -> Result<Value, Throw> {
    arguments
        .uint(0)
        .checked_div(arguments.uint(1))
        .map(Value::uint)
        .ok_or_else(|| {
            let class = context.vm.intern(b"Whim\\Unwind\\DivisionByZeroError");
            context.vm.throw(class, "division by zero", 0)
        })
}
