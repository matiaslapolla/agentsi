//! The `core` namespace, and `rpc.discover`.

use crate::method::{Catalog, Method};
use crate::rpc::ErrorKind;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

pub fn register(c: &mut Catalog) {
    c.method::<Hello>()
        .method::<Subscribe>()
        .method::<Shutdown>()
        .method::<Discover>();
}

/// The params or the result of a method that has none.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct Empty {}

pub struct Hello;

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct HelloParams {
    /// true: after the reply, the connection uses framed mode.
    #[serde(default)]
    pub framed: bool,
    /// The name of the client, for the log of the core.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub client: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct HelloResult {
    /// The protocol number. A break of the protocol increments it.
    pub protocol: u32,
    /// The version of the core program.
    pub version: String,
    /// What this core can do: `framed`, and each method namespace.
    pub features: Vec<String>,
}

impl Method for Hello {
    const NAME: &'static str = "core.hello";
    const DESCRIPTION: &'static str = "Gives the protocol number, the version and the features of the core. \
        With framed: true, the connection changes to framed mode after the reply.";
    const ERRORS: &'static [ErrorKind] = &[];
    type Params = HelloParams;
    type Result = HelloResult;
}

/// A group of events. A subscriber gets each event whose name starts with
/// its topic.
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize, JsonSchema,
)]
#[serde(rename_all = "snake_case")]
pub enum Topic {
    Terminal,
    Data,
    Meeting,
    Voice,
    Dictation,
    Models,
}

impl Topic {
    /// The topic of an event name, or `None` for a `core` event, which goes
    /// to each subscriber. The `converse` events belong to the `voice` topic.
    pub fn of_event(name: &str) -> Option<Topic> {
        let namespace = name.split('.').next()?;
        let namespace = if namespace == "converse" {
            "voice"
        } else {
            namespace
        };
        serde_json::from_value(serde_json::Value::String(namespace.into())).ok()
    }
}

pub struct Subscribe;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct SubscribeParams {
    /// The topics to add to this connection.
    pub topics: Vec<Topic>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct SubscribeResult {
    /// Each topic of this connection after the call.
    pub topics: Vec<Topic>,
}

impl Method for Subscribe {
    const NAME: &'static str = "core.subscribe";
    const DESCRIPTION: &'static str = "Adds topics to this connection. The core then sends their events \
        as notifications on channel 0. A second call adds more topics.";
    const ERRORS: &'static [ErrorKind] = &[];
    type Params = SubscribeParams;
    type Result = SubscribeResult;
}

pub struct Shutdown;

impl Method for Shutdown {
    const NAME: &'static str = "core.shutdown";
    const DESCRIPTION: &'static str =
        "Stops the core after the reply. The core removes its socket.";
    const ERRORS: &'static [ErrorKind] = &[];
    type Params = Empty;
    type Result = Empty;
}

pub struct Discover;

impl Method for Discover {
    const NAME: &'static str = "rpc.discover";
    const DESCRIPTION: &'static str = "Gives the OpenRPC schema of this core.";
    const ERRORS: &'static [ErrorKind] = &[];
    type Params = Empty;
    type Result = serde_json::Value;
}
