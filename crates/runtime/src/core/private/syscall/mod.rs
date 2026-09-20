use std::cell::{Cell, RefCell};

use whim_macros::{whim_class, whim_methods};
pub(crate) use whim_sys::{Descriptor, StandardStream};

use crate::builtin::Context;
use crate::builtin::arguments::Arguments;
use crate::builtin::convert::state_ref;
use crate::builtin::throw::Throw;
use crate::unwrap_option_invariant;
use crate::value::Value;

pub(crate) mod constants;
pub(crate) mod descriptor;
pub(crate) mod message;
pub(crate) mod path;
pub(crate) mod process;
pub(crate) mod socket;
pub(crate) mod system;
pub(crate) mod terminal;

#[whim_class("Whim\\_Private\\SystemError", final)]
#[whim_extends("Whim\\Unwind\\Error")]
pub(crate) struct SystemError;

#[whim_methods]
impl SystemError {
    #[whim_method("__construct(): void", visibility = "private")]
    const fn construct() {}

    #[whim_method("errno(): int")]
    fn errno(cx: &mut Context<'_, '_, '_>) -> Result<Value, Throw> {
        let receiver = cx.receiver();
        cx.get_property(&receiver, "code")
    }

    #[whim_method("call(): (string&!'')")]
    fn call(cx: &mut Context<'_, '_, '_>) -> Result<Value, Throw> {
        let receiver = cx.receiver();
        cx.get_property(&receiver, "message")
    }
}

#[whim_class("Whim\\OS\\FileDescriptor", final)]
#[derive(Default)]
pub(crate) struct FileDescriptor {
    descriptor: RefCell<Option<Descriptor>>,
    number: Cell<i64>,
}

default_built_in_state!(FileDescriptor);

#[whim_methods]
impl FileDescriptor {
    #[whim_method("__construct(): void", visibility = "private")]
    const fn construct() {}

    #[whim_method("duplicate(int $number): Whim\\OS\\FileDescriptor", static)]
    fn duplicate<'call>(
        cx: &mut Context<'call, '_, '_>,
        arguments: Arguments<'call>,
    ) -> Result<Value, Throw> {
        let descriptor =
            Descriptor::duplicate(arguments.int(0)).map_err(|error| io_error(cx, error))?;
        build_file_descriptor(cx, descriptor)
    }

    #[whim_method("toInt(): (0..)")]
    fn to_int(cx: &Context<'_, '_, '_>) -> Value {
        let receiver = cx.receiver();
        Value::int(descriptor_state(&receiver).number.get().max(0))
    }

    #[whim_method("isClosed(): bool")]
    fn is_closed(cx: &Context<'_, '_, '_>) -> Value {
        let receiver = cx.receiver();
        Value::bool(descriptor_state(&receiver).descriptor.borrow().is_none())
    }

    #[whim_method("close(): void")]
    fn close(cx: &Context<'_, '_, '_>) -> Value {
        let receiver = cx.receiver();
        descriptor_state(&receiver).descriptor.borrow_mut().take();
        Value::null()
    }
}

fn descriptor_state(value: &Value) -> &FileDescriptor {
    // SAFETY: callers pass a validated FileDescriptor instance.
    unsafe {
        unwrap_option_invariant(
            state_ref::<FileDescriptor>(value),
            "a file descriptor has built-in state",
        )
    }
}

pub(crate) fn system_error(cx: &mut Context<'_, '_, '_>, call: &'static str, errno: i32) -> Throw {
    let class = cx.vm.intern(b"Whim\\_Private\\SystemError");
    cx.vm.throw(class, call, i64::from(errno))
}

pub(crate) fn io_error(cx: &mut Context<'_, '_, '_>, error: whim_sys::Error) -> Throw {
    match error {
        whim_sys::Error::Unsupported(operation) => {
            let class = cx.vm.intern(b"Whim\\Unwind\\UnsupportedPlatformException");
            cx.vm.throw(
                class,
                &format!("{operation} is not supported on this platform"),
                0,
            )
        }
        error @ whim_sys::Error::Io { call, .. } => system_error(cx, call, error.errno()),
    }
}

pub(crate) fn build_file_descriptor(
    cx: &mut Context<'_, '_, '_>,
    descriptor: Descriptor,
) -> Result<Value, Throw> {
    let number = descriptor.number();
    let object = cx.new_built_in_instance("Whim\\OS\\FileDescriptor")?;
    let state = descriptor_state(&object);
    state.number.set(number);
    *state.descriptor.borrow_mut() = Some(descriptor);
    Ok(object)
}

pub(crate) fn with_descriptor<T>(
    cx: &mut Context<'_, '_, '_>,
    value: &Value,
    call: &'static str,
    operation: impl FnOnce(&Descriptor) -> whim_sys::Result<T>,
) -> Result<T, Throw> {
    let descriptor = descriptor_state(value).descriptor.borrow();
    let Some(descriptor) = descriptor.as_ref() else {
        return Err(io_error(cx, whim_sys::Error::bad_descriptor(call)));
    };
    operation(descriptor).map_err(|error| io_error(cx, error))
}
