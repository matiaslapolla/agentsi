//! `core.hello`, `core.subscribe`, `core.shutdown` and `rpc.discover`.

use super::{Ctx, Router};
use agentsi_proto::RpcError;
use agentsi_proto::core_api::{
    Discover, Empty, Hello, HelloParams, HelloResult, Shutdown, Subscribe, SubscribeParams,
    SubscribeResult,
};

pub fn register(r: Router) -> Router {
    r.add(Hello, hello)
        .add(Subscribe, subscribe)
        .add(Shutdown, shutdown)
        .add(Discover, discover)
}

async fn hello(ctx: Ctx, params: HelloParams) -> Result<HelloResult, RpcError> {
    if let Some(client) = &params.client {
        eprintln!("core: hello from {client}, framed: {}", params.framed);
    }
    if params.framed {
        ctx.session.ask_framed();
    }
    Ok(HelloResult {
        protocol: agentsi_proto::PROTOCOL,
        version: env!("CARGO_PKG_VERSION").into(),
        features: ctx.core.features(),
    })
}

async fn subscribe(ctx: Ctx, params: SubscribeParams) -> Result<SubscribeResult, RpcError> {
    let topics = ctx.session.add_topics(&ctx.core, &params.topics);
    Ok(SubscribeResult { topics })
}

async fn shutdown(ctx: Ctx, _: Empty) -> Result<Empty, RpcError> {
    ctx.session.ask_shutdown();
    Ok(Empty {})
}

/// The schema of this core never changes while it runs, and making it takes milliseconds.
static SCHEMA: std::sync::LazyLock<serde_json::Value> =
    std::sync::LazyLock::new(agentsi_proto::schema::openrpc);

async fn discover(_: Ctx, _: Empty) -> Result<serde_json::Value, RpcError> {
    Ok(SCHEMA.clone())
}
