//! `render`: the call of a meeting. It ports `render.py`. Each segment of the script
//! is spoken with the voice of its role, a pause of 0.35 s separates two segments,
//! and `timings.json` keeps the second at which each segment starts and ends.

use super::codec::{self, RATE};
use super::{Error, Result};
use crate::meeting::Cancel;
use agentsi_proto::call_api::Segment;
use serde_json::Value;
use std::path::{Path, PathBuf};

/// The pause between two segments, as in `render.py`.
const SEGMENT_GAP: f32 = 0.35;

/// The speech engine. The core gives it Kokoro. A test gives it a tone.
pub trait Synth {
    /// The samples of one segment at [`RATE`]: the sentences with their pauses, at one level.
    fn segment(&mut self, role: &str, text: &str) -> std::result::Result<Vec<f32>, String>;
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ScriptItem {
    pub role: String,
    pub text: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Stage {
    /// `of` segments will follow.
    Start,
    /// Segment `done` of `of` is ready.
    Segment,
    /// The file is being written.
    Encode,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Progress {
    pub stage: Stage,
    pub done: usize,
    pub of: usize,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Rendered {
    pub out: PathBuf,
    pub duration: f64,
    pub bytes: u64,
    pub segments: usize,
}

/// The script of stored minutes. Empty or malformed is `Invalid`.
pub fn script_of(minutes: &Value) -> Result<Vec<ScriptItem>> {
    let items = minutes
        .get("script")
        .and_then(Value::as_array)
        .map_or(&[][..], Vec::as_slice);
    if items.is_empty() {
        return Err(Error::Invalid("Minutes have an empty script".into()));
    }
    items
        .iter()
        .enumerate()
        .map(|(i, item)| {
            let field = |k: &str| item.get(k).and_then(Value::as_str).unwrap_or_default();
            let (role, text) = (field("role"), field("text"));
            if role.is_empty() || text.trim().is_empty() {
                return Err(Error::Invalid(format!(
                    "segment {i} needs non-empty role and text"
                )));
            }
            Ok(ScriptItem {
                role: role.into(),
                text: text.into(),
            })
        })
        .collect()
}

fn seconds(samples: usize) -> f64 {
    (samples as f64 / f64::from(RATE) * 1000.0).round() / 1000.0
}

/// Speaks `script` and writes `call.opus` and `timings.json` into `out_dir`. A cancel
/// stops before the next segment and writes nothing. Each file is written whole or not at all.
pub fn render(
    script: &[ScriptItem],
    out_dir: &Path,
    synth: &mut dyn Synth,
    progress: &dyn Fn(Progress),
    cancel: &Cancel,
) -> Result<Rendered> {
    let io = |what: &str, e: std::io::Error| Error::Failed(format!("{what}: {e}"));
    let of = script.len();
    progress(Progress {
        stage: Stage::Start,
        done: 0,
        of,
    });
    let gap = vec![0.0f32; (SEGMENT_GAP * RATE as f32) as usize];
    let mut full: Vec<f32> = Vec::new();
    let mut timings = Vec::with_capacity(of);
    for (idx, item) in script.iter().enumerate() {
        if cancel.is_set() {
            return Err(Error::Canceled);
        }
        if idx > 0 {
            full.extend_from_slice(&gap);
        }
        let audio = synth
            .segment(&item.role, &item.text)
            .map_err(|e| Error::Failed(format!("segment {idx} ({}): {e}", item.role)))?;
        timings.push(Segment {
            idx: idx as u32,
            role: item.role.clone(),
            text: item.text.clone(),
            start: seconds(full.len()),
            end: seconds(full.len() + audio.len()),
        });
        full.extend_from_slice(&audio);
        progress(Progress {
            stage: Stage::Segment,
            done: idx + 1,
            of,
        });
    }
    if cancel.is_set() {
        return Err(Error::Canceled);
    }
    progress(Progress {
        stage: Stage::Encode,
        done: of,
        of,
    });
    let encoded = codec::encode_ogg_opus(&full)?;
    // The last point where a cancel writes nothing, and the folder does not exist yet.
    if cancel.is_set() {
        return Err(Error::Canceled);
    }
    std::fs::create_dir_all(out_dir).map_err(|e| io(&out_dir.display().to_string(), e))?;
    let mut json =
        serde_json::to_string_pretty(&timings).map_err(|e| Error::Failed(e.to_string()))?;
    json.push('\n');
    publish(
        out_dir,
        &[
            (super::Format::Opus.file_name(), &encoded),
            ("timings.json", json.as_bytes()),
        ],
    )
    .map_err(|e| io(&out_dir.display().to_string(), e))?;
    Ok(Rendered {
        out: out_dir.to_path_buf(),
        duration: seconds(full.len()),
        bytes: encoded.len() as u64,
        segments: of,
    })
}

/// Writes each file beside its final name, then renames them one after the other, so a
/// failed write leaves the old call whole and only a failed rename can mix two calls.
fn publish(dir: &Path, files: &[(&str, &[u8])]) -> std::io::Result<()> {
    let part = |name: &str| dir.join(format!("{name}.part"));
    let written = files
        .iter()
        .try_for_each(|(name, bytes)| std::fs::write(part(name), bytes));
    if let Err(e) = written {
        files
            .iter()
            .for_each(|(name, _)| drop(std::fs::remove_file(part(name))));
        return Err(e);
    }
    for (name, _) in files {
        std::fs::rename(part(name), dir.join(name))?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::call::{codec, find_audio, read_timings};
    use std::cell::RefCell;

    /// One tone for each role, as long as 0.1 s for each 10 characters.
    struct Tones;

    impl Synth for Tones {
        fn segment(&mut self, role: &str, text: &str) -> std::result::Result<Vec<f32>, String> {
            if text.contains("BROKEN") {
                return Err("the voice failed".into());
            }
            let pitch = if role == "pm" { 220.0 } else { 330.0 };
            let n = (text.len() as f32 / 10.0 * 0.1 * RATE as f32) as usize;
            Ok((0..n)
                .map(|i| 0.2 * (i as f32 * 2.0 * std::f32::consts::PI * pitch / RATE as f32).sin())
                .collect())
        }
    }

    fn script() -> Vec<ScriptItem> {
        vec![
            ScriptItem {
                role: "pm".into(),
                text: "Welcome to the test.".into(),
            },
            ScriptItem {
                role: "dev".into(),
                text: "The build is green and done.".into(),
            },
        ]
    }

    #[test]
    fn a_render_writes_the_file_and_the_timings_of_the_python_package() {
        let dir = tempfile::tempdir().unwrap();
        let steps = RefCell::new(Vec::new());
        let out = render(
            &script(),
            dir.path(),
            &mut Tones,
            &|p| steps.borrow_mut().push((p.stage, p.done, p.of)),
            &Cancel::default(),
        )
        .unwrap();
        assert_eq!(out.segments, 2);
        assert_eq!(
            *steps.borrow(),
            [
                (Stage::Start, 0, 2),
                (Stage::Segment, 1, 2),
                (Stage::Segment, 2, 2),
                (Stage::Encode, 2, 2)
            ]
        );
        let t = read_timings(dir.path()).unwrap();
        assert_eq!(t.len(), 2);
        assert_eq!(
            (t[0].idx, t[0].role.as_str(), t[0].start, t[0].end),
            (0, "pm", 0.0, 0.2)
        );
        // The second segment starts after the first and the pause of 0.35 s.
        assert_eq!((t[1].start, t[1].end), (0.55, 0.83));
        let file = find_audio(dir.path()).unwrap();
        assert_eq!(file.bytes, out.bytes);
        let samples = codec::decode(&file.path).unwrap();
        assert_eq!(
            samples.len(),
            (out.duration * f64::from(RATE)).round() as usize
        );
        let text = std::fs::read_to_string(dir.path().join("timings.json")).unwrap();
        assert!(text.starts_with("[\n  {\n    \"idx\": 0,\n    \"role\": \"pm\","));
        assert!(text.ends_with("}\n]\n"));
        assert!(!dir.path().join("call.part").exists());
    }

    #[test]
    fn a_cancel_writes_nothing() {
        let dir = tempfile::tempdir().unwrap();
        let cancel = Cancel::default();
        let c2 = cancel.clone();
        let err = render(
            &script(),
            dir.path(),
            &mut Tones,
            &move |p| {
                if p.stage == Stage::Segment {
                    c2.cancel();
                }
            },
            &cancel,
        )
        .unwrap_err();
        assert!(matches!(err, Error::Canceled));
        assert_eq!(std::fs::read_dir(dir.path()).unwrap().count(), 0);
    }

    #[test]
    fn a_cancel_while_the_file_is_encoded_writes_nothing_and_makes_no_folder() {
        let dir = tempfile::tempdir().unwrap();
        let out = dir.path().join("audio").join("m1");
        let cancel = Cancel::default();
        let c2 = cancel.clone();
        let err = render(
            &script(),
            &out,
            &mut Tones,
            &move |p| {
                if p.stage == Stage::Encode {
                    c2.cancel();
                }
            },
            &cancel,
        )
        .unwrap_err();
        assert!(matches!(err, Error::Canceled));
        assert!(!out.exists());
    }

    #[test]
    fn a_failed_write_keeps_the_old_call_whole() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("call.opus"), b"old audio").unwrap();
        std::fs::write(dir.path().join("timings.json"), b"old times").unwrap();
        // A folder in the place of the temporary file makes the second write fail.
        std::fs::create_dir(dir.path().join("timings.json.part")).unwrap();
        let err = render(
            &script(),
            dir.path(),
            &mut Tones,
            &|_| {},
            &Cancel::default(),
        )
        .unwrap_err();
        assert!(matches!(err, Error::Failed(_)));
        assert_eq!(
            std::fs::read(dir.path().join("call.opus")).unwrap(),
            b"old audio"
        );
        assert_eq!(
            std::fs::read(dir.path().join("timings.json")).unwrap(),
            b"old times"
        );
        assert!(!dir.path().join("call.opus.part").exists());
    }

    #[test]
    fn a_failing_voice_names_the_segment_and_writes_nothing() {
        let dir = tempfile::tempdir().unwrap();
        let mut s = script();
        s[1].text = "BROKEN".into();
        let err = render(&s, dir.path(), &mut Tones, &|_| {}, &Cancel::default()).unwrap_err();
        assert_eq!(err.to_string(), "segment 1 (dev): the voice failed");
        assert_eq!(std::fs::read_dir(dir.path()).unwrap().count(), 0);
    }

    #[test]
    fn the_script_of_minutes_is_checked_as_the_python_cli_checks_it() {
        let ok = serde_json::json!({"script": [{"role": "pm", "text": "Hi."}]});
        assert_eq!(script_of(&ok).unwrap().len(), 1);
        for (minutes, message) in [
            (serde_json::json!({}), "Minutes have an empty script"),
            (
                serde_json::json!({"script": []}),
                "Minutes have an empty script",
            ),
            (
                serde_json::json!({"script": [{"role": "pm", "text": "Hi."}, {"role": "", "text": "x"}]}),
                "segment 1 needs non-empty role and text",
            ),
            (
                serde_json::json!({"script": [{"role": "pm", "text": "  "}]}),
                "segment 0 needs non-empty role and text",
            ),
        ] {
            assert_eq!(script_of(&minutes).unwrap_err().to_string(), message);
        }
    }
}
