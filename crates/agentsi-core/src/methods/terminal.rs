//! `terminal.*`: create, list, attach, input, resize and kill.

use super::{Ctx, Router};
use crate::terminal::stream::Attach;
use agentsi_proto::RpcError;
use agentsi_proto::core_api::Empty;
use agentsi_proto::rpc::ErrorKind;
use agentsi_proto::terminal_api::{
    Attach as AttachMethod, AttachParams, AttachResult, Create, CreateParams, Input, InputParams,
    Kill, List, ListResult, Resize, ResizeParams, TerminalId, TerminalInfo,
};
use std::sync::Arc;

pub fn register(r: Router) -> Router {
    r.add(Create, create)
        .add(List, list)
        .add(AttachMethod, attach)
        .add(Input, input)
        .add(Resize, resize)
        .add(Kill, kill)
}

async fn create(ctx: Ctx, params: CreateParams) -> Result<TerminalInfo, RpcError> {
    ctx.core.terminals.create(&ctx.core, params)
}

async fn list(ctx: Ctx, _: Empty) -> Result<ListResult, RpcError> {
    Ok(ListResult {
        terminals: ctx.core.terminals.list(),
    })
}

async fn attach(ctx: Ctx, params: AttachParams) -> Result<AttachResult, RpcError> {
    if !ctx.session.in_frames() {
        return Err(RpcError::new(
            ErrorKind::Invalid,
            "terminal.attach needs framed mode",
            "Call core.hello with framed: true first.",
        ));
    }
    let term = ctx.core.terminals.get(&params.id)?;
    let channel = ctx.session.next_channel();
    let attach = Arc::new(Attach::new(
        ctx.core.terminals.new_attach_id(),
        channel,
        params.observe,
    ));
    ctx.session.add_stream(attach.clone(), term.clone());
    crate::state::lock(&ctx.opened).push(channel);
    term.attach(attach);
    Ok(AttachResult { channel })
}

async fn input(ctx: Ctx, params: InputParams) -> Result<Empty, RpcError> {
    let term = ctx.core.terminals.get(&params.id)?;
    if params.submit {
        term.submit(params.text.as_bytes())?;
    } else {
        term.input(params.text.as_bytes())?;
    }
    Ok(Empty {})
}

async fn resize(ctx: Ctx, params: ResizeParams) -> Result<Empty, RpcError> {
    let term = ctx.core.terminals.get(&params.id)?;
    let cell = params.cell_width.zip(params.cell_height);
    term.resize(params.cols, params.rows, cell)?;
    Ok(Empty {})
}

async fn kill(ctx: Ctx, params: TerminalId) -> Result<Empty, RpcError> {
    let term = ctx.core.terminals.get(&params.id)?;
    term.kill(&ctx.core.terminals);
    Ok(Empty {})
}
