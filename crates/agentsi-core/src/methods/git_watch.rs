//! The git watcher: it sends `git.changed {product}` when the git summary of a product
//! changes. It watches the folder of each product of the registry and its git folders, and
//! reads a summary after a change of the tree, of `HEAD`, of the index or of a ref, at most
//! once every 2 s for each product. It sends the event only when the summary differs, so an
//! idle repo wakes no client. With no subscriber of the `git` topic it reads nothing.
//! Spec: `docs/specs/agentsi-desktop-git-more.md`.

use crate::state::Core;
use agentsi_agency::gitpanel;
use agentsi_agency::registry::Registry;
use agentsi_proto::core_api::Topic;
use agentsi_proto::events::{GitChanged, GitChangedParams};
use agentsi_proto::git_api::GitSummaryInfo;
use notify::{Event, EventKind, RecommendedWatcher, RecursiveMode, Watcher};
use std::path::{Path, PathBuf};
use std::sync::mpsc::{self, RecvTimeoutError};
use std::sync::{Arc, Weak};
use std::time::{Duration, Instant};

/// The shortest time between two summary reads of one product.
pub const GAP: Duration = Duration::from_secs(2);
/// The registry can gain or lose a product while the core runs.
const REGISTRY_EVERY: Duration = Duration::from_secs(30);
const TICK: Duration = Duration::from_secs(1);

struct Product {
    id: String,
    root: PathBuf,
    git_dirs: Vec<PathBuf>,
    watched: Vec<PathBuf>,
    last: Option<GitSummaryInfo>,
    dirty: bool,
    read_at: Instant,
}

/// Starts the watcher on its own thread. It ends with the core.
pub fn start(core: &Arc<Core>) {
    let core = Arc::downgrade(core);
    let spawned = std::thread::Builder::new()
        .name("git-watch".into())
        .spawn(move || run(core));
    if let Err(e) = spawned {
        eprintln!("core: no git watcher: {e}");
    }
}

fn run(core: Weak<Core>) {
    let (send, events) = mpsc::channel();
    let watcher = notify::recommended_watcher(move |e: notify::Result<Event>| {
        if let Ok(e) = e {
            let _ = send.send(e);
        }
    });
    let mut watcher = match watcher {
        Ok(w) => w,
        Err(e) => {
            eprintln!("core: no git watcher: {e}");
            return;
        }
    };
    let mut products: Vec<Product> = Vec::new();
    let mut listed: Option<Instant> = None;
    loop {
        if listed.is_none_or(|t| t.elapsed() >= REGISTRY_EVERY) {
            reconcile(&mut watcher, &mut products, wanted());
            listed = Some(Instant::now());
        }
        let Some(listening) = core.upgrade().map(|c| c.has_subscriber(Topic::Git)) else {
            return;
        };
        let wait = if listening {
            next_read(&products).unwrap_or(TICK).min(TICK)
        } else {
            TICK
        };
        match events.recv_timeout(wait) {
            Ok(e) => {
                mark(&mut products, &e);
                while let Ok(e) = events.try_recv() {
                    mark(&mut products, &e);
                }
            }
            Err(RecvTimeoutError::Timeout) => {}
            Err(RecvTimeoutError::Disconnected) => return,
        }
        let Some(core) = core.upgrade() else {
            return;
        };
        if !core.has_subscriber(Topic::Git) {
            continue;
        }
        for p in products
            .iter_mut()
            .filter(|p| p.dirty && p.read_at.elapsed() >= GAP)
        {
            p.dirty = false;
            p.read_at = Instant::now();
            // A read that fails keeps the last summary, so it sends nothing.
            let Ok(now) = gitpanel::summary(&p.root) else {
                continue;
            };
            if now != p.last {
                p.last = now;
                core.publish::<GitChanged>(&GitChangedParams {
                    product: p.id.clone(),
                });
            }
        }
    }
}

/// The time until the first changed product may be read again.
fn next_read(products: &[Product]) -> Option<Duration> {
    products
        .iter()
        .filter(|p| p.dirty)
        .map(|p| GAP.saturating_sub(p.read_at.elapsed()))
        .min()
}

/// The id and the first path of each product of the registry.
fn wanted() -> Vec<(String, PathBuf)> {
    let Ok(root) = agentsi_store::paths::data_root(None) else {
        return Vec::new();
    };
    let Ok(registry) = Registry::load(&root) else {
        return Vec::new();
    };
    registry
        .products
        .into_iter()
        .filter_map(|p| {
            let first = PathBuf::from(p.paths.first()?);
            Some((p.id, first.canonicalize().unwrap_or(first)))
        })
        .collect()
}

fn reconcile(
    watcher: &mut RecommendedWatcher,
    products: &mut Vec<Product>,
    wanted: Vec<(String, PathBuf)>,
) {
    products.retain(|p| {
        let keep = wanted
            .iter()
            .any(|(id, root)| *id == p.id && *root == p.root);
        if !keep {
            for w in &p.watched {
                let _ = watcher.unwatch(w);
            }
        }
        keep
    });
    for (id, root) in wanted {
        if products.iter().any(|p| p.id == id) {
            continue;
        }
        let Ok(git_dirs) = gitpanel::git_dirs(&root) else {
            continue;
        };
        let mut watched = Vec::new();
        let tree = watcher.watch(&root, RecursiveMode::Recursive);
        if let Err(e) = &tree {
            eprintln!(
                "core: the git watcher sees only HEAD, the index and the refs of {}: {e}",
                root.display()
            );
        } else {
            watched.push(root.clone());
        }
        for dir in &git_dirs {
            if (tree.is_err() || !dir.starts_with(&root))
                && watcher.watch(dir, RecursiveMode::Recursive).is_ok()
            {
                watched.push(dir.clone());
            }
        }
        products.push(Product {
            last: gitpanel::summary(&root).ok().flatten(),
            id,
            root,
            git_dirs,
            watched,
            dirty: false,
            read_at: Instant::now(),
        });
    }
}

/// Marks each product whose summary the event can change.
fn mark(products: &mut [Product], e: &Event) {
    // A read, as by `git status`, changes nothing.
    if matches!(e.kind, EventKind::Access(_)) {
        return;
    }
    for p in products.iter_mut() {
        if !p.dirty && e.paths.iter().any(|path| touches(p, path)) {
            p.dirty = true;
        }
    }
}

/// True for a file of the tree, and for `HEAD`, the index and the refs in a git folder.
fn touches(p: &Product, path: &Path) -> bool {
    let in_git: Vec<&Path> = p
        .git_dirs
        .iter()
        .filter_map(|d| path.strip_prefix(d).ok())
        .collect();
    if in_git.is_empty() {
        return path.starts_with(&p.root);
    }
    in_git.iter().any(|rel| git_state(rel))
}

fn git_state(rel: &Path) -> bool {
    let names: Vec<&str> = rel.iter().filter_map(|c| c.to_str()).collect();
    matches!(
        names[..],
        ["HEAD" | "index" | "packed-refs"] | ["refs", ..] | ["worktrees", _, "HEAD" | "index"]
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn product() -> Product {
        Product {
            id: "a".into(),
            root: "/w/a".into(),
            git_dirs: vec!["/w/a/.git".into()],
            watched: Vec::new(),
            last: None,
            dirty: false,
            read_at: Instant::now(),
        }
    }

    #[test]
    fn a_file_of_the_tree_and_the_state_files_of_git_count_and_the_objects_do_not() {
        let p = product();
        for yes in [
            "/w/a/src/main.rs",
            "/w/a/.git/HEAD",
            "/w/a/.git/index",
            "/w/a/.git/packed-refs",
            "/w/a/.git/refs/heads/main",
            "/w/a/.git/refs/remotes/origin/main",
            "/w/a/.git/worktrees/wt/HEAD",
        ] {
            assert!(touches(&p, Path::new(yes)), "{yes}");
        }
        for no in [
            "/w/a/.git/objects/ab/cdef",
            "/w/a/.git/index.lock",
            "/w/a/.git/logs/HEAD",
            "/w/a/.git/FETCH_HEAD",
            "/w/b/x.txt",
        ] {
            assert!(!touches(&p, Path::new(no)), "{no}");
        }
    }

    #[test]
    fn a_read_marks_nothing_and_a_write_marks_its_product_once() {
        let mut ps = [product()];
        let read = Event::new(EventKind::Access(notify::event::AccessKind::Any))
            .add_path("/w/a/x.txt".into());
        mark(&mut ps, &read);
        assert!(!ps[0].dirty);
        assert_eq!(next_read(&ps), None);
        let write = Event::new(EventKind::Modify(notify::event::ModifyKind::Any))
            .add_path("/w/a/x.txt".into());
        mark(&mut ps, &write);
        assert!(ps[0].dirty);
        // The product was read just now, so the next read waits for most of the gap.
        assert!(next_read(&ps).unwrap() > GAP / 2);
    }
}
