//! What the call screen reads and writes: the call, a part of its audio, and the
//! listened mark. The mark ports `recordListen` and `removeListenRecord` of the web app.

use super::codec;
use super::files::{audio_dir, find_audio, read_timings, safe_segment};
use super::{Error, Result};
use crate::clock;
use agentsi_proto::call_api::{CallAudioInfo, Segment};
use agentsi_store::{calls, read};
use rusqlite::Connection;
use serde_json::Value;
use std::path::Path;

#[derive(Debug, Clone, PartialEq)]
pub struct CallView {
    pub question: String,
    pub segments: Vec<Segment>,
    pub timed: bool,
    pub audio: Option<CallAudioInfo>,
    pub heard: bool,
}

fn check_names(product: &str, minutes_id: &str) -> Result<()> {
    if safe_segment(product) && safe_segment(minutes_id) {
        Ok(())
    } else {
        Err(Error::NotFound(format!(
            "No minutes {minutes_id} in product {product}."
        )))
    }
}

/// The stored minutes `minutes_id` of `product`.
fn stored(c: &Connection, product: &str, minutes_id: &str) -> Result<Value> {
    check_names(product, minutes_id)?;
    read::minutes(c, minutes_id)?
        .filter(|m| m.get("product").and_then(Value::as_str) == Some(product))
        .ok_or_else(|| Error::NotFound(format!("No minutes {minutes_id} in product {product}.")))
}

/// The call of stored minutes. Without `timings.json`, the segments are the script with times of 0.
pub fn get(c: &Connection, root: &Path, product: &str, minutes_id: &str) -> Result<CallView> {
    let minutes = stored(c, product, minutes_id)?;
    let dir = audio_dir(root, product, minutes_id);
    let timings = read_timings(&dir)?;
    let timed = !timings.is_empty();
    let segments = if timed {
        timings
    } else {
        super::script_of(&minutes)
            .map(|script| {
                script
                    .into_iter()
                    .enumerate()
                    .map(|(idx, s)| Segment {
                        idx: idx as u32,
                        role: s.role,
                        text: s.text,
                        start: 0.0,
                        end: 0.0,
                    })
                    .collect()
            })
            .unwrap_or_default()
    };
    let heard = read::listens(c, product)?
        .iter()
        .any(|l| l.minutes_id == minutes_id);
    Ok(CallView {
        question: minutes
            .get("question")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .into(),
        segments,
        timed,
        audio: find_audio(&dir).map(|a| CallAudioInfo {
            format: a.format.name().into(),
            bytes: a.bytes,
        }),
        heard,
    })
}

/// Up to `count` samples of the call from sample `from`, as 16-bit integers, and the
/// samples of the whole call. The whole file is decoded each time, which takes
/// about 10 ms for each 10 seconds of audio.
pub fn audio_chunk(
    c: &Connection,
    root: &Path,
    product: &str,
    minutes_id: &str,
    from: u64,
    count: u64,
) -> Result<(Vec<i16>, u64)> {
    stored(c, product, minutes_id)?;
    let file = find_audio(&audio_dir(root, product, minutes_id)).ok_or_else(|| {
        Error::NotFound(format!(
            "The call {minutes_id} has no audio. Render it first."
        ))
    })?;
    let samples = codec::decode(&file.path)?;
    let total = samples.len() as u64;
    let start = from.min(total) as usize;
    let end = from.saturating_add(count).min(total) as usize;
    let pcm = samples[start..end]
        .iter()
        .map(|s| (s.clamp(-1.0, 1.0) * 32767.0).round() as i16)
        .collect();
    Ok((pcm, total))
}

/// Marks the call as listened, or removes the mark. A call with no audio file cannot be
/// marked, so the answer is false, as in the web app. Returns true when a row changed.
pub fn set_heard(
    c: &Connection,
    root: &Path,
    product: &str,
    minutes_id: &str,
    heard: bool,
) -> Result<bool> {
    check_names(product, minutes_id)?;
    if !heard {
        return Ok(calls::remove_listen(c, product, minutes_id)?);
    }
    if find_audio(&audio_dir(root, product, minutes_id)).is_none() {
        return Ok(false);
    }
    Ok(calls::record_listen(
        c,
        product,
        minutes_id,
        &clock::iso(clock::now_ms()),
    )?)
}
