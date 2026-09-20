use std::collections::HashSet;
use std::ffi::{CStr, CString};
use std::io;
use std::mem::zeroed;
use std::ptr::{null_mut, read_unaligned};

use crate::accounts::{Group, Identity, User};
use crate::{Error, Result};

const FALLBACK_RECORD_BUFFER_SIZE: usize = 16 * 1024;
const MAXIMUM_RECORD_BUFFER_SIZE: usize = 16 * 1024 * 1024;
const MAXIMUM_GROUP_COUNT: usize = 1_048_576;
#[cfg(target_os = "macos")]
type GroupListId = libc::c_int;
#[cfg(any(target_os = "linux", target_os = "freebsd"))]
type GroupListId = libc::gid_t;

pub fn require_support() -> Result<()> {
    Ok(())
}

fn error(call: &'static str, errno: i32) -> Error {
    Error::new(call, io::Error::from_raw_os_error(errno))
}

pub fn user(key: &Identity) -> Result<Option<User>> {
    let call = match key {
        Identity::Name(_) => "getpwnam_r",
        Identity::Id(_) => "getpwuid_r",
    };
    let mut buffer = record_buffer(libc::_SC_GETPW_R_SIZE_MAX);
    loop {
        // SAFETY: an all-zero passwd is valid output storage.
        let mut record = unsafe { zeroed::<libc::passwd>() };
        let mut result = null_mut();
        // SAFETY: record and buffer remain writable throughout the call; names are null terminated.
        let code = unsafe {
            match key {
                Identity::Name(name) => libc::getpwnam_r(
                    name.as_ptr(),
                    &raw mut record,
                    buffer.as_mut_ptr().cast(),
                    buffer.len(),
                    &raw mut result,
                ),
                Identity::Id(id) => libc::getpwuid_r(
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
            return Err(error(call, code));
        }
        if result.is_null() {
            return Ok(None);
        }
        let name = c_field(record.pw_name);
        if name.is_empty() {
            return Err(error(call, libc::EIO));
        }
        return Ok(Some(User {
            name,
            id: record.pw_uid,
            primary_group: record.pw_gid,
            home_directory: c_field(record.pw_dir),
            shell: c_field(record.pw_shell),
        }));
    }
}

pub fn group(key: &Identity) -> Result<Option<Group>> {
    let call = match key {
        Identity::Name(_) => "getgrnam_r",
        Identity::Id(_) => "getgrgid_r",
    };
    let mut buffer = record_buffer(libc::_SC_GETGR_R_SIZE_MAX);
    loop {
        // SAFETY: an all-zero group is valid output storage.
        let mut record = unsafe { zeroed::<libc::group>() };
        let mut result = null_mut();
        // SAFETY: record and buffer remain writable throughout the call; names are null terminated.
        let code = unsafe {
            match key {
                Identity::Name(name) => libc::getgrnam_r(
                    name.as_ptr(),
                    &raw mut record,
                    buffer.as_mut_ptr().cast(),
                    buffer.len(),
                    &raw mut result,
                ),
                Identity::Id(id) => libc::getgrgid_r(
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
            return Err(error(call, code));
        }
        if result.is_null() {
            return Ok(None);
        }
        let name = c_field(record.gr_name);
        if name.is_empty() {
            return Err(error(call, libc::EIO));
        }
        let mut members = Vec::new();
        let mut member = record.gr_mem;
        if !member.is_null() {
            loop {
                // SAFETY: the null-terminated array lies in the live result buffer; Darwin may pack it without pointer alignment.
                let value = unsafe { read_unaligned(member) };
                if value.is_null() {
                    break;
                }
                let name = c_field(value);
                if name.is_empty() {
                    return Err(error(call, libc::EIO));
                }
                members.push(name);
                // SAFETY: the array terminator follows this entry in the live result buffer.
                member = unsafe { member.byte_add(size_of::<*mut libc::c_char>()) };
            }
        }
        return Ok(Some(Group {
            name,
            id: record.gr_gid,
            members,
        }));
    }
}

pub fn groups_for_user(name: &CString, primary_group: u32) -> Result<Vec<Group>> {
    #[cfg(target_os = "macos")]
    let primary =
        GroupListId::try_from(primary_group).map_err(|_| Error::invalid("getgrouplist"))?;
    #[cfg(not(target_os = "macos"))]
    let primary = primary_group;
    let mut groups = vec![GroupListId::default(); 16];
    loop {
        let mut count = libc::c_int::try_from(groups.len())
            .map_err(|_| error("getgrouplist", libc::EOVERFLOW))?;
        // SAFETY: groups is writable and count reports its exact capacity; name is null terminated.
        let code = unsafe {
            libc::getgrouplist(name.as_ptr(), primary, groups.as_mut_ptr(), &raw mut count)
        };
        if code >= 0 {
            let count = usize::try_from(count).map_err(|_| error("getgrouplist", libc::EIO))?;
            if count > groups.len() {
                return Err(error("getgrouplist", libc::EIO));
            }
            groups.truncate(count);
            break;
        }
        let reported = usize::try_from(count).map_err(|_| error("getgrouplist", libc::EIO))?;
        let required = reported.max(
            groups
                .len()
                .checked_mul(2)
                .ok_or_else(|| error("getgrouplist", libc::EOVERFLOW))?,
        );
        if required > MAXIMUM_GROUP_COUNT {
            return Err(error("getgrouplist", libc::EOVERFLOW));
        }
        groups.resize(required, GroupListId::default());
    }
    let mut seen = HashSet::with_capacity(groups.len());
    let mut records = Vec::with_capacity(groups.len());
    for id in groups {
        #[cfg(target_os = "macos")]
        let id = u32::try_from(id).map_err(|_| error("getgrouplist", libc::EIO))?;
        if !seen.insert(id) {
            continue;
        }
        if let Some(record) = group(&Identity::Id(id))? {
            records.push(record);
        }
    }
    Ok(records)
}

fn record_buffer(setting: libc::c_int) -> Vec<u8> {
    // SAFETY: setting is a record-buffer sysconf key.
    let configured = unsafe { libc::sysconf(setting) };
    vec![
        0;
        usize::try_from(configured)
            .unwrap_or(FALLBACK_RECORD_BUFFER_SIZE)
            .clamp(1, MAXIMUM_RECORD_BUFFER_SIZE)
    ]
}

fn grow_record_buffer(buffer: &mut Vec<u8>, call: &'static str) -> Result<()> {
    let capacity = buffer
        .len()
        .checked_mul(2)
        .filter(|capacity| *capacity <= MAXIMUM_RECORD_BUFFER_SIZE)
        .ok_or_else(|| error(call, libc::EOVERFLOW))?;
    buffer.resize(capacity, 0);
    Ok(())
}

fn c_field(pointer: *const libc::c_char) -> Vec<u8> {
    if pointer.is_null() {
        Vec::new()
    } else {
        // SAFETY: successful reentrant account lookups return null-terminated fields.
        unsafe { CStr::from_ptr(pointer) }.to_bytes().to_vec()
    }
}
