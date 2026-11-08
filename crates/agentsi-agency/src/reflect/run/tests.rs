//! The flow of a reflection with a fake `claude`, on a real store. The golden cases
//! of `agentsi-cli` compare the same flow with the TypeScript code. Here the proofs
//! are row comparisons: `dump` prints each table of the database.

use super::*;
use crate::marks::decide::decide_at;
use crate::meeting::log::sha256;
use crate::reflect::proposal::{ProposeInput, propose_for_ready};
use agentsi_proto::marks_api::{Outcome, Verdict};
use rusqlite::types::ValueRef;
use serde_json::json;
use std::collections::BTreeMap;
use std::os::unix::fs::PermissionsExt;
use std::path::PathBuf;

/// Answers from the files of its own folder, and counts its calls.
const FAKE: &str = r##"#!/bin/sh
here=$(dirname "$0")
prompt=$(cat)
echo x >> "$here/calls"
ok() { printf '{"type":"result","subtype":"success","total_cost_usd":%s,"duration_ms":10,"structured_output":%s}\n' "$1" "$2"; }
case "$prompt" in
*"<effective_prompt>"*)
  [ -f "$here/hold-proposal" ] && { : > "$here/held"; while [ -f "$here/hold-proposal" ]; do sleep 0.02; done; }
  ok 0.25 "$(cat "$here/proposal.json")" ;;
*"<agent>"*)
  [ -f "$here/fail" ] && { echo boom >&2; exit 3; }
  [ -f "$here/hold" ] && { : > "$here/held"; while [ -f "$here/hold" ]; do sleep 0.02; done; }
  agent=$(printf '%s\n' "$prompt" | sed -n 's/^<agent>\(.*\)<\/agent>$/\1/p')
  file="$here/curate-$agent.json"
  [ -f "$file" ] || file="$here/curate.json"
  ok 0.125 "$(cat "$file")" ;;
*"You are the dev of"*) ok 0.5 "$(cat "$here/answer.json")" ;;
*) ok 0.5 "$(cat "$here/synthesis.json")" ;;
esac
"##;

const NOTHING: &str = r#"{"add":[],"retire":[],"helpful":[],"harmful":[]}"#;
const GOOD_ANSWER: &str = r#"{"answer":"Better.","priorities":[{"title":"T","why":"W","sources":[{"type":"path","ref":"docs/vision.md"}]}],"concerns":[],"memory_notes":["Never store this."]}"#;
const BAD_ANSWER: &str = r#"{"answer":"Worse.","priorities":[{"title":"T","why":"W","sources":[{"type":"path","ref":"docs/a.md"},{"type":"path","ref":"docs/b.md"},{"type":"path","ref":"docs/c.md"}]}],"concerns":[],"memory_notes":[]}"#;
const SYNTHESIS: &str = r#"{"priorities":[{"title":"T","why":"W","sources":[{"type":"path","ref":"docs/vision.md"}],"owner_role":"dev"}],"disagreements":[],"decisions":[],"script":[]}"#;
const PROPOSAL: &str = r#"{"new_overlay_body":"Cite only files you read.","rationale":"It cited files it did not read.","expected_effect":"Fewer unresolved sources."}"#;

struct World {
    dir: tempfile::TempDir,
    store: Store,
    registry: Registry,
    program: String,
}

impl World {
    fn root(&self) -> PathBuf {
        self.dir.path().join("root")
    }

    fn bin(&self) -> PathBuf {
        self.dir.path().join("bin")
    }

    fn reply(&self, file: &str, text: &str) {
        std::fs::write(self.bin().join(file), text).unwrap();
    }

    fn calls(&self) -> usize {
        std::fs::read_to_string(self.bin().join("calls")).map_or(0, |t| t.lines().count())
    }

    fn reflect_with(&self, o: &Options, cancel: &Cancel) -> Result<ReflectOutcome> {
        let root = self.root();
        let env = Env {
            store: &self.store,
            root: &root,
            registry: &self.registry,
            cancel,
            claude: &self.program,
        };
        reflect(&env, "p", o)
    }

    fn reflect(&self) -> ReflectOutcome {
        self.reflect_with(&Options::default(), &Cancel::default())
            .unwrap()
    }

    fn items(&self, agent: &str) -> Vec<MemoryItem> {
        read_items(&self.store, "p", agent).unwrap()
    }

    fn sql(&self, sql: &'static str) {
        self.store
            .write(move |tx| {
                tx.execute_batch(sql)?;
                Ok(())
            })
            .unwrap();
    }

    /// Each row of each table.
    fn dump(&self) -> BTreeMap<String, Vec<String>> {
        let mut out = BTreeMap::new();
        self.store
            .read(|c| {
                let mut tables =
                    c.prepare("select name from sqlite_master where type = 'table' order by name")?;
                let names: Vec<String> = tables
                    .query_map([], |r| r.get(0))?
                    .collect::<std::result::Result<_, _>>()?;
                for name in names {
                    let mut stmt = c.prepare(&format!("select * from \"{name}\""))?;
                    let n = stmt.column_count();
                    let mut rows = stmt.query([])?;
                    let mut lines = Vec::new();
                    while let Some(r) = rows.next()? {
                        let cells: Vec<String> = (0..n)
                            .map(|i| match r.get_ref(i).unwrap() {
                                ValueRef::Null => "null".to_string(),
                                ValueRef::Integer(v) => v.to_string(),
                                ValueRef::Real(v) => v.to_string(),
                                ValueRef::Text(t) => String::from_utf8_lossy(t).into_owned(),
                                ValueRef::Blob(b) => format!("{b:?}"),
                            })
                            .collect();
                        lines.push(cells.join(" | "));
                    }
                    lines.sort();
                    out.insert(name, lines);
                }
                Ok(())
            })
            .unwrap();
        out
    }
}

fn world() -> World {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().join("root");
    let repo = dir.path().join("repo");
    let bin = dir.path().join("bin");
    std::fs::create_dir_all(root.join("plugin/agents")).unwrap();
    std::fs::create_dir_all(repo.join("docs")).unwrap();
    std::fs::create_dir_all(&bin).unwrap();
    std::fs::write(repo.join("docs/vision.md"), "v").unwrap();
    for role in ["pm", "dev", "qa"] {
        std::fs::write(
            root.join(format!("plugin/agents/{role}.md")),
            format!("---\nmodel: haiku\n---\nYou are {role}.\n"),
        )
        .unwrap();
    }
    let registry = Registry::parse(
        &json!({
            "owner": {"name": "Ada", "language": "English"},
            "products": [{"id": "p", "name": "P", "paths": [repo], "status": "active", "roles": ["pm", "dev"]}],
        })
        .to_string(),
    )
    .unwrap();
    let store = Store::open(&root.join("agentsi.db")).unwrap();
    let file = bin.join("claude");
    std::fs::write(&file, FAKE).unwrap();
    std::fs::set_permissions(&file, std::fs::Permissions::from_mode(0o755)).unwrap();
    let w = World {
        program: file.to_string_lossy().into_owned(),
        dir,
        store,
        registry,
    };
    w.reply("curate.json", NOTHING);
    w.reply("answer.json", GOOD_ANSWER);
    w.reply("synthesis.json", SYNTHESIS);
    w.reply("proposal.json", PROPOSAL);
    w
}

fn item(id: &str, text: &str) -> MemoryItem {
    MemoryItem {
        id: id.into(),
        text: text.into(),
        kind: "fact".into(),
        helpful: 0,
        harmful: 0,
        source: "seed".into(),
        created_at: "2026-10-01T00:00:00.000Z".into(),
        retired_at: None,
        retired_why: None,
    }
}

/// Memory of `dev`, and one minutes with a rejected mark, the evidence of a reflection.
fn seed_memory(w: &World) {
    let items = [item("m-a1", "Old note."), item("m-a2", "Keep this.")];
    w.store
        .write(move |tx| {
            write_memory(tx, "p", "dev", &items)?;
            let data = json!({
                "id": "2026-m1", "product": "p", "question": "q",
                "created_at": "2026-10-02T00:00:00.000Z", "status": "valid", "roles": {},
                "priorities": [{"title": "A", "owner_role": "dev", "mark": "rejected"}],
                "decisions": [],
            });
            tx.execute(
                "insert into minutes (id, product, question, status, created_at, data)
                 values ('2026-m1', 'p', 'q', 'valid', '2026-10-02T00:00:00.000Z', ?1)",
                [data.to_string()],
            )?;
            Ok(())
        })
        .unwrap();
}

const DELTAS: &str = r#"{"add":[{"text":"Cite  what you read.","kind":"lesson","evidence":["2026-m1"]},{"text":"Unproven.","kind":"lesson","evidence":["nope"]}],"retire":[{"id":"m-a1","why":"Stale."}],"helpful":["m-a2"],"harmful":[]}"#;

#[test]
fn a_reflection_applies_the_valid_deltas_and_drops_the_rest() {
    let w = world();
    seed_memory(&w);
    w.reply("curate-dev.json", DELTAS);
    let out = w.reflect();

    let e = out.entry.unwrap();
    assert_eq!(e.memory.len(), 1);
    let change = &e.memory[0];
    assert_eq!(change.agent, "dev");
    assert_eq!(change.retired, ["m-a1"]);
    assert_eq!(change.helpful, ["m-a2"]);
    assert_eq!(change.dropped, ["add \"Unproven.\": cites no signal"]);
    assert_eq!(
        out.warnings,
        ["reflect dev: dropped add \"Unproven.\": cites no signal"]
    );
    assert!(change.diff.contains("+- ["), "{}", change.diff);
    assert_eq!(
        e.consumed.as_deref(),
        Some(&["2026-m1".to_string(), "seed".into()][..])
    );
    assert_eq!(e.cost_usd, Some(0.125));

    let items = w.items("dev");
    assert_eq!(items.len(), 3);
    assert_eq!(items[0].retired_why.as_deref(), Some("Stale."));
    assert_eq!(items[0].retired_at.as_deref(), Some(e.created_at.as_str()));
    assert_eq!(items[1].helpful, 1);
    assert_eq!(items[2].text, "Cite what you read.");
    assert_eq!(items[2].kind, "lesson");
    assert_eq!(items[2].source, e.id);

    let stored = w.dump()["reflections"].clone();
    assert_eq!(stored.len(), 1, "one entry for one reflection");
}

#[test]
fn a_second_reflection_without_new_evidence_writes_nothing_and_calls_no_model() {
    let w = world();
    seed_memory(&w);
    w.reply("curate-dev.json", DELTAS);
    w.reflect();
    let (calls, rows) = (w.calls(), w.dump());
    let out = w.reflect();
    assert!(out.entry.is_none());
    assert_eq!(w.calls(), calls);
    assert_eq!(w.dump(), rows);
}

#[test]
fn a_failed_curation_call_writes_nothing() {
    let w = world();
    seed_memory(&w);
    w.reply("fail", "");
    let rows = w.dump();
    let got = w.reflect_with(&Options::default(), &Cancel::default());
    assert!(matches!(got, Err(Error::Failed(m)) if m.contains("boom")));
    assert_eq!(w.dump(), rows);
}

#[test]
fn a_canceled_reflection_writes_nothing() {
    let w = world();
    seed_memory(&w);
    w.reply("curate-dev.json", DELTAS);
    let rows = w.dump();
    let cancel = Cancel::default();
    cancel.cancel();
    let got = w.reflect_with(&Options::default(), &cancel);
    assert!(
        matches!(&got, Err(Error::Conflict(m)) if m == CANCELED),
        "{got:?}"
    );
    assert_eq!(w.dump(), rows);
}

#[test]
fn facts_that_a_meeting_adds_during_the_calls_survive() {
    let w = world();
    seed_memory(&w);
    w.reply("curate-dev.json", DELTAS);
    w.reply("hold", "");
    std::thread::scope(|scope| {
        let run = scope.spawn(|| w.reflect());
        while !w.bin().join("held").exists() {
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
        // A meeting lands and the item that the deltas retire goes away.
        w.store
            .write(|tx| {
                let mut items = read::memory(tx, "p", "dev")?;
                items.retain(|i| i.id != "m-a1");
                items.push(item("m-a3", "A new fact."));
                write_memory(tx, "p", "dev", &items)
            })
            .unwrap();
        std::fs::remove_file(w.bin().join("hold")).unwrap();
        let out = run.join().unwrap();
        let change = &out.entry.unwrap().memory[0];
        assert!(change.retired.is_empty());
        assert!(
            change
                .dropped
                .iter()
                .any(|d| d == "retire m-a1: no longer active")
        );
    });
    let items = w.items("dev");
    assert!(items.iter().any(|i| i.id == "m-a3"), "the fact survives");
    assert!(
        !items.iter().any(|i| i.id == "m-a1"),
        "the deltas used a fresh read"
    );
}

#[test]
fn a_revert_restores_the_rows_and_marks_the_additions_harmful() {
    let w = world();
    seed_memory(&w);
    w.reply("curate-dev.json", DELTAS);
    let before = w.items("dev");
    let e = w.reflect().entry.unwrap();
    let added = e.memory[0].added[0].clone();

    let r = revert(&w.store, "p", &e.id, clock::now_ms()).unwrap();
    assert_eq!(r.reverts.as_deref(), Some(e.id.as_str()));
    let change = &r.memory[0];
    assert_eq!(change.retired, std::slice::from_ref(&added));
    assert_eq!(change.harmful, std::slice::from_ref(&added));
    assert_eq!(change.restored.as_deref(), Some(&["m-a1".to_string()][..]));

    let after = w.items("dev");
    assert_eq!(
        &after[..2],
        &before[..],
        "each earlier item is back, exactly"
    );
    assert_eq!(after[2].id, added);
    assert_eq!(after[2].harmful, 1);
    assert_eq!(
        after[2].retired_why.as_deref(),
        Some(&*format!("reverted by {}", r.id))
    );

    let stored = read_entries(&w);
    assert_eq!(stored[0]["status"], "reverted");
    assert_eq!(stored[1]["reverts"], json!(e.id));
    assert!(stored[1].get("consumed").is_none());
}

fn read_entries(w: &World) -> Vec<Value> {
    let mut out = Vec::new();
    w.store
        .read(|c| {
            out = read::reflections(c, "p")?;
            Ok(())
        })
        .unwrap();
    out
}

#[test]
fn a_revert_keeps_later_counters_and_a_later_retirement() {
    let w = world();
    seed_memory(&w);
    w.reply("curate-dev.json", DELTAS);
    let e = w.reflect().entry.unwrap();
    w.store
        .write(|tx| {
            let mut items = read::memory(tx, "p", "dev")?;
            items[1].helpful += 4;
            items[0].retired_at = Some("2099-01-01T00:00:00.000Z".into());
            write_memory(tx, "p", "dev", &items)
        })
        .unwrap();
    let r = revert(&w.store, "p", &e.id, clock::now_ms()).unwrap();
    let items = w.items("dev");
    assert_eq!(items[1].helpful, 4, "5 less the one of this reflection");
    assert!(items[0].retired_at.is_some(), "a later retirement stays");
    assert_eq!(r.memory[0].restored.as_deref(), Some(&[][..]));
}

#[test]
fn revert_refuses_what_it_cannot_undo_and_writes_nothing() {
    let w = world();
    seed_memory(&w);
    w.reply("curate-dev.json", NOTHING);
    let empty = w.reflect().entry.unwrap();
    let rows = w.dump();
    let conflict = |id: &str| match revert(&w.store, "p", id, clock::now_ms()) {
        Err(Error::Conflict(m)) => m,
        other => panic!("{other:?}"),
    };
    assert_eq!(
        conflict(&empty.id),
        format!(
            "Reflection \"{}\" changed no memory; there is nothing to revert.",
            empty.id
        )
    );
    assert!(matches!(
        revert(&w.store, "p", "r-9", clock::now_ms()),
        Err(Error::NotFound(m)) if m == "Reflection \"r-9\" not found in p."
    ));
    assert_eq!(w.dump(), rows);

    w.sql("update memory set text = text");
    w.store
        .write(|tx| {
            let mut entries = read::reflections(tx, "p")?;
            entries[0]["memory"] = json!([{"agent": "dev", "added": [], "retired": [],
                "helpful": ["m-a2"], "harmful": [], "dropped": [], "diff": "@@"}]);
            write_reflections(tx, "p", &entries)
        })
        .unwrap();
    w.sql("update memory set helpful = 2 where id = 'm-a2'");
    let r = revert(&w.store, "p", &empty.id, clock::now_ms()).unwrap();
    assert_eq!(
        conflict(&empty.id),
        format!("Reflection \"{}\" is already reverted.", empty.id)
    );
    assert_eq!(
        conflict(&r.id),
        format!(
            "Reflection \"{}\" is a revert; it cannot be reverted.",
            r.id
        )
    );
}

/// Four meetings in which `dev` cited a file that does not exist. `a` and `b` are
/// held out, `x` and `y` are for tuning.
fn seed_cases(w: &World, prompt_hash: &str) {
    let hash = prompt_hash.to_string();
    w.store
        .write(move |tx| {
            for (n, id) in ["a", "b", "x", "y"].iter().enumerate() {
                let at = format!("2026-10-0{}T00:00:00.000Z", n + 1);
                let answer = json!({"answer": "Old.", "priorities": [], "concerns": [], "memory_notes": [], "model": "m"});
                let data = json!({
                    "id": id, "product": "p", "scope": null, "question": format!("Question {id}"),
                    "created_at": at, "status": "valid",
                    "roles": {"pm": answer, "dev": answer},
                    "priorities": [], "decisions": [],
                });
                tx.execute(
                    "insert into minutes (id, product, question, status, created_at, data)
                     values (?1, 'p', 'q', 'valid', ?2, ?3)",
                    [*id, &at, &data.to_string()],
                )?;
                let line = json!({
                    "minutes_id": id, "created_at": at, "question": "q", "scope": null,
                    "agents": {
                        "dev": {"error": null, "prompt_hash": hash, "cost_usd": 0.5, "sin_datos": 0,
                                "unresolved_sources": [{"type": "path", "ref": "docs/gone.md"}]},
                        "pm": {"error": null, "prompt_hash": "other", "cost_usd": 0.5, "sin_datos": 0,
                               "unresolved_sources": []},
                    },
                    "synthesis": {"first_check": {"passed": false, "failures": []},
                                  "invented_sources": [], "repaired": false, "cost_usd": 0.5},
                });
                agentsi_store::meeting::write_meeting_log(tx, "p", id, &line, "# Snapshot\nfrozen")?;
            }
            Ok(())
        })
        .unwrap();
}

fn dev_hash(w: &World) -> String {
    let text = std::fs::read_to_string(w.root().join("plugin/agents/dev.md")).unwrap();
    sha256(&crate::roster::parse_agent_file(&text).1)
}

#[test]
fn a_passing_proposal_is_a_pending_reflection_decision() {
    let w = world();
    seed_cases(&w, &dev_hash(&w));
    let out = w.reflect();
    let e = out.entry.unwrap();
    let p = e.proposal.as_ref().unwrap();
    assert_eq!(p.status, "pending", "{:?}", p.replay);
    assert_eq!(p.target, "workspace/p/agents/dev.md");
    assert_eq!(
        p.content,
        "---\nextends: dev\n---\nCite only files you read.\n"
    );
    assert_eq!(p.replay.tuning, ["x", "y"]);
    assert_eq!(p.replay.held_out, ["a", "b"]);
    assert_eq!(p.replay.rows[0].baseline, "stored");
    assert_eq!(p.replay.verdict.better, ["unresolved_per_case"]);
    assert_eq!(out.candidates[0].outcome, "proposed");

    let id = p.decision_id.unwrap();
    let (kind, reflection, text, why): (String, String, String, String) = {
        let mut got = None;
        w.store
            .read(|c| {
                got = Some(c.query_row(
                    "select kind, reflection_id, text, why from decisions where id = ?1",
                    [id],
                    |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)),
                )?);
                Ok(())
            })
            .unwrap();
        got.unwrap()
    };
    assert_eq!(kind, "reflection");
    assert_eq!(reflection, e.id);
    assert!(
        text.contains("overlay nuevo") && text.contains("Replay sobre 2 casos"),
        "{text}"
    );
    assert_eq!(why, "It cited files it did not read.");
    let stored = read_entries(&w);
    assert_eq!(stored[0]["proposal"]["decision_id"], json!(id));
    assert_eq!(
        w.dump()["agent_overlays"].len(),
        0,
        "no prompt changes before the owner approves"
    );
}

#[test]
fn a_proposal_that_worsens_a_held_out_metric_is_discarded_and_makes_no_decision() {
    let w = world();
    seed_cases(&w, &dev_hash(&w));
    w.reply("answer.json", BAD_ANSWER);
    let e = w.reflect().entry.unwrap();
    let p = e.proposal.unwrap();
    assert_eq!(p.status, "discarded");
    assert!(p.decision_id.is_none());
    assert!(
        p.replay
            .verdict
            .worse
            .contains(&"unresolved_per_case".to_string())
    );
    assert!(w.dump()["decisions"].is_empty());
}

#[test]
fn memory_only_makes_no_proposal() {
    let w = world();
    seed_cases(&w, &dev_hash(&w));
    let o = Options {
        memory_only: true,
        budget_usd: None,
    };
    let out = w.reflect_with(&o, &Cancel::default()).unwrap();
    assert!(out.candidates.is_empty());
    assert!(out.entry.unwrap().proposal.is_none());
}

#[test]
fn below_four_cases_reflect_says_so_and_makes_no_proposal() {
    let w = world();
    seed_cases(&w, &dev_hash(&w));
    w.sql("delete from meeting_log where minutes_id = 'y'");
    w.sql("delete from minutes where id = 'y'");
    let out = w.reflect();
    assert_eq!(out.candidates[0].outcome, "insufficient_evidence");
    assert_eq!(
        out.candidates[0].note.as_deref(),
        Some("3 casos de replay (mínimo 4)")
    );
    assert!(out.entry.unwrap().proposal.is_none());
}

#[test]
fn an_old_prompt_hash_runs_the_baseline_again() {
    let w = world();
    seed_cases(&w, "an older prompt");
    let e = w.reflect().entry.unwrap();
    let p = e.proposal.unwrap();
    assert_eq!(p.replay.rows[0].baseline, "rerun");
    assert_eq!(p.replay.rows[0].before.unresolved, 0);
}

#[test]
fn a_replay_over_budget_is_discarded_with_the_reason() {
    let w = world();
    seed_cases(&w, &dev_hash(&w));
    let o = Options {
        memory_only: false,
        budget_usd: Some(0.5),
    };
    let e = w
        .reflect_with(&o, &Cancel::default())
        .unwrap()
        .entry
        .unwrap();
    let p = e.proposal.unwrap();
    assert_eq!(p.status, "discarded");
    let why = p.replay.aborted.unwrap();
    assert!(
        why.starts_with("replay budget of 0.5 USD exhausted"),
        "{why}"
    );
}

#[test]
fn a_rejected_text_is_never_proposed_again_even_with_other_punctuation() {
    let w = world();
    seed_cases(&w, &dev_hash(&w));
    let first = w.reflect().entry.unwrap();
    let id = first.proposal.unwrap().decision_id.unwrap();
    let root = w.root();
    w.store
        .try_write(move |tx| decide_at(tx, id, Verdict::Rejected, 0, Some(&root)))
        .unwrap()
        .unwrap();
    // The evidence of the first proposal is consumed, so new evidence must come.
    w.sql("update reflections set entry = json_remove(entry, '$.consumed')");
    w.sql("update reflections set entry = json_set(entry, '$.created_at', '2020-01-01T00:00:00.000Z')");
    w.reply("proposal.json", &PROPOSAL.replace("read.", "read!"));
    let e = w.reflect().entry.unwrap();
    let p = e.proposal.unwrap();
    assert_eq!(p.status, "discarded");
    let why = p.replay.aborted.unwrap();
    assert!(
        why.starts_with("repite la propuesta de") && why.ends_with("(rejected)"),
        "{why}"
    );
}

/// The proof of the coordinator's point: a replay writes nothing to the record of the
/// owner. The proposal and the replay run alone here, with no reflection write after
/// them, and each table is compared before and after.
#[test]
fn a_replay_writes_no_row() {
    let w = world();
    seed_cases(&w, "an older prompt");
    let root = w.root();
    let product = w.registry.product("p").unwrap().clone();
    let rows = w.dump();
    let cancel = Cancel::default();
    let program = w.program.clone();
    let call = |c: &crate::story::claude::Call| claude(&program, c, &|_| {}, &cancel);
    let items = |_: &str| -> std::result::Result<Vec<MemoryItem>, String> { Ok(Vec::new()) };
    let mut e = None;
    w.store
        .read(|c| {
            e = Some(read_evidence(c, &root, &product, clock::now_ms()));
            Ok(())
        })
        .unwrap();
    let e = e.unwrap().unwrap();
    assert!(!e.ready.is_empty());
    let out = propose_for_ready(&ProposeInput {
        store: &w.store,
        product: &product,
        owner: &w.registry.owner,
        root: &root,
        ready: &e.ready,
        log: &e.log,
        minutes: &e.minutes,
        reflections: &e.reflections,
        items: &items,
        call: &call,
        budget_usd: None,
    })
    .unwrap();
    let p = out.proposal.unwrap();
    assert_eq!(p.replay.rows.len(), 2, "both held-out cases ran");
    assert_eq!(p.replay.rows[0].baseline, "rerun");
    assert!(
        w.calls() >= 9,
        "1 proposal and 4 calls for each case: {}",
        w.calls()
    );
    assert_eq!(w.dump(), rows, "a replay leaves each table as it was");
    let tree = std::fs::read_dir(root.join("plugin/agents"))
        .unwrap()
        .count();
    assert_eq!(tree, 3, "a replay writes no role file");
}

#[test]
fn an_owner_role_that_is_no_agent_id_stops_the_reflection_before_any_call() {
    let w = world();
    seed_memory(&w);
    w.sql(
        "insert into minutes (id, product, question, status, created_at, data) values ('2026-m2', 'p', 'q', 'valid', '2026-10-03T00:00:00.000Z',
         '{\"id\":\"2026-m2\",\"created_at\":\"2026-10-03T00:00:00.000Z\",\"decisions\":[],\"priorities\":[{\"title\":\"A\",\"owner_role\":\"Dev Lead\",\"mark\":\"rejected\"}]}')",
    );
    let rows = w.dump();
    let got = w.reflect_with(&Options::default(), &Cancel::default());
    assert!(
        matches!(&got, Err(Error::Failed(m)) if m == "Invalid agent id \"Dev Lead\""),
        "{got:?}"
    );
    assert_eq!(w.calls(), 0);
    assert_eq!(w.dump(), rows);
}

#[test]
fn a_priority_with_no_owner_is_a_signal_of_no_agent() {
    let w = world();
    seed_memory(&w);
    w.sql(
        "insert into minutes (id, product, question, status, created_at, data) values ('2026-m2', 'p', 'q', 'valid', '2026-10-03T00:00:00.000Z',
         '{\"id\":\"2026-m2\",\"created_at\":\"2026-10-03T00:00:00.000Z\",\"decisions\":[],\"priorities\":[{\"title\":\"A\",\"mark\":\"rejected\"}]}')",
    );
    let e = w.reflect().entry.unwrap();
    let agents: Vec<&str> = e.memory.iter().map(|m| m.agent.as_str()).collect();
    assert_eq!(agents, ["dev"]);
    assert!(w.items("").is_empty());
}

#[test]
fn an_edit_during_the_proposal_call_makes_the_proposal_stale() {
    let w = world();
    seed_cases(&w, &dev_hash(&w));
    w.reply("hold-proposal", "");
    let e = std::thread::scope(|scope| {
        let run = scope.spawn(|| w.reflect());
        while !w.bin().join("held").exists() {
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
        w.sql("insert into agent_overlays (product, agent, body) values ('p', 'dev', 'mine')");
        std::fs::remove_file(w.bin().join("hold-proposal")).unwrap();
        run.join().unwrap().entry.unwrap()
    });
    let p = e.proposal.unwrap();
    assert_eq!(p.target_hash, None, "the proposal saw no overlay");
    let id = p.decision_id.unwrap();
    let root = w.root();
    let done = w
        .store
        .try_write(move |tx| decide_at(tx, id, Verdict::Approved, 1_000, Some(&root)))
        .unwrap()
        .unwrap();
    assert_eq!(done.status, Outcome::Stale);
    assert!(w.dump()["agent_overlays"][0].ends_with("mine"));
}

#[test]
fn a_revert_during_the_calls_does_not_take_the_id_of_the_reflection() {
    let w = world();
    seed_memory(&w);
    w.reply("curate-dev.json", DELTAS);
    let first = w.reflect().entry.unwrap();
    w.reply("curate-dev.json", NOTHING);
    w.sql(
        "insert into minutes (id, product, question, status, created_at, data) values ('2026-m3', 'p', 'q', 'valid', '2099-01-01T00:00:00.000Z',
         '{\"id\":\"2026-m3\",\"created_at\":\"2099-01-01T00:00:00.000Z\",\"decisions\":[],\"priorities\":[{\"title\":\"B\",\"owner_role\":\"dev\",\"mark\":\"accepted\"}]}')",
    );
    w.reply("hold", "");
    let (second, reverted) = std::thread::scope(|scope| {
        let run = scope.spawn(|| w.reflect());
        while !w.bin().join("held").exists() {
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
        let reverted = revert(&w.store, "p", &first.id, clock::now_ms()).unwrap();
        std::fs::remove_file(w.bin().join("hold")).unwrap();
        (run.join().unwrap().entry.unwrap(), reverted)
    });
    assert_ne!(second.id, reverted.id);
    assert_eq!(read_entries(&w).len(), 3);
}

#[test]
fn a_failed_role_file_rolls_each_row_back() {
    let w = world();
    // A folder where the file should be: the write fails after the rows.
    std::fs::create_dir(w.root().join("plugin/agents/new.md")).unwrap();
    w.store
        .write(|tx| {
            tx.execute(
                "insert into decisions (id, product, kind, reflection_id, text, why)
                 values (41, 'p', 'reflection', 'r-1', 'Change it', 'Why')",
                [],
            )?;
            let entry = json!({"id": "r-1", "created_at": "t", "status": "applied", "proposal": {
                "agent": "dev", "target": "plugin/agents/new.md", "status": "pending",
                "target_hash": null, "content": "New.\n",
            }});
            write_reflections(tx, "p", &[entry])
        })
        .unwrap();
    let rows = w.dump();
    let root = w.root();
    let got = w
        .store
        .try_write(move |tx| decide_at(tx, 41, Verdict::Approved, 1_000, Some(&root)))
        .unwrap();
    assert!(got.is_err());
    assert_eq!(w.dump(), rows);
}

#[test]
fn approving_stores_the_overlay_and_records_the_decision() {
    let w = world();
    seed_cases(&w, &dev_hash(&w));
    let e = w.reflect().entry.unwrap();
    let id = e.proposal.unwrap().decision_id.unwrap();
    let root = w.root();
    let done = w
        .store
        .try_write(move |tx| decide_at(tx, id, Verdict::Approved, 1_000, Some(&root)))
        .unwrap()
        .unwrap();
    assert_eq!(done.status, Outcome::Approved);
    assert_eq!(done.action, None);
    assert_eq!(done.minutes_id, None);
    let rows = w.dump();
    assert!(rows["agent_overlays"][0].contains("Cite only files you read."));
    assert!(rows["decisions"][0].contains("approved"));
    assert_eq!(read_entries(&w)[0]["proposal"]["status"], "approved");
}

#[test]
fn a_target_that_changed_records_a_stale_decision() {
    let w = world();
    seed_cases(&w, &dev_hash(&w));
    let e = w.reflect().entry.unwrap();
    let id = e.proposal.unwrap().decision_id.unwrap();
    w.sql("insert into agent_overlays (product, agent, body) values ('p', 'dev', 'mine')");
    let root = w.root();
    let done = w
        .store
        .try_write(move |tx| decide_at(tx, id, Verdict::Approved, 1_000, Some(&root)))
        .unwrap()
        .unwrap();
    assert_eq!(done.status, Outcome::Stale);
    let rows = w.dump();
    assert!(rows["decisions"][0].contains("stale"));
    assert!(rows["agent_overlays"][0].ends_with("mine"));
}

#[test]
fn a_decision_with_no_proposal_is_rejected_for_good() {
    let w = world();
    w.sql(
        "insert into decisions (id, product, kind, reflection_id, text, why)
         values (40, 'p', 'reflection', 'r-gone', 'Change it', 'Why')",
    );
    let root = w.root();
    let done = w
        .store
        .try_write(move |tx| decide_at(tx, 40, Verdict::Approved, 1_000, Some(&root)))
        .unwrap()
        .unwrap();
    assert_eq!(done.status, Outcome::Rejected);
    assert_eq!(
        done.refused.as_deref(),
        Some(
            "Decision 40 rejected: Reflection \"r-gone\" has no proposal in p. It can never be applied."
        )
    );
    assert!(w.dump()["decisions"][0].contains("rejected"));
}

#[test]
fn a_decision_on_a_settled_proposal_is_refused_and_changes_nothing() {
    let w = world();
    seed_cases(&w, &dev_hash(&w));
    let e = w.reflect().entry.unwrap();
    let id = e.proposal.unwrap().decision_id.unwrap();
    let root = w.root();
    let r2 = root.clone();
    w.store
        .try_write(move |tx| decide_at(tx, id, Verdict::Rejected, 1_000, Some(&root)))
        .unwrap()
        .unwrap();
    let rows = w.dump();
    let got = w
        .store
        .try_write(move |tx| decide_at(tx, id, Verdict::Approved, 2_000, Some(&r2)))
        .unwrap();
    assert!(matches!(got, Err(crate::marks::Error::Invalid(m)) if m.contains("already rejected")));
    assert_eq!(w.dump(), rows);
}
