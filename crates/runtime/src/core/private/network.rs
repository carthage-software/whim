//! Cancellable hostname resolution.

use std::cell::RefCell;
use std::collections::HashSet;
use std::net::SocketAddr;
use std::net::ToSocketAddrs;
use std::str::from_utf8;
use std::sync::Arc;

use whim_macros::whim_class;
use whim_macros::whim_methods;

use crate::builtin::Context;
use crate::builtin::arguments::Arguments;
use crate::builtin::convert::state_ref;
use crate::builtin::throw::Throw;
use crate::core::private::blocking::Operation;
use crate::core::private::syscall::system_error;
use crate::value::Value;

const HOST_RESOLUTION_OPERATION: &str = "Whim\\_Private\\HostResolutionOperation";

type Shared = Operation<Vec<SocketAddr>, i32>;

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
        let family = i32::try_from(arguments.int(1))
            .map_err(|_| system_error(cx, "getaddrinfo", libc::EINVAL))?;
        if !matches!(family, 0 | libc::AF_INET | libc::AF_INET6) {
            return Err(system_error(cx, "getaddrinfo", libc::EAFNOSUPPORT));
        }

        let shared = Shared::new().map_err(|error| {
            system_error(cx, "socketpair", error.raw_os_error().unwrap_or(libc::EIO))
        })?;
        let worker = Arc::clone(&shared);
        cx.vm
            .engine
            .blocking
            .submit(Box::new(move || worker.complete(resolve(&host, family))))
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
            Some(Err(errno)) => return Err(system_error(cx, "getaddrinfo", errno)),
        };

        Ok(cx.vec(addresses.into_iter().map(|address| {
            let (family, host) = match address {
                SocketAddr::V4(address) => (libc::AF_INET, address.ip().to_string()),
                SocketAddr::V6(address) => (libc::AF_INET6, address.ip().to_string()),
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

fn resolve(host: &str, family: i32) -> Result<Vec<SocketAddr>, i32> {
    (host, 0)
        .to_socket_addrs()
        .map(|addresses| {
            let mut seen = HashSet::new();
            addresses
                .filter(|address| {
                    family == 0
                        || matches!(
                            (family, address),
                            (libc::AF_INET, SocketAddr::V4(_))
                                | (libc::AF_INET6, SocketAddr::V6(_))
                        )
                })
                .filter(|address| seen.insert(*address))
                .collect()
        })
        .map_err(|error| error.raw_os_error().unwrap_or(libc::EIO))
}
