use std::env::consts;
use std::io;

use windows_sys::Win32::System::SystemInformation::{
    ComputerNameDnsHostname, GetComputerNameExW, GetTickCount64, GlobalMemoryStatusEx,
    MEMORYSTATUSEX, OSVERSIONINFOW,
};

use crate::{Error, Result};

#[link(name = "ntdll")]
unsafe extern "system" {
    fn RtlGetVersion(version: *mut OSVERSIONINFOW) -> i32;
}

pub fn information() -> Result<[Vec<u8>; 5]> {
    let mut version = OSVERSIONINFOW {
        dwOSVersionInfoSize: u32::try_from(size_of::<OSVERSIONINFOW>()).unwrap(),
        ..OSVERSIONINFOW::default()
    };

    // SAFETY: version has the size and layout that RtlGetVersion requires.
    if unsafe { RtlGetVersion(&raw mut version) } < 0 {
        return Err(Error::new(
            "RtlGetVersion",
            io::Error::other("could not read Windows version"),
        ));
    }

    let mut node = [0_u16; 256];
    let mut length = u32::try_from(node.len()).unwrap();
    // SAFETY: node is a writable UTF-16 buffer of length characters.
    if unsafe { GetComputerNameExW(ComputerNameDnsHostname, node.as_mut_ptr(), &raw mut length) }
        == 0
    {
        return Err(Error::last("GetComputerNameExW"));
    }

    let release = format!(
        "{}.{}.{}",
        version.dwMajorVersion, version.dwMinorVersion, version.dwBuildNumber
    );

    let version_text = format!("Build {}", version.dwBuildNumber);
    Ok([
        b"Windows".to_vec(),
        String::from_utf16_lossy(&node[..length as usize]).into_bytes(),
        release.into_bytes(),
        version_text.into_bytes(),
        consts::ARCH.as_bytes().to_vec(),
    ])
}

pub fn uptime() -> Result<u64> {
    // SAFETY: GetTickCount64 only reads the system uptime.
    Ok(unsafe { GetTickCount64() })
}

pub fn load_averages() -> Result<[f64; 3]> {
    Err(Error::Unsupported("POSIX load averages"))
}

pub fn memory() -> Result<(u64, u64)> {
    let mut memory = MEMORYSTATUSEX {
        dwLength: u32::try_from(size_of::<MEMORYSTATUSEX>()).unwrap(),
        ..MEMORYSTATUSEX::default()
    };

    // SAFETY: memory has the size and layout required by GlobalMemoryStatusEx.
    if unsafe { GlobalMemoryStatusEx(&raw mut memory) } == 0 {
        return Err(Error::last("GlobalMemoryStatusEx"));
    }

    Ok((memory.ullTotalPhys, memory.ullAvailPhys))
}
