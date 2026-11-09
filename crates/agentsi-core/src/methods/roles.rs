//! `roles.list`, `roles.validate`, `roles.apply` and `roles.reset` (U21): the
//! methods of `agentsi roles` and `agentsi reset`. The logic is in
//! `agentsi-agency::roles`. `list` and `validate` never write. `apply` and
//! `reset` write only when `yes` is true: the database in one transaction,
//! then one commit of `agentsi.config.json` in the data root.

use super::{Ctx, Router};
use agentsi_agency::registry::{Product, Registry};
use agentsi_agency::roles::{
    self, Error as RolesError, apply, commit, files::RoleFiles, load, reset,
};
use agentsi_agency::{Error as AgencyError, roster};
use agentsi_proto::RpcError;
use agentsi_proto::desk_api::Objective;
use agentsi_proto::events::{DataChanged, DataChangedParams};
use agentsi_proto::roles_api::{
    BriefView, CommitInfo, Registration, RolesApply, RolesApplyParams, RolesApplyResult,
    RolesBrief, RolesBriefParams, RolesBriefResult, RolesList, RolesListParams, RolesListResult,
    RolesReset, RolesResetParams, RolesResetResult, RolesValidate, RolesValidateParams,
    RolesValidateResult,
};
use agentsi_proto::rpc::ErrorKind;
use agentsi_store::Store;
use rusqlite::Connection;
use std::path::{Path, PathBuf};
use std::sync::Arc;

pub fn register(r: Router) -> Router {
    r.add(RolesList, roles_list)
        .add(RolesBrief, roles_brief)
        .add(RolesValidate, roles_validate)
        .add(RolesApply, roles_apply)
        .add(RolesReset, roles_reset)
}

fn rpc(e: RolesError) -> RpcError {
    match e {
        RolesError::Invalid(m) => RpcError::new(
            ErrorKind::Invalid,
            m,
            "Check the arguments and the stage folder, then run it again.",
        ),
        RolesError::Agency(AgencyError::NotFound(m)) => RpcError::new(
            ErrorKind::NotFound,
            m,
            "Use product.whoami with list: true for the products of the registry.",
        ),
        RolesError::Agency(AgencyError::Unavailable(m)) => RpcError::new(
            ErrorKind::Unavailable,
            m,
            "Set AGENTSI_HOME to the data root. It holds agentsi.config.json and agentsi.db.",
        ),
    }
}

fn internal(e: impl std::fmt::Display) -> RpcError {
    RpcError::internal(e.to_string())
}

/// What a handler works with.
struct Work {
    root: PathBuf,
    registry: Registry,
    store: Arc<Store>,
}

/// Runs `f` off the event loop, with the data root, the registry and the store.
async fn blocking<T: Send + 'static>(
    ctx: &Ctx,
    f: impl FnOnce(Work) -> roles::Result<T> + Send + 'static,
) -> Result<T, RpcError> {
    let core = ctx.core.clone();
    crate::blocking::spawn(move || {
        let root =
            agentsi_store::paths::data_root(None).map_err(|e| rpc(AgencyError::from(e).into()))?;
        let registry = Registry::load(&root).map_err(|e| rpc(e.into()))?;
        let store = core
            .store()
            .map_err(|e| rpc(AgencyError::from(e).into()))?
            .ok_or_else(|| {
                rpc(AgencyError::Unavailable(
                    "the data root has no agentsi.db. Import the data first.".into(),
                )
                .into())
            })?;
        f(Work {
            root,
            registry,
            store,
        })
        .map_err(rpc)
    })
    .await
    .map_err(internal)?
}

/// Runs `f` on a reader connection.
fn read<T: Send + 'static>(
    store: &Store,
    f: impl FnOnce(&Connection) -> roles::Result<T>,
) -> roles::Result<T> {
    store
        .read(|c| Ok(f(c)))
        .map_err(AgencyError::from)
        .map_err(RolesError::from)?
}

/// Runs `f` in one transaction. An error of `f` rolls the transaction back.
fn write<T: Send + 'static>(
    store: &Store,
    f: impl FnOnce(&rusqlite::Transaction) -> roles::Result<T> + Send + 'static,
) -> roles::Result<T> {
    store
        .write(move |tx| {
            f(tx).map_err(|e| match e {
                RolesError::Invalid(m) => agentsi_store::Error::Other(m),
                RolesError::Agency(a) => agentsi_store::Error::Other(a.to_string()),
            })
        })
        .map_err(|e| RolesError::Invalid(e.to_string()))
}

async fn roles_list(ctx: Ctx, p: RolesListParams) -> Result<RolesListResult, RpcError> {
    blocking(&ctx, move |w| {
        let product = w.registry.product(&p.product)?.clone();
        read(&w.store, |c| {
            let files = RoleFiles::at(&w.root);
            let overlays = roster::read_overlays(c, &product.id)?;
            let mut warnings = Vec::new();
            let mut out = RolesListResult {
                product: product.id.clone(),
                roster: None,
                agent: None,
                overlays: overlays.keys().cloned().collect(),
                globals: files.names().unwrap_or_default(),
                warnings: Vec::new(),
            };
            match &p.agent {
                Some(id) => {
                    out.agent = Some(
                        load::load_role(id, &product.id, &files, &overlays, &mut warnings)
                            .map_err(RolesError::Invalid)?,
                    );
                }
                None => {
                    out.roster = Some(load::describe_roster(
                        &product.roles,
                        &product.id,
                        &files,
                        &overlays,
                        &mut warnings,
                    ));
                }
            }
            out.warnings = warnings;
            Ok(out)
        })
    })
    .await
}

/// The view of a stored brief. The stored `product` object holds `what`, `audience` and `stage`.
/// A lacking field, or one of the wrong type, reads as empty, as `product.desk` reads it.
fn brief_view(stored: serde_json::Value) -> BriefView {
    use serde_json::Value;
    let text = |v: &Value, key: &str| {
        v.get(key)
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_string()
    };
    let list = |v: &Value, key: &str| -> Vec<Value> {
        v.get(key)
            .and_then(Value::as_array)
            .cloned()
            .unwrap_or_default()
    };
    let texts = |key: &str| {
        list(&stored, key)
            .iter()
            .filter_map(|i| i.as_str().map(String::from))
            .collect()
    };
    let product = stored.get("product").cloned().unwrap_or_default();
    BriefView {
        what: text(&product, "what"),
        audience: text(&product, "audience"),
        stage: text(&product, "stage"),
        objectives: list(&stored, "objectives")
            .iter()
            .map(|o| Objective {
                id: text(o, "id"),
                text: text(o, "text"),
                signal: text(o, "signal"),
                due: text(o, "due"),
            })
            .collect(),
        north_stars: texts("north_stars"),
        out_of_scope: texts("out_of_scope"),
        roles: stored
            .get("roles")
            .and_then(Value::as_object)
            .into_iter()
            .flatten()
            .filter_map(|(k, v)| Some((k.clone(), v.as_str()?.to_string())))
            .collect(),
        updated_at: stored
            .get("updated_at")
            .and_then(Value::as_str)
            .map(String::from),
    }
}

async fn roles_brief(ctx: Ctx, p: RolesBriefParams) -> Result<RolesBriefResult, RpcError> {
    blocking(&ctx, move |w| {
        let product = w.registry.product(&p.product)?.clone();
        let stored = w
            .store
            .read(|c| agentsi_store::read::brief(c, &product.id))
            .map_err(AgencyError::from)?;
        let brief = stored.map(brief_view);
        Ok(RolesBriefResult {
            product: product.id,
            brief,
        })
    })
    .await
}

async fn roles_validate(ctx: Ctx, p: RolesValidateParams) -> Result<RolesValidateResult, RpcError> {
    blocking(&ctx, move |w| {
        let product = w.registry.product(&p.product)?.clone();
        read(&w.store, |c| {
            let failures = roster::validate_roster(c, &w.root, &product)?;
            let overlays = roster::read_overlays(c, &product.id)?;
            let mut warnings = Vec::new();
            load::describe_roster(
                &product.roles,
                &product.id,
                &RoleFiles::at(&w.root),
                &overlays,
                &mut warnings,
            );
            Ok(RolesValidateResult {
                warnings,
                product: product.id.clone(),
                valid: failures.is_empty(),
                failures,
                agents: product.roles.len() as u64,
            })
        })
    })
    .await
}

/// `registerableProduct`: the product that `apply --register` adds.
fn registerable(id: &str, r: &Registration) -> roles::Result<Product> {
    let valid_id = id.chars().next().is_some_and(|c| c.is_ascii_lowercase())
        && id
            .chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-');
    if !valid_id {
        return Err(RolesError::Invalid(format!("Invalid product id \"{id}\".")));
    }
    if r.name.is_empty() || r.path.is_empty() {
        return Err(RolesError::Invalid(
            "--register needs --name and --path.".into(),
        ));
    }
    let path = Path::new(&r.path);
    if !path.is_absolute() || !path.exists() {
        return Err(RolesError::Invalid(format!(
            "--path must be an existing absolute path, got \"{}\".",
            r.path
        )));
    }
    Ok(Product {
        id: id.into(),
        name: r.name.clone(),
        paths: vec![r.path.clone()],
        status: "active".into(),
        roles: Vec::new(),
    })
}

/// The stage folder of a request: `stage_dir`, or `stage` written to a temporary folder
/// that lives as long as the returned guard.
fn stage_of(p: &RolesApplyParams) -> roles::Result<(PathBuf, Option<apply::InlineStage>)> {
    match (&p.stage_dir, &p.stage) {
        (Some(dir), None) => Ok((PathBuf::from(dir), None)),
        (None, Some(files)) => {
            let inline = apply::InlineStage::write(files).map_err(RolesError::Invalid)?;
            Ok((inline.path().to_path_buf(), Some(inline)))
        }
        _ => Err(RolesError::Invalid(
            "Send one of stage_dir and stage.".into(),
        )),
    }
}

async fn roles_apply(ctx: Ctx, p: RolesApplyParams) -> Result<RolesApplyResult, RpcError> {
    let product = p.product.clone();
    let done = apply_stage(&ctx, p).await?;
    if done.applied {
        ctx.core.publish::<DataChanged>(&DataChangedParams {
            product,
            kind: "product".into(),
        });
    }
    Ok(done)
}

async fn apply_stage(ctx: &Ctx, p: RolesApplyParams) -> Result<RolesApplyResult, RpcError> {
    blocking(ctx, move |w| {
        let registering = p.register.is_some();
        let product = match &p.register {
            Some(r) => registerable(&p.product, r)?,
            None => w.registry.product(&p.product)?.clone(),
        };
        let (stage, _inline) = stage_of(&p)?;
        let (planned, warnings) = read(&w.store, |c| {
            apply::plan_apply(c, &w.root, &stage, &product, registering)
        })?;
        let planned = match planned {
            Ok(planned) => planned,
            Err(failures) => {
                return Ok(RolesApplyResult {
                    plan: None,
                    failures,
                    applied: false,
                    commit: None,
                    warnings,
                });
            }
        };
        let plan = planned.plan.clone();
        if !p.yes {
            return Ok(RolesApplyResult {
                plan: Some(plan),
                failures: Vec::new(),
                applied: false,
                commit: None,
                warnings,
            });
        }
        let root = w.root.clone();
        write(&w.store, move |tx| apply::write_plan(tx, &root, &planned))?;
        let commit = commit::commit_files(&w.root, &[roles_file()], &apply::commit_message(&plan));
        Ok(RolesApplyResult {
            plan: Some(plan),
            failures: Vec::new(),
            applied: true,
            commit: Some(commit),
            warnings,
        })
    })
    .await
}

fn roles_file() -> &'static str {
    agentsi_agency::registry::FILE
}

async fn roles_reset(ctx: Ctx, p: RolesResetParams) -> Result<RolesResetResult, RpcError> {
    blocking(&ctx, move |w| {
        let product = w.registry.product(&p.product)?.clone();
        let planned = read(&w.store, |c| {
            reset::plan_reset(c, &w.root, &product, p.agent.as_deref(), p.memory)
        })?;
        let plan = planned.plan.clone();
        if !p.yes || plan.noop {
            return Ok(RolesResetResult {
                plan,
                applied: false,
                commit: None,
            });
        }
        let root = w.root.clone();
        write(&w.store, move |tx| reset::write_reset(tx, &root, &planned))?;
        let commit: CommitInfo =
            commit::commit_files(&w.root, &[roles_file()], &reset::commit_message(&plan));
        Ok(RolesResetResult {
            plan,
            applied: true,
            commit: Some(commit),
        })
    })
    .await
}

#[cfg(test)]
mod tests {
    use super::{RolesError, write};
    use agentsi_store::Store;

    #[test]
    fn a_failed_step_rolls_the_whole_write_back() {
        let dir = tempfile::tempdir().unwrap();
        let store = Store::open(&dir.path().join("agentsi.db")).unwrap();
        let failed = write(&store, |tx| {
            tx.execute("insert into agent_overlays values ('p', 'a', 'x')", [])?;
            Err::<(), _>(RolesError::Invalid("registry edit failed".into()))
        });
        assert!(failed.is_err());
        let rows: i64 = store
            .read(|c| Ok(c.query_row("select count(*) from agent_overlays", [], |r| r.get(0))?))
            .unwrap();
        assert_eq!(rows, 0);
    }
}
