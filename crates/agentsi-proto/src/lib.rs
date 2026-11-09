//! Protocol types, framing, OpenRPC schema.
//!
//! The protocol of the AgentSI core, version 1. It is unstable until M2.
//! Spec: `docs/specs/agentsi-desktop-protocol.md`. Schema: `schema/core.openrpc.json`.

pub mod agency_api;
pub mod call_api;
pub mod clear_api;
pub mod client;
pub mod compat;
pub mod core_api;
pub mod desk_api;
pub mod desk_session_api;
pub mod doctor;
pub mod events;
pub mod frame;
pub mod git_api;
pub mod hosts_api;
pub mod knowledge_api;
pub mod marks_api;
pub mod meeting_api;
pub mod method;
pub mod models_api;
pub mod ops_api;
pub mod reflect_api;
pub mod registry_api;
pub mod roles_api;
pub mod rpc;
pub mod schema;
pub mod setup_api;
pub mod socket;
pub mod story_api;
pub mod terminal_api;
pub mod ticket_api;
pub mod vocabulary_api;
pub mod voice_api;

pub use method::{Catalog, Event, Method, catalog};
pub use rpc::{ErrorKind, RpcError};
pub use schema::PROTOCOL;
