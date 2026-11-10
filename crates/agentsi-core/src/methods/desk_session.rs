//! `desk.*`: the desk conversation of a product. The process logic is in `crate::desk`.

use super::{Ctx, Router};
use crate::desk::{self, open_store, outbound, registry, store_item, unknown_product};
use agentsi_proto::desk_session_api::{
    AuthState, ChartKind, DeskAuth, DeskAuthParams, DeskAuthResult, DeskCheck, DeskCheckParams,
    DeskEntry, DeskHistory, DeskHistoryParams, DeskHistoryResult, DeskItem, DeskPage,
    DeskPageParams, DeskPageResult, DeskProductParams, DeskReset, DeskResetResult, DeskSend,
    DeskSendParams, DeskSendResult, DeskSeqResult, DeskShow, DeskShowParams, DeskShowResult,
    DeskState, DeskStop, DeskStopResult, View,
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
        .add(DeskCheck, check)
        .add(DeskPage, page)
}

/// The limits of the generic views.
const MAX_TEXT: usize = 200;
const MAX_COLUMNS: usize = 12;
const MAX_ROWS: usize = 200;
const MAX_POINTS: usize = 50;
const MAX_ITEMS: usize = 30;
const MAX_PAGE_TITLE: usize = 120;
const MAX_HTML: usize = 512_000;

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
                    item: outbound(item),
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
        let item = DeskItem::View {
            view: p.view,
            checked: Vec::new(),
            html_len: None,
        };
        let view_id = store_item(&ctx.core, &product, item)?;
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
        generic => check_limits(generic).map_err(RpcError::invalid_params),
    }
}

fn long(what: &str, text: &str, max: usize) -> Result<(), String> {
    let n = text.chars().count();
    if n > max {
        return Err(format!("{what} has {n} characters. The limit is {max}."));
    }
    Ok(())
}

fn count(what: &str, n: usize, min: usize, max: usize) -> Result<(), String> {
    if n < min || n > max {
        return Err(format!("{what} has {n} entries. Give {min} to {max}."));
    }
    Ok(())
}

/// The limits of a generic view. The error text names the row or the point, for Claude.
fn check_limits(view: &View) -> Result<(), String> {
    match view {
        View::Table {
            title,
            columns,
            rows,
        } => {
            long("title", title, MAX_TEXT)?;
            count("columns", columns.len(), 1, MAX_COLUMNS)?;
            count("rows", rows.len(), 0, MAX_ROWS)?;
            for (n, c) in columns.iter().enumerate() {
                long(&format!("Column {}", n + 1), c, MAX_TEXT)?;
            }
            for (n, row) in rows.iter().enumerate() {
                if row.len() != columns.len() {
                    return Err(format!(
                        "Row {} has {} cells, but the table has {} columns. Give each row one \
                         cell for each column.",
                        n + 1,
                        row.len(),
                        columns.len()
                    ));
                }
                for (k, cell) in row.iter().enumerate() {
                    long(&format!("Row {} cell {}", n + 1, k + 1), cell, MAX_TEXT)?;
                }
            }
            Ok(())
        }
        View::Chart {
            chart,
            title,
            labels,
            values,
        } => {
            long("title", title, MAX_TEXT)?;
            count("values", values.len(), 1, MAX_POINTS)?;
            if labels.len() != values.len() {
                return Err(format!(
                    "labels has {} entries and values has {}. Give one label for each value.",
                    labels.len(),
                    values.len()
                ));
            }
            for (n, (label, value)) in labels.iter().zip(values).enumerate() {
                long(&format!("Label {}", n + 1), label, MAX_TEXT)?;
                if !value.is_finite() {
                    return Err(format!("Value {} is not a finite number.", n + 1));
                }
                if *chart == ChartKind::Pie && *value < 0.0 {
                    return Err(format!(
                        "Value {} is negative. A pie takes no negative value.",
                        n + 1
                    ));
                }
            }
            Ok(())
        }
        View::Checklist { title, items } => {
            long("title", title, MAX_TEXT)?;
            count("items", items.len(), 1, MAX_ITEMS)?;
            for (n, item) in items.iter().enumerate() {
                long(&format!("Item {}", n + 1), item, MAX_TEXT)?;
            }
            Ok(())
        }
        View::Page { title, html } => {
            long("title", title, MAX_PAGE_TITLE)?;
            if html.trim().is_empty() {
                return Err("html is empty.".into());
            }
            long("html", html, MAX_HTML)
        }
        View::Decisions { .. }
        | View::Priorities { .. }
        | View::Story { .. }
        | View::Diff { .. } => Ok(()),
    }
}

fn no_item(product: &str, seq: i64) -> RpcError {
    RpcError::new(
        ErrorKind::NotFound,
        format!("The desk conversation of {product} has no item {seq}."),
        "Read desk.history again.",
    )
}

/// The stored item `seq` of a product.
fn stored_item(
    store: &agentsi_store::Store,
    product: &str,
    seq: i64,
) -> Result<DeskItem, RpcError> {
    let p = product.to_string();
    let body = store
        .read(move |c| rows::body(c, &p, seq))
        .map_err(store_error)?
        .ok_or_else(|| no_item(product, seq))?;
    serde_json::from_str(&body).map_err(|e| RpcError::internal(e.to_string()))
}

/// `desk.check`: stores the checked items of a checklist and a desk note. No product data.
async fn check(ctx: Ctx, p: DeskCheckParams) -> Result<DeskSeqResult, RpcError> {
    crate::blocking::spawn(move || {
        known(&p.product)?;
        let store = open_store(&ctx.core)?;
        let (product, seq) = (p.product.clone(), p.seq);
        let DeskItem::View {
            view: View::Checklist { title, items },
            ..
        } = stored_item(&store, &product, seq)?
        else {
            return Err(RpcError::invalid_params(format!(
                "Item {seq} is not a checklist."
            )));
        };
        let mut checked = p.checked;
        checked.sort_unstable();
        checked.dedup();
        if let Some(bad) = checked.iter().find(|i| **i as usize >= items.len()) {
            return Err(RpcError::invalid_params(format!(
                "The checklist has {} items. Index {bad} is out of range.",
                items.len()
            )));
        }
        let names: Vec<&str> = checked
            .iter()
            .map(|i| items[*i as usize].as_str())
            .collect();
        let mut text = format!(
            "The owner checked {} of {} items of {title}",
            checked.len(),
            items.len()
        );
        if names.is_empty() {
            text.push('.');
        } else {
            text.push_str(&format!(": {}.", names.join(", ")));
        }
        let body = serde_json::to_string(&DeskItem::View {
            view: View::Checklist {
                title: title.clone(),
                items: items.clone(),
            },
            checked,
            html_len: None,
        })
        .map_err(|e| RpcError::internal(e.to_string()))?;
        let p2 = product.clone();
        let found = store
            .write(move |tx| rows::set_body(tx, &p2, seq, &body))
            .map_err(store_error)?;
        if !found {
            return Err(no_item(&product, seq));
        }
        desk::note(&ctx.core, &product, &text);
        Ok(DeskSeqResult { seq })
    })
    .await
    .map_err(join)?
}

/// `desk.page`: the HTML of a page view.
async fn page(ctx: Ctx, p: DeskPageParams) -> Result<DeskPageResult, RpcError> {
    crate::blocking::spawn(move || {
        known(&p.product)?;
        let store = open_store(&ctx.core)?;
        match stored_item(&store, &p.product, p.seq)? {
            DeskItem::View {
                view: View::Page { title, html },
                ..
            } => Ok(DeskPageResult { title, html }),
            _ => Err(RpcError::invalid_params(format!(
                "Item {} is not a page.",
                p.seq
            ))),
        }
    })
    .await
    .map_err(join)?
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
