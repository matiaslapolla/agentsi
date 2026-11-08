//! Silero VAD v5 (MIT) on ONNX Runtime. The model reads 512 samples at 16 kHz with 64 samples
//! of the frame before, and a state of 2 x 128 values. It gives a chance of speech and the
//! next state.

use super::{Classifier, Error, FILE, FRAME, RATE};
use ort::environment::Environment;
use ort::session::Session;
use ort::session::builder::GraphOptimizationLevel;
use ort::value::Tensor;
use std::path::{Path, PathBuf};

/// Samples of the last frame that the model reads before the new frame.
const CONTEXT: usize = 64;
const STATE: usize = 2 * 128;

/// The folder that holds the model file: `<models>/catalog/silero-vad-v5/`.
pub fn model_dir() -> Option<PathBuf> {
    agentsi_models::default_root().map(|root| root.join("catalog").join(super::MODEL_ID))
}

fn runtime(e: impl std::fmt::Display) -> Error {
    Error::Runtime(e.to_string())
}

pub struct Silero {
    session: Session,
    _env: Environment,
    state: Vec<f32>,
    context: [f32; CONTEXT],
}

impl Silero {
    /// Loads the model file of `dir`.
    pub fn open(dir: &Path) -> Result<Silero, Error> {
        let path = dir.join(FILE);
        if !path.is_file() {
            return Err(Error::NotDownloaded(path));
        }
        let env = crate::runtime::environment().map_err(Error::Runtime)?;
        // The model is tiny and runs one frame at a time: one thread is the fastest.
        let session = Session::builder(&env)
            .map_err(runtime)?
            .with_optimization_level(GraphOptimizationLevel::Level3)
            .map_err(runtime)?
            .with_intra_threads(1)
            .map_err(runtime)?
            .with_intra_op_spinning(false)
            .map_err(runtime)?
            .commit_from_file(&path)
            .map_err(runtime)?;
        for name in ["input", "state", "sr"] {
            if !session.inputs().iter().any(|i| i.name() == name) {
                return Err(Error::Model(format!("the model has no `{name}` input")));
            }
        }
        let mut me = Silero {
            session,
            _env: env,
            state: vec![0.0; STATE],
            context: [0.0; CONTEXT],
        };
        me.speech(&[0.0; FRAME])?;
        me.reset();
        Ok(me)
    }
}

impl Classifier for Silero {
    fn speech(&mut self, frame: &[f32]) -> Result<f32, Error> {
        if frame.len() != FRAME {
            return Err(Error::Model(format!(
                "a frame has {FRAME} samples, not {}",
                frame.len()
            )));
        }
        let mut input = Vec::with_capacity(CONTEXT + FRAME);
        input.extend_from_slice(&self.context);
        input.extend_from_slice(frame);
        let inputs = ort::inputs![
            "input" => Tensor::from_array(([1, CONTEXT + FRAME], input)).map_err(runtime)?,
            "state" => Tensor::from_array(([2, 1, 128], self.state.clone())).map_err(runtime)?,
            "sr" => Tensor::from_array(([0usize; 0], vec![RATE as i64])).map_err(runtime)?,
        ];
        let out = self.session.run(inputs).map_err(runtime)?;
        let (_, chance) = out["output"].try_extract_tensor::<f32>().map_err(runtime)?;
        let chance = *chance
            .first()
            .ok_or_else(|| Error::Model("the model gave no chance".into()))?;
        let (_, next) = out["stateN"].try_extract_tensor::<f32>().map_err(runtime)?;
        if next.len() != STATE {
            return Err(Error::Model(format!(
                "the model gave a state of {} values, not {STATE}",
                next.len()
            )));
        }
        self.state.copy_from_slice(next);
        self.context.copy_from_slice(&frame[FRAME - CONTEXT..]);
        Ok(chance)
    }

    fn reset(&mut self) {
        self.state.fill(0.0);
        self.context.fill(0.0);
    }
}
