//! Unix system-information primitives.

use std::ffi::CStr;
#[cfg(target_os = "macos")]
use std::mem::size_of;
use std::mem::zeroed;
#[cfg(target_os = "macos")]
use std::ptr::null_mut;
#[cfg(target_os = "macos")]
use std::time::Duration;
#[cfg(target_os = "macos")]
use std::time::SystemTime;
#[cfg(target_os = "macos")]
use std::time::UNIX_EPOCH;

use whim_macros::whim_function;

use crate::builtin::Context;
use crate::builtin::throw::Throw;
use crate::core::private::syscall::last_errno;
use crate::core::private::syscall::last_system_error;
use crate::core::private::syscall::system_error;
use crate::value::Value;

const fn field(field: &[libc::c_char]) -> &[u8] {
    // SAFETY: the arguments follow the platform ABI; pointers and descriptors stay valid.
    unsafe { CStr::from_ptr(field.as_ptr()) }.to_bytes()
}

fn nonnegative_integer(value: u64) -> Value {
    Value::int(i64::try_from(value).unwrap_or(i64::MAX))
}

#[whim_function(
    "Whim\\_Private\\system_information(): ((string&!''), string, string, string, string)"
)]
pub(crate) fn system_information(cx: &mut Context<'_, '_, '_>) -> Result<Value, Throw> {
    // SAFETY: zero is valid for this C output type.
    let mut information = unsafe { zeroed::<libc::utsname>() };
    // SAFETY: the arguments follow the platform ABI; pointers and descriptors stay valid.
    if unsafe { libc::uname(&raw mut information) } < 0 {
        return Err(last_system_error(cx, "uname"));
    }
    let system = cx.string(field(&information.sysname));
    let node = cx.string(field(&information.nodename));
    let release = cx.string(field(&information.release));
    let version = cx.string(field(&information.version));
    let machine = cx.string(field(&information.machine));
    Ok(cx.tuple([system, node, release, version, machine]))
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

#[cfg(target_os = "macos")]
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
        boot.tv_usec.max(0).cast_unsigned() * 1000,
    );
    Ok(u64::try_from(now.saturating_sub(boot).as_millis()).unwrap_or(u64::MAX))
}

#[whim_function("Whim\\_Private\\system_uptime(): (0..)")]
pub(crate) fn system_uptime(cx: &mut Context<'_, '_, '_>) -> Result<Value, Throw> {
    let uptime = uptime_milliseconds().map_err(|errno| system_error(cx, "uptime", errno))?;
    Ok(nonnegative_integer(uptime))
}

#[whim_function("Whim\\_Private\\load_averages(): (float, float, float)")]
pub(crate) fn load_averages(cx: &mut Context<'_, '_, '_>) -> Result<Value, Throw> {
    let mut averages = [0.0_f64; 3];
    // SAFETY: the arguments follow the platform ABI; pointers and descriptors stay valid.
    let count = unsafe { libc::getloadavg(averages.as_mut_ptr(), 3) };
    if count < 3 {
        return Err(last_system_error(cx, "getloadavg"));
    }
    let one = Value::float(averages[0]);
    let five = Value::float(averages[1]);
    let fifteen = Value::float(averages[2]);
    Ok(cx.tuple([one, five, fifteen]))
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

#[cfg(target_os = "macos")]
fn sysctl_u64(name: &'static [u8]) -> Result<u64, i32> {
    let mut value = 0_u64;
    let mut length = size_of::<u64>();
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
    Ok(value)
}

#[cfg(target_os = "macos")]
fn memory_bytes() -> Result<(u64, u64), i32> {
    let total = sysctl_u64(b"hw.memsize\0")?;
    // SAFETY: the arguments follow the platform ABI; pointers and descriptors stay valid.
    let page_size = unsafe { libc::sysconf(libc::_SC_PAGESIZE) };
    if page_size <= 0 {
        return Err(last_errno());
    }
    let free = sysctl_u64(b"vm.page_free_count\0").unwrap_or(0);
    let inactive = sysctl_u64(b"vm.page_inactive_count\0").unwrap_or(0);
    let available = free
        .saturating_add(inactive)
        .saturating_mul(page_size.cast_unsigned());
    Ok((total, available))
}

#[whim_function("Whim\\_Private\\memory_information(): ((0..), (0..))")]
pub(crate) fn memory_information(cx: &mut Context<'_, '_, '_>) -> Result<Value, Throw> {
    let (total, available) =
        memory_bytes().map_err(|errno| system_error(cx, "memory_information", errno))?;
    let total = nonnegative_integer(total);
    let available = nonnegative_integer(available);
    Ok(cx.tuple([total, available]))
}
