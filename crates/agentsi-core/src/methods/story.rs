//! The methods of stories mode (U23): `story.*`, what `agentsi story` and the
//! stories screen call. The logic is in `agentsi-agency::story`. Each write
//! sends `data.changed {product, kind: "stories"}`. `story.run` starts the
//! session on a thread of the core and returns at once.

use super::{Ctx, Router};
use crate::state::Core;
use agentsi_agency::Error as AgencyError;
use agentsi_agency::registry::{Product, Registry};
use agentsi_agency::story::config::story_config;
use agentsi_agency::story::lifecycle;
use agentsi_agency::story::propose::{self, ProposeCtx};
use agentsi_agency::story::record::{actions, markdown};
use agentsi_agency::story::run::{self, RunCtx};
use agentsi_agency::story::store::{self, read_with};
use agentsi_agency::story::{Error as StoryError, Result as StoryResult};
use agentsi_proto::RpcError;
use agentsi_proto::events::{DataChanged, DataChangedParams};
use agentsi_proto::rpc::ErrorKind;
use agentsi_proto::story_api::*;
use agentsi_store::Store;
use std::path::PathBuf;
use std::sync::Arc;

pub fn register(r: Router) -> Router {
    r.add(StoryConfigGet, config)
        .add(StoryList, list)
        .add(StoryGet, get)
        .add(StoryNew, new)
        .add(StorySuggest, suggest)
        .add(StoryApprove, approve)
        .add(StoryDiscard, discard)
        .add(StoryChanges, changes)
        .add(StoryRun, run_story)
        .add(StoryExec, exec)
        .add(StoryReview, review)
        .add(StoryAccept, accept)
        .add(StoryReject, reject)
        .add(StoryResultSet, result)
}

fn rpc(e: StoryError) -> RpcError {
    match e {
        StoryError::Invalid(m) => RpcError::new(
            ErrorKind::Invalid,
            m,
            "Check the story, its state and the arguments, then run it again.",
        ),
        StoryError::NotFound(m) => RpcError::new(
            ErrorKind::NotFound,
            m,
            "Use story.list for the stories of the product.",
        ),
        StoryError::Gone(m) => RpcError::new(
            ErrorKind::Conflict,
            m,
            "The session was rejected. Read the story again.",
        ),
        StoryError::Agency(AgencyError::NotFound(m)) => RpcError::new(
            ErrorKind::NotFound,
            m,
            "Use product.whoami with list: true for the products of the registry.",
        ),
        StoryError::Agency(AgencyError::Unavailable(m)) => RpcError::new(
            ErrorKind::Unavailable,
            m,
            "Set AGENTSI_HOME to the data root. It holds agentsi.config.json and agentsi.db.",
        ),
    }
}

/// What a handler works with.
struct Work {
    root: PathBuf,
    registry: Registry,
    store: Arc<Store>,
    product: Product,
    core: Arc<Core>,
}

impl Work {
    fn config(&self) -> StoryResult<StoryConfig> {
        story_config(&self.root, &self.product)
    }

    fn changed(&self) {
        changed(&self.core, &self.product.id);
    }

    /// The first check of a run. The app calls `story.run` with no `story.config` before it.
    fn refuse_archived(&self) -> StoryResult<()> {
        let id = &self.product.id;
        if read_with(&self.store, |c| store::product_archived(c, id))? {
            return Err(StoryError::invalid(format!(
                "Product {id} is archived. Run agentsi restore --product {id} to bring it back."
            )));
        }
        Ok(())
    }

    fn run_ctx(&self) -> StoryResult<RunCtx> {
        let mut ctx = RunCtx::new(
            self.store.clone(),
            self.root.clone(),
            self.product.clone(),
            self.config()?,
        );
        let (core, product) = (self.core.clone(), self.product.id.clone());
        ctx.notify = Arc::new(move || changed(&core, &product));
        Ok(ctx)
    }

    fn propose_ctx(&self) -> StoryResult<ProposeCtx> {
        Ok(ProposeCtx::new(
            self.store.clone(),
            self.root.clone(),
            self.product.clone(),
            self.config()?,
            self.registry.owner.clone(),
        ))
    }
}

fn changed(core: &Core, product: &str) {
    core.publish::<DataChanged>(&DataChangedParams {
        product: product.into(),
        kind: "stories".into(),
    });
}

/// Runs `f` off the event loop, with the data root, the registry, the store and the product.
async fn blocking<T: Send + 'static>(
    ctx: &Ctx,
    product: String,
    f: impl FnOnce(Work) -> StoryResult<T> + Send + 'static,
) -> Result<T, RpcError> {
    let core = ctx.core.clone();
    crate::blocking::spawn(move || {
        let agency = |e: AgencyError| rpc(StoryError::Agency(e));
        let root = agentsi_store::paths::data_root(None).map_err(|e| agency(e.into()))?;
        let registry = Registry::load(&root).map_err(agency)?;
        let product = registry.product(&product).map_err(agency)?.clone();
        let store = core.store().map_err(|e| agency(e.into()))?.ok_or_else(|| {
            agency(AgencyError::Unavailable(
                "the data root has no agentsi.db. Import the data first.".into(),
            ))
        })?;
        f(Work {
            root,
            registry,
            store,
            product,
            core,
        })
        .map_err(rpc)
    })
    .await
    .map_err(|e| RpcError::internal(e.to_string()))?
}

async fn config(ctx: Ctx, p: StoryConfigParams) -> Result<StoryConfig, RpcError> {
    blocking(&ctx, p.product, move |w| {
        if p.run {
            w.refuse_archived()?;
        }
        w.config()
    })
    .await
}

async fn list(ctx: Ctx, p: StoryListParams) -> Result<StoryListResult, RpcError> {
    blocking(&ctx, p.product, move |w| {
        w.config()?;
        let stories = read_with(&w.store, |c| {
            store::list(c, &w.product.id, p.include_archived)
        })?;
        Ok(StoryListResult {
            product: w.product.id.clone(),
            stories,
        })
    })
    .await
}

async fn get(ctx: Ctx, p: StoryIdParams) -> Result<StoryGetResult, RpcError> {
    blocking(&ctx, p.product, move |w| {
        w.config()?;
        read_with(&w.store, |c| {
            let story = store::load(c, &w.product.id, &p.id)?;
            Ok(StoryGetResult {
                markdown: markdown(&story),
                actions: actions(&story),
                log: store::read_log(c, &w.product.id, &p.id)?,
                archived: store::archived_ids(c, &w.product.id)?.contains(&p.id),
                story,
            })
        })
    })
    .await
}

async fn new(ctx: Ctx, p: StoryNewParams) -> Result<StoryProposeResult, RpcError> {
    blocking(&ctx, p.product, move |w| {
        let r = propose::propose_story(
            &w.propose_ctx()?,
            &p.text,
            p.kind.as_deref(),
            p.minutes.as_deref(),
        )?;
        w.changed();
        Ok(r)
    })
    .await
}

async fn suggest(ctx: Ctx, p: ProductParams) -> Result<StoryProposeResult, RpcError> {
    blocking(&ctx, p.product, move |w| {
        let r = propose::suggest_stories(&w.propose_ctx()?)?;
        w.changed();
        Ok(r)
    })
    .await
}

async fn decide(ctx: Ctx, p: StoryIdParams, verdict: &'static str) -> Result<StoryOne, RpcError> {
    blocking(&ctx, p.product, move |w| {
        w.config()?;
        let story = lifecycle::decide(&w.store, &w.product, &p.id, verdict, "")?;
        w.changed();
        Ok(StoryOne { story })
    })
    .await
}

async fn approve(ctx: Ctx, p: StoryIdParams) -> Result<StoryOne, RpcError> {
    decide(ctx, p, "approve").await
}

async fn discard(ctx: Ctx, p: StoryIdParams) -> Result<StoryOne, RpcError> {
    decide(ctx, p, "discard").await
}

async fn changes(ctx: Ctx, p: StoryChangesParams) -> Result<StoryProposeResult, RpcError> {
    blocking(&ctx, p.product, move |w| {
        let r = if p.rewrite {
            propose::revise_story(&w.propose_ctx()?, &p.id, &p.note)?
        } else {
            w.config()?;
            let story = lifecycle::decide(&w.store, &w.product, &p.id, "changes", &p.note)?;
            StoryProposeResult {
                stories: vec![story],
                usage: Vec::new(),
                total_cost_usd: 0.0,
            }
        };
        w.changed();
        Ok(r)
    })
    .await
}

async fn run_story(ctx: Ctx, p: StoryIdParams) -> Result<StoryOne, RpcError> {
    blocking(&ctx, p.product, move |w| {
        w.refuse_archived()?;
        let run_ctx = w.run_ctx()?;
        let story = run::queue(&run_ctx, &p.id)?;
        if let Some(s) = &story.session {
            let line = format!("[{}] Queued {} from {}.\n", s.started_at, s.branch, s.base);
            store::append_log(&w.store, &w.product.id, &p.id, &line)?;
        }
        w.changed();
        let id = p.id.clone();
        std::thread::Builder::new()
            .name(format!("story-{id}"))
            .spawn(move || {
                if let Err(e) = run::execute(&run_ctx, &id) {
                    let line = format!(
                        "[{}] The run crashed: {e}\n",
                        agentsi_agency::clock::iso(agentsi_agency::clock::now_ms())
                    );
                    let _ = store::append_log(&run_ctx.store, &run_ctx.product.id, &id, &line);
                }
            })
            .map_err(|e| StoryError::invalid(format!("cannot start the story session: {e}")))?;
        Ok(StoryOne { story })
    })
    .await
}

async fn exec(ctx: Ctx, p: StoryIdParams) -> Result<StoryOne, RpcError> {
    blocking(&ctx, p.product, move |w| {
        let story = run::execute(&w.run_ctx()?, &p.id)?;
        Ok(StoryOne { story })
    })
    .await
}

async fn review(ctx: Ctx, p: StoryIdParams) -> Result<StoryReviewResult, RpcError> {
    blocking(&ctx, p.product, move |w| {
        let config = w.config()?;
        let r = lifecycle::review(&w.store, &w.product.id, &p.id, &config)?;
        Ok(StoryReviewResult {
            story: r.story,
            stat: r.stat,
            diff: r.diff,
            repo: config.repo,
        })
    })
    .await
}

async fn accept(ctx: Ctx, p: StoryIdParams) -> Result<StoryAcceptResult, RpcError> {
    blocking(&ctx, p.product, move |w| {
        let a = lifecycle::accept(&w.store, &w.product, &p.id, &w.config()?)?;
        w.changed();
        Ok(StoryAcceptResult {
            story: a.story,
            result: a.result,
            commit: a.commit,
        })
    })
    .await
}

async fn reject(ctx: Ctx, p: StoryRejectParams) -> Result<StoryOne, RpcError> {
    blocking(&ctx, p.product, move |w| {
        let story = lifecycle::reject(&w.store, &w.product, &p.id, p.force, &w.config()?)?;
        w.changed();
        Ok(StoryOne { story })
    })
    .await
}

async fn result(ctx: Ctx, p: StoryResultParams) -> Result<StoryOne, RpcError> {
    blocking(&ctx, p.product, move |w| {
        let config = w.config()?;
        let story =
            lifecycle::record_result(&w.store, &w.product, &p.id, &p.kind, &p.commit, &config)?;
        w.changed();
        Ok(StoryOne { story })
    })
    .await
}
