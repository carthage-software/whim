//! The single-threaded scheduler: the ready-queue of runnable coroutines, the
//! live-task set, the timer heaps, and the reactor the loop blocks on when
//! nothing is runnable.

use std::cmp::Reverse;
use std::collections::BinaryHeap;
use std::collections::VecDeque;
use std::io;
use std::os::fd::BorrowedFd;
use std::os::fd::RawFd;
use std::time::Duration;
use std::time::Instant;

use hashbrown::HashMap;

use crate::reactor::Interest;
use crate::reactor::Reactor;

const TIMER_COMPACTION_THRESHOLD: usize = 1024;
const ERROR_COMPACTION_THRESHOLD: usize = 1024;

/// The identity of one task, stable for the task's whole life.
///
/// Readiness sources use the raw id to find parked tasks. `Ord` allows timer
/// entries to use the id as a tie-breaker.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct TaskId(u64);

impl TaskId {
    /// Returns the numeric task identifier.
    #[must_use]
    pub const fn get(self) -> u64 {
        self.0
    }

    #[expect(
        clippy::cast_possible_truncation,
        reason = "this Whim build supports only 64-bit targets"
    )]
    const fn reactor_key(self) -> usize {
        self.0 as usize
    }
}

/// What a task's next activation feeds its coroutine.
pub enum Activation<V> {
    /// Starts a task with its arguments.
    Start(Vec<V>),
    /// Resumes a suspended task with a value.
    Resume(V),
    /// Resumes a suspended task by throwing a value.
    Throw(V),
}

/// A coroutine and its scheduler state.
#[expect(
    clippy::struct_excessive_bools,
    reason = "the scheduler flags are independent task state"
)]
struct Task<H, V> {
    coroutine: H,
    pending: Option<Activation<V>>,
    /// Whether the task is in a ready queue.
    queued: bool,
    microtask: bool,
    /// Whether this task keeps the loop alive.
    referenced: bool,
    ready_requested: bool,
    /// Changes on each wake so stale timers cannot resume a later suspension.
    generation: u64,
    timer_generation: Option<u64>,
}

/// A task ready for activation.
pub struct ReadyActivation<H, V> {
    /// The task identifier.
    pub id: TaskId,
    /// The coroutine handle.
    pub coroutine: H,
    /// The input for this activation.
    pub activation: Activation<V>,
}

/// The engine's event loop bookkeeping, generic over the task handle `H` and the
/// resume value `V`.
pub struct Scheduler<H, V> {
    reactor: Reactor,
    readiness: Vec<usize>,
    tasks: HashMap<TaskId, Task<H, V>>,
    referenced_tasks: usize,
    ready: VecDeque<TaskId>,
    microtasks: VecDeque<TaskId>,
    /// Descriptors awaited by parked tasks.
    waiters: HashMap<usize, RawFd>,
    /// Registrations retained between rearms.
    registrations: HashMap<usize, (RawFd, Interest)>,
    timers: BinaryHeap<Reverse<(Instant, TaskId, u64)>>,
    live_timers: usize,
    stale_timers: usize,
    current: Option<TaskId>,
    /// Unobserved errors and their cancellation identifiers.
    pending_errors: HashMap<u64, V>,
    pending_error_order: VecDeque<u64>,
    stale_errors: usize,
    next_error_id: u64,
    next_id: u64,
    idle: V,
}

impl<H: Clone, V: Clone> Scheduler<H, V> {
    /// `idle` is the value an internal timer/descriptor wake resumes a task with.
    ///
    /// # Errors
    ///
    /// Returns an operating-system error when the reactor cannot be opened.
    pub fn new(idle: V) -> io::Result<Self> {
        Ok(Self {
            reactor: Reactor::new()?,
            readiness: Vec::new(),
            tasks: HashMap::new(),
            referenced_tasks: 0,
            ready: VecDeque::new(),
            microtasks: VecDeque::new(),
            waiters: HashMap::new(),
            registrations: HashMap::new(),
            timers: BinaryHeap::new(),
            live_timers: 0,
            stale_timers: 0,
            current: None,
            pending_errors: HashMap::new(),
            pending_error_order: VecDeque::new(),
            stale_errors: 0,
            next_error_id: 0,
            next_id: 0,
            idle,
        })
    }

    /// Registers a fresh coroutine as a ready task and returns its id. The task
    /// starts by calling its callback with `arguments`.
    pub fn spawn(&mut self, coroutine: H, arguments: Vec<V>) -> TaskId {
        let id = self.insert_task(coroutine, arguments, false);
        self.mark_queued(id);
        id
    }

    /// Registers a fresh coroutine as a microtask. It runs before ordinary
    /// deferred work on the next scheduler turn.
    pub fn queue(&mut self, coroutine: H, arguments: Vec<V>) -> TaskId {
        let id = self.insert_task(coroutine, arguments, true);
        self.mark_queued(id);
        id
    }

    /// Adds a task that waits for a timer or descriptor before its first run.
    pub fn spawn_armed(&mut self, coroutine: H, arguments: Vec<V>) -> TaskId {
        self.insert_task(coroutine, arguments, false)
    }

    fn insert_task(&mut self, coroutine: H, arguments: Vec<V>, microtask: bool) -> TaskId {
        let id = TaskId(self.next_id);
        self.next_id += 1;
        self.tasks.insert(
            id,
            Task {
                coroutine,
                pending: Some(Activation::Start(arguments)),
                queued: false,
                microtask,
                referenced: true,
                ready_requested: false,
                generation: 0,
                timer_generation: None,
            },
        );
        self.referenced_tasks += 1;

        id
    }

    fn mark_queued(&mut self, id: TaskId) {
        self.mark_queued_with_priority(id, false);
    }

    fn mark_queued_front(&mut self, id: TaskId) {
        self.mark_queued_with_priority(id, true);
    }

    fn mark_queued_with_priority(&mut self, id: TaskId, front: bool) {
        let mut microtask = false;
        let mut queue = false;
        if let Some(task) = self.tasks.get_mut(&id) {
            task.ready_requested = true;
            if !task.queued {
                task.queued = true;
                microtask = task.microtask;
                queue = true;
            }
        }
        if queue {
            match (microtask, front) {
                (true, true) => self.microtasks.push_front(id),
                (true, false) => self.microtasks.push_back(id),
                (false, true) => self.ready.push_front(id),
                (false, false) => self.ready.push_back(id),
            }
        }
    }

    /// Arms an [armed task](Self::spawn_armed) to run at `deadline`. A watcher's
    /// timer, for `delay` (and, unreferenced, a timeout token).
    pub fn arm_timer(&mut self, id: TaskId, deadline: Instant) {
        if let Some(task) = self.tasks.get_mut(&id) {
            let generation = task.generation;
            if task.timer_generation.replace(generation).is_some() {
                self.stale_timers += 1;
            } else {
                self.live_timers += 1;
            }
            self.timers.push(Reverse((deadline, id, generation)));
            self.compact_timers();
        }
    }

    /// Arms an [armed task](Self::spawn_armed) to run when `fd` is ready for
    /// `interest`. A watcher's descriptor, for `onReadable`/`onWritable`.
    ///
    /// # Safety
    ///
    /// `fd` must remain open until the task fires or is disarmed.
    ///
    /// # Errors
    ///
    /// Returns an operating-system error when the descriptor cannot be watched.
    pub unsafe fn arm_descriptor(
        &mut self,
        id: TaskId,
        fd: RawFd,
        interest: Interest,
    ) -> io::Result<()> {
        let key = id.reactor_key();
        // SAFETY: the caller keeps `fd` open until this task fires or is disarmed.
        unsafe { self.register_descriptor(key, fd, interest)? };
        self.waiters.insert(key, fd);
        Ok(())
    }

    /// Marks a task as no longer keeping the loop alive.
    pub fn unreference(&mut self, id: TaskId) {
        if let Some(task) = self.tasks.get_mut(&id)
            && task.referenced
        {
            task.referenced = false;
            self.referenced_tasks -= 1;
        }
    }

    /// Makes a task keep the loop alive.
    pub fn reference(&mut self, id: TaskId) {
        if let Some(task) = self.tasks.get_mut(&id)
            && !task.referenced
        {
            task.referenced = true;
            self.referenced_tasks += 1;
        }
    }

    /// Returns the task whose activation is running.
    pub const fn current_task(&self) -> Option<TaskId> {
        self.current
    }

    /// Whether any task is still live, including unreferenced tasks.
    #[must_use]
    pub fn has_tasks(&self) -> bool {
        !self.tasks.is_empty()
    }

    /// Records `error` as unobserved, returning the id that cancels it.
    pub fn record_error(&mut self, error: V) -> u64 {
        let id = self.next_error_id;
        self.next_error_id = self.next_error_id.wrapping_add(1);
        self.pending_errors.insert(id, error);
        self.pending_error_order.push_back(id);
        id
    }

    /// Cancels the record made under `id`: the error was observed after all.
    pub fn forget_error(&mut self, id: u64) {
        if self.pending_errors.remove(&id).is_none() {
            return;
        }
        if self.pending_errors.is_empty() {
            self.pending_error_order.clear();
            self.stale_errors = 0;
        } else {
            self.stale_errors += 1;
            self.compact_errors();
        }
    }

    /// Takes the first error still unobserved, for the driver to surface.
    pub fn take_pending_error(&mut self) -> Option<V> {
        while let Some(id) = self.pending_error_order.pop_front() {
            if let Some(error) = self.pending_errors.remove(&id) {
                if self.pending_errors.is_empty() {
                    self.pending_error_order.clear();
                    self.stale_errors = 0;
                }

                return Some(error);
            }

            self.stale_errors = self.stale_errors.saturating_sub(1);
        }

        None
    }

    /// The next ready task to activate, marking it current, or `None` when the
    /// ready-queue is empty. The returned handles are owned, so the caller
    /// re-enters the interpreter without holding a borrow of the scheduler.
    pub fn next_activation(&mut self) -> Option<ReadyActivation<H, V>> {
        self.next_microtask_activation()
            .or_else(|| self.next_ready_activation())
    }

    /// Takes the next queued microtask.
    pub fn next_microtask_activation(&mut self) -> Option<ReadyActivation<H, V>> {
        self.next_queued_activation(true)
    }

    /// Takes the next ordinary ready task.
    pub fn next_ready_activation(&mut self) -> Option<ReadyActivation<H, V>> {
        self.next_queued_activation(false)
    }

    fn next_queued_activation(&mut self, microtask: bool) -> Option<ReadyActivation<H, V>> {
        loop {
            let id = if microtask {
                self.microtasks.pop_front()?
            } else {
                self.ready.pop_front()?
            };
            let Some(task) = self.tasks.get_mut(&id) else {
                continue;
            };

            task.queued = false;
            if !task.ready_requested {
                continue;
            }
            task.ready_requested = false;
            let Some(activation) = task.pending.take() else {
                continue;
            };

            let coroutine = task.coroutine.clone();
            self.current = Some(id);
            return Some(ReadyActivation {
                id,
                coroutine,
                activation,
            });
        }
    }

    /// Number of ordinary activations queued for the current turn. Ordinary
    /// work enqueued while those run belongs to the next turn.
    #[must_use]
    pub fn ready_count(&self) -> usize {
        self.ready.len()
    }

    /// Number of queued microtasks.
    #[must_use]
    pub fn microtask_count(&self) -> usize {
        self.microtasks.len()
    }

    /// Clears the task set by the last activation.
    pub const fn clear_current(&mut self) {
        self.current = None;
    }

    /// Whether no task keeps the loop alive: the loop is done. Unreferenced
    /// tasks (a pending timeout watcher, say) do not count, so they are
    /// abandoned when nothing referenced remains.
    pub const fn is_idle(&self) -> bool {
        self.referenced_tasks == 0
    }

    /// Whether anything can still wake a parked task: a task-timer, a parked
    /// descriptor, or a pending timeout cancel-timer. When the ready-queue is
    /// empty and this is false, the remaining tasks can never run, the loop is
    /// deadlocked.
    pub fn has_wake_source(&self) -> bool {
        self.live_timers != 0 || !self.waiters.is_empty()
    }

    /// Whether `id` is still live. A watcher is live from the moment it is armed
    /// until its callback has run, so this is how a `{main}` wait sees that the
    /// watcher it is driving the loop for has fired.
    pub fn has_task(&self, id: TaskId) -> bool {
        self.tasks.contains_key(&id)
    }

    /// Removes a finished (terminated or cancelled) task, dropping its handle.
    pub fn finish(&mut self, id: TaskId) {
        self.disarm(id);
        self.remove_task(id);
    }

    /// Cancels a watcher (or any task) by id: deregisters the descriptor it may
    /// be armed on, then drops it. A stale timer entry for it is skipped by the
    /// generation guard once the task is gone.
    pub fn cancel(&mut self, id: TaskId) {
        self.disarm(id);
        self.remove_task(id);
    }

    /// Deregisters the descriptor a task may be armed on without discarding
    /// the task, so a cooperative cancellation can still activate it.
    pub fn disarm(&mut self, id: TaskId) {
        let key = id.reactor_key();
        self.waiters.remove(&key);
        if let Some((fd, _)) = self.registrations.remove(&key) {
            // SAFETY: registration requires the descriptor to remain open until disarmed.
            let borrowed = unsafe { BorrowedFd::borrow_raw(fd) };
            let _ = self.reactor.deregister(borrowed);
        }
    }

    /// The coroutine handle of a live task, for the runtime to inspect before
    /// deciding how to cancel it.
    pub fn task_handle(&self, id: TaskId) -> Option<H> {
        self.tasks.get(&id).map(|task| task.coroutine.clone())
    }

    /// Re-queues a parked task to resume with `value`, and bumps its generation
    /// so any other delayed wake source for the same park becomes stale.
    pub fn wake(&mut self, id: TaskId, value: V) {
        self.enqueue(id, Activation::Resume(value));
    }

    /// Re-queues a parked task ahead of other ready tasks.
    pub fn wake_front(&mut self, id: TaskId, value: V) {
        self.enqueue_front(id, Activation::Resume(value));
    }

    /// Re-queues a parked task to resume by throwing `error` at its suspension
    /// point, with the same idempotency and generation guard as [`wake`](Self::wake).
    pub fn wake_throw(&mut self, id: TaskId, error: V) {
        self.enqueue(id, Activation::Throw(error));
    }

    fn enqueue(&mut self, id: TaskId, activation: Activation<V>) {
        self.enqueue_with_priority(id, activation, false);
    }

    fn enqueue_front(&mut self, id: TaskId, activation: Activation<V>) {
        self.enqueue_with_priority(id, activation, true);
    }

    fn enqueue_with_priority(&mut self, id: TaskId, activation: Activation<V>, front: bool) {
        let invalidated_timer = {
            let Some(task) = self.tasks.get_mut(&id) else {
                return;
            };
            if task.queued || task.ready_requested {
                return;
            }

            task.pending = Some(activation);
            task.generation = task.generation.wrapping_add(1);
            task.timer_generation.take().is_some()
        };
        self.disarm(id);
        if invalidated_timer {
            self.live_timers -= 1;
            self.stale_timers += 1;
            self.compact_timers();
        }
        if front {
            self.mark_queued_front(id);
        } else {
            self.mark_queued(id);
        }
    }

    fn fire(&mut self, id: TaskId) {
        let idle = self.idle.clone();
        let invalidated_timer = if let Some(task) = self.tasks.get_mut(&id)
            && task.pending.is_none()
        {
            task.pending = Some(Activation::Resume(idle));
            task.generation = task.generation.wrapping_add(1);
            task.timer_generation.take().is_some()
        } else {
            false
        };
        if invalidated_timer {
            self.live_timers -= 1;
            self.stale_timers += 1;
            self.compact_timers();
        }
        self.mark_queued(id);
    }

    fn wake_key(&mut self, key: usize) {
        if self.waiters.contains_key(&key) {
            let task = TaskId(key as u64);
            self.disarm(task);
            self.fire(task);
        }
    }

    /// Blocks until a parked source is ready or the nearest timer is due, then
    /// wakes every task whose descriptor fired or whose timer expired.
    ///
    /// # Errors
    ///
    /// Returns an operating-system error when polling fails.
    pub fn poll_reactor(&mut self) -> io::Result<()> {
        let timeout = self.next_timeout();
        self.poll_reactor_with_timeout(timeout)
    }

    /// Polls ready descriptors and expired timers without blocking.
    ///
    /// # Errors
    ///
    /// Returns an operating-system error when polling fails.
    pub fn poll_reactor_nonblocking(&mut self) -> io::Result<()> {
        if self.waiters.is_empty() {
            self.wake_expired_timers();
            return Ok(());
        }

        self.poll_reactor_with_timeout(Some(Duration::ZERO))
    }

    fn poll_reactor_with_timeout(&mut self, timeout: Option<Duration>) -> io::Result<()> {
        self.reactor.wait(timeout, &mut self.readiness)?;
        for index in 0..self.readiness.len() {
            let key = self.readiness[index];
            self.wake_key(key);
        }

        self.wake_expired_timers();
        Ok(())
    }

    /// Wakes every task whose timer deadline has passed, in deadline order,
    /// skipping a timer whose task has since been woken or finished.
    fn wake_expired_timers(&mut self) {
        let now = Instant::now();
        while self
            .timers
            .peek()
            .is_some_and(|Reverse((deadline, _, _))| *deadline <= now)
        {
            let Some(Reverse((_, task, generation))) = self.timers.pop() else {
                continue;
            };
            let live = self.tasks.get_mut(&task).is_some_and(|scheduled| {
                if scheduled.generation != generation
                    || scheduled.timer_generation != Some(generation)
                {
                    return false;
                }

                scheduled.timer_generation = None;
                true
            });
            if live {
                self.live_timers -= 1;
                self.fire(task);
            } else {
                self.stale_timers = self.stale_timers.saturating_sub(1);
            }
        }
    }

    fn next_timeout(&mut self) -> Option<Duration> {
        self.prune_stale_timer_head();
        self.timers
            .peek()
            .map(|Reverse((deadline, _, _))| deadline.saturating_duration_since(Instant::now()))
    }

    /// Arms the current task for descriptor readiness.
    ///
    /// # Safety
    ///
    /// `fd` must remain open until the current task wakes or is disarmed.
    ///
    /// # Errors
    ///
    /// Returns an operating-system error when the descriptor cannot be watched.
    pub unsafe fn park_current_on_descriptor(
        &mut self,
        fd: RawFd,
        interest: Interest,
    ) -> io::Result<()> {
        let Some(task) = self.current else {
            return Ok(());
        };

        let key = task.reactor_key();
        // SAFETY: the caller keeps `fd` open until the current task wakes or is disarmed.
        unsafe { self.register_descriptor(key, fd, interest)? };
        self.waiters.insert(key, fd);
        Ok(())
    }

    unsafe fn register_descriptor(
        &mut self,
        key: usize,
        fd: RawFd,
        interest: Interest,
    ) -> io::Result<()> {
        if let Some((registered, registered_interest)) = self.registrations.get(&key).copied() {
            if registered == fd {
                if registered_interest == interest && !self.reactor.requires_rearm() {
                    return Ok(());
                }
                // SAFETY: the caller keeps `fd` open while it is registered.
                let borrowed = unsafe { BorrowedFd::borrow_raw(fd) };
                self.reactor.rearm(borrowed, key, interest)?;
                self.registrations.insert(key, (fd, interest));
                return Ok(());
            }

            // SAFETY: registered descriptors remain open until removed.
            let borrowed = unsafe { BorrowedFd::borrow_raw(registered) };
            self.reactor.deregister(borrowed)?;
            self.registrations.remove(&key);
        }

        // SAFETY: the caller keeps `fd` open while it is registered.
        let borrowed = unsafe { BorrowedFd::borrow_raw(fd) };
        // SAFETY: the same caller contract holds until deregistration.
        unsafe { self.reactor.register(borrowed, key, interest)? };
        self.registrations.insert(key, (fd, interest));
        Ok(())
    }

    /// Parks the current task until `deadline`. The caller suspends after this;
    /// the loop wakes the task once the deadline passes, unless it was already
    /// woken (the recorded generation would then be stale). A no-op with no
    /// current task.
    pub fn park_current_on_timer(&mut self, deadline: Instant) {
        if let Some(task) = self.current {
            self.arm_timer(task, deadline);
        }
    }

    fn remove_task(&mut self, id: TaskId) {
        let Some(task) = self.tasks.remove(&id) else {
            return;
        };
        if task.referenced {
            self.referenced_tasks -= 1;
        }

        if task.timer_generation.is_some() {
            self.live_timers -= 1;
            self.stale_timers += 1;
            self.compact_timers();
        }
    }

    fn prune_stale_timer_head(&mut self) {
        while self
            .timers
            .peek()
            .is_some_and(|Reverse((_, task, generation))| {
                self.tasks.get(task).is_none_or(|scheduled| {
                    scheduled.generation != *generation
                        || scheduled.timer_generation != Some(*generation)
                })
            })
        {
            self.timers.pop();
            self.stale_timers = self.stale_timers.saturating_sub(1);
        }
    }

    fn compact_timers(&mut self) {
        if self.stale_timers < TIMER_COMPACTION_THRESHOLD
            || self.stale_timers < self.timers.len().div_ceil(2)
        {
            return;
        }

        let tasks = &self.tasks;
        self.timers.retain(|Reverse((_, task, generation))| {
            tasks.get(task).is_some_and(|scheduled| {
                scheduled.generation == *generation
                    && scheduled.timer_generation == Some(*generation)
            })
        });
        self.stale_timers = 0;
    }

    fn compact_errors(&mut self) {
        if self.stale_errors < ERROR_COMPACTION_THRESHOLD
            || self.stale_errors < self.pending_error_order.len().div_ceil(2)
        {
            return;
        }

        let pending = &self.pending_errors;
        self.pending_error_order
            .retain(|id| pending.contains_key(id));
        self.stale_errors = 0;
    }
}

#[cfg(test)]
mod tests {
    use std::io::Write;
    use std::os::fd::AsRawFd;
    use std::os::unix::net::UnixStream;
    use std::time::Duration;
    use std::time::Instant;

    use crate::reactor::Interest;
    use crate::scheduler::Activation;
    use crate::scheduler::ERROR_COMPACTION_THRESHOLD;
    use crate::scheduler::Scheduler;
    use crate::scheduler::TIMER_COMPACTION_THRESHOLD;

    #[test]
    fn observed_errors_do_not_leave_an_unbounded_order_queue() {
        let mut scheduler: Scheduler<(), usize> =
            Scheduler::new(0).expect("the test reactor opens");
        let retained = scheduler.record_error(1);
        for value in 0..ERROR_COMPACTION_THRESHOLD * 3 {
            let id = scheduler.record_error(value);
            scheduler.forget_error(id);
        }

        assert_eq!(scheduler.pending_errors.len(), 1);
        assert!(scheduler.pending_error_order.len() <= ERROR_COMPACTION_THRESHOLD);
        assert_eq!(scheduler.take_pending_error(), Some(1));
        scheduler.forget_error(retained);
    }

    #[test]
    fn microtasks_are_fifo_and_run_before_deferred_tasks() {
        let mut scheduler = Scheduler::new(0).expect("the test reactor opens");
        let deferred = scheduler.spawn(1, Vec::new());
        let first_microtask = scheduler.queue(2, Vec::new());
        let second_microtask = scheduler.queue(3, Vec::new());

        assert_eq!(scheduler.ready_count(), 1);
        assert_eq!(scheduler.microtask_count(), 2);
        assert_eq!(
            scheduler.next_activation().map(|activation| activation.id),
            Some(first_microtask),
        );
        assert_eq!(
            scheduler.next_activation().map(|activation| activation.id),
            Some(second_microtask),
        );
        assert_eq!(
            scheduler.next_activation().map(|activation| activation.id),
            Some(deferred),
        );
    }

    #[test]
    fn front_wake_precedes_ordinary_ready_work() {
        let mut scheduler = Scheduler::new(0).expect("the test reactor opens");
        let resumed = scheduler.spawn(1, Vec::new());
        assert_eq!(
            scheduler.next_activation().map(|activation| activation.id),
            Some(resumed),
        );
        scheduler.clear_current();
        let ordinary = scheduler.spawn(2, Vec::new());

        scheduler.wake_front(resumed, 7);

        let resumed_activation = scheduler
            .next_activation()
            .expect("the front wake queues an activation");
        assert_eq!(resumed_activation.id, resumed);
        assert!(matches!(
            resumed_activation.activation,
            Activation::Resume(7)
        ));
        assert_eq!(
            scheduler.next_activation().map(|activation| activation.id),
            Some(ordinary),
        );
    }

    #[test]
    fn idle_state_tracks_task_references() {
        let mut scheduler = Scheduler::new(0).expect("the test reactor opens");
        assert!(scheduler.is_idle());

        let first = scheduler.spawn_armed(1, Vec::new());
        let second = scheduler.spawn_armed(2, Vec::new());
        assert_eq!(scheduler.referenced_tasks, 2);
        assert!(!scheduler.is_idle());

        scheduler.unreference(first);
        scheduler.unreference(first);
        assert_eq!(scheduler.referenced_tasks, 1);
        assert!(!scheduler.is_idle());

        scheduler.unreference(second);
        assert_eq!(scheduler.referenced_tasks, 0);
        assert!(scheduler.is_idle());

        scheduler.reference(first);
        scheduler.reference(first);
        assert_eq!(scheduler.referenced_tasks, 1);
        assert!(!scheduler.is_idle());

        scheduler.cancel(second);
        assert_eq!(scheduler.referenced_tasks, 1);
        scheduler.finish(first);
        assert_eq!(scheduler.referenced_tasks, 0);
        assert!(scheduler.is_idle());
    }

    #[test]
    fn pending_errors_preserve_order_and_can_be_forgotten() {
        let mut scheduler: Scheduler<(), _> = Scheduler::new(0).expect("the test reactor opens");
        scheduler.record_error(1);
        let forgotten = scheduler.record_error(2);
        scheduler.record_error(3);

        scheduler.forget_error(forgotten);
        scheduler.forget_error(forgotten);

        assert_eq!(scheduler.take_pending_error(), Some(1));
        assert_eq!(scheduler.take_pending_error(), Some(3));
        assert_eq!(scheduler.take_pending_error(), None);

        scheduler.record_error(4);
        let trailing = scheduler.record_error(5);
        scheduler.forget_error(trailing);
        assert_eq!(scheduler.take_pending_error(), Some(4));
        assert!(scheduler.pending_error_order.is_empty());
    }

    #[test]
    fn forgetting_every_pending_error_clears_its_order() {
        let mut scheduler: Scheduler<(), _> = Scheduler::new(0).expect("the test reactor opens");
        let errors: Vec<_> = (0..1_000)
            .map(|error| scheduler.record_error(error))
            .collect();

        for error in errors {
            scheduler.forget_error(error);
        }

        assert!(scheduler.pending_errors.is_empty());
        assert!(scheduler.pending_error_order.is_empty());
        assert_eq!(scheduler.take_pending_error(), None);
    }

    #[test]
    fn cancelled_timers_do_not_accumulate() {
        let mut scheduler = Scheduler::new(0).expect("the test reactor opens");
        for _ in 0..10_000 {
            let task = scheduler.spawn_armed(1, Vec::new());
            scheduler.arm_timer(task, Instant::now() + Duration::from_mins(1));
            scheduler.cancel(task);
        }

        assert_eq!(scheduler.live_timers, 0);
        assert!(scheduler.timers.len() < TIMER_COMPACTION_THRESHOLD);
        assert!(!scheduler.has_wake_source());
    }

    #[test]
    fn external_wake_disarms_a_parked_descriptor() {
        let Ok(mut scheduler) = Scheduler::new(0) else {
            panic!("the test reactor must open");
        };
        let task = scheduler.spawn(1, Vec::new());
        assert!(scheduler.next_activation().is_some());
        let Ok((reader, _writer)) = UnixStream::pair() else {
            panic!("the socket pair must open");
        };
        // SAFETY: `reader` remains open until the task is woken and disarmed.
        let watched =
            unsafe { scheduler.park_current_on_descriptor(reader.as_raw_fd(), Interest::Readable) };
        let Ok(()) = watched else {
            panic!("the descriptor must be registered");
        };
        assert!(scheduler.has_wake_source());

        scheduler.wake_throw(task, 7);

        assert!(!scheduler.has_wake_source());
        let Some(activation) = scheduler.next_activation() else {
            panic!("the external wake must queue the parked task");
        };
        assert!(matches!(activation.activation, Activation::Throw(7)));
    }

    #[test]
    fn descriptor_readiness_disarms_before_resuming_the_task() {
        let Ok(mut scheduler) = Scheduler::new(0) else {
            panic!("the test reactor must open");
        };
        let task = scheduler.spawn(1, Vec::new());
        assert!(scheduler.next_activation().is_some());
        let Ok((reader, mut writer)) = UnixStream::pair() else {
            panic!("the socket pair must open");
        };
        // SAFETY: `reader` remains open until the readiness event disarms it.
        let watched =
            unsafe { scheduler.park_current_on_descriptor(reader.as_raw_fd(), Interest::Readable) };
        let Ok(()) = watched else {
            panic!("the descriptor must be registered");
        };
        assert!(scheduler.registrations.contains_key(&task.reactor_key()));
        writer
            .write_all(&[0])
            .expect("the socket pair must remain open");

        scheduler
            .poll_reactor_nonblocking()
            .expect("the readable descriptor must wake the reactor");

        assert!(!scheduler.registrations.contains_key(&task.reactor_key()));
        let Some(activation) = scheduler.next_activation() else {
            panic!("the descriptor must resume its task");
        };
        assert_eq!(activation.id, task);
    }
}
