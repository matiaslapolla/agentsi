//! What the front desk and the minutes screens read: one function for each
//! method of `desk_api`, on a reader connection of the store. They use the read
//! functions of `agentsi-store`, and add the few queries that it has not.
//!
//! The rules copy `agentsi status` (`packages/core/src/status.ts`) and the web
//! pages `/p/[product]` and `/p/[product]/m/[id]`: archived minutes and decisions
//! are left out, and the marks and the decision status come from the tables.

use agentsi_proto::desk_api::{
    DeskResult, Disagreement, IndexSummary, LastCommit, MarkCounts, MinutesCheck, MinutesDecision,
    MinutesDetail, MinutesRow, Objective, PendingDecision, Position, PriorityInfo, ProductInfo,
    RoleAnswerInfo, SourceRef, UsageInfo, UsageTotals,
};
use agentsi_store::Result;
use agentsi_store::read::{self, Product};
use rusqlite::Connection;
use serde_json::Value;
use std::collections::{BTreeSet, HashMap};

/// The last minutes that the front desk shows, as `agentsi status` does.
const LAST_MINUTES: usize = 3;

/// The order of the roles of the roster. A role that is not here comes after, by name.
const ROLE_ORDER: [&str; 5] = ["pm", "dev", "qa", "scout", "social"];

pub fn product_info(p: Product) -> ProductInfo {
    ProductInfo {
        path: first_path(p.paths.as_ref()),
        name: p
            .name
            .filter(|n| !n.is_empty())
            .unwrap_or_else(|| p.id.clone()),
        id: p.id,
        archived: false,
    }
}

/// The first folder of the `paths` JSON array of a product.
pub fn first_path(paths: Option<&Value>) -> Option<String> {
    paths?
        .as_array()?
        .iter()
        .find_map(|p| p.as_str().filter(|s| !s.is_empty()))
        .map(String::from)
}

/// The entry of `product` in the registry text of the data root.
fn registry_entry(registry: &str, product: &str) -> Option<Value> {
    let Ok(Value::Object(doc)) = serde_json::from_str::<Value>(registry) else {
        return None;
    };
    doc.get("products")?
        .as_array()?
        .iter()
        .find(|p| p.get("id").and_then(Value::as_str) == Some(product))
        .cloned()
}

/// The agent ids that the registry of the data root lists for `product`.
pub fn registry_roles(registry: &str, product: &str) -> Vec<String> {
    registry_entry(registry, product)
        .map(|p| texts(&p, "roles"))
        .unwrap_or_default()
}

fn text(v: &Value, key: &str) -> Option<String> {
    v.get(key)?.as_str().map(String::from)
}

fn texts(v: &Value, key: &str) -> Vec<String> {
    v.get(key)
        .and_then(Value::as_array)
        .map(|items| {
            items
                .iter()
                .filter_map(|i| i.as_str().map(String::from))
                .collect()
        })
        .unwrap_or_default()
}

fn items<'a>(v: &'a Value, key: &str) -> &'a [Value] {
    v.get(key)
        .and_then(Value::as_array)
        .map_or(&[], Vec::as_slice)
}

fn number(v: &Value, key: &str) -> u64 {
    v.get(key)
        .and_then(|n| n.as_u64().or_else(|| n.as_f64().map(|f| f.max(0.) as u64)))
        .unwrap_or(0)
}

fn archived(c: &Connection, product: &str, kind: &str) -> Result<BTreeSet<String>> {
    Ok(read::archived(c, product)?
        .into_iter()
        .filter(|a| a.kind == kind)
        .map(|a| a.reference)
        .collect())
}

/// The marks of the given minutes, from the `priorities` table.
fn marks_of(c: &Connection, product: &str, ids: &[&str]) -> Result<HashMap<String, MarkCounts>> {
    // The front desk asks for 3 minutes of a product that can have 1,000.
    let one = ids.len() <= 8;
    let mut stmt = c.prepare_cached(if one {
        "select minutes_id, coalesce(mark, 'unmarked'), count(*) from priorities
         where minutes_id = ?1 group by 1, 2"
    } else {
        "select p.minutes_id, coalesce(p.mark, 'unmarked'), count(*)
         from priorities p join minutes m on m.id = p.minutes_id
         where m.product = ?1 group by 1, 2"
    })?;
    let mut out: HashMap<String, MarkCounts> = HashMap::new();
    let mut add = |rows: &mut rusqlite::Rows| -> Result<()> {
        while let Some(r) = rows.next()? {
            let (id, mark, n): (String, String, u32) = (r.get(0)?, r.get(1)?, r.get(2)?);
            let m = out.entry(id).or_default();
            match mark.as_str() {
                "accepted" => m.accepted = n,
                "changed" => m.changed = n,
                "rejected" => m.rejected = n,
                _ => m.unmarked += n,
            }
        }
        Ok(())
    };
    if one {
        for id in ids {
            add(&mut stmt.query([id])?)?;
        }
    } else {
        add(&mut stmt.query([product])?)?;
    }
    Ok(out)
}

/// The minutes of `product` that are not archived, newest first: how many there
/// are, and the rows of the first `limit` of them.
fn minutes_page(c: &Connection, product: &str, limit: usize) -> Result<(usize, Vec<MinutesRow>)> {
    let hidden = archived(c, product, "minutes")?;
    let mut kept: Vec<read::MinutesRow> = read::minutes_list(c, product)?
        .into_iter()
        .filter(|m| !hidden.contains(&m.id))
        .collect();
    let total = kept.len();
    kept.truncate(limit);
    let ids: Vec<&str> = kept.iter().map(|m| m.id.as_str()).collect();
    let marks = marks_of(c, product, &ids)?;
    let heard: BTreeSet<String> = read::listens(c, product)?
        .into_iter()
        .map(|l| l.minutes_id)
        .collect();
    let rows = kept
        .into_iter()
        .map(|m| MinutesRow {
            marks: marks.get(&m.id).copied().unwrap_or_default(),
            heard: heard.contains(&m.id),
            question: m.question.unwrap_or_default(),
            status: m.status,
            created_at: m.created_at,
            scope: m.scope,
            id: m.id,
        })
        .collect();
    Ok((total, rows))
}

/// The rows of the minutes of `product` that are not archived, newest first.
pub fn minutes_rows(c: &Connection, product: &str) -> Result<Vec<MinutesRow>> {
    Ok(minutes_page(c, product, usize::MAX)?.1)
}

fn index_summary(c: &Connection, product: &str) -> Result<IndexSummary> {
    let mut summary = IndexSummary::default();
    let mut stmt =
        c.prepare_cached("select kind, count(*) from documents where product = ?1 group by kind")?;
    let mut rows = stmt.query([product])?;
    while let Some(r) = rows.next()? {
        let (kind, n): (String, i64) = (r.get(0)?, r.get(1)?);
        let n = n.max(0) as u64;
        match kind.as_str() {
            "doc" => summary.docs = n,
            "commit" => summary.commits = n,
            "issue" => summary.issues = n,
            _ => {}
        }
    }
    summary.indexed_at = c.query_row(
        "select max(indexed_at) from index_state where product = ?1",
        [product],
        |r| r.get(0),
    )?;
    let mut stmt = c.prepare_cached(
        "select title, updated_at from documents where product = ?1 and kind = 'commit'
         order by updated_at desc limit 1",
    )?;
    let mut rows = stmt.query([product])?;
    if let Some(r) = rows.next()? {
        let (title, at): (Option<String>, Option<String>) = (r.get(0)?, r.get(1)?);
        summary.last_commit = Some(LastCommit {
            date: at.unwrap_or_default().chars().take(10).collect(),
            title: title.unwrap_or_default(),
        });
    }
    Ok(summary)
}

fn objectives(c: &Connection, product: &str) -> Result<Vec<Objective>> {
    let brief = read::brief(c, product)?;
    Ok(brief
        .as_ref()
        .map(|b| items(b, "objectives"))
        .unwrap_or_default()
        .iter()
        .map(|o| Objective {
            id: text(o, "id").unwrap_or_default(),
            text: text(o, "text").unwrap_or_default(),
            signal: text(o, "signal").unwrap_or_default(),
            due: text(o, "due").unwrap_or_default(),
        })
        .collect())
}

/// The pending decisions of `product` that are not archived, in the order of
/// `agentsi desk` (`pendingItems`): the decisions of each valid minutes, oldest
/// minutes first, then each other decision. Each group is in id order.
fn pending_decisions(c: &Connection, product: &str) -> Result<Vec<PendingDecision>> {
    let hidden = archived(c, product, "decision")?;
    let mut rows: Vec<read::Decision> = read::decisions(c, product, Some("pending"))?
        .into_iter()
        .filter(|d| !hidden.contains(&d.id.to_string()))
        .collect();
    let mut stmt = c.prepare_cached(
        "select id, data from minutes where product = ?1 and data is not null and json_valid(data)
           and json_extract(data, '$.status') = 'valid' order by id",
    )?;
    let valid: Vec<(String, Value)> = stmt
        .query_map([product], |r| {
            Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?))
        })?
        .filter_map(|row| {
            let (id, data) = row.ok()?;
            Some((id, serde_json::from_str(&data).ok()?))
        })
        .collect();
    let rank = |d: &read::Decision| {
        d.minutes_id
            .as_ref()
            .and_then(|m| valid.iter().position(|(v, _)| v == m))
            .unwrap_or(valid.len())
    };
    // The sort is stable, so each group keeps the id order.
    rows.sort_by_key(rank);
    Ok(rows
        .into_iter()
        .map(|d| PendingDecision {
            priorities: linked_priorities(&valid, &d),
            id: d.id,
            kind: d.kind,
            minutes_id: d.minutes_id,
            reflection_id: d.reflection_id,
            text: d.text.unwrap_or_default(),
            why: d.why,
            action: d.action,
        })
        .collect())
}

/// The unmarked priorities that a decision links in its minutes. Old minutes
/// and decisions of a product link none, as `pendingItems` does.
fn linked_priorities(valid: &[(String, Value)], d: &read::Decision) -> Vec<u32> {
    let Some((_, minutes)) = valid
        .iter()
        .find(|(id, _)| Some(id) == d.minutes_id.as_ref())
    else {
        return Vec::new();
    };
    let Some(linked) = items(minutes, "decisions")
        .iter()
        .find(|x| text(x, "text") == d.text)
        .map(|x| items(x, "priorities"))
    else {
        return Vec::new();
    };
    let priorities = items(minutes, "priorities");
    let mut out: Vec<u32> = Vec::new();
    for n in linked {
        let Some(i) = n.as_u64().filter(|i| (*i as usize) < priorities.len()) else {
            continue;
        };
        // `!mark` in TypeScript: an empty text is no mark.
        let marked = priorities[i as usize]
            .get("mark")
            .and_then(Value::as_str)
            .is_some_and(|m| !m.is_empty());
        if !marked && !out.contains(&(i as u32)) {
            out.push(i as u32);
        }
    }
    out
}

fn product_usage(c: &Connection, product: &str) -> Result<UsageTotals> {
    Ok(c.query_row(
        "select count(*), coalesce(sum(u.input_tokens), 0), coalesce(sum(u.output_tokens), 0),
                coalesce(sum(u.cache_read), 0), coalesce(sum(u.cache_write), 0),
                coalesce(sum(u.cost_usd), 0)
         from usage u join minutes m on m.id = u.minutes_id where m.product = ?1",
        [product],
        |r| {
            Ok(UsageTotals {
                rows: r.get::<_, i64>(0)?.max(0) as u64,
                input_tokens: r.get::<_, i64>(1)?.max(0) as u64,
                output_tokens: r.get::<_, i64>(2)?.max(0) as u64,
                cache_read: r.get::<_, i64>(3)?.max(0) as u64,
                cache_write: r.get::<_, i64>(4)?.max(0) as u64,
                cost_usd: r.get(5)?,
            })
        },
    )?)
}

/// The front desk of `product`, or `None` when the store does not know it.
/// `registry` is the text of `agentsi.config.json`, when the data root has one.
pub fn desk(c: &Connection, product: &str, registry: Option<&str>) -> Result<Option<DeskResult>> {
    let Some(found) = read::products(c)?.into_iter().find(|p| p.id == product) else {
        return Ok(None);
    };
    // The registry holds the status of a product, as `agentsi status` shows it.
    let entry = registry.and_then(|r| registry_entry(r, product));
    let status = entry
        .as_ref()
        .and_then(|e| text(e, "status"))
        .or_else(|| found.status.clone());
    let (minutes_total, minutes) = minutes_page(c, product, LAST_MINUTES)?;
    Ok(Some(DeskResult {
        product: product_info(found),
        status,
        roles: entry.map(|e| texts(&e, "roles")).unwrap_or_default(),
        index: index_summary(c, product)?,
        objectives: objectives(c, product)?,
        pending_decisions: pending_decisions(c, product)?,
        minutes_total: minutes_total as u64,
        last_minutes: minutes,
        usage: product_usage(c, product)?,
    }))
}

fn totals(rows: &[UsageInfo]) -> UsageTotals {
    let mut t = UsageTotals {
        rows: rows.len() as u64,
        ..UsageTotals::default()
    };
    for u in rows {
        t.input_tokens += u.input_tokens;
        t.output_tokens += u.output_tokens;
        t.cache_read += u.cache_read;
        t.cache_write += u.cache_write;
        t.cost_usd += u.cost_usd;
    }
    t
}

fn role_rank(role: &str) -> usize {
    let base = role.split('.').next().unwrap_or(role);
    ROLE_ORDER
        .iter()
        .position(|r| *r == base)
        .unwrap_or(ROLE_ORDER.len())
}

fn priority_infos(data: &Value, stored: &[read::Priority]) -> Vec<PriorityInfo> {
    items(data, "priorities")
        .iter()
        .enumerate()
        .map(|(i, p)| {
            // The table wins over the minutes file, as `applyMarks` does in the web app.
            let row = stored.iter().find(|s| s.idx == i as i64);
            PriorityInfo {
                idx: i as u32,
                title: text(p, "title").unwrap_or_default(),
                why: text(p, "why").unwrap_or_default(),
                owner_role: text(p, "owner_role"),
                objective: text(p, "objective"),
                mark: match row {
                    Some(r) => r.mark.clone(),
                    None => text(p, "mark"),
                },
                is_new: match row {
                    Some(r) => r.is_new.unwrap_or(false),
                    None => p.get("is_new").and_then(Value::as_bool).unwrap_or(false),
                },
                sources: items(p, "sources")
                    .iter()
                    .map(|s| SourceRef {
                        kind: text(s, "type").unwrap_or_default(),
                        reference: text(s, "ref").unwrap_or_default(),
                    })
                    .collect(),
            }
        })
        .collect()
}

fn decision_infos(data: &Value, rows: &[read::Decision]) -> Vec<MinutesDecision> {
    // Two decisions with the same text take two rows: a row serves one decision.
    let mut used = vec![false; rows.len()];
    items(data, "decisions")
        .iter()
        .map(|d| {
            let text_of = text(d, "text").unwrap_or_default();
            let row = rows
                .iter()
                .enumerate()
                .find(|(i, r)| !used[*i] && r.text.as_deref() == Some(text_of.as_str()))
                .map(|(i, r)| {
                    used[i] = true;
                    r
                });
            MinutesDecision {
                id: row.map(|r| r.id),
                why: text(d, "why"),
                action: text(d, "action"),
                status: row
                    .and_then(|r| r.status.clone())
                    .or_else(|| text(d, "status"))
                    .unwrap_or_else(|| "pending".into()),
                text: text_of,
            }
        })
        .collect()
}

fn role_answers(data: &Value) -> Vec<RoleAnswerInfo> {
    let mut roles: Vec<RoleAnswerInfo> = data
        .get("roles")
        .and_then(Value::as_object)
        .map(|all| {
            all.iter()
                .map(|(role, a)| RoleAnswerInfo {
                    role: role.clone(),
                    model: text(a, "model"),
                    answer: text(a, "answer").unwrap_or_default(),
                    concerns: texts(a, "concerns"),
                    memory_notes: texts(a, "memory_notes"),
                })
                .collect()
        })
        .unwrap_or_default();
    roles.sort_by(|a, b| {
        role_rank(&a.role)
            .cmp(&role_rank(&b.role))
            .then_with(|| a.role.cmp(&b.role))
    });
    roles
}

fn usage_infos(rows: Vec<Value>) -> Vec<UsageInfo> {
    rows.iter()
        .map(|u| UsageInfo {
            role: text(u, "role").unwrap_or_default(),
            model: text(u, "model").unwrap_or_default(),
            input_tokens: number(u, "input_tokens"),
            output_tokens: number(u, "output_tokens"),
            cache_read: number(u, "cache_read"),
            cache_write: number(u, "cache_write"),
            cost_usd: u.get("cost_usd").and_then(Value::as_f64).unwrap_or(0.),
        })
        .collect()
}

/// One minutes, or `None` when the store has no minutes with this id and a body.
pub fn minutes(c: &Connection, id: &str) -> Result<Option<MinutesDetail>> {
    let Some(data) = read::minutes(c, id)? else {
        return Ok(None);
    };
    let product: Option<String> =
        c.query_row("select product from minutes where id = ?1", [id], |r| {
            r.get(0)
        })?;
    let product = product
        .or_else(|| text(&data, "product"))
        .unwrap_or_default();
    let check = data
        .get("check")
        .filter(|v| v.is_object())
        .map(|k| MinutesCheck {
            passed: k.get("passed").and_then(Value::as_bool).unwrap_or(false),
            repaired: k.get("repaired").and_then(Value::as_bool).unwrap_or(false),
            failures: texts(k, "failures"),
        });
    let usage = usage_infos(read::minutes_usage(c, id)?);
    Ok(Some(MinutesDetail {
        archived: archived(c, &product, "minutes")?.contains(id),
        heard: read::listens(c, &product)?
            .iter()
            .any(|l| l.minutes_id == id),
        check,
        priorities: priority_infos(&data, &read::priorities(c, id)?),
        disagreements: items(&data, "disagreements")
            .iter()
            .map(|d| Disagreement {
                topic: text(d, "topic").unwrap_or_default(),
                positions: items(d, "positions")
                    .iter()
                    .map(|p| Position {
                        role: text(p, "role").unwrap_or_default(),
                        view: text(p, "view").unwrap_or_default(),
                    })
                    .collect(),
            })
            .collect(),
        decisions: decision_infos(&data, &read::decisions_of_minutes(c, id)?),
        roles: role_answers(&data),
        usage_totals: totals(&usage),
        usage,
        id: id.to_string(),
        scope: text(&data, "scope"),
        question: text(&data, "question").unwrap_or_default(),
        status: text(&data, "status"),
        created_at: text(&data, "created_at"),
        product,
    }))
}

#[cfg(test)]
mod tests {
    use super::*;
    use agentsi_store::db::open_writer;
    use serde_json::json;

    fn db(sql: &str) -> (tempfile::TempDir, Connection) {
        let dir = tempfile::tempdir().unwrap();
        let conn = open_writer(&dir.path().join("agentsi.db")).unwrap();
        conn.execute_batch(sql).unwrap();
        (dir, conn)
    }

    const PRODUCT: &str = "insert into products (id, name, paths, status) values
        ('demo', 'Demo', '[\"/src/demo\"]', 'active'), ('other', 'Other', '[]', 'active');";

    #[test]
    fn the_path_is_the_first_folder_of_the_array() {
        assert_eq!(first_path(Some(&json!(["/a", "/b"]))), Some("/a".into()));
        assert_eq!(first_path(Some(&json!(["", "/b"]))), Some("/b".into()));
        assert_eq!(first_path(Some(&json!([]))), None);
        assert_eq!(first_path(Some(&json!("/a"))), None);
        assert_eq!(first_path(None), None);
    }

    #[test]
    fn the_registry_gives_the_roles_of_one_product() {
        let registry = r#"{"products":[{"id":"a","roles":["pm","dev.backend"]},{"id":"b"}]}"#;
        assert_eq!(registry_roles(registry, "a"), ["pm", "dev.backend"]);
        assert!(registry_roles(registry, "b").is_empty());
        assert!(registry_roles(registry, "c").is_empty());
        assert!(registry_roles("not json", "a").is_empty());
    }

    #[test]
    fn an_unknown_product_has_no_desk() {
        let (_d, c) = db(PRODUCT);
        assert!(desk(&c, "nope", None).unwrap().is_none());
    }

    #[test]
    fn the_desk_leaves_out_archived_decisions_and_archived_minutes_as_status_does() {
        let (_d, c) = db(&format!(
            "{PRODUCT}
            insert into minutes (id, product, question, status, created_at, data) values
              ('m1', 'demo', 'First?', 'valid', '2026-11-01T10:00:00.000Z', '{{}}'),
              ('m2', 'demo', 'Second?', 'valid', '2026-11-02T10:00:00.000Z', '{{}}'),
              ('m3', 'demo', 'Third?', 'invalid', '2026-11-03T10:00:00.000Z', '{{}}'),
              ('m4', 'demo', 'Fourth?', 'valid', '2026-11-04T10:00:00.000Z', '{{}}'),
              ('x1', 'other', 'Not mine?', 'valid', '2026-11-05T10:00:00.000Z', '{{}}');
            insert into priorities (minutes_id, idx, title, mark) values
              ('m4', 0, 'a', 'accepted'), ('m4', 1, 'b', null), ('m4', 2, 'c', null);
            insert into decisions (id, product, minutes_id, text, why, status, kind) values
              (1, 'demo', 'm1', 'Ship it', 'because', 'pending', 'minutes'),
              (2, 'demo', 'm1', 'Archived one', null, 'pending', 'minutes'),
              (3, 'demo', null, 'Decided one', null, 'approved', 'product'),
              (4, 'demo', null, 'A product decision', null, 'pending', 'product'),
              (5, 'other', null, 'Of another product', null, 'pending', 'product');
            insert into archive_records (product, kind, ref) values
              ('demo', 'decision', '2'), ('demo', 'minutes', 'm1');
            insert into listens (product, minutes_id, at) values ('demo', 'm4', '2026-11-04T11:00:00.000Z');
            insert into usage (minutes_id, role, cost_usd, input_tokens) values
              ('m4', 'pm', 0.5, 10), ('m2', 'dev', 0.25, 5), ('x1', 'pm', 9, 1);
            insert into documents (product, kind, ref, title, updated_at) values
              ('demo', 'doc', 'a.md', 'A', '2026-11-01T00:00:00.000Z'),
              ('demo', 'commit', 'c1', 'old commit', '2026-10-01T00:00:00.000Z'),
              ('demo', 'commit', 'c2', 'new commit', '2026-10-31T09:00:00.000Z'),
              ('demo', 'issue', '1', 'bug', '2026-10-01T00:00:00.000Z');
            insert into index_state (product, path, indexed_at) values
              ('demo', '/a', '2026-11-01T08:00:00.000Z'), ('demo', '/b', '2026-11-02T08:00:00.000Z');
            insert into briefs (product, data) values
              ('demo', '{{\"objectives\":[{{\"id\":\"o1\",\"text\":\"Ship\",\"signal\":\"s\",\"due\":\"2026-12-01\"}}]}}');"
        ));
        let d = desk(
            &c,
            "demo",
            Some(r#"{"products":[{"id":"demo","status":"planned","roles":["pm","dev"]}]}"#),
        )
        .unwrap()
        .unwrap();
        assert_eq!(d.product.name, "Demo");
        assert_eq!(d.product.path.as_deref(), Some("/src/demo"));
        // The registry wins over the store.
        assert_eq!(d.status.as_deref(), Some("planned"));
        assert_eq!(d.roles, ["pm", "dev"]);
        let pending: Vec<(i64, &str)> = d
            .pending_decisions
            .iter()
            .map(|p| (p.id, p.text.as_str()))
            .collect();
        assert_eq!(pending, [(1, "Ship it"), (4, "A product decision")]);
        assert_eq!(d.pending_decisions[0].minutes_id.as_deref(), Some("m1"));
        assert_eq!(d.minutes_total, 3);
        let last: Vec<&str> = d.last_minutes.iter().map(|m| m.id.as_str()).collect();
        assert_eq!(last, ["m4", "m3", "m2"]);
        assert_eq!(
            d.last_minutes[0].marks,
            MarkCounts {
                accepted: 1,
                unmarked: 2,
                ..Default::default()
            }
        );
        assert!(d.last_minutes[0].heard && !d.last_minutes[1].heard);
        assert_eq!((d.index.docs, d.index.commits, d.index.issues), (1, 2, 1));
        assert_eq!(
            d.index.indexed_at.as_deref(),
            Some("2026-11-02T08:00:00.000Z")
        );
        let last_commit = d.index.last_commit.unwrap();
        assert_eq!(
            (last_commit.date.as_str(), last_commit.title.as_str()),
            ("2026-10-31", "new commit")
        );
        assert_eq!(d.objectives.len(), 1);
        assert_eq!(d.objectives[0].id, "o1");
        assert_eq!((d.usage.rows, d.usage.input_tokens), (2, 15));
        assert!((d.usage.cost_usd - 0.75).abs() < 1e-9);
    }

    #[test]
    fn pending_decisions_come_in_the_order_of_agentsi_desk() {
        let (_d, c) = db(&format!(
            "{PRODUCT}
            insert into minutes (id, product, question, data) values
              ('m-a', 'demo', 'A?', '{{\"status\":\"valid\"}}'),
              ('m-b', 'demo', 'B?', '{{\"status\":\"valid\"}}'),
              ('m-bad', 'demo', 'C?', '{{\"status\":\"invalid\"}}');
            insert into decisions (id, product, minutes_id, text, status, kind) values
              (1, 'demo', 'm-b', 'of b, first id', 'pending', 'minutes'),
              (2, 'demo', null, 'product decision', 'pending', 'product'),
              (3, 'demo', 'm-a', 'of a', 'pending', 'minutes'),
              (4, 'demo', 'm-bad', 'of an invalid minutes', 'pending', 'minutes'),
              (5, 'demo', 'm-b', 'of b, second id', 'pending', 'minutes');"
        ));
        let d = desk(&c, "demo", None).unwrap().unwrap();
        let ids: Vec<i64> = d.pending_decisions.iter().map(|p| p.id).collect();
        assert_eq!(ids, [3, 1, 5, 2, 4]);
    }

    #[test]
    fn a_decision_links_the_unmarked_priorities_of_its_minutes() {
        let data = r#"{"status":"valid",
            "priorities":[{"mark":null},{"mark":"accepted"},{"mark":""},{"mark":null}],
            "decisions":[{"text":"link","priorities":[0,1,2,2,3,9]}]}"#;
        let (_d, c) = db(&format!(
            "{PRODUCT}
            insert into minutes (id, product, question, data) values ('m-a', 'demo', 'A?', '{}');
            insert into decisions (id, product, minutes_id, text, status, kind)
              values (1, 'demo', 'm-a', 'link', 'pending', 'minutes');",
            data.replace('\n', " ")
        ));
        let d = desk(&c, "demo", None).unwrap().unwrap();
        // The accepted priority and the index out of range are left out. An empty mark is no mark.
        assert_eq!(d.pending_decisions[0].priorities, [0, 2, 3]);
    }

    #[test]
    fn a_long_list_has_the_same_marks_as_a_short_one() {
        let mut sql = String::from(PRODUCT);
        for i in 0..12 {
            sql += &format!(
                "insert into minutes (id, product, question, created_at) values ('m{i:02}', 'demo', 'Q{i}', '2026-11-{:02}T10:00:00.000Z');",
                i + 1
            );
        }
        sql += "insert into priorities (minutes_id, idx, title, mark) values
                ('m11', 0, 'a', 'rejected'), ('m11', 1, 'b', 'changed'), ('m11', 2, 'c', null),
                ('m03', 0, 'a', 'accepted');";
        let (_d, c) = db(&sql);
        let all = minutes_rows(&c, "demo").unwrap();
        assert_eq!(all.len(), 12);
        assert_eq!(all[0].id, "m11");
        let marks = |id: &str| all.iter().find(|m| m.id == id).unwrap().marks;
        assert_eq!(
            marks("m11"),
            MarkCounts {
                changed: 1,
                rejected: 1,
                unmarked: 1,
                accepted: 0
            }
        );
        assert_eq!(marks("m03").accepted, 1);
        assert_eq!(marks("m00"), MarkCounts::default());
        // The desk takes 3 rows with the same marks.
        let d = desk(&c, "demo", None).unwrap().unwrap();
        assert_eq!(d.minutes_total, 12);
        assert_eq!(d.last_minutes[0].marks, marks("m11"));
    }

    #[test]
    fn two_decisions_with_the_same_text_take_two_rows() {
        let body = r#"{"decisions":[{"text":"Ship it"},{"text":"Ship it"},{"text":"Ship it"}]}"#;
        let (_d, c) = db(&format!(
            "{PRODUCT}
            insert into minutes (id, product, question, data) values ('m1', 'demo', 'Q?', '{body}');
            insert into decisions (id, product, minutes_id, text, status, kind) values
              (4, 'demo', 'm1', 'Ship it', 'approved', 'minutes'),
              (9, 'demo', 'm1', 'Ship it', 'pending', 'minutes');"
        ));
        let m = minutes(&c, "m1").unwrap().unwrap();
        let seen: Vec<(Option<i64>, &str)> = m
            .decisions
            .iter()
            .map(|d| (d.id, d.status.as_str()))
            .collect();
        assert_eq!(
            seen,
            [
                (Some(4), "approved"),
                (Some(9), "pending"),
                (None, "pending")
            ]
        );
    }

    #[test]
    fn a_product_with_no_data_has_an_empty_desk() {
        let (_d, c) = db(PRODUCT);
        let d = desk(&c, "other", None).unwrap().unwrap();
        assert!(d.pending_decisions.is_empty() && d.last_minutes.is_empty());
        assert_eq!(d.minutes_total, 0);
        assert!(d.index.indexed_at.is_none() && d.index.last_commit.is_none());
        assert_eq!(d.usage, UsageTotals::default());
    }

    #[test]
    fn one_minutes_takes_marks_and_decision_status_from_the_tables() {
        let body = json!({
            "id": "m1", "product": "demo", "scope": "app", "question": "What now?",
            "created_at": "2026-11-02T10:00:00.000Z", "status": "valid",
            "roles": {
                "scout": {"model": "haiku", "answer": "Look around", "concerns": ["c1"], "memory_notes": []},
                "pm": {"model": "opus", "answer": "Ship", "concerns": [], "memory_notes": ["n1"]},
                "dev.backend": {"answer": "Fix the api"}
            },
            "priorities": [
                {"title": "P0", "why": "w0", "owner_role": "pm", "objective": "o1", "mark": "accepted",
                 "sources": [{"type": "commit", "ref": "abc"}, {"type": "url", "ref": "https://x"}]},
                {"title": "P1", "why": "w1", "owner_role": "dev", "mark": null, "is_new": true, "sources": []}
            ],
            "disagreements": [{"topic": "T", "positions": [{"role": "pm", "view": "yes"}, {"role": "dev", "view": "no"}]}],
            "decisions": [
                {"text": "Ship it", "why": "w", "action": "do it"},
                {"text": "No row", "status": "rejected"},
                {"text": "Also no row"}
            ],
            "check": {"passed": false, "failures": ["f1"], "repaired": true}
        });
        let (_d, c) = db(&format!(
            "{PRODUCT}
            insert into minutes (id, product, question, status, created_at, data, usage) values
              ('m1', 'demo', 'What now?', 'valid', '2026-11-02T10:00:00.000Z', '{}',
               '[{{\"role\":\"pm\",\"model\":\"opus\",\"input_tokens\":100,\"output_tokens\":50,\"cache_read\":7,\"cache_write\":3,\"cost_usd\":0.5}},
                 {{\"role\":\"dev\",\"model\":\"sonnet\",\"input_tokens\":10,\"output_tokens\":5,\"cache_read\":0,\"cache_write\":0,\"cost_usd\":0.25}}]');
            insert into priorities (minutes_id, idx, title, mark, is_new) values
              ('m1', 0, 'P0', 'changed', 0), ('m1', 1, 'P1', 'rejected', 1);
            insert into decisions (id, product, minutes_id, text, status, kind) values
              (7, 'demo', 'm1', 'Ship it', 'approved', 'minutes');
            insert into archive_records (product, kind, ref) values ('demo', 'minutes', 'm1');",
            body.to_string().replace('\'', "''")
        ));
        let m = minutes(&c, "m1").unwrap().unwrap();
        assert_eq!(
            (m.product.as_str(), m.question.as_str(), m.scope.as_deref()),
            ("demo", "What now?", Some("app"))
        );
        assert!(m.archived && !m.heard);
        let check = m.check.unwrap();
        assert!(!check.passed && check.repaired);
        assert_eq!(check.failures, ["f1"]);
        assert_eq!(m.priorities[0].mark.as_deref(), Some("changed"));
        assert_eq!(m.priorities[0].sources.len(), 2);
        assert_eq!(m.priorities[0].sources[1].kind, "url");
        assert_eq!(m.priorities[0].objective.as_deref(), Some("o1"));
        assert_eq!(m.priorities[1].mark.as_deref(), Some("rejected"));
        assert!(m.priorities[1].is_new);
        let decisions: Vec<(Option<i64>, &str)> = m
            .decisions
            .iter()
            .map(|d| (d.id, d.status.as_str()))
            .collect();
        assert_eq!(
            decisions,
            [(Some(7), "approved"), (None, "rejected"), (None, "pending")]
        );
        assert_eq!(m.disagreements[0].positions.len(), 2);
        let roles: Vec<&str> = m.roles.iter().map(|r| r.role.as_str()).collect();
        assert_eq!(roles, ["pm", "dev.backend", "scout"]);
        assert_eq!(m.roles[0].memory_notes, ["n1"]);
        assert_eq!(m.usage.len(), 2);
        assert_eq!(
            (
                m.usage_totals.input_tokens,
                m.usage_totals.cache_read,
                m.usage_totals.rows
            ),
            (110, 7, 2)
        );
        assert!((m.usage_totals.cost_usd - 0.75).abs() < 1e-9);
    }

    #[test]
    fn a_minutes_row_without_a_body_or_an_unknown_id_is_none() {
        let (_d, c) = db(&format!(
            "{PRODUCT} insert into minutes (id, product, question) values ('old', 'demo', 'Old?');"
        ));
        assert!(minutes(&c, "old").unwrap().is_none());
        assert!(minutes(&c, "nope").unwrap().is_none());
    }
}
