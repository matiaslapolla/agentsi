//! A core that stops must stop its running work: B3 of the U22 review. These tests
//! run the real `agentsi-core` program, with a fake `claude` on its `PATH` that never
//! ends and that leaves a grandchild in its process group.

mod common;

use agentsi_agency::registry::Product;
use agentsi_agency::sandbox;
use agentsi_agency::story::{lifecycle, store};
use agentsi_proto::client::EXIT_ALREADY_RUNNING;
use agentsi_proto::story_api::{Criterion, Story, StoryConfig};
use agentsi_store::Store;
use common::{CoreProc, TempDir, alive};
use serde_json::json;
use std::io::{BufRead, BufReader, Write};
use std::os::unix::fs::PermissionsExt;
use std::os::unix::net::UnixStream;
use std::path::{Path, PathBuf};
use std::process::{Command, ExitStatus};
use std::time::{Duration, Instant};

/// Longer than a core needs, and far shorter than the fake `claude` runs.
const EXIT_LIMIT: Duration = Duration::from_secs(8);
const CLAUDE_SECONDS: u32 = 60;

struct World {
    dir: TempDir,
    root: PathBuf,
    repo: PathBuf,
    pids: PathBuf,
    path: String,
}

impl World {
    fn new() -> World {
        let dir = TempDir::new();
        let (root, repo, bin) = (dir.0.join("root"), dir.0.join("repo"), dir.0.join("bin"));
        std::fs::create_dir_all(root.join("plugin/agents")).unwrap();
        std::fs::create_dir_all(repo.join("test")).unwrap();
        std::fs::create_dir_all(&bin).unwrap();
        for role in ["pm", "dev", "qa"] {
            std::fs::write(
                root.join(format!("plugin/agents/{role}.md")),
                format!("---\nmodel: haiku\n---\nYou are {role}.\n"),
            )
            .unwrap();
        }
        git(&repo, &["init", "-q", "-b", "main"]);
        git(&repo, &["config", "user.name", "Test"]);
        git(&repo, &["config", "user.email", "test@example.invalid"]);
        git(&repo, &["config", "commit.gpgsign", "false"]);
        std::fs::write(repo.join(".gitignore"), ".worktrees/\n").unwrap();
        git(&repo, &["add", "-A"]);
        git(&repo, &["commit", "-q", "-m", "init"]);
        let pids = dir.0.join("pids");
        script(
            &bin.join("claude"),
            &format!(
                "sleep {CLAUDE_SECONDS} &\necho $! >> {0}\necho $$ >> {0}\nwait\n",
                pids.display()
            ),
        );
        let path = format!(
            "{}:{}",
            bin.display(),
            std::env::var("PATH").unwrap_or_default()
        );
        std::fs::write(
            root.join("agentsi.config.json"),
            json!({
                "owner": {"name": "Ada", "language": "English"},
                "products": [{
                    "id": "p", "name": "P", "paths": [repo], "status": "active", "roles": ["dev", "qa"],
                    "stories": {"repo": repo, "test": "sh {files}", "gate": ["test -f done"], "budget_usd": 1.0},
                }],
            })
            .to_string(),
        )
        .unwrap();
        let store = Store::open(&root.join("agentsi.db")).unwrap();
        store
            .write(|tx| {
                tx.execute(
                    "insert into snapshots (product, minutes_id, body) values ('p', 'old-1', '# Snapshot\nfrozen')",
                    [],
                )?;
                Ok(())
            })
            .unwrap();
        World {
            dir,
            root,
            repo,
            pids,
            path,
        }
    }

    /// A fake reindex that takes `seconds`. It holds a meeting where no cancel reaches it.
    fn slow_index(&self, seconds: u32) -> PathBuf {
        let started = self.dir.0.join("index-started");
        script(
            &self.dir.0.join("bin/codebase-memory-mcp"),
            &format!("touch {}\nsleep {seconds}\n", started.display()),
        );
        started
    }

    /// A fake `git` that takes `seconds`, with a child of its own, as a slow `git log` does.
    /// It writes its pids to the file of the fake `claude`.
    fn slow_git(&self, seconds: u32) {
        script(
            &self.dir.0.join("bin/git"),
            &format!(
                "sleep {seconds} &\necho $! >> {0}\necho $$ >> {0}\nwait\n",
                self.pids.display()
            ),
        );
    }

    fn core(&self) -> CoreProc {
        let home = self.dir.0.join("home");
        std::fs::create_dir_all(&home).unwrap();
        CoreProc::start_with(
            &self.dir,
            &[
                ("AGENTSI_HOME", &self.root),
                ("PATH", Path::new(&self.path)),
                ("HOME", &home),
            ],
        )
    }

    fn product(&self) -> Product {
        Product {
            id: "p".into(),
            name: "P".into(),
            paths: vec![self.repo.to_string_lossy().into()],
            status: "active".into(),
            roles: vec!["dev".into(), "qa".into()],
        }
    }

    fn config(&self) -> StoryConfig {
        StoryConfig {
            repo: self.repo.to_string_lossy().into(),
            test: "sh {files}".into(),
            gate: vec!["test -f done".into()],
            budget_usd: 1.0,
        }
    }

    fn store(&self) -> Store {
        Store::open(&self.root.join("agentsi.db")).unwrap()
    }

    fn count(&self, table: &str) -> i64 {
        self.store()
            .read(|c| Ok(c.query_row(&format!("select count(*) from {table}"), [], |r| r.get(0))?))
            .unwrap()
    }

    fn fake_pids(&self) -> Vec<i32> {
        std::fs::read_to_string(&self.pids)
            .unwrap_or_default()
            .lines()
            .filter_map(|l| l.trim().parse().ok())
            .collect()
    }

    /// The fake `claude` and its grandchild run.
    fn wait_for_claude(&self) {
        wait_for("claude runs", Duration::from_secs(20), || {
            self.fake_pids().len() >= 2
        });
    }

    fn assert_no_claude_alive(&self) {
        let pids = self.fake_pids();
        assert!(pids.len() >= 2, "{pids:?}");
        // A killed child stays a zombie for an instant, until its parent reaps it.
        wait_for("each claude process ends", Duration::from_secs(2), || {
            pids.iter().all(|p| !runs(*p))
        });
    }
}

impl Drop for World {
    /// Without the fix a test fails with its processes alive. They must not outlive it.
    fn drop(&mut self) {
        for pid in self.fake_pids() {
            // SAFETY: a pid that the fake `claude` of this test wrote.
            unsafe { libc::kill(pid, libc::SIGKILL) };
        }
    }
}

fn git(dir: &Path, args: &[&str]) {
    let ok = Command::new("git")
        .args(args)
        .current_dir(dir)
        .env("GIT_CONFIG_GLOBAL", "/dev/null")
        .status()
        .unwrap()
        .success();
    assert!(ok, "git {args:?}");
}

fn script(path: &Path, body: &str) {
    std::fs::write(path, format!("#!/bin/sh\n{body}")).unwrap();
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o755)).unwrap();
}

fn wait_for(what: &str, limit: Duration, mut ok: impl FnMut() -> bool) {
    let deadline = Instant::now() + limit;
    while !ok() {
        assert!(Instant::now() < deadline, "timed out: {what}");
        std::thread::sleep(Duration::from_millis(20));
    }
}

/// True for a process that runs. A zombie has ended, and only waits for its parent.
fn runs(pid: i32) -> bool {
    let stat = std::fs::read_to_string(format!("/proc/{pid}/stat")).unwrap_or_default();
    let state = stat.rsplit(") ").next().and_then(|s| s.chars().next());
    alive(pid) && !matches!(state, Some('Z') | None)
}

/// Sends one request and keeps the connection open, as a client that waits does.
fn request(core: &CoreProc, method: &str, params: serde_json::Value) -> UnixStream {
    let mut stream = UnixStream::connect(&core.socket.path).unwrap();
    let line = json!({"jsonrpc": "2.0", "id": 2, "method": method, "params": params});
    writeln!(stream, "{line}").unwrap();
    stream
}

fn meeting(core: &CoreProc, frozen: Option<&str>) -> UnixStream {
    let mut params = json!({"product": "p", "question": "What next?", "roles": ["dev"]});
    if let Some(id) = frozen {
        params["frozen"] = json!(id);
    }
    request(core, "meeting.start", params)
}

fn sigterm(core: &CoreProc) {
    // SAFETY: the pid of the core that this test started.
    unsafe { libc::kill(core.pid(), libc::SIGTERM) };
}

fn shutdown(core: &CoreProc) {
    let mut stream = UnixStream::connect(&core.socket.path).unwrap();
    writeln!(stream, "{}", common::SHUTDOWN).unwrap();
}

fn wait_exit(core: &mut CoreProc, limit: Duration) -> ExitStatus {
    let child = core.child.as_mut().unwrap();
    let deadline = Instant::now() + limit;
    loop {
        if let Some(status) = child.try_wait().unwrap() {
            return status;
        }
        assert!(
            Instant::now() < deadline,
            "the core still runs after {limit:?}"
        );
        std::thread::sleep(Duration::from_millis(20));
    }
}

fn meeting_stops_with_the_core(stop: fn(&CoreProc)) {
    let world = World::new();
    let mut core = world.core();
    let _client = meeting(&core, Some("old-1"));
    world.wait_for_claude();
    stop(&core);
    assert!(wait_exit(&mut core, EXIT_LIMIT).success());
    world.assert_no_claude_alive();
    assert_eq!(world.count("minutes"), 0);
    assert_eq!(world.count("meeting_log"), 0);
}

#[test]
fn sigterm_stops_a_meeting_and_writes_no_minutes() {
    meeting_stops_with_the_core(sigterm);
}

#[test]
fn core_shutdown_stops_a_meeting_and_writes_no_minutes() {
    meeting_stops_with_the_core(shutdown);
}

fn story() -> Story {
    let criterion = |t: &str| Criterion {
        given: "g".into(),
        when: "w".into(),
        then: "t".into(),
        test: Some(t.into()),
        checks: "c".into(),
    };
    Story {
        id: "S001".into(),
        kind: "story".into(),
        slug: "make-it-done".into(),
        title: "Make it done".into(),
        as_a: Some("user".into()),
        i_want: "it done".into(),
        so_that: "it is done".into(),
        criteria: vec![criterion("test/a.sh"), criterion("test/b.sh")],
        owner_role: "dev".into(),
        objective: None,
        sources: vec![],
        status: "not_implemented".into(),
        approval: "approved".into(),
        changes_note: None,
        proposed_by: "owner".into(),
        minutes: None,
        session: None,
        results: vec![],
        created_at: "2026-10-01T10:00:00.000Z".into(),
        updated_at: "2026-10-01T10:00:00.000Z".into(),
        archived: None,
    }
}

fn story_stops_with_the_core(stop: fn(&CoreProc)) {
    if !sandbox::works() {
        eprintln!("skipped: bubblewrap cannot start a sandbox here");
        return;
    }
    let world = World::new();
    store::save(&world.store(), &world.product(), story()).unwrap();
    let mut core = world.core();
    let client = request(&core, "story.run", json!({"product": "p", "id": "S001"}));
    client
        .set_read_timeout(Some(Duration::from_secs(10)))
        .unwrap();
    let mut reply = String::new();
    BufReader::new(&client).read_line(&mut reply).unwrap();
    assert!(reply.contains(r#""result""#), "{reply}");
    world.wait_for_claude();
    stop(&core);
    assert!(wait_exit(&mut core, EXIT_LIMIT).success());
    world.assert_no_claude_alive();

    // As after `reject --force`, the session records no failure. No reject follows
    // here, so it keeps its last state, and a plain reject then cleans it up.
    let db = world.store();
    let saved = store::read_with(&db, |c| store::load(c, "p", "S001")).unwrap();
    let session = saved.session.unwrap();
    assert_eq!(session.failure, None);
    assert_ne!(session.state, "failed");
    let log = store::read_with(&db, |c| store::read_log(c, "p", "S001")).unwrap();
    assert!(log.contains("stopped with the core"), "{log}");
    let rejected = lifecycle::reject(&db, &world.product(), "S001", false, &world.config());
    assert!(rejected.unwrap().session.is_none());
}

#[test]
fn sigterm_stops_a_story_session_and_records_no_failure() {
    story_stops_with_the_core(sigterm);
}

#[test]
fn core_shutdown_stops_a_story_session_and_records_no_failure() {
    story_stops_with_the_core(shutdown);
}

#[test]
fn no_second_core_binds_while_the_first_still_stops() {
    let world = World::new();
    let started = world.slow_index(4);
    let mut core = world.core();
    let _client = meeting(&core, None);
    wait_for("the index runs", Duration::from_secs(5), || {
        started.exists()
    });
    sigterm(&core);
    wait_for("the socket goes", Duration::from_secs(2), || {
        !core.socket.path.exists()
    });
    // A core that binds never ends by itself, so this waits with a limit.
    let mut second = CoreProc::command(&core.socket).spawn().unwrap();
    let deadline = Instant::now() + Duration::from_secs(2);
    let code = loop {
        if let Some(status) = second.try_wait().unwrap() {
            break status.code();
        }
        if Instant::now() >= deadline {
            second.kill().unwrap();
            second.wait().unwrap();
            panic!("a second core bound while the first still stops");
        }
        std::thread::sleep(Duration::from_millis(20));
    };
    assert_eq!(code, Some(EXIT_ALREADY_RUNNING));
    assert!(wait_exit(&mut core, EXIT_LIMIT).success());
    assert_eq!(world.count("minutes"), 0);
}

#[test]
fn a_core_exits_when_its_work_outlives_the_wait() {
    let world = World::new();
    let started = world.slow_index(CLAUDE_SECONDS);
    let mut core = world.core();
    let _client = meeting(&core, None);
    wait_for("the index runs", Duration::from_secs(5), || {
        started.exists()
    });
    let stopped_at = Instant::now();
    sigterm(&core);
    let status = wait_exit(&mut core, Duration::from_secs(14));
    assert_eq!(status.code(), Some(1));
    assert!(stopped_at.elapsed() >= Duration::from_secs(9));
    assert_eq!(world.count("minutes"), 0);
}

/// Reads until the core closes the link. `None` after `limit` means the link stayed open.
fn closes_within(stream: &UnixStream, limit: Duration) -> Option<Duration> {
    use std::io::Read;
    let started = Instant::now();
    stream
        .set_read_timeout(Some(Duration::from_millis(50)))
        .unwrap();
    let mut sink = [0u8; 4096];
    let mut reader = stream;
    while started.elapsed() < limit {
        match reader.read(&mut sink) {
            Ok(0) => return Some(started.elapsed()),
            Ok(_) => {}
            Err(e)
                if matches!(
                    e.kind(),
                    std::io::ErrorKind::WouldBlock | std::io::ErrorKind::TimedOut
                ) => {}
            Err(_) => return Some(started.elapsed()),
        }
    }
    None
}

#[test]
fn a_stopping_core_closes_its_links_at_once_and_still_waits_for_its_work() {
    let world = World::new();
    let started = world.slow_index(5);
    let mut core = world.core();
    let client = meeting(&core, None);
    let watcher = UnixStream::connect(&core.socket.path).unwrap();
    wait_for("the index runs", Duration::from_secs(5), || {
        started.exists()
    });
    sigterm(&core);
    let two_seconds = Duration::from_secs(2);
    let idle = closes_within(&watcher, two_seconds);
    let busy = closes_within(&client, two_seconds);
    eprintln!("links closed after: idle {idle:?}, request in flight {busy:?}");
    assert!(
        idle.is_some(),
        "an idle link stayed open while the work ended"
    );
    assert!(
        busy.is_some(),
        "the link of a running request stayed open while the work ended"
    );
    assert!(
        core.child.as_mut().unwrap().try_wait().unwrap().is_none(),
        "the core must still wait for its work"
    );
    assert!(wait_exit(&mut core, EXIT_LIMIT).success());
    assert_eq!(world.count("minutes"), 0);
    assert_eq!(world.count("meeting_log"), 0);
}

#[test]
fn a_client_of_a_stopping_core_gets_a_clear_error_and_does_not_hang() {
    use agentsi_proto::client::Client;
    use agentsi_proto::meeting_api::{MeetingStart, MeetingStartParams};
    let world = World::new();
    let started = world.slow_index(5);
    let mut core = world.core();
    let stream = UnixStream::connect(&core.socket.path).unwrap();
    let mut client = Client::new(stream).unwrap();
    let asked = Instant::now();
    let path = core.pid();
    let waiter = std::thread::spawn(move || {
        client.call::<MeetingStart>(&MeetingStartParams {
            product: "p".into(),
            question: "What next?".into(),
            roles: Some(vec!["dev".into()]),
            ..Default::default()
        })
    });
    wait_for("the index runs", Duration::from_secs(5), || {
        started.exists()
    });
    // SAFETY: the pid of the core that this test started.
    unsafe { libc::kill(path, libc::SIGTERM) };
    let error = waiter.join().unwrap().expect_err("the request is cut off");
    assert!(
        error.to_string().contains("core closed the connection"),
        "{error}"
    );
    assert!(asked.elapsed() < Duration::from_secs(5), "the client hung");
    assert!(wait_exit(&mut core, EXIT_LIMIT).success());
}

#[test]
fn core_shutdown_answers_its_caller_before_the_links_close() {
    let world = World::new();
    let mut core = world.core();
    let mut stream = UnixStream::connect(&core.socket.path).unwrap();
    writeln!(stream, "{}", common::SHUTDOWN).unwrap();
    stream
        .set_read_timeout(Some(Duration::from_secs(5)))
        .unwrap();
    let mut line = String::new();
    BufReader::new(&stream).read_line(&mut line).unwrap();
    assert!(line.contains(r#""result""#), "{line}");
    assert!(closes_within(&stream, Duration::from_secs(2)).is_some());
    assert!(wait_exit(&mut core, EXIT_LIMIT).success());
}

/// B1 of the U27 review: a request that no meeting or story session tracks (here
/// `story.new`) still gets its answer within the 1 s grace.
#[test]
fn a_request_in_flight_gets_its_answer_when_the_core_stops() {
    let world = World::new();
    let started = world.dir.0.join("claude-started");
    script(
        &world.dir.0.join("bin/claude"),
        &format!("touch {}\nsleep 0.4\necho '{{}}'\n", started.display()),
    );
    let mut core = world.core();
    let client = request(
        &core,
        "story.new",
        json!({"product": "p", "text": "Make it done"}),
    );
    wait_for("claude runs", Duration::from_secs(10), || started.exists());
    sigterm(&core);
    client
        .set_read_timeout(Some(Duration::from_secs(5)))
        .unwrap();
    let mut reply = String::new();
    let read = BufReader::new(&client).read_line(&mut reply);
    assert!(
        read.is_ok_and(|n| n > 0) && reply.contains(r#""id":2"#),
        "the link closed with no answer: {reply:?}"
    );
    assert!(wait_exit(&mut core, EXIT_LIMIT).success());
}

/// B2 of the U27 review: the lock stays until a request that no cancel reaches has ended.
#[test]
fn no_second_core_binds_while_an_index_still_runs_after_a_stop() {
    let world = World::new();
    world.slow_git(4);
    let mut core = world.core();
    let _client = request(&core, "knowledge.index", json!({"product": "p"}));
    world.wait_for_claude();
    sigterm(&core);
    wait_for("the socket goes", Duration::from_secs(2), || {
        !core.socket.path.exists()
    });
    let mut second = CoreProc::command(&core.socket).spawn().unwrap();
    let deadline = Instant::now() + Duration::from_secs(2);
    let code = loop {
        if let Some(status) = second.try_wait().unwrap() {
            break status.code();
        }
        if Instant::now() >= deadline {
            second.kill().unwrap();
            second.wait().unwrap();
            panic!("a second core bound while the first still wrote");
        }
        std::thread::sleep(Duration::from_millis(20));
    };
    assert_eq!(code, Some(EXIT_ALREADY_RUNNING));
    assert!(wait_exit(&mut core, EXIT_LIMIT).success());
}

/// B2 of the U27 review: at the end of the wait, no process of the core outlives its lock.
#[test]
fn a_core_that_outlives_its_stop_wait_leaves_no_child_process() {
    let world = World::new();
    world.slow_git(CLAUDE_SECONDS);
    let mut core = world.core();
    let _client = request(&core, "knowledge.index", json!({"product": "p"}));
    world.wait_for_claude();
    let stopped_at = Instant::now();
    sigterm(&core);
    let status = wait_exit(&mut core, Duration::from_secs(14));
    assert_eq!(status.code(), Some(1));
    assert!(stopped_at.elapsed() >= Duration::from_secs(9));
    world.assert_no_claude_alive();
}

#[test]
fn a_core_that_outlives_its_stop_wait_leaves_the_panes_to_the_hangup() {
    let world = World::new();
    world.slow_git(CLAUDE_SECONDS);
    let mut core = world.core();
    let pid_file = world.dir.0.join("pane.pid");
    let pane = format!(
        "trap '' HUP; echo $$ > {}; exec sleep 30",
        pid_file.display()
    );
    let _pane = request(
        &core,
        "terminal.create",
        json!({"cwd": std::env::temp_dir(), "argv": ["sh", "-c", pane]}),
    );
    wait_for("the pane writes its pid", Duration::from_secs(5), || {
        std::fs::read_to_string(&pid_file).is_ok_and(|p| p.ends_with('\n'))
    });
    let pid: i32 = std::fs::read_to_string(&pid_file)
        .unwrap()
        .trim()
        .parse()
        .unwrap();
    let _client = request(&core, "knowledge.index", json!({"product": "p"}));
    world.wait_for_claude();
    sigterm(&core);
    let status = wait_exit(&mut core, Duration::from_secs(14));
    let survived = runs(pid);
    // SAFETY: the pid of the pane that this test started.
    unsafe { libc::kill(pid, libc::SIGKILL) };
    assert_eq!(status.code(), Some(1));
    assert!(survived, "the core killed the program of a pane");
}
