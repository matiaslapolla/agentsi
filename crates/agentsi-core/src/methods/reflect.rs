//! `reflect.run`, `reflect.revert`, `reflect.cancel`, `memory.list` and
//! `memory.retire`: the methods of `agentsi reflect` and of the Memory screen. The
//! logic is in `agentsi_agency::reflect`. A reflection takes minutes, so it runs off
//! the event loop. After a write of memory, the core updates the index of the
//! product, because the `documents` rows hold the memory.

use super::{Ctx, Router};
use crate::state::Core;
use agentsi_agency::knowledge::index_product;
use agentsi_agency::meeting::running::{self, Entry, Refused};
use agentsi_agency::reflect::{
    self, Env, Error as ReflectError, Options, ReflectOutcome, memory, reflect as run_reflection,
    revert,
};
use agentsi_agency::registry::{Product, Registry};
use agentsi_agency::{Error as AgencyError, clock};
use agentsi_proto::RpcError;
use agentsi_proto::events::{DataChanged, DataChangedParams};
use agentsi_proto::reflect_api::{
    MemoryList, MemoryListParams, MemoryListResult, MemoryRetire, MemoryRetireParams,
    MemoryRetireResult, ReflectCancel, ReflectCancelParams, ReflectCancelResult, ReflectRevert,
    ReflectRevertParams, ReflectRevertResult, ReflectRun, ReflectRunParams, ReflectRunResult,
};
use agentsi_proto::rpc::ErrorKind;
use agentsi_store::Store;
use std::path::PathBuf;
use std::sync::Arc;

/// The program of each call. Tests put a fake one first on the `PATH`.
const CLAUDE: &str = "claude";

/// The name of the reflection of a product among the running work of the core. A core
/// that stops cancels it there. One name for a product: two reflections would apply
/// the same evidence twice.
fn running_name(product: &str) -> String {
    format!("reflect-{product}")
}

fn claim(product: &str) -> Result<Entry, RpcError> {
    Entry::for_product(running_name(product), product).map_err(|refused| match refused {
        Refused::NameTaken => RpcError::new(
            ErrorKind::Conflict,
            format!("A reflection of {product} runs already."),
            "Wait for it to end, or stop it with reflect.cancel.",
        ),
        Refused::CoreStops => RpcError::new(
            ErrorKind::Unavailable,
            "The core is stopping.",
            "Start the reflection again when the core runs.",
        ),
    })
}

pub fn register(r: Router) -> Router {
    r.add(ReflectRun, run)
        .add(ReflectRevert, revert_reflection)
        .add(ReflectCancel, cancel)
        .add(MemoryList, memory_list)
        .add(MemoryRetire, memory_retire)
}

fn rpc(e: ReflectError) -> RpcError {
    match e {
        ReflectError::Agency(AgencyError::NotFound(m)) | ReflectError::NotFound(m) => {
            RpcError::new(
                ErrorKind::NotFound,
                m,
                "Use memory.list for the items and the reflections that exist.",
            )
        }
        ReflectError::Agency(AgencyError::Unavailable(m)) => RpcError::new(
            ErrorKind::Unavailable,
            m,
            "Set AGENTSI_HOME to the data root. It holds agentsi.config.json and agentsi.db.",
        ),
        ReflectError::Conflict(m) => RpcError::new(
            ErrorKind::Conflict,
            m,
            "Read the state again with memory.list, then decide what to do.",
        ),
        ReflectError::Failed(m) => RpcError::internal(m),
    }
}

/// What a handler works with.
struct Work {
    root: PathBuf,
    registry: Registry,
    store: Arc<Store>,
}

fn work(core: &Core) -> reflect::Result<Work> {
    let root = agentsi_store::paths::data_root(None)?;
    let registry = Registry::load(&root)?;
    let store = core.store()?.ok_or_else(|| {
        AgencyError::Unavailable("the data root has no agentsi.db. Import the data first.".into())
    })?;
    Ok(Work {
        root,
        registry,
        store,
    })
}

/// Runs `f` off the event loop.
async fn blocking<T: Send + 'static>(
    ctx: &Ctx,
    f: impl FnOnce(Work) -> reflect::Result<T> + Send + 'static,
) -> Result<T, RpcError> {
    let core = ctx.core.clone();
    crate::blocking::spawn(move || f(work(&core).map_err(rpc)?).map_err(rpc))
        .await
        .map_err(|e| RpcError::internal(e.to_string()))?
}

/// The product of the registry, or `not_found`.
fn product_of(w: &Work, id: &str) -> reflect::Result<Product> {
    Ok(w.registry.product(id)?.clone())
}

/// `knowledge.index` for the product. The memory is written, so a failed index only warns.
fn reindex(w: &Work, product: &Product) -> Option<String> {
    index_product(&w.store, product, false)
        .err()
        .map(|e| format!("the index was not updated: {e}"))
}

fn changed(ctx: &Ctx, product: &str, kind: &str) {
    ctx.core.publish::<DataChanged>(&DataChangedParams {
        product: product.to_string(),
        kind: kind.to_string(),
    });
}

async fn run(ctx: Ctx, p: ReflectRunParams) -> Result<ReflectRunResult, RpcError> {
    let entry = claim(&p.product)?;
    let cancel = entry.cancel.clone();
    let product = p.product.clone();
    let options = Options {
        memory_only: p.memory_only,
        budget_usd: None,
    };
    let done: ReflectOutcome = blocking(&ctx, move |w| {
        // The blocking work owns its entry: an aborted handler must not free the product.
        let _entry = entry;
        let env = Env {
            store: &w.store,
            root: &w.root,
            registry: &w.registry,
            cancel: &cancel,
            claude: CLAUDE,
        };
        let mut done = run_reflection(&env, &product, &options)?;
        if done.entry.is_some() {
            let product = product_of(&w, &product)?;
            done.warnings.extend(reindex(&w, &product));
        }
        Ok(done)
    })
    .await?;
    if let Some(e) = &done.entry {
        changed(&ctx, &p.product, "memory");
        if e.proposal.as_ref().is_some_and(|x| x.decision_id.is_some()) {
            changed(&ctx, &p.product, "decision");
        }
    }
    Ok(ReflectRunResult {
        entry: done.entry,
        candidates: done.candidates,
        warnings: done.warnings,
    })
}

async fn revert_reflection(
    ctx: Ctx,
    p: ReflectRevertParams,
) -> Result<ReflectRevertResult, RpcError> {
    let (id, reflection) = (p.product.clone(), p.reflection);
    let entry = blocking(&ctx, move |w| {
        let product = product_of(&w, &id)?;
        let entry = revert(&w.store, &id, &reflection, clock::now_ms())?;
        // The CLI prints the entry. A failed index shows in the log of the core.
        if let Some(warning) = reindex(&w, &product) {
            eprintln!("{warning}");
        }
        Ok(entry)
    })
    .await?;
    changed(&ctx, &p.product, "memory");
    Ok(ReflectRevertResult { entry })
}

async fn cancel(_ctx: Ctx, p: ReflectCancelParams) -> Result<ReflectCancelResult, RpcError> {
    Ok(ReflectCancelResult {
        canceled: running::cancel(&running_name(&p.product)),
    })
}

async fn memory_list(ctx: Ctx, p: MemoryListParams) -> Result<MemoryListResult, RpcError> {
    blocking(&ctx, move |w| {
        let product = product_of(&w, &p.product)?;
        w.store.read(|c| Ok(memory::list(c, &product)))?
    })
    .await
}

async fn memory_retire(ctx: Ctx, p: MemoryRetireParams) -> Result<MemoryRetireResult, RpcError> {
    let id = p.product.clone();
    let item = blocking(&ctx, move |w| {
        let product = product_of(&w, &p.product)?;
        let at = clock::iso(clock::now_ms());
        let (agent, item_id, why) = (p.agent, p.id, p.why);
        let owner = product.id.clone();
        let item = w
            .store
            .try_write(move |tx| memory::retire_item(tx, &owner, &agent, &item_id, &why, &at))??;
        if let Some(warning) = reindex(&w, &product) {
            eprintln!("{warning}");
        }
        Ok(item)
    })
    .await?;
    changed(&ctx, &id, "memory");
    Ok(MemoryRetireResult { item })
}
