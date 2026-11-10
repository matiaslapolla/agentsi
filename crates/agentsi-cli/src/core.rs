//! The connection to the core. If no core runs, the client starts one.

use agentsi_proto::client::{Client, ClientError, connect_or_start, find_core_bin};
use agentsi_proto::core_api::{Hello, HelloParams};
use agentsi_proto::method::Method;
use agentsi_proto::rpc::{ErrorKind, Notification};
use agentsi_proto::socket::socket_path;

pub struct Core {
    client: Client,
    /// A write request went out on this connection since the last `take_wrote`.
    wrote: bool,
}

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
        Ok(Core {
            client,
            wrote: false,
        })
    }

    /// A connection over a stream that a test serves.
    #[cfg(test)]
    pub fn over(stream: std::os::unix::net::UnixStream) -> Core {
        Core {
            client: Client::new(stream).expect("the stream clones"),
            wrote: false,
        }
    }

    pub fn call<M: Method>(&mut self, params: &M::Params) -> Result<M::Result, String> {
        self.client.call::<M>(params).map_err(message)
    }

    /// Like `call`, for a request that changes data: the connection remembers that it went
    /// out, so the caller does not send it a second time.
    pub fn write<M: Method>(&mut self, params: &M::Params) -> Result<M::Result, String> {
        self.wrote = true;
        self.call::<M>(params)
    }

    /// True when a write request went out since the last call, and resets it.
    pub fn take_wrote(&mut self) -> bool {
        std::mem::take(&mut self.wrote)
    }

    /// Like `call`, and the error keeps the kind that the core gave it, for a
    /// command that prints some kinds in another way.
    pub fn call_kind<M: Method>(
        &mut self,
        params: &M::Params,
    ) -> Result<M::Result, (Option<ErrorKind>, String)> {
        self.client.call::<M>(params).map_err(|e| {
            let kind = match &e {
                ClientError::Rpc(r) => Some(r.kind()),
                _ => None,
            };
            (kind, message(e))
        })
    }

    /// The next event of the topics of this connection. It waits until one comes.
    pub fn next_event(&mut self) -> Result<Notification, String> {
        self.client.next_notification().map_err(message)
    }
}

/// The text that a command prints for an error of the core.
fn message(e: ClientError) -> String {
    match e {
        ClientError::Rpc(e) => e.message,
        other => other.to_string(),
    }
}
