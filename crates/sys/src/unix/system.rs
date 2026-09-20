//! Unix system-information primitives.

use std::ffi::CStr;
use std::io;
#[cfg(any(target_os = "macos", target_os = "freebsd"))]
use std::mem::size_of;
use std::mem::zeroed;
#[cfg(any(target_os = "macos", target_os = "freebsd"))]
use std::ptr::null_mut;
#[cfg(any(target_os = "macos", target_os = "freebsd"))]
use std::time::Duration;
#[cfg(any(target_os = "macos", target_os = "freebsd"))]
use std::time::SystemTime;
#[cfg(any(target_os = "macos", target_os = "freebsd"))]
use std::time::UNIX_EPOCH;

use crate::Error;

fn last_errno() -> i32 {
    io::Error::last_os_error()
        .raw_os_error()
        .unwrap_or(libc::EIO)
}

const fn field(field: &[libc::c_char]) -> &[u8] {
    // SAFETY: the arguments follow the platform ABI; pointers and descriptors stay valid.
    unsafe { CStr::from_ptr(field.as_ptr()) }.to_bytes()
}

pub fn information() -> crate::Result<[Vec<u8>; 5]> {
    // SAFETY: zero is valid for this C output type.
    let mut information = unsafe { zeroed::<libc::utsname>() };
    // SAFETY: the arguments follow the platform ABI; pointers and descriptors stay valid.
    if unsafe { libc::uname(&raw mut information) } < 0 {
        return Err(Error::last("uname"));
    }
    Ok([
        field(&information.sysname).to_vec(),
        field(&information.nodename).to_vec(),
        field(&information.release).to_vec(),
        field(&information.version).to_vec(),
        field(&information.machine).to_vec(),
    ])
}

#[cfg(target_os = "linux")]
fn uptime_milliseconds() -> Result<u64, i32> {
    // SAFETY: an all-zero timespec is valid and clock_gettime initializes it.
    let mut time = unsafe { zeroed::<libc::timespec>() };
    // SAFETY: time points to a live timespec which clock_gettime may write.
    if unsafe { libc::clock_gettime(libc::CLOCK_BOOTTIME, &raw mut time) } < 0 {
        return Err(last_errno());
    }
    let seconds = u64::try_from(time.tv_sec).map_err(|_| libc::EIO)?;
    let nanoseconds = u64::try_from(time.tv_nsec).map_err(|_| libc::EIO)?;
    Ok(seconds.saturating_mul(1000) + nanoseconds / 1_000_000)
}

#[cfg(any(target_os = "macos", target_os = "freebsd"))]
fn uptime_milliseconds() -> Result<u64, i32> {
    // SAFETY: zero is valid for this C output type.
    let mut boot = unsafe { zeroed::<libc::timeval>() };
    let mut length = size_of::<libc::timeval>();
    let mut name = [libc::CTL_KERN, libc::KERN_BOOTTIME];
    let name_length = libc::c_uint::try_from(name.len()).map_err(|_| libc::EINVAL)?;
    // SAFETY: the arguments follow the platform ABI; pointers and descriptors stay valid.
    if unsafe {
        libc::sysctl(
            name.as_mut_ptr(),
            name_length,
            (&raw mut boot).cast(),
            &raw mut length,
            null_mut(),
            0,
        )
    } < 0
    {
        return Err(last_errno());
    }
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|_| libc::EIO)?;
    let boot = Duration::new(
        boot.tv_sec.max(0).cast_unsigned(),
        u32::try_from(boot.tv_usec.max(0)).map_err(|_| libc::EIO)? * 1000,
    );
    Ok(u64::try_from(now.saturating_sub(boot).as_millis()).unwrap_or(u64::MAX))
}

pub fn uptime() -> crate::Result<u64> {
    uptime_milliseconds().map_err(|errno| Error::new("uptime", io::Error::from_raw_os_error(errno)))
}

pub fn load_averages() -> crate::Result<[f64; 3]> {
    let mut averages = [0.0_f64; 3];
    // SAFETY: the arguments follow the platform ABI; pointers and descriptors stay valid.
    let count = unsafe { libc::getloadavg(averages.as_mut_ptr(), 3) };
    if count < 3 {
        return Err(Error::last("getloadavg"));
    }
    Ok(averages)
}

#[cfg(target_os = "linux")]
fn memory_bytes() -> Result<(u64, u64), i32> {
    // SAFETY: an all-zero sysinfo value is valid and sysinfo initializes it.
    let mut information = unsafe { zeroed::<libc::sysinfo>() };
    // SAFETY: information points to a live sysinfo value which libc may write.
    if unsafe { libc::sysinfo(&raw mut information) } < 0 {
        return Err(last_errno());
    }
    let unit = u128::from(information.mem_unit);
    let total = u128::from(information.totalram).saturating_mul(unit);
    let available = u128::from(information.freeram)
        .saturating_add(u128::from(information.bufferram))
        .saturating_mul(unit);
    Ok((
        u64::try_from(total).unwrap_or(u64::MAX),
        u64::try_from(available).unwrap_or(u64::MAX),
    ))
}

#[cfg(any(target_os = "macos", target_os = "freebsd"))]
fn sysctl_value<T: Default>(name: &'static [u8]) -> Result<T, i32> {
    let mut value = T::default();
    let mut length = size_of::<T>();
    // SAFETY: the arguments follow the platform ABI; pointers and descriptors stay valid.
    if unsafe {
        libc::sysctlbyname(
            name.as_ptr().cast(),
            (&raw mut value).cast(),
            &raw mut length,
            null_mut(),
            0,
        )
    } < 0
    {
        return Err(last_errno());
    }
    if length != size_of::<T>() {
        return Err(libc::EIO);
    }
    Ok(value)
}

#[cfg(target_os = "macos")]
fn memory_bytes() -> Result<(u64, u64), i32> {
    let total = sysctl_value::<u64>(b"hw.memsize\0")?;
    // SAFETY: the arguments follow the platform ABI; pointers and descriptors stay valid.
    let page_size = unsafe { libc::sysconf(libc::_SC_PAGESIZE) };
    if page_size <= 0 {
        return Err(last_errno());
    }
    let free = sysctl_value::<u32>(b"vm.page_free_count\0").unwrap_or(0);
    let inactive = sysctl_value::<u32>(b"vm.page_inactive_count\0").unwrap_or(0);
    let available = (u64::from(free) + u64::from(inactive)) * page_size.cast_unsigned();
    Ok((total, available))
}

#[cfg(target_os = "freebsd")]
fn memory_bytes() -> Result<(u64, u64), i32> {
    let total = sysctl_value::<u64>(b"hw.physmem\0")?;
    let page_size = sysctl_value::<u32>(b"vm.stats.vm.v_page_size\0")?;
    let free = sysctl_value::<u32>(b"vm.stats.vm.v_free_count\0")?;
    let inactive = sysctl_value::<u32>(b"vm.stats.vm.v_inactive_count\0")?;
    let available = (u64::from(free) + u64::from(inactive)) * u64::from(page_size);
    Ok((total, available.min(total)))
}

pub fn memory() -> crate::Result<(u64, u64)> {
    memory_bytes()
        .map_err(|errno| Error::new("memory_information", io::Error::from_raw_os_error(errno)))
}
