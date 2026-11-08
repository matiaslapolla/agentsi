//! The index of a product: the port of `indexProduct` of
//! `packages/core/src/indexer.ts`. A run reads each repo, then writes the
//! rows that changed. A repo whose HEAD did not change in the last 10 minutes
//! is skipped, unless the run is forced.

use super::collect::{self, DocRow};
use super::exec::git;
use super::memory_text::{is_agent_id, render_for_prompt};
use super::minutes_md;
use super::repos::{RepoTarget, repo_targets};
use crate::clock;
use crate::error::Result;
use crate::registry::Product;
use agentsi_proto::knowledge_api::{KbIndexResult, KbRepoIndexed};
use agentsi_store::{Store, read};
use rusqlite::{Connection, OptionalExtension, params};
use serde_json::Value;
use std::collections::HashSet;
use std::sync::Mutex;
use std::sync::atomic::{AtomicUsize, Ordering};

const FRESH_MS: i64 = 10 * 60 * 1000;
const REPO_KINDS: [&str; 2] = ["doc", "journal"];
/// Repos that are read at the same time.
const PARALLEL_REPOS: usize = 4;

/// Inserts the rows, and updates a row only when its text changed. Thus the
/// time of a row that did not change stays as it was. A later row with the
/// same kind and ref replaces an earlier one.
fn upsert_docs(c: &Connection, product: &str, rows: &[DocRow]) -> rusqlite::Result<()> {
    let mut last: Vec<&DocRow> = Vec::new();
    let mut at = std::collections::HashMap::new();
    for row in rows {
        match at.get(&(row.kind, row.reference.as_str())) {
            Some(&i) => last[i] = row,
            None => {
                at.insert((row.kind, row.reference.as_str()), last.len());
                last.push(row);
            }
        }
    }
    let mut stmt = c.prepare_cached(
        "insert into documents (product, scope, kind, ref, title, body, updated_at)
         values (?1, ?2, ?3, ?4, ?5, ?6, ?7)
         on conflict (product, kind, ref) do update
           set scope = excluded.scope, title = excluded.title, body = excluded.body,
               updated_at = excluded.updated_at
           where documents.scope is not excluded.scope
              or documents.title is not excluded.title
              or documents.body is not excluded.body",
    )?;
    for r in last {
        stmt.execute(params![
            product,
            r.scope,
            r.kind,
            r.reference,
            r.title,
            r.body,
            r.updated_at
        ])?;
    }
    Ok(())
}

/// Deletes the documents of `kinds` under `prefix` whose ref is not in `keep`. `scope` is
/// `Some(scope)` for one scope (`Some(None)` for no scope), or `None` for any scope.
fn delete_missing(
    c: &Connection,
    product: &str,
    kinds: &[&str],
    scope: Option<Option<&str>>,
    prefix: &str,
    keep: &HashSet<&str>,
) -> rusqlite::Result<()> {
    let mut stmt = c.prepare_cached(
        "select id, ref from documents where product = ?1 and kind = ?2
           and (?4 or scope is ?3)",
    )?;
    let mut gone = Vec::new();
    for kind in kinds {
        let rows = stmt.query_map(
            params![product, kind, scope.flatten(), scope.is_none()],
            |r| Ok((r.get::<_, i64>(0)?, r.get::<_, String>(1)?)),
        )?;
        for row in rows {
            let (id, reference) = row?;
            let under = prefix.is_empty()
                || reference
                    .strip_prefix(prefix)
                    .is_some_and(|rest| rest.starts_with('/') || rest.starts_with('#'));
            if under && !keep.contains(reference.as_str()) {
                gone.push(id);
            }
        }
    }
    let mut delete = c.prepare_cached("delete from documents where id = ?1")?;
    for id in gone {
        delete.execute([id])?;
    }
    Ok(())
}

fn refs(rows: &[DocRow]) -> HashSet<&str> {
    rows.iter().map(|r| r.reference.as_str()).collect()
}

/// True when the repo was indexed at this HEAD in the last 10 minutes.
fn is_fresh(
    c: &Connection,
    product: &str,
    path: &str,
    head: &str,
    now_ms: i64,
) -> rusqlite::Result<bool> {
    let row: Option<(Option<String>, Option<String>)> = c
        .query_row(
            "select last_commit, indexed_at from index_state where product = ?1 and path = ?2",
            [product, path],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .optional()?;
    Ok(row.is_some_and(|(last, at)| {
        last.as_deref() == Some(head)
            && at
                .and_then(|t| clock::parse_iso_ms(&t))
                .is_some_and(|t| t > now_ms - FRESH_MS)
    }))
}

fn index_repo(
    store: &Store,
    product: &Product,
    target: &RepoTarget,
    force: bool,
) -> Result<KbRepoIndexed> {
    let dir = target.dir();
    let head = git(&dir, &["rev-parse", "HEAD"]).stdout.trim().to_string();
    let head = if head.is_empty() { "none".into() } else { head };
    let result = |skipped, docs, commits, issues| KbRepoIndexed {
        path: target.path.clone(),
        skipped,
        docs,
        commits,
        issues,
    };
    if !force {
        let (id, path, h) = (product.id.clone(), target.path.clone(), head.clone());
        let fresh = store.read(|c| Ok(is_fresh(c, &id, &path, &h, clock::now_ms())?))?;
        if fresh {
            return Ok(result(true, 0, 0, None));
        }
    }

    // `git log` and `gh` read at the same time, as `Promise.all` did.
    let (commits, issues) = std::thread::scope(|s| {
        let issues = s.spawn(|| collect::collect_issues(target, clock::now_ms()));
        let commits = collect::collect_commits(target);
        (commits, issues.join().unwrap_or(None))
    });
    let docs = collect::collect_docs(target);
    let (counts, writes) = (
        (
            docs.len() as u64,
            commits.as_ref().map(Vec::len),
            issues.as_ref().map(Vec::len),
        ),
        (
            product.id.clone(),
            product.paths.len(),
            target.clone(),
            head,
        ),
    );
    store.write(move |tx| {
        let (id, paths, target, head) = writes;
        let all: Vec<DocRow> = docs
            .iter()
            .chain(commits.iter().flatten())
            .chain(issues.iter().flatten())
            .cloned()
            .collect();
        upsert_docs(tx, &id, &all)?;
        let scope = target.scope.as_deref();
        delete_missing(tx, &id, &REPO_KINDS, Some(scope), &target.prefix, &refs(&docs))?;
        if let (Some(commits), 1) = (&commits, paths) {
            delete_missing(tx, &id, &["commit"], Some(scope), "", &refs(commits))?;
        }
        if let Some(issues) = &issues {
            delete_missing(tx, &id, &["issue"], Some(scope), &target.prefix, &refs(issues))?;
        }
        tx.execute(
            "insert into index_state (product, path, last_commit, indexed_at) values (?1, ?2, ?3, ?4)
             on conflict (product, path) do update
               set last_commit = excluded.last_commit, indexed_at = excluded.indexed_at",
            params![id, target.path, head, clock::iso(clock::now_ms())],
        )?;
        Ok(())
    })?;
    Ok(result(
        false,
        counts.0,
        counts.1.unwrap_or(0) as u64,
        counts.2.map(|n| n as u64),
    ))
}

/// The minutes and the memory of a product as documents.
fn index_workspace(store: &Store, product: &str) -> Result<(u64, u64)> {
    let id = product.to_string();
    Ok(store.write(move |tx| {
        let mut minutes = Vec::new();
        let mut stmt = tx.prepare_cached(
            "select data from minutes where product = ?1 and data is not null order by id",
        )?;
        let texts = stmt.query_map([&id], |r| r.get::<_, String>(0))?;
        for text in texts {
            let Ok(m) = serde_json::from_str::<Value>(&text?) else {
                continue;
            };
            let field = |k: &str| m.get(k).and_then(Value::as_str);
            let created = field("created_at")
                .and_then(clock::parse_iso_ms)
                .unwrap_or_else(clock::now_ms);
            minutes.push(DocRow {
                scope: field("scope").map(String::from),
                kind: "minutes",
                reference: field("id").unwrap_or_default().to_string(),
                title: field("question").unwrap_or_default().to_string(),
                body: minutes_md::render(&m),
                updated_at: clock::iso(created),
            });
        }
        drop(stmt);

        let mut memory = Vec::new();
        for agent in read::memory_agents(tx, &id)? {
            if !is_agent_id(&agent) {
                continue;
            }
            let items = read::memory(tx, &id, &agent)?;
            let body = render_for_prompt(&items, None);
            if body.is_empty() {
                continue;
            }
            let last = items
                .iter()
                .flat_map(|i| [i.created_at.as_str(), i.retired_at.as_deref().unwrap_or("")])
                .max();
            let at = last
                .filter(|t| !t.is_empty())
                .and_then(clock::parse_iso_ms)
                .unwrap_or_else(clock::now_ms);
            memory.push(DocRow {
                scope: None,
                kind: "memory",
                title: format!("Memory: {agent}"),
                reference: agent,
                body,
                updated_at: clock::iso(at),
            });
        }
        let all: Vec<DocRow> = minutes.iter().chain(&memory).cloned().collect();
        upsert_docs(tx, &id, &all)?;
        for (kind, rows) in [("minutes", &minutes), ("memory", &memory)] {
            // `indexWorkspace` filters by kind and ref only, so a scoped minutes leaves too.
            delete_missing(tx, &id, &[kind], None, "", &refs(rows))?;
        }
        Ok((minutes.len() as u64, memory.len() as u64))
    })?)
}

pub fn index_product(store: &Store, product: &Product, force: bool) -> Result<KbIndexResult> {
    let started = std::time::Instant::now();
    let targets = repo_targets(product);
    let next = AtomicUsize::new(0);
    let done: Mutex<Vec<Option<Result<KbRepoIndexed>>>> =
        Mutex::new((0..targets.len()).map(|_| None).collect());
    std::thread::scope(|s| {
        for _ in 0..PARALLEL_REPOS.min(targets.len()) {
            s.spawn(|| {
                loop {
                    let i = next.fetch_add(1, Ordering::SeqCst);
                    let Some(target) = targets.get(i) else { break };
                    let result = index_repo(store, product, target, force);
                    done.lock().expect("no panic holds the lock")[i] = Some(result);
                }
            });
        }
    });
    let mut repos = Vec::new();
    for slot in done.into_inner().expect("no panic holds the lock") {
        repos.push(slot.expect("each repo ran")?);
    }
    let (minutes, memory) = index_workspace(store, &product.id)?;
    Ok(KbIndexResult {
        product: product.id.clone(),
        repos,
        minutes,
        memory,
        ms: started.elapsed().as_millis() as u64,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;

    fn git(dir: &Path, args: &[&str]) {
        let ok = std::process::Command::new("git")
            .args(["-c", "user.name=T", "-c", "user.email=t@example.invalid"])
            .args(["-c", "commit.gpgsign=false"])
            .args(args)
            .current_dir(dir)
            .env("GIT_CONFIG_GLOBAL", "/dev/null")
            .env("GIT_CONFIG_SYSTEM", "/dev/null")
            .status()
            .unwrap()
            .success();
        assert!(ok, "git {args:?}");
    }

    fn repo(dir: &Path, doc: &str) {
        std::fs::create_dir_all(dir.join("docs")).unwrap();
        std::fs::write(
            dir.join("README.md"),
            format!("# {doc}\n\nWords about {doc}.\n"),
        )
        .unwrap();
        git(dir, &["init", "-q", "-b", "main"]);
        git(dir, &["add", "-A"]);
        git(
            dir,
            &["commit", "-q", "-m", format!("feat: {doc}").as_str()],
        );
    }

    fn product(paths: &[&Path]) -> Product {
        Product {
            id: "p".into(),
            name: "P".into(),
            paths: paths
                .iter()
                .map(|p| p.to_string_lossy().into_owned())
                .collect(),
            status: "active".into(),
            roles: Vec::new(),
        }
    }

    fn store(dir: &Path) -> Store {
        Store::open(&dir.join("agentsi.db")).unwrap()
    }

    fn refs(store: &Store, kind: &'static str) -> Vec<String> {
        store
            .read(|c| {
                let mut stmt =
                    c.prepare("select ref from documents where kind = ?1 order by ref")?;
                let rows = stmt.query_map([kind], |r| r.get::<_, String>(0))?;
                Ok(rows.collect::<std::result::Result<Vec<_>, _>>()?)
            })
            .unwrap()
    }

    #[test]
    fn a_second_run_skips_a_fresh_repo_and_a_forced_run_does_not() {
        let dir = tempfile::tempdir().unwrap();
        let repo_dir = dir.path().join("web");
        repo(&repo_dir, "alpha");
        let (store, p) = (store(dir.path()), product(&[&repo_dir]));

        let first = index_product(&store, &p, false).unwrap();
        assert_eq!(
            (
                first.repos[0].skipped,
                first.repos[0].docs,
                first.repos[0].commits,
                first.repos[0].issues
            ),
            (false, 1, 1, None)
        );
        assert!(index_product(&store, &p, false).unwrap().repos[0].skipped);
        assert!(!index_product(&store, &p, true).unwrap().repos[0].skipped);

        git(
            &repo_dir,
            &["commit", "-q", "--allow-empty", "-m", "chore: more"],
        );
        assert!(
            !index_product(&store, &p, false).unwrap().repos[0].skipped,
            "a new HEAD is not fresh"
        );
        assert_eq!(refs(&store, "commit").len(), 2);
    }

    #[test]
    fn a_doc_that_left_the_repo_leaves_the_index_and_an_unchanged_doc_keeps_its_time() {
        let dir = tempfile::tempdir().unwrap();
        let repo_dir = dir.path().join("web");
        repo(&repo_dir, "alpha");
        std::fs::write(repo_dir.join("docs/old.md"), "# Old\n").unwrap();
        let (store, p) = (store(dir.path()), product(&[&repo_dir]));
        index_product(&store, &p, true).unwrap();
        assert_eq!(refs(&store, "doc"), ["README.md", "docs/old.md"]);

        store
            .write(|tx| {
                tx.execute(
                    "update documents set updated_at = '2001-01-01T00:00:00.000Z' where ref = 'README.md'",
                    [],
                )?;
                Ok(())
            })
            .unwrap();
        std::fs::remove_file(repo_dir.join("docs/old.md")).unwrap();
        index_product(&store, &p, true).unwrap();
        assert_eq!(refs(&store, "doc"), ["README.md"]);
        let kept: String = store
            .read(|c| {
                Ok(c.query_row(
                    "select updated_at from documents where ref = 'README.md'",
                    [],
                    |r| r.get(0),
                )?)
            })
            .unwrap();
        assert_eq!(kept, "2001-01-01T00:00:00.000Z", "same text, same time");
    }

    #[test]
    fn two_repos_prefix_their_refs_and_keep_the_commits_of_each_other() {
        let dir = tempfile::tempdir().unwrap();
        let (web, api) = (dir.path().join("web"), dir.path().join("api"));
        repo(&web, "frontend");
        repo(&api, "backend");
        let (store, p) = (store(dir.path()), product(&[&web, &api]));
        let result = index_product(&store, &p, true).unwrap();
        assert_eq!(result.repos.len(), 2);
        assert_eq!(refs(&store, "doc"), ["api/README.md", "web/README.md"]);
        assert_eq!(
            refs(&store, "commit").len(),
            2,
            "no repo deletes the commits of the other"
        );
    }

    #[test]
    fn minutes_and_memory_become_documents_and_leave_with_their_rows() {
        let dir = tempfile::tempdir().unwrap();
        let store = store(dir.path());
        let p = product(&[]);
        store
            .write(|tx| {
                tx.execute(
                    "insert into minutes (id, product, question, status, created_at, data)
                     values ('m-1', 'p', 'What now?', 'valid', '2026-10-01T10:00:00.000Z',
                       '{\"id\":\"m-1\",\"product\":\"p\",\"scope\":null,\"question\":\"What now?\",\"created_at\":\"2026-10-01T10:00:00.000Z\",\"status\":\"valid\",\"priorities\":[],\"disagreements\":[],\"decisions\":[],\"roles\":{},\"check\":{\"failures\":[],\"repaired\":false}}')",
                    [],
                )?;
                for (agent, id, retired) in [("pm", "m-a", None), ("dev", "m-b", Some("2026-10-03T00:00:00.000Z")), ("Bad Agent", "m-c", None)] {
                    tx.execute(
                        "insert into memory (product, agent, id, position, text, kind, helpful, harmful, source, created_at, retired_at)
                         values ('p', ?1, ?2, 0, 'a note', 'fact', 0, 0, 's', '2026-10-02T00:00:00.000Z', ?3)",
                        rusqlite::params![agent, id, retired],
                    )?;
                }
                Ok(())
            })
            .unwrap();
        let result = index_product(&store, &p, false).unwrap();
        assert_eq!(
            (result.minutes, result.memory),
            (1, 1),
            "a retired item and a bad agent id give no memory doc"
        );
        assert_eq!(refs(&store, "minutes"), ["m-1"]);
        assert_eq!(refs(&store, "memory"), ["pm"]);

        store
            .write(|tx| {
                tx.execute("delete from minutes", [])?;
                Ok(())
            })
            .unwrap();
        index_product(&store, &p, false).unwrap();
        assert!(refs(&store, "minutes").is_empty());
    }

    #[test]
    fn a_scoped_minutes_leaves_the_index_with_its_row() {
        let dir = tempfile::tempdir().unwrap();
        let store = store(dir.path());
        let p = product(&[]);
        store
            .write(|tx| {
                tx.execute(
                    "insert into minutes (id, product, scope, question, status, created_at, data)
                     values ('m-s', 'p', 'web', 'Scoped?', 'valid', '2026-10-01T10:00:00.000Z',
                       '{\"id\":\"m-s\",\"product\":\"p\",\"scope\":\"web\",\"question\":\"Scoped?\",\"created_at\":\"2026-10-01T10:00:00.000Z\",\"status\":\"valid\",\"priorities\":[],\"disagreements\":[],\"decisions\":[],\"roles\":{},\"check\":{\"failures\":[],\"repaired\":false}}')",
                    [],
                )?;
                Ok(())
            })
            .unwrap();
        index_product(&store, &p, false).unwrap();
        assert_eq!(refs(&store, "minutes"), ["m-s"]);
        store
            .write(|tx| {
                tx.execute("delete from minutes", [])?;
                Ok(())
            })
            .unwrap();
        index_product(&store, &p, false).unwrap();
        assert!(refs(&store, "minutes").is_empty());
    }
}
