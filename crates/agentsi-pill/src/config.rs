//! The settings of the pill. Order of precedence: flags, then environment, then `pill.toml`.

use crate::keep::KeepTime;
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
    /// Only the file sets it.
    pub models: Option<Models>,
}

/// The `[models]` table.
#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Models {
    /// The keep time: `"1m"`, `"5m"`, `"15m"`, `"1h"`, `"always"` or `"start"`.
    pub keep: Option<String>,
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
            models: over.models.or(self.models),
        }
    }

    pub fn from_env() -> Config {
        let get = |name| std::env::var(name).ok().filter(|v| !v.is_empty());
        Config {
            input: get(INPUT_ENV),
            output: get(OUTPUT_ENV),
            models: None,
        }
    }

    /// The keep time, and the line that says why a bad value gives the default.
    pub fn keep(&self) -> (KeepTime, Option<String>) {
        let Some(value) = self.models.as_ref().and_then(|m| m.keep.as_deref()) else {
            return (KeepTime::default(), None);
        };
        match KeepTime::from_key(value) {
            Some(keep) => (keep, None),
            None => {
                let valid: Vec<&str> = KeepTime::ALL.iter().map(|k| k.key()).collect();
                (
                    KeepTime::default(),
                    Some(format!(
                        "pill.toml: [models] keep = \"{value}\" is not one of {}. The pill uses \"{}\".",
                        valid.join(", "),
                        KeepTime::default().key()
                    )),
                )
            }
        }
    }
}

/// The keep time of `pill.toml` now, for a reconnect to the core. A file that does not read
/// gives the default, with no line: the start of the pill printed it.
pub fn keep_now() -> KeepTime {
    Config::load().map_or(KeepTime::default(), |c| c.keep().0)
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
                output: Some("alsa:x".into()),
                models: None,
            }
        );
    }

    #[test]
    fn the_models_table_sets_the_keep_time() {
        let config = Config::parse("input = \"USB\"\n\n[models]\nkeep = \"1h\"\n").unwrap();
        assert_eq!(config.keep(), (KeepTime::OneHour, None));
        assert_eq!(
            Config::parse("[models]\n").unwrap().keep(),
            (KeepTime::FiveMinutes, None)
        );
        assert_eq!(Config::default().keep(), (KeepTime::FiveMinutes, None));
        assert!(Config::parse("[models]\nkeeps = \"1h\"\n").is_err());
    }

    #[test]
    fn a_bad_keep_time_gives_one_line_and_the_default() {
        let config = Config::parse("[models]\nkeep = \"2m\"\n").unwrap();
        let (keep, line) = config.keep();
        assert_eq!(keep, KeepTime::FiveMinutes);
        let line = line.unwrap();
        assert!(line.contains("\"2m\""), "{line}");
        assert!(!line.contains('\n'));
        assert!(line.ends_with("The pill uses \"5m\"."), "{line}");
    }

    #[test]
    fn an_unknown_key_is_an_error() {
        assert!(Config::parse("inputs = \"USB\"").is_err());
    }

    #[test]
    fn a_later_source_wins_only_where_it_is_set() {
        let models = Some(Models {
            keep: Some("1m".into()),
        });
        let file = Config {
            input: Some("a".into()),
            output: Some("b".into()),
            models: models.clone(),
        };
        let flags = Config {
            input: Some("z".into()),
            output: None,
            models: None,
        };
        assert_eq!(
            file.over(flags),
            Config {
                input: Some("z".into()),
                output: Some("b".into()),
                models,
            }
        );
    }
}
