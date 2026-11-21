//! The git watcher: it sends `git.changed {product}` when the git summary of a product
//! changes. It watches each folder of a product that git does not ignore, one by one, and
//! `HEAD`, the index and the refs of its git folders, never `.git/objects`. It reads a summary
//! after a change of the tree, of `HEAD`, of the index or of a ref, at most once every 2 s for
//! each product. It sends the event only when the summary differs, so an idle repo wakes no
//! client. A product whose watches cannot all be added (the inotify limit) is read every
//! 10 s instead. With no subscriber of the `git` topic it reads nothing.
//! Spec: `docs/specs/agentsi-desktop-git-more.md`.

use crate::state::Core;
use agentsi_agency::files::{ignored, walker};
use agentsi_agency::gitpanel;
use agentsi_agency::registry::Registry;
use agentsi_proto::core_api::Topic;
use agentsi_proto::events::{GitChanged, GitChangedParams};
use agentsi_proto::git_api::GitSummaryInfo;
use notify::{Event, EventKind, RecommendedWatcher, RecursiveMode, Watcher};
use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::sync::mpsc::{self, RecvTimeoutError};
use std::sync::{Arc, Weak};
use std::time::{Duration, Instant};

/// The shortest time between two summary reads of one product.
pub const GAP: Duration = Duration::from_secs(2);
/// The time between two summary reads of a product the watcher cannot fully watch.
pub const POLL: Duration = Duration::from_secs(10);
/// The registry can gain or lose a product while the core runs.
const REGISTRY_EVERY: Duration = Duration::from_secs(30);
const TICK: Duration = Duration::from_secs(1);

struct Product {
    id: String,
    root: PathBuf,
    git_dirs: Vec<PathBuf>,
    watched: HashSet<PathBuf>,
    /// The folders of the tree that git ignores, next to a watched folder.
    ignored: HashSet<PathBuf>,
    poll: bool,
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
    // With no watcher the channel stays open, and each product is polled.
    let _open = send.clone();
    let watcher = notify::recommended_watcher(move |e: notify::Result<Event>| {
        if let Ok(e) = e {
            let _ = send.send(e);
        }
    });
    let mut watcher = match watcher {
        Ok(w) => Some(w),
        Err(e) => {
            eprintln!("core: the git watcher reads each summary every 10 s: {e}");
            None
        }
    };
    let mut products: Vec<Product> = Vec::new();
    let mut listed: Option<Instant> = None;
    loop {
        if listed.is_none_or(|t| t.elapsed() >= REGISTRY_EVERY) {
            reconcile(watcher.as_mut(), &mut products, wanted());
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
        let mut new_dirs = Vec::new();
        match events.recv_timeout(wait) {
            Ok(e) => {
                new_dirs.extend(mark(&mut products, &e));
                while let Ok(e) = events.try_recv() {
                    new_dirs.extend(mark(&mut products, &e));
                }
            }
            Err(RecvTimeoutError::Timeout) => {}
            Err(RecvTimeoutError::Disconnected) => return,
        }
        for (i, dir) in new_dirs {
            let p = &mut products[i];
            if p.watched.contains(&dir) || p.ignored.contains(&dir) {
                continue;
            }
            if matches!(place(p, &dir), Place::Worktrees) {
                add(watcher.as_mut(), p, &dir, RecursiveMode::NonRecursive);
            } else if ignored(&dir) {
                p.ignored.insert(dir);
            } else {
                watch_tree(watcher.as_mut(), p, &dir);
                p.dirty = true;
            }
        }
        let Some(core) = core.upgrade() else {
            return;
        };
        if !core.has_subscriber(Topic::Git) {
            continue;
        }
        for p in products.iter_mut() {
            if p.poll && p.read_at.elapsed() >= POLL {
                p.dirty = true;
            }
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
    mut watcher: Option<&mut RecommendedWatcher>,
    products: &mut Vec<Product>,
    wanted: Vec<(String, PathBuf)>,
) {
    products.retain(|p| {
        let keep = wanted
            .iter()
            .any(|(id, root)| *id == p.id && *root == p.root);
        if !keep && let Some(w) = watcher.as_deref_mut() {
            for path in &p.watched {
                let _ = w.unwatch(path);
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
        let mut p = Product {
            last: gitpanel::summary(&root).ok().flatten(),
            id,
            root: root.clone(),
            git_dirs,
            watched: HashSet::new(),
            ignored: HashSet::new(),
            poll: false,
            dirty: false,
            read_at: Instant::now(),
        };
        watch_git(watcher.as_deref_mut(), &mut p);
        watch_tree(watcher.as_deref_mut(), &mut p, &root);
        products.push(p);
    }
}

/// Adds one watch. On a failure the product is polled, and the watches it has stay.
fn add(
    watcher: Option<&mut RecommendedWatcher>,
    p: &mut Product,
    path: &Path,
    mode: RecursiveMode,
) -> bool {
    if p.poll {
        return false;
    }
    if p.watched.contains(path) {
        return true;
    }
    let res = match watcher {
        Some(w) => w.watch(path, mode),
        None => Err(notify::Error::generic("no watcher")),
    };
    match res {
        Ok(()) => {
            p.watched.insert(path.to_path_buf());
            true
        }
        Err(e) => {
            eprintln!(
                "core: the git watcher reads the summary of {} every 10 s: {e}",
                p.root.display()
            );
            p.poll = true;
            false
        }
    }
}

/// Watches `HEAD`, the index, `packed-refs`, the refs and the worktree files of each git folder.
fn watch_git(mut watcher: Option<&mut RecommendedWatcher>, p: &mut Product) {
    for dir in p.git_dirs.clone() {
        add(watcher.as_deref_mut(), p, &dir, RecursiveMode::NonRecursive);
        let refs = dir.join("refs");
        if refs.is_dir() {
            add(watcher.as_deref_mut(), p, &refs, RecursiveMode::Recursive);
        }
        let worktrees = dir.join("worktrees");
        if worktrees.is_dir() {
            add(
                watcher.as_deref_mut(),
                p,
                &worktrees,
                RecursiveMode::NonRecursive,
            );
            for wt in subdirs(&worktrees) {
                add(watcher.as_deref_mut(), p, &wt, RecursiveMode::NonRecursive);
            }
        }
    }
}

/// Watches each folder under `start` that git does not ignore, and notes the ignored ones.
fn watch_tree(mut watcher: Option<&mut RecommendedWatcher>, p: &mut Product, start: &Path) {
    let dirs = tree_dirs(start);
    for d in &dirs {
        for sub in subdirs(d) {
            if !dirs.contains(&sub) && sub.file_name().is_some_and(|n| n != ".git") {
                p.ignored.insert(sub);
            }
        }
    }
    for d in &dirs {
        if !add(watcher.as_deref_mut(), p, d, RecursiveMode::NonRecursive) {
            break;
        }
    }
}

/// `start` and each folder under it that git does not ignore, with no `.git` folder.
fn tree_dirs(start: &Path) -> HashSet<PathBuf> {
    walker(start)
        .filter_entry(|e| e.file_type().is_some_and(|t| t.is_dir()) && e.file_name() != ".git")
        .build()
        .filter_map(Result::ok)
        .map(|e| e.into_path())
        .collect()
}

fn subdirs(dir: &Path) -> Vec<PathBuf> {
    let Ok(read) = std::fs::read_dir(dir) else {
        return Vec::new();
    };
    read.filter_map(Result::ok)
        .filter(|e| e.file_type().is_ok_and(|t| t.is_dir()))
        .map(|e| e.path())
        .collect()
}

/// Marks each product whose summary the event can change, and returns the new folders, with
/// the index of their product, that may need a watch.
fn mark(products: &mut [Product], e: &Event) -> Vec<(usize, PathBuf)> {
    let mut new = Vec::new();
    // A read, as by `git status`, changes nothing.
    if matches!(e.kind, EventKind::Access(_)) {
        return new;
    }
    for (i, p) in products.iter_mut().enumerate() {
        for path in &e.paths {
            match place(p, path) {
                Place::Outside | Place::GitOther => {}
                Place::GitState => p.dirty = true,
                Place::Worktrees => {
                    if path.is_dir() && !p.watched.contains(path) {
                        new.push((i, path.clone()));
                    }
                }
                Place::Tree => {
                    let gone = !path.exists();
                    if path.ancestors().any(|a| p.ignored.contains(a)) {
                        if gone {
                            p.ignored.remove(path);
                        }
                        continue;
                    }
                    if gone {
                        p.watched.remove(path);
                    } else if path.is_dir() && !p.watched.contains(path) {
                        new.push((i, path.clone()));
                        continue;
                    }
                    p.dirty = true;
                }
            }
        }
    }
    new
}

#[derive(Debug, PartialEq)]
enum Place {
    Outside,
    /// A file or folder of the tree.
    Tree,
    /// `HEAD`, the index or a ref in a git folder.
    GitState,
    /// The worktrees folder of a git folder, or a folder in it.
    Worktrees,
    GitOther,
}

fn place(p: &Product, path: &Path) -> Place {
    let in_git: Vec<&Path> = p
        .git_dirs
        .iter()
        .filter_map(|d| path.strip_prefix(d).ok())
        .collect();
    if in_git.is_empty() {
        return if path.starts_with(&p.root) {
            Place::Tree
        } else {
            Place::Outside
        };
    }
    if in_git.iter().any(|rel| git_state(rel)) {
        return Place::GitState;
    }
    let worktrees = in_git.iter().any(|rel| {
        let names: Vec<&str> = rel.iter().filter_map(|c| c.to_str()).collect();
        matches!(names[..], ["worktrees"] | ["worktrees", _])
    });
    if worktrees {
        Place::Worktrees
    } else {
        Place::GitOther
    }
}

/// True for a file of the tree, and for `HEAD`, the index and the refs in a git folder.
#[cfg(test)]
fn touches(p: &Product, path: &Path) -> bool {
    matches!(place(p, path), Place::Tree | Place::GitState)
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
            watched: HashSet::new(),
            ignored: HashSet::new(),
            poll: false,
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

    fn repo() -> (tempfile::TempDir, PathBuf) {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().canonicalize().unwrap();
        let init = std::process::Command::new("git")
            .args(["init", "-q"])
            .current_dir(&root)
            .env("GIT_CONFIG_GLOBAL", "/dev/null")
            .env("GIT_CONFIG_SYSTEM", "/dev/null")
            .status()
            .unwrap();
        assert!(init.success());
        std::fs::write(root.join(".gitignore"), "target/\n/build\n").unwrap();
        std::fs::write(root.join(".git/info/exclude"), "local/\n").unwrap();
        for d in [
            "src/a",
            "target/debug/deps",
            "build/out",
            "local",
            ".github",
        ] {
            std::fs::create_dir_all(root.join(d)).unwrap();
        }
        std::fs::write(root.join("target/debug/deps/x.o"), "x").unwrap();
        (dir, root)
    }

    #[test]
    fn the_walk_finds_the_folders_git_keeps_and_not_the_ignored_ones_or_git() {
        let (_dir, root) = repo();
        let dirs = tree_dirs(&root);
        for yes in ["", "src", "src/a", ".github"] {
            assert!(dirs.contains(&root.join(yes)), "{yes}");
        }
        for no in [
            "target",
            "target/debug",
            "build",
            "local",
            ".git",
            ".git/objects",
        ] {
            assert!(!dirs.contains(&root.join(no)), "{no}");
        }
        assert!(ignored(&root.join("target")));
        assert!(ignored(&root.join("local")));
        assert!(!ignored(&root.join("src/a")));
    }

    #[test]
    fn the_watcher_watches_no_ignored_folder_and_a_write_in_one_marks_nothing() {
        let (_dir, root) = repo();
        let mut w = notify::recommended_watcher(|_: notify::Result<Event>| {}).unwrap();
        let mut ps = Vec::new();
        reconcile(Some(&mut w), &mut ps, vec![("a".into(), root.clone())]);
        let p = &ps[0];
        assert!(!p.poll);
        assert!(p.watched.contains(&root.join("src/a")));
        assert!(p.watched.contains(&root.join(".git")));
        assert!(p.watched.contains(&root.join(".git/refs")));
        assert!(
            p.watched
                .iter()
                .all(|w| !w.starts_with(root.join("target")))
        );
        assert!(
            p.watched
                .iter()
                .all(|w| !w.starts_with(root.join(".git/objects")))
        );
        assert!(p.ignored.contains(&root.join("target")));

        let write = |path: PathBuf| {
            Event::new(EventKind::Modify(notify::event::ModifyKind::Any)).add_path(path)
        };
        assert!(mark(&mut ps, &write(root.join("target/debug/deps/x.o"))).is_empty());
        assert!(mark(&mut ps, &write(root.join("target"))).is_empty());
        assert!(!ps[0].dirty);

        // A new folder that git keeps needs a watch, and one it ignores is noted.
        std::fs::create_dir_all(root.join("src/b/c")).unwrap();
        let new = mark(&mut ps, &write(root.join("src/b")));
        assert_eq!(new, vec![(0, root.join("src/b"))]);
        assert!(!ignored(&root.join("src/b")));
        watch_tree(Some(&mut w), &mut ps[0], &root.join("src/b"));
        assert!(ps[0].watched.contains(&root.join("src/b/c")));
        mark(&mut ps, &write(root.join("src/b/c/x.rs")));
        assert!(ps[0].dirty);
    }

    #[test]
    fn a_watch_that_cannot_be_added_polls_the_product_and_keeps_its_watches() {
        let (_dir, root) = repo();
        let mut ps = Vec::new();
        reconcile(None, &mut ps, vec![("a".into(), root.clone())]);
        assert!(ps[0].poll);
        assert!(ps[0].watched.is_empty());

        let mut w = notify::recommended_watcher(|_: notify::Result<Event>| {}).unwrap();
        let mut p = product();
        p.root = root.clone();
        assert!(add(
            Some(&mut w),
            &mut p,
            &root,
            RecursiveMode::NonRecursive
        ));
        assert!(!add(
            Some(&mut w),
            &mut p,
            &root.join("gone"),
            RecursiveMode::NonRecursive
        ));
        assert!(p.poll);
        assert!(p.watched.contains(&root));
    }
}
