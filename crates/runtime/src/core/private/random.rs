//! Unstable random primitives exposed to the Whim standard library.

use whim_macros::whim_function;

use crate::builtin::Context;
use crate::builtin::arguments::Arguments;
use crate::value::Value;

#[whim_function("Whim\\_Private\\random_bytes(int $length): null|string")]
pub(crate) fn random_bytes(scope: &Context<'_, '_, '_>, arguments: Arguments<'_>) -> Value {
    let length = arguments.int(0);
    let Ok(length) = usize::try_from(length) else {
        return Value::null();
    };

    let mut bytes = vec![0_u8; length];
    if getrandom::fill(&mut bytes).is_err() {
        return Value::null();
    }

    scope.string(&bytes)
}
