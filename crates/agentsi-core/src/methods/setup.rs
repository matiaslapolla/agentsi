//! `setup.scan`, `setup.draft` and `setup.outcome`: the methods of the setup screen. The
//! logic is in `agentsi-agency::setup`.

use super::{Ctx, Router};
use agentsi_agency::registry::Registry;
use agentsi_agency::story::claude::run_call;
use agentsi_agency::{clock, setup};
use agentsi_proto::RpcError;
use agentsi_proto::core_api::Empty;
use agentsi_proto::rpc::ErrorKind;
use agentsi_proto::setup_api::{
    FieldOutcome, SetupDraft, SetupDraftParams, SetupDraftResult, SetupOutcome, SetupOutcomeParams,
    SetupScan, SetupScanParams, SetupScanResult,
};
use serde_json::{Value, json};
use std::path::{Path, PathBuf};
use std::time::Instant;

pub fn register(r: Router) -> Router {
    r.add(SetupScan, scan)
        .add(SetupDraft, draft)
        .add(SetupOutcome, outcome)
}

fn invalid(m: String) -> RpcError {
    RpcError::new(
        ErrorKind::Invalid,
        m,
        "Give an existing absolute folder on the host of the core.",
    )
}

/// The folder of a request: an existing absolute folder.
fn folder(path: &str) -> Result<PathBuf, RpcError> {
    let dir = PathBuf::from(path);
    if !dir.is_absolute() || !dir.is_dir() {
        return Err(invalid(format!(
            "\"{path}\" is not an existing absolute folder."
        )));
    }
    Ok(dir)
}

async fn scan(_: Ctx, p: SetupScanParams) -> Result<SetupScanResult, RpcError> {
    let dir = folder(&p.path)?;
    crate::blocking::spawn(move || {
        let registry = agentsi_store::paths::data_root(None)
            .ok()
            .and_then(|root| Registry::load(&root).ok());
        setup::scan(&dir, registry.as_ref())
    })
    .await
    .map_err(|e| RpcError::internal(e.to_string()))
}

fn unavailable(m: String) -> RpcError {
    RpcError::new(
        ErrorKind::Unavailable,
        m,
        "Set AGENTSI_HOME to the data root. It holds agentsi.config.json and agentsi.db.",
    )
}

fn data_root() -> Result<PathBuf, RpcError> {
    agentsi_store::paths::data_root(None).map_err(|e| unavailable(e.to_string()))
}

/// Appends a line to the log. A failed write only goes to the core log: the call stands.
fn log(root: &Path, line: Value) {
    if let Err(e) = setup::log_line(root, &line) {
        eprintln!("core: cannot write logs/setup.jsonl: {e}");
    }
}

async fn draft(_: Ctx, p: SetupDraftParams) -> Result<SetupDraftResult, RpcError> {
    let dir = folder(&p.path)?;
    let root = data_root()?;
    crate::blocking::spawn(move || {
        let started = Instant::now();
        let context = setup::context(&dir);
        let prompt = setup::draft_prompt(&dir, &context, p.previous.as_ref(), &p.failures);
        let call = setup::draft_call(&dir, prompt);
        let ran = run_call(&call, "setup");
        let seconds = started.elapsed().as_secs_f64();
        let (output, usage) = match ran {
            Ok(done) => done,
            Err(error) => {
                log(
                    &root,
                    json!({"at": clock::iso(clock::now_ms()), "kind": "draft",
                           "path": dir.display().to_string(), "files": context.files.len(),
                           "bytes": context.bytes, "repair": p.previous.is_some(),
                           "error": error, "seconds": seconds}),
                );
                return Err(RpcError::internal(error));
            }
        };
        let no_data: Vec<String> = output["no_data"]
            .as_array()
            .map(|a| {
                a.iter()
                    .filter_map(|v| v.as_str().map(String::from))
                    .collect()
            })
            .unwrap_or_default();
        let now = clock::now_ms();
        let id = format!("d{now}");
        log(
            &root,
            json!({"at": clock::iso(now), "kind": "draft", "draft": id,
                   "path": dir.display().to_string(), "files": context.files.len(),
                   "bytes": context.bytes, "repair": p.previous.is_some(),
                   "no_data": no_data.len(), "cost_usd": usage.cost_usd, "seconds": seconds}),
        );
        Ok(SetupDraftResult {
            draft: id,
            fields: output["fields"].clone(),
            no_data,
            cost_usd: usage.cost_usd,
            seconds,
        })
    })
    .await
    .map_err(|e| RpcError::internal(e.to_string()))?
}

async fn outcome(_: Ctx, p: SetupOutcomeParams) -> Result<Empty, RpcError> {
    if p.draft.trim().is_empty() || p.product.trim().is_empty() {
        return Err(RpcError::new(
            ErrorKind::Invalid,
            "setup.outcome needs the draft id and the product.",
            "Send the draft id of setup.draft and the id of the applied product.",
        ));
    }
    let root = data_root()?;
    crate::blocking::spawn(move || {
        let count = |o: FieldOutcome| p.fields.values().filter(|v| **v == o).count();
        log(
            &root,
            json!({"at": clock::iso(clock::now_ms()), "kind": "outcome", "draft": p.draft,
                   "product": p.product, "kept": count(FieldOutcome::Kept),
                   "edited": count(FieldOutcome::Edited),
                   "cleared": count(FieldOutcome::Cleared), "fields": p.fields}),
        );
        Empty {}
    })
    .await
    .map_err(|e| RpcError::internal(e.to_string()))
}
