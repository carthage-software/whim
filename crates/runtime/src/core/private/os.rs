use std::cell::RefCell;
use std::ffi::CString;
use std::sync::Arc;

use whim_macros::{whim_class, whim_methods};
use whim_sys::accounts::{self, Group, Identity, User};
use whim_sys::operation::Operation;
use whim_value::Value;

use crate::builtin::Context;
use crate::builtin::arguments::Arguments;
use crate::builtin::convert::state_ref;
use crate::builtin::throw::Throw;
use crate::core::private::syscall::{io_error, system_error};

enum OSResult {
    User(Option<User>),
    Group(Option<Group>),
    Groups(Vec<Group>),
}
type Shared = Operation<OSResult, whim_sys::Error>;

#[whim_class("Whim\\_Private\\OSOperation", final)]
#[derive(Default)]
pub(crate) struct OSOperation {
    shared: RefCell<Option<Arc<Shared>>>,
}

default_built_in_state!(OSOperation);

#[whim_methods]
impl OSOperation {
    #[whim_method("__construct(): void", visibility = "private")]
    const fn construct() {}

    #[whim_method(
        "user(((string&!'')|(0..)) $identity): Whim\\_Private\\OSOperation",
        static,
        must_use
    )]
    fn user(cx: &mut Context<'_, '_, '_>, arguments: Arguments<'_>) -> Result<Value, Throw> {
        accounts::require_support().map_err(|error| io_error(cx, error))?;
        let key = identity(cx, arguments, "getpwnam_r")?;
        start(cx, move || accounts::user(&key).map(OSResult::User))
    }

    #[whim_method(
        "group(((string&!'')|(0..)) $identity): Whim\\_Private\\OSOperation",
        static,
        must_use
    )]
    fn group(cx: &mut Context<'_, '_, '_>, arguments: Arguments<'_>) -> Result<Value, Throw> {
        accounts::require_support().map_err(|error| io_error(cx, error))?;
        let key = identity(cx, arguments, "getgrnam_r")?;
        start(cx, move || accounts::group(&key).map(OSResult::Group))
    }

    #[whim_method(
        "groupsForUser((string&!'') $name, (0..) $primaryGroup): Whim\\_Private\\OSOperation",
        static,
        must_use
    )]
    fn groups_for_user(
        cx: &mut Context<'_, '_, '_>,
        arguments: Arguments<'_>,
    ) -> Result<Value, Throw> {
        accounts::require_support().map_err(|error| io_error(cx, error))?;
        let name = CString::new(arguments.bytes(0))
            .map_err(|_| system_error(cx, "getgrouplist", libc::EINVAL))?;
        let primary_group = u32::try_from(arguments.int(1))
            .map_err(|_| system_error(cx, "getgrouplist", libc::EINVAL))?;
        start(cx, move || {
            accounts::groups_for_user(&name, primary_group).map(OSResult::Groups)
        })
    }

    #[whim_method("wait(): void")]
    fn wait(cx: &mut Context<'_, '_, '_>) -> Result<Value, Throw> {
        let shared = operation(cx)?;
        while !shared.is_complete() {
            cx.io_wait_until_readable(shared.descriptor())?;
            shared.drain();
        }
        shared.drain();
        Ok(Value::null())
    }

    #[whim_method(
        "takeUser(): null|((string&!''), (0..), (0..), string, string)",
        must_use
    )]
    fn take_user(cx: &mut Context<'_, '_, '_>) -> Result<Value, Throw> {
        match take(cx)? {
            Some(OSResult::User(Some(record))) => Ok(cx.tuple([
                cx.string(&record.name),
                Value::int(i64::from(record.id)),
                Value::int(i64::from(record.primary_group)),
                cx.string(&record.home_directory),
                cx.string(&record.shell),
            ])),
            Some(OSResult::User(None)) | None => Ok(Value::null()),
            _ => Err(cx.type_error("the OS operation does not contain a user")),
        }
    }

    #[whim_method("takeGroup(): null|((string&!''), (0..), vec<(string&!'')>)", must_use)]
    fn take_group(cx: &mut Context<'_, '_, '_>) -> Result<Value, Throw> {
        match take(cx)? {
            Some(OSResult::Group(Some(record))) => Ok(group_value(cx, record)),
            Some(OSResult::Group(None)) | None => Ok(Value::null()),
            _ => Err(cx.type_error("the OS operation does not contain a group")),
        }
    }

    #[whim_method(
        "takeGroups(): null|vec<((string&!''), (0..), vec<(string&!'')>)>",
        must_use
    )]
    fn take_groups(cx: &mut Context<'_, '_, '_>) -> Result<Value, Throw> {
        match take(cx)? {
            Some(OSResult::Groups(groups)) => {
                Ok(cx.vec(groups.into_iter().map(|group| group_value(cx, group))))
            }
            None => Ok(Value::null()),
            _ => Err(cx.type_error("the OS operation does not contain groups")),
        }
    }

    #[whim_method("cancel(): void", no_track_caller, no_trace_boundary)]
    fn cancel(cx: &mut Context<'_, '_, '_>) -> Result<Value, Throw> {
        operation(cx)?.cancel();
        Ok(Value::null())
    }
}

fn identity(
    cx: &mut Context<'_, '_, '_>,
    arguments: Arguments<'_>,
    call: &'static str,
) -> Result<Identity, Throw> {
    let value = arguments.local(0);
    if let Some(id) = value.as_int() {
        return u32::try_from(id)
            .map(Identity::Id)
            .map_err(|_| system_error(cx, call, libc::EINVAL));
    }
    let Some(name) = value.as_string_bytes() else {
        return Err(cx.type_error("an account identity must be a string or integer"));
    };
    CString::new(name)
        .map(Identity::Name)
        .map_err(|_| system_error(cx, call, libc::EINVAL))
}

fn group_value(cx: &Context<'_, '_, '_>, record: Group) -> Value {
    let members = cx.vec(record.members.into_iter().map(|name| cx.string(&name)));
    cx.tuple([
        cx.string(&record.name),
        Value::int(i64::from(record.id)),
        members,
    ])
}

fn operation(cx: &mut Context<'_, '_, '_>) -> Result<Arc<Shared>, Throw> {
    let shared = cx.state::<OSOperation>()?.shared.borrow().clone();
    shared.ok_or_else(|| cx.type_error("the OS operation is not initialized"))
}

fn take(cx: &mut Context<'_, '_, '_>) -> Result<Option<OSResult>, Throw> {
    match operation(cx)?.take() {
        Some(Ok(result)) => Ok(result),
        Some(Err(error)) => Err(io_error(cx, error)),
        None => Ok(None),
    }
}

fn start(
    cx: &mut Context<'_, '_, '_>,
    operation: impl FnOnce() -> whim_sys::Result<OSResult> + Send + 'static,
) -> Result<Value, Throw> {
    let shared =
        Shared::new().map_err(|error| io_error(cx, whim_sys::Error::new("event", error)))?;
    let worker = Arc::clone(&shared);
    cx.vm
        .engine
        .blocking
        .submit(Box::new(move || worker.complete(operation())))
        .map_err(|error| io_error(cx, whim_sys::Error::new("thread", error)))?;
    let object = cx.new_built_in_instance("Whim\\_Private\\OSOperation")?;
    let Some(state) = state_ref::<OSOperation>(&object) else {
        return Err(cx.type_error("the OS operation has no built-in state"));
    };
    *state.shared.borrow_mut() = Some(shared);
    Ok(object)
}
