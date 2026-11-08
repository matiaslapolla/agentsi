//! SSH hosts: the app sends the voice events of a remote core to the local core,
//! so the local pill speaks them (U17).

use crate::core_api::Empty;
use crate::method::{Catalog, Method};
use crate::rpc::ErrorKind;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

pub fn register(c: &mut Catalog) {
    c.method::<VoiceRelay>();
}

/// The character between the host name and the remote terminal id in a relayed id.
pub const RELAY_SEPARATOR: char = '/';

/// The terminal id of a relayed event: `<host>/<terminal>`.
pub fn relayed_terminal(host: &str, terminal: &str) -> String {
    format!("{host}{RELAY_SEPARATOR}{terminal}")
}

/// Checks a host name of `hosts.toml` and of `voice.relay`: not empty, no `/`, no
/// control character.
pub fn check_host(name: &str) -> Result<(), String> {
    if name.is_empty() || name.contains(RELAY_SEPARATOR) || name.chars().any(char::is_control) {
        return Err(format!(
            "the host name {name:?} is empty or holds '{RELAY_SEPARATOR}' or a control character"
        ));
    }
    Ok(())
}

/// true for the terminal id of an event that a core already relayed.
pub fn is_relayed(terminal: &str) -> bool {
    terminal.contains(RELAY_SEPARATOR)
}

pub struct VoiceRelay;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct VoiceRelayParams {
    /// The name of the SSH host in `hosts.toml`.
    pub host: String,
    /// The terminal id on the remote core.
    pub terminal: String,
    /// The role of a `voice.speak`. The local core picks no voice: the pill reads the voice of
    /// the role from its own data root.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub role: Option<String>,
    /// The text of a `voice.speak`. Without it, the relay is a `voice.stop`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub text: Option<String>,
}

impl Method for VoiceRelay {
    const NAME: &'static str = "voice.relay";
    const DESCRIPTION: &'static str = "Sends a voice.speak (with text) or a voice.stop (without text) \
        of a remote core to the subscribers of this core. The terminal of the event is \
        <host>/<terminal>. A terminal of this core returns conflict, so a host that is this \
        device does not speak a reply two times.";
    const ERRORS: &'static [ErrorKind] = &[ErrorKind::Invalid, ErrorKind::Conflict];
    type Params = VoiceRelayParams;
    type Result = Empty;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_relayed_id_names_the_host_and_is_seen_as_relayed() {
        let id = relayed_terminal("studio", "t1-abc123");
        assert_eq!(id, "studio/t1-abc123");
        assert!(is_relayed(&id));
        assert!(!is_relayed("t1-abc123"));
    }

    #[test]
    fn a_host_name_is_not_empty_and_holds_no_slash_or_control_character() {
        assert!(check_host("studio.lan").is_ok());
        for bad in ["", "a/b", "a\u{0}b", "a\nb"] {
            assert!(check_host(bad).is_err(), "{bad:?}");
        }
    }
}
