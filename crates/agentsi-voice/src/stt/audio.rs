//! Audio for speech in: a WAV reader and a resampler to the 16 kHz that Parakeet expects.

use super::Error;
use std::path::Path;

/// The sample rate of the model.
pub const RATE: u32 = 16_000;

/// Mono samples in the range -1 to 1, and their sample rate.
pub struct Clip {
    pub samples: Vec<f32>,
    pub rate: u32,
}

impl Clip {
    pub fn seconds(&self) -> f32 {
        self.samples.len() as f32 / self.rate as f32
    }

    /// The samples at [`RATE`].
    pub fn at_model_rate(&self) -> Vec<f32> {
        resample(&self.samples, self.rate, RATE)
    }
}

/// Reads a PCM (8, 16, 24 or 32 bit) or float WAV file and mixes the channels to mono.
pub fn read_wav(path: &Path) -> Result<Clip, Error> {
    let bad = |why: &str| Error::Audio(format!("{}: {why}", path.display()));
    let bytes = std::fs::read(path).map_err(|e| bad(&e.to_string()))?;
    if bytes.len() < 12 || &bytes[0..4] != b"RIFF" || &bytes[8..12] != b"WAVE" {
        return Err(bad("not a WAV file"));
    }
    let (mut format, mut data) = (None, None);
    let mut at = 12;
    while at + 8 <= bytes.len() {
        let id = &bytes[at..at + 4];
        let size = u32::from_le_bytes(bytes[at + 4..at + 8].try_into().expect("4 bytes")) as usize;
        let body = at + 8;
        let end = body.saturating_add(size).min(bytes.len());
        match id {
            b"fmt " => format = Some(&bytes[body..end]),
            b"data" => data = Some(&bytes[body..end]),
            _ => {}
        }
        at = body.saturating_add(size).saturating_add(size & 1);
    }
    let fmt = format
        .filter(|f| f.len() >= 16)
        .ok_or_else(|| bad("no fmt chunk"))?;
    let data = data.ok_or_else(|| bad("no data chunk"))?;
    let u16_at = |i: usize| u16::from_le_bytes([fmt[i], fmt[i + 1]]);
    let (mut tag, channels, rate, bits) = (
        u16_at(0),
        u16_at(2) as usize,
        u32::from_le_bytes(fmt[4..8].try_into().expect("4 bytes")),
        u16_at(14) as usize,
    );
    if tag == 0xFFFE && fmt.len() >= 26 {
        tag = u16_at(24);
    }
    if channels == 0 || rate == 0 {
        return Err(bad("no channels or no sample rate"));
    }
    let width = bits / 8;
    let sample = |b: &[u8]| -> Option<f32> {
        Some(match (tag, bits) {
            (1, 8) => (b[0] as f32 - 128.0) / 128.0,
            (1, 16) => i16::from_le_bytes([b[0], b[1]]) as f32 / 32_768.0,
            (1, 24) => i32::from_le_bytes([0, b[0], b[1], b[2]]) as f32 / 2_147_483_648.0,
            (1, 32) => i32::from_le_bytes([b[0], b[1], b[2], b[3]]) as f32 / 2_147_483_648.0,
            (3, 32) => f32::from_le_bytes([b[0], b[1], b[2], b[3]]),
            _ => return None,
        })
    };
    if sample(&vec![0; width.max(4)]).is_none() {
        return Err(bad(&format!(
            "format {tag} with {bits} bits is not supported"
        )));
    }
    let frame = width * channels;
    let samples = data
        .chunks_exact(frame)
        .map(|f| {
            f.chunks_exact(width)
                .map(|b| sample(b).unwrap_or(0.0))
                .sum::<f32>()
                / channels as f32
        })
        .collect();
    Ok(Clip { samples, rate })
}

const ZERO_CROSSINGS: f64 = 16.0;

/// The settings of the filter for a pair of rates.
#[derive(Clone, Copy)]
struct Filter {
    ratio: f64,
    /// The cutoff, as a fraction of the input rate's Nyquist frequency.
    cutoff: f64,
    half: isize,
}

impl Filter {
    fn new(from: u32, to: u32) -> Filter {
        let ratio = to as f64 / from as f64;
        let cutoff = ratio.min(1.0) * 0.97;
        Filter {
            ratio,
            cutoff,
            half: (ZERO_CROSSINGS / cutoff).ceil() as isize,
        }
    }

    /// Output sample `n`. `at` gives the input sample of an index, or `None` outside the input.
    fn output(&self, n: u64, at: impl Fn(isize) -> Option<f32>) -> f32 {
        let centre = n as f64 / self.ratio;
        let base = centre.floor() as isize;
        let mut acc = 0.0f64;
        for k in (base - self.half + 1)..=(base + self.half) {
            let Some(s) = at(k) else {
                continue;
            };
            let x = centre - k as f64;
            let window = 0.5 + 0.5 * (std::f64::consts::PI * x / self.half as f64).cos();
            let arg = std::f64::consts::PI * x * self.cutoff;
            let sinc = if arg.abs() < 1e-9 {
                1.0
            } else {
                arg.sin() / arg
            };
            acc += s as f64 * sinc * self.cutoff * window;
        }
        acc as f32
    }
}

/// Resamples with a Hann-windowed sinc filter. It filters below the lower Nyquist
/// rate when it lowers the rate, so a 48 kHz microphone does not alias into speech.
pub fn resample(samples: &[f32], from: u32, to: u32) -> Vec<f32> {
    if from == to || samples.is_empty() {
        return samples.to_vec();
    }
    let filter = Filter::new(from, to);
    let out_len = (samples.len() as f64 * filter.ratio).floor() as usize;
    (0..out_len as u64)
        .map(|n| {
            filter.output(n, |k| {
                usize::try_from(k)
                    .ok()
                    .and_then(|k| samples.get(k))
                    .copied()
            })
        })
        .collect()
}

/// The same filter for a stream that arrives in chunks. The output of all the chunks is the
/// output of [`resample`] on their sum, except the last few input samples, which wait for
/// more input. A chunk by chunk call of [`resample`] would dip at each edge.
pub struct Resampler {
    from: u32,
    to: u32,
    filter: Filter,
    /// Input samples from index `base` on, which the next outputs may still read.
    buf: Vec<f32>,
    base: usize,
    /// The index of the next output sample.
    next: u64,
}

impl Resampler {
    pub fn new(from: u32, to: u32) -> Resampler {
        Resampler {
            from,
            to,
            filter: Filter::new(from, to),
            buf: Vec::new(),
            base: 0,
            next: 0,
        }
    }

    /// Takes the next input samples and gives the output samples that they complete.
    pub fn push(&mut self, chunk: &[f32]) -> Vec<f32> {
        if self.from == self.to {
            return chunk.to_vec();
        }
        self.buf.extend_from_slice(chunk);
        let end = self.base + self.buf.len();
        let mut out = Vec::new();
        loop {
            let centre = self.next as f64 / self.filter.ratio;
            // The filter reads up to `half` samples after the centre.
            if centre.floor() as isize + self.filter.half >= end as isize {
                break;
            }
            let base = self.base;
            out.push(self.filter.output(self.next, |k| {
                usize::try_from(k)
                    .ok()
                    .and_then(|k| k.checked_sub(base))
                    .and_then(|k| self.buf.get(k))
                    .copied()
            }));
            self.next += 1;
        }
        let keep_from = (self.next as f64 / self.filter.ratio).floor() as isize - self.filter.half;
        if let Ok(drop) = usize::try_from(keep_from - self.base as isize)
            && drop > 0
        {
            self.buf.drain(..drop);
            self.base += drop;
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn wav(rate: u32, channels: u16, samples: &[i16]) -> Vec<u8> {
        let data: Vec<u8> = samples.iter().flat_map(|s| s.to_le_bytes()).collect();
        let mut out = b"RIFF".to_vec();
        out.extend_from_slice(&(36 + data.len() as u32).to_le_bytes());
        out.extend_from_slice(b"WAVEfmt ");
        out.extend_from_slice(&16u32.to_le_bytes());
        out.extend_from_slice(&1u16.to_le_bytes());
        out.extend_from_slice(&channels.to_le_bytes());
        out.extend_from_slice(&rate.to_le_bytes());
        out.extend_from_slice(&(rate * 2 * channels as u32).to_le_bytes());
        out.extend_from_slice(&(2 * channels).to_le_bytes());
        out.extend_from_slice(&16u16.to_le_bytes());
        out.extend_from_slice(b"data");
        out.extend_from_slice(&(data.len() as u32).to_le_bytes());
        out.extend_from_slice(&data);
        out
    }

    #[test]
    fn a_stereo_wav_mixes_to_mono() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("a.wav");
        std::fs::write(&path, wav(16_000, 2, &[16_384, 0, -16_384, -16_384])).unwrap();
        let clip = read_wav(&path).unwrap();
        assert_eq!(clip.rate, 16_000);
        assert_eq!(clip.samples, vec![0.25, -0.5]);
        std::fs::write(&path, b"nope").unwrap();
        assert!(read_wav(&path).is_err());
    }

    fn tone(rate: u32, hz: f64, seconds: f64) -> Vec<f32> {
        (0..(rate as f64 * seconds) as usize)
            .map(|i| (2.0 * std::f64::consts::PI * hz * i as f64 / rate as f64).sin() as f32)
            .collect()
    }

    fn rms(x: &[f32]) -> f32 {
        (x.iter().map(|v| v * v).sum::<f32>() / x.len() as f32).sqrt()
    }

    #[test]
    fn a_stream_in_chunks_gives_the_samples_of_one_call() {
        let input: Vec<f32> = tone(44_100, 440.0, 0.4)
            .iter()
            .zip(tone(44_100, 3_100.0, 0.4))
            .map(|(a, b)| 0.5 * (a + b))
            .collect();
        let whole = resample(&input, 44_100, 16_000);
        let mut stream = Resampler::new(44_100, 16_000);
        let mut chunked = Vec::new();
        for chunk in input.chunks(701) {
            chunked.extend(stream.push(chunk));
        }
        // The last samples wait for more input.
        assert!(chunked.len() + 40 > whole.len() && chunked.len() <= whole.len());
        for (i, (a, b)) in chunked.iter().zip(&whole).enumerate() {
            assert!((a - b).abs() < 1e-5, "sample {i}: {a} {b}");
        }
        // The buffer stays small: a long stream keeps no history.
        assert!(stream.buf.len() < 701 + 200);
        assert_eq!(
            Resampler::new(16_000, 16_000).push(&[0.5, 0.25]),
            [0.5, 0.25]
        );
    }

    #[test]
    fn resampling_keeps_a_tone_and_cuts_what_would_alias() {
        let keep = resample(&tone(48_000, 1_000.0, 0.5), 48_000, 16_000);
        assert_eq!(keep.len(), 8_000);
        assert!((rms(&keep[400..7_600]) - 0.707).abs() < 0.02);
        // 12 kHz is above the Nyquist rate of 16 kHz audio: it must vanish, not fold to 4 kHz.
        let cut = resample(&tone(48_000, 12_000.0, 0.5), 48_000, 16_000);
        assert!(rms(&cut[400..7_600]) < 0.02);
        let up = resample(&tone(8_000, 500.0, 0.5), 8_000, 16_000);
        assert!((rms(&up[400..7_600]) - 0.707).abs() < 0.02);
    }
}
