//! `data.clear_plan` and `data.clear`: the delete of the "Data" section and
//! `agentsi clear`. The rows and files are in `agentsi_store::clear`. This module
//! adds the registry, the check for running work, the backup that comes first, and
//! the plans that a delete must match.

use super::{Ctx, Router};
use crate::state::{Core, lock};
use agentsi_agency::meeting::running;
use agentsi_agency::registry::{self, Registry, normalize};
use agentsi_agency::roles::registry_edit::remove_products;
use agentsi_agency::story;
use agentsi_proto::RpcError;
use agentsi_proto::clear_api::{
    ClearFailure, ClearFile, ClearItem, ClearItemPlan, ClearKind, ClearTable, DataClear,
    DataClearParams, DataClearPlan, DataClearPlanParams, DataClearPlanResult, DataClearResult,
};
use agentsi_proto::events::{DataChanged, DataChangedParams};
use agentsi_proto::rpc::ErrorKind;
use agentsi_store::backup::{self, KEEP_BACKUPS};
use agentsi_store::clear::{self, Item, ItemPlan, Kind};
use agentsi_store::{Store, paths};
use std::collections::{BTreeSet, HashMap};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, LazyLock, Mutex};
use std::time::{Duration, Instant, SystemTime};

/// How long a plan holds for `data.clear`.
const PLAN_TTL: Duration = Duration::from_secs(5 * 60);
/// The plans a core keeps. Past it, the oldest go.
const MAX_PLANS: usize = 32;

/// The plans of `data.clear_plan` by id: when it was made, and what it listed.
static PLANS: LazyLock<Mutex<HashMap<String, (Instant, String)>>> = LazyLock::new(Mutex::default);
static NEXT_PLAN: AtomicU64 = AtomicU64::new(1);

pub fn register(r: Router) -> Router {
    r.add(DataClearPlan, plan).add(DataClear, clear_data)
}

fn unavailable(message: String) -> RpcError {
    RpcError::new(
        ErrorKind::Unavailable,
        message,
        "Set AGENTSI_HOME to the data root. It holds agentsi.config.json and agentsi.db.",
    )
}

fn internal(e: impl ToString) -> RpcError {
    RpcError::internal(e.to_string())
}

fn kind_of(k: ClearKind) -> Kind {
    match k {
        ClearKind::Product => Kind::Product,
        ClearKind::Meetings => Kind::Meetings,
        ClearKind::Stories => Kind::Stories,
        ClearKind::RoleLearning => Kind::RoleLearning,
        ClearKind::CallAudio => Kind::CallAudio,
        ClearKind::VoiceTranscripts => Kind::VoiceTranscripts,
        ClearKind::Logs => Kind::Logs,
        ClearKind::Backups => Kind::Backups,
        ClearKind::Everything => Kind::Everything,
    }
}

fn store_items(items: &[ClearItem]) -> Result<Vec<Item>, RpcError> {
    let converted: Vec<Item> = items
        .iter()
        .map(|i| Item {
            kind: kind_of(i.item),
            product: i.product.clone(),
            before: i.before.clone(),
        })
        .collect();
    for item in &converted {
        clear::validate(item).map_err(|e| {
            RpcError::new(
                ErrorKind::Invalid,
                e.to_string(),
                "Check the item: a product id, and a date as YYYY-MM-DD.",
            )
        })?;
    }
    Ok(converted)
}

/// What a handler works with.
struct Work {
    root: PathBuf,
    store: Arc<Store>,
    /// `None` while the data root has no readable registry.
    registry: Option<Registry>,
}

fn work(core: &Core) -> Result<Work, RpcError> {
    let root = paths::data_root(None).map_err(|e| unavailable(e.to_string()))?;
    let store = core
        .store()
        .map_err(|e| unavailable(e.to_string()))?
        .ok_or_else(|| {
            unavailable(format!(
                "{} does not exist",
                paths::db_path(&root).display()
            ))
        })?;
    let registry = Registry::load(&root).ok();
    Ok(Work {
        root,
        store,
        registry,
    })
}

impl Work {
    fn registry_products(&self) -> Vec<String> {
        self.registry
            .iter()
            .flat_map(|r| r.products.iter().map(|p| p.id.clone()))
            .collect()
    }

    /// The products that the delete takes out of the registry.
    fn registry_ids(&self, items: &[Item]) -> Vec<String> {
        let known = self.registry_products();
        let mut ids = BTreeSet::new();
        for item in items {
            match (item.kind, &item.product) {
                (Kind::Everything, _) => ids.extend(known.iter().cloned()),
                (Kind::Product, Some(p)) if known.contains(p) => {
                    ids.insert(p.clone());
                }
                _ => {}
            }
        }
        ids.into_iter().collect()
    }

    /// The folders of the registered products.
    fn repos(&self) -> Vec<PathBuf> {
        self.registry
            .iter()
            .flat_map(|r| r.products.iter().flat_map(|p| p.paths.iter()))
            .map(|p| normalize(Path::new(p)))
            .collect()
    }
}

/// A delete never touches a repo: a registered product folder that lies in a file to
/// delete, or holds one, stops the delete.
fn repo_clash(root: &Path, repos: &[PathBuf], plans: &[ItemPlan]) -> Option<RpcError> {
    plans.iter().flat_map(|p| &p.files).find_map(|file| {
        let target = normalize(&root.join(&file.path));
        repos
            .iter()
            .any(|r| r.starts_with(&target) || target.starts_with(r))
            .then(|| {
                RpcError::new(
                    ErrorKind::Invalid,
                    format!(
                        "{} holds or lies in the folder of a product repo.",
                        file.path
                    ),
                    "AgentSI never deletes a repo. Take the repo out of the data root first.",
                )
            })
    })
}

/// The plan in protocol shape. `known` are the products of the registry.
fn describe(known: &[String], items: &[Item], plans: &[ItemPlan]) -> Vec<ClearItemPlan> {
    items
        .iter()
        .zip(plans)
        .map(|(item, plan)| {
            let registry = match (item.kind, &item.product) {
                (Kind::Everything, _) => !known.is_empty(),
                (Kind::Product, Some(p)) => known.contains(p),
                _ => false,
            };
            ClearItemPlan {
                item: ClearItem {
                    item: proto_kind(item.kind),
                    product: item.product.clone(),
                    before: item.before.clone(),
                },
                tables: plan
                    .tables
                    .iter()
                    .map(|(t, n)| ClearTable {
                        table: (*t).into(),
                        rows: *n,
                    })
                    .collect(),
                files: plan
                    .files
                    .iter()
                    .map(|f| ClearFile {
                        path: f.path.clone(),
                        bytes: f.bytes,
                        dir: f.dir,
                    })
                    .collect(),
                rows: plan.rows(),
                bytes: plan.files.iter().map(|f| f.bytes).sum(),
                registry,
            }
        })
        .collect()
}

fn proto_kind(k: Kind) -> ClearKind {
    match k {
        Kind::Product => ClearKind::Product,
        Kind::Meetings => ClearKind::Meetings,
        Kind::Stories => ClearKind::Stories,
        Kind::RoleLearning => ClearKind::RoleLearning,
        Kind::CallAudio => ClearKind::CallAudio,
        Kind::VoiceTranscripts => ClearKind::VoiceTranscripts,
        Kind::Logs => ClearKind::Logs,
        Kind::Backups => ClearKind::Backups,
        Kind::Everything => ClearKind::Everything,
    }
}

/// What a plan lists, without the sizes: the sizes of a log change while the owner reads.
fn digest(items: &[ClearItemPlan]) -> String {
    let flat: Vec<ClearItemPlan> = items
        .iter()
        .cloned()
        .map(|mut i| {
            i.bytes = 0;
            i.files.iter_mut().for_each(|f| f.bytes = 0);
            i
        })
        .collect();
    serde_json::to_string(&flat).unwrap_or_default()
}

fn remember(plan_digest: String) -> String {
    let mut plans = lock(&PLANS);
    plans.retain(|_, (at, _)| at.elapsed() < PLAN_TTL);
    while plans.len() >= MAX_PLANS {
        let Some(oldest) = plans
            .iter()
            .min_by_key(|(_, (at, _))| *at)
            .map(|(k, _)| k.clone())
        else {
            break;
        };
        plans.remove(&oldest);
    }
    let id = format!(
        "plan-{}-{}",
        std::process::id(),
        NEXT_PLAN.fetch_add(1, Ordering::Relaxed)
    );
    plans.insert(id.clone(), (Instant::now(), plan_digest));
    id
}

/// Takes the plan out: a plan serves one delete.
fn take(plan_id: &str) -> Option<String> {
    let (at, digest) = lock(&PLANS).remove(plan_id)?;
    (at.elapsed() < PLAN_TTL).then_some(digest)
}

fn plan_changed(why: &str) -> RpcError {
    RpcError::new(
        ErrorKind::PlanChanged,
        why,
        "Review the plan again, then delete.",
    )
}

async fn plan(ctx: Ctx, p: DataClearPlanParams) -> Result<DataClearPlanResult, RpcError> {
    let items = store_items(&p.items)?;
    let core = ctx.core.clone();
    crate::blocking::spawn(move || {
        let w = work(&core)?;
        let (root, store_items_) = (w.root.clone(), items.clone());
        // A dry run on the writer, so no other write lands in the middle of it.
        let plans = w
            .store
            .with_writer(move |conn| clear::run(conn, &root, &store_items_, false))
            .map_err(internal)?;
        if let Some(e) = repo_clash(&w.root, &w.repos(), &plans) {
            return Err(e);
        }
        let described = describe(&w.registry_products(), &items, &plans);
        for (item, plan) in items.iter().zip(&described) {
            if item.kind == Kind::Product
                && !plan.registry
                && plan.rows == 0
                && plan.files.is_empty()
            {
                return Err(RpcError::new(
                    ErrorKind::NotFound,
                    format!(
                        "No data of product {}.",
                        item.product.as_deref().unwrap_or("")
                    ),
                    "Use product.whoami with list: true for the products of the registry.",
                ));
            }
        }
        let rows = described.iter().map(|i| i.rows).sum();
        let files = described.iter().map(|i| i.files.len() as u64).sum();
        let bytes = described.iter().map(|i| i.bytes).sum();
        Ok(DataClearPlanResult {
            plan_id: remember(digest(&described)),
            data_root: w.root.display().to_string(),
            root_bytes: clear::root_bytes(&w.root),
            items: described,
            rows,
            files,
            bytes,
        })
    })
    .await
    .map_err(internal)?
}

/// The running work that blocks a delete of `items`, named for a person.
fn running_work(core: &Core, items: &[Item]) -> Vec<String> {
    let all = items
        .iter()
        .any(|i| i.kind == Kind::Everything || (i.kind == Kind::CallAudio && i.product.is_none()));
    let products: BTreeSet<&str> = items.iter().filter_map(|i| i.product.as_deref()).collect();
    let hit = |product: Option<&str>| all || product.is_some_and(|p| products.contains(p));
    let mut out = Vec::new();
    for t in core.terminals.list() {
        if t.running && hit(t.product.as_deref()) {
            out.push(format!("terminal {}", t.id));
        }
    }
    for (name, product) in running::active() {
        if hit(product.as_deref()) {
            out.push(format!("meeting or reflection {name}"));
        }
    }
    for (product, id) in story::run::active() {
        if hit(Some(&product)) {
            out.push(format!("story session {id} of {product}"));
        }
    }
    out
}

/// Stops the desk processes whose rows a delete of `items` removes. A stopped actor stores
/// no more items, so no row comes back after the delete.
fn stop_desks(core: &Core, items: &[Item]) {
    let everything = items.iter().any(|i| i.kind == Kind::Everything);
    for product in core.desks.running() {
        let covered = items
            .iter()
            .any(|i| i.kind == Kind::Product && i.product.as_deref() == Some(product.as_str()));
        if everything || covered {
            crate::desk::stop(core, &product);
        }
    }
}

enum Outcome {
    /// The data differs from the plan.
    Changed,
    Refused(RpcError),
    BackupFailed(String),
    Done {
        backup: backup::BackupResult,
        plans: Vec<ItemPlan>,
        products: Vec<String>,
    },
}

async fn clear_data(ctx: Ctx, p: DataClearParams) -> Result<DataClearResult, RpcError> {
    let items = store_items(&p.items)?;
    if items.is_empty() {
        return Err(RpcError::new(
            ErrorKind::Invalid,
            "No item to delete.",
            "Name at least one item.",
        ));
    }
    let planned = take(&p.plan_id)
        .ok_or_else(|| plan_changed("The plan is unknown or older than 5 minutes."))?;
    let busy = running_work(&ctx.core, &items);
    if !busy.is_empty() {
        return Err(RpcError::new(
            ErrorKind::Busy,
            format!("Running work blocks the delete: {}.", busy.join(", ")),
            "Stop that work or wait until it ends, then delete again.",
        ));
    }
    stop_desks(&ctx.core, &items);
    let core = ctx.core.clone();
    let done = crate::blocking::spawn(move || run_clear(&core, items, planned))
        .await
        .map_err(internal)??;
    for product in &done.touched {
        ctx.core.publish::<DataChanged>(&DataChangedParams {
            product: product.clone(),
            kind: "clear".into(),
        });
    }
    if done.result.rows > 0 {
        // The checkpoint and the VACUUM hold the writer for as long as they run.
        let store = done.store;
        drop(crate::blocking::spawn(move || {
            if let Err(e) = store.with_writer(|c| clear::compact(c)) {
                eprintln!("core: the database was not compacted after a clear: {e}");
            }
        }));
    }
    Ok(done.result)
}

struct Done {
    result: DataClearResult,
    touched: Vec<String>,
    store: Arc<Store>,
}

fn run_clear(core: &Core, items: Vec<Item>, expected: String) -> Result<Done, RpcError> {
    let w = work(core)?;
    let known = w.registry_products();
    let (root, run_items, repos, known_in) =
        (w.root.clone(), items.clone(), w.repos(), known.clone());
    // The plan is checked on the writer, then the backup and the delete follow on it, so no
    // write of the core lands between the check and the delete.
    let outcome = w
        .store
        .with_writer(move |conn| {
            let plans = clear::run(conn, &root, &run_items, false)?;
            if let Some(e) = repo_clash(&root, &repos, &plans) {
                return Ok(Outcome::Refused(e));
            }
            if digest(&describe(&known_in, &run_items, &plans)) != expected {
                return Ok(Outcome::Changed);
            }
            let products = clear::product_ids(conn)?;
            let backup = match backup::backup(conn, &root, KEEP_BACKUPS, SystemTime::now()) {
                Ok(b) => b,
                Err(e) => return Ok(Outcome::BackupFailed(e.to_string())),
            };
            let plans = clear::run(conn, &root, &run_items, true)?;
            Ok(Outcome::Done {
                backup,
                plans,
                products,
            })
        })
        .map_err(internal)?;
    let (backup, plans, db_products) = match outcome {
        Outcome::Changed => return Err(plan_changed("The data changed since the plan.")),
        Outcome::Refused(e) => return Err(e),
        Outcome::BackupFailed(e) => {
            return Err(RpcError::new(
                ErrorKind::Unavailable,
                format!("The backup failed, so nothing was deleted: {e}"),
                "Free space in the backups folder of the data root, then delete again.",
            ));
        }
        Outcome::Done {
            backup,
            plans,
            products,
        } => (backup, plans, products),
    };

    let keep: Vec<PathBuf> = std::iter::once(backup.file.clone())
        .chain(backup.registry.clone())
        .collect();
    let (files, mut failed) = clear::remove_files(&w.root, &plans, &keep);
    let mut registry_removed = Vec::new();
    let ids = w.registry_ids(&items);
    if !ids.is_empty() {
        match edit_registry(&w.root, &ids) {
            Ok(()) => registry_removed = ids,
            Err(error) => failed.push(clear::Failed {
                path: registry::FILE.into(),
                error,
            }),
        }
    }

    let mut touched = BTreeSet::new();
    for item in &items {
        match (&item.product, item.kind) {
            (_, Kind::Everything) | (None, Kind::CallAudio) => {
                touched.extend(db_products.iter().cloned());
                touched.extend(known.iter().cloned());
            }
            (Some(p), _) => {
                touched.insert(p.clone());
            }
            _ => {}
        }
    }
    let text = |p: PathBuf| p.display().to_string();
    Ok(Done {
        result: DataClearResult {
            backup: text(backup.file),
            registry_backup: backup.registry.map(text),
            rows: plans.iter().map(ItemPlan::rows).sum(),
            files,
            bytes: plans.iter().flat_map(|p| &p.files).map(|f| f.bytes).sum(),
            registry_removed,
            failed: failed
                .into_iter()
                .map(|f| ClearFailure {
                    path: f.path,
                    error: f.error,
                })
                .collect(),
        },
        touched: touched.into_iter().collect(),
        store: w.store,
    })
}

/// Takes the products out of `agentsi.config.json`, keeping every other line.
fn edit_registry(root: &Path, ids: &[String]) -> Result<(), String> {
    let file = root.join(registry::FILE);
    let text = std::fs::read_to_string(&file).map_err(|e| e.to_string())?;
    let refs: Vec<&str> = ids.iter().map(String::as_str).collect();
    let edited = remove_products(&text, &refs)?;
    let tmp = root.join(format!("{}.{}.tmp", registry::FILE, std::process::id()));
    std::fs::write(&tmp, edited).map_err(|e| e.to_string())?;
    std::fs::rename(&tmp, &file).map_err(|e| {
        let _ = std::fs::remove_file(&tmp);
        e.to_string()
    })
}
