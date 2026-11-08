//! The ONNX Runtime side of Parakeet: three sessions and the joint network state.

use super::decode::{self, Joiner, Joint, Vocab};
use super::{Error, MAX_SECONDS, RATE};
use ort::environment::Environment;
use ort::session::Session;
use ort::session::builder::GraphOptimizationLevel;
use ort::value::{DynValue, Tensor, TensorElementType};
use std::path::Path;

pub(super) const PREPROCESSOR: &str = "nemo128.onnx";
pub(super) const ENCODER: &str = "encoder-model.int8.onnx";
pub(super) const DECODER: &str = "decoder_joint-model.int8.onnx";
pub(super) const VOCAB: &str = "vocab.txt";
/// A clip shorter than this holds no word, and the feature step needs a full window.
const MIN_SAMPLES: usize = RATE as usize / 5;

/// Half the logical CPUs, at least 1. Same rule as the Kokoro voice.
fn threads() -> usize {
    std::thread::available_parallelism().map_or(1, |n| (n.get() / 2).max(1))
}

fn runtime(e: impl std::fmt::Display) -> Error {
    Error::Runtime(e.to_string())
}

pub struct Transcriber {
    preprocessor: Session,
    encoder: Session,
    decoder: Session,
    _env: Environment,
    vocab: Vocab,
    layout: JointLayout,
}

/// What the decoder model asks for, read from its inputs.
struct JointLayout {
    targets: TensorElementType,
    target_length: TensorElementType,
    /// The layers and the width of the two prediction-network states.
    states: [(usize, usize); 2],
}

impl Transcriber {
    /// Loads the model files of `dir` and runs one second of silence, which warms the sessions.
    pub fn open(dir: &Path) -> Result<Transcriber, Error> {
        for name in [PREPROCESSOR, ENCODER, DECODER, VOCAB] {
            let path = dir.join(name);
            if !path.is_file() {
                return Err(Error::NotDownloaded(path));
            }
        }
        let vocab = std::fs::read_to_string(dir.join(VOCAB))
            .map_err(|e| Error::Model(format!("{VOCAB}: {e}")))
            .and_then(|text| Vocab::parse(&text))?;
        let env = crate::runtime::environment().map_err(Error::Runtime)?;
        let session = |name: &str| -> Result<Session, Error> {
            Session::builder(&env)
                .map_err(runtime)?
                .with_optimization_level(GraphOptimizationLevel::Level3)
                .map_err(runtime)?
                .with_intra_threads(threads())
                .map_err(runtime)?
                .with_intra_op_spinning(false)
                .map_err(runtime)?
                .commit_from_file(dir.join(name))
                .map_err(runtime)
        };
        let preprocessor = session(PREPROCESSOR)?;
        let encoder = session(ENCODER)?;
        let decoder = session(DECODER)?;
        let layout = joint_layout(&decoder)?;
        let mut me = Transcriber {
            preprocessor,
            encoder,
            decoder,
            _env: env,
            vocab,
            layout,
        };
        me.transcribe(&vec![0.0; RATE as usize])?;
        Ok(me)
    }

    /// The text of 16 kHz mono samples in the range -1 to 1. An empty or very short clip gives "".
    pub fn transcribe(&mut self, samples: &[f32]) -> Result<String, Error> {
        if samples.len() < MIN_SAMPLES {
            return Ok(String::new());
        }
        if samples.len() as f32 > MAX_SECONDS * RATE as f32 {
            return Err(Error::Audio(format!(
                "The recording is longer than {MAX_SECONDS} seconds"
            )));
        }
        let (features, frames) = self.features(samples)?;
        let (encoded, width, count) = self.encode(features, frames)?;
        let blank = self.vocab.blank;
        let mut joiner = OrtJoiner::new(self, encoded, width);
        let ids = decode::greedy(&mut joiner, count, blank)?;
        Ok(self.vocab.text(&ids))
    }

    /// Mel features: a `[1, 128, T]` tensor and its length.
    fn features(&mut self, samples: &[f32]) -> Result<(DynValue, i64), Error> {
        let n = samples.len();
        let inputs = ort::inputs![
            "waveforms" => Tensor::from_array(([1, n], samples.to_vec())).map_err(runtime)?,
            "waveforms_lens" => Tensor::from_array(([1], vec![n as i64])).map_err(runtime)?,
        ];
        let mut out = self.preprocessor.run(inputs).map_err(runtime)?;
        let (_, lens) = out["features_lens"]
            .try_extract_tensor::<i64>()
            .map_err(runtime)?;
        let len = *lens.first().ok_or_else(|| runtime("no feature length"))?;
        let features = out
            .remove("features")
            .ok_or_else(|| runtime("no features"))?;
        Ok((features, len))
    }

    /// The encoder output as one vector of `width` values for each of `count` frames.
    fn encode(
        &mut self,
        features: DynValue,
        frames: i64,
    ) -> Result<(Vec<f32>, usize, usize), Error> {
        let inputs = ort::inputs![
            "audio_signal" => features,
            "length" => Tensor::from_array(([1], vec![frames])).map_err(runtime)?,
        ];
        let out = self.encoder.run(inputs).map_err(runtime)?;
        let (shape, data) = out["outputs"]
            .try_extract_tensor::<f32>()
            .map_err(runtime)?;
        let (_, lens) = out["encoded_lengths"]
            .try_extract_tensor::<i64>()
            .map_err(runtime)?;
        // The tensor is [1, width, time]. The decoder wants one vector for each frame.
        let (width, time) = (shape[1] as usize, shape[2] as usize);
        let count = lens.first().map_or(time, |&l| (l as usize).min(time));
        let mut frames_major = vec![0.0; count * width];
        for t in 0..count {
            for d in 0..width {
                frames_major[t * width + d] = data[d * time + t];
            }
        }
        Ok((frames_major, width, count))
    }
}

/// The type of `targets` and the shape of the two states, read from the decoder model.
fn joint_layout(decoder: &Session) -> Result<JointLayout, Error> {
    let find = |name: &str| {
        decoder
            .inputs()
            .iter()
            .find(|i| i.name() == name)
            .ok_or_else(|| Error::Model(format!("the decoder has no `{name}` input")))
    };
    let int_type = |name: &str| {
        let ty = find(name)?
            .dtype()
            .tensor_type()
            .ok_or_else(|| Error::Model(format!("`{name}` is not a tensor")))?;
        if matches!(ty, TensorElementType::Int32 | TensorElementType::Int64) {
            Ok(ty)
        } else {
            Err(Error::Model(format!("`{name}` has type {ty:?}")))
        }
    };
    let (targets, target_length) = (int_type("targets")?, int_type("target_length")?);
    let mut shapes = [(0, 0); 2];
    for (i, name) in ["input_states_1", "input_states_2"].into_iter().enumerate() {
        let shape = find(name)?
            .dtype()
            .tensor_shape()
            .filter(|s| s.len() == 3 && s[0] > 0 && s[2] > 0)
            .ok_or_else(|| Error::Model(format!("`{name}` has an unexpected shape")))?;
        shapes[i] = (shape[0] as usize, shape[2] as usize);
    }
    Ok(JointLayout {
        targets,
        target_length,
        states: shapes,
    })
}

fn int<const N: usize>(
    ty: TensorElementType,
    shape: [usize; N],
    value: i64,
) -> Result<DynValue, Error> {
    let tensor = match ty {
        TensorElementType::Int32 => Tensor::from_array((shape, vec![value as i32]))
            .map_err(runtime)?
            .into_dyn(),
        _ => Tensor::from_array((shape, vec![value]))
            .map_err(runtime)?
            .into_dyn(),
    };
    Ok(tensor)
}

/// The decoder and joint network for one clip.
struct OrtJoiner<'a> {
    owner: &'a mut Transcriber,
    encoded: Vec<f32>,
    width: usize,
    states: [Vec<f32>; 2],
    pending: Option<[Vec<f32>; 2]>,
}

impl<'a> OrtJoiner<'a> {
    fn new(owner: &'a mut Transcriber, encoded: Vec<f32>, width: usize) -> Self {
        let states = owner
            .layout
            .states
            .map(|(layers, size)| vec![0.0; layers * size]);
        OrtJoiner {
            owner,
            encoded,
            width,
            states,
            pending: None,
        }
    }
}

impl Joiner for OrtJoiner<'_> {
    fn step(&mut self, frame: usize, previous: Option<i64>) -> Result<Joint, Error> {
        let o = &mut *self.owner;
        let blank = o.vocab.blank;
        let [(l1, s1), (l2, s2)] = o.layout.states;
        let frame_vec = self.encoded[frame * self.width..(frame + 1) * self.width].to_vec();
        let inputs = ort::inputs![
            "encoder_outputs" => Tensor::from_array(([1, self.width, 1], frame_vec)).map_err(runtime)?,
            "targets" => int(o.layout.targets, [1, 1], previous.unwrap_or(blank))?,
            "target_length" => int(o.layout.target_length, [1], 1)?,
            "input_states_1" => Tensor::from_array(([l1, 1, s1], self.states[0].clone())).map_err(runtime)?,
            "input_states_2" => Tensor::from_array(([l2, 1, s2], self.states[1].clone())).map_err(runtime)?,
        ];
        let out = o.decoder.run(inputs).map_err(runtime)?;
        let (_, scores) = out["outputs"]
            .try_extract_tensor::<f32>()
            .map_err(runtime)?;
        let tokens = o.vocab.len();
        if scores.len() <= tokens {
            return Err(Error::Model(format!(
                "the joint gave {} scores for {tokens} tokens",
                scores.len()
            )));
        }
        let joint = Joint {
            tokens: scores[..tokens].to_vec(),
            durations: scores[tokens..].to_vec(),
        };
        let take = |name: &str| -> Result<Vec<f32>, Error> {
            let (_, data) = out[name].try_extract_tensor::<f32>().map_err(runtime)?;
            Ok(data.to_vec())
        };
        self.pending = Some([take("output_states_1")?, take("output_states_2")?]);
        Ok(joint)
    }

    fn commit(&mut self) {
        if let Some(states) = self.pending.take() {
            self.states = states;
        }
    }
}
