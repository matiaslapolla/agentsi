//! One way to write a file of the app: through a temporary file, so a crash leaves the old
//! file whole.

use std::path::Path;

/// Writes `text` to `path` and makes the folder first. The temporary file is next to `path`.
pub fn write(path: &Path, text: &str) -> std::io::Result<()> {
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    let mut tmp = path.as_os_str().to_owned();
    tmp.push(".tmp");
    let tmp = std::path::PathBuf::from(tmp);
    std::fs::write(&tmp, text)?;
    std::fs::rename(tmp, path)
}

#[cfg(test)]
mod tests {
    #[test]
    fn a_write_makes_the_folder_and_leaves_no_temporary_file() {
        let dir = std::env::temp_dir().join(format!("agentsi-atomic-{}", std::process::id()));
        let path = dir.join("deep").join("a.toml");
        super::write(&path, "one").unwrap();
        super::write(&path, "two").unwrap();
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "two");
        assert!(!dir.join("deep").join("a.toml.tmp").exists());
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
