//! `product.whoami`, `product.status`, `data.snapshot` and `data.audit`: the
//! methods of `agentsi whoami`, `status`, `snapshot` and `audit`. The logic is
//! in `agentsi-agency`. The handlers read the registry of the data root and
//! the store, off the event loop, and never write.

use super::{Ctx, Router};
use agentsi_agency::registry::{self, Registry};
use agentsi_agency::{Error as AgencyError, audit, clock, snapshot, status};
use agentsi_proto::RpcError;
use agentsi_proto::agency_api::{
    AuditParams, AuditReport, DataAudit, DataSnapshot, ListedProduct, ProductStatus, ProductWhoami,
    SnapshotParams, SnapshotResult, StatusParams, StatusResult, WhoamiParams, WhoamiResult,
};
use agentsi_proto::rpc::ErrorKind;
use rusqlite::Connection;
use std::path::{Path, PathBuf};
use std::sync::Arc;

pub fn register(r: Router) -> Router {
    r.add(ProductWhoami, whoami)
        .add(ProductStatus, product_status)
        .add(DataSnapshot, data_snapshot)
        .add(DataAudit, data_audit)
}

fn rpc(e: AgencyError) -> RpcError {
    match e {
        AgencyError::NotFound(m) => RpcError::new(
            ErrorKind::NotFound,
            m,
            "Use product.whoami with list: true for the products of the registry.",
        ),
        AgencyError::Unavailable(m) => RpcError::new(
            ErrorKind::Unavailable,
            m,
            "Set AGENTSI_HOME to the data root. It holds agentsi.config.json and agentsi.db.",
        ),
    }
}

fn root() -> Result<PathBuf, RpcError> {
    agentsi_store::paths::data_root(None).map_err(|e| rpc(e.into()))
}

/// Runs `f` off the event loop, with the data root and the registry.
async fn blocking<T: Send + 'static>(
    f: impl FnOnce(&Path, &Registry) -> Result<T, AgencyError> + Send + 'static,
) -> Result<T, RpcError> {
    crate::blocking::spawn(move || {
        let root = root()?;
        let registry = Registry::load(&root).map_err(rpc)?;
        f(&root, &registry).map_err(rpc)
    })
    .await
    .map_err(|e| RpcError::internal(e.to_string()))?
}

/// Like `blocking`, with a reader connection of the store. A data root with
/// no `agentsi.db` is `unavailable`.
async fn with_store<T: Send + 'static>(
    ctx: &Ctx,
    product: String,
    f: impl FnOnce(&Connection, &Path, &registry::Product) -> Result<T, AgencyError> + Send + 'static,
) -> Result<T, RpcError> {
    let core = ctx.core.clone();
    blocking(move |root, registry| {
        let found = registry.product(&product)?.clone();
        let store: Arc<agentsi_store::Store> = core.store()?.ok_or_else(|| {
            AgencyError::Unavailable(
                "the data root has no agentsi.db. Import the data first.".into(),
            )
        })?;
        store
            .read(|c| Ok(f(c, root, &found)))
            .map_err(AgencyError::from)?
    })
    .await
}

async fn whoami(ctx: Ctx, p: WhoamiParams) -> Result<WhoamiResult, RpcError> {
    if p.home {
        // The data root needs no registry, as in `agentsi whoami --home` of TypeScript.
        let home = crate::blocking::spawn(registry::data_root)
            .await
            .map_err(|e| RpcError::internal(e.to_string()))?
            .map_err(rpc)?;
        return Ok(WhoamiResult {
            product: None,
            products: None,
            home: Some(home),
        });
    }
    let core = ctx.core.clone();
    blocking(move |_, registry| {
        if p.list {
            // With no database, nothing is archived.
            let store = core.store().ok().flatten();
            let archived = |id: &str| {
                store
                    .as_ref()
                    .and_then(|s| s.read(|c| agentsi_store::read::archived(c, id)).ok())
                    .is_some_and(|a| a.iter().any(|r| r.kind == "product"))
            };
            return Ok(WhoamiResult {
                product: None,
                products: Some(
                    registry
                        .products
                        .iter()
                        .map(|x| ListedProduct {
                            id: x.id.clone(),
                            name: x.name.clone(),
                            status: x.status.clone(),
                            paths: x.paths.clone(),
                            archived: archived(&x.id),
                        })
                        .collect(),
                ),
                home: None,
            });
        }
        let product = match (&p.product, &p.cwd) {
            (Some(id), _) => Some(registry.product(id)?),
            (None, Some(cwd)) => registry.detect(Path::new(cwd)),
            (None, None) => None,
        };
        Ok(WhoamiResult {
            product: product.map(|x| registry.identity(x)),
            products: None,
            home: None,
        })
    })
    .await
}

async fn product_status(ctx: Ctx, p: StatusParams) -> Result<StatusResult, RpcError> {
    with_store(&ctx, p.product, |c, _, product| {
        status::status(c, product, clock::now_ms())
    })
    .await
}

async fn data_snapshot(ctx: Ctx, p: SnapshotParams) -> Result<SnapshotResult, RpcError> {
    let scope = p.scope;
    with_store(&ctx, p.product, move |c, _, product| {
        Ok(SnapshotResult {
            markdown: snapshot::snapshot(c, product, scope.as_deref())?,
        })
    })
    .await
}

async fn data_audit(ctx: Ctx, p: AuditParams) -> Result<AuditReport, RpcError> {
    with_store(&ctx, p.product, |c, root, product| {
        audit::audit(c, root, product, clock::now_ms())
    })
    .await
}
