//! Pill window and voice.
//!
//! The pill is a client of the local core. It subscribes to the `voice` and
//! `dictation` events, draws one small overlay at the top center of the focused
//! output, and sends a stop to the core when the owner clicks it.
//! Spec: `docs/specs/agentsi-desktop-voice.md`.

pub mod audio;
pub mod config;
pub mod dictate;
pub mod link;
pub mod speak;
pub mod state;
#[cfg(target_os = "linux")]
pub mod view;

/// `AGENTSI_PILL_STATS` makes the pill print its frame count and window open time.
pub fn stats_enabled() -> bool {
    std::env::var_os("AGENTSI_PILL_STATS").is_some()
}

/// The Wayland display of this session, or the reason there is none.
///
/// GPUI falls back to X11 without one, and that makes an ordinary window that takes
/// focus. The old pill crashed Quickshell when it started with no display.
pub fn wayland_display(env: impl Fn(&str) -> Option<String>) -> Result<String, String> {
    let name = env("WAYLAND_DISPLAY")
        .filter(|v| !v.is_empty())
        .ok_or("WAYLAND_DISPLAY is not set")?;
    let path = if name.starts_with('/') {
        std::path::PathBuf::from(&name)
    } else {
        let runtime = env("XDG_RUNTIME_DIR")
            .filter(|v| !v.is_empty())
            .ok_or("XDG_RUNTIME_DIR is not set")?;
        std::path::Path::new(&runtime).join(&name)
    };
    if path.exists() {
        Ok(name)
    } else {
        Err(format!(
            "the Wayland socket {} does not exist",
            path.display()
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    fn env(map: HashMap<&'static str, String>) -> impl Fn(&str) -> Option<String> {
        move |k| map.get(k).cloned()
    }

    #[test]
    fn no_display_variable_is_an_error() {
        assert_eq!(
            wayland_display(env(HashMap::new())),
            Err("WAYLAND_DISPLAY is not set".into())
        );
    }

    #[test]
    fn a_missing_socket_is_an_error() {
        let dir = std::env::temp_dir().join(format!("agentsi-pill-test-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let map = HashMap::from([
            ("WAYLAND_DISPLAY", "wayland-9".to_string()),
            ("XDG_RUNTIME_DIR", dir.display().to_string()),
        ]);
        assert!(
            wayland_display(env(map))
                .unwrap_err()
                .contains("does not exist")
        );
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn an_existing_socket_path_passes() {
        let dir = std::env::temp_dir().join(format!("agentsi-pill-test-ok-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("wayland-1"), "").unwrap();
        let map = HashMap::from([
            ("WAYLAND_DISPLAY", "wayland-1".to_string()),
            ("XDG_RUNTIME_DIR", dir.display().to_string()),
        ]);
        assert_eq!(wayland_display(env(map)), Ok("wayland-1".into()));
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
