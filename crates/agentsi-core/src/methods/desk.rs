//! `product.*` and `minutes.*`: the products of the store, their front desk and
//! their minutes. The handlers only read. The queries are in `desk_data`.

use super::{Ctx, Router};
use crate::desk_data;
use agentsi_proto::RpcError;
use agentsi_proto::core_api::Empty;
use agentsi_proto::desk_api::{
    DeskResult, MinutesDetail, MinutesGet, MinutesGetParams, MinutesList, MinutesListParams,
    MinutesListResult, ProductDesk, ProductDeskParams, ProductList, ProductListResult,
};
use agentsi_proto::rpc::ErrorKind;
use agentsi_store::Store;
use std::sync::Arc;

pub fn register(r: Router) -> Router {
    r.add(ProductList, list)
        .add(ProductDesk, desk)
        .add(MinutesList, minutes_list)
        .add(MinutesGet, minutes_get)
}

fn unavailable(e: agentsi_store::Error) -> RpcError {
    RpcError::new(
        ErrorKind::Unavailable,
        format!("the store cannot open: {e}"),
        "Set AGENTSI_HOME to the data root, or import the data first.",
    )
}

fn no_database() -> RpcError {
    RpcError::new(
        ErrorKind::Unavailable,
        "the data root has no agentsi.db",
        "Import the data first: agentsi-core import-postgres <url> --fresh.",
    )
}

/// Runs `f` on a reader connection, off the event loop. `Ok(None)` while the
/// database file does not exist.
async fn read<T: Send + 'static>(
    ctx: &Ctx,
    f: impl FnOnce(&Store, &std::path::Path) -> agentsi_store::Result<T> + Send + 'static,
) -> Result<Option<T>, RpcError> {
    let core = ctx.core.clone();
    crate::blocking::spawn(move || {
        let Some(store): Option<Arc<Store>> = core.store()? else {
            return Ok(None);
        };
        let root = agentsi_store::paths::data_root(None)?;
        f(&store, &root).map(Some)
    })
    .await
    .map_err(|e| RpcError::internal(e.to_string()))?
    .map_err(unavailable)
}

async fn list(ctx: Ctx, _: Empty) -> Result<ProductListResult, RpcError> {
    let rows = read(&ctx, |store, _| {
        store.read(|c| {
            Ok((
                agentsi_store::read::products(c)?,
                agentsi_store::read::archived_products(c)?,
            ))
        })
    })
    .await?;
    let (products, archived) = rows.unwrap_or_default();
    Ok(ProductListResult {
        products: products
            .into_iter()
            .map(|p| {
                let mut info = desk_data::product_info(p);
                info.archived = archived.contains(&info.id);
                info
            })
            .collect(),
    })
}

async fn desk(ctx: Ctx, p: ProductDeskParams) -> Result<DeskResult, RpcError> {
    let product = p.product.clone();
    let found = read(&ctx, move |store, root| {
        let registry = std::fs::read_to_string(root.join("agentsi.config.json")).ok();
        store.read(|c| desk_data::desk(c, &product, registry.as_deref()))
    })
    .await?
    .ok_or_else(no_database)?;
    found.ok_or_else(|| {
        RpcError::new(
            ErrorKind::NotFound,
            format!("no product {}", p.product),
            "Use product.list for the ids of the products.",
        )
    })
}

async fn minutes_list(ctx: Ctx, p: MinutesListParams) -> Result<MinutesListResult, RpcError> {
    let rows = read(&ctx, move |store, _| {
        store.read(|c| desk_data::minutes_rows(c, &p.product))
    })
    .await?;
    Ok(MinutesListResult {
        minutes: rows.unwrap_or_default(),
    })
}

async fn minutes_get(ctx: Ctx, p: MinutesGetParams) -> Result<MinutesDetail, RpcError> {
    let id = p.id.clone();
    let found = read(&ctx, move |store, _| {
        store.read(|c| desk_data::minutes(c, &id))
    })
    .await?
    .ok_or_else(no_database)?;
    found.ok_or_else(|| {
        RpcError::new(
            ErrorKind::NotFound,
            format!("no minutes {} with a stored body", p.id),
            "Use minutes.list for the ids of the minutes of a product.",
        )
    })
}
