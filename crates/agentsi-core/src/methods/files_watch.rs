//! The files watcher: it sends `files.changed {product, path}` when an entry of a folder that
//! `files.list` read changes. It watches each such folder alone, never one that git ignores,
//! and at most 512 of them: a new one drops the one read longest ago. A name that comes, goes
//! or moves sends within 150 ms. A write into a file sends at most once a second for its
//! folder, so a build that writes all the time costs one list a second. With no subscriber of
//! the `files` topic, it watches nothing. Spec: `docs/specs/agentsi-desktop-files.md`.

use crate::state::Core;
use agentsi_proto::core_api::Topic;
use agentsi_proto::events::{FilesChanged, FilesChangedParams};
use notify::event::ModifyKind;
use notify::{Event, EventKind, RecommendedWatcher, RecursiveMode, Watcher};
use std::collections::{BTreeSet, HashMap, VecDeque};
use std::path::PathBuf;
use std::sync::mpsc::{self, RecvTimeoutError, Sender};
use std::sync::{Arc, Mutex, Weak};
use std::time::{Duration, Instant};

/// The most folders that the watcher holds.
pub const MAX_FOLDERS: usize = 512;
/// The shortest time between two events of one folder after a name changed.
const GAP_NAMES: Duration = Duration::from_millis(150);
/// The shortest time between two events of one folder after only a write.
const GAP_WRITES: Duration = Duration::from_secs(1);
/// A burst of changes ends in one event.
const SETTLE: Duration = Duration::from_millis(40);
const TICK: Duration = Duration::from_secs(1);

/// One folder to watch: the product, its relative path and its resolved path.
pub struct Folder {
    pub product: String,
    pub path: String,
    pub dir: PathBuf,
}

enum Msg {
    Watch(Folder),
    Fs(Event),
}

/// The handle of the watcher thread. The thread starts at the first folder.
#[derive(Default)]
pub struct FileWatch {
    send: Mutex<Option<Sender<Msg>>>,
}

impl FileWatch {
    /// Watches `folder` while a client subscribes to `files`. The caller checks that git does
    /// not ignore it.
    pub fn watch(&self, core: &Arc<Core>, folder: Folder) {
        let mut send = self.send.lock().unwrap_or_else(|e| e.into_inner());
        if send.is_none() {
            let (tx, rx) = mpsc::channel();
            let weak = Arc::downgrade(core);
            let fs = tx.clone();
            let spawned = std::thread::Builder::new()
                .name("files-watch".into())
                .spawn(move || run(weak, fs, rx));
            if let Err(e) = spawned {
                eprintln!("core: no files watcher: {e}");
                return;
            }
            *send = Some(tx);
        }
        if let Some(tx) = send.as_ref() {
            let _ = tx.send(Msg::Watch(folder));
        }
    }
}

struct Watched {
    /// Each product and path that read this folder: two products can share a folder.
    owners: BTreeSet<(String, String)>,
    sent: Option<Instant>,
    due: Option<Instant>,
}

#[derive(Default)]
struct Folders {
    by_dir: HashMap<PathBuf, Watched>,
    /// The folders, the one read longest ago first.
    order: VecDeque<PathBuf>,
}

impl Folders {
    fn touch(&mut self, dir: &PathBuf) {
        self.order.retain(|d| d != dir);
        self.order.push_back(dir.clone());
    }

    fn drop_dir(&mut self, w: Option<&mut RecommendedWatcher>, dir: &PathBuf) {
        if self.by_dir.remove(dir).is_some() {
            self.order.retain(|d| d != dir);
            if let Some(w) = w {
                let _ = w.unwatch(dir);
            }
        }
    }

    fn clear(&mut self, mut w: Option<&mut RecommendedWatcher>) {
        for dir in std::mem::take(&mut self.order) {
            if let Some(w) = w.as_deref_mut() {
                let _ = w.unwatch(&dir);
            }
        }
        self.by_dir.clear();
    }

    /// Marks the folder `dir` changed. `names` is true when a name came, went or moved.
    fn mark(&mut self, dir: &PathBuf, names: bool, now: Instant) {
        let Some(w) = self.by_dir.get_mut(dir) else {
            return;
        };
        let gap = if names { GAP_NAMES } else { GAP_WRITES };
        let at = w.sent.map_or(now + SETTLE, |s| (s + gap).max(now + SETTLE));
        w.due = Some(w.due.map_or(at, |d| d.min(at)));
    }

    fn next_due(&self) -> Option<Instant> {
        self.by_dir.values().filter_map(|w| w.due).min()
    }
}

fn run(core: Weak<Core>, fs: Sender<Msg>, rx: mpsc::Receiver<Msg>) {
    let watcher = notify::recommended_watcher(move |e: notify::Result<Event>| {
        if let Ok(e) = e {
            let _ = fs.send(Msg::Fs(e));
        }
    });
    let mut watcher = match watcher {
        Ok(w) => Some(w),
        Err(e) => {
            eprintln!("core: the files watcher sends no files.changed: {e}");
            None
        }
    };
    let mut folders = Folders::default();
    loop {
        // With no folder there is nothing to time: the thread sleeps until the next list.
        let msg = if folders.by_dir.is_empty() {
            match rx.recv() {
                Ok(m) => Some(m),
                Err(_) => return,
            }
        } else {
            let wait = folders
                .next_due()
                .map_or(TICK, |d| d.saturating_duration_since(Instant::now()))
                .min(TICK);
            match rx.recv_timeout(wait) {
                Ok(m) => Some(m),
                Err(RecvTimeoutError::Timeout) => None,
                Err(RecvTimeoutError::Disconnected) => return,
            }
        };
        let Some(core) = core.upgrade() else {
            return;
        };
        if !core.has_subscriber(Topic::Files) {
            folders.clear(watcher.as_mut());
            continue;
        }
        match msg {
            Some(Msg::Watch(f)) => add(watcher.as_mut(), &mut folders, f),
            Some(Msg::Fs(e)) => take(watcher.as_mut(), &mut folders, &e),
            None => {}
        }
        while let Ok(m) = rx.try_recv() {
            match m {
                Msg::Watch(f) => add(watcher.as_mut(), &mut folders, f),
                Msg::Fs(e) => take(watcher.as_mut(), &mut folders, &e),
            }
        }
        let now = Instant::now();
        for w in folders.by_dir.values_mut() {
            if w.due.is_some_and(|d| d <= now) {
                w.due = None;
                w.sent = Some(now);
                for (product, path) in &w.owners {
                    core.publish::<FilesChanged>(&FilesChangedParams {
                        product: product.clone(),
                        path: path.clone(),
                    });
                }
            }
        }
    }
}

fn add(mut watcher: Option<&mut RecommendedWatcher>, folders: &mut Folders, f: Folder) {
    if let Some(w) = folders.by_dir.get_mut(&f.dir) {
        w.owners.insert((f.product, f.path));
        folders.touch(&f.dir);
        return;
    }
    let Some(w) = watcher.as_deref_mut() else {
        return;
    };
    if let Err(e) = w.watch(&f.dir, RecursiveMode::NonRecursive) {
        eprintln!(
            "core: the files watcher cannot watch {}: {e}",
            f.dir.display()
        );
        return;
    }
    if folders.order.len() >= MAX_FOLDERS
        && let Some(old) = folders.order.front().cloned()
    {
        folders.drop_dir(watcher, &old);
    }
    folders.touch(&f.dir);
    folders.by_dir.insert(
        f.dir,
        Watched {
            owners: BTreeSet::from([(f.product, f.path)]),
            sent: None,
            due: None,
        },
    );
}

fn take(mut watcher: Option<&mut RecommendedWatcher>, folders: &mut Folders, e: &Event) {
    let names = match e.kind {
        EventKind::Access(_) => return,
        EventKind::Create(_) | EventKind::Remove(_) | EventKind::Modify(ModifyKind::Name(_)) => {
            true
        }
        _ => false,
    };
    let now = Instant::now();
    for path in &e.paths {
        if folders.by_dir.contains_key(path) && !path.is_dir() {
            folders.drop_dir(watcher.as_deref_mut(), path);
        }
        if let Some(parent) = path.parent() {
            folders.mark(&parent.to_path_buf(), names, now);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn folders(dirs: &[&str]) -> Folders {
        let mut f = Folders::default();
        for d in dirs {
            let dir = PathBuf::from(d);
            f.touch(&dir);
            f.by_dir.insert(
                dir,
                Watched {
                    owners: BTreeSet::from([(
                        "a".to_string(),
                        d.trim_start_matches("/p")
                            .trim_start_matches('/')
                            .to_string(),
                    )]),
                    sent: None,
                    due: None,
                },
            );
        }
        f
    }

    #[test]
    fn a_name_change_is_due_soon_and_a_write_after_an_event_waits_a_second() {
        let mut f = folders(&["/p", "/p/src"]);
        let now = Instant::now();
        let ev = |kind, path: &str| Event::new(kind).add_path(path.into());
        take(
            None,
            &mut f,
            &ev(EventKind::Access(notify::event::AccessKind::Any), "/p/a"),
        );
        assert_eq!(f.next_due(), None);
        take(
            None,
            &mut f,
            &ev(
                EventKind::Create(notify::event::CreateKind::File),
                "/p/src/x.rs",
            ),
        );
        let due = f.by_dir[&PathBuf::from("/p/src")].due.unwrap();
        assert!(due <= now + SETTLE + Duration::from_millis(50));
        assert!(f.by_dir[&PathBuf::from("/p")].due.is_none());

        let sent = Instant::now();
        f.by_dir.get_mut(&PathBuf::from("/p/src")).unwrap().due = None;
        f.by_dir.get_mut(&PathBuf::from("/p/src")).unwrap().sent = Some(sent);
        take(
            None,
            &mut f,
            &ev(
                EventKind::Modify(ModifyKind::Data(notify::event::DataChange::Any)),
                "/p/src/x.rs",
            ),
        );
        assert!(f.by_dir[&PathBuf::from("/p/src")].due.unwrap() >= sent + GAP_WRITES);
        // A new name then comes sooner than the write.
        take(
            None,
            &mut f,
            &ev(
                EventKind::Remove(notify::event::RemoveKind::File),
                "/p/src/x.rs",
            ),
        );
        assert!(f.by_dir[&PathBuf::from("/p/src")].due.unwrap() < sent + GAP_WRITES);
    }

    #[test]
    fn a_watched_folder_that_goes_away_is_dropped() {
        let mut f = folders(&["/p", "/nonexistent-agentsi-test/gone"]);
        let ev = Event::new(EventKind::Remove(notify::event::RemoveKind::Folder))
            .add_path("/nonexistent-agentsi-test/gone".into());
        take(None, &mut f, &ev);
        assert!(
            !f.by_dir
                .contains_key(&PathBuf::from("/nonexistent-agentsi-test/gone"))
        );
        assert_eq!(f.order, [PathBuf::from("/p")]);
    }
}
