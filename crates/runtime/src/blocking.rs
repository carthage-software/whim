#![deny(clippy::nursery, clippy::pedantic)]
#![expect(
    clippy::redundant_pub_crate,
    reason = "the pool is shared across sibling runtime modules"
)]

use std::collections::VecDeque;
use std::io;
use std::sync::Arc;
use std::sync::Mutex;
use std::sync::PoisonError;
use std::sync::mpsc;
use std::thread;
use std::thread::JoinHandle;

use whim_sqlite::Executor;
use whim_sqlite::Job as SqliteJob;

pub(crate) type Job = Box<dyn FnOnce() + Send + 'static>;

pub(crate) struct BlockingPool {
    general: Mutex<Option<Arc<Pool>>>,
    sqlite: Mutex<Option<Arc<Pool>>>,
}

impl BlockingPool {
    pub(crate) const fn new() -> Self {
        Self {
            general: Mutex::new(None),
            sqlite: Mutex::new(None),
        }
    }

    pub(crate) fn submit(&self, job: Job) -> io::Result<()> {
        with_pool(&self.general, "whim-blocking", |pool| pool.enqueue(job))?
    }

    pub(crate) fn sqlite_executor(&self) -> io::Result<Arc<dyn Executor>> {
        with_pool(&self.sqlite, "whim-sqlite", |pool| {
            Arc::new(SerialExecutor::new(Arc::clone(pool))) as Arc<dyn Executor>
        })
    }
}

fn with_pool<T>(
    slot: &Mutex<Option<Arc<Pool>>>,
    thread_name: &'static str,
    use_pool: impl FnOnce(&Arc<Pool>) -> T,
) -> io::Result<T> {
    let pool = {
        let mut slot = slot.lock().unwrap_or_else(PoisonError::into_inner);
        if slot.is_none() {
            *slot = Some(Arc::new(Pool::new(thread_name)?));
        }

        let Some(pool) = slot.as_ref() else {
            return Err(io::Error::other("the blocking worker pool did not start"));
        };
        let pool = Arc::clone(pool);
        drop(slot);
        pool
    };

    Ok(use_pool(&pool))
}

struct SerialQueue {
    jobs: VecDeque<SqliteJob>,
    running: bool,
}

struct SerialExecutor {
    pool: Arc<Pool>,
    queue: Arc<Mutex<SerialQueue>>,
}

impl SerialExecutor {
    fn new(pool: Arc<Pool>) -> Self {
        Self {
            pool,
            queue: Arc::new(Mutex::new(SerialQueue {
                jobs: VecDeque::new(),
                running: false,
            })),
        }
    }
}

impl Executor for SerialExecutor {
    fn submit(&self, job: SqliteJob) -> io::Result<()> {
        let start = {
            let mut queue = self.queue.lock().unwrap_or_else(PoisonError::into_inner);
            queue.jobs.push_back(job);
            if queue.running {
                false
            } else {
                queue.running = true;
                true
            }
        };

        if !start {
            return Ok(());
        }

        let queue = Arc::clone(&self.queue);
        self.pool.enqueue(Box::new(move || run_serial(&queue)))
    }
}

fn run_serial(queue: &Mutex<SerialQueue>) {
    loop {
        let job = {
            let mut queue = queue.lock().unwrap_or_else(PoisonError::into_inner);
            let job = queue.jobs.pop_front();
            let Some(job) = job else {
                queue.running = false;
                drop(queue);
                return;
            };

            drop(queue);
            job
        };

        job();
    }
}

struct Pool {
    sender: Option<mpsc::Sender<Job>>,
    workers: Vec<JoinHandle<()>>,
}

impl Pool {
    fn new(thread_name: &str) -> io::Result<Self> {
        let (sender, receiver) = mpsc::channel::<Job>();
        let receiver = Arc::new(Mutex::new(receiver));
        let count = worker_count();
        let mut workers = Vec::with_capacity(count);
        for index in 0..count {
            let receiver = Arc::clone(&receiver);
            match thread::Builder::new()
                .name(format!("{thread_name}-{index}"))
                .spawn(move || worker(&receiver))
            {
                Ok(worker) => workers.push(worker),
                Err(error) => {
                    drop(sender);
                    for worker in workers {
                        let _ = worker.join();
                    }

                    return Err(error);
                }
            }
        }

        Ok(Self {
            sender: Some(sender),
            workers,
        })
    }

    fn enqueue(&self, job: Job) -> io::Result<()> {
        self.sender
            .as_ref()
            .ok_or_else(|| io::Error::other("the blocking worker pool is closed"))?
            .send(job)
            .map_err(|_| io::Error::other("the blocking worker pool stopped"))
    }
}

fn worker_count() -> usize {
    thread::available_parallelism().map_or(4, |count| count.get().clamp(2, 8))
}

impl Drop for Pool {
    fn drop(&mut self) {
        self.sender.take();
        for worker in self.workers.drain(..) {
            let _ = worker.join();
        }
    }
}

fn worker(receiver: &Mutex<mpsc::Receiver<Job>>) {
    loop {
        let job = receiver
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .recv();
        match job {
            Ok(job) => job(),
            Err(_) => return,
        }
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Condvar;
    use std::time::Duration;

    use super::*;

    #[test]
    fn sqlite_workers_are_bounded_and_do_not_starve_general_jobs() {
        let pool = BlockingPool::new();
        let worker_count = worker_count();
        let gate = Arc::new((Mutex::new(false), Condvar::new()));
        let (started_sender, started_receiver) = mpsc::channel();
        let mut executors = Vec::with_capacity(worker_count + 1);

        for _ in 0..=worker_count {
            let Ok(executor) = pool.sqlite_executor() else {
                panic!("could not start a SQLite worker");
            };

            let worker_gate = Arc::clone(&gate);
            let worker_started = started_sender.clone();
            let submitted = executor.submit(Box::new(move || {
                let _ = worker_started.send(());
                let (released, condition) = &*worker_gate;
                let released = released.lock().unwrap_or_else(PoisonError::into_inner);
                drop(
                    condition
                        .wait_while(released, |released| !*released)
                        .unwrap_or_else(PoisonError::into_inner),
                );
            }));

            assert!(submitted.is_ok(), "could not submit a SQLite worker job");
            executors.push(executor);
        }

        drop(started_sender);
        let timeout = Duration::from_secs(2);
        let workers_started =
            (0..worker_count).all(|_| started_receiver.recv_timeout(timeout).is_ok());
        let extra_waited = started_receiver
            .recv_timeout(Duration::from_millis(50))
            .is_err();
        let (completed_sender, completed_receiver) = mpsc::channel();
        let general_submitted = pool
            .submit(Box::new(move || {
                let _ = completed_sender.send(());
            }))
            .is_ok();
        let general_completed = completed_receiver.recv_timeout(timeout).is_ok();

        let (released, condition) = &*gate;
        *released.lock().unwrap_or_else(PoisonError::into_inner) = true;
        condition.notify_all();
        let extra_completed = started_receiver.recv_timeout(timeout).is_ok();
        drop(executors);

        let sqlite_workers = {
            let sqlite = pool.sqlite.lock().unwrap_or_else(PoisonError::into_inner);
            let Some(sqlite_pool) = sqlite.as_ref() else {
                panic!("the SQLite worker pool did not start");
            };
            let workers = sqlite_pool.workers.len();
            drop(sqlite);
            workers
        };
        assert_eq!(sqlite_workers, worker_count);
        assert!(workers_started, "the SQLite worker pool did not fill");
        assert!(extra_waited, "the SQLite worker pool exceeded its bound");
        assert!(extra_completed, "the queued SQLite job did not resume");
        assert!(general_submitted, "the general worker rejected its job");
        assert!(
            general_completed,
            "blocked SQLite workers starved the general blocking pool"
        );
    }

    #[test]
    fn sqlite_executor_preserves_submission_order() {
        let pool = BlockingPool::new();
        let Ok(executor) = pool.sqlite_executor() else {
            panic!("could not start a SQLite executor");
        };
        let gate = Arc::new((Mutex::new(false), Condvar::new()));
        let (sender, receiver) = mpsc::channel();
        let first_gate = Arc::clone(&gate);
        let first_sender = sender.clone();
        assert!(
            executor
                .submit(Box::new(move || {
                    let _ = first_sender.send(1);
                    let (released, condition) = &*first_gate;
                    let released = released.lock().unwrap_or_else(PoisonError::into_inner);
                    drop(
                        condition
                            .wait_while(released, |released| !*released)
                            .unwrap_or_else(PoisonError::into_inner),
                    );
                }))
                .is_ok()
        );
        assert!(
            executor
                .submit(Box::new(move || {
                    let _ = sender.send(2);
                }))
                .is_ok()
        );

        let timeout = Duration::from_secs(2);
        assert_eq!(receiver.recv_timeout(timeout), Ok(1));
        assert!(receiver.recv_timeout(Duration::from_millis(50)).is_err());
        let (released, condition) = &*gate;
        *released.lock().unwrap_or_else(PoisonError::into_inner) = true;
        condition.notify_one();
        assert_eq!(receiver.recv_timeout(timeout), Ok(2));
    }
}
