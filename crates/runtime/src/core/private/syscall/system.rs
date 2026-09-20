use whim_macros::whim_function;
use whim_sys::system;
use whim_value::Value;

use crate::builtin::Context;
use crate::builtin::throw::Throw;
use crate::core::private::syscall::io_error;

#[whim_function(
    "Whim\\_Private\\system_information(): ((string&!''), string, string, string, string)"
)]
pub(crate) fn system_information(cx: &mut Context<'_, '_, '_>) -> Result<Value, Throw> {
    let fields = system::information().map_err(|error| io_error(cx, error))?;
    Ok(cx.tuple(fields.map(|field| cx.string(&field))))
}

#[whim_function("Whim\\_Private\\system_uptime(): (0..)")]
pub(crate) fn system_uptime(cx: &mut Context<'_, '_, '_>) -> Result<Value, Throw> {
    let uptime = system::uptime().map_err(|error| io_error(cx, error))?;
    Ok(Value::int(i64::try_from(uptime).unwrap_or(i64::MAX)))
}

#[whim_function("Whim\\_Private\\load_averages(): (float, float, float)")]
pub(crate) fn load_averages(cx: &mut Context<'_, '_, '_>) -> Result<Value, Throw> {
    let averages = system::load_averages().map_err(|error| io_error(cx, error))?;
    Ok(cx.tuple(averages.map(Value::float)))
}

#[whim_function("Whim\\_Private\\memory_information(): ((0..), (0..))")]
pub(crate) fn memory_information(cx: &mut Context<'_, '_, '_>) -> Result<Value, Throw> {
    let memory: [u64; 2] = system::memory()
        .map_err(|error| io_error(cx, error))?
        .into();
    Ok(cx.tuple(memory.map(|bytes| Value::int(i64::try_from(bytes).unwrap_or(i64::MAX)))))
}
