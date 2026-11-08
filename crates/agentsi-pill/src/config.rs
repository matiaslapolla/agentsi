//! The settings of the pill. Order of precedence: flags, then environment, then `pill.toml`.

use serde::Deserialize;
use std::path::PathBuf;

pub const INPUT_ENV: &str = "AGENTSI_PILL_INPUT";
pub const OUTPUT_ENV: &str = "AGENTSI_PILL_OUTPUT";

#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Config {
    /// An id, a name or a part of a name. Empty means the default device.
    pub input: Option<String>,
    pub output: Option<String>,
}

impl Config {
    /// `<config>/agentsi/pill.toml`, where `<config>` is `$XDG_CONFIG_HOME` or `~/.config`.
    pub fn path() -> Option<PathBuf> {
        let base = std::env::var_os("XDG_CONFIG_HOME")
            .filter(|v| !v.is_empty())
            .map(PathBuf::from)
            .or_else(|| {
                std::env::var_os("HOME")
                    .filter(|v| !v.is_empty())
                    .map(|h| PathBuf::from(h).join(".config"))
            })?;
        Some(base.join("agentsi").join("pill.toml"))
    }

    pub fn parse(text: &str) -> Result<Config, String> {
        toml::from_str(text).map_err(|e| e.to_string())
    }

    /// Reads the file. A missing file gives the default.
    pub fn load() -> Result<Config, String> {
        let Some(path) = Config::path() else {
            return Ok(Config::default());
        };
        match std::fs::read_to_string(&path) {
            Ok(text) => Config::parse(&text).map_err(|e| format!("{}: {e}", path.display())),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(Config::default()),
            Err(e) => Err(format!("{}: {e}", path.display())),
        }
    }

    /// Takes each field that `over` sets.
    pub fn over(self, over: Config) -> Config {
        Config {
            input: over.input.or(self.input),
            output: over.output.or(self.output),
        }
    }

    pub fn from_env() -> Config {
        let get = |name| std::env::var(name).ok().filter(|v| !v.is_empty());
        Config {
            input: get(INPUT_ENV),
            output: get(OUTPUT_ENV),
        }
    }
}

/// Whether `<config>/agentsi/keys.toml` sets `mode = "toggle"` in `[dictation]`. The app writes
/// that key with the bind file, and the bind of the toggle form runs `dictate toggle`. Any
/// other value, a missing file or a file that does not parse is hold dictation.
pub fn dictation_toggle() -> bool {
    Config::path()
        .map(|p| p.with_file_name("keys.toml"))
        .and_then(|p| std::fs::read_to_string(p).ok())
        .is_some_and(|text| toggle_in(&text))
}

fn toggle_in(keys: &str) -> bool {
    toml::from_str::<toml::Table>(keys)
        .ok()
        .and_then(|t| {
            t.get("dictation")?
                .get("mode")?
                .as_str()
                .map(|m| m == "toggle")
        })
        .unwrap_or(false)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_key_file_names_toggle_dictation() {
        assert!(toggle_in(
            "[dictation]\nkey = \"SUPER + F10\"\nmode = \"toggle\"\n"
        ));
        assert!(!toggle_in("[dictation]\nmode = \"hold\"\n"));
        assert!(!toggle_in("[dictation]\nkey = \"F10\"\n"));
        assert!(!toggle_in("[app]\nmode = \"toggle\"\n"));
        assert!(!toggle_in("[dictation\nmode = toggle"));
        assert!(!toggle_in(""));
    }

    #[test]
    fn a_file_sets_the_devices() {
        let config = Config::parse("input = \"USB\"\noutput = \"alsa:x\"\n").unwrap();
        assert_eq!(
            config,
            Config {
                input: Some("USB".into()),
                output: Some("alsa:x".into())
            }
        );
    }

    #[test]
    fn an_unknown_key_is_an_error() {
        assert!(Config::parse("inputs = \"USB\"").is_err());
    }

    #[test]
    fn a_later_source_wins_only_where_it_is_set() {
        let file = Config {
            input: Some("a".into()),
            output: Some("b".into()),
        };
        let flags = Config {
            input: Some("z".into()),
            output: None,
        };
        assert_eq!(
            file.over(flags),
            Config {
                input: Some("z".into()),
                output: Some("b".into())
            }
        );
    }
}
