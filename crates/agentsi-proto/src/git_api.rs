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
        .method::<GitSummary>()
        .method::<GitShow>()
        .method::<GitBranches>()
        .method::<GitBranchCreate>()
        .method::<GitSwitch>()
        .method::<GitOutgoing>()
        .method::<GitFetch>()
        .method::<GitPull>()
        .method::<GitPush>()
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

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct GitSummaryParams {
    /// The product whose first path the core reads. Give this or `cwd`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub product: Option<String>,
    /// Any folder, for example the folder of a terminal.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cwd: Option<String>,
}

/// The git summary of one folder: branch, upstream, ahead, behind and the count of changed
/// files.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct GitSummaryInfo {
    /// The root of the repo.
    pub repo: String,
    /// Null for a detached head.
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
    /// The files with a staged, unstaged or untracked change.
    pub changed: u32,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct GitSummaryResult {
    /// Null when the folder is not in a git repo.
    #[serde(default)]
    pub summary: Option<GitSummaryInfo>,
}

pub struct GitSummary;

impl Method for GitSummary {
    const NAME: &'static str = "git.summary";
    const DESCRIPTION: &'static str = "Gives the git summary of a product or of a folder: \
        branch, upstream, ahead, behind and the count of changed files. A folder outside a \
        repo gives a null summary.";
    const ERRORS: &'static [ErrorKind] = &[ErrorKind::NotFound, ErrorKind::Unavailable];
    type Params = GitSummaryParams;
    type Result = GitSummaryResult;
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct GitShowParams {
    pub product: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub repo: Option<String>,
    /// The id of the commit.
    pub commit: String,
}

/// One file of a commit.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct GitShowFile {
    pub path: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub orig_path: Option<String>,
    /// `A` `M` `D` `R` `C` or `T`.
    pub letter: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct GitShowResult {
    pub commit: GitCommitInfo,
    /// The files of the commit, against its first parent.
    pub files: Vec<GitShowFile>,
    /// The patch of all the files.
    pub patch: String,
    /// True when the core cut the patch at 512 KiB.
    pub truncated: bool,
}

pub struct GitShow;

impl Method for GitShow {
    const NAME: &'static str = "git.show";
    const DESCRIPTION: &'static str = "Gives the files of a commit and its patch against the \
        first parent, cut at 512 KiB.";
    const ERRORS: &'static [ErrorKind] = &[ErrorKind::NotFound, ErrorKind::Unavailable];
    type Params = GitShowParams;
    type Result = GitShowResult;
}

/// One local or remote branch.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct GitBranch {
    /// `main` for a local branch, `origin/main` for a remote one.
    pub name: String,
    /// The short id of the tip.
    pub head: String,
    /// The subject of the tip.
    pub subject: String,
    /// True for the branch of the worktree of the product.
    pub current: bool,
    #[serde(default)]
    pub upstream: Option<String>,
    /// Null with no upstream, or when the upstream is gone.
    #[serde(default)]
    pub ahead: Option<u32>,
    #[serde(default)]
    pub behind: Option<u32>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct GitBranchesResult {
    /// The local branches, the current one first.
    pub local: Vec<GitBranch>,
    pub remote: Vec<GitBranch>,
    pub worktrees: Vec<GitWorktree>,
}

pub struct GitBranches;

impl Method for GitBranches {
    const NAME: &'static str = "git.branches";
    const DESCRIPTION: &'static str = "Gives the local and the remote branches with upstream, \
        ahead and behind, and the worktrees of the repo.";
    const ERRORS: &'static [ErrorKind] = &[ErrorKind::NotFound, ErrorKind::Unavailable];
    type Params = GitRepoParams;
    type Result = GitBranchesResult;
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct GitBranchCreateParams {
    pub product: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub repo: Option<String>,
    /// The name of the new local branch.
    pub name: String,
    /// The branch or commit to start from. Null starts from the head.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub from: Option<String>,
}

pub struct GitBranchCreate;

impl Method for GitBranchCreate {
    const NAME: &'static str = "git.branch.create";
    const DESCRIPTION: &'static str = "Makes a local branch. It does not switch to it. It sends \
        data.changed with kind git.";
    const ERRORS: &'static [ErrorKind] = &[ErrorKind::NotFound, ErrorKind::Unavailable];
    type Params = GitBranchCreateParams;
    type Result = GitDone;
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct GitSwitchParams {
    pub product: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub repo: Option<String>,
    /// A local branch, or a remote branch such as `origin/main`. A remote branch gets a
    /// local branch that tracks it.
    pub name: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum GitOutcome {
    Done,
    /// `git.switch`: the tree has changes, and the core did not switch.
    Dirty,
    /// `git.pull`: the branch and its upstream both have new commits. The tree did not change.
    Diverged,
    /// Git refused or failed. `output` says why.
    Failed,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct GitSwitchResult {
    pub outcome: GitOutcome,
    /// The changed files of a `dirty` tree.
    pub files: Vec<GitFile>,
    /// The output of git.
    pub output: String,
}

pub struct GitSwitch;

impl Method for GitSwitch {
    const NAME: &'static str = "git.switch";
    const DESCRIPTION: &'static str = "Switches the worktree of the product to a branch. A tree \
        with changes gives dirty and the files, and the core does not switch. It sends \
        data.changed with kind git.";
    const ERRORS: &'static [ErrorKind] = &[ErrorKind::NotFound, ErrorKind::Unavailable];
    type Params = GitSwitchParams;
    type Result = GitSwitchResult;
}

/// What a push sends.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct GitOutgoingResult {
    pub branch: String,
    pub remote: String,
    /// The branch on the remote that gets the commits.
    pub remote_branch: String,
    /// True when the branch has no upstream: the push sets it.
    pub set_upstream: bool,
    /// At most 50 commits that go out, newest first.
    pub commits: Vec<GitCommitInfo>,
    /// The count of all the commits that go out.
    pub count: u32,
}

pub struct GitOutgoing;

impl Method for GitOutgoing {
    const NAME: &'static str = "git.outgoing";
    const DESCRIPTION: &'static str = "Gives the branch, the remote and the commits that \
        git.push sends, for the confirm of the app.";
    const ERRORS: &'static [ErrorKind] = &[ErrorKind::NotFound, ErrorKind::Unavailable];
    type Params = GitRepoParams;
    type Result = GitOutgoingResult;
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct GitSyncResult {
    pub outcome: GitOutcome,
    /// The output of git, progress lines left out.
    pub output: String,
}

pub struct GitFetch;

impl Method for GitFetch {
    const NAME: &'static str = "git.fetch";
    const DESCRIPTION: &'static str = "Fetches each remote of the repo, with a 120 s limit. It \
        sends git.progress lines while it runs, and data.changed with kind git after.";
    const ERRORS: &'static [ErrorKind] = &[ErrorKind::NotFound, ErrorKind::Unavailable];
    type Params = GitRepoParams;
    type Result = GitSyncResult;
}

pub struct GitPull;

impl Method for GitPull {
    const NAME: &'static str = "git.pull";
    const DESCRIPTION: &'static str = "Pulls the upstream with --ff-only, with a 120 s limit. A \
        branch that cannot fast-forward gives diverged, and the tree does not change. It sends \
        git.progress lines while it runs, and data.changed with kind git after.";
    const ERRORS: &'static [ErrorKind] = &[ErrorKind::NotFound, ErrorKind::Unavailable];
    type Params = GitRepoParams;
    type Result = GitSyncResult;
}

pub struct GitPush;

impl Method for GitPush {
    const NAME: &'static str = "git.push";
    const DESCRIPTION: &'static str = "Pushes the branch to its upstream, or with \
        --set-upstream to origin when it has none, with a 120 s limit. It never forces. It \
        sends git.progress lines while it runs, and data.changed with kind git after.";
    const ERRORS: &'static [ErrorKind] = &[ErrorKind::NotFound, ErrorKind::Unavailable];
    type Params = GitRepoParams;
    type Result = GitSyncResult;
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
