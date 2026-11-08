//! The connection to the core. If no core runs, the client starts one.

use agentsi_proto::client::{Client, ClientError, connect_or_start, find_core_bin};
use agentsi_proto::core_api::{Hello, HelloParams};
use agentsi_proto::method::Method;
use agentsi_proto::rpc::{ErrorKind, Notification};
use agentsi_proto::socket::socket_path;

pub struct Core(Client);

impl Core {
    pub fn connect() -> Result<Core, String> {
        let socket = socket_path()?;
        let bin = find_core_bin()
            .ok_or("cannot find agentsi-core. Put it on the PATH, or set AGENTSI_CORE_BIN.")?;
        let stream = connect_or_start(&socket, &bin).map_err(|e| e.to_string())?;
        let mut client = Client::new(stream).map_err(|e| e.to_string())?;
        client
            .call::<Hello>(&HelloParams {
                framed: false,
                client: Some("agentsi-cli".into()),
            })
            .map_err(message)?;
        Ok(Core(client))
    }

    pub fn call<M: Method>(&mut self, params: &M::Params) -> Result<M::Result, String> {
        self.0.call::<M>(params).map_err(message)
    }

    /// Like `call`, and the error keeps the kind that the core gave it, for a
    /// command that prints some kinds in another way.
    pub fn call_kind<M: Method>(
        &mut self,
        params: &M::Params,
    ) -> Result<M::Result, (Option<ErrorKind>, String)> {
        self.0.call::<M>(params).map_err(|e| {
            let kind = match &e {
                ClientError::Rpc(r) => Some(r.kind()),
                _ => None,
            };
            (kind, message(e))
        })
    }

    /// The next event of the topics of this connection. It waits until one comes.
    pub fn next_event(&mut self) -> Result<Notification, String> {
        self.0.next_notification().map_err(message)
    }
}

/// The text that a command prints for an error of the core.
fn message(e: ClientError) -> String {
    match e {
        ClientError::Rpc(e) => e.message,
        other => other.to_string(),
    }
}
