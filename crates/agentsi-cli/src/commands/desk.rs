//! `agentsi desk`: the product, the index line, the status and the pending
//! decisions as one JSON object. It updates the index first, as the
//! TypeScript `desk` does.

use super::index::index_line;
use super::{Fail, product_only, resolve_product};
use crate::core::Core;
use crate::json;
use agentsi_proto::agency_api::{ProductIdentity, ProductStatus, StatusParams};
use agentsi_proto::desk_api::{ProductDesk, ProductDeskParams};
use agentsi_proto::knowledge_api::{KbIndexParams, KnowledgeIndex};
use serde::Serialize;

pub const USAGE: &str = "agentsi desk [--product <id>]";

#[derive(Serialize)]
#[serde(untagged)]
enum Step {
    Index { ok: bool, summary: String },
    Status { ok: bool, markdown: String },
    Failed { ok: bool, error: String },
}

#[derive(Serialize)]
struct Pending {
    kind: &'static str,
    id: i64,
    minutes_id: Option<String>,
    text: String,
    why: Option<String>,
    action: Option<String>,
    priorities: Vec<u32>,
}

#[derive(Serialize)]
struct Out {
    #[serde(flatten)]
    identity: ProductIdentity,
    index: Step,
    status: Step,
    pending_decisions: Vec<Pending>,
    #[serde(skip_serializing_if = "Option::is_none")]
    decisions_error: Option<String>,
}

pub fn run(argv: &[String]) -> Result<i32, Fail> {
    let flag = product_only(argv)?;
    let mut core = Core::connect()?;
    let identity = resolve_product(&mut core, flag.as_deref())?;
    let id = identity.product.clone();

    let index = match core.call::<KnowledgeIndex>(&KbIndexParams {
        product: id.clone(),
        force: false,
    }) {
        Ok(r) => Step::Index {
            ok: true,
            summary: index_line(&r),
        },
        Err(error) => Step::Failed { ok: false, error },
    };
    let status = match core.call::<ProductStatus>(&StatusParams {
        product: id.clone(),
    }) {
        Ok(r) => Step::Status {
            ok: true,
            markdown: r.markdown,
        },
        Err(error) => Step::Failed { ok: false, error },
    };
    let ok = matches!(index, Step::Index { .. }) && matches!(status, Step::Status { .. });

    let (pending_decisions, decisions_error) =
        match core.call::<ProductDesk>(&ProductDeskParams { product: id }) {
            Ok(desk) => (
                desk.pending_decisions
                    .into_iter()
                    .map(|d| Pending {
                        kind: "decision",
                        id: d.id,
                        minutes_id: d.minutes_id,
                        text: d.text,
                        why: d.why,
                        action: d.action,
                        priorities: d.priorities,
                    })
                    .collect(),
                None,
            ),
            Err(error) => (Vec::new(), Some(error)),
        };
    println!(
        "{}",
        json::pretty(&Out {
            identity,
            index,
            status,
            pending_decisions,
            decisions_error
        })
    );
    Ok(i32::from(!ok))
}
