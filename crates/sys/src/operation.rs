//! Shared state for cancellable operations on the blocking worker pool.

use std::io;
#[cfg(unix)]
use std::io::ErrorKind;
use std::mem::replace;
#[cfg(unix)]
use std::os::fd::AsRawFd;
#[cfg(unix)]
use std::os::unix::net::UnixDatagram;
#[cfg(windows)]
use std::os::windows::io::AsRawHandle;
#[cfg(windows)]
use std::os::windows::io::FromRawHandle;
#[cfg(windows)]
use std::os::windows::io::OwnedHandle;
#[cfg(windows)]
use std::ptr;
use std::sync::Arc;
use std::sync::Mutex;
use std::sync::PoisonError;
use std::sync::atomic::AtomicBool;
use std::sync::atomic::Ordering;
use whim_loop::RawDescriptor;
#[cfg(windows)]
use windows_sys::Win32::Foundation::WAIT_FAILED;
#[cfg(windows)]
use windows_sys::Win32::System::Threading::{
    CreateEventW, INFINITE, ResetEvent, SetEvent, WaitForSingleObject,
};

enum Response<R, E> {
    Pending,
    Ready(Result<R, E>),
    Consumed,
}

pub struct Operation<R, E> {
    response: Mutex<Response<R, E>>,
    cancelled: AtomicBool,
    #[cfg(unix)]
    reader: UnixDatagram,
    #[cfg(unix)]
    writer: UnixDatagram,
    #[cfg(windows)]
    event: OwnedHandle,
}

impl<R, E> Operation<R, E> {
    pub fn new() -> io::Result<Arc<Self>> {
        #[cfg(unix)]
        let (reader, writer) = {
            let (reader, writer) = UnixDatagram::pair()?;
            reader.set_nonblocking(true)?;
            writer.set_nonblocking(true)?;
            (reader, writer)
        };

        #[cfg(windows)]
        let event = {
            // SAFETY: no name or security descriptor is supplied; the returned event is owned here.
            let event = unsafe { CreateEventW(ptr::null(), 1, 0, ptr::null()) };
            if event.is_null() {
                return Err(io::Error::last_os_error());
            }

            // SAFETY: CreateEventW returned a new, valid handle.
            unsafe { OwnedHandle::from_raw_handle(event) }
        };

        Ok(Arc::new(Self {
            response: Mutex::new(Response::Pending),
            cancelled: AtomicBool::new(false),
            #[cfg(unix)]
            reader,
            #[cfg(unix)]
            writer,
            #[cfg(windows)]
            event,
        }))
    }

    #[cfg(unix)]
    pub fn descriptor(&self) -> RawDescriptor {
        self.reader.as_raw_fd()
    }

    #[cfg(windows)]
    pub fn descriptor(&self) -> RawDescriptor {
        RawDescriptor::Waitable(self.event.as_raw_handle())
    }

    pub fn complete(&self, result: Result<R, E>) {
        if self.cancelled.load(Ordering::Acquire) {
            return;
        }

        {
            let mut response = self.response.lock().unwrap_or_else(PoisonError::into_inner);
            if !matches!(*response, Response::Pending) {
                return;
            }

            *response = Response::Ready(result);
        }

        self.signal();
    }

    pub fn is_complete(&self) -> bool {
        self.cancelled.load(Ordering::Acquire)
            || !matches!(
                *self.response.lock().unwrap_or_else(PoisonError::into_inner),
                Response::Pending
            )
    }

    pub fn take(&self) -> Option<Result<Option<R>, E>> {
        let mut response = self.response.lock().unwrap_or_else(PoisonError::into_inner);
        if self.cancelled.load(Ordering::Acquire) {
            *response = Response::Consumed;
            return Some(Ok(None));
        }

        let Response::Ready(_) = &*response else {
            return None;
        };
        let Response::Ready(result) = replace(&mut *response, Response::Consumed) else {
            return None;
        };
        drop(response);

        Some(result.map(Some))
    }

    pub fn cancel(&self) {
        if !self.cancelled.swap(true, Ordering::AcqRel) {
            self.signal();
        }
    }

    #[cfg(unix)]
    pub fn drain(&self) {
        let mut bytes = [0_u8; 64];
        loop {
            match self.reader.recv(&mut bytes) {
                Ok(_) => {}
                Err(error) if error.kind() == ErrorKind::WouldBlock => return,
                Err(_) => return,
            }
        }
    }

    #[cfg(windows)]
    pub fn drain(&self) {
        // SAFETY: the operation owns the event throughout this call.
        unsafe { ResetEvent(self.event.as_raw_handle()) };
    }

    #[cfg(unix)]
    pub fn wait(&self) -> io::Result<()> {
        let mut request = libc::pollfd {
            fd: self.reader.as_raw_fd(),
            events: libc::POLLIN,
            revents: 0,
        };
        loop {
            // SAFETY: `request` remains live and describes the operation's reader.
            if unsafe { libc::poll(&raw mut request, 1, -1) } >= 0 {
                return Ok(());
            }

            let error = io::Error::last_os_error();
            if error.kind() != ErrorKind::Interrupted {
                return Err(error);
            }
        }
    }

    #[cfg(windows)]
    pub fn wait(&self) -> io::Result<()> {
        // SAFETY: the operation owns the event throughout the wait.
        if unsafe { WaitForSingleObject(self.event.as_raw_handle(), INFINITE) } == WAIT_FAILED {
            return Err(io::Error::last_os_error());
        }

        Ok(())
    }

    #[cfg(unix)]
    fn signal(&self) {
        let _ = self.writer.send(&[1]);
    }

    #[cfg(windows)]
    fn signal(&self) {
        // SAFETY: the operation owns the event throughout this call.
        unsafe { SetEvent(self.event.as_raw_handle()) };
    }
}
