//! `mark.*`, `decision.*` and `archive.*`: the writes of the owner (U20). The
//! logic is in `agentsi-agency`. Each handler runs one transaction on the
//! writer thread of the store, off the event loop, and sends `data.changed`
//! for each product that it changed once the transaction committed.

use super::{Ctx, Router};
use agentsi_agency::archive as archive_records;
use agentsi_agency::marks::{self, Error as MarksError, decide, mark, pending};
use agentsi_agency::registry::Registry;
use agentsi_agency::{Error as AgencyError, clock};
use agentsi_proto::RpcError;
use agentsi_proto::events::{DataChanged, DataChangedParams};
use agentsi_proto::marks_api::{
    ArchiveArchive, ArchiveParams, ArchiveRestore, ArchiveResult, ArchivedDecision, DecideParams,
    DecideResult, DecisionAdd, DecisionAddParams, DecisionAddResult, DecisionArchived,
    DecisionArchivedParams, DecisionArchivedResult, DecisionDecide, DecisionGet, DecisionGetParams,
    DecisionGetResult, DecisionInfo, MarkPending, MarkPendingParams, MarkPendingResult, MarkSet,
    MarkSetParams, MarkSetResult, Outcome,
};
use agentsi_proto::rpc::ErrorKind;
use agentsi_store::Store;
use rusqlite::Transaction;
use std::sync::Arc;

pub fn register(r: Router) -> Router {
    r.add(MarkPending, mark_pending)
        .add(MarkSet, mark_set)
        .add(DecisionDecide, decision_decide)
        .add(DecisionAdd, decision_add)
        .add(DecisionArchived, decision_archived)
        .add(DecisionGet, decision_get)
        .add(ArchiveArchive, archive_archive)
        .add(ArchiveRestore, archive_restore)
}

fn rpc(e: MarksError) -> RpcError {
    match e {
        MarksError::NotFound(m) => RpcError::new(
            ErrorKind::NotFound,
            m,
            "Use mark.pending or minutes.list for the ids that exist.",
        ),
        MarksError::Invalid(m) => RpcError::new(
            ErrorKind::Invalid,
            m,
            "Fix the id or the marks and try again.",
        ),
        MarksError::Conflict(m) => RpcError::new(
            ErrorKind::Conflict,
            m,
            "Read the state again with mark.pending, then decide what to do.",
        ),
        MarksError::Unavailable(m) => RpcError::new(
            ErrorKind::Unavailable,
            m,
            "Set AGENTSI_HOME to the data root. It holds agentsi.config.json and agentsi.db.",
        ),
    }
}

fn agency(e: AgencyError) -> RpcError {
    match e {
        AgencyError::NotFound(m) => rpc(MarksError::NotFound(m)),
        AgencyError::Unavailable(m) => rpc(MarksError::Unavailable(m)),
    }
}

fn join(e: tokio::task::JoinError) -> RpcError {
    RpcError::internal(e.to_string())
}

fn no_database() -> RpcError {
    rpc(MarksError::Unavailable(
        "the data root has no agentsi.db. Import the data first.".into(),
    ))
}

/// The registry of the data root, or none when it cannot be read.
fn registry() -> Result<Registry, RpcError> {
    let root = agentsi_store::paths::data_root(None).map_err(|e| rpc(e.into()))?;
    Registry::load(&root).map_err(agency)
}

fn open_store(core: &crate::state::Core) -> Result<Arc<Store>, RpcError> {
    core.store()
        .map_err(|e| rpc(e.into()))?
        .ok_or_else(no_database)
}

/// The product must be in the registry of the data root.
async fn known_product(id: String) -> Result<(), RpcError> {
    crate::blocking::spawn(move || registry()?.product(&id).map(|_| ()).map_err(agency))
        .await
        .map_err(join)?
}

/// Runs `f` in one transaction on the writer thread. A failure rolls it back.
async fn write<T: Send + 'static>(
    ctx: &Ctx,
    f: impl FnOnce(&Transaction) -> marks::Result<T> + Send + 'static,
) -> Result<T, RpcError> {
    let core = ctx.core.clone();
    crate::blocking::spawn(move || {
        let store = open_store(&core)?;
        store.try_write(f).map_err(|e| rpc(e.into()))?.map_err(rpc)
    })
    .await
    .map_err(join)?
}

fn changed(ctx: &Ctx, product: &str, kind: &str) {
    ctx.core.publish::<DataChanged>(&DataChangedParams {
        product: product.to_string(),
        kind: kind.to_string(),
    });
}

/// Stores a desk note for the product. It waits for the store, so the next owner message
/// carries the note, and it never fails the write of the owner.
async fn desk_note(ctx: &Ctx, product: &str, text: String) {
    let (core, product) = (ctx.core.clone(), product.to_string());
    let _ = crate::blocking::spawn(move || crate::desk::note(&core, &product, &text)).await;
}

async fn mark_pending(ctx: Ctx, p: MarkPendingParams) -> Result<MarkPendingResult, RpcError> {
    crate::blocking::spawn(move || {
        registry()?.product(&p.product).map_err(agency)?;
        let store = open_store(&ctx.core)?;
        store
            .read(|c| Ok(pending::pending(c, &p.product)))
            .map_err(|e| rpc(e.into()))
    })
    .await
    .map_err(join)?
}

async fn mark_set(ctx: Ctx, p: MarkSetParams) -> Result<MarkSetResult, RpcError> {
    let unmarked_only = p.unmarked_only;
    let marks: Vec<String> = p
        .entries
        .iter()
        .map(|e| {
            format!(
                "priority {} of minutes {} as {}",
                e.idx + 1,
                e.minutes_id,
                e.mark.as_str()
            )
        })
        .collect();
    let done = write(&ctx, move |tx| mark::set(tx, &p.entries, unmarked_only)).await?;
    for product in &done.products {
        changed(&ctx, product, "minutes");
        desk_note(
            &ctx,
            product,
            format!("The owner marked {}.", marks.join(", ")),
        )
        .await;
    }
    Ok(MarkSetResult {
        marked: done.marked,
    })
}

async fn decision_decide(ctx: Ctx, p: DecideParams) -> Result<DecideResult, RpcError> {
    let products: Vec<String> = registry()
        .map(|r| r.products.iter().map(|x| x.id.clone()).collect())
        .unwrap_or_default();
    let id = p.decision;
    let verdict = p.status;
    let done = write(&ctx, move |tx| {
        if archive_records::archived_decision_product(tx, &products, id)?.is_some() {
            return Err(MarksError::Conflict(format!(
                "Decision {id} is archived. Run agentsi restore {id} first, then decide."
            )));
        }
        decide::decide(tx, id, verdict, clock::now_ms())
    })
    .await?;
    changed(&ctx, &done.product, "decision");
    if done.minutes_id.is_some() {
        changed(&ctx, &done.product, "minutes");
    }
    let note = match done.status {
        Outcome::Approved => format!("The owner approved decision {id}."),
        Outcome::Rejected => format!("The owner rejected decision {id}."),
        Outcome::Stale => {
            format!("The owner approved decision {id}, but its target changed: it is stale.")
        }
    };
    desk_note(&ctx, &done.product, note).await;
    Ok(done)
}

async fn decision_add(ctx: Ctx, p: DecisionAddParams) -> Result<DecisionAddResult, RpcError> {
    let (text, why) = (p.text.trim().to_string(), p.why.trim().to_string());
    if text.is_empty() || why.is_empty() {
        return Err(rpc(MarksError::Invalid(
            "A decision needs a text and a why.".into(),
        )));
    }
    let action = p.action.map(|a| a.trim().to_string());
    known_product(p.product.clone()).await?;
    let product = p.product;
    let note_text = text.clone();
    let id = {
        let product = product.clone();
        write(&ctx, move |tx| {
            decide::add(tx, &product, &text, &why, action.as_deref())
        })
        .await?
    };
    changed(&ctx, &product, "decision");
    desk_note(
        &ctx,
        &product,
        format!("The owner added the pending decision {id}: {note_text}"),
    )
    .await;
    Ok(DecisionAddResult { id })
}

async fn decision_archived(
    ctx: Ctx,
    p: DecisionArchivedParams,
) -> Result<DecisionArchivedResult, RpcError> {
    crate::blocking::spawn(move || {
        registry()?.product(&p.product).map_err(agency)?;
        let store = open_store(&ctx.core)?;
        let rows = store
            .read(|c| agentsi_store::read::archived_decisions(c, &p.product))
            .map_err(|e| rpc(e.into()))?;
        Ok(DecisionArchivedResult {
            decisions: rows
                .into_iter()
                .map(|d| ArchivedDecision {
                    id: d.id,
                    kind: d.kind,
                    minutes_id: d.minutes_id,
                    text: d.text,
                    why: d.why,
                    action: d.action,
                    status: d.status,
                })
                .collect(),
        })
    })
    .await
    .map_err(join)?
}

async fn decision_get(ctx: Ctx, p: DecisionGetParams) -> Result<DecisionGetResult, RpcError> {
    crate::blocking::spawn(move || {
        registry()?.product(&p.product).map_err(agency)?;
        let store = open_store(&ctx.core)?;
        let (product, ids) = (p.product.clone(), p.ids.clone());
        let (rows, records) = store
            .read(move |c| {
                Ok((
                    agentsi_store::read::decisions_by_id(c, &product, &ids)?,
                    agentsi_store::read::archived(c, &product)?,
                ))
            })
            .map_err(|e| rpc(e.into()))?;
        let archived: Vec<String> = records
            .into_iter()
            .filter(|r| r.kind == "decision")
            .map(|r| r.reference)
            .collect();
        let decisions = p
            .ids
            .iter()
            .filter_map(|id| rows.iter().find(|d| d.id == *id))
            .map(|d| DecisionInfo {
                id: d.id,
                kind: d.kind.clone(),
                minutes_id: d.minutes_id.clone(),
                text: d.text.clone().unwrap_or_default(),
                why: d.why.clone(),
                action: d.action.clone(),
                status: d.status.clone(),
                archived: archived.contains(&d.id.to_string()),
            })
            .collect();
        Ok(DecisionGetResult { decisions })
    })
    .await
    .map_err(join)?
}

async fn archive_change(
    ctx: Ctx,
    p: ArchiveParams,
    archive: bool,
) -> Result<ArchiveResult, RpcError> {
    known_product(p.product.clone()).await?;
    let (kind, id) = (p.kind, p.id.clone());
    let product = p.product.clone();
    write(&ctx, move |tx| {
        archive_records::change(tx, &product, kind, id.as_deref(), archive)
    })
    .await?;
    changed(&ctx, &p.product, kind.as_str());
    let verb = if archive { "archived" } else { "restored" };
    let what = match &p.id {
        Some(id) => format!("{} {id}", kind.as_str()),
        None => format!("the {}", kind.as_str()),
    };
    desk_note(&ctx, &p.product, format!("The owner {verb} {what}.")).await;
    Ok(ArchiveResult {
        product: p.product,
        kind: p.kind,
        id: p.id,
    })
}

async fn archive_archive(ctx: Ctx, p: ArchiveParams) -> Result<ArchiveResult, RpcError> {
    archive_change(ctx, p, true).await
}

async fn archive_restore(ctx: Ctx, p: ArchiveParams) -> Result<ArchiveResult, RpcError> {
    archive_change(ctx, p, false).await
}
