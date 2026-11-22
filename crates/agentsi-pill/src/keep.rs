//! When a loaded voice model drops: after the keep time with no use.
//!
//! Each worker thread keeps one [`Keeper`]. It marks each use, and it waits for its next
//! job no longer than [`Keeper::wait`]. So an idle pill wakes once for each drop, and a pill
//! with no model loaded, or with the keep time "Always", waits with no limit.

pub use agentsi_proto::voice_api::KeepTime;
use std::sync::Arc;
use std::time::{Duration, Instant};

/// The time source of the workers. A test sets a clock that it moves by hand.
pub type Clock = Arc<dyn Fn() -> Instant + Send + Sync>;

pub fn system_clock() -> Clock {
    Arc::new(Instant::now)
}

/// The keep time and the clock that a worker starts with.
#[derive(Clone)]
pub struct Policy {
    pub keep: KeepTime,
    pub clock: Clock,
}

impl Policy {
    pub fn new(keep: KeepTime) -> Policy {
        Policy {
            keep,
            clock: system_clock(),
        }
    }
}

/// The keep time of one model, and its last use while it is loaded.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Keeper {
    keep: KeepTime,
    last_use: Option<Instant>,
}

impl Keeper {
    pub fn new(keep: KeepTime) -> Keeper {
        Keeper {
            keep,
            last_use: None,
        }
    }

    pub fn keep(&self) -> KeepTime {
        self.keep
    }

    /// A new keep time applies to the last use, so a shorter one can make the drop due at once.
    pub fn set(&mut self, keep: KeepTime) {
        self.keep = keep;
    }

    /// The model loaded, or did work, at `now`.
    pub fn used(&mut self, now: Instant) {
        self.last_use = Some(now);
    }

    pub fn dropped(&mut self) {
        self.last_use = None;
    }

    /// How long the worker may wait for its next job. `None` is no limit: no model is loaded,
    /// or the keep time keeps it.
    pub fn wait(&self, now: Instant) -> Option<Duration> {
        let limit = self.keep.limit()?;
        let last = self.last_use?;
        Some((last + limit).saturating_duration_since(now))
    }

    /// The keep time passed with no use since the last one.
    pub fn due(&self, now: Instant) -> bool {
        self.wait(now) == Some(Duration::ZERO)
    }
}

/// The log line of a drop.
pub fn dropped_line(what: &str, keep: KeepTime) -> String {
    format!(
        "agentsi-pill: {what} dropped after {} with no use",
        keep.key()
    )
}

/// A clock that a test moves by hand. It counts its reads, so a test sees each wake.
#[cfg(test)]
pub(crate) struct FakeClock {
    base: Instant,
    offset: std::sync::Mutex<Duration>,
    reads: std::sync::atomic::AtomicUsize,
}

#[cfg(test)]
impl FakeClock {
    pub(crate) fn new() -> Arc<FakeClock> {
        Arc::new(FakeClock {
            base: Instant::now(),
            offset: std::sync::Mutex::new(Duration::ZERO),
            reads: std::sync::atomic::AtomicUsize::new(0),
        })
    }

    pub(crate) fn policy(self: &Arc<Self>, keep: KeepTime) -> Policy {
        let me = self.clone();
        Policy {
            keep,
            clock: Arc::new(move || {
                me.reads.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                me.base + *me.offset.lock().unwrap()
            }),
        }
    }

    pub(crate) fn advance(&self, by: Duration) {
        *self.offset.lock().unwrap() += by;
    }

    pub(crate) fn reads(&self) -> usize {
        self.reads.load(std::sync::atomic::Ordering::SeqCst)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const MINUTE: Duration = Duration::from_secs(60);

    #[test]
    fn with_no_model_loaded_the_wait_has_no_limit() {
        let keeper = Keeper::new(KeepTime::OneMinute);
        assert_eq!(keeper.wait(Instant::now()), None);
        assert!(!keeper.due(Instant::now()));
    }

    #[test]
    fn a_model_drops_one_keep_time_after_its_last_use() {
        let t0 = Instant::now();
        let mut keeper = Keeper::new(KeepTime::OneMinute);
        keeper.used(t0);
        assert_eq!(keeper.wait(t0), Some(MINUTE));
        assert_eq!(keeper.wait(t0 + MINUTE / 2), Some(MINUTE / 2));
        assert!(!keeper.due(t0 + MINUTE / 2));
        // A use moves the drop time.
        keeper.used(t0 + MINUTE / 2);
        assert!(!keeper.due(t0 + MINUTE));
        assert!(keeper.due(t0 + MINUTE + MINUTE / 2));
        assert!(keeper.due(t0 + 10 * MINUTE));
        keeper.dropped();
        assert_eq!(keeper.wait(t0 + 10 * MINUTE), None);
    }

    #[test]
    fn always_and_load_at_start_never_drop() {
        let t0 = Instant::now();
        for keep in [KeepTime::Always, KeepTime::Start] {
            let mut keeper = Keeper::new(keep);
            keeper.used(t0);
            assert_eq!(keeper.wait(t0 + 1000 * MINUTE), None);
            assert!(!keeper.due(t0 + 1000 * MINUTE));
        }
    }

    #[test]
    fn a_shorter_keep_time_applies_to_the_last_use() {
        let t0 = Instant::now();
        let mut keeper = Keeper::new(KeepTime::OneHour);
        keeper.used(t0);
        assert!(!keeper.due(t0 + 2 * MINUTE));
        keeper.set(KeepTime::OneMinute);
        assert!(keeper.due(t0 + 2 * MINUTE));
    }
}
