//! Kokoro v1.0 on ONNX Runtime: phonemes in, 24 kHz samples out.
//!
//! The steps follow `Kokoro.create` of kokoro-onnx 0.6.1 (MIT) with `trim`:
//! batches of at most 510 phonemes, a style row chosen by the token count, a trim
//! of the edge silence, and a pause after each batch but the last.
//! A model with a `duration` output also lets kokoro-onnx lengthen the pauses after
//! marks. The catalog model has no such output, so that step does not exist here.

use super::{Error, audio, chunk, vocab};
use ort::session::Session;
use ort::session::builder::GraphOptimizationLevel;
use ort::value::Tensor;
use std::collections::HashMap;
use std::path::Path;

const SENTENCE_PAUSE: f32 = 0.25;
const CLAUSE_PAUSE: f32 = 0.1;
const STYLE: usize = 256;

pub struct Kokoro {
    session: Session,
    _env: ort::environment::Environment,
    tokens_input: String,
    /// Each voice holds one style row of 256 values for each token count.
    voices: HashMap<String, Vec<f32>>,
}

/// Half the logical CPUs, at least 1. Spinning threads, one per CPU, slow a busy machine and themselves.
fn threads() -> usize {
    std::thread::available_parallelism().map_or(1, |n| (n.get() / 2).max(1))
}

fn runtime(e: impl std::fmt::Display) -> Error {
    Error::Runtime(e.to_string())
}

impl Kokoro {
    pub fn open(model: &Path, voices: &Path) -> Result<Kokoro, Error> {
        for path in [model, voices] {
            if !path.is_file() {
                return Err(Error::NotDownloaded(path.to_owned()));
            }
        }
        let voices = read_voices(voices)?;
        let env = crate::runtime::environment().map_err(Error::Runtime)?;
        let session = Session::builder(&env)
            .map_err(runtime)?
            .with_optimization_level(GraphOptimizationLevel::Level3)
            .map_err(runtime)?
            .with_intra_threads(threads())
            .map_err(runtime)?
            .with_intra_op_spinning(false)
            .map_err(runtime)?
            .commit_from_file(model)
            .map_err(runtime)?;
        let names: Vec<&str> = session.inputs().iter().map(|i| i.name()).collect();
        let tokens_input = ["input_ids", "tokens"]
            .into_iter()
            .find(|n| names.contains(n))
            .ok_or_else(|| Error::Runtime(format!("the model has no token input: {names:?}")))?
            .to_owned();
        for need in ["style", "speed"] {
            if !names.contains(&need) {
                return Err(Error::Runtime(format!("the model has no `{need}` input")));
            }
        }
        Ok(Kokoro {
            session,
            _env: env,
            tokens_input,
            voices,
        })
    }

    pub fn has_voice(&self, name: &str) -> bool {
        self.voices.contains_key(name)
    }

    /// The samples of `phonemes`, already cleaned with `vocab::clean`.
    pub fn speak(&mut self, phonemes: &str, voice: &str) -> Result<Vec<f32>, Error> {
        let batches = chunk::split(phonemes, chunk::MAX_PHONEMES);
        if batches.is_empty() {
            return Err(Error::Runtime(format!("no phonemes in {phonemes:?}")));
        }
        let mut out = Vec::new();
        let last = batches.len() - 1;
        for (index, batch) in batches.iter().enumerate() {
            let mut samples = self.infer(&vocab::tokens(batch), voice)?;
            let (start, end) = audio::trim_range(&samples);
            samples.truncate(end);
            out.extend_from_slice(&samples[start..]);
            if index < last {
                let pause = chunk::pause_after(batch, SENTENCE_PAUSE, CLAUSE_PAUSE);
                out.extend(audio::silence(pause));
            }
        }
        Ok(out)
    }

    fn infer(&mut self, tokens: &[i64], voice: &str) -> Result<Vec<f32>, Error> {
        if tokens.is_empty() {
            return Err(Error::Runtime(
                "no phoneme is in the model vocabulary".into(),
            ));
        }
        let rows = self
            .voices
            .get(voice)
            .ok_or_else(|| Error::Runtime(format!("unknown voice {voice}")))?;
        let row = tokens.len().min(rows.len() / STYLE) - 1;
        let style = rows[row * STYLE..(row + 1) * STYLE].to_vec();
        let mut ids = Vec::with_capacity(tokens.len() + 2);
        ids.push(0);
        ids.extend_from_slice(tokens);
        ids.push(0);
        let inputs = ort::inputs![
            self.tokens_input.as_str() => Tensor::from_array(([1, ids.len()], ids)).map_err(runtime)?,
            "style" => Tensor::from_array(([1, STYLE], style)).map_err(runtime)?,
            "speed" => Tensor::from_array(([1], vec![1.0f32])).map_err(runtime)?,
        ];
        let outputs = self.session.run(inputs).map_err(runtime)?;
        let (_, samples) = outputs[0].try_extract_tensor::<f32>().map_err(runtime)?;
        Ok(samples.to_vec())
    }
}

/// `voices-v1.0.bin` is a NumPy `.npz`: a zip of stored `.npy` arrays, one per voice.
fn read_voices(path: &Path) -> Result<HashMap<String, Vec<f32>>, Error> {
    let bad = |why: &str| Error::Voices(format!("{}: {why}", path.display()));
    let bytes = std::fs::read(path).map_err(|e| bad(&e.to_string()))?;
    let u16_at = |at: usize| -> Option<usize> {
        Some(u16::from_le_bytes(bytes.get(at..at + 2)?.try_into().ok()?) as usize)
    };
    let u32_at = |at: usize| -> Option<usize> {
        Some(u32::from_le_bytes(bytes.get(at..at + 4)?.try_into().ok()?) as usize)
    };
    let end = (0..bytes.len().saturating_sub(21))
        .rev()
        .take(65_557)
        .find(|&i| bytes[i..].starts_with(&[0x50, 0x4b, 0x05, 0x06]))
        .ok_or_else(|| bad("not a zip file"))?;
    let count = u16_at(end + 10).ok_or_else(|| bad("short end record"))?;
    let mut at = u32_at(end + 16).ok_or_else(|| bad("short end record"))?;
    let mut voices = HashMap::new();
    for _ in 0..count {
        let entry = (|| {
            if !bytes.get(at..)?.starts_with(&[0x50, 0x4b, 0x01, 0x02]) {
                return None;
            }
            let method = u16_at(at + 10)?;
            let size = u32_at(at + 20)?;
            let (name_len, extra, comment) = (u16_at(at + 28)?, u16_at(at + 30)?, u16_at(at + 32)?);
            let local = u32_at(at + 42)?;
            let name = std::str::from_utf8(bytes.get(at + 46..at + 46 + name_len)?).ok()?;
            at += 46 + name_len + extra + comment;
            let data = local + 30 + u16_at(local + 26)? + u16_at(local + 28)?;
            Some((name.to_owned(), method, bytes.get(data..data + size)?))
        })()
        .ok_or_else(|| bad("broken zip directory"))?;
        let (name, method, data) = entry;
        if method != 0 {
            return Err(bad("compressed entries are not supported"));
        }
        let Some(voice) = name.strip_suffix(".npy") else {
            continue;
        };
        let rows = read_npy(data).map_err(|why| bad(&format!("{name}: {why}")))?;
        voices.insert(voice.to_owned(), rows);
    }
    if voices.is_empty() {
        return Err(bad("no voices"));
    }
    Ok(voices)
}

/// A little-endian `f32` array whose last dimension is 256.
fn read_npy(data: &[u8]) -> Result<Vec<f32>, String> {
    if !data.starts_with(b"\x93NUMPY") || data.len() < 10 {
        return Err("not an .npy array".into());
    }
    let (len, start) = match data[6] {
        1 => (u16::from_le_bytes([data[8], data[9]]) as usize, 10),
        _ if data.len() >= 12 => (
            u32::from_le_bytes([data[8], data[9], data[10], data[11]]) as usize,
            12,
        ),
        _ => return Err("short header".into()),
    };
    let header = data
        .get(start..start + len)
        .and_then(|h| std::str::from_utf8(h).ok())
        .ok_or("bad header")?;
    if !header.contains("'descr': '<f4'") || !header.contains("'fortran_order': False") {
        return Err(format!("unsupported array: {header}"));
    }
    let shape: Vec<usize> = header
        .split_once("'shape': (")
        .and_then(|(_, rest)| rest.split_once(')'))
        .ok_or("no shape")?
        .0
        .split(',')
        .filter(|s| !s.trim().is_empty())
        .map(|s| s.trim().parse().map_err(|_| format!("bad shape {s}")))
        .collect::<Result<_, _>>()?;
    let total: usize = shape.iter().product();
    if shape.last() != Some(&STYLE) || total == 0 {
        return Err(format!("unexpected shape {shape:?}"));
    }
    let body = &data[start + len..];
    if body.len() != total * 4 {
        return Err(format!("{} bytes for shape {shape:?}", body.len()));
    }
    Ok(body
        .as_chunks::<4>()
        .0
        .iter()
        .map(|b| f32::from_le_bytes(*b))
        .collect())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn npy(shape: &str, values: &[f32]) -> Vec<u8> {
        let mut header =
            format!("{{'descr': '<f4', 'fortran_order': False, 'shape': ({shape}), }}");
        while (10 + header.len() + 1) % 64 != 0 {
            header.push(' ');
        }
        header.push('\n');
        let mut out = b"\x93NUMPY\x01\x00".to_vec();
        out.extend_from_slice(&(header.len() as u16).to_le_bytes());
        out.extend_from_slice(header.as_bytes());
        for v in values {
            out.extend_from_slice(&v.to_le_bytes());
        }
        out
    }

    /// A zip with stored entries, as `numpy.savez` writes it.
    fn zip(entries: &[(&str, Vec<u8>)]) -> Vec<u8> {
        let mut out = Vec::new();
        let mut central = Vec::new();
        for (name, data) in entries {
            let offset = out.len() as u32;
            out.extend_from_slice(&[0x50, 0x4b, 0x03, 0x04]);
            out.extend_from_slice(&[0; 22]);
            out.extend_from_slice(&(name.len() as u16).to_le_bytes());
            out.extend_from_slice(&0u16.to_le_bytes());
            out.extend_from_slice(name.as_bytes());
            out.extend_from_slice(data);
            let mut c = vec![0x50, 0x4b, 0x01, 0x02];
            c.extend_from_slice(&[0; 16]);
            c.extend_from_slice(&(data.len() as u32).to_le_bytes());
            c.extend_from_slice(&(data.len() as u32).to_le_bytes());
            c.extend_from_slice(&(name.len() as u16).to_le_bytes());
            c.extend_from_slice(&[0; 12]);
            c.extend_from_slice(&offset.to_le_bytes());
            c.extend_from_slice(name.as_bytes());
            central.push(c);
        }
        let cd_offset = out.len() as u32;
        let cd: Vec<u8> = central.concat();
        out.extend_from_slice(&cd);
        out.extend_from_slice(&[0x50, 0x4b, 0x05, 0x06, 0, 0, 0, 0]);
        out.extend_from_slice(&(entries.len() as u16).to_le_bytes());
        out.extend_from_slice(&(entries.len() as u16).to_le_bytes());
        out.extend_from_slice(&(cd.len() as u32).to_le_bytes());
        out.extend_from_slice(&cd_offset.to_le_bytes());
        out.extend_from_slice(&0u16.to_le_bytes());
        out
    }

    #[test]
    fn voices_read_from_an_npz_file() {
        let rows: Vec<f32> = (0..2 * STYLE).map(|i| i as f32).collect();
        let dir = std::env::temp_dir().join(format!("agentsi-npz-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("voices.bin");
        std::fs::write(&path, zip(&[("af_test.npy", npy("2, 1, 256", &rows))])).unwrap();
        let voices = read_voices(&path).unwrap();
        assert_eq!(voices["af_test"], rows);
        std::fs::write(&path, b"nope").unwrap();
        assert!(matches!(read_voices(&path), Err(Error::Voices(_))));
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn an_array_of_another_type_or_size_is_refused() {
        let bad = npy("2, 1, 256", &[0.0; 3]);
        assert!(read_npy(&bad).unwrap_err().contains("bytes"));
        let wrong = npy("4", &[0.0; 4]);
        assert!(read_npy(&wrong).unwrap_err().contains("shape"));
    }

    #[test]
    fn a_missing_model_is_not_downloaded() {
        let e = Kokoro::open(
            Path::new("/nonexistent/m.onnx"),
            Path::new("/nonexistent/v.bin"),
        );
        assert!(matches!(e, Err(Error::NotDownloaded(_))));
    }
}
