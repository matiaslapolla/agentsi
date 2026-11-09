//! `agentsi doctor`: checks the tools that AgentSI needs. It writes nothing.
//! The lines have the shape of the TypeScript command. The checks differ: there
//! is no Postgres, no Node and no Docker. The core, the voice tools and the
//! audio devices take their place.

use super::Fail;
use crate::args;
use crate::core::Core;
use agentsi_proto::core_api::Empty;
use agentsi_proto::doctor::{CheckResult, Level, Probes, Status, checks, run_tool, sandbox_starts};
use agentsi_proto::models_api::{ModelState, ModelsList};
use agentsi_proto::ops_api::{CoreStatus, CoreStatusResult};

pub const USAGE: &str = "agentsi doctor";

pub fn passed(results: &[CheckResult]) -> bool {
    results
        .iter()
        .all(|r| r.status == Status::Ok || r.level != Level::Required)
}

/// One line for each check: status, name with its level, and the version or what a failure blocks.
pub fn lines(results: &[CheckResult]) -> Vec<String> {
    let names: Vec<String> = results
        .iter()
        .map(|r| match r.level {
            Level::Required => r.name.to_string(),
            Level::Stories => format!("{} (stories)", r.name),
            Level::Optional => format!("{} (optional)", r.name),
        })
        .collect();
    let width = names.iter().map(String::len).max().unwrap_or(0);
    results
        .iter()
        .zip(&names)
        .map(|(r, name)| {
            let text = match r.status {
                Status::Ok => r.detail.clone(),
                Status::Missing => format!("Not found. It blocks {}.", r.blocks),
                Status::Fail => format!("{}. It blocks {}.", r.detail, r.blocks),
            };
            format!("{:<7}  {name:<width$}  {text}", r.status.word())
        })
        .collect()
}

/// Prints the report and returns the exit code.
pub fn report(results: &[CheckResult]) -> i32 {
    for line in lines(results) {
        println!("{line}");
    }
    if passed(results) {
        println!("\nEvery required check passes.");
        0
    } else {
        println!("\nA required check failed. AgentSI cannot run until you fix it.");
        1
    }
}

/// The real machine.
struct Machine {
    core: Option<Result<Core, String>>,
}

impl Machine {
    fn core(&mut self) -> Result<&mut Core, String> {
        self.core
            .get_or_insert_with(Core::connect)
            .as_mut()
            .map_err(|e| e.clone())
    }
}

impl Probes for Machine {
    fn output(&mut self, cmd: &str, args: &[&str]) -> Option<String> {
        run_tool(cmd, args)
    }

    fn sandbox(&mut self) -> bool {
        sandbox_starts()
    }

    fn core(&mut self) -> Result<CoreStatusResult, String> {
        self.core()?.call::<CoreStatus>(&Empty {})
    }

    fn models(&mut self) -> Result<(usize, usize), String> {
        let list = self.core()?.call::<ModelsList>(&Empty {})?;
        let have = list
            .models
            .iter()
            .filter(|m| m.state == ModelState::Downloaded)
            .count();
        Ok((have, list.models.len()))
    }
}

pub fn run(argv: &[String]) -> Result<i32, Fail> {
    args::parse(argv, &[]).map_err(Fail::Error)?;
    let mut machine = Machine { core: None };
    Ok(report(&checks(&mut machine)))
}

#[cfg(test)]
mod tests {
    use super::*;
    use agentsi_proto::ops_api::DatabaseStatus;
    use std::collections::BTreeMap;

    #[derive(Default)]
    struct Fake {
        tools: BTreeMap<&'static str, &'static str>,
        sandbox: bool,
        core: Option<CoreStatusResult>,
        models: Option<(usize, usize)>,
    }

    impl Probes for Fake {
        fn output(&mut self, cmd: &str, args: &[&str]) -> Option<String> {
            let key = if args.first() == Some(&"--help") {
                "claude --help"
            } else {
                cmd
            };
            self.tools.get(key).map(|s| s.to_string())
        }
        fn sandbox(&mut self) -> bool {
            self.sandbox
        }
        fn core(&mut self) -> Result<CoreStatusResult, String> {
            self.core.clone().ok_or_else(|| "no socket".into())
        }
        fn models(&mut self) -> Result<(usize, usize), String> {
            self.models.ok_or_else(|| "no core".into())
        }
    }

    fn status(schema: u32) -> CoreStatusResult {
        CoreStatusResult {
            version: "0.1.0".into(),
            data_root: "/data".into(),
            database: Some(DatabaseStatus {
                path: "/data/agentsi.db".into(),
                schema_version: schema,
                latest: 3,
            }),
            pill: false,
        }
    }

    fn healthy() -> Fake {
        Fake {
            tools: BTreeMap::from([
                ("claude", "2.1.0 (Claude Code)"),
                (
                    "claude --help",
                    "--safe-mode --restricted --permission-prompts --max-budget-usd --strict-mcp-config --disable-slash-commands",
                ),
                ("git", "git version 2.50.0"),
                ("bwrap", "bubblewrap 0.11.0"),
                ("gh", "gh version 2.80.0"),
                ("espeak-ng", "eSpeak NG text-to-speech: 1.52"),
                (
                    "pactl",
                    "50\talsa_output.pci\tPipeWire\ts32le 2ch 48000Hz\tSUSPENDED",
                ),
            ]),
            sandbox: true,
            core: Some(status(3)),
            models: Some((2, 2)),
        }
    }

    #[test]
    fn a_healthy_machine_passes_and_names_each_level() {
        let r = checks(&mut healthy());
        assert!(passed(&r));
        let text = lines(&r);
        let width = "codebase-memory-mcp (optional)".len();
        assert_eq!(
            text[0],
            format!("{:<7}  {:<width$}  agentsi-core 0.1.0", "ok", "core")
        );
        assert_eq!(
            text[4],
            format!(
                "{:<7}  {:<width$}  bubblewrap 0.11.0",
                "ok", "bwrap (stories)"
            )
        );
        assert_eq!(
            text[6],
            format!(
                "{:<7}  {:<width$}  Not found. It blocks the code graph of meetings and story sessions.",
                "missing", "codebase-memory-mcp (optional)"
            )
        );
    }

    #[test]
    fn a_missing_required_tool_fails_the_run() {
        let mut f = healthy();
        f.tools.remove("claude");
        let r = checks(&mut f);
        assert!(!passed(&r));
        assert_eq!(r[2].status, Status::Missing);
    }

    #[test]
    fn an_old_claude_names_the_flags_it_lacks() {
        let mut f = healthy();
        f.tools.insert("claude --help", "--safe-mode --restricted");
        let r = checks(&mut f);
        assert_eq!(r[2].status, Status::Fail);
        assert!(r[2].detail.contains("--max-budget-usd"));
    }

    #[test]
    fn an_optional_failure_does_not_fail_the_run() {
        let mut f = healthy();
        f.tools.remove("espeak-ng");
        f.models = None;
        assert!(passed(&checks(&mut f)));
    }

    #[test]
    fn no_core_fails_the_core_and_the_data_root() {
        let mut f = healthy();
        f.core = None;
        let r = checks(&mut f);
        assert_eq!((r[0].status, r[1].status), (Status::Fail, Status::Fail));
        assert!(!passed(&r));
    }

    #[test]
    fn an_old_schema_fails_the_data_root() {
        let mut f = healthy();
        f.core = Some(status(2));
        assert_eq!(checks(&mut f)[1].status, Status::Fail);
    }

    #[test]
    fn a_sandbox_that_cannot_start_fails_the_stories_check() {
        let mut f = healthy();
        f.sandbox = false;
        let r = checks(&mut f);
        assert_eq!(r[4].status, Status::Fail);
        assert!(passed(&r));
    }
}
