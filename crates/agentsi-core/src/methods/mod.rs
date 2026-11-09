//! The router: the handler of each method of the catalog in `agentsi-proto`.
//!
//! To add a namespace, write `pub fn register(r: Router) -> Router` in its
//! module and add one `.group(...)` line to `router()`. A test fails when the
//! router and the catalog do not have the same methods.

mod agency;
mod call;
mod core_api;
mod data;
mod desk;
mod git;
mod hosts;
mod knowledge;
mod marks;
mod meeting;
pub mod models;
mod ops;
mod reflect;
mod registry;
mod roles;
mod story;
mod terminal;
mod tickets;
mod vocabulary;
mod voice;

use crate::session::Session;
use crate::state::Core;
use agentsi_proto::{Method, RpcError};
use serde_json::Value;
use std::collections::BTreeMap;
use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;

pub fn router() -> Router {
    Router::default()
        .group(core_api::register)
        .group(terminal::register)
        .group(voice::register)
        .group(vocabulary::register)
        .group(desk::register)
        .group(agency::register)
        .group(marks::register)
        .group(knowledge::register)
        .group(models::register)
        .group(hosts::register)
        .group(roles::register)
        .group(story::register)
        .group(meeting::register)
        .group(call::register)
        .group(ops::register)
        .group(data::register)
        .group(reflect::register)
        .group(git::register)
        .group(tickets::register)
        .group(registry::register)
}

/// What a handler can use: the core, and the state of its connection.
#[derive(Clone)]
pub struct Ctx {
    pub core: Arc<Core>,
    pub session: Arc<Session>,
    /// The stream channels that this request opened. Their frames start after the reply.
    pub opened: Arc<std::sync::Mutex<Vec<u32>>>,
}

pub type Reply = Pin<Box<dyn Future<Output = Result<Value, RpcError>> + Send>>;
type Handler = Box<dyn Fn(Ctx, Value) -> Reply + Send + Sync>;

#[derive(Default)]
pub struct Router {
    handlers: BTreeMap<&'static str, Handler>,
}

impl Router {
    /// Adds the handler of method `M`. The router parses the params and
    /// returns `invalid` for params that do not match `M::Params`.
    pub fn add<M, F, Fut>(mut self, _method: M, handler: F) -> Router
    where
        M: Method,
        F: Fn(Ctx, M::Params) -> Fut + Send + Sync + 'static,
        Fut: Future<Output = Result<M::Result, RpcError>> + Send + 'static,
    {
        let boxed: Handler =
            Box::new(
                move |ctx, params| match serde_json::from_value::<M::Params>(params) {
                    Err(e) => Box::pin(std::future::ready(Err(RpcError::invalid_params(
                        e.to_string(),
                    )))),
                    Ok(params) => {
                        let reply = handler(ctx, params);
                        Box::pin(async move {
                            let result = reply.await?;
                            serde_json::to_value(result)
                                .map_err(|e| RpcError::internal(e.to_string()))
                        })
                    }
                },
            );
        assert!(
            self.handlers.insert(M::NAME, boxed).is_none(),
            "{} has two handlers",
            M::NAME
        );
        self
    }

    pub fn group(self, register: fn(Router) -> Router) -> Router {
        register(self)
    }

    pub fn names(&self) -> impl Iterator<Item = &'static str> + '_ {
        self.handlers.keys().copied()
    }

    pub fn call(&self, ctx: Ctx, method: &str, params: Value) -> Reply {
        match self.handlers.get(method) {
            Some(handler) => handler(ctx, params),
            None => Box::pin(std::future::ready(Err(RpcError::unknown_method(method)))),
        }
    }
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeSet;

    #[test]
    fn the_router_has_a_handler_for_each_method_of_the_catalog() {
        let routed: BTreeSet<&str> = super::router().names().collect();
        let catalog: BTreeSet<&str> = agentsi_proto::catalog()
            .methods
            .iter()
            .map(|m| m.name)
            .collect();
        assert_eq!(routed, catalog);
    }
}
