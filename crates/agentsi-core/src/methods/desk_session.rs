//! `desk.*`: the desk conversation of a product. The process logic is in `crate::desk`.

use super::{Ctx, Router};
use crate::desk::{self, open_store, registry, store_item, unknown_product};
use agentsi_proto::desk_session_api::{
    AuthState, DeskAuth, DeskAuthParams, DeskAuthResult, DeskEntry, DeskHistory, DeskHistoryParams,
    DeskHistoryResult, DeskItem, DeskProductParams, DeskReset, DeskResetResult, DeskSend,
    DeskSendParams, DeskSendResult, DeskShow, DeskShowParams, DeskShowResult, DeskState, DeskStop,
    DeskStopResult, View,
};
use agentsi_proto::rpc::{ErrorKind, RpcError};
use agentsi_store::desk as rows;

/// The items that `desk.history` gives with no `limit`.
const HISTORY_LIMIT: u32 = 200;

pub fn register(r: Router) -> Router {
    r.add(DeskHistory, history)
        .add(DeskSend, send)
        .add(DeskStop, stop)
        .add(DeskReset, reset)
        .add(DeskShow, show)
        .add(DeskAuth, auth)
}

fn join(e: tokio::task::JoinError) -> RpcError {
    RpcError::internal(e.to_string())
}

fn store_error(e: agentsi_store::Error) -> RpcError {
    RpcError::internal(e.to_string())
}

fn known(product: &str) -> Result<(), RpcError> {
    registry()?
        .product(product)
        .map(|_| ())
        .map_err(|_| unknown_product(product))
}

async fn history(ctx: Ctx, p: DeskHistoryParams) -> Result<DeskHistoryResult, RpcError> {
    crate::blocking::spawn(move || {
        known(&p.product)?;
        let (mut state, mut message) = ctx.core.desks.status(&p.product);
        // The first desk request checks the login. A check at the start of the core would
        // run `claude` for each core, also for one that never opens the front desk.
        let auth = ctx
            .core
            .desks
            .auth()
            .unwrap_or_else(|| desk::check_auth(&ctx.core));
        if state == DeskState::Stopped && auth.state != AuthState::Ready {
            (state, message) = (DeskState::Auth, auth.message);
        }
        let Some(store) = ctx.core.store().map_err(store_error)? else {
            return Ok(DeskHistoryResult {
                session: None,
                state,
                message,
                items: Vec::new(),
            });
        };
        let product = p.product.clone();
        let limit = p.limit.unwrap_or(HISTORY_LIMIT);
        let (session, found) = store
            .read(move |c| {
                Ok((
                    rows::session(c, &product)?,
                    rows::items(c, &product, p.before, limit)?,
                ))
            })
            .map_err(store_error)?;
        let items = found
            .into_iter()
            .filter_map(|r| {
                let item = serde_json::from_str::<DeskItem>(&r.body).ok()?;
                Some(DeskEntry {
                    seq: r.seq,
                    at: r.at,
                    item,
                })
            })
            .collect();
        Ok(DeskHistoryResult {
            session,
            state,
            message,
            items,
        })
    })
    .await
    .map_err(join)?
}

async fn send(ctx: Ctx, p: DeskSendParams) -> Result<DeskSendResult, RpcError> {
    crate::blocking::spawn(move || desk::send(&ctx.core, &p.product, &p.text))
        .await
        .map_err(join)?
        .map(|seq| DeskSendResult { seq })
}

async fn stop(ctx: Ctx, p: DeskProductParams) -> Result<DeskStopResult, RpcError> {
    crate::blocking::spawn(move || {
        known(&p.product)?;
        Ok(DeskStopResult {
            stopped: desk::stop(&ctx.core, &p.product),
        })
    })
    .await
    .map_err(join)?
}

async fn reset(ctx: Ctx, p: DeskProductParams) -> Result<DeskResetResult, RpcError> {
    crate::blocking::spawn(move || {
        desk::reset(&ctx.core, &p.product).map(|session| DeskResetResult { session })
    })
    .await
    .map_err(join)?
}

async fn auth(ctx: Ctx, p: DeskAuthParams) -> Result<DeskAuthResult, RpcError> {
    crate::blocking::spawn(move || {
        let result = match ctx.core.desks.auth() {
            Some(a) if !p.recheck => a,
            _ => desk::check_auth(&ctx.core),
        };
        DeskAuthResult {
            state: result.state,
            message: result.message,
        }
    })
    .await
    .map_err(join)
}

async fn show(ctx: Ctx, p: DeskShowParams) -> Result<DeskShowResult, RpcError> {
    crate::blocking::spawn(move || {
        let store = open_store(&ctx.core)?;
        let session = p.session.clone();
        let product = store
            .read(move |c| rows::product_of(c, &session))
            .map_err(store_error)?
            .ok_or_else(|| {
                RpcError::new(
                    ErrorKind::NotFound,
                    format!("No product has the desk session \"{}\".", p.session),
                    "The owner started a new conversation. Answer in text.",
                )
            })?;
        check_view(&store, &product, &p.view)?;
        let view_id = store_item(&ctx.core, &product, DeskItem::View { view: p.view })?;
        Ok(DeskShowResult { view_id })
    })
    .await
    .map_err(join)?
}

/// The objects that a view names must exist in its product.
fn check_view(store: &agentsi_store::Store, product: &str, view: &View) -> Result<(), RpcError> {
    match view {
        View::Decisions { ids } => check_decisions(store, product, ids),
        View::Priorities { minutes_id } => {
            let id = minutes_id.clone();
            let found = store
                .read(move |c| crate::desk_data::minutes(c, &id))
                .map_err(store_error)?;
            match found {
                Some(m) if m.product == product => Ok(()),
                _ => Err(RpcError::new(
                    ErrorKind::NotFound,
                    format!("The product {product} has no minutes with the id {minutes_id}."),
                    "Use list_minutes for the ids that exist.",
                )),
            }
        }
        View::Story { id } | View::Diff { story_id: id } => {
            let (p, wanted) = (product.to_string(), id.clone());
            let found = store
                .read(move |c| agentsi_store::read::story(c, &p, &wanted))
                .map_err(store_error)?;
            match found {
                Some(_) => Ok(()),
                None => Err(RpcError::new(
                    ErrorKind::NotFound,
                    format!("The product {product} has no story with the id {id}."),
                    "Use search or the docs of the product for the story ids.",
                )),
            }
        }
    }
}

fn check_decisions(
    store: &agentsi_store::Store,
    product: &str,
    ids: &[i64],
) -> Result<(), RpcError> {
    if ids.is_empty() {
        return Err(RpcError::invalid_params(
            "ids is empty. Give the ids of one or more decisions.",
        ));
    }
    let (p, wanted) = (product.to_string(), ids.to_vec());
    let found = store
        .read(move |c| {
            agentsi_store::read::decisions_by_id(c, &p, &wanted)
                .map(|d| d.into_iter().map(|d| d.id).collect::<Vec<_>>())
        })
        .map_err(store_error)?;
    let missing: Vec<String> = ids
        .iter()
        .filter(|id| !found.contains(id))
        .map(i64::to_string)
        .collect();
    if !missing.is_empty() {
        return Err(RpcError::new(
            ErrorKind::NotFound,
            format!(
                "The product {product} has no decision with the id {}.",
                missing.join(", ")
            ),
            "Use pending_decisions for the ids that exist.",
        ));
    }
    Ok(())
}
