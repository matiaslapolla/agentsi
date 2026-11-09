//! The `claude -p` calls of stories mode: the PM call of `claude.ts` and the
//! story session of `story-run.ts`, with the Bash rules and the Claude Code
//! sandbox settings of ADR 0002. The argument lists are the TypeScript ones.

use crate::sandbox::{Mask, die_with_parent, finish};
use agentsi_proto::story_api::{StoryConfig, StoryUsage};
use serde_json::{Value, json};
use std::io::Write;
use std::path::Path;
use std::process::{Command, Stdio};
use std::time::Duration;

/// The `subtype` of a run that reached `--max-budget-usd`.
pub const BUDGET_STOP: &str = "error_max_budget_usd";

const FILE_TOOLS: [&str; 5] = ["Read", "Grep", "Glob", "Edit", "Write"];
const READ_COMMANDS: [&str; 6] = ["ls", "cat", "head", "tail", "wc", "pwd"];

/// Denied even when an allow rule matches: the runner, not the session,
/// commits, and nothing leaves the machine.
pub const DENIED_TOOLS: [&str; 21] = [
    "Bash(git:*)",
    "Bash(git push:*)",
    "Bash(gh:*)",
    "Bash(vercel:*)",
    "Bash(curl:*)",
    "Bash(wget:*)",
    "Bash(ssh:*)",
    "Bash(scp:*)",
    "Bash(rsync:*)",
    "Bash(*db:migrate*)",
    "Bash(*db:push*)",
    "Bash(*deploy*)",
    "Bash(npm publish:*)",
    "Bash(pnpm publish:*)",
    "Bash(yarn publish:*)",
    "WebFetch",
    "WebSearch",
    // git diff cannot see a change to ignored files, so the session must not touch the toolchain or the git link.
    "Edit(./node_modules/**)",
    "Edit(./**/node_modules/**)",
    "Edit(./.git)",
    "Edit(./.git/**)",
];

/// A loop guard, not a cost target: about 3x the most expensive call measured per model.
pub fn default_budget_usd(model: &str) -> f64 {
    if model.contains("haiku") { 0.1 } else { 0.4 }
}

/// A number as `String(n)` of JavaScript prints it.
pub fn js_number(v: f64) -> String {
    if v.fract() == 0.0 && v.abs() < 1e15 {
        format!("{}", v as i64)
    } else {
        format!("{v}")
    }
}

pub fn shell_quote(s: &str) -> String {
    format!("'{}'", s.replace('\'', "'\\''"))
}

/// The test command with the files in place of `{files}`.
pub fn test_command(config: &StoryConfig, files: &[String]) -> String {
    let quoted: Vec<String> = files.iter().map(|f| shell_quote(f)).collect();
    config.test.replace("{files}", &quoted.join(" "))
}

fn test_prefix(config: &StoryConfig) -> String {
    let at = config.test.find("{files}").unwrap_or(config.test.len());
    config.test[..at].trim().to_string()
}

/// The exact rule of `command`: each unescaped `*` gets a backslash, so it is
/// not a wildcard.
pub fn exact_rule(command: &str) -> String {
    let mut out = String::new();
    let mut slashes = 0usize;
    for c in command.chars() {
        if c == '*' && slashes.is_multiple_of(2) {
            out.push('\\');
        }
        out.push(c);
        slashes = if c == '\\' { slashes + 1 } else { 0 };
    }
    out
}

/// The text of a rule inside `Bash(...)`. Claude Code removes one backslash
/// before a backslash, `(` or `)` when it reads the rule.
fn rule_text(rule: &str) -> String {
    rule.replace('\\', "\\\\")
        .replace('(', "\\(")
        .replace(')', "\\)")
}

/// The Bash rules of a story session: the test command up to `{files}` as a
/// prefix, the test command of the story's files exactly, each gate command
/// exactly, and a few read commands as prefixes.
pub fn bash_allowlist(config: &StoryConfig, files: &[String]) -> Vec<String> {
    let test = test_prefix(config);
    let mut rules: Vec<String> = Vec::new();
    if !test.is_empty() {
        rules.push(format!("{test}:*"));
        if !files.is_empty() {
            rules.push(exact_rule(&test_command(config, files)));
        }
    }
    rules.extend(config.gate.iter().map(|g| exact_rule(g.trim())));
    rules.extend(READ_COMMANDS.iter().map(|c| format!("{c}:*")));
    let mut seen = std::collections::BTreeSet::new();
    rules
        .into_iter()
        .filter(|r| seen.insert(r.clone()))
        .map(|r| format!("Bash({})", rule_text(&r)))
        .collect()
}

/// Deny rules for a flag after the test prefix, such as `--import ./evil.mjs`.
pub fn bash_denylist(config: &StoryConfig) -> Vec<String> {
    let test = test_prefix(config);
    if test.is_empty() {
        return Vec::new();
    }
    let flags = ["-*", "'-*", "\"-*"];
    let mut all: Vec<String> = flags.iter().map(|f| f.to_string()).collect();
    if config.test.trim().ends_with("{files}") {
        all.extend(flags.iter().map(|f| format!("* {f}")));
    }
    all.iter()
        .map(|f| format!("Bash({})", rule_text(&format!("{test} {f}"))))
        .collect()
}

/// The Bash sandbox of a session: writes in the cwd and the temp dir, minus
/// `deny_write`, reads everywhere except the masked paths, no network, no
/// unsandboxed retry, and no start without the sandbox.
pub fn sandbox_settings(deny_write: &[String], mask: &Mask) -> Value {
    json!({
        "sandbox": {
            "enabled": true,
            "failIfUnavailable": true,
            "allowUnsandboxedCommands": false,
            "autoAllowBashIfSandboxed": false,
            "network": { "allowedDomains": [], "strictAllowlist": true },
            "filesystem": { "denyWrite": deny_write, "denyRead": mask.hide, "allowRead": mask.keep },
        }
    })
}

/// One `claude -p` session of a story run, with its cwd in the story worktree.
#[derive(Debug, Clone, Default)]
pub struct SessionCall {
    pub agent: String,
    pub model: String,
    pub effort: Option<String>,
    pub system: String,
    pub prompt: String,
    pub cwd: String,
    /// `Bash(...)` rules the session may run, besides the file tools.
    pub bash: Vec<String>,
    pub deny_bash: Vec<String>,
    pub deny_write: Vec<String>,
    pub mask: Mask,
    pub budget_usd: f64,
    /// The code graph server, when its reindex worked.
    pub mcp: Option<McpAccess>,
}

/// MCP servers and the tools of theirs that a call may use.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct McpAccess {
    /// JSON for `--mcp-config`.
    pub config: String,
    /// `mcp__<server>__<tool>`
    pub tools: Vec<String>,
}

pub fn session_args(c: &SessionCall) -> Vec<String> {
    let tools: Vec<&str> = FILE_TOOLS.iter().copied().chain(["Bash"]).collect();
    let allowed: Vec<String> = FILE_TOOLS
        .iter()
        .map(|t| t.to_string())
        .chain(c.bash.iter().cloned())
        .chain(c.mcp.iter().flat_map(|m| m.tools.iter().cloned()))
        .collect();
    let denied: Vec<String> = DENIED_TOOLS
        .iter()
        .map(|t| t.to_string())
        .chain(c.deny_bash.iter().cloned())
        .collect();
    let mut args: Vec<String> = [
        "-p",
        "--model",
        &c.model,
        "--system-prompt",
        &c.system,
        "--output-format",
        "json",
        "--no-session-persistence",
    ]
    .iter()
    .map(|s| s.to_string())
    .collect();
    // --safe-mode also drops the servers of --mcp-config. --restricted and --strict-mcp-config still apply.
    match &c.mcp {
        Some(m) => args.extend(["--mcp-config".into(), m.config.clone()]),
        None => args.push("--safe-mode".into()),
    }
    args.extend(
        [
            "--restricted",
            "--strict-mcp-config",
            "--disable-slash-commands",
            "--permission-mode",
            "dontAsk",
            "--permission-prompts",
            "none",
            "--max-budget-usd",
        ]
        .map(String::from),
    );
    args.push(js_number(c.budget_usd));
    args.push(format!("--tools={}", tools.join(",")));
    args.extend(["--allowedTools".into(), allowed.join(",")]);
    args.extend(["--disallowedTools".into(), denied.join(",")]);
    args.extend([
        "--settings".into(),
        sandbox_settings(&c.deny_write, &c.mask).to_string(),
    ]);
    if let Some(e) = &c.effort {
        args.extend(["--effort".into(), e.clone()]);
    }
    args
}

/// One headless call that answers with structured output (the PM).
#[derive(Debug, Clone, Default)]
pub struct Call {
    pub prompt: String,
    pub model: String,
    pub system: String,
    pub cwd: String,
    pub tools: Vec<String>,
    pub schema: Option<Value>,
    pub effort: Option<String>,
    pub timeout: Duration,
    /// The MCP servers, and the tools of theirs that the call may use.
    pub mcp: Option<McpAccess>,
    /// The cap of the call in USD. `None` takes `default_budget_usd` of the model.
    pub budget_usd: Option<f64>,
}

pub fn call_args(c: &Call) -> Vec<String> {
    let mut args: Vec<String> = [
        "-p",
        "--model",
        &c.model,
        "--system-prompt",
        &c.system,
        "--output-format",
        "json",
        "--no-session-persistence",
    ]
    .iter()
    .map(|s| s.to_string())
    .collect();
    // --safe-mode also drops the servers of --mcp-config. --restricted and --strict-mcp-config still apply.
    if c.mcp.is_none() {
        args.push("--safe-mode".into());
    }
    args.extend(
        [
            "--restricted",
            "--strict-mcp-config",
            "--disable-slash-commands",
            "--permission-mode",
            "dontAsk",
            "--permission-prompts",
            "none",
            "--max-budget-usd",
        ]
        .map(String::from),
    );
    args.push(js_number(
        c.budget_usd.unwrap_or_else(|| default_budget_usd(&c.model)),
    ));
    args.push(format!("--tools={}", c.tools.join(",")));
    let mut allowed = c.tools.clone();
    if let Some(mcp) = &c.mcp {
        args.extend(["--mcp-config".into(), mcp.config.clone()]);
        allowed.extend(mcp.tools.iter().cloned());
    }
    if !allowed.is_empty() {
        args.extend(["--allowedTools".into(), allowed.join(",")]);
    }
    if let Some(e) = &c.effort {
        args.extend(["--effort".into(), e.clone()]);
    }
    if let Some(s) = &c.schema {
        args.extend(["--json-schema".into(), s.to_string()]);
    }
    args
}

/// Maps the `claude -p --output-format json` envelope to a usage row.
pub fn map_usage(raw: &Value, role: &str, requested: &str) -> StoryUsage {
    let u = &raw["usage"];
    let n = |v: &Value| v.as_f64().unwrap_or(0.0).max(0.0) as u64;
    let mut models: Vec<(&String, f64)> = raw["modelUsage"]
        .as_object()
        .map(|m| {
            m.iter()
                .map(|(k, v)| (k, v["costUSD"].as_f64().unwrap_or(0.0)))
                .collect()
        })
        .unwrap_or_default();
    models.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));
    StoryUsage {
        role: role.into(),
        model: models
            .first()
            .map_or(requested.to_string(), |m| m.0.clone()),
        input_tokens: n(&u["input_tokens"]),
        output_tokens: n(&u["output_tokens"]),
        cache_read: n(&u["cache_read_input_tokens"]),
        cache_write: n(&u["cache_creation_input_tokens"]),
        cost_usd: raw["total_cost_usd"].as_f64().unwrap_or(0.0),
        duration_ms: raw["duration_ms"].as_f64().unwrap_or(0.0).round().max(0.0) as u64,
    }
}

/// Runs `claude` once and parses its JSON envelope. The prompt goes through
/// stdin. `on_start` gets the pid of its process group.
pub fn run_claude(
    args: &[String],
    cwd: &Path,
    prompt: &str,
    timeout: Duration,
    env: &[(&str, &str)],
    on_start: &dyn Fn(Option<u32>),
) -> Result<Value, String> {
    use std::os::unix::process::CommandExt;
    let mut command = Command::new("claude");
    command
        .args(args)
        .current_dir(cwd)
        .envs(crate::git_env::env_for_cwd(cwd))
        .envs(env.iter().copied())
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .process_group(0);
    die_with_parent(&mut command);
    let mut child = command
        .spawn()
        .map_err(|e| format!("claude does not start: {e}"))?;
    if let Some(mut stdin) = child.stdin.take() {
        let prompt = prompt.to_string();
        std::thread::spawn(move || {
            let _ = stdin.write_all(prompt.as_bytes());
        });
    }
    let done = finish(child, timeout, on_start);
    let (stdout, stderr, code) = (done.stdout, done.stderr, done.code);
    if done.timed_out {
        return Err(format!("claude timed out after {}s", timeout.as_secs()));
    }
    let raw: Value = serde_json::from_str(&stdout).map_err(|_| {
        let text = if stderr.is_empty() { &stdout } else { &stderr };
        let text: String = text.trim().chars().take(300).collect();
        format!(
            "claude exited {} without JSON: {text}",
            code.map_or("null".into(), |c| c.to_string())
        )
    })?;
    Ok(raw)
}

/// The error text of a run that ended with `is_error`.
pub fn error_text(raw: &Value) -> String {
    let body = raw["result"]
        .as_str()
        .map(String::from)
        .or_else(|| {
            raw["errors"].as_array().map(|e| {
                e.iter()
                    .filter_map(Value::as_str)
                    .collect::<Vec<_>>()
                    .join("; ")
            })
        })
        .unwrap_or_default();
    format!(
        "claude error ({}): {}",
        raw["subtype"].as_str().unwrap_or("unknown"),
        body.chars().take(300).collect::<String>()
    )
}

/// The answer of a story session.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct SessionResult {
    pub text: String,
    pub cost_usd: f64,
    pub duration_ms: u64,
    /// True when Claude Code stopped the session at its budget cap.
    pub budget_stop: bool,
}

/// Maps the JSON result of a session. A budget stop is a result; any other error is not.
pub fn session_result(raw: &Value, model: &str) -> Result<SessionResult, String> {
    let budget = raw["subtype"].as_str() == Some(BUDGET_STOP);
    if raw["is_error"].as_bool() == Some(true) && !budget {
        return Err(error_text(raw));
    }
    let usage = map_usage(raw, "", model);
    Ok(SessionResult {
        text: raw["result"].as_str().unwrap_or_default().to_string(),
        cost_usd: usage.cost_usd,
        duration_ms: usage.duration_ms,
        budget_stop: budget,
    })
}

/// The real session caller.
pub fn run_session(
    c: &SessionCall,
    timeout: Duration,
    on_start: &dyn Fn(Option<u32>),
) -> Result<SessionResult, String> {
    let raw = run_claude(
        &session_args(c),
        Path::new(&c.cwd),
        &c.prompt,
        timeout,
        &[],
        on_start,
    )?;
    session_result(&raw, &c.model)
}

/// One PM call: the structured output and its usage.
pub fn run_call(c: &Call, role: &str) -> Result<(Value, StoryUsage), String> {
    // A call without tools is a single turn: a cache write would cost 25% more and never be read.
    let env: &[(&str, &str)] = if c.tools.is_empty() {
        &[("DISABLE_PROMPT_CACHING", "1")]
    } else {
        &[]
    };
    let raw = run_claude(
        &call_args(c),
        Path::new(&c.cwd),
        &c.prompt,
        c.timeout,
        env,
        &|_| {},
    )?;
    call_output(&raw, c, role)
}

/// The answer in the envelope of one call, and its usage.
pub fn call_output(raw: &Value, c: &Call, role: &str) -> Result<(Value, StoryUsage), String> {
    if raw["is_error"].as_bool() == Some(true) {
        return Err(error_text(raw));
    }
    let output = if c.schema.is_some() {
        raw["structured_output"].clone()
    } else {
        raw["result"].clone()
    };
    if output.is_null() || output == Value::String(String::new()) {
        return Err(format!(
            "claude returned no {} ({})",
            if c.schema.is_some() {
                "structured output"
            } else {
                "text"
            },
            raw["subtype"].as_str().unwrap_or("unknown")
        ));
    }
    Ok((output, map_usage(raw, role, &c.model)))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn config(test: &str, gate: &[&str]) -> StoryConfig {
        StoryConfig {
            repo: "/r".into(),
            test: test.into(),
            gate: gate.iter().map(|g| g.to_string()).collect(),
            budget_usd: 5.0,
        }
    }

    #[test]
    fn a_star_in_a_gate_is_escaped_and_the_test_prefix_is_a_rule() {
        let c = config("node --test {files}", &["pnpm lint *.ts", "pnpm typecheck"]);
        assert_eq!(
            bash_allowlist(&c, &["test/a b.test.ts".into()]),
            vec![
                "Bash(node --test:*)",
                "Bash(node --test 'test/a b.test.ts')",
                "Bash(pnpm lint \\\\*.ts)",
                "Bash(pnpm typecheck)",
                "Bash(ls:*)",
                "Bash(cat:*)",
                "Bash(head:*)",
                "Bash(tail:*)",
                "Bash(wc:*)",
                "Bash(pwd:*)",
            ]
        );
        assert_eq!(
            bash_denylist(&c),
            vec![
                "Bash(node --test -*)",
                "Bash(node --test '-*)",
                "Bash(node --test \"-*)",
                "Bash(node --test * -*)",
                "Bash(node --test * '-*)",
                "Bash(node --test * \"-*)",
            ]
        );
        assert_eq!(exact_rule("a \\* b * c"), "a \\* b \\* c");
    }

    #[test]
    fn the_session_args_hold_the_sandbox_settings_and_the_budget() {
        let call = SessionCall {
            model: "sonnet".into(),
            budget_usd: 0.5,
            deny_write: vec!["/r/.git".into()],
            mask: Mask {
                hide: vec!["/h/.ssh".into()],
                keep: vec!["/h/.config/git".into()],
            },
            ..SessionCall::default()
        };
        let args = session_args(&call);
        let after = |flag: &str| args[args.iter().position(|a| a == flag).unwrap() + 1].clone();
        assert_eq!(after("--max-budget-usd"), "0.5");
        let settings: Value = serde_json::from_str(&after("--settings")).unwrap();
        assert_eq!(settings["sandbox"]["failIfUnavailable"], true);
        assert_eq!(settings["sandbox"]["filesystem"]["denyRead"][0], "/h/.ssh");
        assert!(after("--disallowedTools").starts_with("Bash(git:*),Bash(git push:*)"));
    }

    #[test]
    fn a_budget_stop_is_a_result_and_another_error_is_not() {
        let stop = json!({"is_error": true, "subtype": BUDGET_STOP, "total_cost_usd": 0.51});
        assert!(session_result(&stop, "m").unwrap().budget_stop);
        let other =
            json!({"is_error": true, "subtype": "error_during_execution", "result": "boom"});
        assert_eq!(
            session_result(&other, "m").unwrap_err(),
            "claude error (error_during_execution): boom"
        );
    }
}
