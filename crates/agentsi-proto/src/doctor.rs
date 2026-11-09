//! The checks of `agentsi doctor`: the tools that AgentSI needs. Nothing is written.
//! The CLI prints them, and the core gives them to the app. They live here, because
//! the CLI depends only on this crate. The checks run on the host of the caller.

use crate::ops_api::CoreStatusResult;
use std::io::Read;
use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

/// `required` checks decide the exit code. The others only name what they block.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Level {
    Required,
    Stories,
    Optional,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Status {
    Ok,
    Missing,
    Fail,
}

impl Status {
    pub fn word(self) -> &'static str {
        match self {
            Status::Ok => "ok",
            Status::Missing => "missing",
            Status::Fail => "fail",
        }
    }
}

#[derive(Debug, PartialEq, Eq)]
pub struct CheckResult {
    pub name: &'static str,
    pub level: Level,
    pub status: Status,
    /// The version or the found path when ok. Why it failed otherwise. Empty when missing.
    pub detail: String,
    /// The features that a failed check blocks.
    pub blocks: &'static str,
}

/// What the checks read from the machine. The tests pass fakes.
pub trait Probes {
    /// The trimmed output of `cmd args`, or `None` when the command is missing or fails.
    fn output(&mut self, cmd: &str, args: &[&str]) -> Option<String>;
    /// True when bwrap can start a sandbox with no network.
    fn sandbox(&mut self) -> bool;
    /// What the core says about itself. Starts the core when none runs.
    fn core(&mut self) -> Result<CoreStatusResult, String>;
    /// The number of downloaded models and the number of models of the catalog.
    fn models(&mut self) -> Result<(usize, usize), String>;
}

/// Claude Code flags that every model call of AgentSI passes. An older CLI rejects them.
pub const CLAUDE_FLAGS: &[&str] = &[
    "--safe-mode",
    "--restricted",
    "--permission-prompts",
    "--max-budget-usd",
    "--strict-mcp-config",
    "--disable-slash-commands",
];

fn first_line(s: &str) -> String {
    s.lines().next().unwrap_or("").trim().to_string()
}

struct Check {
    name: &'static str,
    level: Level,
    blocks: &'static str,
}

const fn check(name: &'static str, level: Level, blocks: &'static str) -> Check {
    Check {
        name,
        level,
        blocks,
    }
}

const CHECKS: &[Check] = &[
    check("core", Level::Required, "every command"),
    check(
        "data root",
        Level::Required,
        "every command that reads the registry or the workspace",
    ),
    check(
        "claude",
        Level::Required,
        "meetings, ask, reflect and stories",
    ),
    check("git", Level::Required, "stories and the knowledge base"),
    check(
        "bwrap",
        Level::Stories,
        "story sessions (agentsi story run)",
    ),
    check(
        "gh",
        Level::Optional,
        "open issues in the knowledge base and the issue check of the minutes",
    ),
    check(
        "codebase-memory-mcp",
        Level::Optional,
        "the code graph of meetings and story sessions",
    ),
    check("espeak-ng", Level::Optional, "voice mode and calls"),
    check("voice models", Level::Optional, "voice mode and calls"),
    check("audio devices", Level::Optional, "voice mode and calls"),
];

fn ok(detail: impl Into<String>) -> (Status, String) {
    (Status::Ok, detail.into())
}
fn failed(detail: impl Into<String>) -> (Status, String) {
    (Status::Fail, detail.into())
}
fn missing() -> (Status, String) {
    (Status::Missing, String::new())
}

fn tool(p: &mut dyn Probes, cmd: &str, args: &[&str]) -> (Status, String) {
    p.output(cmd, args)
        .map_or_else(missing, |out| ok(first_line(&out)))
}

fn run_one(
    c: &Check,
    p: &mut dyn Probes,
    core: &Result<CoreStatusResult, String>,
) -> (Status, String) {
    match c.name {
        "core" => match core {
            Ok(s) => ok(format!("agentsi-core {}", s.version)),
            Err(e) => failed(format!("not reachable: {e}")),
        },
        "data root" => match core {
            Err(_) => failed("the core does not run, so the data root is unknown"),
            Ok(s) => match &s.database {
                Some(d) if d.schema_version < d.latest => failed(format!(
                    "{} has schema {}, the core needs {}",
                    s.data_root, d.schema_version, d.latest
                )),
                Some(d) => ok(format!("{}, schema {}", s.data_root, d.schema_version)),
                None => ok(format!("{}, no database yet", s.data_root)),
            },
        },
        "claude" => {
            let Some(version) = p.output("claude", &["--version"]) else {
                return missing();
            };
            let help = p.output("claude", &["--help"]).unwrap_or_default();
            let absent: Vec<&str> = CLAUDE_FLAGS
                .iter()
                .copied()
                .filter(|f| !help.contains(f))
                .collect();
            if absent.is_empty() {
                ok(first_line(&version))
            } else {
                failed(format!(
                    "{} lacks {}. Update Claude Code",
                    first_line(&version),
                    absent.join(", ")
                ))
            }
        }
        "git" | "gh" | "codebase-memory-mcp" | "espeak-ng" => tool(p, c.name, &["--version"]),
        "bwrap" => match p.output("bwrap", &["--version"]) {
            None => missing(),
            Some(v) if p.sandbox() => ok(first_line(&v)),
            Some(v) => failed(format!("{} cannot start a sandbox", first_line(&v))),
        },
        "voice models" => match p.models() {
            Err(e) => failed(format!("cannot ask the core: {e}")),
            Ok((have, total)) if have == total => ok(format!("{have} of {total} downloaded")),
            Ok((have, total)) => failed(format!(
                "{have} of {total} downloaded. Download them in the app"
            )),
        },
        "audio devices" => audio(p),
        _ => missing(),
    }
}

/// The sound servers in the order that a desktop uses them: PipeWire or
/// PulseAudio through `pactl`, then ALSA.
fn audio(p: &mut dyn Probes) -> (Status, String) {
    if let Some(out) = p.output("pactl", &["list", "short", "sinks"]) {
        let n = out.lines().filter(|l| !l.trim().is_empty()).count();
        if n > 0 {
            return ok(format!("{n} output(s) (pactl)"));
        }
    }
    if let Some(out) = p.output("aplay", &["-l"]) {
        let n = out.lines().filter(|l| l.starts_with("card ")).count();
        if n > 0 {
            return ok(format!("{n} card(s) (aplay)"));
        }
    }
    failed("no audio output found (pactl, aplay)")
}

/// Runs every check in order. Nothing is written.
pub fn checks(p: &mut dyn Probes) -> Vec<CheckResult> {
    let core = p.core();
    CHECKS
        .iter()
        .map(|c| {
            let (status, detail) = run_one(c, p, &core);
            CheckResult {
                name: c.name,
                level: c.level,
                status,
                detail,
                blocks: c.blocks,
            }
        })
        .collect()
}

const TOOL_TIMEOUT: Duration = Duration::from_secs(15);

/// Runs `cmd args` and returns its output, or `None` when it fails or takes
/// more than the timeout. The output is stdout, or stderr when stdout is empty.
pub fn run_tool(cmd: &str, args: &[&str]) -> Option<String> {
    let mut child = Command::new(cmd)
        .args(args)
        .current_dir(std::env::var_os("HOME").map_or_else(|| PathBuf::from("/"), PathBuf::from))
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .ok()?;
    // Read both pipes while the tool runs: a tool that writes more than the pipe holds
    // (`claude --help`) would block on a full pipe until the timeout. A grandchild can keep
    // a pipe open after the tool exits, so the text is shared and the wait for it is bounded.
    let drain = |pipe: Option<Box<dyn Read + Send>>| {
        let shared = Arc::new(Mutex::new((Vec::<u8>::new(), false)));
        let writer = Arc::clone(&shared);
        std::thread::spawn(move || {
            let mut chunk = [0u8; 8192];
            if let Some(mut p) = pipe {
                while let Ok(n) = p.read(&mut chunk) {
                    if n == 0 {
                        break;
                    }
                    writer.lock().unwrap().0.extend_from_slice(&chunk[..n]);
                }
            }
            writer.lock().unwrap().1 = true;
        });
        shared
    };
    let out = drain(child.stdout.take().map(|p| Box::new(p) as _));
    let err = drain(child.stderr.take().map(|p| Box::new(p) as _));
    let start = Instant::now();
    let status = loop {
        match child.try_wait() {
            Ok(Some(s)) => break s,
            Ok(None) if start.elapsed() <= TOOL_TIMEOUT => {
                std::thread::sleep(Duration::from_millis(10))
            }
            _ => {
                let _ = child.kill();
                let _ = child.wait();
                return None;
            }
        }
    };
    let end = Instant::now() + Duration::from_secs(2);
    while Instant::now() < end && !(out.lock().unwrap().1 && err.lock().unwrap().1) {
        std::thread::sleep(Duration::from_millis(5));
    }
    let text_of = |s: &Arc<Mutex<(Vec<u8>, bool)>>| {
        String::from_utf8_lossy(&s.lock().unwrap().0).into_owned()
    };
    let (out, err) = (text_of(&out), text_of(&err));
    let text = if out.trim().is_empty() { err } else { out };
    status.success().then(|| text.trim().to_string())
}

/// True when bwrap can start a sandbox with no network.
pub fn sandbox_starts() -> bool {
    run_tool(
        "bwrap",
        &[
            "--ro-bind",
            "/",
            "/",
            "--dev",
            "/dev",
            "--unshare-net",
            "--die-with-parent",
            "true",
        ],
    )
    .is_some()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_tool_with_a_long_output_is_read_to_the_end() {
        let start = Instant::now();
        let out = run_tool(
            "sh",
            &["-c", "head -c 300000 /dev/zero | tr '\\0' x; echo end"],
        )
        .unwrap();
        assert!(out.ends_with("end"), "{}", out.len());
        assert!(start.elapsed() < Duration::from_secs(10));
    }

    #[test]
    fn a_grandchild_that_keeps_the_pipe_open_does_not_hold_the_run() {
        let start = Instant::now();
        let out = run_tool("sh", &["-c", "sleep 30 & echo hi"]).unwrap();
        assert_eq!(out, "hi");
        assert!(start.elapsed() < Duration::from_secs(10));
    }

    #[test]
    fn stderr_does_not_count_when_stdout_has_text() {
        let out = run_tool("sh", &["-c", "echo warn >&2; echo out"]).unwrap();
        assert_eq!(out, "out");
    }
}
