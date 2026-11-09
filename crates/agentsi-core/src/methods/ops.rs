//! `core.status` (U24), `data.backup` (U26b) and `core.doctor`.

use super::{Ctx, Router};
use crate::state::Core;
use agentsi_proto::RpcError;
use agentsi_proto::core_api::{Empty, Topic};
use agentsi_proto::doctor::{self, Probes};
use agentsi_proto::ops_api::{
    CoreDoctor, CoreDoctorResult, CoreStatus, CoreStatusResult, DataBackup, DataBackupParams,
    DataBackupResult, DatabaseStatus, DoctorCheck,
};
use agentsi_proto::rpc::ErrorKind;
use agentsi_store::backup::{self, KEEP_BACKUPS};
use agentsi_store::{db, paths};
use std::path::PathBuf;
use std::sync::Arc;
use std::time::SystemTime;

pub fn register(r: Router) -> Router {
    r.add(CoreStatus, status)
        .add(DataBackup, backup)
        .add(CoreDoctor, doctor)
}

fn unavailable(message: String) -> RpcError {
    RpcError::new(
        ErrorKind::Unavailable,
        message,
        "Set AGENTSI_HOME to the data root. It holds agentsi.config.json and agentsi.db.",
    )
}

async fn status(ctx: Ctx, _: Empty) -> Result<CoreStatusResult, RpcError> {
    let core = ctx.core.clone();
    crate::blocking::spawn(move || status_of(&core))
        .await
        .map_err(|e| RpcError::internal(e.to_string()))?
}

fn status_of(core: &Core) -> Result<CoreStatusResult, RpcError> {
    let root = paths::data_root(None).map_err(|e| unavailable(e.to_string()))?;
    let database = match core.store().map_err(|e| unavailable(e.to_string()))? {
        None => None,
        Some(store) => {
            let schema_version = store
                .read(|c| Ok(c.pragma_query_value(None, "user_version", |r| r.get::<_, u32>(0))?))
                .map_err(|e| RpcError::internal(e.to_string()))?;
            Some(DatabaseStatus {
                path: paths::db_path(&root).display().to_string(),
                schema_version,
                latest: db::MIGRATIONS.last().map_or(0, |m| m.0),
            })
        }
    };
    Ok(CoreStatusResult {
        version: env!("CARGO_PKG_VERSION").into(),
        data_root: root.display().to_string(),
        database,
        pill: core.has_subscriber(Topic::Dictation),
    })
}

/// The host of the core. The core and the models rows read the core itself.
struct Host(Arc<Core>);

impl Probes for Host {
    fn output(&mut self, cmd: &str, args: &[&str]) -> Option<String> {
        doctor::run_tool(cmd, args)
    }

    fn sandbox(&mut self) -> bool {
        doctor::sandbox_starts()
    }

    fn core(&mut self) -> Result<CoreStatusResult, String> {
        status_of(&self.0).map_err(|e| e.message)
    }

    fn models(&mut self) -> Result<(usize, usize), String> {
        Ok(self.0.models.downloaded())
    }
}

async fn doctor(ctx: Ctx, _: Empty) -> Result<CoreDoctorResult, RpcError> {
    let core = ctx.core.clone();
    let checks = crate::blocking::spawn(move || doctor::checks(&mut Host(core)))
        .await
        .map_err(|e| RpcError::internal(e.to_string()))?;
    Ok(CoreDoctorResult {
        checks: checks
            .into_iter()
            .map(|c| DoctorCheck {
                name: c.name.into(),
                level: c.level,
                status: c.status,
                detail: c.detail,
                blocks: c.blocks.into(),
            })
            .collect(),
    })
}

/// The backup of the writer connection. It holds the writer, so no write of the core
/// lands in the middle of the copy, and the readers go on.
async fn backup(ctx: Ctx, p: DataBackupParams) -> Result<DataBackupResult, RpcError> {
    let keep = match p.keep {
        None => KEEP_BACKUPS,
        Some(k) if k >= 1 => k as usize,
        Some(_) => {
            return Err(RpcError::new(
                ErrorKind::Invalid,
                "keep must be 1 or more",
                "Pass a number of backups to keep, 1 or more.",
            ));
        }
    };
    let core = ctx.core.clone();
    crate::blocking::spawn(move || {
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
        let done = store
            .with_writer(move |conn| backup::backup(conn, &root, keep, SystemTime::now()))
            .map_err(|e| RpcError::internal(e.to_string()))?;
        let text = |p: PathBuf| p.display().to_string();
        Ok(DataBackupResult {
            file: text(done.file),
            registry: done.registry.map(text),
            removed: done.removed.into_iter().map(text).collect(),
        })
    })
    .await
    .map_err(|e| RpcError::internal(e.to_string()))?
}
