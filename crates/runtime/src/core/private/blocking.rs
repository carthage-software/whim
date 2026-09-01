//! Shared state for cancellable operations on the blocking worker pool.

use std::io;
use std::io::ErrorKind;
use std::mem::replace;
use std::os::fd::AsRawFd;
use std::os::unix::net::UnixDatagram;
use std::sync::Arc;
use std::sync::Mutex;
use std::sync::PoisonError;
use std::sync::atomic::AtomicBool;
use std::sync::atomic::Ordering;

enum Response<R, E> {
    Pending,
    Ready(Result<R, E>),
    Consumed,
}

pub(super) struct Operation<R, E> {
    response: Mutex<Response<R, E>>,
    cancelled: AtomicBool,
    reader: UnixDatagram,
    writer: UnixDatagram,
}

impl<R, E> Operation<R, E> {
    pub(super) fn new() -> io::Result<Arc<Self>> {
        let (reader, writer) = UnixDatagram::pair()?;
        reader.set_nonblocking(true)?;
        writer.set_nonblocking(true)?;

        Ok(Arc::new(Self {
            response: Mutex::new(Response::Pending),
            cancelled: AtomicBool::new(false),
            reader,
            writer,
        }))
    }

    pub(super) fn descriptor(&self) -> i32 {
        self.reader.as_raw_fd()
    }

    pub(super) fn complete(&self, result: Result<R, E>) {
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

    pub(super) fn is_complete(&self) -> bool {
        self.cancelled.load(Ordering::Acquire)
            || !matches!(
                *self.response.lock().unwrap_or_else(PoisonError::into_inner),
                Response::Pending
            )
    }

    pub(super) fn take(&self) -> Option<Result<Option<R>, E>> {
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

    pub(super) fn cancel(&self) {
        if !self.cancelled.swap(true, Ordering::AcqRel) {
            self.signal();
        }
    }

    pub(super) fn drain(&self) {
        let mut bytes = [0_u8; 64];
        loop {
            match self.reader.recv(&mut bytes) {
                Ok(_) => {}
                Err(error) if error.kind() == ErrorKind::WouldBlock => return,
                Err(_) => return,
            }
        }
    }

    pub(super) fn wait(&self) -> io::Result<()> {
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

    fn signal(&self) {
        let _ = self.writer.send(&[1]);
    }
}
