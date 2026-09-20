use crate::{Descriptor, Result};
use std::fs::OpenOptions;
use std::os::windows::io::AsRawHandle;
use windows_sys::Win32::System::Console::{
    CONSOLE_SCREEN_BUFFER_INFO, GetConsoleMode, GetConsoleScreenBufferInfo,
    GetNumberOfConsoleInputEvents,
};

pub fn is_terminal(descriptor: &Descriptor) -> bool {
    let Ok(handle) = descriptor.handle() else {
        return false;
    };
    let mut mode = 0;
    // SAFETY: the OS checks the handle and writes to live mode storage.
    unsafe { GetConsoleMode(handle, &raw mut mode) != 0 }
}

pub fn path(descriptor: &Descriptor) -> Result<Option<Vec<u8>>> {
    if !is_terminal(descriptor) {
        return Ok(None);
    }
    let Ok(handle) = descriptor.handle() else {
        return Ok(None);
    };
    let mut events = 0;
    // SAFETY: the OS checks the handle and writes to live count storage.
    let input = unsafe { GetNumberOfConsoleInputEvents(handle, &raw mut events) != 0 };
    Ok(Some(if input {
        b"CONIN$".to_vec()
    } else {
        b"CONOUT$".to_vec()
    }))
}

pub fn size(descriptor: &Descriptor) -> Result<Option<(u16, u16)>> {
    let Ok(handle) = descriptor.handle() else {
        return Ok(None);
    };
    let mut size = CONSOLE_SCREEN_BUFFER_INFO::default();
    // SAFETY: the OS checks the handle and fills the live structure.
    if unsafe { GetConsoleScreenBufferInfo(handle, &raw mut size) } == 0 {
        if !is_terminal(descriptor) {
            return Ok(None);
        }
        let Ok(output) = OpenOptions::new().read(true).write(true).open("CONOUT$") else {
            return Ok(None);
        };
        // SAFETY: output owns an open console screen buffer and size is a live output.
        if unsafe { GetConsoleScreenBufferInfo(output.as_raw_handle(), &raw mut size) } == 0 {
            return Ok(None);
        }
    }
    Ok(Some((
        (size.srWindow.Right - size.srWindow.Left + 1) as u16,
        (size.srWindow.Bottom - size.srWindow.Top + 1) as u16,
    )))
}
