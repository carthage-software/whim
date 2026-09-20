use std::io;

use thiserror::Error as ThisError;

#[derive(Debug, ThisError)]
pub enum Error {
    #[error("{call}: {source}")]
    Io {
        call: &'static str,
        #[source]
        source: io::Error,
    },
    #[error("{0} is not supported on this platform")]
    Unsupported(&'static str),
}

impl Error {
    pub fn new(call: &'static str, source: impl Into<io::Error>) -> Self {
        Self::Io {
            call,
            source: source.into(),
        }
    }

    pub fn last(call: &'static str) -> Self {
        Self::new(call, io::Error::last_os_error())
    }

    pub fn invalid(call: &'static str) -> Self {
        Self::new(call, io::ErrorKind::InvalidInput)
    }

    pub fn bad_descriptor(call: &'static str) -> Self {
        #[cfg(unix)]
        let code = libc::EBADF;
        #[cfg(windows)]
        let code = 6;
        Self::new(call, io::Error::from_raw_os_error(code))
    }

    pub fn errno(&self) -> i32 {
        match self {
            Self::Io { source, .. } => errno(source),
            Self::Unsupported(_) => libc::ENOTSUP,
        }
    }
}

pub(crate) fn errno(error: &io::Error) -> i32 {
    #[cfg(unix)]
    if let Some(code) = error.raw_os_error() {
        return code;
    }

    use io::ErrorKind;
    match error.kind() {
        ErrorKind::NotFound => libc::ENOENT,
        ErrorKind::PermissionDenied => libc::EACCES,
        ErrorKind::AlreadyExists => libc::EEXIST,
        ErrorKind::WouldBlock => libc::EAGAIN,
        ErrorKind::Interrupted => libc::EINTR,
        ErrorKind::InvalidInput | ErrorKind::InvalidData => libc::EINVAL,
        ErrorKind::BrokenPipe => libc::EPIPE,
        ErrorKind::NotADirectory => libc::ENOTDIR,
        ErrorKind::IsADirectory => libc::EISDIR,
        ErrorKind::DirectoryNotEmpty => libc::ENOTEMPTY,
        ErrorKind::ConnectionRefused => libc::ECONNREFUSED,
        ErrorKind::ConnectionReset => libc::ECONNRESET,
        ErrorKind::ConnectionAborted => libc::ECONNABORTED,
        ErrorKind::NotConnected => libc::ENOTCONN,
        ErrorKind::AddrInUse => libc::EADDRINUSE,
        ErrorKind::AddrNotAvailable => libc::EADDRNOTAVAIL,
        ErrorKind::TimedOut => libc::ETIMEDOUT,
        ErrorKind::Unsupported => libc::ENOTSUP,
        ErrorKind::OutOfMemory => libc::ENOMEM,
        _ => platform_errno(error),
    }
}

fn platform_errno(error: &io::Error) -> i32 {
    #[cfg(windows)]
    {
        use windows_sys::Win32::Networking::WinSock as ws;
        match error.raw_os_error() {
            Some(6) => libc::EBADF,
            Some(ws::WSAEINPROGRESS | ws::WSAEALREADY) => libc::EINPROGRESS,
            Some(ws::WSAENOTSOCK) => libc::ENOTSOCK,
            Some(ws::WSAEMSGSIZE) => libc::EMSGSIZE,
            _ => libc::EIO,
        }
    }
    #[cfg(unix)]
    {
        let _ = error;
        libc::EIO
    }
}
