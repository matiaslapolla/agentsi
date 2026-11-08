//! `agentsi up`, `migrate` and `doctor` against a real core on a throwaway
//! data root. The TypeScript commands ask Postgres and Docker, so no
//! TypeScript output exists for them. The expected text is in this file.
//!
//! Machine facts are normalized by one rule: the work folder is `<WORK>`, the
//! version of the core is `<VERSION>` and the size of the model catalog is
//! `<N>`. The tools come from fake programs in `bin/` of the work folder, and
//! `PATH` holds only that folder.

use agentsi_proto::client::Client;
use agentsi_proto::core_api::{Empty, Shutdown};
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::process::Command;

/// What `voice on` says on stderr while no pill runs.
const NO_PILL: &str = "Warning: agentsi-pill is not running, so nothing is spoken and the dictation key does nothing. Start agentsi-pill.\n";

struct Work {
    dir: tempfile::TempDir,
    socket: PathBuf,
    core: PathBuf,
}

impl Drop for Work {
    fn drop(&mut self) {
        if let Ok(mut c) = Client::connect(&self.socket) {
            let _ = c.call::<Shutdown>(&Empty {});
        }
    }
}

fn core_program() -> PathBuf {
    let manifest = PathBuf::from(std::env::var("CARGO_MANIFEST_DIR").expect("run with cargo"));
    // The build folder may hold a core of another checkout: mark the sources as changed.
    let main = manifest.join("../agentsi-core/src/main.rs");
    let file = std::fs::OpenOptions::new().write(true).open(&main).unwrap();
    file.set_modified(std::time::SystemTime::now()).unwrap();
    let cargo = std::env::var("CARGO").unwrap_or_else(|_| "cargo".into());
    let status = Command::new(cargo)
        .args(["build", "-q", "-p", "agentsi-core", "--bins"])
        .status()
        .expect("cargo runs");
    assert!(status.success(), "cargo build -p agentsi-core");
    Path::new(env!("CARGO_BIN_EXE_agentsi"))
        .parent()
        .unwrap()
        .join("agentsi-core")
}

fn fake(dir: &Path, name: &str, script: &str) {
    let path = dir.join(name);
    std::fs::write(&path, format!("#!/bin/sh\n{script}\n")).unwrap();
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).unwrap();
}

fn work(with_database: bool) -> Work {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().join("root");
    std::fs::create_dir_all(&root).unwrap();
    if with_database {
        let mut conn =
            agentsi_store::db::open_writer(&agentsi_store::paths::db_path(&root)).unwrap();
        agentsi_store::db::migrate(&mut conn).unwrap();
    }
    let bin = dir.path().join("bin");
    std::fs::create_dir_all(&bin).unwrap();
    fake(
        &bin,
        "claude",
        "case \"$1\" in --version) echo '2.1.0 (Claude Code)';; --help) echo '--safe-mode --restricted --permission-prompts --max-budget-usd --strict-mcp-config --disable-slash-commands';; esac",
    );
    fake(&bin, "git", "echo 'git version 2.50.0'");
    fake(&bin, "gh", "echo 'gh version 2.80.0'");
    fake(&bin, "espeak-ng", "echo 'eSpeak NG text-to-speech: 1.52'");
    fake(
        &bin,
        "pactl",
        "echo '50\talsa_output.pci\tPipeWire\ts32le 2ch 48000Hz\tSUSPENDED'",
    );
    Work {
        socket: dir.path().join("run/core.sock"),
        core: core_program(),
        dir,
    }
}

fn agentsi(w: &Work, args: &[&str], path: &Path) -> (i32, String, String) {
    agentsi_env(w, args, path, &[])
}

fn agentsi_env(
    w: &Work,
    args: &[&str],
    path: &Path,
    extra: &[(&str, &str)],
) -> (i32, String, String) {
    let out = Command::new(env!("CARGO_BIN_EXE_agentsi"))
        .args(args)
        .current_dir(w.dir.path())
        .env_clear()
        .env("PATH", path)
        .env("HOME", w.dir.path())
        .env("AGENTSI_HOME", w.dir.path().join("root"))
        .env("XDG_DATA_HOME", w.dir.path().join("data"))
        .env("AGENTSI_CORE_SOCKET", &w.socket)
        .env("AGENTSI_CORE_BIN", &w.core)
        .envs(extra.iter().copied())
        .output()
        .expect("agentsi runs");
    (
        out.status.code().unwrap_or(-1),
        normalize(&String::from_utf8_lossy(&out.stdout), w),
        normalize(&String::from_utf8_lossy(&out.stderr), w),
    )
}

fn normalize(text: &str, w: &Work) -> String {
    let re = regex::Regex::new(r"agentsi-core \d+\.\d+\.\d+(-[0-9A-Za-z.]+)?").unwrap();
    let text = text.replace(w.dir.path().to_str().unwrap(), "<WORK>");
    let text = re.replace_all(&text, "agentsi-core <VERSION>");
    regex::Regex::new(r"of \d+ downloaded")
        .unwrap()
        .replace_all(&text, "of <N> downloaded")
        .into_owned()
}

#[test]
fn migrate_prints_the_schema_version_that_the_core_applied() {
    let w = work(true);
    let path = w.dir.path().join("bin");
    let (code, out, err) = agentsi(&w, &["migrate"], &path);
    assert_eq!(
        (code, out.as_str(), err.as_str()),
        (0, "Database up to date, at schema version 1.\n", "")
    );
}

#[test]
fn migrate_on_a_root_with_no_database_says_so() {
    let w = work(false);
    let path = w.dir.path().join("bin");
    let (code, out, _) = agentsi(&w, &["migrate"], &path);
    assert_eq!(code, 0);
    assert!(out.starts_with("No database yet."), "{out}");
}

#[test]
fn up_starts_the_core_and_prints_where_it_runs() {
    let w = work(true);
    let path = w.dir.path().join("bin");
    let (code, out, err) = agentsi(&w, &["up"], &path);
    assert_eq!(
        (code, out.as_str(), err.as_str()),
        (
            0,
            "Core running: agentsi-core <VERSION>.\nData root: <WORK>/root\nDatabase up to date, at schema version 1.\n",
            ""
        )
    );
    // A second run finds the same core.
    assert_eq!(agentsi(&w, &["up"], &path).0, 0);
}

#[test]
fn doctor_reports_each_check_and_passes_when_the_required_ones_do() {
    let w = work(true);
    let path = w.dir.path().join("bin");
    let (code, out, err) = agentsi(&w, &["doctor"], &path);
    assert_eq!(err, "");
    assert_eq!(code, 0, "{out}");
    let expected = "\
ok       core                            agentsi-core <VERSION>
ok       data root                       <WORK>/root, schema 1
ok       claude                          2.1.0 (Claude Code)
ok       git                             git version 2.50.0
missing  bwrap (stories)                 Not found. It blocks story sessions (agentsi story run).
ok       gh (optional)                   gh version 2.80.0
missing  codebase-memory-mcp (optional)  Not found. It blocks the code graph of meetings and story sessions.
ok       espeak-ng (optional)            eSpeak NG text-to-speech: 1.52
fail     voice models (optional)         0 of <N> downloaded. Download them in the app. It blocks voice mode and calls.
ok       audio devices (optional)        1 output(s) (pactl)

Every required check passes.
";
    assert_eq!(out, expected);
}

#[test]
fn doctor_exits_with_1_when_claude_is_missing() {
    let w = work(true);
    std::fs::remove_file(w.dir.path().join("bin/claude")).unwrap();
    let path = w.dir.path().join("bin");
    let (code, out, _) = agentsi(&w, &["doctor"], &path);
    assert_eq!(code, 1);
    assert!(out.contains("missing  claude"), "{out}");
    assert!(
        out.ends_with("\nA required check failed. AgentSI cannot run until you fix it.\n"),
        "{out}"
    );
}

#[test]
fn the_three_commands_reject_an_option() {
    let w = work(false);
    let path = w.dir.path().join("bin");
    for name in ["up", "migrate", "doctor"] {
        let (code, _, err) = agentsi(&w, &[name, "--json"], &path);
        assert_eq!(code, 1, "{name}");
        assert!(err.starts_with(&format!("agentsi {name}: ")), "{err}");
    }
}

// ---- backup, voice and web (U26b). TypeScript ran `pg_dump` and a web server, so no
// golden case exists for them. The expected text is in this file.

use rusqlite::Connection;

fn db_file(w: &Work) -> PathBuf {
    w.dir.path().join("root/agentsi.db")
}

/// The name and row count of each table of a database, to compare a copy with its source.
fn table_counts(conn: &Connection) -> Vec<(String, i64)> {
    let names: Vec<String> = conn
        .prepare("select name from sqlite_master where type = 'table' and name not like 'sqlite_%' order by name")
        .unwrap()
        .query_map([], |r| r.get(0))
        .unwrap()
        .map(Result::unwrap)
        .collect();
    names
        .into_iter()
        .map(|n| {
            let c = conn
                .query_row(&format!("select count(*) from \"{n}\""), [], |r| r.get(0))
                .unwrap();
            (n, c)
        })
        .collect()
}

fn integrity(conn: &Connection) -> String {
    conn.query_row("pragma integrity_check", [], |r| r.get(0))
        .unwrap()
}

fn backup_file(w: &Work, out: &str) -> PathBuf {
    let line = out.lines().next().unwrap();
    let path = line.strip_prefix("Backup: ").expect(line);
    PathBuf::from(path.replace("<WORK>", w.dir.path().to_str().unwrap()))
}

#[test]
fn backup_writes_a_database_that_restores_to_the_same_rows() {
    let w = work(true);
    let path = w.dir.path().join("bin");
    let source = Connection::open(db_file(&w)).unwrap();
    source
        .execute_batch("create table probe(n integer primary key, text text);")
        .unwrap();
    for n in 1..=50 {
        source
            .execute(
                "insert into probe(n, text) values (?1, ?2)",
                (n, format!("row {n}")),
            )
            .unwrap();
    }
    std::fs::write(
        w.dir.path().join("root/agentsi.config.json"),
        "{\"products\":[]}\n",
    )
    .unwrap();

    let (code, out, err) = agentsi(&w, &["backup"], &path);
    assert_eq!((code, err.as_str()), (0, ""), "{out}");
    let file = backup_file(&w, &out);
    assert!(
        file.starts_with(w.dir.path().join("root/backups")),
        "{}",
        file.display()
    );
    let name = file.file_name().unwrap().to_str().unwrap();
    assert!(
        regex::Regex::new(r"^agentsi-\d{4}-\d{2}-\d{2}\.db$")
            .unwrap()
            .is_match(name),
        "{name}"
    );
    let registry = out
        .lines()
        .find_map(|l| l.strip_prefix("Registry: "))
        .expect("a registry line")
        .replace("<WORK>", w.dir.path().to_str().unwrap());
    assert_eq!(
        std::fs::read_to_string(registry).unwrap(),
        "{\"products\":[]}\n"
    );

    // The drill of the data spec: open the file and compare the rows.
    let copy = Connection::open(&file).unwrap();
    assert_eq!(integrity(&copy), "ok");
    assert_eq!(table_counts(&copy), table_counts(&source));
    let sample: String = copy
        .query_row("select text from probe where n = 37", [], |r| r.get(0))
        .unwrap();
    assert_eq!(sample, "row 37");
    assert_eq!(out.lines().count(), 2, "{out}");
}

#[test]
fn backup_while_the_database_is_written_is_a_complete_copy() {
    let w = work(true);
    let path = w.dir.path().join("bin");
    {
        let conn = Connection::open(db_file(&w)).unwrap();
        conn.execute_batch("create table probe(n integer primary key);")
            .unwrap();
    }
    let db = db_file(&w);
    let stop = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
    let writer = {
        let stop = stop.clone();
        std::thread::spawn(move || {
            let conn = agentsi_store::db::open_writer(&db).unwrap();
            let mut n = 0;
            while !stop.load(std::sync::atomic::Ordering::Relaxed) {
                // One row for each transaction, so a torn copy shows as a gap.
                n += 1;
                conn.execute("insert into probe(n) values (?1)", [n])
                    .unwrap();
            }
            n
        })
    };
    for _ in 0..4 {
        let (code, out, err) = agentsi(&w, &["backup"], &path);
        assert_eq!((code, err.as_str()), (0, ""), "{out}");
        let copy = Connection::open(backup_file(&w, &out)).unwrap();
        assert_eq!(integrity(&copy), "ok");
        let (count, max): (i64, i64) = copy
            .query_row("select count(*), coalesce(max(n), 0) from probe", [], |r| {
                Ok((r.get(0)?, r.get(1)?))
            })
            .unwrap();
        assert_eq!(count, max, "the copy holds a gap");
        std::thread::sleep(std::time::Duration::from_millis(20));
    }
    stop.store(true, std::sync::atomic::Ordering::Relaxed);
    let written = writer.join().unwrap();
    assert!(written > 0, "the writer never ran");
}

#[test]
fn backup_removes_the_backups_over_keep_and_lists_them() {
    let w = work(true);
    let path = w.dir.path().join("bin");
    let dir = w.dir.path().join("root/backups");
    std::fs::create_dir_all(&dir).unwrap();
    for date in ["2020-01-01", "2020-01-02"] {
        std::fs::write(dir.join(format!("agentsi-{date}.db")), "old").unwrap();
    }
    let (code, out, err) = agentsi(&w, &["backup", "--keep", "1"], &path);
    assert_eq!((code, err.as_str()), (0, ""), "{out}");
    let removed = out.lines().last().unwrap();
    assert_eq!(
        removed,
        "Removed: <WORK>/root/backups/agentsi-2020-01-01.db, <WORK>/root/backups/agentsi-2020-01-02.db"
    );
    assert!(!dir.join("agentsi-2020-01-01.db").exists());
    assert_eq!(std::fs::read_dir(&dir).unwrap().count(), 1);
}

#[test]
fn backup_refuses_a_bad_keep_a_stray_word_and_a_missing_database() {
    let w = work(false);
    let path = w.dir.path().join("bin");
    for keep in ["0", "x", "1.5"] {
        let (code, _, err) = agentsi(&w, &["backup", "--keep", keep], &path);
        assert_eq!(code, 1, "{keep}");
        assert!(
            err.starts_with("agentsi backup: --keep must be a whole number of 1 or more"),
            "{err}"
        );
    }
    let (code, _, err) = agentsi(&w, &["backup", "now"], &path);
    assert_eq!(code, 1);
    assert!(err.contains("Usage: agentsi backup [--keep <n>]"), "{err}");
    let (code, out, err) = agentsi(&w, &["backup"], &path);
    assert_eq!((code, out.as_str()), (1, ""));
    assert_eq!(
        err,
        "agentsi backup: <WORK>/root/agentsi.db does not exist\n"
    );
}

#[test]
fn voice_in_a_pane_turns_the_mode_of_that_pane_on_and_off() {
    use agentsi_proto::terminal_api::{Create, CreateParams, Kill, TerminalId};
    let w = work(true);
    let path = w.dir.path().join("bin");
    let (code, _, _) = agentsi(&w, &["up"], &path);
    assert_eq!(code, 0);
    let mut client = Client::connect(&w.socket).unwrap();
    let pane = client
        .call::<Create>(&CreateParams {
            cwd: w.dir.path().display().to_string(),
            argv: vec!["/bin/sleep".into(), "30".into()],
            env: Default::default(),
            product: None,
            cols: None,
            rows: None,
        })
        .unwrap();
    let env = [("AGENTSI_TERMINAL", pane.id.as_str())];
    let say = |action: &str| agentsi_env(&w, &["voice", action], &path, &env);
    assert_eq!(
        say("status"),
        (
            0,
            "Voice mode is off for this pane.\n".into(),
            String::new()
        )
    );
    assert_eq!(
        say("on"),
        (
            0,
            "Voice mode is on for this pane.\nTranscripts: <WORK>/root/voice/transcripts\n".into(),
            NO_PILL.into()
        )
    );
    assert_eq!(say("status").1, "Voice mode is on for this pane.\n");
    assert_eq!(say("off").1, "Voice mode is off for this pane.\n");
    // A pane that the core does not know is an error of the core.
    let (code, _, err) = agentsi_env(&w, &["voice", "on"], &path, &[("AGENTSI_TERMINAL", "t999")]);
    assert_eq!(code, 1);
    assert!(err.starts_with("agentsi voice: "), "{err}");
    let _ = client.call::<Kill>(&TerminalId { id: pane.id });
}

#[test]
fn voice_in_a_plain_claude_code_session_turns_the_mode_of_that_session_on_and_off() {
    let w = work(false);
    let path = w.dir.path().join("bin");
    let say = |action: &str, id: &str| {
        agentsi_env(
            &w,
            &["voice", action],
            &path,
            &[("CLAUDE_CODE_SESSION_ID", id)],
        )
    };
    assert_eq!(
        say("status", "s-1"),
        (
            0,
            "Voice mode is off for this session.\n".into(),
            String::new()
        )
    );
    assert_eq!(
        say("on", "s-1"),
        (
            0,
            "Voice mode is on for this session.\nTranscripts: <WORK>/root/voice/transcripts\n"
                .into(),
            NO_PILL.into()
        )
    );
    assert_eq!(
        say("status", "s-1").1,
        "Voice mode is on for this session.\n"
    );
    assert_eq!(
        say("status", "s-2").1,
        "Voice mode is off for this session.\n"
    );
    assert!(w.dir.path().join("root/voice/sessions/s-1").exists());
    assert_eq!(say("off", "s-1").1, "Voice mode is off for this session.\n");
    assert!(!w.dir.path().join("root/voice/sessions/s-1").exists());
    // An id with no letters or digits is an error of the core. A pane wins over a session.
    let (code, _, err) = say("on", "///");
    assert_eq!(code, 1);
    assert!(err.starts_with("agentsi voice: "), "{err}");
    let (code, _, err) = agentsi_env(
        &w,
        &["voice", "on"],
        &path,
        &[
            ("AGENTSI_TERMINAL", "t999"),
            ("CLAUDE_CODE_SESSION_ID", "s-1"),
        ],
    );
    assert_eq!(code, 1);
    assert!(err.starts_with("agentsi voice: "), "{err}");
    assert!(!w.dir.path().join("root/voice/sessions/s-1").exists());
}

#[test]
fn voice_on_warns_once_while_no_pill_runs_and_is_quiet_while_one_does() {
    use agentsi_proto::core_api::{Subscribe, SubscribeParams, Topic};
    let w = work(false);
    let path = w.dir.path().join("bin");
    let env = [("CLAUDE_CODE_SESSION_ID", "s-9")];
    let (code, out, err) = agentsi_env(&w, &["voice", "on"], &path, &env);
    assert_eq!((code, err.as_str()), (0, NO_PILL), "{out}");
    // Only the pill subscribes to the dictation topic.
    let mut pill = Client::connect(&w.socket).unwrap();
    pill.call::<Subscribe>(&SubscribeParams {
        topics: vec![Topic::Voice, Topic::Dictation],
    })
    .unwrap();
    let (code, _, err) = agentsi_env(&w, &["voice", "on"], &path, &env);
    assert_eq!((code, err.as_str()), (0, ""));
    // Off and status say nothing about the pill.
    drop(pill);
    for action in ["status", "off"] {
        let (code, _, err) = agentsi_env(&w, &["voice", action], &path, &env);
        assert_eq!((code, err.as_str()), (0, ""), "{action}");
    }
}

#[test]
fn voice_outside_a_pane_says_so_and_a_wrong_word_prints_the_usage() {
    let w = work(false);
    let path = w.dir.path().join("bin");
    for env in [&[][..], &[("AGENTSI_TERMINAL", "  ")][..]] {
        let (code, out, err) = agentsi_env(&w, &["voice", "on"], &path, env);
        assert_eq!((code, out.as_str()), (1, ""));
        assert!(
            err.starts_with(
                "Run this command in a pane of AgentSI or in a Claude Code session: AGENTSI_TERMINAL and CLAUDE_CODE_SESSION_ID are not set."
            ),
            "{err}"
        );
    }
    for argv in [&["voice"][..], &["voice", "maybe"], &["voice", "on", "off"]] {
        let (code, _, err) = agentsi(&w, argv, &path);
        assert_eq!(code, 1);
        assert!(
            err.ends_with("Usage: agentsi voice on|off|status\n"),
            "{err}"
        );
    }
}

#[test]
fn web_opens_the_desktop_app() {
    let w = work(false);
    let path = w.dir.path().join("bin");
    let marker = w.dir.path().join("app-ran");
    fake(
        &w.dir.path().join("bin"),
        "agentsi-app",
        &format!("echo started > '{}'", marker.display()),
    );
    let app = w.dir.path().join("bin/agentsi-app");
    let (code, out, err) = agentsi_env(
        &w,
        &["web"],
        &path,
        &[("AGENTSI_APP_BIN", app.to_str().unwrap())],
    );
    assert_eq!(
        (code, out.as_str(), err.as_str()),
        (0, "The web app is gone. Opening agentsi-app.\n", "")
    );
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
    while !marker.exists() && std::time::Instant::now() < deadline {
        std::thread::sleep(std::time::Duration::from_millis(20));
    }
    assert!(marker.exists(), "agentsi-app never started");

    // With no app on the path, the command says where to put it.
    let (code, out, err) = agentsi_env(
        &w,
        &["web"],
        Path::new("/nonexistent"),
        &[("AGENTSI_APP_BIN", "/nonexistent/agentsi-app")],
    );
    assert_eq!(code, 1);
    assert_eq!(out, "");
    assert!(
        err.starts_with("agentsi web: The web app is gone."),
        "{err}"
    );
    let (code, _, err) = agentsi(&w, &["web", "--port", "1"], &path);
    assert_eq!(code, 1);
    assert!(err.ends_with("Usage: agentsi web\n"), "{err}");
}
