//! `roles.roster` and `role.general.*`: the writes of the roles screen. Spec:
//! `docs/specs/agentsi-desktop-global-roles.md`. The logic is in
//! `agentsi-agency::roles::{general, picker}`. Each write commits its files in the data root
//! and sends `data.changed` of kind `roles`: with the product for a roster or a promote, and
//! with no product for a general role, which every product can use.

use super::roles::{Work, blocking, read, write};
use super::{Ctx, Router};
use agentsi_agency::roles::{commit, general, picker};
use agentsi_proto::RpcError;
use agentsi_proto::events::{DataChanged, DataChangedParams};
use agentsi_proto::roles_api::{
    GeneralCreate, GeneralDelete, GeneralDeleteParams, GeneralDeleteResult, GeneralEntry,
    GeneralList, GeneralListParams, GeneralListResult, GeneralPromote, GeneralPromoteParams,
    GeneralRole, GeneralUpdate, GeneralWriteResult, RolesRoster, RolesRosterParams,
    RolesRosterResult,
};

pub fn register(r: Router) -> Router {
    r.add(RolesRoster, roles_roster)
        .add(GeneralList, general_list)
        .add(GeneralCreate, general_create)
        .add(GeneralUpdate, general_update)
        .add(GeneralDelete, general_delete)
        .add(GeneralPromote, general_promote)
}

const KIND: &str = "roles";

fn changed(ctx: &Ctx, product: &str) {
    ctx.core.publish::<DataChanged>(&DataChangedParams {
        product: product.into(),
        kind: KIND.into(),
    });
}

async fn roles_roster(ctx: Ctx, p: RolesRosterParams) -> Result<RolesRosterResult, RpcError> {
    let done = blocking(&ctx, move |w| {
        let product = w.registry.product(&p.product)?.clone();
        let root = w.root.clone();
        let roster = p.roles.clone();
        let target = product.clone();
        write(&w.store, move |tx| {
            picker::write_roster(tx, &root, &target, &roster)
        })?;
        let commit = commit::commit_files(
            &w.root,
            &[agentsi_agency::registry::FILE],
            &picker::commit_message(&product.id),
        );
        Ok(RolesRosterResult {
            product: product.id,
            roster: p.roles,
            commit,
        })
    })
    .await?;
    // The desk process holds the brief of its start, and the brief follows the roster.
    crate::desk::stop(&ctx.core, &done.product);
    changed(&ctx, &done.product);
    Ok(done)
}

async fn general_list(ctx: Ctx, _: GeneralListParams) -> Result<GeneralListResult, RpcError> {
    blocking(&ctx, |w| {
        let roles = general::list(&w.root)?;
        let roles = read(&w.store, |c| {
            roles
                .into_iter()
                .map(|role| {
                    let used_by = general::used_by(c, &w.registry.products, &role.id)?;
                    Ok(GeneralEntry { role, used_by })
                })
                .collect()
        })?;
        Ok(GeneralListResult {
            roles,
            builtin: agentsi_agency::roles::files::BUILTIN_ROLES
                .iter()
                .map(|r| r.to_string())
                .collect(),
        })
    })
    .await
}

fn committed(w: &Work, paths: &[&str], message: String, role: GeneralRole) -> GeneralWriteResult {
    GeneralWriteResult {
        commit: commit::commit_files(&w.root, paths, &message),
        role,
    }
}

async fn general_create(ctx: Ctx, p: GeneralRole) -> Result<GeneralWriteResult, RpcError> {
    let done = blocking(&ctx, move |w| {
        let role = general::create(&w.root, &p)?;
        let message = format!("role({}): new general role", role.id);
        Ok(committed(&w, &[general::FOLDER], message, role))
    })
    .await?;
    changed(&ctx, "");
    Ok(done)
}

async fn general_update(ctx: Ctx, p: GeneralRole) -> Result<GeneralWriteResult, RpcError> {
    let done = blocking(&ctx, move |w| {
        let role = general::update(&w.root, &p)?;
        let users = read(&w.store, |c| {
            general::used_by(c, &w.registry.products, &role.id)
        })?;
        let message = format!("role({}): edit the general role", role.id);
        Ok((committed(&w, &[general::FOLDER], message, role), users))
    })
    .await?;
    let (done, users) = done;
    // A desk process holds the roles of its start.
    for product in &users {
        crate::desk::stop(&ctx.core, product);
    }
    changed(&ctx, "");
    Ok(done)
}

async fn general_delete(ctx: Ctx, p: GeneralDeleteParams) -> Result<GeneralDeleteResult, RpcError> {
    let done = blocking(&ctx, move |w| {
        read(&w.store, |c| {
            general::delete(c, &w.root, &w.registry, &p.id)
        })?;
        let commit = commit::commit_files(
            &w.root,
            &[general::FOLDER],
            &format!("role({}): delete the general role", p.id),
        );
        Ok(GeneralDeleteResult { id: p.id, commit })
    })
    .await?;
    changed(&ctx, "");
    Ok(done)
}

async fn general_promote(
    ctx: Ctx,
    p: GeneralPromoteParams,
) -> Result<GeneralWriteResult, RpcError> {
    let product = p.product.clone();
    let done = blocking(&ctx, move |w| {
        let target = w.registry.product(&p.product)?.clone();
        let root = w.root.clone();
        let agent = p.agent.clone();
        let role = write(&w.store, move |tx| {
            general::promote(tx, &root, &target, &agent)
        })?;
        let message = format!("role({}): general role from {}", role.id, p.product);
        Ok(committed(&w, &[general::FOLDER], message, role))
    })
    .await?;
    crate::desk::stop(&ctx.core, &product);
    changed(&ctx, &product);
    Ok(done)
}
