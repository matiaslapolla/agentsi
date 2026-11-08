//! The meetings and questions that run in this process, by name. A core that stops
//! cancels them here and waits until `idle` says that none is left.

use super::Cancel;
use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{LazyLock, Mutex, MutexGuard};

/// The cancel handle and the product of each running work, by name.
type Running = HashMap<String, (Cancel, Option<String>)>;

static RUNNING: LazyLock<Mutex<Running>> = LazyLock::new(Mutex::default);
static STOPPING: AtomicBool = AtomicBool::new(false);

fn running() -> MutexGuard<'static, Running> {
    RUNNING.lock().unwrap_or_else(|p| p.into_inner())
}

/// Why a meeting cannot start.
#[derive(Debug, PartialEq, Eq)]
pub enum Refused {
    NameTaken,
    CoreStops,
}

/// One running meeting. Keep it for as long as the work runs. Drop removes it.
pub struct Entry {
    pub name: String,
    pub cancel: Cancel,
}

impl Entry {
    pub fn new(name: String) -> Result<Entry, Refused> {
        Entry::claim(name, None)
    }

    /// An entry that names the product it works for, so a clear of that product sees it.
    pub fn for_product(name: String, product: &str) -> Result<Entry, Refused> {
        Entry::claim(name, Some(product.to_string()))
    }

    fn claim(name: String, product: Option<String>) -> Result<Entry, Refused> {
        let mut all = running();
        // Under the lock: `stop_all` must see each entry that this check lets through.
        if STOPPING.load(Ordering::SeqCst) {
            return Err(Refused::CoreStops);
        }
        if all.contains_key(&name) {
            return Err(Refused::NameTaken);
        }
        let cancel = Cancel::default();
        all.insert(name.clone(), (cancel.clone(), product));
        Ok(Entry { name, cancel })
    }
}

impl Drop for Entry {
    fn drop(&mut self) {
        running().remove(&self.name);
    }
}

/// Cancels the meeting `name`. Returns false when none of that name runs.
pub fn cancel(name: &str) -> bool {
    let found = running().get(name).map(|(c, _)| c.clone());
    if let Some(c) = &found {
        c.cancel();
    }
    found.is_some()
}

/// Cancels each meeting, and refuses new ones from now on. Only a core that stops calls it.
pub fn stop_all() {
    let all = running();
    STOPPING.store(true, Ordering::SeqCst);
    all.values().for_each(|(c, _)| c.cancel());
}

/// The name and the product of each running work. The product is `None` for work
/// that no product owns.
pub fn active() -> Vec<(String, Option<String>)> {
    let mut all: Vec<_> = running()
        .iter()
        .map(|(name, (_, product))| (name.clone(), product.clone()))
        .collect();
    all.sort();
    all
}

/// True when no meeting runs.
pub fn idle() -> bool {
    running().is_empty()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_name_runs_once_and_its_entry_frees_it() {
        let first = Entry::new("running-a".into()).unwrap();
        assert_eq!(
            Entry::new("running-a".into()).err(),
            Some(Refused::NameTaken)
        );
        assert!(cancel("running-a"));
        assert!(first.cancel.is_set());
        drop(first);
        assert!(!cancel("running-a"));
    }

    #[test]
    fn running_work_names_its_product() {
        let mine = Entry::for_product("running-b".into(), "prod-b").unwrap();
        let free = Entry::new("running-c".into()).unwrap();
        let now = active();
        assert!(now.contains(&("running-b".into(), Some("prod-b".into()))));
        assert!(now.contains(&("running-c".into(), None)));
        drop((mine, free));
        assert!(!active().iter().any(|(n, _)| n == "running-b"));
    }
}
