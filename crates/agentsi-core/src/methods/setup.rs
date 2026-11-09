//! `setup.scan`: the methods of the setup screen. The logic is in `agentsi-agency::setup`.

use super::{Ctx, Router};
use agentsi_agency::registry::Registry;
use agentsi_agency::setup;
use agentsi_proto::RpcError;
use agentsi_proto::rpc::ErrorKind;
use agentsi_proto::setup_api::{SetupScan, SetupScanParams, SetupScanResult};
use std::path::PathBuf;

pub fn register(r: Router) -> Router {
    r.add(SetupScan, scan)
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
