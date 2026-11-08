//! The one ONNX Runtime environment of the process.
//!
//! ONNX Runtime allows one environment for each process, and the pill runs Kokoro (speech
//! out) and Parakeet (speech in) together. Each model takes the environment from here.

use ort::environment::Environment;
use std::sync::OnceLock;

static ENVIRONMENT: OnceLock<Result<Environment, String>> = OnceLock::new();

/// The environment, made at the first call. A failure repeats at each call.
pub fn environment() -> Result<Environment, String> {
    ENVIRONMENT
        .get_or_init(|| {
            ort::init()
                .with_name("agentsi-voice")
                .build()
                .map_err(|e| e.to_string())
        })
        .clone()
}
