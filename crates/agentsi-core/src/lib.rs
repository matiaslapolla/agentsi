//! The AgentSI core: the daemon of one user on one host.
//! Spec: `docs/specs/agentsi-desktop-protocol.md`.

pub mod backup;
pub mod blocking;
pub mod clock;
pub mod converse;
pub mod desk;
pub mod desk_data;
pub mod dictate;
pub mod hook;
pub mod import;
pub mod methods;
pub mod relay;
pub mod server;
pub mod session;
pub mod state;
pub mod terminal;
pub mod voice_session;
