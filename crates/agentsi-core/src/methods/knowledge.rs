//! `knowledge.index`, `knowledge.search`, `knowledge.get_doc`,
//! `knowledge.recent_minutes`, `knowledge.pending_decisions` and
//! `knowledge.role_memory`. The logic is in `agentsi_agency::knowledge`.
//! `knowledge.index` writes through the writer thread of the store. The other
//! methods only read.

use super::{Ctx, Router};
use agentsi_agency::knowledge::{memory_text, query};
use agentsi_agency::registry::{Product, Registry};
use agentsi_agency::{Error as AgencyError, knowledge};
use agentsi_proto::RpcError;
use agentsi_proto::events::{DataChanged, DataChangedParams};
use agentsi_proto::knowledge_api::{
    KbDoc, KbGetDocParams, KbIndexParams, KbIndexResult, KbPendingParams, KbPendingResult,
    KbRecentMinutesParams, KbRecentMinutesResult, KbRoleMemoryParams, KbRoleMemoryResult,
    KbSearchParams, KbSearchResult, KnowledgeGetDoc, KnowledgeIndex, KnowledgePendingDecisions,
    KnowledgeRecentMinutes, KnowledgeRoleMemory, KnowledgeSearch,
};
use agentsi_proto::rpc::ErrorKind;
use agentsi_store::Store;
use rusqlite::Connection;
use std::sync::Arc;

const KINDS: [&str; 6] = ["doc", "journal", "commit", "issue", "minutes", "memory"];
const DEFAULT_LIMIT: u32 = 5;

pub fn register(r: Router) -> Router {
    r.add(KnowledgeIndex, index)
        .add(KnowledgeSearch, search)
        .add(KnowledgeGetDoc, get_doc)
        .add(KnowledgeRecentMinutes, recent_minutes)
        .add(KnowledgePendingDecisions, pending_decisions)
        .add(KnowledgeRoleMemory, role_memory)
}

fn rpc(e: AgencyError) -> RpcError {
    match e {
        AgencyError::NotFound(m) => RpcError::new(
            ErrorKind::NotFound,
            m,
            "Use product.whoami with list: true for the products, and knowledge.search for the refs.",
        ),
        AgencyError::Unavailable(m) => RpcError::new(
            ErrorKind::Unavailable,
            m,
            "Set AGENTSI_HOME to the data root. It holds agentsi.config.json and agentsi.db.",
        ),
    }
}

/// Runs `f` off the event loop, with the store and the product of the registry.
async fn with_product<T: Send + 'static>(
    ctx: &Ctx,
    product: String,
    f: impl FnOnce(&Store, &Product) -> Result<T, AgencyError> + Send + 'static,
) -> Result<T, RpcError> {
    let core = ctx.core.clone();
    crate::blocking::spawn(move || {
        let root = agentsi_store::paths::data_root(None).map_err(|e| rpc(e.into()))?;
        let registry = Registry::load(&root).map_err(rpc)?;
        let found = registry.product(&product).map_err(rpc)?.clone();
        let store: Arc<Store> = core.store().map_err(|e| rpc(e.into()))?.ok_or_else(|| {
            rpc(AgencyError::Unavailable(
                "the data root has no agentsi.db. Import the data first.".into(),
            ))
        })?;
        f(&store, &found).map_err(rpc)
    })
    .await
    .map_err(|e| RpcError::internal(e.to_string()))?
}

/// Like `with_product`, on a reader connection.
async fn reading<T: Send + 'static>(
    ctx: &Ctx,
    product: String,
    f: impl FnOnce(&Connection, &Product) -> Result<T, AgencyError> + Send + 'static,
) -> Result<T, RpcError> {
    with_product(ctx, product, move |store, found| {
        store.read(|c| Ok(f(c, found))).map_err(AgencyError::from)?
    })
    .await
}

async fn index(ctx: Ctx, p: KbIndexParams) -> Result<KbIndexResult, RpcError> {
    let result = with_product(&ctx, p.product, move |store, product| {
        knowledge::index_product(store, product, p.force)
    })
    .await?;
    ctx.core.publish::<DataChanged>(&DataChangedParams {
        product: result.product.clone(),
        kind: "knowledge".into(),
    });
    Ok(result)
}

fn invalid(message: String, hint: &str) -> RpcError {
    RpcError::new(ErrorKind::Invalid, message, hint)
}

fn unknown_kind(kind: &str) -> RpcError {
    invalid(
        format!("Unknown kind \"{kind}\"."),
        "Use doc, journal, commit, issue, minutes or memory.",
    )
}

async fn search(ctx: Ctx, p: KbSearchParams) -> Result<KbSearchResult, RpcError> {
    if let Some(kind) = &p.kind
        && !KINDS.contains(&kind.as_str())
    {
        return Err(unknown_kind(kind));
    }
    let limit = p.limit.unwrap_or(DEFAULT_LIMIT).clamp(1, 50);
    reading(&ctx, p.product, move |c, product| {
        let (hits, any_term) =
            query::search_with_fallback(c, &product.id, &p.query, p.kind.as_deref(), limit)?;
        Ok(KbSearchResult { hits, any_term })
    })
    .await
}

async fn get_doc(ctx: Ctx, p: KbGetDocParams) -> Result<KbDoc, RpcError> {
    if !KINDS.contains(&p.kind.as_str()) {
        return Err(unknown_kind(&p.kind));
    }
    reading(&ctx, p.product, move |c, product| {
        query::get_doc(c, &product.id, &p.kind, &p.reference)
    })
    .await
}

async fn recent_minutes(
    ctx: Ctx,
    p: KbRecentMinutesParams,
) -> Result<KbRecentMinutesResult, RpcError> {
    reading(&ctx, p.product, move |c, product| {
        Ok(KbRecentMinutesResult {
            minutes: query::recent_minutes(c, &product.id, p.limit, p.include_archived)?,
        })
    })
    .await
}

async fn pending_decisions(ctx: Ctx, p: KbPendingParams) -> Result<KbPendingResult, RpcError> {
    reading(&ctx, p.product, move |c, product| {
        query::pending_decisions(c, &product.id, p.limit)
    })
    .await
}

async fn role_memory(ctx: Ctx, p: KbRoleMemoryParams) -> Result<KbRoleMemoryResult, RpcError> {
    if !memory_text::is_agent_id(&p.role) {
        return Err(invalid(
            format!("Invalid agent id \"{}\"", p.role),
            "An agent id is a role such as pm, or a specialist such as dev.backend.",
        ));
    }
    reading(&ctx, p.product, move |c, product| {
        Ok(KbRoleMemoryResult {
            text: query::role_memory(c, &product.id, &p.role)?,
        })
    })
    .await
}
