use std::os::windows::io::BorrowedHandle;
use std::os::windows::io::BorrowedSocket;
use std::os::windows::io::RawHandle;
use std::os::windows::io::RawSocket;

/// A Windows source of readiness events.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum RawDescriptor {
    /// A Winsock socket.
    Socket(RawSocket),
    /// A waitable event, console input, or process handle.
    Waitable(RawHandle),
}

impl From<RawSocket> for RawDescriptor {
    fn from(socket: RawSocket) -> Self {
        Self::Socket(socket)
    }
}

#[expect(
    clippy::redundant_pub_crate,
    reason = "the private descriptor is shared by the reactor and scheduler"
)]
pub(crate) enum BorrowedDescriptor<'a> {
    Socket(BorrowedSocket<'a>),
    Waitable(BorrowedHandle<'a>),
}

impl BorrowedDescriptor<'_> {
    pub(crate) const unsafe fn borrow_raw(descriptor: RawDescriptor) -> Self {
        match descriptor {
            RawDescriptor::Socket(socket) => {
                // SAFETY: the caller keeps the source open for the borrow.
                Self::Socket(unsafe { BorrowedSocket::borrow_raw(socket) })
            }
            RawDescriptor::Waitable(handle) => {
                // SAFETY: the caller keeps the source open for the borrow.
                Self::Waitable(unsafe { BorrowedHandle::borrow_raw(handle) })
            }
        }
    }
}

impl<'a> From<BorrowedSocket<'a>> for BorrowedDescriptor<'a> {
    fn from(socket: BorrowedSocket<'a>) -> Self {
        Self::Socket(socket)
    }
}
