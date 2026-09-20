use rustix::io::Errno;
use signal_hook::low_level::unregister;
use std::ffi::c_void;
use std::fs::File;
use std::io;
use std::os::fd::{AsRawFd, BorrowedFd, FromRawFd, OwnedFd, RawFd};
use std::sync::Arc;

use rustix::{fs, io as unix_io, net, pipe};

use crate::{Error, Interest, Readiness, Result, StandardStream};

pub struct Descriptor {
    pub(crate) inner: Resource,
}

pub(crate) enum Resource {
    Owned(OwnedFd),
    Signal {
        read: OwnedFd,
        registration: signal_hook::SigId,
    },
    Standard(StandardStream),
}

impl Drop for Resource {
    fn drop(&mut self) {
        if let Self::Signal { registration, .. } = self {
            unregister(*registration);
        }
    }
}

#[cfg(any(target_os = "macos", target_os = "freebsd"))]
unsafe extern "C" {
    static mut __stdinp: *mut libc::FILE;
    static mut __stdoutp: *mut libc::FILE;
    static mut __stderrp: *mut libc::FILE;
}

#[cfg(target_os = "linux")]
unsafe extern "C" {
    static mut stdin: *mut libc::FILE;
    static mut stdout: *mut libc::FILE;
    static mut stderr: *mut libc::FILE;
}

impl StandardStream {
    fn file(self) -> *mut libc::FILE {
        #[cfg(any(target_os = "macos", target_os = "freebsd"))]
        // SAFETY: libc owns these process-wide streams for the process lifetime.
        unsafe {
            match self {
                Self::Input => __stdinp,
                Self::Output => __stdoutp,
                Self::Error => __stderrp,
            }
        }
        #[cfg(target_os = "linux")]
        // SAFETY: libc owns these process-wide streams for the process lifetime.
        unsafe {
            match self {
                Self::Input => stdin,
                Self::Output => stdout,
                Self::Error => stderr,
            }
        }
    }

    fn number(self) -> RawFd {
        match self {
            Self::Input => 0,
            Self::Output => 1,
            Self::Error => 2,
        }
    }

    pub fn write_all_blocking(self, mut bytes: &[u8]) -> io::Result<()> {
        let descriptor = Descriptor {
            inner: Resource::Standard(self),
        };

        while !bytes.is_empty() {
            match unix_io::write(descriptor.borrowed(), bytes) {
                Ok(0) => return Err(io::ErrorKind::WriteZero.into()),
                Ok(count) => bytes = &bytes[count..],
                Err(Errno::INTR) => {}
                Err(Errno::AGAIN) => self.wait_writable()?,
                Err(error) => return Err(error.into()),
            }
        }

        Ok(())
    }

    fn wait_writable(self) -> io::Result<()> {
        let mut request = libc::pollfd {
            fd: self.number(),
            events: libc::POLLOUT,
            revents: 0,
        };

        loop {
            // SAFETY: request is a live poll record and its descriptor remains open.
            if unsafe { libc::poll(&raw mut request, 1, -1) } >= 0 {
                return Ok(());
            }
            let error = io::Error::last_os_error();
            if error.kind() != io::ErrorKind::Interrupted {
                return Err(error);
            }
        }
    }
}

impl Descriptor {
    pub(crate) fn from_socket(socket: socket2::Socket) -> Self {
        Self::owned(socket.into())
    }

    pub(crate) fn with_socket<T>(
        &self,
        operation: impl FnOnce(&socket2::Socket) -> io::Result<T>,
    ) -> io::Result<T> {
        operation(&socket2::SockRef::from(&self.borrowed()))
    }

    pub(crate) fn owned(descriptor: OwnedFd) -> Self {
        Self {
            inner: Resource::Owned(descriptor),
        }
    }

    pub fn from_file(file: File) -> Self {
        Self::owned(file.into())
    }

    pub fn standard(stream: StandardStream) -> Result<Self> {
        let descriptor = Self {
            inner: Resource::Standard(stream),
        };
        descriptor.set_non_blocking(true)?;
        Ok(descriptor)
    }

    pub(crate) fn raw(&self) -> RawFd {
        match &self.inner {
            Resource::Owned(fd) | Resource::Signal { read: fd, .. } => fd.as_raw_fd(),
            Resource::Standard(stream) => stream.number(),
        }
    }

    pub(crate) fn borrowed(&self) -> BorrowedFd<'_> {
        // SAFETY: self owns or borrows the descriptor throughout the returned borrow.
        unsafe { BorrowedFd::borrow_raw(self.raw()) }
    }

    pub fn number(&self) -> i64 {
        i64::from(self.raw())
    }

    pub fn duplicate(number: i64) -> Result<Self> {
        let number = RawFd::try_from(number).map_err(|_| Error::bad_descriptor("fcntl"))?;
        // SAFETY: fcntl validates the number without dereferencing any pointers.
        let duplicate = unsafe { libc::fcntl(number, libc::F_DUPFD_CLOEXEC, 0) };
        if duplicate < 0 {
            return Err(Error::last("fcntl"));
        }

        // SAFETY: fcntl returned a new descriptor owned by this call.
        Ok(Self::owned(unsafe { OwnedFd::from_raw_fd(duplicate) }))
    }

    pub fn try_clone_file(&self) -> Result<File> {
        unix_io::fcntl_dupfd_cloexec(self.borrowed(), 0)
            .map(File::from)
            .map_err(|e| Error::new("fcntl", e))
    }

    pub fn file_for_lock(&self) -> Result<Arc<File>> {
        self.try_clone_file().map(Arc::new)
    }

    pub fn set_non_blocking(&self, enabled: bool) -> Result<()> {
        let flags = fs::fcntl_getfl(self.borrowed()).map_err(|e| Error::new("fcntl", e))?;
        let flags = if enabled {
            flags | fs::OFlags::NONBLOCK
        } else {
            flags & !fs::OFlags::NONBLOCK
        };

        fs::fcntl_setfl(self.borrowed(), flags).map_err(|e| Error::new("fcntl", e))
    }

    pub fn read(&self, maximum: usize) -> Result<Option<Vec<u8>>> {
        let mut bytes = Vec::<u8>::with_capacity(maximum);
        let count = if let Resource::Standard(stream) = &self.inner {
            let file = stream.file();
            // SAFETY: bytes has maximum writable spare bytes and file is a live C stream.
            let count =
                unsafe { libc::fread(bytes.as_mut_ptr().cast::<c_void>(), 1, maximum, file) };
            // SAFETY: file is a live C stream.
            if count == 0 && unsafe { libc::ferror(file) } != 0 {
                let error = io::Error::last_os_error();
                // SAFETY: file is a live C stream.
                unsafe { libc::clearerr(file) };
                if error.kind() == io::ErrorKind::WouldBlock {
                    return Ok(None);
                }
                return Err(Error::new("read", error));
            }

            count
        } else {
            match unix_io::read(self.borrowed(), bytes.spare_capacity_mut()) {
                Ok((initialized, _)) => initialized.len(),
                Err(Errno::AGAIN) => return Ok(None),
                Err(error) => return Err(Error::new("read", error)),
            }
        };

        // SAFETY: the read initialized exactly count bytes within capacity.
        unsafe { bytes.set_len(count) };
        Ok(Some(bytes))
    }

    pub fn write(&self, bytes: &[u8]) -> Result<usize> {
        match unix_io::write(self.borrowed(), bytes) {
            Ok(count) => Ok(count),
            Err(Errno::AGAIN) => Ok(0),
            Err(error) => Err(Error::new("write", error)),
        }
    }

    pub fn flush(&self) -> Result<()> {
        Ok(())
    }

    pub fn pipe() -> Result<(Self, Self)> {
        let (read, write) = pipe::pipe().map_err(|e| Error::new("pipe", e))?;
        close_on_exec(read.as_raw_fd())?;
        close_on_exec(write.as_raw_fd())?;
        Ok((Self::owned(read), Self::owned(write)))
    }

    pub fn socket_pair() -> Result<(Self, Self)> {
        let (first, second) = net::socketpair(
            net::AddressFamily::UNIX,
            net::SocketType::STREAM,
            net::SocketFlags::empty(),
            None,
        )
        .map_err(|e| Error::new("socketpair", e))?;
        close_on_exec(first.as_raw_fd())?;
        close_on_exec(second.as_raw_fd())?;
        let (first, second) = (Self::owned(first), Self::owned(second));
        first.set_non_blocking(true)?;
        second.set_non_blocking(true)?;
        Ok((first, second))
    }

    pub fn readiness(&self) -> Readiness {
        Readiness::Descriptor(self.raw())
    }

    pub fn is_ready(&self, interest: Interest) -> Result<bool> {
        let events = match interest {
            Interest::Readable => libc::POLLIN,
            Interest::Writable => libc::POLLOUT,
            Interest::ReadableOrWritable => libc::POLLIN | libc::POLLOUT,
        };
        let mut request = libc::pollfd {
            fd: self.raw(),
            events,
            revents: 0,
        };
        // SAFETY: request is live and self owns its descriptor throughout the call.
        let result = unsafe { libc::poll(&raw mut request, 1, 0) };
        if result < 0 {
            Err(Error::last("poll"))
        } else {
            Ok(result != 0)
        }
    }
}

pub(crate) fn close_on_exec(number: RawFd) -> Result<()> {
    // SAFETY: fcntl validates the descriptor, and this borrow does not close it.
    let fd = unsafe { BorrowedFd::borrow_raw(number) };
    let flags = unix_io::fcntl_getfd(fd).map_err(|e| Error::new("fcntl", e))?;
    unix_io::fcntl_setfd(fd, flags | unix_io::FdFlags::CLOEXEC).map_err(|e| Error::new("fcntl", e))
}
