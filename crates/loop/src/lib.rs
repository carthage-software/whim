//! The single-threaded event loop behind Whim's colorless async.
//! Watches file descriptors on Unix and sockets on Windows.

#![deny(clippy::nursery, clippy::pedantic)]
#![forbid(unsafe_op_in_unsafe_fn)]

#[cfg(unix)]
use std::os::fd::BorrowedFd as BorrowedDescriptor;
#[cfg(unix)]
pub use std::os::fd::RawFd as RawDescriptor;
#[cfg(windows)]
use std::os::windows::io::BorrowedSocket as BorrowedDescriptor;
#[cfg(windows)]
pub use std::os::windows::io::RawSocket as RawDescriptor;

mod coroutine;
#[expect(
    clippy::redundant_pub_crate,
    reason = "the private reactor is shared with its sibling scheduler module"
)]
mod reactor;
mod scheduler;

pub use coroutine::Coroutine;
pub use coroutine::Resumption;
pub use coroutine::Stack;
pub use coroutine::Yielder;
pub use reactor::Interest;
pub use scheduler::Activation;
pub use scheduler::ReadyActivation;
pub use scheduler::Scheduler;
pub use scheduler::TaskId;
