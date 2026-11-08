//! The blocking work of the requests. A core that stops waits until it has ended.

use std::sync::atomic::{AtomicUsize, Ordering};
use tokio::task::JoinHandle;

static RUNNING: AtomicUsize = AtomicUsize::new(0);

struct Running;

impl Drop for Running {
    fn drop(&mut self) {
        RUNNING.fetch_sub(1, Ordering::SeqCst);
    }
}

/// Like `tokio::task::spawn_blocking`. The count falls when the closure returns, also when
/// the request that waits for it is gone.
pub fn spawn<F, T>(f: F) -> JoinHandle<T>
where
    F: FnOnce() -> T + Send + 'static,
    T: Send + 'static,
{
    RUNNING.fetch_add(1, Ordering::SeqCst);
    let running = Running;
    tokio::task::spawn_blocking(move || {
        let _running = running;
        f()
    })
}

/// The closures that run now or wait for a thread.
pub fn running() -> usize {
    RUNNING.load(Ordering::SeqCst)
}
