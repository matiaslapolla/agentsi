//! The keep time of the voice models in `<config>/agentsi/pill.toml`, as `[models] keep`.
//!
//! The pill reads the file at its start, and the core sends it each change with
//! `voice.models`. The core edits only that key, so the other keys and the comments stay. Both
//! read the key with [`agentsi_voice::keep::keep_in`], so they agree on it.

use agentsi_proto::voice_api::KeepTime;
use std::path::{Path, PathBuf};
use toml_edit::{DocumentMut, Item, Table, value};

/// `<config>/agentsi/pill.toml`.
pub fn path() -> Option<PathBuf> {
    agentsi_voice::vocabulary::path().map(|p| p.with_file_name("pill.toml"))
}

/// The keep time of the file. A missing file, a file that does not parse, or a bad value
/// gives the default, as the pill does.
pub fn read_keep(path: &Path) -> KeepTime {
    std::fs::read_to_string(path).map_or(KeepTime::default(), |text| {
        agentsi_voice::keep::keep_in(&text).0
    })
}

/// Sets `[models] keep` and keeps the rest of the file.
pub fn write_keep(path: &Path, keep: KeepTime) -> Result<(), String> {
    let text = match std::fs::read_to_string(path) {
        Ok(text) => text,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => String::new(),
        Err(e) => return Err(format!("{}: {e}", path.display())),
    };
    let mut doc: DocumentMut = text.parse().map_err(|e: toml_edit::TomlError| {
        format!(
            "{} does not parse: {}. Fix it, or remove it.",
            path.display(),
            e.message()
        )
    })?;
    let models = doc.entry("models").or_insert(Item::Table(Table::new()));
    let Some(models) = models.as_table_like_mut() else {
        return Err(format!(
            "{}: `models` is not a table. Fix it, or remove it.",
            path.display()
        ));
    };
    models.insert("keep", value(keep.key()));
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir).map_err(|e| format!("{}: {e}", dir.display()))?;
    }
    let mut tmp = path.as_os_str().to_owned();
    tmp.push(".tmp");
    let tmp = PathBuf::from(tmp);
    std::fs::write(&tmp, doc.to_string()).map_err(|e| format!("{}: {e}", tmp.display()))?;
    std::fs::rename(&tmp, path).map_err(|e| format!("{}: {e}", path.display()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_write_keeps_the_other_keys_and_the_comments() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("agentsi/pill.toml");
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        let before = "# My microphone.\ninput = \"USB\"\noutput = \"alsa:x\" # the desk speakers\n";
        std::fs::write(&path, before).unwrap();
        write_keep(&path, KeepTime::OneHour).unwrap();
        let after = std::fs::read_to_string(&path).unwrap();
        assert!(after.starts_with(before), "{after}");
        assert!(after.contains("[models]\nkeep = \"1h\"\n"), "{after}");
        assert_eq!(read_keep(&path), KeepTime::OneHour);
        write_keep(&path, KeepTime::Start).unwrap();
        let again = std::fs::read_to_string(&path).unwrap();
        assert!(again.starts_with(before));
        assert_eq!(again.matches("[models]").count(), 1);
        assert_eq!(read_keep(&path), KeepTime::Start);
        assert!(!dir.path().join("agentsi/pill.toml.tmp").exists());
    }

    #[test]
    fn a_missing_file_or_folder_is_made() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("agentsi/pill.toml");
        assert_eq!(read_keep(&path), KeepTime::FiveMinutes);
        write_keep(&path, KeepTime::OneMinute).unwrap();
        assert_eq!(
            std::fs::read_to_string(&path).unwrap(),
            "[models]\nkeep = \"1m\"\n"
        );
    }

    #[test]
    fn a_bad_value_reads_as_the_default_and_a_broken_file_is_not_written() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("pill.toml");
        std::fs::write(&path, "[models]\nkeep = \"2m\"\n").unwrap();
        assert_eq!(read_keep(&path), KeepTime::FiveMinutes);
        std::fs::write(&path, "input = \n").unwrap();
        assert_eq!(read_keep(&path), KeepTime::FiveMinutes);
        assert!(write_keep(&path, KeepTime::OneHour).is_err());
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "input = \n");
    }
}
