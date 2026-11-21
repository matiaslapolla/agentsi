//! `git.*` and `git.config.*`: the git screen of a product, and the git config that AgentSI
//! gives to each git command of a product. The logic is in `agentsi-agency` (`gitpanel`,
//! `git_env`). A write sends `data.changed {product, kind: "git"}`. A fetch, a pull and a
//! push send `git.progress` while they run. `git_watch` sends `git.changed`.

use super::{Ctx, Router};
use crate::state::Core;
use agentsi_agency::git_env::{self, GitConfig};
use agentsi_agency::gitpanel::{self, Error as PanelError};
use agentsi_agency::registry::{Product, Registry};
use agentsi_proto::RpcError;
use agentsi_proto::events::{DataChanged, DataChangedParams, GitProgress, GitProgressParams};
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
        .add(GitSummary, summary)
        .add(GitShow, show)
        .add(GitBranches, branches)
        .add(GitBranchCreate, branch_create)
        .add(GitSwitch, switch)
        .add(GitOutgoing, outgoing)
        .add(GitFetch, fetch)
        .add(GitPull, pull)
        .add(GitPush, push)
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

async fn summary(_: Ctx, p: GitSummaryParams) -> Result<GitSummaryResult, RpcError> {
    let invalid = || {
        RpcError::new(
            ErrorKind::Invalid,
            "Give a product or a cwd, not both.",
            "git.summary reads the first path of a product, or any folder.",
        )
    };
    let dir = match (p.product, p.cwd) {
        (Some(product), None) => {
            return in_repo(product, None, |repo| {
                gitpanel::summary(&repo).map(|summary| GitSummaryResult { summary })
            })
            .await;
        }
        (None, Some(cwd)) => PathBuf::from(cwd),
        _ => return Err(invalid()),
    };
    if !dir.is_dir() {
        return Ok(GitSummaryResult { summary: None });
    }
    crate::blocking::spawn(move || {
        gitpanel::summary(&dir)
            .map(|summary| GitSummaryResult { summary })
            .map_err(rpc)
    })
    .await
    .map_err(|e| RpcError::internal(e.to_string()))?
}

async fn show(_: Ctx, p: GitShowParams) -> Result<GitShowResult, RpcError> {
    in_repo(p.product, p.repo, move |repo| {
        gitpanel::show(&repo, &p.commit)
    })
    .await
}

async fn branches(_: Ctx, p: GitRepoParams) -> Result<GitBranchesResult, RpcError> {
    in_repo(p.product, p.repo, |repo| gitpanel::branches(&repo)).await
}

async fn branch_create(ctx: Ctx, p: GitBranchCreateParams) -> Result<GitDone, RpcError> {
    let product = p.product.clone();
    in_repo(p.product, p.repo, move |repo| {
        gitpanel::branch_create(&repo, &p.name, p.from.as_deref())
    })
    .await?;
    changed(&ctx.core, &product);
    Ok(GitDone {})
}

async fn switch(ctx: Ctx, p: GitSwitchParams) -> Result<GitSwitchResult, RpcError> {
    let product = p.product.clone();
    let result = in_repo(p.product, p.repo, move |repo| {
        gitpanel::switch(&repo, &p.name)
    })
    .await?;
    if result.outcome == GitOutcome::Done {
        changed(&ctx.core, &product);
    }
    Ok(result)
}

async fn outgoing(_: Ctx, p: GitRepoParams) -> Result<GitOutgoingResult, RpcError> {
    in_repo(p.product, p.repo, |repo| gitpanel::outgoing(&repo)).await
}

/// Runs a fetch, a pull or a push, sends its progress lines, then `data.changed`: a failed
/// call can still move the remote refs.
async fn network(
    ctx: Ctx,
    p: GitRepoParams,
    op: &'static str,
    f: impl FnOnce(&std::path::Path, &mut dyn FnMut(&str)) -> Result<GitSyncResult, PanelError>
    + Send
    + 'static,
) -> Result<GitSyncResult, RpcError> {
    let (core, product, id) = (ctx.core.clone(), p.product.clone(), p.product.clone());
    let result = in_repo(id, p.repo, move |repo| {
        let mut progress = |line: &str| {
            core.publish::<GitProgress>(&GitProgressParams {
                product: product.clone(),
                op: op.into(),
                line: line.into(),
            });
        };
        f(&repo, &mut progress)
    })
    .await?;
    changed(&ctx.core, &p.product);
    Ok(result)
}

async fn fetch(ctx: Ctx, p: GitRepoParams) -> Result<GitSyncResult, RpcError> {
    network(ctx, p, "fetch", |repo, progress| {
        Ok(gitpanel::fetch(repo, progress))
    })
    .await
}

async fn pull(ctx: Ctx, p: GitRepoParams) -> Result<GitSyncResult, RpcError> {
    network(ctx, p, "pull", |repo, progress| {
        Ok(gitpanel::pull(repo, progress))
    })
    .await
}

async fn push(ctx: Ctx, p: GitPushParams) -> Result<GitSyncResult, RpcError> {
    let repo = GitRepoParams {
        product: p.product,
        repo: p.repo,
    };
    network(ctx, repo, "push", move |dir, progress| {
        let expect = p.branch.as_deref().zip(p.head.as_deref());
        gitpanel::push(dir, expect, progress)
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
