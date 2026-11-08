//! The audio files of calls. A new call is Ogg Opus (`call.opus`). An old call is
//! Opus in WebM (`call.webm`), which the Python package wrote with `ffmpeg`. Both
//! decode with libopus to mono samples at 24 kHz, the rate of Kokoro.
//!
//! The test in `docs/specs/agentsi-desktop-calls.md` chose this format by size and
//! by match with the old decode.

use matroska_demuxer::{Frame, MatroskaFile};
use ogg::writing::PacketWriteEndInfo;
use ogg::{PacketReader, PacketWriter};
use opusic_sys as opus;
use std::fs::File;
use std::io::{Cursor, Read};
use std::path::Path;

/// The rate of Kokoro and of the samples that this module reads and writes.
pub const RATE: u32 = agentsi_proto::call_api::SAMPLE_RATE;
/// Bits per second of the encoder, as `render.py` sets it.
const BITRATE: i32 = 32_000;
/// 20 ms.
const FRAME: usize = 480;
/// The longest packet that an Opus stream can hold is 120 ms.
const MAX_FRAME: usize = 2_880;
/// Ogg Opus counts granule positions and the pre-skip at 48 kHz, whatever the input rate.
const GRANULE_PER_SAMPLE: u64 = 48_000 / RATE as u64;
const STREAM_SERIAL: u32 = 0x4147_534f;

#[derive(Debug)]
pub struct Error(pub String);

impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for Error {}

type Result<T> = std::result::Result<T, Error>;

fn fail<T>(what: impl Into<String>) -> Result<T> {
    Err(Error(what.into()))
}

struct Encoder(*mut opus::OpusEncoder);

impl Encoder {
    fn new() -> Result<(Encoder, u16)> {
        let mut err = 0;
        // SAFETY: the arguments are valid, and `err` outlives the call.
        let raw = unsafe {
            opus::opus_encoder_create(RATE as i32, 1, opus::OPUS_APPLICATION_VOIP, &mut err)
        };
        if err != 0 || raw.is_null() {
            return fail(format!("libopus cannot make an encoder (code {err})"));
        }
        let enc = Encoder(raw);
        let mut lookahead = 0i32;
        // SAFETY: `raw` is a live encoder, and each request takes the one argument that is given.
        unsafe {
            opus::opus_encoder_ctl(raw, opus::OPUS_SET_BITRATE_REQUEST, BITRATE);
            opus::opus_encoder_ctl(
                raw,
                opus::OPUS_GET_LOOKAHEAD_REQUEST,
                &mut lookahead as *mut i32,
            );
        }
        let pre_skip = u16::try_from(lookahead.max(0) as u64 * GRANULE_PER_SAMPLE).unwrap_or(0);
        Ok((enc, pre_skip))
    }

    fn encode(&mut self, frame: &[i16], out: &mut [u8]) -> Result<usize> {
        // SAFETY: `frame` holds `FRAME` samples, and `out` is the capacity that is passed.
        let n = unsafe {
            opus::opus_encode(
                self.0,
                frame.as_ptr(),
                FRAME as i32,
                out.as_mut_ptr(),
                out.len() as i32,
            )
        };
        if n < 0 {
            return fail(format!("libopus cannot encode (code {n})"));
        }
        Ok(n as usize)
    }
}

impl Drop for Encoder {
    fn drop(&mut self) {
        // SAFETY: the encoder is live, and nothing uses it after this.
        unsafe { opus::opus_encoder_destroy(self.0) }
    }
}

struct Decoder(*mut opus::OpusDecoder);

impl Decoder {
    fn new() -> Result<Decoder> {
        let mut err = 0;
        // SAFETY: the arguments are valid, and `err` outlives the call.
        let raw = unsafe { opus::opus_decoder_create(RATE as i32, 1, &mut err) };
        if err != 0 || raw.is_null() {
            return fail(format!("libopus cannot make a decoder (code {err})"));
        }
        Ok(Decoder(raw))
    }

    fn decode(&mut self, packet: &[u8], out: &mut Vec<f32>) -> Result<()> {
        let start = out.len();
        out.resize(start + MAX_FRAME, 0.0);
        // SAFETY: `out[start..]` has room for `MAX_FRAME` samples of one channel.
        let n = unsafe {
            opus::opus_decode_float(
                self.0,
                packet.as_ptr(),
                packet.len() as i32,
                out[start..].as_mut_ptr(),
                MAX_FRAME as i32,
                0,
            )
        };
        if n < 0 {
            out.truncate(start);
            return fail(format!("libopus cannot decode a packet (code {n})"));
        }
        out.truncate(start + n as usize);
        Ok(())
    }
}

impl Drop for Decoder {
    fn drop(&mut self) {
        // SAFETY: the decoder is live, and nothing uses it after this.
        unsafe { opus::opus_decoder_destroy(self.0) }
    }
}

// SAFETY: a decoder or an encoder is one state with no thread affinity, and `&mut self` guards each use.
unsafe impl Send for Encoder {}
unsafe impl Send for Decoder {}

fn opus_head(pre_skip: u16) -> Vec<u8> {
    let mut h = b"OpusHead".to_vec();
    h.push(1);
    h.push(1);
    h.extend(pre_skip.to_le_bytes());
    h.extend(RATE.to_le_bytes());
    h.extend(0i16.to_le_bytes());
    h.push(0);
    h
}

fn opus_tags() -> Vec<u8> {
    let vendor = b"agentsi";
    let mut t = b"OpusTags".to_vec();
    t.extend((vendor.len() as u32).to_le_bytes());
    t.extend(vendor);
    t.extend(0u32.to_le_bytes());
    t
}

/// The samples as Ogg Opus: mono, 32 kbit/s, tuned for voice, as `render.py` encodes.
pub fn encode_ogg_opus(samples: &[f32]) -> Result<Vec<u8>> {
    let (mut enc, pre_skip) = Encoder::new()?;
    let mut out = Vec::new();
    let mut w = PacketWriter::new(&mut out);
    let io = |e: std::io::Error| Error(e.to_string());
    w.write_packet(
        opus_head(pre_skip),
        STREAM_SERIAL,
        PacketWriteEndInfo::EndPage,
        0,
    )
    .map_err(io)?;
    w.write_packet(opus_tags(), STREAM_SERIAL, PacketWriteEndInfo::EndPage, 0)
        .map_err(io)?;
    // The encoder holds back `lookahead` samples, so frames of silence at the end flush them.
    let lookahead = (u64::from(pre_skip) / GRANULE_PER_SAMPLE) as usize;
    let frames = (samples.len() + lookahead).div_ceil(FRAME).max(1);
    let mut pcm = vec![0i16; FRAME];
    let mut packet = vec![0u8; 1_500];
    for i in 0..frames {
        pcm.iter_mut().for_each(|s| *s = 0);
        let chunk = &samples[(i * FRAME).min(samples.len())..((i + 1) * FRAME).min(samples.len())];
        for (d, s) in pcm.iter_mut().zip(chunk) {
            *d = (s.clamp(-1.0, 1.0) * 32767.0) as i16;
        }
        let n = enc.encode(&pcm, &mut packet)?;
        let last = i + 1 == frames;
        // The granule of the last page is the end of the real audio: the padding of the last
        // frame is cut. No page may end after it, so the granules never decrease.
        let final_granule = (samples.len() as u64) * GRANULE_PER_SAMPLE + u64::from(pre_skip);
        let so_far = (((i + 1) * FRAME) as u64 * GRANULE_PER_SAMPLE + u64::from(pre_skip))
            .min(final_granule);
        let (end, granule) = if last {
            (PacketWriteEndInfo::EndStream, final_granule)
        } else if i % 50 == 49 {
            (PacketWriteEndInfo::EndPage, so_far)
        } else {
            (PacketWriteEndInfo::NormalPacket, so_far)
        };
        w.write_packet(packet[..n].to_vec(), STREAM_SERIAL, end, granule)
            .map_err(io)?;
    }
    Ok(out)
}

fn head_pre_skip(head: &[u8]) -> Result<u64> {
    if head.len() < 19 || &head[..8] != b"OpusHead" {
        return fail("the file has no OpusHead");
    }
    Ok(u64::from(u16::from_le_bytes([head[10], head[11]])))
}

/// Cuts `pre_skip` (at 48 kHz) from the start and, when known, the padding past `total`
/// samples (at the rate of `samples`).
fn trim(mut samples: Vec<f32>, pre_skip: u64, total: Option<usize>) -> Vec<f32> {
    let skip = ((pre_skip / GRANULE_PER_SAMPLE) as usize).min(samples.len());
    samples.drain(..skip);
    if let Some(total) = total {
        samples.truncate(total);
    }
    samples
}

fn decode_ogg_opus(bytes: &[u8]) -> Result<Vec<f32>> {
    let mut reader = PacketReader::new(Cursor::new(bytes));
    let io = |e: ogg::OggReadError| Error(e.to_string());
    let head = reader.read_packet_expected().map_err(io)?;
    let pre_skip = head_pre_skip(&head.data)?;
    reader.read_packet_expected().map_err(io)?;
    let mut dec = Decoder::new()?;
    let mut samples = Vec::new();
    let mut last_granule = 0u64;
    while let Some(p) = reader.read_packet().map_err(io)? {
        dec.decode(&p.data, &mut samples)?;
        last_granule = p.absgp_page();
    }
    let total = (last_granule > 0 && last_granule >= pre_skip)
        .then(|| ((last_granule - pre_skip) / GRANULE_PER_SAMPLE) as usize);
    Ok(trim(samples, pre_skip, total))
}

fn decode_webm(file: File) -> Result<Vec<f32>> {
    let mut mkv = MatroskaFile::open(file).map_err(|e| Error(e.to_string()))?;
    let Some(track) = mkv
        .tracks()
        .iter()
        .find(|t| t.codec_id() == "A_OPUS")
        .cloned()
    else {
        return fail("the WebM file has no Opus track");
    };
    let pre_skip = track.codec_private().map_or(Ok(0), head_pre_skip)?;
    let mut dec = Decoder::new()?;
    let mut samples = Vec::new();
    let mut frame = Frame::default();
    while mkv
        .next_frame(&mut frame)
        .map_err(|e| Error(e.to_string()))?
    {
        if frame.track == track.track_number().get() {
            dec.decode(&frame.data, &mut samples)?;
        }
    }
    Ok(trim(samples, pre_skip, None))
}

/// The samples of a call file, by its extension.
pub fn decode(path: &Path) -> Result<Vec<f32>> {
    let io = |e: std::io::Error| Error(format!("{}: {e}", path.display()));
    match path.extension().and_then(|e| e.to_str()) {
        Some("opus") => {
            let mut bytes = Vec::new();
            File::open(path)
                .map_err(io)?
                .read_to_end(&mut bytes)
                .map_err(io)?;
            decode_ogg_opus(&bytes)
        }
        Some("webm") => decode_webm(File::open(path).map_err(io)?),
        _ => fail(format!("{}: not a call audio file", path.display())),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tone(seconds: f32) -> Vec<f32> {
        (0..(seconds * RATE as f32) as usize)
            .map(|i| 0.3 * (i as f32 * 2.0 * std::f32::consts::PI * 220.0 / RATE as f32).sin())
            .collect()
    }

    #[test]
    fn ogg_opus_keeps_the_length_and_the_shape_of_the_samples() {
        let samples = tone(1.234);
        let bytes = encode_ogg_opus(&samples).unwrap();
        assert_eq!(&bytes[..4], b"OggS");
        let back = decode_ogg_opus(&bytes).unwrap();
        assert_eq!(back.len(), samples.len());
        let rms = |v: &[f32]| (v.iter().map(|s| s * s).sum::<f32>() / v.len() as f32).sqrt();
        assert!(
            (rms(&back) - rms(&samples)).abs() < 0.03,
            "{} {}",
            rms(&back),
            rms(&samples)
        );
    }

    #[test]
    fn the_granule_positions_never_decrease_when_the_end_is_near_a_page_end() {
        // A page ends each 50 frames (1 s). 23,900 samples end 100 samples before that page end.
        for len in [23_900, 24_000, 47_990, 100] {
            let bytes = encode_ogg_opus(&tone(len as f32 / RATE as f32)).unwrap();
            let mut reader = PacketReader::new(Cursor::new(&bytes));
            let mut last = 0;
            while let Some(p) = reader.read_packet().unwrap() {
                assert!(
                    p.absgp_page() >= last,
                    "{len}: {} after {last}",
                    p.absgp_page()
                );
                last = p.absgp_page();
            }
            assert_eq!(decode_ogg_opus(&bytes).unwrap().len(), len);
        }
    }

    #[test]
    fn an_empty_call_makes_a_valid_file() {
        let bytes = encode_ogg_opus(&[]).unwrap();
        assert!(decode_ogg_opus(&bytes).unwrap().is_empty());
    }

    #[test]
    fn a_file_that_is_not_opus_is_an_error() {
        assert!(decode_ogg_opus(b"not an ogg file at all, just text").is_err());
    }
}
