//! `git.*` and `git.config.*`: the git screen of a product, and the git config that AgentSI
//! gives to each git command of a product. The logic is in `agentsi-agency` (`gitpanel`,
//! `git_env`). A write sends `data.changed {product, kind: "git"}`.

use super::{Ctx, Router};
use crate::state::Core;
use agentsi_agency::git_env::{self, GitConfig};
use agentsi_agency::gitpanel::{self, Error as PanelError};
use agentsi_agency::registry::{Product, Registry};
use agentsi_proto::RpcError;
use agentsi_proto::events::{DataChanged, DataChangedParams};
use agentsi_proto::git_api::*;
use agentsi_proto::rpc::ErrorKind;
use std::path::PathBuf;
use std::sync::Arc;

pub fn register(r: Router) -> Router {
    r.add(GitStatus, status)
        .add(GitDiff, diff)
        .add(GitStage, stage)
        .add(GitUnstage, unstage)
        .add(GitCommit, commit)
        .add(GitLog, log)
        .add(GitConfigGet, config_get)
        .add(GitConfigSet, config_set)
}

fn unavailable(message: impl Into<String>, hint: &str) -> RpcError {
    RpcError::new(ErrorKind::Unavailable, message, hint)
}

fn rpc(e: PanelError) -> RpcError {
    match e {
        PanelError::Invalid(m) => RpcError::new(
            ErrorKind::Invalid,
            m,
            "Give a path inside the repo, and a repo that is one of the paths of the product.",
        ),
        PanelError::Git(m) => unavailable(m, "Check that the folder of the product is a git repo."),
    }
}

/// The registry, read from the data root.
fn registry() -> Result<Registry, RpcError> {
    let hint = "Set AGENTSI_HOME to the data root. It holds agentsi.config.json.";
    let root =
        agentsi_store::paths::data_root(None).map_err(|e| unavailable(e.to_string(), hint))?;
    Registry::load(&root).map_err(|e| unavailable(e.to_string(), hint))
}

fn product_of(registry: &Registry, id: &str) -> Result<Product, RpcError> {
    registry.product(id).cloned().map_err(|e| {
        RpcError::new(
            ErrorKind::NotFound,
            e.to_string(),
            "Use product.whoami with list: true for the products of the registry.",
        )
    })
}

fn changed(core: &Core, product: &str) {
    core.publish::<DataChanged>(&DataChangedParams {
        product: product.into(),
        kind: "git".into(),
    });
}

/// Runs `f` off the event loop with the repo of the request.
async fn in_repo<T: Send + 'static>(
    product: String,
    repo: Option<String>,
    f: impl FnOnce(PathBuf) -> Result<T, PanelError> + Send + 'static,
) -> Result<T, RpcError> {
    crate::blocking::spawn(move || {
        let product = product_of(&registry()?, &product)?;
        let repo = gitpanel::resolve_repo(&product, repo.as_deref()).map_err(rpc)?;
        f(repo).map_err(rpc)
    })
    .await
    .map_err(|e| RpcError::internal(e.to_string()))?
}

async fn status(_: Ctx, p: GitRepoParams) -> Result<GitStatusResult, RpcError> {
    in_repo(p.product, p.repo, |repo| gitpanel::status(&repo)).await
}

async fn diff(_: Ctx, p: GitDiffParams) -> Result<GitDiffResult, RpcError> {
    in_repo(p.product, p.repo, move |repo| {
        gitpanel::diff(&repo, &p.path, p.staged)
    })
    .await
}

async fn stage(ctx: Ctx, p: GitPathsParams) -> Result<GitDone, RpcError> {
    let product = p.product.clone();
    in_repo(p.product, p.repo, move |repo| {
        gitpanel::stage(&repo, &p.paths)
    })
    .await?;
    changed(&ctx.core, &product);
    Ok(GitDone {})
}

async fn unstage(ctx: Ctx, p: GitPathsParams) -> Result<GitDone, RpcError> {
    let product = p.product.clone();
    in_repo(p.product, p.repo, move |repo| {
        gitpanel::unstage(&repo, &p.paths)
    })
    .await?;
    changed(&ctx.core, &product);
    Ok(GitDone {})
}

async fn commit(ctx: Ctx, p: GitCommitParams) -> Result<GitCommitResult, RpcError> {
    let product = p.product.clone();
    let result = in_repo(p.product, p.repo, move |repo| {
        gitpanel::commit(&repo, &p.message)
    })
    .await?;
    if result.committed {
        changed(&ctx.core, &product);
    }
    Ok(result)
}

async fn log(_: Ctx, p: GitLogParams) -> Result<GitLogResult, RpcError> {
    in_repo(p.product, p.repo, move |repo| {
        gitpanel::log(&repo, p.limit).map(|commits| GitLogResult { commits })
    })
    .await
}

fn entries(list: &[(String, String)]) -> Vec<GitConfigEntry> {
    list.iter()
        .map(|(key, value)| GitConfigEntry {
            key: key.clone(),
            value: value.clone(),
        })
        .collect()
}

fn config_file() -> Result<PathBuf, RpcError> {
    git_env::path().ok_or_else(|| {
        unavailable(
            "no config folder: HOME is not set",
            "Set XDG_CONFIG_HOME or HOME for the core.",
        )
    })
}

fn config_result(
    file: &std::path::Path,
    product: Option<&str>,
) -> Result<GitConfigResult, RpcError> {
    let config = GitConfig::read(file).map_err(|e| {
        unavailable(
            e,
            "Fix or remove the file, or save the tables again from the git screen.",
        )
    })?;
    let (own, result) = match product {
        Some(id) => (
            config.product(id).to_vec(),
            config
                .resolve(id)
                .into_iter()
                .map(|r| GitConfigSource {
                    key: r.key,
                    value: r.value,
                    source: if r.from_product { "product" } else { "shared" }.into(),
                })
                .collect(),
        ),
        None => (Vec::new(), Vec::new()),
    };
    Ok(GitConfigResult {
        path: file.to_string_lossy().into_owned(),
        shared: entries(&config.shared),
        product: entries(&own),
        result,
    })
}

async fn config_get(_: Ctx, p: GitConfigGetParams) -> Result<GitConfigResult, RpcError> {
    crate::blocking::spawn(move || {
        if let Some(id) = &p.product {
            product_of(&registry()?, id)?;
        }
        config_result(&config_file()?, p.product.as_deref())
    })
    .await
    .map_err(|e| RpcError::internal(e.to_string()))?
}

async fn config_set(ctx: Ctx, p: GitConfigSetParams) -> Result<GitConfigResult, RpcError> {
    let core: Arc<Core> = ctx.core.clone();
    crate::blocking::spawn(move || {
        // The shared table changes the result of each product.
        let touched: Vec<String> = match &p.product {
            Some(id) => {
                product_of(&registry()?, id)?;
                vec![id.clone()]
            }
            None => registry()
                .map(|r| r.products.into_iter().map(|x| x.id).collect())
                .unwrap_or_default(),
        };
        let file = config_file()?;
        let list: Vec<(String, String)> = p
            .entries
            .iter()
            .map(|e| (e.key.clone(), e.value.clone()))
            .collect();
        git_env::set_table(&file, p.product.as_deref(), &list).map_err(|e| {
            RpcError::new(
                ErrorKind::Invalid,
                e,
                "Check the keys (section.name) and the values, then save again.",
            )
        })?;
        for id in &touched {
            changed(&core, id);
        }
        config_result(&file, p.product.as_deref())
    })
    .await
    .map_err(|e| RpcError::internal(e.to_string()))?
}
