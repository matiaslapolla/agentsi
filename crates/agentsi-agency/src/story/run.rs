//! The story session of `story-run.ts`: `queueStoryRun` and
//! `executeStoryRun`. The core runs a queued session on a thread of its own.
//! The session records the pid of the core, and the registry of this module
//! lets `reject --force` stop it: it stops the child process group (claude, a
//! test, a gate) and the session ends at its next step.

use super::claude::{
    McpAccess, SessionCall, SessionResult, bash_allowlist, bash_denylist, run_session, test_command,
};
use super::git::{branch_exists, commit_files, git, git_out, main_branch};
use super::inputs;
use super::record::{action_error, file_base, fixed};
use super::store::{self, log_ref, read_with, transaction};
use super::{Error, Result, code_graph};
use crate::clock;
use crate::registry::Product;
use crate::sandbox::{self, Mask, kill_group, node_modules_dirs, run_command, sandboxed};
use agentsi_proto::story_api::{Story, StoryConfig, StorySession, StoryStop};
use agentsi_store::Store;
use regex::Regex;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, LazyLock, Mutex};
use std::time::Duration;

const SESSION_TIMEOUT: Duration = Duration::from_secs(30 * 60);
const COMMAND_TIMEOUT: Duration = Duration::from_secs(15 * 60);
const OUTPUT_TAIL: usize = 4000;
const MAX_ATTEMPTS: i64 = 2;
const TRAILER: &str = "\n\nCo-Authored-By: Claude <noreply@anthropic.com>";
/// QA writes and repairs the criterion tests on this model, whatever the model of its role.
pub const QA_TEST_MODEL: &str = "sonnet";

pub const NO_SANDBOX: &str = "bwrap cannot start a sandbox. Story sessions run tests and gates inside bubblewrap (Linux only): install the bubblewrap package, check it with `agentsi doctor` and run again.";
pub const LINUX_ONLY: &str = "Story sessions run only on a Linux host: the tests and gates run inside bubblewrap, and this system has none. Run the story from a core on a Linux host.";

/// Why no sandbox can start here, or `None` when one can.
pub fn sandbox_error() -> Option<&'static str> {
    if !cfg!(target_os = "linux") {
        return Some(LINUX_ONLY);
    }
    (!sandbox::works()).then_some(NO_SANDBOX)
}

pub fn archived_error(id: &str) -> String {
    format!("Story {id} is archived. Run agentsi restore {id} first.")
}

/// A session that runs in this process.
#[derive(Default)]
struct Handle {
    cancelled: AtomicBool,
    child: Mutex<Option<u32>>,
}

/// The sessions that run in this process, by product and story id.
type Running = HashMap<(String, String), Arc<Handle>>;

static RUNNING: LazyLock<Mutex<Running>> = LazyLock::new(|| Mutex::new(HashMap::new()));
/// Set when the core stops. A session that starts after this is cancelled at once.
static STOPPING: AtomicBool = AtomicBool::new(false);

fn running() -> std::sync::MutexGuard<'static, Running> {
    RUNNING.lock().unwrap_or_else(|e| e.into_inner())
}

fn stop_handle(h: &Handle) {
    h.cancelled.store(true, Ordering::SeqCst);
    if let Some(pgid) = *h.child.lock().unwrap_or_else(|e| e.into_inner()) {
        kill_group(pgid);
    }
}

/// True while a session of this story runs in this process.
pub fn is_running(product: &str, id: &str) -> bool {
    running().contains_key(&(product.into(), id.into()))
}

/// The product and the id of each story whose session runs in this process.
pub fn active() -> Vec<(String, String)> {
    let mut all: Vec<_> = running().keys().cloned().collect();
    all.sort();
    all
}

/// Stops the session of this story that runs in this process: its child
/// process group now, and the session at its next step.
pub fn cancel(product: &str, id: &str) {
    if let Some(h) = running().get(&(product.into(), id.into())) {
        stop_handle(h);
    }
}

/// Stops each session of this process, as `cancel` does, and each session that
/// starts later. Only a core that stops calls it. The story keeps its last saved
/// state and the pid of the core, so `reject` works on it with no `--force`.
pub fn stop_all() {
    let all = running();
    STOPPING.store(true, Ordering::SeqCst);
    all.values().for_each(|h| stop_handle(h));
}

/// True when no session runs in this process.
pub fn idle() -> bool {
    running().is_empty()
}

/// The session caller. The core uses `run_session`; a test passes a fake.
pub type Caller = dyn Fn(&SessionCall, &dyn Fn(Option<u32>)) -> std::result::Result<SessionResult, String>
    + Send
    + Sync;

/// What a session run works with.
#[derive(Clone)]
pub struct RunCtx {
    pub store: Arc<Store>,
    /// The data root: the role files and the mask.
    pub root: PathBuf,
    pub product: Product,
    pub config: StoryConfig,
    /// Called after each write of the story.
    pub notify: Arc<dyn Fn() + Send + Sync>,
    pub caller: Arc<Caller>,
    /// Tests turn the code graph off.
    pub code_graph: bool,
}

impl RunCtx {
    pub fn new(store: Arc<Store>, root: PathBuf, product: Product, config: StoryConfig) -> RunCtx {
        RunCtx {
            store,
            root,
            product,
            config,
            notify: Arc::new(|| {}),
            caller: Arc::new(|c: &SessionCall, on: &dyn Fn(Option<u32>)| {
                run_session(c, SESSION_TIMEOUT, on)
            }),
            code_graph: true,
        }
    }
}

/// Checks that the story may run and records a `queued` session with its
/// branch, worktree and base.
pub fn queue(ctx: &RunCtx, id: &str) -> Result<Story> {
    let repo = Path::new(&ctx.config.repo);
    let product = &ctx.product;
    transaction(&product.id, || {
        let (story, archived) = read_with(&ctx.store, |c| {
            Ok((
                store::load(c, &product.id, id)?,
                store::archived_ids(c, &product.id)?.contains(id),
            ))
        })?;
        if archived {
            return Err(Error::invalid(archived_error(id)));
        }
        if let Some(e) = action_error(&story, "run") {
            return Err(Error::invalid(e));
        }
        if is_running(&product.id, id) {
            return Err(Error::invalid(format!(
                "The rejected story session of {id} still stops. Run it again in a moment."
            )));
        }
        if let Some(e) = sandbox_error() {
            return Err(Error::invalid(e));
        }
        let name = file_base(&story);
        let branch = format!("story/{name}");
        let worktree = repo.join(".worktrees").join(&name);
        if !git(
            repo,
            &["check-ignore", "-q", &format!(".worktrees/{name}/")],
        )
        .ok
        {
            return Err(Error::invalid(format!(
                ".worktrees/ is not gitignored in {}. Add it to the product's .gitignore before a story session.",
                ctx.config.repo
            )));
        }
        if branch_exists(repo, &branch) || worktree.exists() {
            return Err(Error::invalid(format!(
                "Branch {branch} or worktree {} exists from an earlier session. Run agentsi story reject {id} first.",
                worktree.display()
            )));
        }
        let base = git_out(repo, &["rev-parse", &main_branch(repo)?])?;
        let session = StorySession {
            state: "queued".into(),
            branch,
            worktree: worktree.to_string_lossy().into_owned(),
            base,
            tests_commit: None,
            final_commit: None,
            attempts: 0,
            failure: None,
            log: log_ref(&product.id, id),
            started_at: clock::iso(clock::now_ms()),
            pid: Some(None),
            stop: None,
        };
        store::save(
            &ctx.store,
            product,
            Story {
                session: Some(session),
                ..story
            },
        )
    })
}

/// The criterion test paths of a story, each one once.
pub fn test_paths(s: &Story) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for c in &s.criteria {
        if let Some(t) = &c.test
            && !t.is_empty()
            && !out.contains(t)
        {
            out.push(t.clone());
        }
    }
    out
}

/// A criterion test that the owner session says contradicts its criterion.
#[derive(Debug, Clone, PartialEq)]
pub struct TestDispute {
    pub test: String,
    pub reason: String,
}

static DISPUTE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"^test_disputed:\s*(\S+?):\s+(.+)$").unwrap());

/// The last `test_disputed: <test path>: <reason>` line of a session answer whose path is in `paths`.
pub fn test_dispute(text: &str, paths: &[String]) -> Option<TestDispute> {
    for line in text.split('\n').rev() {
        let clean: String = line.chars().filter(|c| *c != '`' && *c != '*').collect();
        if let Some(m) = DISPUTE.captures(clean.trim())
            && paths.contains(&m[1].to_string())
        {
            return Some(TestDispute {
                test: m[1].into(),
                reason: m[2].trim().into(),
            });
        }
    }
    None
}

/// The install command of a checkout, from its lockfile. It runs outside the
/// sandbox, so it runs no package scripts.
pub fn install_command(dir: &Path) -> Option<&'static str> {
    const INSTALL: [(&str, &str); 5] = [
        (
            "pnpm-lock.yaml",
            "pnpm install --frozen-lockfile --prefer-offline --ignore-scripts",
        ),
        ("package-lock.json", "npm ci --ignore-scripts"),
        (
            "yarn.lock",
            "yarn install --frozen-lockfile --ignore-scripts",
        ),
        ("bun.lock", "bun install --frozen-lockfile --ignore-scripts"),
        (
            "bun.lockb",
            "bun install --frozen-lockfile --ignore-scripts",
        ),
    ];
    let (file, command) = INSTALL.iter().find(|(f, _)| dir.join(f).exists())?;
    // Yarn 2 and later has no --ignore-scripts. Its skip-build mode runs no package scripts.
    if *file == "yarn.lock" && dir.join(".yarnrc.yml").exists() {
        return Some("yarn install --immutable --mode=skip-build");
    }
    Some(command)
}

fn tail(s: &str) -> String {
    let n = s.chars().count();
    if n > OUTPUT_TAIL {
        format!("…{}", s.chars().skip(n - OUTPUT_TAIL).collect::<String>())
    } else {
        s.to_string()
    }
}

fn lines(s: &str) -> Vec<String> {
    s.split('\n')
        .map(str::trim)
        .filter(|l| !l.is_empty())
        .map(String::from)
        .collect()
}

fn story_text(s: &Story) -> String {
    let mut out = vec![format!("# Story {}: {}", s.id, s.title), String::new()];
    if let Some(a) = s.as_a.as_deref().filter(|a| !a.is_empty()) {
        out.push(format!("As a {a},"));
    }
    out.push(format!("I want {},", s.i_want));
    out.push(format!("so that {}.", s.so_that));
    out.extend([String::new(), "## Criteria".into(), String::new()]);
    for (i, c) in s.criteria.iter().enumerate() {
        out.push(format!(
            "{}. Given {}, when {}, then {}.",
            i + 1,
            c.given,
            c.when,
            c.then
        ));
        out.push(format!(
            "   Test: {}. It checks: {}",
            c.test.as_deref().unwrap_or("none"),
            c.checks
        ));
    }
    out.join("\n")
}

fn system_prompt(body: &str, memory: &str, story: &Story) -> String {
    [
        body,
        "## Your memory of this product",
        if memory.is_empty() { "(empty)" } else { memory },
        &story_text(story),
    ]
    .join("\n\n")
}

const COMMON_RULES: [&str; 3] = [
    "- You work in a git worktree of the product repo. Read CLAUDE.md and CONTEXT.md at its root, if they exist, for the conventions.",
    "- Do not run git. The runner commits your changes.",
    "- Do not push, deploy, migrate a database or call a network service.",
];

fn qa_prompt(s: &Story, config: &StoryConfig) -> String {
    let paths = test_paths(s);
    let mut out = vec![
        format!(
            "Write the criterion tests of story {}, and nothing else.",
            s.id
        ),
        String::new(),
    ];
    for c in s
        .criteria
        .iter()
        .filter(|c| c.test.as_deref().is_some_and(|t| !t.is_empty()))
    {
        out.push(format!(
            "- {}: given {}, when {}, then {}.",
            c.test.as_deref().unwrap_or_default(),
            c.given,
            c.when,
            c.then
        ));
    }
    out.push(String::new());
    out.push("- Each test checks its criterion as a user of the product sees it.".into());
    out.push("- Each test must fail now, because nobody has done the work yet. Do not implement the story.".into());
    out.push("- Write only the test files above. Change no other file.".into());
    out.push(format!(
        "- Run the tests with: {}",
        test_command(config, &paths)
    ));
    out.extend(COMMON_RULES.iter().map(|r| r.to_string()));
    out.join("\n")
}

fn repair_prompt(s: &Story, config: &StoryConfig, d: &TestDispute) -> String {
    let mut out = vec![
        format!(
            "The owner session of story {} disputes the criterion test {}. Its reason: {}",
            s.id, d.test, d.reason
        ),
        String::new(),
    ];
    for c in s
        .criteria
        .iter()
        .filter(|c| c.test.as_deref() == Some(d.test.as_str()))
    {
        out.push(format!(
            "- The criterion: given {}, when {}, then {}.",
            c.given, c.when, c.then
        ));
    }
    out.push("- If the test contradicts its criterion, correct the test. If the test is right, leave it as it is.".into());
    out.push(format!("- Change only {}. Change no other file.", d.test));
    out.push("- The test must still fail now, because nobody has done the work yet. Do not implement the story.".into());
    out.push(format!(
        "- Run it with: {}",
        test_command(config, std::slice::from_ref(&d.test))
    ));
    out.extend(COMMON_RULES.iter().map(|r| r.to_string()));
    out.join("\n")
}

fn owner_prompt(
    s: &Story,
    config: &StoryConfig,
    failure: Option<&str>,
    note: Option<&str>,
) -> String {
    let paths = test_paths(s);
    let mut out = vec![
        if paths.is_empty() {
            format!("Do the work of task {}.", s.id)
        } else {
            format!(
                "Do the work of story {}, so that its criterion tests pass.",
                s.id
            )
        },
        String::new(),
    ];
    if !paths.is_empty() {
        out.push(format!(
            "- The criterion tests are: {}. Do not change them.",
            paths.join(", ")
        ));
        out.push(format!("- Run them with: {}", test_command(config, &paths)));
        out.push("- Before you change code, read each criterion test and run the tests.".into());
        out.push("- If a test contradicts its criterion, change no file. End your answer with this line: test_disputed: <test path>: <reason>".into());
        out.push(
            "- Then QA repairs that test one time, and a new owner session does the work.".into(),
        );
    }
    if !config.gate.is_empty() {
        out.push(format!(
            "- These gate commands must pass: {}",
            config.gate.join("; ")
        ));
    }
    out.extend(COMMON_RULES.iter().map(|r| r.to_string()));
    if let Some(n) = note {
        out.extend([String::new(), n.to_string()]);
    }
    if let Some(f) = failure {
        out.extend([
            String::new(),
            "The previous attempt failed the checks. Correct it.".into(),
            String::new(),
            f.to_string(),
        ]);
    }
    out.join("\n")
}

/// Paths under .claude/ or .worktrees/ that differ from HEAD, ignored files
/// included: Claude Code loads config from there.
fn agent_config_changes(wt: &Path) -> Result<Vec<String>> {
    let r = git(
        wt,
        &[
            "status",
            "--porcelain",
            "--ignored",
            "--untracked-files=all",
            "--",
            ".claude",
            ".worktrees",
        ],
    );
    if !r.ok {
        return Err(Error::invalid(format!(
            "git status failed in {}: {}",
            wt.display(),
            r.stderr.trim()
        )));
    }
    Ok(r.stdout
        .split('\n')
        .filter(|l| !l.is_empty())
        .map(|l| l.get(3..).unwrap_or_default().to_string())
        .collect())
}

/// The first agent of the roster whose role is `qa`.
fn qa_agent(ctx: &RunCtx) -> Result<String> {
    let product = &ctx.product;
    read_with(&ctx.store, |c| {
        for a in &product.roles {
            if inputs::role(c, &ctx.root, &product.id, a).is_ok_and(|d| d.role == "qa") {
                return Ok(a.clone());
            }
        }
        Err(Error::invalid(format!(
            "The roster of {} has no qa agent to write the criterion tests.",
            product.id
        )))
    })
}

/// The state of one run of `execute`.
struct Run<'a> {
    ctx: &'a RunCtx,
    id: String,
    started: String,
    story: Story,
    handle: Arc<Handle>,
    mask: Mask,
    graph: Option<(McpAccess, String)>,
    git_link: String,
    log_error: Option<String>,
}

impl Run<'_> {
    fn gone(&self) -> Error {
        let why = if STOPPING.load(Ordering::SeqCst) {
            "stopped with the core"
        } else {
            "was rejected while it ran"
        };
        Error::Gone(format!("The story session of {} {why}.", self.id))
    }

    fn live(&self) -> Result<()> {
        if self.handle.cancelled.load(Ordering::SeqCst) {
            return Err(self.gone());
        }
        Ok(())
    }

    fn log(&mut self, text: &str) {
        let line = format!("[{}] {text}\n", clock::iso(clock::now_ms()));
        if let Err(e) = store::append_log(&self.ctx.store, &self.ctx.product.id, &self.id, &line) {
            self.log_error.get_or_insert(e.to_string());
        }
    }

    /// Saves a change of the session, unless the session was rejected.
    fn update(&mut self, patch: impl FnOnce(&mut StorySession)) -> Result<()> {
        let ctx = self.ctx;
        let (pid, id, started) = (&ctx.product.id, self.id.clone(), self.started.clone());
        let gone = self.gone();
        // Before the lock: `reject` waits for this thread, and holds no lock while it waits.
        self.live()?;
        let handle = self.handle.clone();
        let saved = transaction(pid, || {
            let fresh = read_with(&ctx.store, |c| store::load(c, pid, &id))?;
            let cancelled = handle.cancelled.load(Ordering::SeqCst);
            let mut session = match &fresh.session {
                Some(s) if s.started_at == started && !cancelled => s.clone(),
                _ => return Err(gone),
            };
            patch(&mut session);
            store::save(
                &ctx.store,
                &ctx.product,
                Story {
                    session: Some(session),
                    ..fresh
                },
            )
        })?;
        self.story = saved;
        (ctx.notify)();
        Ok(())
    }

    fn fail(&mut self, failure: &str, stop: Option<StoryStop>) -> Result<Story> {
        self.log(&format!("Failed: {failure}"));
        let failure = failure.to_string();
        self.update(|s| {
            s.state = "failed".into();
            s.failure = Some(failure);
            if let Some(stop) = stop {
                s.stop = Some(Some(stop));
            }
        })?;
        Ok(self.story.clone())
    }

    fn stopped(&mut self, agent: &str, stop: StoryStop) -> Result<Story> {
        let text = format!(
            "Budget: the {agent} session reached its cap of USD {} (stories.budget_usd in the registry) and Claude Code stopped it.",
            fixed(stop.cap_usd, 2)
        );
        self.fail(&text, Some(stop))
    }

    fn on_start(&self) -> impl Fn(Option<u32>) + '_ {
        move |pid| {
            *self.handle.child.lock().unwrap_or_else(|e| e.into_inner()) = pid;
            if let Some(pid) = pid
                && self.handle.cancelled.load(Ordering::SeqCst)
            {
                kill_group(pid);
            }
        }
    }

    fn wt(&self) -> PathBuf {
        PathBuf::from(
            &self
                .story
                .session
                .as_ref()
                .map(|s| s.worktree.clone())
                .unwrap_or_default(),
        )
    }

    /// Runs one session: its answer, and its stop when Claude Code stopped it at the cap.
    fn session(
        &mut self,
        agent: &str,
        prompt: &str,
        model: Option<&str>,
    ) -> Result<(String, Option<StoryStop>)> {
        self.live()?;
        let ctx = self.ctx;
        let product = &ctx.product.id;
        let (def, memory) = read_with(&ctx.store, |c| {
            Ok((
                inputs::role(c, &ctx.root, product, agent)?,
                inputs::memory_for_prompt(c, product, agent)?,
            ))
        })?;
        let model = model.unwrap_or(&def.model).to_string();
        self.log(&format!("Session {agent} ({model}) starts."));
        let wt = self.wt();
        let paths = test_paths(&self.story);
        let mut deny_write = vec![
            Path::new(&ctx.config.repo)
                .join(".git")
                .to_string_lossy()
                .into_owned(),
            wt.join(".git").to_string_lossy().into_owned(),
        ];
        deny_write.extend(node_modules_dirs(&wt));
        let call = SessionCall {
            agent: agent.into(),
            model: model.clone(),
            effort: def.effort.clone(),
            system: system_prompt(&def.body, &memory, &self.story),
            prompt: match &self.graph {
                Some((_, rule)) => format!("{prompt}\n{rule}"),
                None => prompt.into(),
            },
            cwd: wt.to_string_lossy().into_owned(),
            bash: bash_allowlist(&ctx.config, &paths),
            deny_bash: bash_denylist(&ctx.config),
            deny_write,
            mask: self.mask.clone(),
            budget_usd: ctx.config.budget_usd,
            mcp: self.graph.as_ref().map(|g| g.0.clone()),
        };
        let r = (ctx.caller)(&call, &self.on_start()).map_err(Error::invalid)?;
        self.live()?;
        self.log(&format!(
            "Session {agent} ends after {}s, ${}:\n{}",
            (r.duration_ms as f64 / 1000.0).round(),
            fixed(r.cost_usd, 2),
            tail(&r.text)
        ));
        let stop = r.budget_stop.then(|| StoryStop {
            reason: "budget".into(),
            cap_usd: ctx.config.budget_usd,
        });
        Ok((r.text, stop))
    }

    /// Why the changes of the session cannot go to git: a changed git link, or
    /// files where Claude Code loads its config.
    fn escaped(&self, who: &str) -> Result<Option<String>> {
        let wt = self.wt();
        if std::fs::read_to_string(wt.join(".git")).unwrap_or_default() != self.git_link {
            return Ok(Some(format!(
                "{who} changed the .git file of the worktree."
            )));
        }
        let config = agent_config_changes(&wt)?;
        Ok((!config.is_empty()).then(|| {
            format!(
                "{who} left files in .claude/ or .worktrees/: {}",
                config.join(", ")
            )
        }))
    }

    /// A command of the story in the worktree sandbox: its result and the tail of its output.
    fn in_sandbox(&self, command: &str) -> Result<(bool, String)> {
        self.live()?;
        let r = sandboxed(
            &self.wt(),
            command,
            COMMAND_TIMEOUT,
            &self.mask,
            &self.on_start(),
        );
        self.live()?;
        if let Some(e) = self.escaped("A test or gate command")? {
            return Err(Error::invalid(e));
        }
        Ok((r.ok, tail(format!("{}{}", r.stdout, r.stderr).trim())))
    }

    fn head(&self) -> Result<String> {
        git_out(&self.wt(), &["rev-parse", "HEAD"])
    }

    /// The whole run. An `Err` that is not `Gone` becomes the failure of the session.
    fn steps(&mut self) -> Result<Story> {
        let ctx = self.ctx;
        let config = &ctx.config;
        let id = self.id.clone();
        let own = i64::from(std::process::id());
        self.update(|s| s.pid = Some(Some(own)))?;
        let s = self
            .story
            .session
            .clone()
            .unwrap_or_else(|| unreachable!("queued"));
        self.log(&format!(
            "Story session of {id} starts on {} from {}.",
            s.branch, s.base
        ));
        if let Some(e) = sandbox_error() {
            return self.fail(e, None);
        }
        let repo = Path::new(&config.repo);
        self.mask = sandbox::story_mask(repo, &sandbox::home(), &ctx.root);
        if ctx.code_graph {
            let lines = std::cell::RefCell::new(Vec::new());
            let project = code_graph::reindex(
                repo,
                &|l| lines.borrow_mut().push(l.to_string()),
                &self.on_start(),
            );
            for l in lines.into_inner() {
                self.log(&l);
            }
            if let Some(project) = project {
                let rule = format!(
                    "- The code graph tools (mcp__codegraph__*, project {project}) find symbols, callers and call paths fast. Read the file to confirm before you rely on it."
                );
                self.graph = Some((code_graph::mcp(), rule));
            }
        }
        self.live()?;
        git_out(
            repo,
            &["worktree", "add", "-b", &s.branch, &s.worktree, &s.base],
        )?;
        let wt = self.wt();
        self.git_link = std::fs::read_to_string(wt.join(".git")).unwrap_or_default();
        if let Some(install) = install_command(&wt) {
            self.log(&format!("Install: {install}"));
            self.live()?;
            let r = run_command(
                "sh",
                &["-c".into(), install.into()],
                &wt,
                COMMAND_TIMEOUT,
                &self.on_start(),
            );
            if !r.ok {
                let out = tail(format!("{}{}", r.stdout, r.stderr).trim());
                return self.fail(&format!("The install failed: {install}\n{out}"), None);
            }
        }

        let paths = test_paths(&self.story);
        if !paths.is_empty() {
            self.update(|s| s.state = "tests".into())?;
            let qa = qa_agent(ctx)?;
            let prompt = qa_prompt(&self.story, config);
            let (_, stop) = self.session(&qa, &prompt, Some(QA_TEST_MODEL))?;
            if let Some(e) = self.escaped("QA")? {
                return self.fail(&e, None);
            }
            if let Some(stop) = stop {
                return self.stopped(&qa, stop);
            }
            git_out(&wt, &["add", "-A"])?;
            let changed = lines(&git_out(&wt, &["diff", "--cached", "--name-only"])?);
            let outside: Vec<&String> = changed.iter().filter(|f| !paths.contains(f)).collect();
            if !outside.is_empty() {
                let list: Vec<&str> = outside.iter().map(|s| s.as_str()).collect();
                return self.fail(
                    &format!(
                        "QA changed files outside the criterion tests: {}",
                        list.join(", ")
                    ),
                    None,
                );
            }
            let missing: Vec<&str> = paths
                .iter()
                .filter(|p| !changed.contains(p))
                .map(String::as_str)
                .collect();
            if !missing.is_empty() {
                return self.fail(
                    &format!(
                        "QA did not write these criterion tests: {}",
                        missing.join(", ")
                    ),
                    None,
                );
            }
            let refs: Vec<&str> = paths.iter().map(String::as_str).collect();
            if !commit_files(&wt, &refs, &format!("test({id}): criteria tests{TRAILER}"))
                .map_err(|e| Error::invalid(format!("The tests commit failed: {e}")))?
            {
                return Err(Error::invalid("The tests commit failed: nothing to commit"));
            }
            let head = self.head()?;
            self.update(|s| s.tests_commit = Some(head))?;
            for file in &paths {
                let (ok, out) =
                    self.in_sandbox(&test_command(config, std::slice::from_ref(file)))?;
                if ok {
                    return self.fail(
                        &format!("The test {file} passes on tests_commit; it must fail before the work.\n{out}"),
                        None,
                    );
                }
            }
            self.log("Each criterion test fails on tests_commit.");
        }

        let mut failure: Option<String> = None;
        let mut note: Option<String> = None;
        let mut repaired = false;
        let mut attempt = 1;
        // The disputed attempt does not use up one of the attempts.
        while attempt <= MAX_ATTEMPTS + i64::from(repaired) {
            let f = failure.clone();
            self.update(|s| {
                s.state = "work".into();
                s.attempts = attempt;
                s.failure = f;
            })?;
            let owner_role = self.story.owner_role.clone();
            let prompt = owner_prompt(&self.story, config, failure.as_deref(), note.as_deref());
            let (text, stop) = self.session(&owner_role, &prompt, None)?;
            if let Some(e) = self.escaped("The owner session")? {
                return self.fail(&e, None);
            }
            if let Some(stop) = stop {
                return self.stopped(&owner_role, stop);
            }
            let has_tests = self
                .story
                .session
                .as_ref()
                .is_some_and(|s| s.tests_commit.as_deref().is_some_and(|c| !c.is_empty()));
            let dispute = if has_tests {
                test_dispute(&text, &paths)
            } else {
                None
            };
            if let Some(d) = dispute {
                self.log(&format!(
                    "The owner session disputes {}: {}",
                    d.test, d.reason
                ));
                if repaired {
                    return self.fail(
                        &format!(
                            "The owner session disputes the criterion test {} again, after the QA repair: {}",
                            d.test, d.reason
                        ),
                        None,
                    );
                }
                repaired = true;
                match self.repair(&d)? {
                    Ok(outcome) => {
                        failure = None;
                        note = Some(outcome);
                        attempt += 1;
                        continue;
                    }
                    Err(failed) => return Ok(failed),
                }
            }
            let title = &self.story.title;
            let suffix = if attempt > 1 {
                format!(" (attempt {attempt})")
            } else {
                String::new()
            };
            commit_files(
                &wt,
                &["."],
                &format!("feat({id}): {title}{suffix}{TRAILER}"),
            )
            .map_err(|e| Error::invalid(format!("The work commit failed: {e}")))?;
            self.update(|s| s.state = "checks".into())?;
            failure = self.checks()?;
            if failure.is_none() {
                self.log("Every check passes.");
                let head = self.head()?;
                self.update(|s| {
                    s.state = "ready".into();
                    s.failure = None;
                    s.final_commit = Some(head);
                })?;
                return Ok(self.story.clone());
            }
            self.log(&format!(
                "Check failed on attempt {attempt}: {}",
                failure.as_deref().unwrap_or_default()
            ));
            attempt += 1;
        }
        let f = failure.unwrap_or_default();
        self.fail(&f, None)
    }

    /// QA's one repair round: the work goes, and QA may change the disputed
    /// test. `Ok(note)` for the owner, or `Err(story)` when the run failed.
    fn repair(&mut self, d: &TestDispute) -> Result<std::result::Result<String, Story>> {
        let ctx = self.ctx;
        let config = &ctx.config;
        let id = self.id.clone();
        let wt = self.wt();
        let tests_commit = self
            .story
            .session
            .as_ref()
            .and_then(|s| s.tests_commit.clone())
            .unwrap_or_default();
        git_out(&wt, &["reset", "-q", "--hard", &tests_commit])?;
        git_out(&wt, &["clean", "-fdq"])?;
        self.update(|s| s.state = "tests".into())?;
        let qa = qa_agent(ctx)?;
        let prompt = repair_prompt(&self.story, config, d);
        let (_, stop) = self.session(&qa, &prompt, Some(QA_TEST_MODEL))?;
        if let Some(e) = self.escaped("QA")? {
            return self.fail(&e, None).map(Err);
        }
        if let Some(stop) = stop {
            return self.stopped(&qa, stop).map(Err);
        }
        git_out(&wt, &["add", "-A"])?;
        let changed = lines(&git_out(&wt, &["diff", "--cached", "--name-only"])?);
        let outside: Vec<&str> = changed
            .iter()
            .filter(|f| **f != d.test)
            .map(String::as_str)
            .collect();
        if !outside.is_empty() {
            return self
                .fail(
                    &format!(
                        "QA changed files outside the disputed test {}: {}",
                        d.test,
                        outside.join(", ")
                    ),
                    None,
                )
                .map(Err);
        }
        let last = "QA repairs no test again in this run. A new dispute fails the run.";
        if changed.is_empty() {
            return Ok(Ok(format!(
                "QA reviewed the disputed test {} and kept it as it is. The dispute was: {}\n{last}",
                d.test, d.reason
            )));
        }
        let message = format!("test({id}): repair criterion test {}{TRAILER}", d.test);
        if !commit_files(&wt, &[d.test.as_str()], &message)
            .map_err(|e| Error::invalid(format!("The repair commit failed: {e}")))?
        {
            return Err(Error::invalid(
                "The repair commit failed: nothing to commit",
            ));
        }
        let head = self.head()?;
        self.update(|s| s.tests_commit = Some(head))?;
        let (ok, out) = self.in_sandbox(&test_command(config, std::slice::from_ref(&d.test)))?;
        if ok {
            return self
                .fail(
                    &format!(
                        "The repaired test {} passes on tests_commit; it must fail before the work.\n{out}",
                        d.test
                    ),
                    None,
                )
                .map(Err);
        }
        self.log(&format!(
            "QA repaired {}. It fails on the new tests_commit.",
            d.test
        ));
        Ok(Ok(format!(
            "QA corrected the disputed test {}. The dispute was: {}\n{last}",
            d.test, d.reason
        )))
    }

    /// The first failed check of the work, or `None` when each check passes.
    fn checks(&self) -> Result<Option<String>> {
        let config = &self.ctx.config;
        let wt = self.wt();
        let paths = test_paths(&self.story);
        let tests_commit = self
            .story
            .session
            .as_ref()
            .and_then(|s| s.tests_commit.clone());
        if let Some(tc) = tests_commit.filter(|c| !c.is_empty()) {
            // A repair commit holds one test only, so the check compares every criterion test path.
            let touched = lines(&git_out(&wt, &["diff", "--name-only", &tc, "HEAD"])?);
            let changed: Vec<&str> = paths
                .iter()
                .filter(|f| touched.contains(f))
                .map(String::as_str)
                .collect();
            if !changed.is_empty() {
                return Ok(Some(format!(
                    "The work changed files of tests_commit: {}. Restore them; the tests are fixed.",
                    changed.join(", ")
                )));
            }
        }
        if !paths.is_empty() {
            let (ok, out) = self.in_sandbox(&test_command(config, &paths))?;
            if !ok {
                return Ok(Some(format!("The criterion tests fail:\n{out}")));
            }
        }
        for gate in &config.gate {
            let (ok, out) = self.in_sandbox(gate)?;
            if !ok {
                return Ok(Some(format!("The gate command \"{gate}\" fails:\n{out}")));
            }
        }
        Ok(None)
    }
}

/// Runs a queued session to its end: worktree, QA tests, the check that they
/// fail, the owner's work, the checks, and one more attempt after a failed
/// check. Each step saves the session. A reject during the run stops it at
/// its next step.
pub fn execute(ctx: &RunCtx, id: &str) -> Result<Story> {
    let story = read_with(&ctx.store, |c| store::load(c, &ctx.product.id, id))?;
    let Some(session) = story.session.clone().filter(|s| s.state == "queued") else {
        return Err(Error::invalid(format!(
            "Story {id} has no queued story session."
        )));
    };
    let key = (ctx.product.id.clone(), id.to_string());
    let handle = {
        let mut map = running();
        if map.contains_key(&key) {
            return Err(Error::invalid(format!(
                "Story {id} has a story session that runs."
            )));
        }
        let handle = map.entry(key.clone()).or_default().clone();
        // Under the lock of the map, as in `stop_all`, so no session slips between them.
        if STOPPING.load(Ordering::SeqCst) {
            handle.cancelled.store(true, Ordering::SeqCst);
        }
        handle
    };
    let mut run = Run {
        ctx,
        id: id.into(),
        started: session.started_at,
        story,
        handle,
        mask: Mask::default(),
        graph: None,
        git_link: String::new(),
        log_error: None,
    };
    let result = match run.steps() {
        Ok(s) => Ok(s),
        Err(Error::Gone(m)) => {
            run.log(&m);
            read_with(&ctx.store, |c| store::load(c, &ctx.product.id, id))
        }
        Err(e) => match run.fail(&e.to_string(), None) {
            Err(Error::Gone(m)) => {
                run.log(&m);
                read_with(&ctx.store, |c| store::load(c, &ctx.product.id, id))
            }
            other => other,
        },
    };
    running().remove(&key);
    (ctx.notify)();
    match run.log_error {
        Some(e) => Err(Error::invalid(format!("The story log failed: {e}"))),
        None => result,
    }
}
