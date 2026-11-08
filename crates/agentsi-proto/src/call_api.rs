//! The methods of calls (U26): `call.get`, `call.audio`, `call.render`,
//! `call.cancel` and `call.heard`, and the event `meeting.render`. `agentsi render`
//! and `meet --audio` call `call.render`. The call screen calls the others.
//!
//! A call is the audio of a meeting: one voice for each role, and the times of
//! each segment. A new call is `call.opus`. An old call is `call.webm`, from the
//! Python package. The core decodes both, so a client never sees the format.

use crate::method::{Catalog, Event, Method};
use crate::rpc::ErrorKind;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

pub fn register(c: &mut Catalog) {
    c.method::<CallGet>()
        .method::<CallAudio>()
        .method::<CallRender>()
        .method::<CallCancel>()
        .method::<CallHeard>()
        .event::<MeetingRender>();
}

/// The samples per second of `call.audio`.
pub const SAMPLE_RATE: u32 = 24_000;

/// One segment of the script, with its place in the audio. The names are the keys of `timings.json`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct Segment {
    pub idx: u32,
    pub role: String,
    pub text: String,
    /// Seconds. Both are 0 for a segment of a call that has no timings.
    pub start: f64,
    pub end: f64,
}

pub struct CallGet;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct CallGetParams {
    pub product: String,
    pub minutes_id: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct CallAudioInfo {
    /// `opus` for a new call, `webm` for an old one.
    pub format: String,
    pub bytes: u64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct CallGetResult {
    pub product: String,
    pub minutes_id: String,
    pub question: String,
    /// The segments of `timings.json`. Without the file, the script with times of 0.
    pub segments: Vec<Segment>,
    /// True when `timings.json` gave the segments.
    pub timed: bool,
    /// Absent when no audio file exists.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub audio: Option<CallAudioInfo>,
    /// True when the owner marked the call as listened.
    pub heard: bool,
    /// The render of this call that runs in this core, if any.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub render: Option<RenderInfo>,
}

/// A running render, as the last event of it said.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct RenderInfo {
    /// The name to give `call.cancel`.
    pub render: String,
    /// Segments that are ready.
    pub done: u32,
    /// Segments of the script.
    pub of: u32,
}

impl Method for CallGet {
    const NAME: &'static str = "call.get";
    const DESCRIPTION: &'static str = "The call of stored minutes: the segments with their times, \
        whether audio exists, and the listened mark. Only reads.";
    const ERRORS: &'static [ErrorKind] = &[ErrorKind::NotFound, ErrorKind::Unavailable];
    type Params = CallGetParams;
    type Result = CallGetResult;
}

pub struct CallAudio;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct CallAudioParams {
    pub product: String,
    pub minutes_id: String,
    /// The first sample to return.
    pub from: u64,
    /// The most samples to return. The core may return less. A client asks for about a minute.
    pub count: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct CallAudioResult {
    pub from: u64,
    /// The samples of the whole call.
    pub total: u64,
    /// Samples as little-endian 16-bit integers at [`SAMPLE_RATE`], mono, in base64.
    pub pcm: String,
}

impl Method for CallAudio {
    const NAME: &'static str = "call.audio";
    const DESCRIPTION: &'static str = "A part of the audio of a call, decoded to 16-bit samples \
        at 24 kHz. It plays old and new files alike.";
    const ERRORS: &'static [ErrorKind] = &[ErrorKind::NotFound, ErrorKind::Unavailable];
    type Params = CallAudioParams;
    type Result = CallAudioResult;
}

pub struct CallRender;

#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize, JsonSchema)]
pub struct CallRenderParams {
    pub product: String,
    pub minutes_id: String,
    /// A name for the events of this render, which the client picks. Without it, the core makes one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub render: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct CallRenderResult {
    pub render: String,
    /// The folder of the call: `<data root>/workspace/<product>/audio/<minutes id>`.
    pub out: String,
    pub duration: f64,
    pub bytes: u64,
    pub segments: u32,
}

impl Method for CallRender {
    const NAME: &'static str = "call.render";
    const DESCRIPTION: &'static str = "Renders the script of stored minutes with Kokoro, one \
        voice for each role, and writes call.opus and timings.json. It takes minutes. The \
        reply comes at the end. A cancel returns `conflict`.";
    const ERRORS: &'static [ErrorKind] = &[
        ErrorKind::NotFound,
        ErrorKind::Invalid,
        ErrorKind::Conflict,
        ErrorKind::Unavailable,
    ];
    type Params = CallRenderParams;
    type Result = CallRenderResult;
}

pub struct CallCancel;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct CallCancelParams {
    pub render: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct CallCancelResult {
    /// False when no render of that name runs.
    pub canceled: bool,
}

impl Method for CallCancel {
    const NAME: &'static str = "call.cancel";
    const DESCRIPTION: &'static str =
        "Stops a render. It writes nothing, and the old files of the call stay.";
    const ERRORS: &'static [ErrorKind] = &[];
    type Params = CallCancelParams;
    type Result = CallCancelResult;
}

pub struct CallHeard;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct CallHeardParams {
    pub product: String,
    pub minutes_id: String,
    /// True marks the call as listened. False removes the mark.
    pub heard: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct CallHeardResult {
    /// True when a row was written or removed.
    pub changed: bool,
}

impl Method for CallHeard {
    const NAME: &'static str = "call.heard";
    const DESCRIPTION: &'static str = "Marks a call as listened, or removes the mark. A call \
        with no audio file cannot be marked.";
    const ERRORS: &'static [ErrorKind] = &[ErrorKind::NotFound, ErrorKind::Unavailable];
    type Params = CallHeardParams;
    type Result = CallHeardResult;
}

pub struct MeetingRender;

/// One step of a render. `stage` is one of these:
///
/// - `start`: the render began. `of` is the number of segments.
/// - `segment`: segment `done` of `of` is ready.
/// - `encode`: the file is being written.
/// - `done`: the last event. `note` holds the folder, or `error` the reason of the end.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize, JsonSchema)]
pub struct MeetingRenderParams {
    pub render: String,
    pub product: String,
    pub minutes_id: String,
    pub stage: String,
    #[serde(default)]
    pub done: u32,
    #[serde(default)]
    pub of: u32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub note: Option<String>,
}

impl Event for MeetingRender {
    const NAME: &'static str = "meeting.render";
    const DESCRIPTION: &'static str =
        "One step of the render of a call, for the log of the CLI and the call screen.";
    type Params = MeetingRenderParams;
}

const B64: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";

/// Samples as the `pcm` text of `call.audio`.
pub fn encode_pcm(samples: &[i16]) -> String {
    let bytes: Vec<u8> = samples.iter().flat_map(|s| s.to_le_bytes()).collect();
    let mut out = String::with_capacity(bytes.len().div_ceil(3) * 4);
    for chunk in bytes.chunks(3) {
        let n = chunk
            .iter()
            .enumerate()
            .fold(0u32, |n, (i, b)| n | u32::from(*b) << (16 - 8 * i));
        for i in 0..4 {
            if i <= chunk.len() {
                out.push(B64[(n >> (18 - 6 * i) & 63) as usize] as char);
            } else {
                out.push('=');
            }
        }
    }
    out
}

/// The samples of a `pcm` text. `None` for text that is not valid base64 of whole samples.
pub fn decode_pcm(text: &str) -> Option<Vec<i16>> {
    let mut bytes = Vec::with_capacity(text.len() / 4 * 3);
    for group in text.as_bytes().chunks(4) {
        if group.len() != 4 {
            return None;
        }
        let mut n = 0u32;
        let mut pad = 0;
        for c in group {
            n <<= 6;
            if *c == b'=' {
                pad += 1;
            } else {
                n |= B64.iter().position(|b| b == c)? as u32;
            }
        }
        bytes.extend_from_slice(&n.to_be_bytes()[1..4 - pad]);
    }
    if bytes.len() % 2 != 0 {
        return None;
    }
    Some(
        bytes
            .as_chunks::<2>()
            .0
            .iter()
            .map(|b| i16::from_le_bytes(*b))
            .collect(),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pcm_text_round_trips_for_each_length_of_padding() {
        for len in 0..9usize {
            let samples: Vec<i16> = (0..len as i16).map(|i| i * 4001 - 9000).collect();
            assert_eq!(decode_pcm(&encode_pcm(&samples)).unwrap(), samples, "{len}");
        }
    }

    #[test]
    fn pcm_text_is_standard_base64() {
        // "AQID" is the bytes 1, 2, 3 and "BA==" is 4: two samples, 0x0201 and 0x0403.
        assert_eq!(encode_pcm(&[0x0201, 0x0403]), "AQIDBA==");
        assert_eq!(decode_pcm("AQIDBA==").unwrap(), vec![0x0201, 0x0403]);
    }

    #[test]
    fn bad_pcm_text_is_refused() {
        assert_eq!(decode_pcm("AQI"), None);
        assert_eq!(decode_pcm("AQ!D"), None);
        assert_eq!(decode_pcm("AQID"), None, "an odd number of bytes");
    }
}
