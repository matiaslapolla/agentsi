//! The `git` and `git.config` namespaces: the git screen of a product, and the git config
//! that AgentSI applies to each git command of a product.
//! Spec: `docs/specs/agentsi-desktop-git.md`. A method that writes sends
//! `data.changed {product, kind: "git"}`.

use crate::method::{Catalog, Method};
use crate::rpc::ErrorKind;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

pub fn register(c: &mut Catalog) {
    c.method::<GitStatus>()
        .method::<GitDiff>()
        .method::<GitStage>()
        .method::<GitUnstage>()
        .method::<GitCommit>()
        .method::<GitLog>()
        .method::<GitConfigGet>()
        .method::<GitConfigSet>();
}

/// The most commits that `git.log` gives.
pub const LOG_MAX: u32 = 50;
/// The most bytes of a patch that `git.diff` gives.
pub const DIFF_MAX_BYTES: usize = 512 * 1024;

/// A repo of a product. `repo` is one of the paths of the product, and defaults to the first.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct GitRepoParams {
    pub product: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub repo: Option<String>,
}

/// One changed file. A letter is one of `M` `A` `D` `R` `C` `T` `U` or `?`. The
/// field is null when that side has no change.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct GitFile {
    /// The path from the root of the repo.
    pub path: String,
    /// The old path of a renamed or copied file.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub orig_path: Option<String>,
    /// The state in the index (staged).
    #[serde(default)]
    pub staged: Option<String>,
    /// The state in the working tree (not staged). `?` is an untracked file, `U` an unmerged one.
    #[serde(default)]
    pub unstaged: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct GitWorktree {
    pub path: String,
    /// Null for a detached head.
    #[serde(default)]
    pub branch: Option<String>,
    /// The short id of the head.
    pub head: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct GitStatusResult {
    /// The repo that the core read.
    pub repo: String,
    /// The branch. Null for a detached head.
    #[serde(default)]
    pub branch: Option<String>,
    /// The short id of the head. Null in a repo with no commit.
    #[serde(default)]
    pub head: Option<String>,
    #[serde(default)]
    pub upstream: Option<String>,
    /// Null when the branch has no upstream.
    #[serde(default)]
    pub ahead: Option<u32>,
    #[serde(default)]
    pub behind: Option<u32>,
    pub worktrees: Vec<GitWorktree>,
    pub files: Vec<GitFile>,
}

pub struct GitStatus;

impl Method for GitStatus {
    const NAME: &'static str = "git.status";
    const DESCRIPTION: &'static str = "Gives the branch, the upstream with ahead and behind, \
        the worktrees, and each changed file of a repo of the product.";
    const ERRORS: &'static [ErrorKind] = &[ErrorKind::NotFound, ErrorKind::Unavailable];
    type Params = GitRepoParams;
    type Result = GitStatusResult;
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct GitDiffParams {
    pub product: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub repo: Option<String>,
    /// The path from the root of the repo.
    pub path: String,
    /// True for the staged patch, false for the unstaged one.
    pub staged: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct GitDiffResult {
    /// A unified patch. It is empty when the file has no change on that side.
    pub patch: String,
    /// True when the core cut the patch at 512 KiB.
    pub truncated: bool,
}

pub struct GitDiff;

impl Method for GitDiff {
    const NAME: &'static str = "git.diff";
    const DESCRIPTION: &'static str = "Gives the patch of one file, cut at 512 KiB. An untracked \
        file shows as a new file.";
    const ERRORS: &'static [ErrorKind] = &[ErrorKind::NotFound, ErrorKind::Unavailable];
    type Params = GitDiffParams;
    type Result = GitDiffResult;
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct GitPathsParams {
    pub product: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub repo: Option<String>,
    /// Paths from the root of the repo. A path that leaves the repo is refused.
    pub paths: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct GitDone {}

pub struct GitStage;

impl Method for GitStage {
    const NAME: &'static str = "git.stage";
    const DESCRIPTION: &'static str = "Stages the paths. It sends data.changed with kind git.";
    const ERRORS: &'static [ErrorKind] = &[ErrorKind::NotFound, ErrorKind::Unavailable];
    type Params = GitPathsParams;
    type Result = GitDone;
}

pub struct GitUnstage;

impl Method for GitUnstage {
    const NAME: &'static str = "git.unstage";
    const DESCRIPTION: &'static str = "Takes the paths out of the index. It sends data.changed \
        with kind git.";
    const ERRORS: &'static [ErrorKind] = &[ErrorKind::NotFound, ErrorKind::Unavailable];
    type Params = GitPathsParams;
    type Result = GitDone;
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct GitCommitParams {
    pub product: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub repo: Option<String>,
    pub message: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct GitCommitResult {
    /// False when git refused the commit, for example when a hook exits 1.
    pub committed: bool,
    /// The full id of the new commit. Null when git refused the commit.
    #[serde(default)]
    pub commit: Option<String>,
    /// The output of git and of the hooks.
    pub output: String,
}

pub struct GitCommit;

impl Method for GitCommit {
    const NAME: &'static str = "git.commit";
    const DESCRIPTION: &'static str = "Commits the staged files. The hooks of the repo run, and \
        a hook can refuse the commit: then committed is false and output holds its text.";
    const ERRORS: &'static [ErrorKind] = &[ErrorKind::NotFound, ErrorKind::Unavailable];
    type Params = GitCommitParams;
    type Result = GitCommitResult;
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct GitLogParams {
    pub product: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub repo: Option<String>,
    /// At most 50.
    pub limit: u32,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct GitCommitInfo {
    pub id: String,
    pub subject: String,
    pub author: String,
    /// Seconds since the Unix epoch.
    pub time: i64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct GitLogResult {
    pub commits: Vec<GitCommitInfo>,
}

pub struct GitLog;

impl Method for GitLog {
    const NAME: &'static str = "git.log";
    const DESCRIPTION: &'static str =
        "Gives at most 50 commits of the current branch, newest first.";
    const ERRORS: &'static [ErrorKind] = &[ErrorKind::NotFound, ErrorKind::Unavailable];
    type Params = GitLogParams;
    type Result = GitLogResult;
}

/// One git config key and its value.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct GitConfigEntry {
    pub key: String,
    /// An empty value in a product table removes the shared key for that product.
    pub value: String,
}

/// One key of the result, with the table that gave it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct GitConfigSource {
    pub key: String,
    pub value: String,
    /// `shared` or `product`.
    pub source: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct GitConfigGetParams {
    /// Null for the shared table only.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub product: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct GitConfigResult {
    /// The file that holds the tables: `<config>/agentsi/git.toml`.
    pub path: String,
    pub shared: Vec<GitConfigEntry>,
    /// The table of the product. Empty with no product.
    pub product: Vec<GitConfigEntry>,
    /// The keys that git gets for the product, with their source. Empty with no product.
    pub result: Vec<GitConfigSource>,
}

pub struct GitConfigGet;

impl Method for GitConfigGet {
    const NAME: &'static str = "git.config.get";
    const DESCRIPTION: &'static str = "Reads the shared git config table, the table of the \
        product, and the result for the product with the source of each value.";
    const ERRORS: &'static [ErrorKind] = &[ErrorKind::NotFound, ErrorKind::Unavailable];
    type Params = GitConfigGetParams;
    type Result = GitConfigResult;
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct GitConfigSetParams {
    /// Null writes the shared table.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub product: Option<String>,
    /// The whole table. It replaces the old one. An empty list removes the table.
    pub entries: Vec<GitConfigEntry>,
}

pub struct GitConfigSet;

impl Method for GitConfigSet {
    const NAME: &'static str = "git.config.set";
    const DESCRIPTION: &'static str = "Replaces one table of git.toml and keeps the other \
        tables and their comments. The core writes the file atomically and sends data.changed \
        with kind git. It never writes a .git/config or the global git config.";
    const ERRORS: &'static [ErrorKind] = &[ErrorKind::NotFound, ErrorKind::Unavailable];
    type Params = GitConfigSetParams;
    type Result = GitConfigResult;
}

/// True for a key that makes git start a program. The agents then run that program, so the
/// owner confirms the save.
pub fn starts_program(key: &str) -> bool {
    let key = key.to_ascii_lowercase();
    key.starts_with("alias.")
        || key.ends_with(".helper")
        || key.ends_with(".program")
        || matches!(
            key.as_str(),
            "core.sshcommand" | "core.fsmonitor" | "core.pager" | "core.editor" | "core.hookspath"
        )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_keys_that_start_a_program_are_found() {
        for key in [
            "core.sshCommand",
            "core.fsmonitor",
            "core.pager",
            "credential.helper",
            "credential.https://x.example.helper",
            "alias.st",
        ] {
            assert!(starts_program(key), "{key}");
        }
        for key in ["user.name", "user.email", "commit.gpgsign", "core.autocrlf"] {
            assert!(!starts_program(key), "{key}");
        }
    }
}
