//! The golden test: each ported command runs on the fixed fixture, against a real
//! core, and its output must equal the file in `tests/golden/` that the
//! TypeScript CLI wrote on the same data. `cargo test -p agentsi-cli --test golden`.
//! A case with `rows` also leaves rows that must equal `<case>.rows`.
//! To write the files again: `tests/golden/make-golden.sh`.

#[path = "golden/support.rs"]
mod support;

use agentsi_proto::client::Client;
use agentsi_proto::core_api::{Empty, Shutdown};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

/// A core that the first command starts. Drop stops it.
struct CoreGuard {
    socket: PathBuf,
}

impl Drop for CoreGuard {
    fn drop(&mut self) {
        if let Ok(mut c) = Client::connect(&self.socket) {
            let _ = c.call::<Shutdown>(&Empty {});
        }
    }
}

/// The two programs under test, built from this checkout and copied to a folder
/// of this run.
///
/// The shared `CARGO_TARGET_DIR` holds one `agentsi` and one `agentsi-core` for
/// all the workers, and cargo does not rebuild a program when the file in the
/// folder is newer than the sources, also when another checkout built it. So
/// the test marks the two `main.rs` files as changed, builds, and copies at once.
/// `cargo test` of this package builds the CLI but not the core.
struct Programs {
    _dir: tempfile::TempDir,
    cli: PathBuf,
    core: PathBuf,
}

fn programs() -> Programs {
    // Read at run time: the shared target folder reuses this test binary across checkouts.
    let manifest = PathBuf::from(std::env::var("CARGO_MANIFEST_DIR").expect("run with cargo"));
    for main in [
        manifest.join("src/main.rs"),
        manifest.join("../agentsi-core/src/main.rs"),
    ] {
        let file = std::fs::OpenOptions::new().write(true).open(&main).unwrap();
        file.set_modified(std::time::SystemTime::now()).unwrap();
    }
    let cargo = std::env::var("CARGO").unwrap_or_else(|_| "cargo".into());
    let status = Command::new(cargo)
        .args([
            "build",
            "-q",
            "-p",
            "agentsi-cli",
            "-p",
            "agentsi-core",
            "--bins",
        ])
        .status()
        .expect("cargo runs");
    assert!(
        status.success(),
        "cargo build -p agentsi-cli -p agentsi-core"
    );
    let built = Path::new(env!("CARGO_BIN_EXE_agentsi")).parent().unwrap();
    let dir = tempfile::tempdir().unwrap();
    let (cli, core) = (dir.path().join("agentsi"), dir.path().join("agentsi-core"));
    std::fs::copy(built.join("agentsi"), &cli).unwrap();
    std::fs::copy(built.join("agentsi-core"), &core).unwrap();
    Programs {
        _dir: dir,
        cli,
        core,
    }
}

/// The `agentsi` command with the environment of a golden run.
fn agentsi(work: &Path, socket: &Path, p: &Programs, cwd: &str) -> Command {
    let mut command = Command::new(&p.cli);
    command
        .current_dir(support::cwd_of(work, cwd))
        .env_clear()
        .env("PATH", support::path_of(work))
        .env("HOME", work.join("home"))
        .env("XDG_CONFIG_HOME", work.join("config"))
        .env("TZ", "UTC")
        .env("AGENTSI_HOME", work.join("root"))
        .env("AGENTSI_CORE_SOCKET", socket)
        .env("AGENTSI_CORE_BIN", &p.core)
        .envs(support::GIT_ENV)
        .stdin(Stdio::null());
    command
}

fn run(work: &Path, socket: &Path, p: &Programs, cwd: &str, args: &[String]) -> String {
    if args.first().is_some_and(|a| a.starts_with('@')) {
        let (code, out, err) = support::harness(work, &args[0]);
        return support::golden_text(work, code, &out, &err);
    }
    let out = agentsi(work, socket, p, cwd)
        .args(args)
        .output()
        .expect("agentsi runs");
    support::golden_text(
        work,
        out.status.code().unwrap_or(-1),
        &String::from_utf8_lossy(&out.stdout),
        &String::from_utf8_lossy(&out.stderr),
    )
}

/// In a step, the id of the last reflection of the run.
const LAST_REFLECTION: &str = "{LAST_REFLECTION}";

/// The text of the golden file of one case. A case with more than one step
/// prints `## <step>` before the output of each.
fn run_case(work: &Path, socket: &Path, p: &Programs, case: &support::Case) -> String {
    let steps = case.steps(work);
    let many = steps.len() > 1;
    steps
        .iter()
        .map(|step| {
            let step: Vec<String> = if step.iter().any(|a| a.contains(LAST_REFLECTION)) {
                let id = support::last_reflection(work);
                step.iter()
                    .map(|a| a.replace(LAST_REFLECTION, &id))
                    .collect()
            } else {
                step.clone()
            };
            let text = run(work, socket, p, &case.cwd, &step);
            if many {
                support::step_text(work, &step.join(" "), &text)
            } else {
                text
            }
        })
        .collect()
}

/// The output of a case, and for a case with `rows` the rows that it left.
/// A fresh case has a work folder and a core of its own.
fn run_one(
    shared: &(PathBuf, PathBuf),
    p: &Programs,
    case: &support::Case,
) -> (String, Option<String>) {
    if !case.fresh {
        return (run_case(&shared.0, &shared.1, p, case), None);
    }
    let dir = tempfile::tempdir().unwrap();
    let work = std::fs::canonicalize(dir.path()).unwrap();
    support::prepare(&work);
    let socket = work.join("run/core.sock");
    let _guard = CoreGuard {
        socket: socket.clone(),
    };
    let text = run_case(&work, &socket, p, case);
    // The rows of SQLite after the Rust CLI, against the rows of Postgres after the TypeScript CLI.
    let rows = if case.reflect_rows {
        Some(support::reflect_rows_text(
            &work,
            support::reflect_sqlite_rows(&work),
        ))
    } else {
        case.rows
            .then(|| support::rows_text(support::sqlite_rows(&work.join("root/agentsi.db"))))
    };
    (text, rows)
}

#[test]
fn each_ported_command_prints_what_the_typescript_cli_printed() {
    let programs = programs();
    let dir = tempfile::tempdir().unwrap();
    // `/tmp/.tmpXXXX`: no symbolic link in the path, so a process cwd equals the work path.
    let work = std::fs::canonicalize(dir.path()).unwrap();
    support::prepare(&work);
    let socket = work.join("run/core.sock");
    let _guard = CoreGuard {
        socket: socket.clone(),
    };

    let mut failures = Vec::new();
    let mut check = |name: &str, extension: &str, got: String| {
        let file = support::golden_dir().join(format!("{name}.{extension}"));
        if std::env::var_os("AGENTSI_BLESS_RUST").is_some() {
            // For a look at a difference only. A file that the Rust CLI wrote proves nothing.
            std::fs::write(file.with_extension("rust"), &got).unwrap();
        }
        let want =
            std::fs::read_to_string(&file).unwrap_or_else(|e| panic!("{}: {e}", file.display()));
        if got != want {
            failures.push(format!(
                "--- {name} ({extension}) ---\nwant:\n{want}\ngot:\n{got}"
            ));
        }
    };
    let shared = (work.clone(), socket.clone());
    for case in support::cases() {
        let (got, rows) = run_one(&shared, &programs, &case);
        check(&case.name, "golden", got);
        if let Some(rows) = rows {
            check(&case.name, "rows", rows);
        }
    }
    // The MCP sessions run after the cases, so the index holds what `index` and `desk` wrote.
    for case in support::mcp_cases() {
        let requests = std::fs::File::open(&case.requests).unwrap();
        let out = agentsi(&work, &socket, &programs, &case.cwd)
            .arg("mcp")
            .stdin(requests)
            .output()
            .expect("agentsi mcp runs");
        check(
            &case.name,
            "golden",
            support::mcp_golden(&work, &String::from_utf8_lossy(&out.stdout)),
        );
    }
    check("documents", "golden", documents(&work));
    assert!(
        failures.is_empty(),
        "{} golden file(s) differ:\n\n{}",
        failures.len(),
        failures.join("\n")
    );
}

/// The file `reflect-revert.golden` holds the rows before the reflection and after its
/// revert, as Postgres gave them to the TypeScript CLI. The test above proves that the
/// Rust CLI leaves the same rows. This one proves what those rows say: each earlier
/// item is back exactly, and the only new items are the retired additions.
#[test]
fn a_revert_restores_each_earlier_item_exactly() {
    let text =
        std::fs::read_to_string(support::golden_dir().join("reflect-revert.golden")).unwrap();
    let rows: Vec<serde_json::Value> = text
        .split("## @reflect-rows\nexit: 0\n--- stdout\n")
        .skip(1)
        .map(|part| {
            let json = part.split("\n--- stderr").next().unwrap();
            serde_json::from_str(json).unwrap()
        })
        .collect();
    let (before, after) = (&rows[0]["memory"], &rows[1]["memory"]);
    let before = before.as_array().unwrap();
    let after = after.as_array().unwrap();
    assert!(before.len() >= 6);
    for item in before {
        assert!(after.contains(item), "not restored: {item}");
    }
    let added: Vec<_> = after.iter().filter(|i| !before.contains(i)).collect();
    assert_eq!(added.len(), 2, "the two additions of the reflection");
    for item in added {
        assert_eq!(item["kind"], "lesson");
        assert_eq!(item["harmful"], 1);
        assert!(
            item["retired_why"]
                .as_str()
                .unwrap()
                .starts_with("reverted by r-")
        );
    }
}

/// A new id is random. `m-000e` failed this test once, when a rule kept each id that
/// starts like a fixture id.
#[test]
fn a_new_memory_id_never_stays_in_the_output_of_a_reflection_case() {
    let dir = tempfile::tempdir().unwrap();
    let text = "m-000e m-0001 m-abcd m-k001";
    assert_eq!(support::normalize(text, dir.path()), text);
    std::fs::write(dir.path().join(support::SQL_LOADED), "").unwrap();
    assert_eq!(
        support::normalize(text, dir.path()),
        "m-<NEW> m-<NEW> m-<NEW> m-k001"
    );
}

/// The `documents` table that the Rust indexer left, in the golden form.
fn documents(work: &Path) -> String {
    let conn = rusqlite::Connection::open_with_flags(
        work.join("root/agentsi.db"),
        rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY,
    )
    .unwrap();
    let mut stmt = conn
        .prepare("select product, scope, kind, ref, title, body, updated_at from documents")
        .unwrap();
    let rows = stmt
        .query_map([], |r| {
            Ok(support::DocRow {
                product: r.get(0)?,
                scope: r.get(1)?,
                kind: r.get(2)?,
                reference: r.get(3)?,
                title: r.get(4)?,
                body: r.get(5)?,
                updated_at: r.get(6)?,
            })
        })
        .unwrap()
        .collect::<Result<Vec<_>, _>>()
        .unwrap();
    support::documents_golden(work, rows)
}
