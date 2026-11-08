//! `core.status` (U24) and `data.backup` (U26b).

use super::{Ctx, Router};
use agentsi_proto::RpcError;
use agentsi_proto::core_api::{Empty, Topic};
use agentsi_proto::ops_api::{
    CoreStatus, CoreStatusResult, DataBackup, DataBackupParams, DataBackupResult, DatabaseStatus,
};
use agentsi_proto::rpc::ErrorKind;
use agentsi_store::backup::{self, KEEP_BACKUPS};
use agentsi_store::{db, paths};
use std::path::PathBuf;
use std::time::SystemTime;

pub fn register(r: Router) -> Router {
    r.add(CoreStatus, status).add(DataBackup, backup)
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
    crate::blocking::spawn(move || {
        let root = paths::data_root(None).map_err(|e| unavailable(e.to_string()))?;
        let database = match core.store().map_err(|e| unavailable(e.to_string()))? {
            None => None,
            Some(store) => {
                let schema_version = store
                    .read(|c| {
                        Ok(c.pragma_query_value(None, "user_version", |r| r.get::<_, u32>(0))?)
                    })
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
    })
    .await
    .map_err(|e| RpcError::internal(e.to_string()))?
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
