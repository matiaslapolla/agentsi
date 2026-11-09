//! `registry.get` and `registry.update`: the owner block and the products of
//! `agentsi.config.json`. An update edits the file on the writer thread of the store, as
//! `roles.apply` and `roles.reset` do, so their edits of the file never cross. It makes
//! no commit (ADR 0003).

use super::{Ctx, Router};
use agentsi_agency::Error as AgencyError;
use agentsi_agency::registry::{self, Registry};
use agentsi_agency::roles::registry_edit::{self, Patch};
use agentsi_proto::RpcError;
use agentsi_proto::core_api::Empty;
use agentsi_proto::events::{DataChanged, DataChangedParams};
use agentsi_proto::registry_api::{
    OwnerChange, ProductChange, RegistryGet, RegistryProduct, RegistryResult, RegistryUpdate,
    RegistryUpdateParams,
};
use agentsi_proto::rpc::ErrorKind;
use rusqlite::Transaction;
use std::path::Path;

pub fn register(r: Router) -> Router {
    r.add(RegistryGet, get).add(RegistryUpdate, update)
}

fn agency(e: AgencyError) -> RpcError {
    match e {
        AgencyError::NotFound(m) => RpcError::new(
            ErrorKind::NotFound,
            m,
            "Use registry.get for the products of the registry.",
        ),
        AgencyError::Unavailable(m) => unavailable(m),
    }
}

fn unavailable(m: String) -> RpcError {
    RpcError::new(
        ErrorKind::Unavailable,
        m,
        "Set AGENTSI_HOME to the data root. It holds agentsi.config.json and agentsi.db.",
    )
}

fn invalid(m: impl Into<String>) -> RpcError {
    RpcError::new(
        ErrorKind::Invalid,
        m.into(),
        "Fix the fields and try again. Each other key of the registry stays.",
    )
}

fn join(e: tokio::task::JoinError) -> RpcError {
    RpcError::internal(e.to_string())
}

fn root() -> Result<std::path::PathBuf, RpcError> {
    agentsi_store::paths::data_root(None).map_err(|e| agency(e.into()))
}

fn result(r: Registry) -> RegistryResult {
    RegistryResult {
        owner: r.owner,
        products: r
            .products
            .into_iter()
            .map(|p| RegistryProduct {
                id: p.id,
                name: p.name,
                paths: p.paths,
                status: p.status,
            })
            .collect(),
    }
}

async fn get(_: Ctx, _: Empty) -> Result<RegistryResult, RpcError> {
    crate::blocking::spawn(|| Ok(result(Registry::load(&root()?).map_err(agency)?)))
        .await
        .map_err(join)?
}

/// A field that must hold text: trimmed, not empty.
fn text(field: &str, value: &Option<String>) -> Result<Option<String>, RpcError> {
    match value.as_deref().map(str::trim) {
        None => Ok(None),
        Some("") => Err(invalid(format!("{field} must not be empty."))),
        Some(t) => Ok(Some(t.to_string())),
    }
}

/// Checks the change before any read of the file.
fn checked(
    p: RegistryUpdateParams,
) -> Result<(Option<OwnerChange>, Option<ProductChange>), RpcError> {
    if p.owner.is_none() && p.product.is_none() {
        return Err(invalid("Nothing to change. Send owner, product or both."));
    }
    let owner = match p.owner {
        None => None,
        Some(o) => Some(OwnerChange {
            name: text("owner.name", &o.name)?,
            language: text("owner.language", &o.language)?,
        }),
    };
    let product = match p.product {
        None => None,
        Some(c) => {
            if let Some(paths) = &c.paths {
                if paths.is_empty() {
                    return Err(invalid("A product keeps one path at least."));
                }
                for path in paths {
                    let at = Path::new(path);
                    if !at.is_absolute() || !at.is_dir() {
                        return Err(invalid(format!(
                            "\"{path}\" is not an existing absolute folder."
                        )));
                    }
                }
            }
            Some(ProductChange {
                name: text("product.name", &c.name)?,
                ..c
            })
        }
    };
    Ok((owner, product))
}

/// Edits the file, writes the `products` row in `tx` when there is one, and replaces the
/// file as the last step. A failed step leaves the file as it was.
fn edit(
    root: &Path,
    owner: &Option<OwnerChange>,
    product: &Option<ProductChange>,
    tx: Option<&Transaction>,
) -> Result<Registry, RpcError> {
    let file = root.join(registry::FILE);
    let mut text = std::fs::read_to_string(&file)
        .map_err(|e| unavailable(format!("cannot read {}: {e}", file.display())))?;
    if let Some(o) = owner {
        text = registry_edit::update_owner(&text, o.name.as_deref(), o.language.as_deref())
            .map_err(invalid)?;
    }
    if let Some(c) = product {
        Registry::parse(&text)
            .map_err(agency)?
            .product(&c.id)
            .map_err(agency)?;
        let patch = Patch {
            name: c.name.clone(),
            paths: c.paths.clone(),
            ..Patch::default()
        };
        text = registry_edit::update_product(&text, &c.id, &patch).map_err(invalid)?;
    }
    let next = Registry::parse(&text).map_err(|e| invalid(e.to_string()))?;
    if let (Some(tx), Some(c)) = (tx, product) {
        next.product(&c.id)
            .map_err(agency)?
            .store_row(tx)
            .map_err(|e| RpcError::internal(e.to_string()))?;
    }
    registry::write_file(root, &text)
        .map_err(|e| RpcError::internal(format!("cannot write {}: {e}", file.display())))?;
    Ok(next)
}

async fn update(ctx: Ctx, p: RegistryUpdateParams) -> Result<RegistryResult, RpcError> {
    let (owner, product) = checked(p)?;
    let changed = product.as_ref().map(|c| c.id.clone());
    let core = ctx.core.clone();
    let next = crate::blocking::spawn(move || {
        let root = root()?;
        match core.store().map_err(|e| agency(e.into()))? {
            Some(store) => store
                .try_write(move |tx| edit(&root, &owner, &product, Some(tx)))
                .map_err(|e| RpcError::internal(e.to_string()))?,
            None => edit(&root, &owner, &product, None),
        }
    })
    .await
    .map_err(join)??;
    if let Some(product) = changed {
        ctx.core.publish::<DataChanged>(&DataChangedParams {
            product,
            kind: "product".into(),
        });
    }
    Ok(result(next))
}
