//! Cancellable hostname resolution.

use std::cell::RefCell;
use std::net::IpAddr;
use std::str::from_utf8;
use std::sync::Arc;

use whim_macros::whim_class;
use whim_macros::whim_methods;
use whim_sys::constants;
use whim_sys::dns::{self, Family};
use whim_sys::operation::Operation;
use whim_value::Value;

use crate::builtin::Context;
use crate::builtin::arguments::Arguments;
use crate::builtin::convert::state_ref;
use crate::builtin::throw::Throw;
use crate::core::private::syscall::io_error;
use crate::core::private::syscall::system_error;

const HOST_RESOLUTION_OPERATION: &str = "Whim\\_Private\\HostResolutionOperation";

type Shared = Operation<Vec<IpAddr>, whim_sys::Error>;

#[whim_class("Whim\\_Private\\HostResolutionOperation", final)]
#[derive(Default)]
pub(crate) struct HostResolutionOperation {
    shared: RefCell<Option<Arc<Shared>>>,
}

default_built_in_state!(HostResolutionOperation);

#[whim_methods]
impl HostResolutionOperation {
    #[whim_method("__construct(): void", visibility = "private")]
    const fn construct() {}

    #[whim_method(
        "start((string&!'') $host, int $family): Whim\\_Private\\HostResolutionOperation",
        static,
        must_use
    )]
    fn start(cx: &mut Context<'_, '_, '_>, arguments: Arguments<'_>) -> Result<Value, Throw> {
        let host = from_utf8(arguments.bytes(0))
            .map_err(|_| system_error(cx, "getaddrinfo", libc::EINVAL))?
            .to_owned();
        let family = Family::from_raw(arguments.int(1)).map_err(|error| io_error(cx, error))?;

        let shared = Shared::new().map_err(|error| {
            system_error(cx, "socketpair", error.raw_os_error().unwrap_or(libc::EIO))
        })?;
        let worker = Arc::clone(&shared);
        cx.vm
            .engine
            .blocking
            .submit(Box::new(move || {
                worker.complete(dns::resolve(&host, family));
            }))
            .map_err(|error| {
                system_error(cx, "thread", error.raw_os_error().unwrap_or(libc::EAGAIN))
            })?;

        let object = cx.new_built_in_instance(HOST_RESOLUTION_OPERATION)?;
        let Some(state) = state_ref::<Self>(&object) else {
            return Err(cx.type_error("the host resolution operation has no built-in state"));
        };

        *state.shared.borrow_mut() = Some(shared);
        Ok(object)
    }

    #[whim_method("wait(): void")]
    fn wait(cx: &mut Context<'_, '_, '_>) -> Result<Value, Throw> {
        let shared = operation(cx)?;
        while !shared.is_complete() {
            if cx.vm.loop_has_tasks() {
                cx.io_wait_until_readable(shared.descriptor())?;
            } else {
                shared.wait().map_err(|error| {
                    system_error(cx, "poll", error.raw_os_error().unwrap_or(libc::EIO))
                })?;
            }
            shared.drain();
        }

        shared.drain();
        Ok(Value::null())
    }

    #[whim_method("take(): null|vec<(int, (string&!''))>", must_use)]
    fn take(cx: &mut Context<'_, '_, '_>) -> Result<Value, Throw> {
        let shared = operation(cx)?;
        let addresses = match shared.take() {
            Some(Ok(Some(addresses))) => addresses,
            Some(Ok(None)) | None => return Ok(Value::null()),
            Some(Err(error)) => return Err(io_error(cx, error)),
        };

        Ok(cx.vec(addresses.into_iter().map(|address| {
            let (family, host) = match address {
                IpAddr::V4(address) => (constants::AF_INET, address.to_string()),
                IpAddr::V6(address) => (constants::AF_INET6, address.to_string()),
            };
            let family = Value::int(i64::from(family));
            let host = cx.string(host.as_bytes());
            cx.tuple([family, host])
        })))
    }

    #[whim_method("cancel(): void", no_track_caller, no_trace_boundary)]
    fn cancel(cx: &mut Context<'_, '_, '_>) -> Result<Value, Throw> {
        operation(cx)?.cancel();
        Ok(Value::null())
    }
}

fn operation(cx: &mut Context<'_, '_, '_>) -> Result<Arc<Shared>, Throw> {
    let operation = cx
        .state::<HostResolutionOperation>()?
        .shared
        .borrow()
        .clone();
    operation.ok_or_else(|| cx.type_error("the host resolution operation is not initialized"))
}
