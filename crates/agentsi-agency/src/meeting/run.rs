//! The flow of a meeting and of one question. The rules copy `runMeeting` and
//! `askRole` of `packages/core/src/meeting.ts`.

use super::check::{check, resolve_source, source_context};
use super::claude::{Cancel, ClaudeResult, claude, total_input_tokens};
use super::log::{AgentAnswerLog, build_log_entry, js_number, sha256};
use super::minutes::{MinutesMeta, build_minutes, empty_answer, new_minutes_id, to_role_answer};
use super::prompts::{
    Repair, RoleContext, SynthesisContext, compact_snapshot, role_call, synthesis_call,
};
use super::{Error, Result};
use crate::knowledge::repos::repo_targets;
use crate::knowledge::{code_graph, index_product};
use crate::prompt_inputs::memory_for_prompt;
use crate::registry::{Product, Registry};
use crate::roles::files::RoleFiles;
use crate::roles::load::load_role;
use crate::story::claude::Call;
use crate::{clock, roster, snapshot};
use agentsi_proto::meeting_api::{MeetingProgressParams, MeetingUsage};
use agentsi_proto::roles_api::RoleDef;
use agentsi_store::meeting::{self as write, MinutesHead, UsageWrite};
use agentsi_store::{Store, read};
use rusqlite::Connection;
use serde_json::{Value, json};
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::Instant;

/// Where the events of a meeting go. The calls come from several threads.
pub trait Sink: Sync {
    fn progress(&self, event: MeetingProgressParams);
    /// One stream-json line of one role.
    fn line(&self, role: &str, line: &str);
}

/// What a meeting runs on.
pub struct Env<'a> {
    pub store: &'a Store,
    pub root: &'a Path,
    pub registry: &'a Registry,
    pub sink: &'a dyn Sink,
    pub cancel: &'a Cancel,
    /// The program to run: `claude`, or a fake one in a test.
    pub claude: &'a str,
}

pub struct MeetingOptions {
    /// The name of the meeting in its events.
    pub meeting: String,
    pub product: String,
    pub scope: Option<String>,
    pub question: String,
    /// The agents that attend. Empty: the roster.
    pub roles: Vec<String>,
    /// The id of stored minutes: the meeting reads their frozen snapshot and does not index.
    pub frozen: Option<String>,
}

pub struct MeetingOutcome {
    pub minutes: Value,
    /// In the order in which the runs ended.
    pub usage: Vec<MeetingUsage>,
    pub total_cost_usd: f64,
}

pub struct AskOutcome {
    pub answer: String,
    pub usage: MeetingUsage,
    /// What TypeScript prints on stderr: tools that a meeting does not allow.
    pub warnings: Vec<String>,
}

const CANCELED: &str = "The meeting was canceled. Nothing was saved.";

/// `productCwd`: the first repo of the product, else its first folder.
pub(crate) fn product_cwd(product: &Product, scope: Option<&str>) -> Result<PathBuf> {
    // No target has a scope today, so a scope finds no repo.
    let target = repo_targets(product)
        .into_iter()
        .find(|_| scope.is_none())
        .map(|t| t.path);
    target
        .or_else(|| {
            product
                .paths
                .iter()
                .find(|p| Path::new(p).exists())
                .cloned()
        })
        .map(PathBuf::from)
        .ok_or_else(|| Error::Invalid(format!("No existing path for product \"{}\"", product.id)))
}

fn reading<T>(store: &Store, f: impl FnOnce(&Connection) -> Result<T>) -> Result<T> {
    let mut out = None;
    store.read(|c| {
        out = Some(f(c));
        Ok(())
    })?;
    out.expect("the read ran")
}

fn seconds(started: Instant) -> f64 {
    started.elapsed().as_secs_f64()
}

fn usage_of(role: &str, r: &ClaudeResult) -> MeetingUsage {
    MeetingUsage {
        role: role.into(),
        ..r.usage.clone()
    }
}

/// One agent with its memory text.
struct Agent {
    def: RoleDef,
    memory: String,
}

/// The definition and the memory of each agent, and the pm that writes the synthesis.
/// It fails before any call is paid.
fn prepare_agents(
    c: &Connection,
    root: &Path,
    product: &str,
    ids: &[String],
    warnings: &mut Vec<String>,
) -> Result<(Vec<Agent>, RoleDef)> {
    let files = RoleFiles::at(root);
    let overlays = roster::read_overlays(c, product).map_err(Error::Agency)?;
    let mut agents = Vec::new();
    for id in ids {
        let def = load_role(id, product, &files, &overlays, warnings).map_err(Error::Invalid)?;
        agents.push(Agent {
            def,
            memory: memory_for_prompt(c, product, id)?,
        });
    }
    let pm = load_role("pm", product, &files, &overlays, warnings).map_err(Error::Invalid)?;
    Ok((agents, pm))
}

/// Emits the events of one meeting.
struct Emit<'a> {
    meeting: &'a str,
    product: &'a str,
    sink: &'a dyn Sink,
}

impl Emit<'_> {
    fn event(&self, stage: &str, phase: &str) -> MeetingProgressParams {
        MeetingProgressParams {
            meeting: self.meeting.into(),
            product: self.product.into(),
            stage: stage.into(),
            phase: phase.into(),
            ..MeetingProgressParams::default()
        }
    }

    fn send(&self, stage: &str, phase: &str) {
        self.sink.progress(self.event(stage, phase));
    }

    fn warn(&self, text: &str) {
        self.sink.progress(MeetingProgressParams {
            note: Some(text.into()),
            ..self.event("warning", "end")
        });
    }

    fn call_end(
        &self,
        stage: &str,
        role: &str,
        started: Instant,
        result: &std::result::Result<&ClaudeResult, &String>,
    ) {
        let mut event = MeetingProgressParams {
            role: Some(role.into()),
            seconds: Some(seconds(started)),
            ..self.event(stage, "end")
        };
        match result {
            Ok(r) => {
                event.cost_usd = Some(r.usage.cost_usd);
                event.input_tokens = Some(total_input_tokens(&r.usage));
            }
            Err(e) => event.error = Some((*e).clone()),
        }
        self.sink.progress(event);
    }

    fn run(&self, env: &Env, role: &str, call: &Call) -> std::result::Result<ClaudeResult, String> {
        claude(
            env.claude,
            call,
            &|line| self.sink.line(role, line),
            env.cancel,
        )
    }
}

/// The index, the code graph and the snapshot, or the frozen snapshot.
fn context_of(
    env: &Env,
    emit: &Emit,
    product: &Product,
    scope: Option<&str>,
    frozen: Option<&str>,
) -> Result<(String, Vec<String>)> {
    let mut code_graph_projects = Vec::new();
    let snap = match frozen {
        Some(id) => reading(env.store, |c| {
            read::snapshot(c, &product.id, id)?.ok_or_else(|| {
                Error::Agency(crate::Error::NotFound(format!(
                    "Minutes \"{id}\" have no frozen snapshot in {}",
                    product.id
                )))
            })
        })?,
        None => {
            emit.send("index", "start");
            index_product(env.store, product, false)?;
            for path in product.paths.iter().filter(|p| Path::new(p).exists()) {
                let project = code_graph::reindex(Path::new(path), &mut |line| emit.warn(line));
                code_graph_projects.extend(project);
            }
            let snap = reading(env.store, |c| {
                snapshot::snapshot(c, product, scope).map_err(Error::Agency)
            })?;
            emit.send("index", "end");
            snap
        }
    };
    Ok((snap, code_graph_projects))
}

fn usage_json(rows: &[MeetingUsage]) -> String {
    let list: Vec<Value> = rows
        .iter()
        .map(|u| {
            json!({
                "role": u.role,
                "model": u.model,
                "input_tokens": u.input_tokens,
                "output_tokens": u.output_tokens,
                "cache_read": u.cache_read,
                "cache_write": u.cache_write,
                "cost_usd": js_number(u.cost_usd),
                "duration_ms": u.duration_ms,
            })
        })
        .collect();
    Value::Array(list).to_string()
}

/// The check result of minutes in place: `{...result, first_failures, repaired}`.
fn verify(
    env: &Env,
    emit: &Emit,
    m: &mut Value,
    roles: &[String],
    product: &Product,
    brief: Option<&Value>,
) -> Result<()> {
    emit.send("check", "start");
    let result = reading(env.store, |c| Ok(check(c, m, product, Some(roles), brief)))?;
    let first = m["check"]["first_failures"].clone();
    let repaired = m["check"]["repaired"].clone();
    m["check"] = json!({
        "passed": result.passed,
        "failures": result.failures,
        "first_failures": first,
        "repaired": repaired,
    });
    m["status"] = json!(if result.passed { "valid" } else { "invalid" });
    let note = if result.passed {
        "passed".to_string()
    } else {
        format!(
            "{} failures: {}",
            result.failures.len(),
            result.failures.join(" | ")
        )
    };
    env.sink.progress(MeetingProgressParams {
        note: Some(note),
        ..emit.event("check", "end")
    });
    Ok(())
}

fn strings(v: &Value) -> Vec<String> {
    v.as_array()
        .map(|list| {
            list.iter()
                .filter_map(Value::as_str)
                .map(String::from)
                .collect()
        })
        .unwrap_or_default()
}

fn meeting_inner(env: &Env, o: &MeetingOptions, emit: &Emit) -> Result<MeetingOutcome> {
    let product = env.registry.product(&o.product)?.clone();
    let owner = env.registry.owner.clone();
    let scope = o.scope.as_deref();
    let archived = reading(env.store, |c| {
        Ok(read::archived(c, &product.id)?
            .iter()
            .any(|r| r.kind == "product"))
    })?;
    if archived {
        return Err(Error::Conflict(format!(
            "Product {0} is archived. Run agentsi restore --product {0} to bring it back.",
            product.id
        )));
    }
    let role_names = if o.roles.is_empty() {
        product.roles.clone()
    } else {
        o.roles.clone()
    };
    let mut warnings = Vec::new();
    let (agents, pm) = reading(env.store, |c| {
        prepare_agents(c, env.root, &product.id, &role_names, &mut warnings)
    })?;
    for w in &warnings {
        emit.warn(w);
    }
    let cwd = product_cwd(&product, scope)?;
    let brief = reading(env.store, |c| Ok(read::brief(c, &product.id)?))?;
    let (snap, code_graph_projects) = context_of(env, emit, &product, scope, o.frozen.as_deref())?;

    let usage: Mutex<Vec<MeetingUsage>> = Mutex::new(Vec::new());
    let calls: Vec<Call> = agents
        .iter()
        .map(|a| {
            let ctx = RoleContext {
                question: &o.question,
                snapshot: &snap,
                memory: &a.memory,
                scope,
                cwd: &cwd,
                brief: brief.as_ref(),
                owner: &owner,
                code_graph: &code_graph_projects,
            };
            role_call(&a.def, &ctx, true)
        })
        .collect();
    for a in &agents {
        env.sink.progress(MeetingProgressParams {
            role: Some(a.def.id.clone()),
            ..emit.event("role", "start")
        });
    }
    let answers: Vec<(Value, AgentAnswerLog)> = std::thread::scope(|scope| {
        let handles: Vec<_> = agents
            .iter()
            .zip(&calls)
            .map(|(agent, call)| {
                let usage = &usage;
                scope.spawn(move || {
                    let id = &agent.def.id;
                    let started = Instant::now();
                    let mut log = AgentAnswerLog {
                        id: id.clone(),
                        prompt: agent.def.body.clone(),
                        memory: agent.memory.clone(),
                        model: agent.def.model.clone(),
                        error: None,
                        duration_ms: 0,
                        unresolved_sources: Vec::new(),
                    };
                    let result = emit.run(env, id, call);
                    let answer = match &result {
                        Ok(r) => {
                            usage
                                .lock()
                                .unwrap_or_else(|p| p.into_inner())
                                .push(usage_of(id, r));
                            log.model = r.usage.model.clone();
                            to_role_answer(&r.output, &r.usage.model)
                        }
                        Err(e) => {
                            log.error = Some(e.clone());
                            empty_answer(&agent.def.model, e)
                        }
                    };
                    emit.call_end("role", id, started, &result.as_ref());
                    log.duration_ms = started.elapsed().as_millis() as u64;
                    (answer, log)
                })
            })
            .collect();
        handles
            .into_iter()
            .map(|h| h.join().expect("a role thread does not panic"))
            .collect()
    });
    if env.cancel.is_set() {
        return Err(Error::Conflict(CANCELED.into()));
    }
    let mut ordered: Vec<(String, Value)> = Vec::new();
    let mut logs: Vec<AgentAnswerLog> = Vec::new();
    for (answer, log) in answers {
        ordered.push((log.id.clone(), answer));
        logs.push(log);
    }

    reading(env.store, |c| {
        let ctx = source_context(c, &product);
        for ((_, answer), log) in ordered.iter().zip(&mut logs) {
            let priorities = answer["priorities"]
                .as_array()
                .map_or(&[][..], Vec::as_slice);
            for p in priorities {
                for s in p["sources"].as_array().map_or(&[][..], Vec::as_slice) {
                    if !resolve_source(s, &ctx) {
                        log.unresolved_sources.push(s.clone());
                    }
                }
            }
        }
        Ok(())
    })?;

    let compact = compact_snapshot(&snap);
    let synthesis_ctx = SynthesisContext {
        question: &o.question,
        answers: &ordered,
        compact: &compact,
        cwd: &cwd,
        brief: brief.as_ref(),
        owner: &owner,
    };
    let synth = |role: &str, repair: Option<&Repair>| -> Option<Value> {
        let started = Instant::now();
        env.sink.progress(MeetingProgressParams {
            role: Some(role.into()),
            note: repair.map(|_| "repair".to_string()),
            ..emit.event("synthesis", "start")
        });
        let result = emit.run(env, role, &synthesis_call(&pm, &synthesis_ctx, repair));
        emit.call_end("synthesis", role, started, &result.as_ref());
        let r = result.ok()?;
        usage
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .push(usage_of(role, &r));
        Some(r.output)
    };

    let now = clock::now_ms();
    let meta = MinutesMeta {
        id: new_minutes_id(&o.question, now),
        product: product.id.clone(),
        scope: o.scope.clone(),
        question: o.question.clone(),
        created_at: clock::iso(now),
    };

    let mut synthesis = synth("synthesis", None);
    if env.cancel.is_set() {
        return Err(Error::Conflict(CANCELED.into()));
    }
    let mut minutes = build_minutes(&meta, &ordered, synthesis.as_ref(), false, &[]);
    verify(
        env,
        emit,
        &mut minutes,
        &role_names,
        &product,
        brief.as_ref(),
    )?;
    minutes["check"]["first_failures"] = minutes["check"]["failures"].clone();
    let passed = minutes["check"]["passed"] == json!(true);
    if let Some(previous) = synthesis.take().filter(|_| !passed) {
        let failures = strings(&minutes["check"]["failures"]);
        let repair = Repair {
            previous: &previous,
            failures: &failures,
        };
        let repaired = synth("synthesis-repair", Some(&repair));
        if env.cancel.is_set() {
            return Err(Error::Conflict(CANCELED.into()));
        }
        match repaired {
            Some(output) => {
                minutes = build_minutes(&meta, &ordered, Some(&output), true, &failures);
                verify(
                    env,
                    emit,
                    &mut minutes,
                    &role_names,
                    &product,
                    brief.as_ref(),
                )?;
            }
            None => minutes["check"]["repaired"] = json!(true),
        }
    }

    // The check can take long, and a canceled meeting writes nothing.
    if env.cancel.is_set() {
        return Err(Error::Conflict(CANCELED.into()));
    }
    let usage = usage.into_inner().unwrap_or_else(|p| p.into_inner());
    let brief_hash = brief.as_ref().map(|b| sha256(&b.to_string()));
    let minutes = store_meeting(env.store, minutes, &usage, logs, &ordered, snap, brief_hash)?;

    if o.frozen.is_none()
        && let Err(e) = index_product(env.store, &product, false)
    {
        emit.warn(&format!("the index was not updated: {e}"));
    }
    let total_cost_usd = usage.iter().fold(0.0, |n, u| n + u.cost_usd);
    Ok(MeetingOutcome {
        minutes,
        usage,
        total_cost_usd,
    })
}

/// Claims the id, then writes the memory, the minutes, the log line and the
/// snapshot in one transaction. Returns the minutes with their final id.
fn store_meeting(
    store: &Store,
    mut minutes: Value,
    usage: &[MeetingUsage],
    logs: Vec<AgentAnswerLog>,
    answers: &[(String, Value)],
    snap: String,
    brief_hash: Option<String>,
) -> Result<Value> {
    let rows = usage.to_vec();
    let notes: Vec<(String, Vec<String>)> = answers
        .iter()
        .map(|(role, a)| (role.clone(), strings(&a["memory_notes"])))
        .collect();
    Ok(store.write(move |tx| {
        let text = |key: &str| minutes[key].as_str().unwrap_or_default().to_string();
        let (base, product, question, status, created_at) = (
            text("id"),
            text("product"),
            text("question"),
            text("status"),
            text("created_at"),
        );
        let scope = minutes["scope"].as_str().map(String::from);
        let id = write::reserve_minutes_id(
            tx,
            &MinutesHead {
                id: &base,
                product: &product,
                scope: scope.as_deref(),
                question: &question,
                status: &status,
                created_at: &created_at,
            },
        )?;
        minutes["id"] = json!(id);
        let entry = build_log_entry(&minutes, &rows, &logs, &snap, brief_hash.as_deref());
        for (role, role_notes) in &notes {
            write::append_facts(tx, &product, role, role_notes, &id, &created_at)?;
        }
        let writes: Vec<UsageWrite> = rows
            .iter()
            .map(|u| UsageWrite {
                role: &u.role,
                model: &u.model,
                input_tokens: u.input_tokens,
                output_tokens: u.output_tokens,
                cache_read: u.cache_read,
                cache_write: u.cache_write,
                cost_usd: u.cost_usd,
                duration_ms: u.duration_ms,
            })
            .collect();
        write::store_minutes(tx, &minutes, &writes, &usage_json(&rows))?;
        write::write_meeting_log(tx, &product, &id, &entry, &snap)?;
        Ok(minutes)
    })?)
}

/// `runMeeting`. The events go to `env.sink`. The last one is `done`.
pub fn run_meeting(env: &Env, o: &MeetingOptions) -> Result<MeetingOutcome> {
    let emit = Emit {
        meeting: &o.meeting,
        product: &o.product,
        sink: env.sink,
    };
    env.sink.progress(MeetingProgressParams {
        note: Some(o.question.clone()),
        ..emit.event("start", "start")
    });
    let result = meeting_inner(env, o, &emit);
    let mut done = emit.event("done", "end");
    match &result {
        Ok(outcome) => {
            done.note = outcome.minutes["id"].as_str().map(String::from);
            done.cost_usd = Some(outcome.total_cost_usd);
        }
        Err(e) => done.error = Some(e.to_string()),
    }
    env.sink.progress(done);
    result
}

/// `askRole`: one agent answers in plain text. It writes no minutes.
pub fn ask_role(
    env: &Env,
    product_id: &str,
    scope: Option<&str>,
    role: &str,
    question: &str,
) -> Result<AskOutcome> {
    let product = env.registry.product(product_id)?.clone();
    let mut warnings = Vec::new();
    let (def, memory) = reading(env.store, |c| {
        let overlays = roster::read_overlays(c, &product.id).map_err(Error::Agency)?;
        let def = load_role(
            role,
            &product.id,
            &RoleFiles::at(env.root),
            &overlays,
            &mut warnings,
        )
        .map_err(Error::Invalid)?;
        let memory = memory_for_prompt(c, &product.id, &def.id)?;
        Ok((def, memory))
    })?;
    let emit = Emit {
        meeting: "",
        product: &product.id,
        sink: env.sink,
    };
    let cwd = product_cwd(&product, scope)?;
    index_product(env.store, &product, false)?;
    let (snap, brief) = reading(env.store, |c| {
        Ok((
            snapshot::snapshot(c, &product, scope).map_err(Error::Agency)?,
            read::brief(c, &product.id)?,
        ))
    })?;
    let owner = env.registry.owner.clone();
    let ctx = RoleContext {
        question,
        snapshot: &snap,
        memory: &memory,
        scope,
        cwd: &cwd,
        brief: brief.as_ref(),
        owner: &owner,
        code_graph: &[],
    };
    let r = emit
        .run(env, &def.id, &role_call(&def, &ctx, false))
        .map_err(|e| {
            if env.cancel.is_set() {
                Error::Conflict(CANCELED.into())
            } else {
                Error::Failed(e)
            }
        })?;
    let answer = match &r.output {
        Value::String(s) => s.trim().to_string(),
        other => other.to_string(),
    };
    Ok(AskOutcome {
        answer,
        usage: usage_of(&def.id, &r),
        warnings,
    })
}

#[cfg(test)]
mod tests;
