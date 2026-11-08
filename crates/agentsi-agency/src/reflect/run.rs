//! `reflect` and `revertReflection` of `reflect.ts`.
//!
//! The model calls run first and read only. One transaction then writes the memory,
//! the entry and the decision. A failure or a cancel before that write leaves no row.

use super::curate::{Applied, Deltas, apply_deltas, curation_call, validate_deltas};
use super::decision::record_decision;
use super::memory::{active, retire};
use super::proposal::{ProposeInput, ProposeResult, decision_text, propose_for_ready};
use super::replay::Caller;
use super::signals::{collect_signals, consumed_by, is_consumed};
use super::{Error, Result, list, text, unified_diff};
use crate::audit;
use crate::clock;
use crate::knowledge::memory_text::{is_agent_id, render_for_prompt};
use crate::meeting::Cancel;
use crate::meeting::claude::{ClaudeResult, claude};
use crate::meeting::minutes::local_stamp;
use crate::prompt_inputs::brief_xml;
use crate::registry::{Product, Registry};
use agentsi_proto::agency_api::ReadyAgent;
use agentsi_proto::reflect_api::{
    MemoryChange, Proposal, ProposalCandidate, ReflectionEntry, Signal,
};
use agentsi_store::Store;
use agentsi_store::read::{self, MemoryItem};
use agentsi_store::reflection::{write_memory, write_reflections};
use rusqlite::{Connection, Transaction};
use serde_json::Value;
use std::collections::{BTreeMap, BTreeSet, HashSet};
use std::path::Path;

const CANCELED: &str = "The reflection was canceled. Nothing was saved.";

/// What a reflection runs on.
pub struct Env<'a> {
    pub store: &'a Store,
    pub root: &'a Path,
    pub registry: &'a Registry,
    pub cancel: &'a Cancel,
    /// The program to run: `claude`, or a fake one in a test.
    pub claude: &'a str,
}

#[derive(Debug, Clone, Default)]
pub struct Options {
    /// Curates memory only, and makes no prompt proposal.
    pub memory_only: bool,
    /// The cap of the proposal and its replay. By default `REPLAY_BUDGET_USD`.
    pub budget_usd: Option<f64>,
}

#[derive(Debug, Default)]
pub struct ReflectOutcome {
    /// `None` when no agent had signals or new items and no proposal was made, so
    /// nothing was written.
    pub entry: Option<ReflectionEntry>,
    /// Agents past the proposal threshold, and what came of each.
    pub candidates: Vec<ProposalCandidate>,
    /// What TypeScript prints on stderr.
    pub warnings: Vec<String>,
}

/// What the reflection reads before its calls.
struct Evidence {
    reflections: Vec<Value>,
    log: Vec<Value>,
    minutes: Vec<Value>,
    signals: Vec<Signal>,
    /// The agents with memory items that no reflection processed.
    new_sources: Vec<(String, Vec<String>)>,
    ready: Vec<ReadyAgent>,
    brief_xml: Option<String>,
}

fn read_minutes(c: &Connection, product: &str) -> agentsi_store::Result<Vec<Value>> {
    let mut stmt = c.prepare_cached(
        "select data from minutes where product = ?1 and data is not null order by id",
    )?;
    let rows: Vec<String> = stmt
        .query_map([product], |r| r.get(0))?
        .collect::<std::result::Result<_, _>>()?;
    Ok(rows
        .iter()
        .filter_map(|t| serde_json::from_str(t).ok())
        .collect())
}

fn read_evidence(c: &Connection, root: &Path, product: &Product, now_ms: i64) -> Result<Evidence> {
    let p = &product.id;
    let reflections = read::reflections(c, p)?;
    let log = read::meeting_log(c, p)?;
    let minutes = read_minutes(c, p)?;
    let signals = collect_signals(&log, &minutes, &reflections);
    let consumed = consumed_by(&reflections);
    let reflection_ids: HashSet<&str> = reflections.iter().map(|e| text(e, "id")).collect();
    let mut new_sources = Vec::new();
    for agent in read::memory_agents(c, p)? {
        let items = read::memory(c, p, &agent)?;
        let sources: Vec<String> = active(&items)
            .into_iter()
            .filter(|i| {
                !reflection_ids.contains(i.source.as_str())
                    && !is_consumed(&consumed, &i.source, &i.created_at)
            })
            .map(|i| i.source.clone())
            .collect();
        new_sources.push((agent, sources));
    }
    let ready = audit::audit(c, root, product, now_ms)?.ready_to_reflect;
    // A brief that does not read counts as none, as `readBriefXml` does.
    let brief_xml = read::brief(c, p).ok().flatten().map(|b| brief_xml(&b));
    Ok(Evidence {
        reflections,
        log,
        minutes,
        signals,
        new_sources,
        ready,
        brief_xml,
    })
}

/// The ids that this reflection processes: its log lines and minutes that no earlier
/// reflection consumed, and the sources of the new items.
fn newly_consumed(e: &Evidence) -> Vec<String> {
    let consumed = consumed_by(&e.reflections);
    let mut ids: BTreeSet<String> = BTreeSet::new();
    for line in &e.log {
        let id = text(line, "minutes_id");
        if !is_consumed(&consumed, id, text(line, "created_at")) {
            ids.insert(id.into());
        }
    }
    for m in &e.minutes {
        let id = text(m, "id");
        if !is_consumed(&consumed, id, text(m, "created_at")) {
            ids.insert(id.into());
        }
    }
    ids.extend(e.new_sources.iter().flat_map(|(_, s)| s.iter().cloned()));
    ids.into_iter().collect()
}

fn agents_of(e: &Evidence) -> Vec<String> {
    // A priority with no owner gives a signal, but no agent can learn from it.
    let owned = e.signals.iter().filter(|s| !s.agent.is_empty());
    let mut agents: BTreeSet<String> = owned.map(|s| s.agent.clone()).collect();
    agents.extend(
        e.new_sources
            .iter()
            .filter(|(_, sources)| !sources.is_empty())
            .map(|(a, _)| a.clone()),
    );
    agents.into_iter().collect()
}

fn new_reflection_id(now_ms: i64, entries: &[Value]) -> String {
    let taken: HashSet<&str> = entries.iter().map(|e| text(e, "id")).collect();
    let base = format!("r-{}", local_stamp(now_ms));
    let mut id = base.clone();
    let mut n = 2;
    while taken.contains(id.as_str()) {
        id = format!("{base}-{n}");
        n += 1;
    }
    id
}

fn diff_of(before: &[MemoryItem], after: &[MemoryItem], label: &str) -> Result<String> {
    let (before, after) = (
        render_for_prompt(before, None),
        render_for_prompt(after, None),
    );
    unified_diff(&before, &after, label).map_err(|e| Error::Failed(e.to_string()))
}

/// The validated deltas of one agent.
struct Curated {
    agent: String,
    items: Vec<MemoryItem>,
    deltas: Deltas,
    dropped: Vec<String>,
}

fn read_items(store: &Store, product: &str, agent: &str) -> Result<Vec<MemoryItem>> {
    Ok(store.read(|c| read::memory(c, product, agent))?)
}

/// One call for each agent, at the same time. Returns the deltas in the order of
/// the agents, and what the calls cost.
fn curate(
    env: &Env,
    product: &str,
    agents: &[String],
    e: &Evidence,
    call: &Caller,
) -> Result<(Vec<Curated>, f64)> {
    let mut inputs = Vec::new();
    for agent in agents {
        // An owner role of a priority is free text, and memory takes only agent ids.
        if !is_agent_id(agent) {
            return Err(Error::Failed(format!("Invalid agent id \"{agent}\"")));
        }
        let items = read_items(env.store, product, agent)?;
        let own: Vec<&Signal> = e.signals.iter().filter(|s| s.agent == *agent).collect();
        inputs.push((agent, items, own));
    }
    let results: Vec<std::result::Result<ClaudeResult, String>> = std::thread::scope(|scope| {
        let handles: Vec<_> = inputs
            .iter()
            .map(|(agent, items, own)| {
                let request = curation_call(agent, items, own, e.brief_xml.as_deref(), env.root);
                scope.spawn(move || call(&request))
            })
            .collect();
        handles
            .into_iter()
            .map(|h| h.join().expect("a curation thread does not panic"))
            .collect()
    });
    if env.cancel.is_set() {
        return Err(Error::Conflict(CANCELED.into()));
    }
    let mut curated = Vec::new();
    let mut cost = 0.0;
    for ((agent, items, own), result) in inputs.iter().zip(results) {
        let done = result.map_err(Error::Failed)?;
        cost += done.usage.cost_usd;
        let (deltas, dropped) = validate_deltas(&done.output, items, own);
        curated.push(Curated {
            agent: (*agent).clone(),
            items: items.clone(),
            deltas,
            dropped,
        });
    }
    Ok((curated, cost))
}

/// What the write of a reflection needs.
struct WriteJob {
    product: String,
    now_ms: i64,
    signals: Vec<Signal>,
    curated: Vec<Curated>,
    proposed: ProposeResult,
    consumed: Vec<String>,
    curation_cost: f64,
}

/// The decision of a pending proposal. When the insert fails, the proposal is
/// discarded with the reason, and the curated memory is still kept.
fn attach_decision(tx: &Transaction, entry: &mut ReflectionEntry, warnings: &mut Vec<String>) {
    let Some(proposal) = entry.proposal.as_mut() else {
        return;
    };
    if proposal.status != "pending" {
        return;
    }
    let (body, why) = decision_text(proposal, &entry.id);
    match record_decision(tx, &entry.product, &entry.id, &body, &why) {
        Ok(id) => proposal.decision_id = Some(id),
        Err(e) => discard(proposal, e.to_string(), warnings),
    }
}

fn discard(proposal: &mut Proposal, error: String, warnings: &mut Vec<String>) {
    let reason = format!("no se pudo registrar la decisión: {error}");
    warnings.push(format!("reflect {}: {reason}", proposal.agent));
    proposal.status = "discarded".into();
    proposal.replay.aborted = Some(reason);
}

/// A meeting may add facts during the calls, so the deltas go on a fresh read.
fn write(tx: &Transaction, w: WriteJob) -> Result<(ReflectionEntry, Vec<String>)> {
    let p = &w.product;
    // A revert may have taken an id during the calls, so the id comes from a fresh read too.
    let mut entries = read::reflections(tx, p)?;
    let id = new_reflection_id(w.now_ms, &entries);
    let now_iso = clock::iso(w.now_ms);
    let mut warnings = Vec::new();
    let mut memory: Vec<MemoryChange> = Vec::new();
    for c in w.curated {
        let before = read::memory(tx, p, &c.agent)?;
        let next: Applied = apply_deltas(&before, &c.deltas, &id, &now_iso)?;
        for reason in &next.skipped {
            warnings.push(format!("reflect {}: skipped {reason}", c.agent));
        }
        if next.items != before {
            write_memory(tx, p, &c.agent, &next.items)?;
        }
        let diff = diff_of(&before, &next.items, &format!("{p}/memory/{}", c.agent))?;
        let mut dropped = c.dropped;
        dropped.extend(next.skipped);
        memory.push(MemoryChange {
            agent: c.agent,
            added: next.added,
            retired: next.retired,
            helpful: next.helpful,
            harmful: next.harmful,
            restored: None,
            dropped,
            diff,
        });
    }
    let candidates = w.proposed.candidates;
    let mut entry = ReflectionEntry {
        id,
        product: p.clone(),
        created_at: now_iso,
        trigger: "manual".into(),
        reverts: None,
        signals: w.signals,
        memory,
        proposal: w.proposed.proposal,
        candidates: (!candidates.is_empty()).then_some(candidates),
        consumed: Some(w.consumed),
        status: "applied".into(),
        commit: None,
        cost_usd: Some(w.curation_cost + w.proposed.cost_usd),
    };
    attach_decision(tx, &mut entry, &mut warnings);
    entries.push(serde_json::to_value(&entry).map_err(|e| Error::Failed(e.to_string()))?);
    write_reflections(tx, p, &entries)?;
    Ok((entry, warnings))
}

/// Curates the memory of each agent from the signals since the last reflection, then,
/// unless `memory_only`, proposes one prompt change that replay judges.
pub fn reflect(env: &Env, product_id: &str, o: &Options) -> Result<ReflectOutcome> {
    let product = env.registry.product(product_id)?.clone();
    let p = product.id.clone();
    let now_ms = clock::now_ms();
    let e = env
        .store
        .read(|c| Ok(read_evidence(c, env.root, &product, now_ms)))??;

    let agents = agents_of(&e);
    let proposing = !o.memory_only && !e.ready.is_empty();
    if agents.is_empty() && !proposing {
        return Ok(ReflectOutcome::default());
    }

    let call = |c: &crate::story::claude::Call| claude(env.claude, c, &|_| {}, env.cancel);
    let (curated, curation_cost) = curate(env, &p, &agents, &e, &call)?;
    let now_iso = clock::iso(now_ms);

    let mut warnings: Vec<String> = Vec::new();
    for c in &curated {
        for reason in &c.dropped {
            warnings.push(format!("reflect {}: dropped {reason}", c.agent));
        }
    }

    // The memory that the proposal and its replay see. The write applies the deltas again.
    let draft_id = new_reflection_id(now_ms, &e.reflections);
    let mut with_deltas: BTreeMap<&str, Vec<MemoryItem>> = BTreeMap::new();
    for c in &curated {
        let applied = apply_deltas(&c.items, &c.deltas, &draft_id, &now_iso)?;
        with_deltas.insert(&c.agent, applied.items);
    }
    let items = |agent: &str| -> std::result::Result<Vec<MemoryItem>, String> {
        match with_deltas.get(agent) {
            Some(items) => Ok(items.clone()),
            None => read_items(env.store, &p, agent).map_err(|e| e.to_string()),
        }
    };
    let proposed = if proposing {
        propose_for_ready(&ProposeInput {
            store: env.store,
            product: &product,
            owner: &env.registry.owner,
            root: env.root,
            ready: &e.ready,
            log: &e.log,
            minutes: &e.minutes,
            reflections: &e.reflections,
            items: &items,
            call: &call,
            budget_usd: o.budget_usd,
        })
        .map_err(Error::Failed)?
    } else {
        ProposeResult::default()
    };
    if env.cancel.is_set() {
        return Err(Error::Conflict(CANCELED.into()));
    }
    warnings.extend(proposed.warnings.iter().cloned());
    if agents.is_empty() && proposed.proposal.is_none() {
        return Ok(ReflectOutcome {
            entry: None,
            candidates: proposed.candidates,
            warnings,
        });
    }

    let candidates = proposed.candidates.clone();
    let job = WriteJob {
        product: p,
        now_ms,
        signals: e.signals.clone(),
        curated,
        proposed,
        consumed: newly_consumed(&e),
        curation_cost,
    };
    let (entry, more) = env.store.try_write(move |tx| write(tx, job))??;
    warnings.extend(more);
    Ok(ReflectOutcome {
        entry: Some(entry),
        candidates,
        warnings,
    })
}

fn strings(change: &Value, key: &str) -> Vec<String> {
    list(change, key)
        .iter()
        .filter_map(|v| v.as_str().map(String::from))
        .collect()
}

/// The inverse of one change on the items of today, so later counters survive. Its
/// additions are retired and marked harmful, its retirements come back, its counter
/// bumps are undone.
fn invert(
    change: &Value,
    before: &[MemoryItem],
    target_created: &str,
    revert_id: &str,
    at: &str,
) -> (Vec<MemoryItem>, MemoryChange) {
    let mut items = before.to_vec();
    let (mut retired, mut harmful, mut restored) = (Vec::new(), Vec::new(), Vec::new());
    for id in strings(change, "added") {
        let Some(i) = items.iter().position(|x| x.id == id) else {
            continue;
        };
        if items[i].retired_at.is_none() {
            retire(&mut items, &id, &format!("reverted by {revert_id}"), at);
            retired.push(id.clone());
        }
        items[i].harmful += 1;
        harmful.push(id);
    }
    for id in strings(change, "retired") {
        let Some(item) = items.iter_mut().find(|x| x.id == id) else {
            continue;
        };
        if item.retired_at.as_deref() != Some(target_created) {
            continue;
        }
        item.retired_at = None;
        item.retired_why = None;
        restored.push(id);
    }
    for id in strings(change, "helpful") {
        if let Some(item) = items.iter_mut().find(|x| x.id == id) {
            item.helpful = (item.helpful - 1).max(0);
        }
    }
    for id in strings(change, "harmful") {
        if let Some(item) = items.iter_mut().find(|x| x.id == id) {
            item.harmful = (item.harmful - 1).max(0);
        }
    }
    let done = MemoryChange {
        agent: text(change, "agent").into(),
        retired,
        harmful,
        restored: Some(restored),
        ..MemoryChange::default()
    };
    (items, done)
}

fn revert_in(tx: &Transaction, p: &str, id: &str, now_ms: i64) -> Result<ReflectionEntry> {
    let mut entries = read::reflections(tx, p)?;
    let Some(index) = entries.iter().position(|e| text(e, "id") == id) else {
        return Err(Error::NotFound(format!(
            "Reflection \"{id}\" not found in {p}."
        )));
    };
    let target = entries[index].clone();
    if !text(&target, "reverts").is_empty() {
        return Err(Error::Conflict(format!(
            "Reflection \"{id}\" is a revert; it cannot be reverted."
        )));
    }
    if text(&target, "status") == "reverted" || entries.iter().any(|e| text(e, "reverts") == id) {
        return Err(Error::Conflict(format!(
            "Reflection \"{id}\" is already reverted."
        )));
    }
    let changed = list(&target, "memory")
        .iter()
        .any(|m| !text(m, "diff").trim().is_empty());
    if text(&target, "status") != "applied" || !changed {
        return Err(Error::Conflict(format!(
            "Reflection \"{id}\" changed no memory; there is nothing to revert."
        )));
    }

    let revert_id = new_reflection_id(now_ms, &entries);
    let at = clock::iso(now_ms);
    let mut memory = Vec::new();
    for change in list(&target, "memory") {
        let agent = text(change, "agent");
        let before = read::memory(tx, p, agent)?;
        let created = text(&target, "created_at");
        let (items, mut done) = invert(change, &before, created, &revert_id, &at);
        if items == before {
            continue;
        }
        write_memory(tx, p, agent, &items)?;
        done.diff = diff_of(&before, &items, &format!("{p}/memory/{agent}"))?;
        memory.push(done);
    }

    let entry = ReflectionEntry {
        id: revert_id,
        product: p.into(),
        created_at: at,
        trigger: "manual".into(),
        reverts: Some(id.into()),
        memory,
        status: "applied".into(),
        ..ReflectionEntry::default()
    };
    entries[index]["status"] = "reverted".into();
    entries.push(serde_json::to_value(&entry).map_err(|e| Error::Failed(e.to_string()))?);
    write_reflections(tx, p, &entries)?;
    Ok(entry)
}

/// Applies the inverse of the memory changes of one entry, and appends a revert line.
pub fn revert(store: &Store, product: &str, id: &str, now_ms: i64) -> Result<ReflectionEntry> {
    let (p, id) = (product.to_string(), id.to_string());
    store.try_write(move |tx| revert_in(tx, &p, &id, now_ms))?
}

#[cfg(test)]
mod tests;
