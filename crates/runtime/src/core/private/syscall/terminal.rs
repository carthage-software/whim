use crate::builtin::Context;
use crate::builtin::arguments::Arguments;
use crate::builtin::throw::Throw;
use crate::core::private::syscall::with_descriptor;
use crate::value::Value;
use whim_macros::whim_function;
use whim_sys::terminal;

#[whim_function("Whim\\_Private\\is_terminal(Whim\\OS\\FileDescriptor $descriptor): bool")]
pub(crate) fn is_terminal<'call>(
    cx: &mut Context<'call, '_, '_>,
    arguments: Arguments<'call>,
) -> Result<Value, Throw> {
    with_descriptor(cx, &arguments.local(0), "isatty", |descriptor| {
        Ok(terminal::is_terminal(descriptor))
    })
    .map(Value::bool)
}

#[whim_function(
    "Whim\\_Private\\terminal_path(Whim\\OS\\FileDescriptor $descriptor): null|(string&!'')"
)]
pub(crate) fn terminal_path<'call>(
    cx: &mut Context<'call, '_, '_>,
    arguments: Arguments<'call>,
) -> Result<Value, Throw> {
    let path = with_descriptor(cx, &arguments.local(0), "ttyname_r", terminal::path)?;
    Ok(path.map_or_else(Value::null, |path| cx.string(&path)))
}

#[whim_function(
    "Whim\\_Private\\terminal_size(Whim\\OS\\FileDescriptor $descriptor): null|((1..), (1..))"
)]
pub(crate) fn terminal_size<'call>(
    cx: &mut Context<'call, '_, '_>,
    arguments: Arguments<'call>,
) -> Result<Value, Throw> {
    let size = with_descriptor(cx, &arguments.local(0), "ioctl", terminal::size)?;
    Ok(size.map_or_else(Value::null, |(columns, rows)| {
        cx.tuple([Value::int(i64::from(columns)), Value::int(i64::from(rows))])
    }))
}
