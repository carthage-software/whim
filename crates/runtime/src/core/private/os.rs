//! Cancellable operating-system directory lookups.

use std::cell::RefCell;
use std::collections::HashSet;
use std::ffi::CStr;
use std::ffi::CString;
use std::mem::size_of;
use std::mem::zeroed;
use std::ptr::null_mut;
use std::ptr::read_unaligned;
use std::sync::Arc;

use whim_macros::whim_class;
use whim_macros::whim_methods;

use crate::builtin::Context;
use crate::builtin::arguments::Arguments;
use crate::builtin::convert::state_ref;
use crate::builtin::throw::Throw;
use crate::core::private::blocking::Operation;
use crate::core::private::syscall::system_error;
use crate::unreachable_invariant;
use crate::value::Value;

const OS_OPERATION: &str = "Whim\\_Private\\OSOperation";
const FALLBACK_RECORD_BUFFER_SIZE: usize = 16 * 1024;
const MAXIMUM_RECORD_BUFFER_SIZE: usize = 16 * 1024 * 1024;
const INITIAL_GROUP_CAPACITY: usize = 16;
const MAXIMUM_GROUP_COUNT: usize = 1_048_576;

#[cfg(target_os = "macos")]
type GroupListId = libc::c_int;

#[cfg(target_os = "linux")]
type GroupListId = libc::gid_t;

struct OperationError {
    call: &'static str,
    errno: i32,
}

struct UserRecord {
    name: Vec<u8>,
    id: libc::uid_t,
    primary_group: libc::gid_t,
    home_directory: Vec<u8>,
    shell: Vec<u8>,
}

struct GroupRecord {
    name: Vec<u8>,
    id: libc::gid_t,
    members: Vec<Vec<u8>>,
}

enum UserKey {
    Name(CString),
    Id(libc::uid_t),
}

enum GroupKey {
    Name(CString),
    Id(libc::gid_t),
}

enum OSResult {
    User(Option<UserRecord>),
    Group(Option<GroupRecord>),
    Groups(Vec<GroupRecord>),
}

type Shared = Operation<OSResult, OperationError>;

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
        let key = user_key(cx, arguments)?;
        start(cx, move || lookup_user(&key).map(OSResult::User))
    }

    #[whim_method(
        "group(((string&!'')|(0..)) $identity): Whim\\_Private\\OSOperation",
        static,
        must_use
    )]
    fn group(cx: &mut Context<'_, '_, '_>, arguments: Arguments<'_>) -> Result<Value, Throw> {
        let key = group_key(cx, arguments)?;
        start(cx, move || lookup_group(&key).map(OSResult::Group))
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
        let name = c_string(arguments.bytes(0), "getgrouplist")
            .map_err(|error| system_error(cx, error.call, error.errno))?;
        let primary_group = libc::gid_t::try_from(arguments.int(1))
            .map_err(|_| system_error(cx, "getgrouplist", libc::EINVAL))?;
        start(cx, move || {
            lookup_groups_for_user(&name, primary_group).map(OSResult::Groups)
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
            Some(OSResult::User(Some(record))) => Ok(user_value(cx, &record)),
            Some(OSResult::User(None)) | None => Ok(Value::null()),
            Some(_) => Err(cx.type_error("the OS operation does not contain a user")),
        }
    }

    #[whim_method("takeGroup(): null|((string&!''), (0..), vec<(string&!'')>)", must_use)]
    fn take_group(cx: &mut Context<'_, '_, '_>) -> Result<Value, Throw> {
        match take(cx)? {
            Some(OSResult::Group(Some(record))) => Ok(group_value(cx, record)),
            Some(OSResult::Group(None)) | None => Ok(Value::null()),
            Some(_) => Err(cx.type_error("the OS operation does not contain a group")),
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
            Some(_) => Err(cx.type_error("the OS operation does not contain groups")),
        }
    }

    #[whim_method("cancel(): void", no_track_caller, no_trace_boundary)]
    fn cancel(cx: &mut Context<'_, '_, '_>) -> Result<Value, Throw> {
        operation(cx)?.cancel();
        Ok(Value::null())
    }
}

fn operation(cx: &mut Context<'_, '_, '_>) -> Result<Arc<Shared>, Throw> {
    let operation = cx.state::<OSOperation>()?.shared.borrow().clone();
    operation.ok_or_else(|| cx.type_error("the OS operation is not initialized"))
}

fn take(cx: &mut Context<'_, '_, '_>) -> Result<Option<OSResult>, Throw> {
    let shared = operation(cx)?;
    match shared.take() {
        Some(Ok(result)) => Ok(result),
        Some(Err(error)) => Err(system_error(cx, error.call, error.errno)),
        None => Ok(None),
    }
}

fn start(
    cx: &mut Context<'_, '_, '_>,
    operation: impl FnOnce() -> Result<OSResult, OperationError> + Send + 'static,
) -> Result<Value, Throw> {
    let shared = Shared::new().map_err(|error| {
        system_error(cx, "socketpair", error.raw_os_error().unwrap_or(libc::EIO))
    })?;
    let worker = Arc::clone(&shared);
    cx.vm
        .engine
        .blocking
        .submit(Box::new(move || worker.complete(operation())))
        .map_err(|error| {
            system_error(cx, "thread", error.raw_os_error().unwrap_or(libc::EAGAIN))
        })?;

    let object = cx.new_built_in_instance(OS_OPERATION)?;
    let Some(state) = state_ref::<OSOperation>(&object) else {
        return Err(cx.type_error("the OS operation has no built-in state"));
    };

    *state.shared.borrow_mut() = Some(shared);
    Ok(object)
}

fn user_key(cx: &mut Context<'_, '_, '_>, arguments: Arguments<'_>) -> Result<UserKey, Throw> {
    // SAFETY: built-in argument validation guarantees this argument is present.
    let identity = unsafe { arguments.value_unchecked(0) };
    if let Some(id) = identity.as_int() {
        return libc::uid_t::try_from(id)
            .map(UserKey::Id)
            .map_err(|_| system_error(cx, "getpwuid_r", libc::EINVAL));
    }

    let Some(name) = identity.as_string_bytes() else {
        // SAFETY: built-in argument validation limits the union to strings and integers.
        unsafe { unreachable_invariant("a user identity is a string or integer") }
    };
    c_string(name, "getpwnam_r")
        .map(UserKey::Name)
        .map_err(|error| system_error(cx, error.call, error.errno))
}

fn group_key(cx: &mut Context<'_, '_, '_>, arguments: Arguments<'_>) -> Result<GroupKey, Throw> {
    // SAFETY: built-in argument validation guarantees this argument is present.
    let identity = unsafe { arguments.value_unchecked(0) };
    if let Some(id) = identity.as_int() {
        return libc::gid_t::try_from(id)
            .map(GroupKey::Id)
            .map_err(|_| system_error(cx, "getgrgid_r", libc::EINVAL));
    }

    let Some(name) = identity.as_string_bytes() else {
        // SAFETY: built-in argument validation limits the union to strings and integers.
        unsafe { unreachable_invariant("a group identity is a string or integer") }
    };
    c_string(name, "getgrnam_r")
        .map(GroupKey::Name)
        .map_err(|error| system_error(cx, error.call, error.errno))
}

fn lookup_user(key: &UserKey) -> Result<Option<UserRecord>, OperationError> {
    let call = match key {
        UserKey::Name(_) => "getpwnam_r",
        UserKey::Id(_) => "getpwuid_r",
    };
    let mut buffer = record_buffer(libc::_SC_GETPW_R_SIZE_MAX);
    loop {
        // SAFETY: zero is valid for this C output type.
        let mut record = unsafe { zeroed::<libc::passwd>() };
        let mut result = null_mut();
        // SAFETY: the record and buffer remain writable for the duration of the call.
        let code = unsafe {
            match key {
                UserKey::Name(name) => libc::getpwnam_r(
                    name.as_ptr(),
                    &raw mut record,
                    buffer.as_mut_ptr().cast(),
                    buffer.len(),
                    &raw mut result,
                ),
                UserKey::Id(id) => libc::getpwuid_r(
                    *id,
                    &raw mut record,
                    buffer.as_mut_ptr().cast(),
                    buffer.len(),
                    &raw mut result,
                ),
            }
        };
        if code == libc::ERANGE {
            grow_record_buffer(&mut buffer, call)?;
            continue;
        }
        if code != 0 {
            return Err(operation_error(call, code));
        }
        if result.is_null() {
            return Ok(None);
        }

        let name = c_field(record.pw_name);
        if name.is_empty() {
            return Err(operation_error(call, libc::EIO));
        }

        return Ok(Some(UserRecord {
            name,
            id: record.pw_uid,
            primary_group: record.pw_gid,
            home_directory: c_field(record.pw_dir),
            shell: c_field(record.pw_shell),
        }));
    }
}

fn lookup_group(key: &GroupKey) -> Result<Option<GroupRecord>, OperationError> {
    let call = match key {
        GroupKey::Name(_) => "getgrnam_r",
        GroupKey::Id(_) => "getgrgid_r",
    };
    let mut buffer = record_buffer(libc::_SC_GETGR_R_SIZE_MAX);
    loop {
        // SAFETY: zero is valid for this C output type.
        let mut record = unsafe { zeroed::<libc::group>() };
        let mut result = null_mut();
        // SAFETY: the record and buffer remain writable for the duration of the call.
        let code = unsafe {
            match key {
                GroupKey::Name(name) => libc::getgrnam_r(
                    name.as_ptr(),
                    &raw mut record,
                    buffer.as_mut_ptr().cast(),
                    buffer.len(),
                    &raw mut result,
                ),
                GroupKey::Id(id) => libc::getgrgid_r(
                    *id,
                    &raw mut record,
                    buffer.as_mut_ptr().cast(),
                    buffer.len(),
                    &raw mut result,
                ),
            }
        };
        if code == libc::ERANGE {
            grow_record_buffer(&mut buffer, call)?;
            continue;
        }
        if code != 0 {
            return Err(operation_error(call, code));
        }
        if result.is_null() {
            return Ok(None);
        }

        let name = c_field(record.gr_name);
        if name.is_empty() {
            return Err(operation_error(call, libc::EIO));
        }

        let mut members = Vec::new();
        let mut member = record.gr_mem;
        if !member.is_null() {
            loop {
                // SAFETY: the null-terminated member array points into the live result buffer.
                // Darwin may pack this array without pointer alignment.
                let value = unsafe { read_unaligned(member) };
                if value.is_null() {
                    break;
                }

                let name = c_field(value);
                if name.is_empty() {
                    return Err(operation_error(call, libc::EIO));
                }
                members.push(name);
                // SAFETY: the terminator follows the current entry in the live result buffer.
                member = unsafe { member.byte_add(size_of::<*mut libc::c_char>()) };
            }
        }

        return Ok(Some(GroupRecord {
            name,
            id: record.gr_gid,
            members,
        }));
    }
}

fn lookup_groups_for_user(
    name: &CString,
    primary_group: libc::gid_t,
) -> Result<Vec<GroupRecord>, OperationError> {
    #[cfg(target_os = "macos")]
    let primary = group_list_id(primary_group)?;
    #[cfg(target_os = "linux")]
    let primary = primary_group;

    let mut groups = vec![GroupListId::default(); INITIAL_GROUP_CAPACITY];
    loop {
        let mut count = libc::c_int::try_from(groups.len())
            .map_err(|_| operation_error("getgrouplist", libc::EOVERFLOW))?;
        // SAFETY: the group buffer is writable and `count` reports its exact capacity.
        let code = unsafe {
            libc::getgrouplist(name.as_ptr(), primary, groups.as_mut_ptr(), &raw mut count)
        };
        if code >= 0 {
            let count =
                usize::try_from(count).map_err(|_| operation_error("getgrouplist", libc::EIO))?;
            if count > groups.len() {
                return Err(operation_error("getgrouplist", libc::EIO));
            }
            groups.truncate(count);
            break;
        }

        let reported =
            usize::try_from(count).map_err(|_| operation_error("getgrouplist", libc::EIO))?;
        let required = reported.max(
            groups
                .len()
                .checked_mul(2)
                .ok_or_else(|| operation_error("getgrouplist", libc::EOVERFLOW))?,
        );
        if required > MAXIMUM_GROUP_COUNT {
            return Err(operation_error("getgrouplist", libc::EOVERFLOW));
        }
        groups.resize(required, GroupListId::default());
    }

    let mut seen = HashSet::with_capacity(groups.len());
    let mut records = Vec::with_capacity(groups.len());
    for group in groups {
        #[cfg(target_os = "macos")]
        let id = group_id(group)?;
        #[cfg(target_os = "linux")]
        let id = group;

        if !seen.insert(id) {
            continue;
        }
        if let Some(record) = lookup_group(&GroupKey::Id(id))? {
            records.push(record);
        }
    }

    Ok(records)
}

fn record_buffer(setting: libc::c_int) -> Vec<u8> {
    // SAFETY: `setting` is one of the supported record-buffer sysconf keys.
    let configured = unsafe { libc::sysconf(setting) };
    let size = usize::try_from(configured)
        .unwrap_or(FALLBACK_RECORD_BUFFER_SIZE)
        .clamp(1, MAXIMUM_RECORD_BUFFER_SIZE);
    vec![0; size]
}

fn grow_record_buffer(buffer: &mut Vec<u8>, call: &'static str) -> Result<(), OperationError> {
    let capacity = buffer
        .len()
        .checked_mul(2)
        .filter(|capacity| *capacity <= MAXIMUM_RECORD_BUFFER_SIZE)
        .ok_or_else(|| operation_error(call, libc::EOVERFLOW))?;
    buffer.resize(capacity, 0);
    Ok(())
}

fn c_string(bytes: &[u8], call: &'static str) -> Result<CString, OperationError> {
    CString::new(bytes).map_err(|_| operation_error(call, libc::EINVAL))
}

fn c_field(pointer: *const libc::c_char) -> Vec<u8> {
    if pointer.is_null() {
        Vec::new()
    } else {
        // SAFETY: successful reentrant directory lookups return null-terminated fields.
        unsafe { CStr::from_ptr(pointer) }.to_bytes().to_vec()
    }
}

fn user_value(cx: &Context<'_, '_, '_>, record: &UserRecord) -> Value {
    let name = cx.string(&record.name);
    let id = Value::int(i64::from(record.id));
    let primary_group = Value::int(i64::from(record.primary_group));
    let home_directory = cx.string(&record.home_directory);
    let shell = cx.string(&record.shell);
    cx.tuple([name, id, primary_group, home_directory, shell])
}

fn group_value(cx: &Context<'_, '_, '_>, record: GroupRecord) -> Value {
    let name = cx.string(&record.name);
    let id = Value::int(i64::from(record.id));
    let members = cx.vec(record.members.into_iter().map(|name| cx.string(&name)));
    cx.tuple([name, id, members])
}

#[cfg(target_os = "macos")]
fn group_list_id(id: libc::gid_t) -> Result<GroupListId, OperationError> {
    GroupListId::try_from(id).map_err(|_| operation_error("getgrouplist", libc::EINVAL))
}

#[cfg(target_os = "macos")]
fn group_id(id: GroupListId) -> Result<libc::gid_t, OperationError> {
    libc::gid_t::try_from(id).map_err(|_| operation_error("getgrouplist", libc::EIO))
}

const fn operation_error(call: &'static str, errno: i32) -> OperationError {
    OperationError { call, errno }
}
