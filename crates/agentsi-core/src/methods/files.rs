//! `files.*`: the file tree of a product. The logic is in `agentsi-agency` (`files`). A folder
//! that `files.list` read goes to the files watcher (`files_watch`), unless git ignores it.

use super::files_watch::Folder;
use super::{Ctx, Router};
use agentsi_agency::files::{self, Error, Root};
use agentsi_agency::registry::Registry;
use agentsi_proto::RpcError;
use agentsi_proto::files_api::*;
use agentsi_proto::rpc::ErrorKind;

pub fn register(r: Router) -> Router {
    r.add(FilesList, list)
        .add(FilesCreate, create)
        .add(FilesRename, rename)
        .add(FilesTrash, trash)
        .add(FilesFind, find)
}

fn rpc(e: Error) -> RpcError {
    match e {
        Error::Invalid(m) => RpcError::new(
            ErrorKind::Invalid,
            m,
            "Give a path from the folder of the product, with no .. and no .git part.",
        ),
        Error::NotFound(m) => RpcError::new(
            ErrorKind::NotFound,
            m,
            "List the folder again: the entry may have moved.",
        ),
        Error::Conflict(m) => RpcError::new(ErrorKind::Conflict, m, "Pick another name."),
        Error::Io(m) => RpcError::new(
            ErrorKind::Unavailable,
            m,
            "Check the permissions of the folder and that the system has a trash.",
        ),
    }
}

fn root_of(product: &str) -> Result<Root, RpcError> {
    let hint = "Set AGENTSI_HOME to the data root. It holds agentsi.config.json.";
    let unavailable = |m: String| RpcError::new(ErrorKind::Unavailable, m, hint);
    let data = agentsi_store::paths::data_root(None).map_err(|e| unavailable(e.to_string()))?;
    let registry = Registry::load(&data).map_err(|e| unavailable(e.to_string()))?;
    let product = registry.product(product).map_err(|e| {
        RpcError::new(
            ErrorKind::NotFound,
            e.to_string(),
            "Use product.whoami with list: true for the products of the registry.",
        )
    })?;
    Root::of(product).map_err(rpc)
}

/// Runs `f` off the event loop with the folder of the product.
async fn in_root<T: Send + 'static>(
    product: String,
    f: impl FnOnce(Root) -> Result<T, Error> + Send + 'static,
) -> Result<T, RpcError> {
    crate::blocking::spawn(move || f(root_of(&product)?).map_err(rpc))
        .await
        .map_err(|e| RpcError::internal(e.to_string()))?
}

async fn list(ctx: Ctx, p: FilesPathParams) -> Result<FilesListResult, RpcError> {
    let product = p.product.clone();
    let (result, watch) = in_root(p.product, move |root| {
        let result = root.list(&p.path)?;
        let dir = root.folder(&p.path)?;
        let watch = (!files::ignored_within(root.dir(), &dir)).then(|| Folder {
            product,
            path: result.path.clone(),
            dir,
        });
        Ok((result, watch))
    })
    .await?;
    if let Some(folder) = watch
        && ctx
            .core
            .has_subscriber(agentsi_proto::core_api::Topic::Files)
    {
        ctx.core.files.watch(&ctx.core, folder);
    }
    Ok(result)
}

async fn create(_: Ctx, p: FilesCreateParams) -> Result<FilesDone, RpcError> {
    in_root(p.product, move |root| root.create(&p.path, p.kind)).await?;
    Ok(FilesDone {})
}

async fn rename(_: Ctx, p: FilesRenameParams) -> Result<FilesDone, RpcError> {
    in_root(p.product, move |root| root.rename(&p.from, &p.to)).await?;
    Ok(FilesDone {})
}

async fn trash(_: Ctx, p: FilesPathParams) -> Result<FilesDone, RpcError> {
    in_root(p.product, move |root| root.trash(&p.path)).await?;
    Ok(FilesDone {})
}

async fn find(_: Ctx, p: FilesFindParams) -> Result<FilesFindResult, RpcError> {
    let limit = p.limit.unwrap_or(50);
    in_root(p.product, move |root| Ok(root.find(&p.query, limit))).await
}
