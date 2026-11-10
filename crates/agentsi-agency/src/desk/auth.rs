//! The login check of `claude`: `claude auth status --json`. AgentSI never reads the
//! credentials of Claude Code. It only asks the installed program for its state.

use agentsi_proto::desk_session_api::AuthState;
use serde_json::Value;
use std::io::Read;
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

/// The limit of the check.
pub const AUTH_TIMEOUT: Duration = Duration::from_secs(5);

/// The result of one check.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Auth {
    pub state: AuthState,
    /// What the owner must do, or the first line of other output. `None` for `ready`.
    pub message: Option<String>,
}

impl Auth {
    pub fn of(state: AuthState) -> Auth {
        Auth {
            state,
            message: state.message().map(String::from),
        }
    }
}

/// Maps the output of `claude auth status --json`.
pub fn parse_status(stdout: &str, stderr: &str) -> Auth {
    if let Ok(v) = serde_json::from_str::<Value>(stdout.trim())
        && let Some(logged_in) = v.get("loggedIn").and_then(Value::as_bool)
    {
        return Auth::of(if logged_in {
            AuthState::Ready
        } else {
            AuthState::SignedOut
        });
    }
    let first = stdout
        .lines()
        .chain(stderr.lines())
        .map(str::trim)
        .find(|l| !l.is_empty())
        .unwrap_or("`claude auth status` gave no output.");
    Auth {
        state: AuthState::Unknown,
        message: Some(first.chars().take(300).collect()),
    }
}

/// Runs `<program> auth status --json`, with a limit of `timeout`.
pub fn check(program: &str, timeout: Duration) -> Auth {
    let spawned = super::process::spawn(
        Command::new(program)
            .args(["auth", "status", "--json"])
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped()),
    );
    let mut child = match spawned {
        Ok(c) => c,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Auth::of(AuthState::Missing),
        Err(e) => {
            return Auth {
                state: AuthState::Unknown,
                message: Some(format!("AgentSI cannot run `{program}`: {e}")),
            };
        }
    };
    let pipe = |mut p: Box<dyn Read + Send>| {
        std::thread::spawn(move || {
            let mut text = String::new();
            let _ = p.read_to_string(&mut text);
            text
        })
    };
    let out = pipe(Box::new(child.stdout.take().expect("stdout is piped")));
    let err = pipe(Box::new(child.stderr.take().expect("stderr is piped")));
    let deadline = Instant::now() + timeout;
    loop {
        match child.try_wait() {
            Ok(Some(_)) => break,
            Ok(None) if Instant::now() < deadline => {
                std::thread::sleep(Duration::from_millis(20));
            }
            _ => {
                let _ = child.kill();
                let _ = child.wait();
                return Auth {
                    state: AuthState::Unknown,
                    message: Some(format!(
                        "`claude auth status` did not answer in {} s.",
                        timeout.as_secs()
                    )),
                };
            }
        }
    }
    let stdout = out.join().unwrap_or_default();
    let stderr = err.join().unwrap_or_default();
    parse_status(&stdout, &stderr)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn logged_in_maps_to_ready_or_signed_out() {
        let ready = parse_status(r#"{"loggedIn":true,"authMethod":"claude.ai"}"#, "");
        assert_eq!(ready, Auth::of(AuthState::Ready));
        assert_eq!(ready.message, None);
        let out = parse_status("{\"loggedIn\": false}\n", "");
        assert_eq!(out.state, AuthState::SignedOut);
        assert!(out.message.unwrap().contains("claude auth login"));
    }

    #[test]
    fn other_output_is_unknown_with_its_first_line() {
        let a = parse_status("\nerror: unknown command 'auth'\nmore\n", "");
        assert_eq!(
            (a.state, a.message.as_deref()),
            (AuthState::Unknown, Some("error: unknown command 'auth'"))
        );
        let b = parse_status("", "boom\n");
        assert_eq!(b.message.as_deref(), Some("boom"));
        assert_eq!(
            parse_status("{\"x\":1}", "").message.as_deref(),
            Some("{\"x\":1}")
        );
    }

    #[test]
    fn no_program_is_missing() {
        let a = check("/no/such/claude", AUTH_TIMEOUT);
        assert_eq!(a.state, AuthState::Missing);
        assert!(a.message.unwrap().contains("PATH"));
    }
}
