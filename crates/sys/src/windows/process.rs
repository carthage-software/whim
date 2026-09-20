use std::collections::HashMap;
use std::ffi::{OsStr, OsString};
use std::fs::{File, OpenOptions};
use std::io;
use std::os::windows::io::{AsRawHandle, FromRawHandle, IntoRawHandle, OwnedHandle, RawHandle};
use std::process::{self, Child};

use windows_sys::Win32::Foundation::{
    ERROR_ACCESS_DENIED, ERROR_INVALID_PARAMETER, FILETIME, INVALID_HANDLE_VALUE, WAIT_TIMEOUT,
};
use windows_sys::Win32::System::Diagnostics::ToolHelp::{
    CreateToolhelp32Snapshot, PROCESSENTRY32W, Process32FirstW, Process32NextW, TH32CS_SNAPPROCESS,
};
use windows_sys::Win32::System::Threading::{
    GetCurrentProcess, GetExitCodeProcess, GetProcessTimes, OpenProcess,
    PROCESS_QUERY_LIMITED_INFORMATION, PROCESS_SYNCHRONIZE, PROCESS_TERMINATE, TerminateProcess,
    WaitForSingleObject,
};

use crate::platform::descriptor::duplicate_handle;
use crate::process::{Exit, Spawn, Spawned};
use crate::{Descriptor, Error, RawDescriptor, Result};

pub fn user() -> Result<(u32, u32)> {
    Err(Error::Unsupported("POSIX user identifiers"))
}
pub fn group() -> Result<(u32, u32)> {
    Err(Error::Unsupported("POSIX group identifiers"))
}
pub fn supplementary_groups() -> Result<Vec<u32>> {
    Err(Error::Unsupported("POSIX supplementary groups"))
}
pub fn set_user(_real: i64, _effective: i64) -> Result<()> {
    Err(Error::Unsupported("POSIX user identifiers"))
}
pub fn set_group(_real: i64, _effective: i64) -> Result<()> {
    Err(Error::Unsupported("POSIX group identifiers"))
}
pub fn set_supplementary_groups(_groups: &[i64]) -> Result<()> {
    Err(Error::Unsupported("POSIX supplementary groups"))
}
pub fn initialize_groups(_user: &[u8], _group: i64) -> Result<()> {
    Err(Error::Unsupported("POSIX supplementary groups"))
}
pub fn session_id(_process: i64) -> Result<u32> {
    Err(Error::Unsupported("POSIX process sessions"))
}
pub fn start_session() -> Result<u32> {
    Err(Error::Unsupported("POSIX process sessions"))
}
pub fn group_id(_process: i64) -> Result<u32> {
    Err(Error::Unsupported("POSIX process groups"))
}
pub fn set_group_id(_process: i64, _group: i64) -> Result<()> {
    Err(Error::Unsupported("POSIX process groups"))
}
pub fn priority(_process: i64) -> Result<i32> {
    Err(Error::Unsupported("POSIX process priorities"))
}
pub fn set_priority(_process: i64, _priority: i64) -> Result<()> {
    Err(Error::Unsupported("POSIX process priorities"))
}
pub fn resource_limit(_resource: i64) -> Result<(i64, i64)> {
    Err(Error::Unsupported("POSIX resource limits"))
}
pub fn set_resource_limit(_resource: i64, _soft: i64, _hard: i64) -> Result<()> {
    Err(Error::Unsupported("POSIX resource limits"))
}
pub fn exchange_creation_mask(_mask: i64) -> Result<u32> {
    Err(Error::Unsupported("POSIX file mode masks"))
}
pub fn replace(
    _program: &OsStr,
    _arguments: &[OsString],
    _environment: Option<&[(OsString, OsString)]>,
) -> Result<()> {
    Err(Error::Unsupported("replacing the current process"))
}
pub fn send_signal(_process: i64, _signal: i64) -> Result<()> {
    Err(Error::Unsupported("POSIX signals"))
}
pub fn watch_signal(_signal: i64) -> Result<Descriptor> {
    Err(Error::Unsupported("POSIX signals"))
}

pub fn parent_id() -> Result<u32> {
    // SAFETY: this query returns an owned snapshot without borrowing memory.
    let raw = unsafe { CreateToolhelp32Snapshot(TH32CS_SNAPPROCESS, 0) };
    if raw == INVALID_HANDLE_VALUE {
        return Err(Error::last("CreateToolhelp32Snapshot"));
    }
    // SAFETY: the snapshot call returned a new owned handle.
    let snapshot = unsafe { OwnedHandle::from_raw_handle(raw) };
    let mut entry = PROCESSENTRY32W::default();
    entry.dwSize = size_of_val(&entry) as u32;
    // SAFETY: entry is writable and its size identifies the layout.
    let mut found = unsafe { Process32FirstW(snapshot.as_raw_handle(), &raw mut entry) } != 0;
    while found {
        if entry.th32ProcessID == process::id() {
            return Ok(entry.th32ParentProcessID);
        }

        // SAFETY: snapshot remains open and entry remains writable.
        found = unsafe { Process32NextW(snapshot.as_raw_handle(), &raw mut entry) } != 0;
    }
    Ok(0)
}

fn cpu_times(handle: RawHandle) -> Result<[u64; 2]> {
    let (mut created, mut exited, mut kernel, mut user) = (
        FILETIME::default(),
        FILETIME::default(),
        FILETIME::default(),
        FILETIME::default(),
    );
    // SAFETY: the OS checks the process handle and fills four live FILETIME records.
    if unsafe {
        GetProcessTimes(
            handle,
            &raw mut created,
            &raw mut exited,
            &raw mut kernel,
            &raw mut user,
        )
    } == 0
    {
        return Err(Error::last("GetProcessTimes"));
    }
    Ok([user, kernel]
        .map(|time| ((u64::from(time.dwHighDateTime) << 32) | u64::from(time.dwLowDateTime)) / 10))
}

pub fn exists(process: i64) -> Result<bool> {
    let Ok(process) = u32::try_from(process) else {
        return Ok(false);
    };
    // SAFETY: OpenProcess checks the process identifier and returns a new owned handle.
    let handle = unsafe {
        OpenProcess(
            PROCESS_QUERY_LIMITED_INFORMATION | PROCESS_SYNCHRONIZE,
            0,
            process,
        )
    };
    if handle.is_null() {
        let error = io::Error::last_os_error();
        return match error.raw_os_error().map(i32::cast_unsigned) {
            Some(ERROR_ACCESS_DENIED) => Ok(true),
            Some(ERROR_INVALID_PARAMETER) => Ok(false),
            _ => Err(Error::new("OpenProcess", error)),
        };
    }
    // SAFETY: OpenProcess returned a new owned handle.
    let handle = unsafe { OwnedHandle::from_raw_handle(handle) };
    // SAFETY: handle remains open through the nonblocking wait.
    Ok(unsafe { WaitForSingleObject(handle.as_raw_handle(), 0) } == WAIT_TIMEOUT)
}

pub fn terminate(process: i64) -> Result<()> {
    let process = u32::try_from(process).map_err(|_| Error::invalid("OpenProcess"))?;
    // SAFETY: OpenProcess checks the process identifier and returns a new owned handle.
    let handle = unsafe { OpenProcess(PROCESS_TERMINATE | PROCESS_SYNCHRONIZE, 0, process) };
    if handle.is_null() {
        return Err(Error::last("OpenProcess"));
    }
    // SAFETY: OpenProcess returned a new owned handle.
    let handle = unsafe { OwnedHandle::from_raw_handle(handle) };
    // SAFETY: the handle grants termination rights and remains open through both calls.
    if unsafe {
        TerminateProcess(handle.as_raw_handle(), 1) == 0
            && WaitForSingleObject(handle.as_raw_handle(), 0) == WAIT_TIMEOUT
    } {
        return Err(Error::last("TerminateProcess"));
    }
    Ok(())
}

pub(crate) fn terminal_stream(input: bool) -> Result<File> {
    OpenOptions::new()
        .read(input)
        .write(!input)
        .open(if input { "CONIN$" } else { "CONOUT$" })
        .map_err(|error| Error::new("CreateFileW", error))
}

pub fn validate_spawn(inherited: bool, group: bool) -> Result<()> {
    if inherited {
        return Err(Error::Unsupported("inherited POSIX descriptor numbers"));
    }
    if group {
        return Err(Error::Unsupported("POSIX process groups"));
    }
    Ok(())
}

#[derive(Default)]
pub struct Processes {
    children: HashMap<u32, Child>,
    child_times: [u64; 2],
}

impl Processes {
    pub fn spawn(&mut self, request: Spawn) -> Result<Spawned> {
        let mut command = request.command()?.command;
        let mut child = command
            .spawn()
            .map_err(|error| Error::new("CreateProcessW", error))?;
        let id = child.id();
        let result = (|| {
            Ok(Spawned {
                id,
                input: child_descriptor(child.stdin.take())?,
                output: child_descriptor(child.stdout.take())?,
                error: child_descriptor(child.stderr.take())?,
            })
        })();
        match result {
            Ok(spawned) => {
                self.children.insert(id, child);
                Ok(spawned)
            }
            Err(error) => {
                let _ = child.kill();
                let _ = child.wait();
                Err(error)
            }
        }
    }

    pub fn watch(&mut self, process: i64) -> Result<Descriptor> {
        let process = u32::try_from(process).map_err(|_| Error::invalid("wait"))?;
        let child = self
            .children
            .remove(&process)
            .ok_or_else(|| Error::invalid("the process is not an unwatched child"))?;
        duplicate_handle(child.as_raw_handle())
            .map(Descriptor::from_file)
            .map_err(|error| Error::new("DuplicateHandle", error))
    }

    pub fn record_exit(&mut self, exit: Exit) -> i64 {
        for (total, elapsed) in self
            .child_times
            .iter_mut()
            .zip([exit.user_time, exit.system_time])
        {
            *total = total.saturating_add(elapsed);
        }
        exit.status
    }

    pub fn times(&self) -> Result<[i64; 4]> {
        // SAFETY: GetCurrentProcess returns the current process pseudo-handle.
        let [user, kernel] = cpu_times(unsafe { GetCurrentProcess() })?;
        let [child_user, child_kernel] = self.child_times;
        Ok([user, kernel, child_user, child_kernel]
            .map(|time| i64::try_from(time).unwrap_or(i64::MAX)))
    }
}

fn child_descriptor<T: IntoRawHandle>(stream: Option<T>) -> Result<Option<Descriptor>> {
    stream
        .map(|stream| {
            // SAFETY: ownership transfers from the child stream to File.
            let descriptor =
                Descriptor::from_file(unsafe { File::from_raw_handle(stream.into_raw_handle()) });
            descriptor.set_non_blocking(true)?;
            Ok(descriptor)
        })
        .transpose()
}

pub fn watch_descriptor(descriptor: &Descriptor) -> Result<RawDescriptor> {
    descriptor
        .handle()
        .map(RawDescriptor::Waitable)
        .map_err(|error| Error::new("WaitForSingleObject", error))
}

pub fn read_exit(descriptor: &Descriptor) -> Result<Option<Exit>> {
    let handle = descriptor
        .handle()
        .map_err(|error| Error::new("GetExitCodeProcess", error))?;
    // SAFETY: descriptor keeps the process handle alive through the nonblocking wait.
    if unsafe { WaitForSingleObject(handle, 0) } == WAIT_TIMEOUT {
        return Ok(None);
    }
    let mut code = 0;
    // SAFETY: the descriptor keeps the process handle alive and code is a live output.
    if unsafe { GetExitCodeProcess(handle, &raw mut code) } == 0 {
        return Err(Error::last("GetExitCodeProcess"));
    }
    let [user_time, system_time] = cpu_times(handle)?;
    Ok(Some(Exit {
        status: i64::from(code) << 8,
        user_time,
        system_time,
    }))
}
