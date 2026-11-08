//! Sample work: trim the edge silence, level a sentence, write a WAV file.
//!
//! `trim` follows `librosa.effects.trim` as kokoro-onnx 0.6.1 bundles it (ISC).
//! `level` follows `_level` of `packages/voice/src/agentsi_voice/render.py`.

use std::io::Write;
use std::path::Path;

/// Kokoro makes 24 kHz mono samples.
pub const RATE: u32 = 24_000;
const TRIM_FRAME: usize = 2048;
const TRIM_HOP: usize = 512;
const TRIM_TOP_DB: f64 = 60.0;
const TARGET_RMS: f32 = 0.08;
const PEAK_LIMIT: f32 = 0.95;

/// The range of `audio` with sound: frames over -60 dB from the loudest frame.
/// The frames are 2048 samples, 512 apart, with 1024 zeros added on each side.
pub fn trim_range(audio: &[f32]) -> (usize, usize) {
    let pad = TRIM_FRAME / 2;
    let padded = audio.len() + 2 * pad;
    let frames = 1 + (padded - TRIM_FRAME) / TRIM_HOP;
    let sample = |i: usize| -> f64 {
        i.checked_sub(pad)
            .and_then(|k| audio.get(k))
            .map_or(0.0, |&s| f64::from(s))
    };
    let rms: Vec<f64> = (0..frames)
        .map(|f| {
            let start = f * TRIM_HOP;
            let power: f64 = (start..start + TRIM_FRAME)
                .map(|i| sample(i).powi(2))
                .sum::<f64>()
                / TRIM_FRAME as f64;
            power.sqrt()
        })
        .collect();
    let db = |x: f64| 10.0 * x.powi(2).max(1e-10).log10();
    let top = db(rms.iter().copied().fold(0.0, f64::max));
    let loud: Vec<usize> = (0..frames)
        .filter(|&f| db(rms[f]) - top > -TRIM_TOP_DB)
        .collect();
    match (loud.first(), loud.last()) {
        (Some(&first), Some(&last)) => (first * TRIM_HOP, ((last + 1) * TRIM_HOP).min(audio.len())),
        _ => (0, 0),
    }
}

/// Brings the RMS level to 0.08, then lowers the peak to 0.95 if it is higher.
pub fn level(audio: &mut [f32]) {
    if audio.is_empty() {
        return;
    }
    let mean = audio.iter().map(|s| f64::from(*s).powi(2)).sum::<f64>() / audio.len() as f64;
    let rms = match mean.sqrt() as f32 {
        0.0 => 1.0,
        r => r,
    };
    let gain = TARGET_RMS / rms;
    audio.iter_mut().for_each(|s| *s *= gain);
    let peak = audio.iter().fold(0.0f32, |m, s| m.max(s.abs()));
    if peak > PEAK_LIMIT {
        let gain = PEAK_LIMIT / peak;
        audio.iter_mut().for_each(|s| *s *= gain);
    }
}

pub fn silence(seconds: f32) -> Vec<f32> {
    vec![0.0; (seconds * RATE as f32) as usize]
}

/// A 16-bit mono WAV file at `RATE`.
pub fn write_wav(path: &Path, audio: &[f32]) -> std::io::Result<()> {
    let data = (audio.len() * 2) as u32;
    let mut out = Vec::with_capacity(44 + data as usize);
    out.extend_from_slice(b"RIFF");
    out.extend_from_slice(&(36 + data).to_le_bytes());
    out.extend_from_slice(b"WAVEfmt ");
    out.extend_from_slice(&16u32.to_le_bytes());
    out.extend_from_slice(&1u16.to_le_bytes());
    out.extend_from_slice(&1u16.to_le_bytes());
    out.extend_from_slice(&RATE.to_le_bytes());
    out.extend_from_slice(&(RATE * 2).to_le_bytes());
    out.extend_from_slice(&2u16.to_le_bytes());
    out.extend_from_slice(&16u16.to_le_bytes());
    out.extend_from_slice(b"data");
    out.extend_from_slice(&data.to_le_bytes());
    for s in audio {
        out.extend_from_slice(&((s.clamp(-1.0, 1.0) * 32767.0) as i16).to_le_bytes());
    }
    std::fs::File::create(path)?.write_all(&out)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tone(len: usize) -> Vec<f32> {
        (0..len).map(|i| (i as f32 * 0.1).sin() * 0.5).collect()
    }

    #[test]
    fn trim_cuts_the_silence_at_both_ends() {
        let mut audio = vec![0.0; 24_000];
        audio.extend(tone(12_000));
        audio.extend(vec![0.0; 24_000]);
        let (start, end) = trim_range(&audio);
        // The first loud frame starts within one hop before the sound and the last ends within 2 frames after.
        assert!(start <= 24_000 && 24_000 - start <= TRIM_FRAME, "{start}");
        assert!(end >= 36_000 && end - 36_000 <= TRIM_FRAME, "{end}");
    }

    #[test]
    fn a_silent_signal_stays_whole_like_librosa_and_a_short_one_keeps_its_end() {
        assert_eq!(trim_range(&[0.0; 5000]), (0, 5000));
        let short = tone(100);
        assert_eq!(trim_range(&short), (0, 100));
    }

    #[test]
    fn level_sets_the_rms_and_limits_the_peak() {
        let mut audio = tone(24_000);
        level(&mut audio);
        let rms = (audio.iter().map(|s| s * s).sum::<f32>() / audio.len() as f32).sqrt();
        assert!((rms - TARGET_RMS).abs() < 1e-3, "{rms}");
        let mut spike = vec![0.0; 1000];
        spike[0] = 1.0;
        level(&mut spike);
        assert!((spike[0] - PEAK_LIMIT).abs() < 1e-6);
        let mut quiet = vec![0.0; 10];
        level(&mut quiet);
        assert!(quiet.iter().all(|s| *s == 0.0));
    }

    #[test]
    fn a_wav_file_has_a_44_byte_header() {
        let dir = std::env::temp_dir().join(format!("agentsi-voice-wav-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("a.wav");
        write_wav(&path, &[0.0, 0.5, -0.5]).unwrap();
        let bytes = std::fs::read(&path).unwrap();
        assert_eq!(bytes.len(), 44 + 6);
        assert_eq!(&bytes[..4], b"RIFF");
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
