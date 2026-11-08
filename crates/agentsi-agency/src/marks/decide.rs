//! `agentsi decide` and `agentsi decision add`. A port of `decide` and
//! `addProductDecision` of `minutes.ts`.

use super::{Error, Result};
use crate::clock;
use crate::reflect::{Settled, settle};
use agentsi_proto::marks_api::{DecideResult, Outcome, Verdict};
use rusqlite::{OptionalExtension, Transaction, params};
use serde_json::Value;
use std::path::Path;

/// Settles the proposal of a reflection decision, then records what came of it on
/// the decision. A role file and its commit come after the last row, so a failed
/// write rolls each row back.
///
/// A reflection with no proposal rejects the decision. The reason travels in the
/// result, not as an error, because an error would roll the row back.
fn settle_reflection(
    tx: &Transaction,
    id: i64,
    product: String,
    reflection: Option<String>,
    status: Verdict,
    now_ms: i64,
    root: Option<&Path>,
) -> Result<DecideResult> {
    let Some(reflection) = reflection else {
        return Err(Error::Unavailable(format!(
            "Decision {id} is a reflection decision without a reflection id"
        )));
    };
    let root = agentsi_store::paths::data_root(root)?;
    let approve = status == Verdict::Approved;
    let mut refused = None;
    let (outcome, role_file) = match settle(tx, &root, &product, &reflection, approve)? {
        Settled::Approved { role_file } => (Outcome::Approved, role_file),
        Settled::Rejected => (Outcome::Rejected, None),
        Settled::MissingProposal(reason) => {
            refused = Some(format!(
                "Decision {id} rejected: {reason} It can never be applied."
            ));
            (Outcome::Rejected, None)
        }
        Settled::Stale => (Outcome::Stale, None),
        Settled::Refused(reason) => return Err(Error::Invalid(reason)),
    };
    let name = match outcome {
        Outcome::Approved => "approved",
        Outcome::Rejected => "rejected",
        Outcome::Stale => "stale",
    };
    tx.execute(
        "update decisions set status = ?2, decided_at = ?3 where id = ?1",
        params![id, name, clock::iso(now_ms)],
    )?;
    let commit = match role_file {
        Some(file) => Some(file.write(&root).map_err(Error::Unavailable)?),
        None => None,
    };
    Ok(DecideResult {
        product,
        minutes_id: None,
        status: outcome,
        action: None,
        commit_error: commit.and_then(|c| c.error),
        refused,
    })
}

/// Records `status` for a decision. The `decisions` row and the same decision
/// inside the minutes JSON change in the one transaction of the caller. A
/// reflection decision settles its reflection.
pub fn decide(tx: &Transaction, id: i64, status: Verdict, now_ms: i64) -> Result<DecideResult> {
    decide_at(tx, id, status, now_ms, None)
}

/// `decide` with a data root of its own, for a test. `None` is the root of the core.
pub(crate) fn decide_at(
    tx: &Transaction,
    id: i64,
    status: Verdict,
    now_ms: i64,
    root: Option<&Path>,
) -> Result<DecideResult> {
    let target: Option<(String, String, Option<String>)> = tx
        .query_row(
            "select kind, product, reflection_id from decisions where id = ?1",
            [id],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
        )
        .optional()?;
    let Some((kind, product, reflection)) = target else {
        return Err(Error::NotFound(format!("Decision {id} not found")));
    };
    if kind == "reflection" {
        return settle_reflection(tx, id, product, reflection, status, now_ms, root);
    }

    let at = clock::iso(now_ms);
    let (product, minutes_id, action, pos): (String, Option<String>, Option<String>, i64) = tx
        .query_row(
            "update decisions set status = ?2, decided_at = ?3 where id = ?1
             returning product, minutes_id, action,
               (select count(*) from decisions d2 where d2.minutes_id = decisions.minutes_id and d2.id < decisions.id)",
            params![id, status.as_str(), at],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)),
        )
        .optional()?
        .ok_or_else(|| Error::NotFound(format!("Decision {id} not found")))?;

    if let Some(minutes) = &minutes_id {
        let data: Option<String> = tx
            .query_row(
                "select data from minutes where id = ?1 and data is not null",
                [minutes],
                |r| r.get(0),
            )
            .optional()?;
        let data =
            data.ok_or_else(|| Error::NotFound(format!("Minutes \"{minutes}\" not found")))?;
        let mut json: Value = serde_json::from_str(&data)?;
        if let Some(d) = json
            .get_mut("decisions")
            .and_then(|l| l.get_mut(pos as usize))
            .and_then(Value::as_object_mut)
        {
            d.insert("status".into(), Value::String(status.as_str().into()));
            d.insert("decided_at".into(), Value::String(at));
            tx.execute(
                "update minutes set data = ?2 where id = ?1",
                params![minutes, serde_json::to_string(&json)?],
            )?;
        }
    }
    Ok(DecideResult {
        product,
        minutes_id,
        status: match status {
            Verdict::Approved => Outcome::Approved,
            Verdict::Rejected => Outcome::Rejected,
        },
        action,
        commit_error: None,
        refused: None,
    })
}

/// A pending decision outside any meeting (`kind` product). Returns its id.
pub fn add(
    tx: &Transaction,
    product: &str,
    text: &str,
    why: &str,
    action: Option<&str>,
) -> Result<i64> {
    let action = action.filter(|a| !a.is_empty());
    Ok(tx.query_row(
        "insert into decisions (product, kind, minutes_id, text, why, action, status)
         values (?1, 'product', null, ?2, ?3, ?4, 'pending') returning id",
        params![product, text, why, action],
        |r| r.get(0),
    )?)
}
