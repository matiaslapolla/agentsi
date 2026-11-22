//! The keep time of the voice models in `<config>/agentsi/pill.toml`, as `[models] keep`.
//!
//! The pill reads the file at its start, and the core sends it each change with
//! `voice.models`. The core edits only that key, so the other keys and the comments stay. Both
//! read the key with [`agentsi_voice::keep::keep_in`], so they agree on it.

use agentsi_proto::voice_api::KeepTime;
use std::io::Write as _;
use std::os::unix::fs::{OpenOptionsExt as _, PermissionsExt as _};
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

/// Sets `[models] keep` and keeps the rest of the file. A `pill.toml` that is a symlink stays
/// one: the write goes to its target, with the mode of the target.
pub fn write_keep(path: &Path, keep: KeepTime) -> Result<(), String> {
    let path = match std::fs::canonicalize(path) {
        Ok(target) => target,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => path.to_owned(),
        Err(e) => return Err(format!("{}: {e}", path.display())),
    };
    let path = path.as_path();
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
    let mode = std::fs::metadata(path)
        .ok()
        .map(|m| m.permissions().mode() & 0o7777);
    replace(path, &doc.to_string(), mode)
}

/// Writes `text` to a file next to `path`, then renames it over `path`, so a reader never sees
/// half a file. The new file gets `mode`. A failure leaves no temporary file.
fn replace(path: &Path, text: &str, mode: Option<u32>) -> Result<(), String> {
    let mut tmp = path.as_os_str().to_owned();
    tmp.push(".tmp");
    let tmp = PathBuf::from(tmp);
    let written = (|| {
        let mut options = std::fs::OpenOptions::new();
        options.write(true).create(true).truncate(true);
        if let Some(mode) = mode {
            options.mode(mode);
        }
        let mut file = options.open(&tmp)?;
        // The umask cuts the mode of a new file, and an old temporary file keeps its own.
        if let Some(mode) = mode {
            file.set_permissions(std::fs::Permissions::from_mode(mode))?;
        }
        file.write_all(text.as_bytes())
    })()
    .map_err(|e| format!("{}: {e}", tmp.display()))
    .and_then(|()| std::fs::rename(&tmp, path).map_err(|e| format!("{}: {e}", path.display())));
    if written.is_err() {
        let _ = std::fs::remove_file(&tmp);
    }
    written
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
    fn a_symlinked_file_stays_a_symlink_and_keeps_its_mode() {
        let dir = tempfile::tempdir().unwrap();
        let target = dir.path().join("dotfiles/pill.toml");
        std::fs::create_dir_all(target.parent().unwrap()).unwrap();
        std::fs::write(&target, "input = \"USB\"\n").unwrap();
        std::fs::set_permissions(&target, std::fs::Permissions::from_mode(0o600)).unwrap();
        let link = dir.path().join("agentsi/pill.toml");
        std::fs::create_dir_all(link.parent().unwrap()).unwrap();
        std::os::unix::fs::symlink(&target, &link).unwrap();
        write_keep(&link, KeepTime::OneHour).unwrap();
        assert!(std::fs::symlink_metadata(&link).unwrap().is_symlink());
        assert_eq!(read_keep(&link), KeepTime::OneHour);
        assert_eq!(
            std::fs::read_to_string(&target).unwrap(),
            "input = \"USB\"\n\n[models]\nkeep = \"1h\"\n"
        );
        let mode = std::fs::metadata(&target).unwrap().permissions().mode() & 0o7777;
        assert_eq!(mode, 0o600);
        assert!(!dir.path().join("dotfiles/pill.toml.tmp").exists());
        assert!(!dir.path().join("agentsi/pill.toml.tmp").exists());
    }

    #[test]
    fn a_failed_rename_leaves_no_temporary_file() {
        let dir = tempfile::tempdir().unwrap();
        // A folder that is not empty cannot be replaced by a file.
        let path = dir.path().join("pill.toml");
        std::fs::create_dir_all(path.join("inside")).unwrap();
        assert!(replace(&path, "[models]\nkeep = \"1h\"\n", Some(0o644)).is_err());
        assert!(!dir.path().join("pill.toml.tmp").exists());
        assert!(path.join("inside").is_dir());
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
