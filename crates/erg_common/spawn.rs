#[cfg(all(unix, feature = "backtrace"))]
pub use backtrace_on_stack_overflow;
use std::thread::{self, JoinHandle};
#[cfg(all(windows, feature = "backtrace"))]
pub use w_boson;

const STACK_SIZE: usize = if cfg!(feature = "large_thread") {
    16 * 1024 * 1024
} else {
    8 * 1024 * 1024
};

/// Maximum depth of nested compile-time calls.
///
/// This bounds the host stack rather than the number of calls: one const call
/// clones a `Context` and lowers the callee's body, which costs roughly 70 KiB of
/// stack, so a budget expressed in calls has to be derived from [`STACK_SIZE`].
/// Counting calls alone is what let a recursive const function overflow the
/// stack instead of reporting a `RecursionError`.
///
/// The divisor leaves roughly a 2x margin over the measured frame size.
pub const CONST_CALL_LIMIT: usize = STACK_SIZE / (128 * 1024);

/// How many stacks deep compile-time evaluation may go.
///
/// [`CONST_CALL_LIMIT`] bounds *one* stack; when it is reached the evaluation
/// continues on a fresh one (see [`run_on_new_stack`]), so what bounds a
/// runaway recursion is this times that -- about a thousand levels of a const
/// function the user wrote, in the region of CPython's own limit.
pub const CONST_CALL_STACKS: usize = 32;

/// Run `f` on a thread of its own with a full [`STACK_SIZE`], and wait for it.
///
/// Compile-time evaluation recurses on the host stack, so how deep a const
/// function may recurse is otherwise decided by how much stack is left. Handing
/// the rest of the evaluation a new stack turns that into a budget we choose.
/// The thread is scoped, so `f` may borrow -- the evaluator hands it the
/// `Context` it is already holding.
pub fn run_on_new_stack<F, T>(f: F) -> T
where
    F: FnOnce() -> T + Send,
    T: Send,
{
    thread::scope(|scope| {
        thread::Builder::new()
            .stack_size(STACK_SIZE)
            .name("erg_const_eval".to_string())
            .spawn_scoped(scope, f)
            .expect("failed to spawn a thread for compile-time evaluation")
            .join()
            .unwrap_or_else(|e| std::panic::resume_unwind(e))
    })
}

#[macro_export]
macro_rules! enable_overflow_stacktrace {
    () => {
        #[cfg(all(unix, feature = "backtrace"))]
        unsafe {
            $crate::spawn::backtrace_on_stack_overflow::enable()
        };
        #[cfg(all(windows, feature = "backtrace"))]
        unsafe {
            $crate::spawn::w_boson::enable()
        };
    };
}

/// Execute the function in a new thread.
/// The default stack size is 4MB, and with the `large_thread` flag, the stack size is 8MB.
pub fn exec_new_thread<F, T>(run: F, name: &str) -> T
where
    F: FnOnce() -> T + Send + 'static,
    T: Send + 'static,
{
    enable_overflow_stacktrace!();
    let child = thread::Builder::new()
        .name(name.to_string())
        .stack_size(STACK_SIZE)
        .spawn(run)
        .unwrap();
    // Wait for thread to join
    child.join().unwrap_or_else(|err| {
        eprintln!("Thread panicked: {err:?}");
        std::process::exit(1);
    })
}

pub fn spawn_new_thread<F, T>(run: F, name: &str) -> JoinHandle<T>
where
    F: FnOnce() -> T + Send + 'static,
    T: Send + 'static,
{
    enable_overflow_stacktrace!();
    thread::Builder::new()
        .name(name.to_string())
        .stack_size(STACK_SIZE)
        .spawn(run)
        .unwrap()
}

pub fn safe_yield() {
    std::thread::yield_now();
    std::thread::sleep(std::time::Duration::from_millis(10));
}
