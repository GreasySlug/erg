//! A bounded pool of worker threads for the LSP request handlers.
//!
//! Each request type used to get a thread of its own: some three dozen threads,
//! each with a 16 MiB stack reserved and a clone of the `Server`, for work that
//! is at most [`MAX_WORKERS`](crate::scheduler::MAX_WORKERS) wide anyway, since
//! the scheduler admits no more requests than that at once. The pool keeps that
//! many threads, each owning one worker context (a `Server` clone), and runs the
//! jobs a dispatcher queues: a job is ordered by the priority the scheduler
//! gives its request kind, then by arrival.

use std::cmp::Ordering;
use std::collections::BinaryHeap;
use std::fmt;
use std::panic::{self, AssertUnwindSafe};
use std::sync::{Arc, Condvar, Mutex, MutexGuard};

use erg_common::lsp_log;
use erg_common::spawn::spawn_new_thread;

/// One unit of work for a pool worker: the request's id, its priority (lower
/// runs sooner) and what to do with the worker's context.
pub struct Job<T> {
    pub id: i64,
    pub priority: u8,
    pub run: Box<dyn FnOnce(&mut T) + Send>,
}

impl<T> Job<T> {
    pub fn new(id: i64, priority: u8, run: impl FnOnce(&mut T) + Send + 'static) -> Self {
        Self {
            id,
            priority,
            run: Box::new(run),
        }
    }
}

impl<T> fmt::Debug for Job<T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Job")
            .field("id", &self.id)
            .field("priority", &self.priority)
            .finish_non_exhaustive()
    }
}

impl<T> PartialEq for Job<T> {
    fn eq(&self, other: &Self) -> bool {
        self.priority == other.priority && self.id == other.id
    }
}

impl<T> Eq for Job<T> {}

/// `BinaryHeap` pops its greatest element, so the job that should run first
/// -- the lower priority value, and of two equal ones the earlier id -- is
/// made the greater.
impl<T> Ord for Job<T> {
    fn cmp(&self, other: &Self) -> Ordering {
        other
            .priority
            .cmp(&self.priority)
            .then_with(|| other.id.cmp(&self.id))
    }
}

impl<T> PartialOrd for Job<T> {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

struct Queue<T> {
    jobs: BinaryHeap<Job<T>>,
    /// No more jobs will come; the workers exit once `jobs` is empty.
    closed: bool,
}

/// The pool; a handle, cheap to clone, over the shared queue.
#[derive(Debug)]
pub struct ThreadPool<T> {
    queue: Arc<(Mutex<Queue<T>>, Condvar)>,
}

impl<T> Clone for ThreadPool<T> {
    fn clone(&self) -> Self {
        Self {
            queue: self.queue.clone(),
        }
    }
}

impl<T> fmt::Debug for Queue<T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Queue")
            .field("jobs", &self.jobs.len())
            .field("closed", &self.closed)
            .finish()
    }
}

impl<T: Send + 'static> ThreadPool<T> {
    /// Spawns `size` workers (at least one), named `{name}_{i}`, each owning the
    /// context `make_context` returns for it.
    pub fn new(size: usize, name: &str, mut make_context: impl FnMut() -> T) -> Self {
        let queue = Arc::new((
            Mutex::new(Queue {
                jobs: BinaryHeap::new(),
                closed: false,
            }),
            Condvar::new(),
        ));
        for i in 0..size.max(1) {
            let queue = queue.clone();
            let mut context = make_context();
            spawn_new_thread(
                move || worker_loop(&queue, &mut context),
                &format!("{name}_{i}"),
            );
        }
        Self { queue }
    }

    /// Queues a job for the next free worker. Returns the job if the pool has
    /// been closed and will not run it.
    pub fn submit(&self, job: Job<T>) -> Result<(), Job<T>> {
        let (lock, cvar) = &*self.queue;
        let mut queue = lock_ignoring_poison(lock);
        if queue.closed {
            return Err(job);
        }
        queue.jobs.push(job);
        cvar.notify_one();
        Ok(())
    }

    /// Accepts no more jobs. The workers finish what is queued and then exit.
    pub fn close(&self) {
        let (lock, cvar) = &*self.queue;
        lock_ignoring_poison(lock).closed = true;
        cvar.notify_all();
    }

    #[cfg(test)]
    fn pending(&self) -> usize {
        lock_ignoring_poison(&self.queue.0).jobs.len()
    }
}

/// The lock is held only around the queue itself, never while a job runs, so a
/// poisoned lock says nothing about the queue's state.
fn lock_ignoring_poison<T>(lock: &Mutex<Queue<T>>) -> MutexGuard<'_, Queue<T>> {
    lock.lock().unwrap_or_else(|e| e.into_inner())
}

fn worker_loop<T>(queue: &(Mutex<Queue<T>>, Condvar), context: &mut T) {
    let (lock, cvar) = queue;
    loop {
        let job = {
            let mut queue = lock_ignoring_poison(lock);
            loop {
                if let Some(job) = queue.jobs.pop() {
                    break Some(job);
                }
                if queue.closed {
                    break None;
                }
                queue = cvar.wait(queue).unwrap_or_else(|e| e.into_inner());
            }
        };
        let Some(Job { id, run, .. }) = job else {
            return;
        };
        // A handler that panics must not take a worker with it: the pool is all
        // the workers there are, and a request type has no thread of its own to
        // fall back on.
        if panic::catch_unwind(AssertUnwindSafe(|| run(context))).is_err() {
            lsp_log!("a worker panicked while handling request {id}");
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering as AtomicOrdering};
    use std::sync::mpsc;
    use std::time::Duration;

    #[test]
    fn runs_every_job() {
        let pool = ThreadPool::new(2, "test_worker", || ());
        let counter = Arc::new(AtomicUsize::new(0));
        let (done_tx, done_rx) = mpsc::channel();
        for i in 0..5 {
            let counter = counter.clone();
            let done_tx = done_tx.clone();
            pool.submit(Job::new(i, 50, move |_: &mut ()| {
                counter.fetch_add(1, AtomicOrdering::SeqCst);
                let _ = done_tx.send(());
            }))
            .unwrap();
        }
        for _ in 0..5 {
            done_rx.recv_timeout(Duration::from_secs(5)).unwrap();
        }
        assert_eq!(counter.load(AtomicOrdering::SeqCst), 5);
        pool.close();
    }

    #[test]
    fn a_lower_priority_value_runs_first_then_the_earlier_id() {
        // One worker, held up by the first job while the others queue, so the
        // queue order alone decides who runs next.
        let (release_tx, release_rx) = mpsc::channel::<()>();
        let pool = ThreadPool::new(1, "test_worker", || ());
        let order = Arc::new(Mutex::new(Vec::new()));
        pool.submit(Job::new(0, 0, move |_: &mut ()| {
            let _ = release_rx.recv();
        }))
        .unwrap();
        while pool.pending() > 0 {
            std::thread::yield_now();
        }
        let (done_tx, done_rx) = mpsc::channel();
        for (id, priority) in [(1, 100), (2, 50), (3, 10), (4, 75), (5, 50)] {
            let order = order.clone();
            let done_tx = done_tx.clone();
            pool.submit(Job::new(id, priority, move |_: &mut ()| {
                order.lock().unwrap().push(id);
                let _ = done_tx.send(());
            }))
            .unwrap();
        }
        release_tx.send(()).unwrap();
        for _ in 0..5 {
            done_rx.recv_timeout(Duration::from_secs(5)).unwrap();
        }
        assert_eq!(*order.lock().unwrap(), vec![3, 2, 5, 4, 1]);
        pool.close();
    }

    #[test]
    fn closing_finishes_the_queue_and_refuses_new_jobs() {
        let pool = ThreadPool::new(1, "test_worker", || ());
        let (done_tx, done_rx) = mpsc::channel();
        for i in 0..3 {
            let done_tx = done_tx.clone();
            pool.submit(Job::new(i, 50, move |_: &mut ()| {
                std::thread::sleep(Duration::from_millis(20));
                let _ = done_tx.send(i);
            }))
            .unwrap();
        }
        pool.close();
        let mut done = vec![];
        for _ in 0..3 {
            done.push(done_rx.recv_timeout(Duration::from_secs(5)).unwrap());
        }
        done.sort();
        assert_eq!(done, vec![0, 1, 2]);
        assert!(pool.submit(Job::new(9, 0, |_: &mut ()| {})).is_err());
    }

    #[test]
    fn a_panicking_job_does_not_take_the_worker() {
        let pool = ThreadPool::new(1, "test_worker", || ());
        pool.submit(Job::new(0, 0, |_: &mut ()| panic!("boom")))
            .unwrap();
        let (done_tx, done_rx) = mpsc::channel();
        pool.submit(Job::new(1, 0, move |_: &mut ()| {
            let _ = done_tx.send(());
        }))
        .unwrap();
        done_rx.recv_timeout(Duration::from_secs(5)).unwrap();
        pool.close();
    }
}
