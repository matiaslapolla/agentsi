//! The file tree of a product: list one folder, create, rename and trash, inside the folder of
//! the product only. Spec: `docs/specs/agentsi-desktop-files.md`.
//!
//! A path is relative to the folder of the product. A `..` or a `.git` part is refused. The
//! folder that holds the target is resolved with its links, and it must stay inside the folder
//! of the product, so a link that points outside cannot carry a write out. The last part of a
//! path is not followed: a rename or a trash of a link acts on the link.

use crate::registry::Product;
use crate::story::git::git_limited;
use agentsi_proto::files_api::{
    FIND_MAX, FileEntry, FileGit, FileKind, FilesFindResult, FilesListResult, LIST_MAX, NewKind,
};
use ignore::WalkBuilder;
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::time::{Duration, UNIX_EPOCH};

const GIT_LIMIT: Duration = Duration::from_secs(10);
/// The most entries that one `find` reads, so a huge folder cannot hold a core thread.
const FIND_WALK_MAX: usize = 100_000;

#[derive(Debug, PartialEq, Eq)]
pub enum Error {
    /// The path leaves the folder of the product, or is not a path.
    Invalid(String),
    NotFound(String),
    /// The target exists.
    Conflict(String),
    /// The system refused: no permission, no trash, a full disk.
    Io(String),
}

pub type Result<T> = std::result::Result<T, Error>;

impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let (Error::Invalid(m) | Error::NotFound(m) | Error::Conflict(m) | Error::Io(m)) = self;
        f.write_str(m)
    }
}

fn io(path: &str, e: std::io::Error) -> Error {
    match e.kind() {
        std::io::ErrorKind::NotFound => Error::NotFound(format!("{path} does not exist.")),
        std::io::ErrorKind::AlreadyExists => Error::Conflict(format!("{path} exists.")),
        _ => Error::Io(format!("{path}: {e}")),
    }
}

/// The names of a relative path. The empty path has none.
fn parts(rel: &str) -> Result<Vec<&str>> {
    let invalid = || Error::Invalid(format!("{rel:?} is not a path inside the product."));
    if rel.contains('\0') || rel.starts_with('/') {
        return Err(invalid());
    }
    let names: Vec<&str> = rel
        .split('/')
        .filter(|n| !n.is_empty() && *n != ".")
        .collect();
    // A disk that ignores case reads `.GIT` as `.git`.
    if names
        .iter()
        .any(|n| *n == ".." || n.eq_ignore_ascii_case(".git"))
    {
        return Err(invalid());
    }
    Ok(names)
}

/// `rel` with no empty or `.` part and no `/` at the ends: the key of a folder in a tree.
pub fn normal(rel: &str) -> Result<String> {
    Ok(parts(rel)?.join("/"))
}

/// The folder of one product, resolved.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Root {
    dir: PathBuf,
}

impl Root {
    /// The first path of the product.
    pub fn of(product: &Product) -> Result<Root> {
        let first = product
            .paths
            .first()
            .ok_or_else(|| Error::NotFound(format!("Product {} has no folder.", product.id)))?;
        let dir = match (first.strip_prefix("~/"), std::env::var_os("HOME")) {
            (Some(rest), Some(home)) => PathBuf::from(home).join(rest),
            _ => PathBuf::from(first),
        };
        Root::new(&dir)
    }

    pub fn new(dir: &Path) -> Result<Root> {
        let dir = dir
            .canonicalize()
            .map_err(|e| io(&dir.to_string_lossy(), e))?;
        if !dir.is_dir() {
            return Err(Error::NotFound(format!(
                "{} is not a folder.",
                dir.display()
            )));
        }
        Ok(Root { dir })
    }

    pub fn dir(&self) -> &Path {
        &self.dir
    }

    /// `abs` from the folder of the product, or `None` when it is outside.
    pub fn relative(&self, abs: &Path) -> Option<String> {
        let rest = abs.strip_prefix(&self.dir).ok()?;
        let names: Vec<_> = rest.iter().map(|n| n.to_string_lossy()).collect();
        Some(names.join("/"))
    }

    /// A resolved path that must stay inside the folder, and in no `.git` folder.
    fn inside(&self, resolved: PathBuf, rel: &str) -> Result<PathBuf> {
        let in_git = resolved.strip_prefix(&self.dir).map(|r| {
            r.iter()
                .any(|n| n.to_str().is_some_and(|n| n.eq_ignore_ascii_case(".git")))
        });
        match in_git {
            Ok(false) => Ok(resolved),
            _ => Err(Error::Invalid(format!(
                "{rel:?} leaves the folder of the product."
            ))),
        }
    }

    /// The folder `rel`, with its links resolved.
    pub fn folder(&self, rel: &str) -> Result<PathBuf> {
        let names = parts(rel)?;
        let mut path = self.dir.clone();
        path.extend(&names);
        let resolved = path.canonicalize().map_err(|e| io(rel, e))?;
        let resolved = self.inside(resolved, rel)?;
        if !resolved.is_dir() {
            return Err(Error::Invalid(format!("{rel} is not a folder.")));
        }
        Ok(resolved)
    }

    /// The entry `rel`: its folder resolved, and its own name not followed.
    fn entry(&self, rel: &str) -> Result<PathBuf> {
        let names = parts(rel)?;
        let Some((last, folder)) = names.split_last() else {
            return Err(Error::Invalid(
                "The folder of the product itself cannot change.".into(),
            ));
        };
        Ok(self.folder(&folder.join("/"))?.join(last))
    }

    /// The new path `rel`, for a create: the nearest folder that exists is resolved, and the
    /// rest is made. Returns the path and the first folder to make, if any.
    fn fresh(&self, rel: &str) -> Result<(PathBuf, Option<PathBuf>)> {
        let names = parts(rel)?;
        if names.is_empty() {
            return Err(Error::Conflict("The folder of the product exists.".into()));
        }
        let mut have = names.len() - 1;
        while have > 0 {
            let mut probe = self.dir.clone();
            probe.extend(&names[..have]);
            if probe.symlink_metadata().is_ok() {
                break;
            }
            have -= 1;
        }
        let base = self.folder(&names[..have].join("/"))?;
        let mut path = base.clone();
        path.extend(&names[have..]);
        let first = (have + 1 < names.len()).then(|| base.join(names[have]));
        Ok((path, first))
    }

    pub fn list(&self, rel: &str) -> Result<FilesListResult> {
        let dir = self.folder(rel)?;
        let read = std::fs::read_dir(&dir).map_err(|e| io(rel, e))?;
        // A walk from inside an ignored folder does not see the rule of the folder.
        let all_ignored = ignored_within(&self.dir, &dir);
        let kept = if all_ignored {
            HashSet::new()
        } else {
            kept_names(&dir)
        };
        let git = if all_ignored {
            Marks::default()
        } else {
            git_marks(&dir)
        };
        let mut entries: Vec<FileEntry> = read
            .filter_map(std::result::Result::ok)
            .filter(|e| e.file_name() != ".git")
            .filter_map(|e| {
                let name = e.file_name().to_string_lossy().into_owned();
                let meta = e.path().symlink_metadata().ok()?;
                let kind = if meta.file_type().is_symlink() {
                    FileKind::Link
                } else if meta.is_dir() {
                    FileKind::Folder
                } else {
                    FileKind::File
                };
                let state = if !kept.contains(&name) {
                    Some(FileGit::Ignored)
                } else if git.all_new {
                    Some(FileGit::New)
                } else {
                    git.marks.get(&name).copied()
                };
                Some(FileEntry {
                    size: if kind == FileKind::Folder {
                        0
                    } else {
                        meta.len()
                    },
                    modified: meta
                        .modified()
                        .ok()
                        .and_then(|t| t.duration_since(UNIX_EPOCH).ok())
                        .map(|d| d.as_secs() as i64),
                    git: state,
                    kind,
                    name,
                })
            })
            .collect();
        entries.sort_by(|a, b| {
            (a.kind != FileKind::Folder)
                .cmp(&(b.kind != FileKind::Folder))
                .then_with(|| a.name.to_lowercase().cmp(&b.name.to_lowercase()))
                .then_with(|| a.name.cmp(&b.name))
        });
        let truncated = entries.len() > LIST_MAX;
        entries.truncate(LIST_MAX);
        Ok(FilesListResult {
            path: normal(rel)?,
            entries,
            truncated,
        })
    }

    pub fn create(&self, rel: &str, kind: NewKind) -> Result<()> {
        let (path, first) = self.fresh(rel)?;
        if path.symlink_metadata().is_ok() {
            return Err(Error::Conflict(format!("{rel} exists.")));
        }
        if let Some(first) = first {
            // `create_dir` refuses a folder that a link took meanwhile.
            std::fs::create_dir(&first).map_err(|e| io(rel, e))?;
        }
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).map_err(|e| io(rel, e))?;
        }
        match kind {
            NewKind::Folder => std::fs::create_dir(&path),
            NewKind::File => std::fs::OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(&path)
                .map(drop),
        }
        .map_err(|e| io(rel, e))
    }

    pub fn rename(&self, from: &str, to: &str) -> Result<()> {
        let src = self.entry(from)?;
        src.symlink_metadata().map_err(|e| io(from, e))?;
        let dst = self.entry(to)?;
        if dst.symlink_metadata().is_ok() {
            return Err(Error::Conflict(format!("{to} exists.")));
        }
        rename_no_replace(&src, &dst).map_err(|e| io(to, e))
    }

    pub fn trash(&self, rel: &str) -> Result<()> {
        let path = self.entry(rel)?;
        path.symlink_metadata().map_err(|e| io(rel, e))?;
        trash::delete(&path).map_err(|e| Error::Io(format!("{rel} did not go to the trash: {e}")))
    }
}

/// A rename that fails with `AlreadyExists` when `dst` exists, also when it came after the check.
impl Root {
    /// The files whose name holds `query`, with no regard to case, best first. A query with a
    /// `/` matches the path. The walk skips what git ignores and `.git`, and follows no link.
    pub fn find(&self, query: &str, limit: usize) -> FilesFindResult {
        let query = query.trim().to_lowercase();
        let limit = limit.clamp(1, FIND_MAX);
        let mut found: Vec<(u8, String)> = Vec::new();
        let mut truncated = false;
        if query.is_empty() {
            return FilesFindResult {
                paths: Vec::new(),
                truncated,
            };
        }
        let by_path = query.contains('/');
        let tail = format!("/{query}");
        let walk = walker(&self.dir)
            .filter_entry(|e| e.depth() == 0 || !e.file_name().eq_ignore_ascii_case(".git"))
            .build()
            .filter_map(std::result::Result::ok)
            .filter(|e| e.depth() > 0);
        for (n, e) in walk.enumerate() {
            if n == FIND_WALK_MAX {
                truncated = true;
                break;
            }
            if !e.file_type().is_some_and(|t| t.is_file()) {
                continue;
            }
            let Some(rel) = self.relative(e.path()) else {
                continue;
            };
            let name = e.file_name().to_string_lossy().to_lowercase();
            let lower = rel.to_lowercase();
            let hay = if by_path { &lower } else { &name };
            let rank = if name == query || (by_path && (lower == query || lower.ends_with(&tail))) {
                0
            } else if hay.starts_with(&query) {
                1
            } else if hay.contains(&query) {
                2
            } else {
                continue;
            };
            found.push((rank, rel));
        }
        found.sort_by(|a, b| (a.0, a.1.len(), &a.1).cmp(&(b.0, b.1.len(), &b.1)));
        truncated |= found.len() > limit;
        found.truncate(limit);
        FilesFindResult {
            paths: found.into_iter().map(|(_, p)| p).collect(),
            truncated,
        }
    }
}

fn rename_no_replace(src: &Path, dst: &Path) -> std::io::Result<()> {
    use std::ffi::CString;
    use std::os::unix::ffi::OsStrExt;
    let c = |p: &Path| CString::new(p.as_os_str().as_bytes()).map_err(std::io::Error::other);
    let (src, dst) = (c(src)?, c(dst)?);
    // SAFETY: both are valid NUL-terminated paths, and AT_FDCWD needs no open descriptor.
    #[cfg(target_os = "linux")]
    let r = unsafe {
        libc::renameat2(
            libc::AT_FDCWD,
            src.as_ptr(),
            libc::AT_FDCWD,
            dst.as_ptr(),
            libc::RENAME_NOREPLACE,
        )
    };
    // SAFETY: both are valid NUL-terminated paths.
    #[cfg(target_os = "macos")]
    let r = unsafe { libc::renamex_np(src.as_ptr(), dst.as_ptr(), libc::RENAME_EXCL) };
    if r == 0 {
        Ok(())
    } else {
        Err(std::io::Error::last_os_error())
    }
}

/// A walk that keeps what git keeps: it reads `.gitignore`, `.git/info/exclude` and the global
/// excludes file, shows hidden files and follows no link.
pub fn walker(start: &Path) -> WalkBuilder {
    let mut w = WalkBuilder::new(start);
    w.hidden(false)
        .ignore(false)
        .parents(true)
        .git_ignore(true)
        .git_exclude(true)
        .git_global(true)
        .follow_links(false);
    w
}

/// The names in `dir` that git does not ignore.
fn kept_names(dir: &Path) -> HashSet<String> {
    walker(dir)
        .max_depth(Some(1))
        .build()
        .filter_map(std::result::Result::ok)
        .filter(|e| e.depth() == 1)
        .map(|e| e.file_name().to_string_lossy().into_owned())
        .collect()
}

/// True when git ignores the folder `dir` itself, or when it is a `.git` folder.
pub fn ignored(dir: &Path) -> bool {
    let Some(parent) = dir.parent() else {
        return false;
    };
    if dir.file_name().is_some_and(|n| n == ".git") {
        return true;
    }
    let target = dir.to_path_buf();
    !walker(parent)
        .max_depth(Some(1))
        .filter_entry(move |e| e.depth() == 0 || e.path() == target)
        .build()
        .filter_map(std::result::Result::ok)
        .any(|e| e.depth() == 1)
}

/// True when git ignores `dir` or a folder between `root` and it.
pub fn ignored_within(root: &Path, dir: &Path) -> bool {
    dir.ancestors()
        .take_while(|a| *a != root && a.starts_with(root))
        .any(ignored)
}

#[derive(Debug, Default)]
struct Marks {
    /// The state of each name of the folder that has one.
    marks: HashMap<String, FileGit>,
    /// The whole folder is untracked.
    all_new: bool,
}

/// The git state of the names of `dir`, from one `git status` of that folder.
fn git_marks(dir: &Path) -> Marks {
    let env = [("GIT_OPTIONAL_LOCKS".to_string(), "0".to_string())];
    let prefix = git_limited(dir, &["rev-parse", "--show-prefix"], &env, GIT_LIMIT);
    if !prefix.ok {
        return Marks::default();
    }
    let status = git_limited(
        dir,
        &[
            "status",
            "--porcelain=v1",
            "-z",
            "--untracked-files=normal",
            "--ignored=no",
            "--",
            ".",
        ],
        &env,
        GIT_LIMIT,
    );
    if !status.ok {
        return Marks::default();
    }
    parse_marks(prefix.stdout.trim_end_matches('\n'), &status.stdout)
}

/// Reads `git status --porcelain=v1 -z` of the folder whose path from the root of the repo is
/// `prefix` (empty, or ending with `/`).
fn parse_marks(prefix: &str, status: &str) -> Marks {
    let mut out = Marks::default();
    let mut fields = status.split('\0');
    while let Some(field) = fields.next() {
        if field.len() < 4 {
            continue;
        }
        let (xy, path) = field.split_at(3);
        let xy = xy.as_bytes();
        if matches!(xy[0], b'R' | b'C') {
            // The old path of a rename or a copy.
            fields.next();
        }
        let state = match (xy[0], xy[1]) {
            (b'?', b'?') | (b'A', b' ') => FileGit::New,
            _ => FileGit::Changed,
        };
        match path.strip_prefix(prefix) {
            Some(rest) if !rest.is_empty() => {
                let name = rest.split('/').next().unwrap_or(rest).to_string();
                let mark = out.marks.entry(name).or_insert(state);
                if state == FileGit::Changed {
                    *mark = FileGit::Changed;
                }
            }
            _ if path.ends_with('/') && prefix.starts_with(path) => out.all_new = true,
            _ => {}
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn git(dir: &Path, args: &[&str]) {
        let ok = std::process::Command::new("git")
            .args(args)
            .current_dir(dir)
            .env("GIT_CONFIG_GLOBAL", "/dev/null")
            .env("GIT_CONFIG_SYSTEM", "/dev/null")
            .env("GIT_AUTHOR_NAME", "T")
            .env("GIT_AUTHOR_EMAIL", "t@example.invalid")
            .env("GIT_COMMITTER_NAME", "T")
            .env("GIT_COMMITTER_EMAIL", "t@example.invalid")
            .status()
            .unwrap()
            .success();
        assert!(ok, "git {args:?}");
    }

    /// A repo with a commit, a change, a new file, an ignored folder and a link outside.
    fn product() -> (tempfile::TempDir, Root, PathBuf) {
        let base = tempfile::tempdir().unwrap();
        let dir = base.path().join("p");
        let outside = base.path().join("outside");
        std::fs::create_dir_all(dir.join("src")).unwrap();
        std::fs::create_dir_all(&outside).unwrap();
        git(&dir, &["init", "-q"]);
        std::fs::write(dir.join(".gitignore"), "target/\n*.log\n").unwrap();
        std::fs::write(dir.join("src/lib.rs"), "a").unwrap();
        std::fs::write(dir.join("README.md"), "a").unwrap();
        git(&dir, &["add", "-A"]);
        git(&dir, &["commit", "-q", "-m", "x"]);
        std::fs::write(dir.join("src/lib.rs"), "b").unwrap();
        std::fs::write(dir.join("new.txt"), "n").unwrap();
        std::fs::write(dir.join("run.log"), "l").unwrap();
        std::fs::create_dir_all(dir.join("target/debug")).unwrap();
        std::fs::create_dir_all(dir.join("fresh/deep")).unwrap();
        std::fs::write(dir.join("fresh/deep/a.txt"), "a").unwrap();
        std::os::unix::fs::symlink(&outside, dir.join("out")).unwrap();
        let root = Root::new(&dir).unwrap();
        let outside = outside.canonicalize().unwrap();
        (base, root, outside)
    }

    fn names(r: &FilesListResult) -> Vec<(&str, FileKind, Option<FileGit>)> {
        r.entries
            .iter()
            .map(|e| (e.name.as_str(), e.kind, e.git))
            .collect()
    }

    #[test]
    fn the_list_gives_folders_first_skips_git_and_marks_the_git_state() {
        let (_b, root, _) = product();
        let r = root.list("").unwrap();
        assert_eq!(
            names(&r),
            [
                ("fresh", FileKind::Folder, Some(FileGit::New)),
                ("src", FileKind::Folder, Some(FileGit::Changed)),
                ("target", FileKind::Folder, Some(FileGit::Ignored)),
                (".gitignore", FileKind::File, None),
                ("new.txt", FileKind::File, Some(FileGit::New)),
                ("out", FileKind::Link, Some(FileGit::New)),
                ("README.md", FileKind::File, None),
                ("run.log", FileKind::File, Some(FileGit::Ignored)),
            ]
        );
        assert!(!r.truncated);
        assert_eq!(r.entries[4].size, 1);
        assert!(r.entries[4].modified.is_some());
        let src = root.list("src").unwrap();
        assert_eq!(
            names(&src),
            [("lib.rs", FileKind::File, Some(FileGit::Changed))]
        );
        // A folder that git does not track yet: each entry in it is new.
        let deep = root.list("fresh/deep").unwrap();
        assert_eq!(
            names(&deep),
            [("a.txt", FileKind::File, Some(FileGit::New))]
        );
        // The entries of an ignored folder are ignored too.
        std::fs::write(root.dir().join("target/debug/x.o"), "x").unwrap();
        let t = root.list("target/debug").unwrap();
        assert_eq!(names(&t), [("x.o", FileKind::File, Some(FileGit::Ignored))]);
    }

    #[test]
    fn a_folder_outside_a_repo_lists_with_no_git_state() {
        let base = tempfile::tempdir().unwrap();
        std::fs::write(base.path().join("a.txt"), "a").unwrap();
        std::fs::write(base.path().join(".gitignore"), "a.txt\n").unwrap();
        let root = Root::new(base.path()).unwrap();
        let r = root.list("").unwrap();
        assert!(r.entries.iter().all(|e| e.git.is_none()), "{r:?}");
    }

    #[test]
    fn create_makes_files_and_folders_with_their_parents_and_refuses_one_that_exists() {
        let (_b, root, _) = product();
        root.create("docs/tmp", NewKind::Folder).unwrap();
        assert!(root.dir().join("docs/tmp").is_dir());
        root.create("docs/tmp/a.md", NewKind::File).unwrap();
        assert!(root.dir().join("docs/tmp/a.md").is_file());
        assert!(matches!(
            root.create("docs/tmp/a.md", NewKind::File),
            Err(Error::Conflict(_))
        ));
        assert!(matches!(
            root.create("docs", NewKind::Folder),
            Err(Error::Conflict(_))
        ));
        assert!(matches!(
            root.create("", NewKind::Folder),
            Err(Error::Conflict(_))
        ));
        // A file is not a folder to create in.
        assert!(root.create("README.md/x", NewKind::File).is_err());
    }

    #[test]
    fn rename_moves_inside_the_product_and_never_replaces() {
        let (_b, root, _) = product();
        root.rename("new.txt", "src/moved.txt").unwrap();
        assert!(root.dir().join("src/moved.txt").is_file());
        assert!(!root.dir().join("new.txt").exists());
        assert!(matches!(
            root.rename("README.md", "src/moved.txt"),
            Err(Error::Conflict(_))
        ));
        assert_eq!(
            std::fs::read_to_string(root.dir().join("src/moved.txt")).unwrap(),
            "n"
        );
        assert!(matches!(
            root.rename("gone.txt", "x.txt"),
            Err(Error::NotFound(_))
        ));
        assert!(matches!(
            root.rename("README.md", "nofolder/x.md"),
            Err(Error::NotFound(_))
        ));
        // A link moves as a link.
        root.rename("out", "out2").unwrap();
        assert!(
            root.dir()
                .join("out2")
                .symlink_metadata()
                .unwrap()
                .file_type()
                .is_symlink()
        );
    }

    #[test]
    fn a_path_that_leaves_the_product_is_refused_and_nothing_is_made_outside() {
        let (base, root, outside) = product();
        for bad in [
            "../x",
            "a/../../x",
            "/etc/x",
            ".git/x",
            ".GIT/hooks/x",
            "src/.git",
            "a\0b",
        ] {
            assert!(
                matches!(root.create(bad, NewKind::File), Err(Error::Invalid(_))),
                "{bad:?}"
            );
        }
        assert!(!base.path().join("x").exists());
        // Through a link that points outside: no write, no list, no move, no trash.
        assert!(matches!(
            root.create("out/x.txt", NewKind::File),
            Err(Error::Invalid(_))
        ));
        assert!(matches!(
            root.create("out/a/b", NewKind::Folder),
            Err(Error::Invalid(_))
        ));
        assert!(matches!(root.list("out"), Err(Error::Invalid(_))));
        assert!(matches!(root.list(".."), Err(Error::Invalid(_))));
        assert!(matches!(
            root.rename("README.md", "out/README.md"),
            Err(Error::Invalid(_))
        ));
        std::fs::write(outside.join("keep.txt"), "k").unwrap();
        assert!(matches!(
            root.rename("out/keep.txt", "keep.txt"),
            Err(Error::Invalid(_))
        ));
        assert!(matches!(root.trash("out/keep.txt"), Err(Error::Invalid(_))));
        assert!(matches!(root.trash(""), Err(Error::Invalid(_))));
        assert!(matches!(root.trash("../outside"), Err(Error::Invalid(_))));
        assert_eq!(std::fs::read_dir(&outside).unwrap().count(), 1);
        assert!(outside.join("keep.txt").exists());
    }

    #[test]
    fn the_marks_of_a_status_name_the_first_part_under_the_folder() {
        let status = " M src/a.rs\0?? src/new/\0A  src/b.rs\0R  src/c.rs\0src/old.rs\0?? top.txt\0";
        let m = parse_marks("src/", status);
        assert_eq!(m.marks.get("a.rs"), Some(&FileGit::Changed));
        assert_eq!(m.marks.get("new"), Some(&FileGit::New));
        assert_eq!(m.marks.get("b.rs"), Some(&FileGit::New));
        assert_eq!(m.marks.get("c.rs"), Some(&FileGit::Changed));
        assert_eq!(m.marks.get("old.rs"), None);
        assert_eq!(m.marks.len(), 4);
        assert!(!m.all_new);
        assert!(parse_marks("src/new/", status).all_new);
        let root = parse_marks("", status);
        assert_eq!(root.marks.get("src"), Some(&FileGit::Changed));
        assert_eq!(root.marks.get("top.txt"), Some(&FileGit::New));
    }

    #[test]
    fn an_ignored_folder_and_the_folders_in_it_are_ignored() {
        let (_b, root, _) = product();
        let dir = root.dir();
        assert!(ignored_within(dir, &dir.join("target")));
        assert!(ignored_within(dir, &dir.join("target/debug")));
        assert!(!ignored_within(dir, &dir.join("src")));
        assert!(!ignored_within(dir, dir));
        assert_eq!(root.relative(&dir.join("src/a")), Some("src/a".into()));
        assert_eq!(root.relative(dir), Some(String::new()));
        assert_eq!(root.relative(Path::new("/elsewhere")), None);
    }

    #[test]
    fn find_ranks_by_name_skips_ignored_folders_git_and_links_and_keeps_to_the_cap() {
        let (_b, root, outside) = product();
        let dir = root.dir().to_path_buf();
        std::fs::write(dir.join("src/main.rs"), "m").unwrap();
        std::fs::create_dir_all(dir.join("app/src")).unwrap();
        std::fs::write(dir.join("app/src/main.rs"), "m").unwrap();
        std::fs::write(dir.join("app/main.rsx"), "m").unwrap();
        std::fs::write(dir.join("my_main.rs"), "m").unwrap();
        std::fs::write(dir.join("target/debug/main.rs"), "m").unwrap();
        std::fs::write(outside.join("main.rs"), "m").unwrap();
        std::fs::write(dir.join(".git/main.rs"), "m").unwrap();

        let found = root.find("Main.RS", 50);
        assert_eq!(
            found.paths,
            [
                "src/main.rs",
                "app/src/main.rs",
                "app/main.rsx",
                "my_main.rs"
            ]
        );
        assert!(!found.truncated);

        let capped = root.find("main", 2);
        assert_eq!(capped.paths, ["src/main.rs", "app/main.rsx"]);
        assert!(capped.truncated);

        assert_eq!(root.find("app/src/main", 50).paths, ["app/src/main.rs"]);
        assert_eq!(root.find("run.log", 50).paths, Vec::<String>::new());
        assert_eq!(root.find("  ", 50).paths, Vec::<String>::new());
        assert_eq!(root.find("deep/a.txt", 50).paths, ["fresh/deep/a.txt"]);
    }
}
