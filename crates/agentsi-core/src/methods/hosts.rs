//! `voice.relay`: the voice events of a remote core, which the app sends to this core.

use super::{Ctx, Router};
use agentsi_proto::RpcError;
use agentsi_proto::core_api::Empty;
use agentsi_proto::events::{VoiceSpeak, VoiceSpeakParams, VoiceStop, VoiceStopParams};
use agentsi_proto::hosts_api::{
    RELAY_SEPARATOR, VoiceRelay, VoiceRelayParams, check_host, relayed_terminal,
};
use agentsi_proto::rpc::ErrorKind;

pub fn register(r: Router) -> Router {
    r.add(VoiceRelay, relay)
}

async fn relay(ctx: Ctx, p: VoiceRelayParams) -> Result<Empty, RpcError> {
    let bad = |message: String| {
        RpcError::new(
            ErrorKind::Invalid,
            message,
            "Give the host name of hosts.toml and the terminal id of the remote core.",
        )
    };
    check_host(&p.host).map_err(bad)?;
    if p.terminal.is_empty() || p.terminal.contains(RELAY_SEPARATOR) {
        return Err(bad(format!(
            "the terminal is empty or holds '{RELAY_SEPARATOR}'"
        )));
    }
    if ctx.core.terminals.get(&p.terminal).is_ok() {
        return Err(RpcError::new(
            ErrorKind::Conflict,
            format!("{} is a terminal of this core", p.terminal),
            "The host is this device. The local pill already speaks its replies.",
        ));
    }
    let terminal = relayed_terminal(&p.host, &p.terminal);
    match p.text {
        Some(text) => ctx.core.publish::<VoiceSpeak>(&VoiceSpeakParams {
            terminal,
            role: p.role.unwrap_or_else(|| "pm".into()),
            text,
        }),
        None => ctx.core.publish::<VoiceStop>(&VoiceStopParams { terminal }),
    }
    Ok(Empty {})
}
