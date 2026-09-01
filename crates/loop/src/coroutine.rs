//! The stackful coroutine primitive behind Whim's colorless async.

use std::io;
use std::ptr;

use corosensei::CoroutineResult;
use corosensei::stack::DefaultStack;

/// An owned, reusable coroutine stack.
pub struct Stack(DefaultStack);

impl Stack {
    /// Allocates a fresh coroutine stack of at least `bytes` usable bytes.
    ///
    /// # Errors
    ///
    /// Returns an operating-system error when the stack cannot be allocated.
    pub fn new(bytes: usize) -> io::Result<Self> {
        DefaultStack::new(bytes).map(Stack)
    }
}

/// Lets a coroutine suspend itself.
///
/// Its layout must match `corosensei::Yielder`.
#[repr(transparent)]
pub struct Yielder<I, Y>(corosensei::Yielder<I, Y>);

impl<I, Y> Yielder<I, Y> {
    /// Suspends the coroutine with `value` and returns its next input.
    pub fn suspend(&self, value: Y) -> I {
        self.0.suspend(value)
    }
}

/// The result of resuming a coroutine.
pub enum Resumption<Y, R> {
    /// The coroutine suspended with a value.
    Suspended(Y),
    /// The coroutine returned and its stack may be reused.
    Finished(R),
}

/// A resumable stackful coroutine.
pub struct Coroutine<I, Y, R>(corosensei::Coroutine<I, Y, R, DefaultStack>);

impl<I, Y, R> Coroutine<I, Y, R> {
    /// Builds a coroutine on the supplied pooled `stack`, whose body is `body`.
    pub fn with_stack<F>(stack: Stack, body: F) -> Self
    where
        F: FnOnce(&Yielder<I, Y>, I) -> R + 'static,
        I: 'static,
        Y: 'static,
        R: 'static,
    {
        Self(corosensei::Coroutine::with_stack(
            stack.0,
            move |yielder, input| {
                // SAFETY: `Yielder` is transparent over this value's type.
                let yielder = unsafe { &*ptr::from_ref(yielder).cast::<Yielder<I, Y>>() };
                body(yielder, input)
            },
        ))
    }

    /// Resumes the coroutine with `input`.
    #[expect(
        clippy::inline_always,
        reason = "coroutine transitions are a hot VM boundary"
    )]
    #[inline(always)]
    pub fn resume(&mut self, input: I) -> Resumption<Y, R> {
        match self.0.resume(input) {
            CoroutineResult::Yield(value) => Resumption::Suspended(value),
            CoroutineResult::Return(value) => Resumption::Finished(value),
        }
    }

    /// Reclaims the coroutine's [`Stack`] for reuse.
    #[expect(clippy::inline_always, reason = "stack recycling is a hot VM boundary")]
    #[inline(always)]
    #[must_use]
    pub fn into_stack(self) -> Stack {
        Stack(self.0.into_stack())
    }

    /// Discards a suspended coroutine's execution without unwinding its stack.
    ///
    /// # Safety
    ///
    /// The coroutine must never be resumed again.
    pub unsafe fn force_reset(&mut self) {
        // SAFETY: the caller promises the coroutine is never resumed again.
        unsafe { self.0.force_reset() };
    }
}
