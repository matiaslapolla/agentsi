//! `ticket.*`: the tracker. The rules and the SQL are in `agentsi_agency::tickets`.
//! Each write runs in one transaction on the writer thread of the store, and sends
//! `data.changed {product, kind: "tickets"}` after the commit.

use super::{Ctx, Router};
use agentsi_agency::registry::{Product, Registry};
use agentsi_agency::story::config::story_config;
use agentsi_agency::story::propose::ProposeCtx;
use agentsi_agency::tickets::{self, Error, text, write};
use agentsi_agency::{Error as AgencyError, clock};
use agentsi_proto::RpcError;
use agentsi_proto::events::{DataChanged, DataChangedParams};
use agentsi_proto::rpc::ErrorKind;
use agentsi_proto::ticket_api::{
    Author, AuthorBy, PromoteTo, TicketAdd, TicketAddParams, TicketGet, TicketGetParams,
    TicketGetResult, TicketList, TicketListParams, TicketListResult, TicketMove, TicketMoveParams,
    TicketPromote, TicketPromoteParams, TicketPromoteResult, TicketResult, TicketUpdate,
    TicketUpdateParams,
};
use agentsi_store::Store;
use rusqlite::Transaction;
use std::path::PathBuf;
use std::sync::Arc;

pub fn register(r: Router) -> Router {
    r.add(TicketList, ticket_list)
        .add(TicketGet, ticket_get)
        .add(TicketAdd, ticket_add)
        .add(TicketUpdate, ticket_update)
        .add(TicketMove, ticket_move)
        .add(TicketPromote, ticket_promote)
}

/// The name that the registry gives the owner when its block has none.
const DEFAULT_OWNER_NAME: &str = "the owner";

fn rpc(e: Error) -> RpcError {
    match e {
        Error::NotFound(m) => RpcError::new(
            ErrorKind::NotFound,
            m,
            "Use ticket.list for the tickets that exist, and agency.products for the products.",
        ),
        Error::Invalid(m) => RpcError::new(ErrorKind::Invalid, m, "Fix the value and try again."),
        Error::Conflict(m) => RpcError::new(
            ErrorKind::Conflict,
            m,
            "Add a new ticket, or ask the owner to change this one.",
        ),
        Error::Unavailable(m) => RpcError::new(
            ErrorKind::Unavailable,
            m,
            "Set AGENTSI_HOME to the data root. It holds agentsi.config.json and agentsi.db.",
        ),
    }
}

fn agency(e: AgencyError) -> RpcError {
    match e {
        AgencyError::NotFound(m) => rpc(Error::NotFound(m)),
        AgencyError::Unavailable(m) => rpc(Error::Unavailable(m)),
    }
}

fn join(e: tokio::task::JoinError) -> RpcError {
    RpcError::internal(e.to_string())
}

fn open_store(core: &crate::state::Core) -> Result<Arc<Store>, RpcError> {
    core.store().map_err(|e| rpc(e.into()))?.ok_or_else(|| {
        rpc(Error::Unavailable(
            "the data root has no agentsi.db. Import the data first.".into(),
        ))
    })
}

/// What a handler works with: the data root, the registry and the product.
struct Place {
    root: PathBuf,
    registry: Registry,
    product: Product,
}

impl Place {
    fn of(product: &str) -> Result<Place, RpcError> {
        let root = agentsi_store::paths::data_root(None).map_err(|e| rpc(e.into()))?;
        let registry = Registry::load(&root).map_err(agency)?;
        let product = registry.product(product).map_err(agency)?.clone();
        Ok(Place {
            root,
            registry,
            product,
        })
    }

    /// The writer, with the name of the owner block as the id of the owner.
    fn author(&self, by: &Author) -> Author {
        match by.by {
            AuthorBy::Owner => Author {
                by: AuthorBy::Owner,
                id: if self.registry.owner.name == DEFAULT_OWNER_NAME {
                    "owner".into()
                } else {
                    self.registry.owner.name.clone()
                },
            },
            _ => by.clone(),
        }
    }
}

/// Runs `f` in one transaction on the writer thread. A failure rolls it back.
fn write_blocking<T: Send + 'static>(
    core: &crate::state::Core,
    f: impl FnOnce(&Transaction) -> tickets::Result<T> + Send + 'static,
) -> Result<T, RpcError> {
    open_store(core)?
        .try_write(f)
        .map_err(|e| rpc(e.into()))?
        .map_err(rpc)
}

fn changed(ctx: &Ctx, product: &str) {
    changed_kind(ctx, product, "tickets");
}

fn changed_kind(ctx: &Ctx, product: &str, kind: &str) {
    ctx.core.publish::<DataChanged>(&DataChangedParams {
        product: product.to_string(),
        kind: kind.to_string(),
    });
}

async fn ticket_list(ctx: Ctx, p: TicketListParams) -> Result<TicketListResult, RpcError> {
    crate::blocking::spawn(move || {
        Place::of(&p.product)?;
        let store = open_store(&ctx.core)?;
        let tickets = store
            .read(|c| Ok(tickets::list(c, &p)))
            .map_err(|e| rpc(e.into()))?
            .map_err(rpc)?;
        Ok(TicketListResult { tickets })
    })
    .await
    .map_err(join)?
}

async fn ticket_get(ctx: Ctx, p: TicketGetParams) -> Result<TicketGetResult, RpcError> {
    crate::blocking::spawn(move || {
        Place::of(&p.product)?;
        let store = open_store(&ctx.core)?;
        let ticket = store
            .read(|c| Ok(tickets::get(c, &p.product, &p.id)))
            .map_err(|e| rpc(e.into()))?
            .map_err(rpc)?;
        let markdown = text::markdown(&ticket);
        Ok(TicketGetResult { ticket, markdown })
    })
    .await
    .map_err(join)?
}

async fn ticket_add(ctx: Ctx, mut p: TicketAddParams) -> Result<TicketResult, RpcError> {
    let core = ctx.core.clone();
    let ticket = crate::blocking::spawn(move || {
        let Place { product, .. } = {
            let place = Place::of(&p.product)?;
            p.author = place.author(&p.author);
            place
        };
        write_blocking(&core, move |tx| {
            write::add(tx, &product, &p, clock::now_ms())
        })
    })
    .await
    .map_err(join)??;
    changed(&ctx, &ticket.product);
    Ok(TicketResult { ticket })
}

async fn ticket_update(ctx: Ctx, mut p: TicketUpdateParams) -> Result<TicketResult, RpcError> {
    let core = ctx.core.clone();
    let ticket = crate::blocking::spawn(move || {
        let Place { product, .. } = {
            let place = Place::of(&p.product)?;
            p.by = place.author(&p.by);
            place
        };
        write_blocking(&core, move |tx| {
            write::update(tx, &product, &p, clock::now_ms())
        })
    })
    .await
    .map_err(join)??;
    changed(&ctx, &ticket.product);
    Ok(TicketResult { ticket })
}

async fn ticket_move(ctx: Ctx, mut p: TicketMoveParams) -> Result<TicketResult, RpcError> {
    let core = ctx.core.clone();
    let moved = crate::blocking::spawn(move || {
        let Place { product, .. } = {
            let place = Place::of(&p.product)?;
            p.by = place.author(&p.by);
            place
        };
        write_blocking(&core, move |tx| {
            write::move_to(tx, &product, &p, clock::now_ms())
        })
    })
    .await
    .map_err(join)??;
    if moved.changed {
        changed(&ctx, &moved.ticket.product);
    }
    Ok(TicketResult {
        ticket: moved.ticket,
    })
}

async fn ticket_promote(ctx: Ctx, p: TicketPromoteParams) -> Result<TicketPromoteResult, RpcError> {
    let core = ctx.core.clone();
    let (result, kind) = crate::blocking::spawn(move || {
        let place = Place::of(&p.product)?;
        let by = place.author(&p.by);
        let id = p.id;
        match p.to {
            PromoteTo::Decision => {
                let product = place.product.id.clone();
                let (ticket, decision) = write_blocking(&core, move |tx| {
                    write::promote_decision(tx, &product, &id, &by, clock::now_ms())
                })?;
                let result = TicketPromoteResult {
                    ticket,
                    story: None,
                    decision: Some(decision),
                };
                Ok::<_, RpcError>((result, "decision"))
            }
            PromoteTo::Story => {
                let config = story_config(&place.root, &place.product)
                    .map_err(|e| rpc(write::story_error(e)))?;
                let ctx = ProposeCtx::new(
                    open_store(&core)?,
                    place.root,
                    place.product,
                    config,
                    place.registry.owner,
                );
                let (ticket, story) = write::promote_story(&ctx, &id, &by, clock::now_ms())
                    .map_err(|e| {
                        if e.story.is_some() {
                            core.publish::<DataChanged>(&DataChangedParams {
                                product: ctx.product.id.clone(),
                                kind: "stories".into(),
                            });
                        }
                        rpc(e.error)
                    })?;
                let result = TicketPromoteResult {
                    ticket,
                    story: Some(story),
                    decision: None,
                };
                Ok((result, "stories"))
            }
        }
    })
    .await
    .map_err(join)??;
    changed_kind(&ctx, &result.ticket.product, kind);
    changed(&ctx, &result.ticket.product);
    Ok(result)
}
