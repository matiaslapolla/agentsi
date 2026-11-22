//! The `files` namespace: the file tree of a product in the sidebar.
//! Spec: `docs/specs/agentsi-desktop-files.md`.
//!
//! Each path is relative to the folder of the product (its first path), with `/` between the
//! names. The empty path is the folder itself. The core refuses a path that leaves the folder
//! after it resolves the links, and a path in a `.git` folder. No method deletes for good:
//! `files.trash` moves to the system trash. A folder that `files.list` read is watched, and a
//! change in it sends `files.changed`.

use crate::method::{Catalog, Method};
use crate::rpc::ErrorKind;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

pub fn register(c: &mut Catalog) {
    c.method::<FilesList>()
        .method::<FilesCreate>()
        .method::<FilesRename>()
        .method::<FilesTrash>();
}

/// The most entries that `files.list` gives for one folder.
pub const LIST_MAX: usize = 5000;

#[derive(
    Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize, JsonSchema,
)]
#[serde(rename_all = "snake_case")]
pub enum FileKind {
    Folder,
    File,
    /// A symbolic link. The tree does not open it as a folder.
    Link,
}

/// The git state of an entry. A folder takes the state of the files in it: `changed` wins
/// over `new`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum FileGit {
    /// A tracked file with a staged or unstaged change.
    Changed,
    /// A file that git does not track yet, or that is only in the index.
    New,
    /// A file that `.gitignore`, `.git/info/exclude` or the global excludes file ignores.
    Ignored,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct FileEntry {
    pub name: String,
    pub kind: FileKind,
    /// Bytes. 0 for a folder.
    pub size: u64,
    /// Seconds since the Unix epoch. Null when the system does not give it.
    #[serde(default)]
    pub modified: Option<i64>,
    /// Null for a file with no change, and outside a git repo.
    #[serde(default)]
    pub git: Option<FileGit>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct FilesPathParams {
    pub product: String,
    /// From the folder of the product. Empty for the folder itself.
    #[serde(default)]
    pub path: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct FilesListResult {
    /// The path of the request.
    pub path: String,
    /// The folders first, then the files, each by name. No `.git` entry.
    pub entries: Vec<FileEntry>,
    /// True when the folder has more than 5000 entries and the list stops there.
    pub truncated: bool,
}

pub struct FilesList;

impl Method for FilesList {
    const NAME: &'static str = "files.list";
    const DESCRIPTION: &'static str = "Gives the entries of one folder of the product: name, \
        kind, size, modified time and git state, at most 5000. It leaves out .git and marks the \
        entries that git ignores. While a client subscribes to the files topic, the core \
        watches the folder, unless git ignores it, and sends files.changed.";
    const ERRORS: &'static [ErrorKind] = &[ErrorKind::NotFound, ErrorKind::Unavailable];
    type Params = FilesPathParams;
    type Result = FilesListResult;
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum NewKind {
    File,
    Folder,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct FilesCreateParams {
    pub product: String,
    /// The new path. The core makes each missing folder on the way, as `mkdir -p` does.
    pub path: String,
    pub kind: NewKind,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct FilesDone {}

pub struct FilesCreate;

impl Method for FilesCreate {
    const NAME: &'static str = "files.create";
    const DESCRIPTION: &'static str = "Makes an empty file or a folder in the product. A path \
        that exists gives conflict.";
    const ERRORS: &'static [ErrorKind] = &[
        ErrorKind::NotFound,
        ErrorKind::Conflict,
        ErrorKind::Unavailable,
    ];
    type Params = FilesCreateParams;
    type Result = FilesDone;
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct FilesRenameParams {
    pub product: String,
    pub from: String,
    /// The new path. Its folder must exist, and the path must not.
    pub to: String,
}

pub struct FilesRename;

impl Method for FilesRename {
    const NAME: &'static str = "files.rename";
    const DESCRIPTION: &'static str = "Renames or moves a file or a folder inside the product. \
        A link moves as a link. A target that exists gives conflict.";
    const ERRORS: &'static [ErrorKind] = &[
        ErrorKind::NotFound,
        ErrorKind::Conflict,
        ErrorKind::Unavailable,
    ];
    type Params = FilesRenameParams;
    type Result = FilesDone;
}

pub struct FilesTrash;

impl Method for FilesTrash {
    const NAME: &'static str = "files.trash";
    const DESCRIPTION: &'static str = "Moves a file or a folder of the product to the system \
        trash. A link goes to the trash, not its target. Nothing is deleted for good: a trash \
        that fails gives unavailable and the file stays.";
    const ERRORS: &'static [ErrorKind] = &[ErrorKind::NotFound, ErrorKind::Unavailable];
    type Params = FilesPathParams;
    type Result = FilesDone;
}
