//! Where the audio of a call lives: `<data root>/workspace/<product>/audio/<minutes id>/`.

use super::{Error, Result};
use agentsi_proto::call_api::Segment;
use std::path::{Path, PathBuf};

/// `safeSegment` of the web app: a name that cannot leave its folder.
pub fn safe_segment(s: &str) -> bool {
    let mut chars = s.chars();
    chars
        .next()
        .is_some_and(|c| c.is_ascii_alphanumeric() || c == '_')
        && chars.all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-'))
        && !s.contains("..")
}

pub fn audio_dir(root: &Path, product: &str, minutes_id: &str) -> PathBuf {
    root.join("workspace")
        .join(product)
        .join("audio")
        .join(minutes_id)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Format {
    /// `call.opus`, Ogg Opus, written by this program.
    Opus,
    /// `call.webm`, Opus in WebM, written by the Python package.
    Webm,
}

impl Format {
    pub fn name(self) -> &'static str {
        match self {
            Format::Opus => "opus",
            Format::Webm => "webm",
        }
    }
    pub fn file_name(self) -> &'static str {
        match self {
            Format::Opus => "call.opus",
            Format::Webm => "call.webm",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AudioFile {
    pub path: PathBuf,
    pub format: Format,
    pub bytes: u64,
}

/// The audio file of a call. A new file wins over an old one when both exist.
pub fn find_audio(dir: &Path) -> Option<AudioFile> {
    [Format::Opus, Format::Webm].into_iter().find_map(|format| {
        let path = dir.join(format.file_name());
        let meta = std::fs::metadata(&path).ok().filter(|m| m.is_file())?;
        Some(AudioFile {
            path,
            format,
            bytes: meta.len(),
        })
    })
}

/// The segments of `timings.json`. A missing file is no segments. A malformed one is an error.
pub fn read_timings(dir: &Path) -> Result<Vec<Segment>> {
    let file = dir.join("timings.json");
    let text = match std::fs::read_to_string(&file) {
        Ok(t) => t,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(e) => return Err(Error::Unavailable(format!("{}: {e}", file.display()))),
    };
    serde_json::from_str(&text)
        .map_err(|e| Error::Unavailable(format!("{} is not valid: {e}", file.display())))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_segment_name_cannot_leave_its_folder() {
        assert!(safe_segment("20261020-1000-a"));
        assert!(safe_segment("a.b_c-d"));
        assert!(!safe_segment(""));
        assert!(!safe_segment("../x"));
        assert!(!safe_segment("a/b"));
        assert!(!safe_segment(".hidden"));
        assert!(!safe_segment("a..b"));
    }

    #[test]
    fn a_new_file_wins_over_an_old_one() {
        let dir = tempfile::tempdir().unwrap();
        assert_eq!(find_audio(dir.path()), None);
        std::fs::write(dir.path().join("call.webm"), b"old").unwrap();
        assert_eq!(find_audio(dir.path()).unwrap().format, Format::Webm);
        std::fs::write(dir.path().join("call.opus"), b"new!").unwrap();
        let found = find_audio(dir.path()).unwrap();
        assert_eq!((found.format, found.bytes), (Format::Opus, 4));
    }

    #[test]
    fn timings_read_and_a_missing_file_is_empty() {
        let dir = tempfile::tempdir().unwrap();
        assert!(read_timings(dir.path()).unwrap().is_empty());
        std::fs::write(
            dir.path().join("timings.json"),
            r#"[{"idx":0,"role":"pm","text":"Hi.","start":0.0,"end":1.5}]"#,
        )
        .unwrap();
        let t = read_timings(dir.path()).unwrap();
        assert_eq!((t[0].role.as_str(), t[0].end), ("pm", 1.5));
        std::fs::write(dir.path().join("timings.json"), "{").unwrap();
        assert!(read_timings(dir.path()).is_err());
    }
}
