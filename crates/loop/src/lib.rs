//! The single-threaded event loop behind Whim's colorless async.

#![deny(clippy::nursery, clippy::pedantic)]
#![forbid(unsafe_op_in_unsafe_fn)]

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
