//! `vocabulary.*`: the list of terms that the transcript is corrected with.

use super::{Ctx, Router};
use crate::dictate;
use agentsi_proto::RpcError;
use agentsi_proto::core_api::Empty;
use agentsi_proto::vocabulary_api::{
    VocabularyGet, VocabularyList, VocabularySet, VocabularySetParams,
};

pub fn register(r: Router) -> Router {
    r.add(VocabularyGet, get).add(VocabularySet, set)
}

async fn get(_: Ctx, _: Empty) -> Result<VocabularyList, RpcError> {
    blocking(|| {
        let (vocab, path) = dictate::load_vocabulary()?;
        Ok(dictate::list_of(&vocab, &path))
    })
    .await
}

async fn set(_: Ctx, p: VocabularySetParams) -> Result<VocabularyList, RpcError> {
    blocking(move || dictate::save_vocabulary(p.entries)).await
}

async fn blocking<T: Send + 'static>(
    work: impl FnOnce() -> Result<T, RpcError> + Send + 'static,
) -> Result<T, RpcError> {
    crate::blocking::spawn(work)
        .await
        .map_err(|e| RpcError::internal(e.to_string()))?
}
