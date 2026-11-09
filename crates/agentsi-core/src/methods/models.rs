//! `models.*`: the voice models of the catalog, their downloads in the core, and the
//! voice of each role. The file rules are in `agentsi-models`: a download never replaces
//! a file that it did not write, and a delete removes only the files that it wrote.

use super::{Ctx, Router};
use crate::state::{Core, lock};
use agentsi_models::{Catalog, Downloader, Error, Event, Kind, Model, State, Store};
use agentsi_proto::RpcError;
use agentsi_proto::core_api::Empty;
use agentsi_proto::events::{
    ModelsDone, ModelsDoneParams, ModelsError, ModelsErrorParams, ModelsProgress,
    ModelsProgressParams,
};
use agentsi_proto::models_api::{
    DownloadResult, ModelInfo, ModelParams, ModelState, ModelsDelete, ModelsDownload, ModelsList,
    ModelsListResult, ModelsVoices, RoleVoice, VoicesResult,
};
use agentsi_proto::rpc::ErrorKind;
use serde::Deserialize;
use std::collections::{BTreeMap, HashMap};
use std::path::Path;
use std::sync::atomic::AtomicBool;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

/// `agentsi_voice::tts::Voices` reads the same file, but it sits behind the `tts` feature,
/// which links ONNX Runtime into the core. So the core repeats its rules here.
const VOICES: &str = include_str!("../../../../packages/voice/voices.toml");
/// The engines that `voices.toml` may name. The pill speaks only Kokoro, and uses the
/// default voice for a role with another engine.
const ENGINES: [&str; 3] = ["kokoro", "piper", "chatterbox"];
const KOKORO: &str = "kokoro";
/// At most 10 progress events each second, so the app draws at most 10 frames each second.
const PROGRESS_EVERY: Duration = Duration::from_millis(100);

pub fn register(r: Router) -> Router {
    r.add(ModelsList, list)
        .add(ModelsDownload, download)
        .add(ModelsDelete, delete)
        .add(ModelsVoices, voices)
}

/// What the core knows of a model beyond its files.
#[derive(Debug, Clone)]
enum Slot {
    Running {
        bytes: u64,
        total: u64,
        verifying: bool,
    },
    /// The last download failed. It holds the `error` or `conflict` state.
    Failed(ModelState),
}

/// The catalog, the models folder and the downloads of this core.
pub struct Models {
    catalog: Catalog,
    /// `None` when the device has no data folder (no `HOME`).
    store: Option<Store>,
    downloader: Downloader,
    slots: Mutex<HashMap<String, Slot>>,
}

impl Models {
    pub fn new(catalog: Catalog, store: Option<Store>, downloader: Downloader) -> Models {
        Models {
            catalog,
            store,
            downloader,
            slots: Mutex::default(),
        }
    }

    /// The built-in catalog, Hugging Face and the default models folder.
    /// `AGENTSI_MODELS_CATALOG` and `AGENTSI_MODELS_URL` replace the first two, for tests
    /// and manual runs.
    pub fn standard() -> Models {
        let catalog = std::env::var_os("AGENTSI_MODELS_CATALOG")
            .and_then(|path| {
                let parsed = std::fs::read_to_string(&path)
                    .map_err(|e| e.to_string())
                    .and_then(|text| Catalog::parse(&text));
                parsed
                    .inspect_err(|e| {
                        eprintln!(
                            "core: AGENTSI_MODELS_CATALOG {}: {e}. The built-in catalog stays.",
                            Path::new(&path).display()
                        )
                    })
                    .ok()
            })
            .unwrap_or_else(Catalog::builtin);
        let downloader = match std::env::var("AGENTSI_MODELS_URL") {
            Ok(url) if !url.is_empty() => Downloader::with_base_url(url),
            _ => Downloader::hugging_face(),
        };
        Models::new(
            catalog,
            agentsi_models::default_root().map(Store::new),
            downloader,
        )
    }

    fn model(&self, id: &str) -> Result<&Model, RpcError> {
        self.catalog.get(id).map_err(|e| {
            RpcError::new(
                ErrorKind::NotFound,
                e.to_string(),
                "Use models.list for the ids of the models.",
            )
        })
    }

    fn store(&self) -> Result<&Store, RpcError> {
        self.store.as_ref().ok_or_else(|| {
            RpcError::new(
                ErrorKind::Unavailable,
                "this device has no data folder for models",
                "Set HOME or XDG_DATA_HOME.",
            )
        })
    }

    /// The row of one model. It reads the folder of the model, so call it off the event loop.
    fn info(&self, model: &Model) -> ModelInfo {
        let slot = lock(&self.slots).get(&model.id).cloned();
        let state = match (slot, &self.store) {
            (
                Some(Slot::Running {
                    bytes,
                    total,
                    verifying,
                }),
                _,
            ) => ModelState::Downloading {
                bytes,
                total,
                verifying,
            },
            (_, None) => ModelState::Missing,
            (slot, Some(store)) => match (store.state(model), slot) {
                (State::Ready, _) => ModelState::Downloaded,
                (_, Some(Slot::Failed(state))) => state,
                (state, _) => match wrong_size(store, model) {
                    Some(file) => ModelState::Conflict {
                        message: Error::Conflict { file: file.clone() }.to_string(),
                        file,
                    },
                    None => match state {
                        State::Partial { bytes } => ModelState::Partial { bytes },
                        _ => ModelState::Missing,
                    },
                },
            },
        };
        ModelInfo {
            id: model.id.clone(),
            name: model.name.clone(),
            kind: match model.kind {
                Kind::SpeechOut => "speech-out",
                Kind::SpeechIn => "speech-in",
            }
            .to_string(),
            license: model.license.clone(),
            description: model.description.clone(),
            size: model.total_size(),
            state,
        }
    }

    /// Runs one download to its end and sends its events. It blocks.
    fn run(&self, core: &Core, model: &Model, store: &Store) {
        let id = model.id.clone();
        let mut gate = Gate::default();
        let mut verifying = false;
        let mut on_event = |event: Event| {
            let (bytes, total) = match event {
                Event::Progress { bytes, total, .. } => (bytes, total),
                Event::Verifying { .. } => {
                    verifying = true;
                    self.running_bytes(&id)
                }
                Event::FileDone { .. } => {
                    verifying = false;
                    self.running_bytes(&id)
                }
                Event::Retry { .. } | Event::Done => return,
            };
            if let Some(Slot::Running {
                bytes: b,
                total: t,
                verifying: v,
            }) = lock(&self.slots).get_mut(&id)
            {
                (*b, *t, *v) = (bytes, total, verifying);
            }
            if gate.open(Instant::now(), verifying) {
                core.publish::<ModelsProgress>(&ModelsProgressParams {
                    id: id.clone(),
                    bytes,
                    total,
                    verifying,
                });
            }
        };
        let result = self
            .downloader
            .download(store, model, &AtomicBool::new(false), &mut on_event);
        match result {
            Ok(_) => {
                lock(&self.slots).remove(&id);
                core.publish::<ModelsDone>(&ModelsDoneParams { id });
            }
            Err(e) => {
                let failure = ModelsErrorParams {
                    id: id.clone(),
                    error: e.kind().to_string(),
                    message: e.to_string(),
                    file: match &e {
                        Error::Conflict { file } => Some(file.clone()),
                        _ => None,
                    },
                };
                eprintln!("core: download of {id}: {}", failure.message);
                lock(&self.slots).insert(id, Slot::Failed(ModelState::from(&failure)));
                core.publish::<ModelsError>(&failure);
            }
        }
    }

    /// The number of downloaded models and the number of models of the catalog.
    pub fn downloaded(&self) -> (usize, usize) {
        let models = self.catalog.models();
        let have = models
            .iter()
            .filter(|m| self.info(m).state == ModelState::Downloaded)
            .count();
        (have, models.len())
    }

    fn running_bytes(&self, id: &str) -> (u64, u64) {
        match lock(&self.slots).get(id) {
            Some(Slot::Running { bytes, total, .. }) => (*bytes, *total),
            _ => (0, 0),
        }
    }
}

/// The first catalog file of `model` on disk with another size than the pinned size.
/// The download places a file only after its hash check. So that file is the owner's, or a
/// file of an older catalog entry.
fn wrong_size(store: &Store, model: &Model) -> Option<String> {
    model.files.iter().find_map(|f| {
        let len = std::fs::metadata(store.file_path(model, &f.name))
            .ok()?
            .len();
        (len != f.size).then(|| f.name.clone())
    })
}

/// Lets a progress event through at most once in `PROGRESS_EVERY`, and at once when
/// `verifying` changes.
#[derive(Default)]
struct Gate {
    last: Option<(Instant, bool)>,
}

impl Gate {
    fn open(&mut self, now: Instant, verifying: bool) -> bool {
        let open = match self.last {
            None => true,
            Some((at, v)) => v != verifying || now.duration_since(at) >= PROGRESS_EVERY,
        };
        if open {
            self.last = Some((now, verifying));
        }
        open
    }
}

/// Runs `f` with the models of the core off the event loop.
async fn blocking<T: Send + 'static>(
    ctx: &Ctx,
    f: impl FnOnce(&Arc<Core>) -> Result<T, RpcError> + Send + 'static,
) -> Result<T, RpcError> {
    let core = ctx.core.clone();
    crate::blocking::spawn(move || f(&core))
        .await
        .map_err(|e| RpcError::internal(e.to_string()))?
}

async fn list(ctx: Ctx, _: Empty) -> Result<ModelsListResult, RpcError> {
    blocking(&ctx, |core| {
        let m = &core.models;
        Ok(ModelsListResult {
            root: m
                .store
                .as_ref()
                .map(|s| s.root().display().to_string())
                .unwrap_or_default(),
            models: m
                .catalog
                .models()
                .iter()
                .map(|model| m.info(model))
                .collect(),
        })
    })
    .await
}

async fn download(ctx: Ctx, p: ModelParams) -> Result<DownloadResult, RpcError> {
    blocking(&ctx, move |core| {
        let m = &core.models;
        let model = m.model(&p.id)?.clone();
        let store = m.store()?.clone();
        {
            let mut slots = lock(&m.slots);
            if matches!(slots.get(&model.id), Some(Slot::Running { .. })) {
                return Ok(DownloadResult {
                    id: p.id,
                    started: false,
                });
            }
            let bytes = match store.state(&model) {
                State::Ready => {
                    slots.remove(&model.id);
                    return Ok(DownloadResult {
                        id: p.id,
                        started: false,
                    });
                }
                State::Partial { bytes } => bytes,
                State::NotDownloaded => 0,
            };
            slots.insert(
                model.id.clone(),
                Slot::Running {
                    bytes,
                    total: model.total_size(),
                    verifying: false,
                },
            );
        }
        let core = core.clone();
        std::thread::Builder::new()
            .name(format!("download-{}", model.id))
            .spawn({
                let core = core.clone();
                let model = model.clone();
                move || core.models.run(&core, &model, &store)
            })
            .map_err(|e| {
                lock(&core.models.slots).remove(&model.id);
                RpcError::internal(format!("no download thread: {e}"))
            })?;
        Ok(DownloadResult {
            id: p.id,
            started: true,
        })
    })
    .await
}

async fn delete(ctx: Ctx, p: ModelParams) -> Result<ModelInfo, RpcError> {
    blocking(&ctx, move |core| {
        let m = &core.models;
        let model = m.model(&p.id)?;
        let store = m.store()?;
        let busy = || {
            RpcError::new(
                ErrorKind::Busy,
                format!("a download of {} runs", model.id),
                "Wait for the download to end, then delete the model.",
            )
        };
        if matches!(lock(&m.slots).get(&model.id), Some(Slot::Running { .. })) {
            return Err(busy());
        }
        match store.delete(model) {
            Ok(()) => {}
            Err(Error::Busy(_)) => return Err(busy()),
            Err(e) => return Err(RpcError::internal(e.to_string())),
        }
        // Only a failure: a download that started after the check keeps its slot.
        let mut slots = lock(&m.slots);
        if matches!(slots.get(&model.id), Some(Slot::Failed(_))) {
            slots.remove(&model.id);
        }
        drop(slots);
        Ok(m.info(model))
    })
    .await
}

#[derive(Deserialize)]
struct VoicesFile {
    lang: Option<String>,
    default: String,
    #[serde(default)]
    roles: BTreeMap<String, String>,
}

async fn voices(ctx: Ctx, _: Empty) -> Result<VoicesResult, RpcError> {
    blocking(&ctx, |_| {
        let path = agentsi_store::paths::data_root(None)
            .ok()
            .map(|root| root.join("voices.toml"))
            .filter(|p| p.is_file());
        let (source, text) = match &path {
            Some(p) => (
                p.display().to_string(),
                std::fs::read_to_string(p).map_err(|e| RpcError::internal(e.to_string()))?,
            ),
            None => ("built-in".to_string(), VOICES.to_string()),
        };
        parse_voices(&source, &text)
    })
    .await
}

/// The rows of `voices.toml`. The pill reads the same file with `agentsi_voice::tts::Voices`.
fn parse_voices(source: &str, text: &str) -> Result<VoicesResult, RpcError> {
    let file: VoicesFile = toml::from_str(text).map_err(|e| {
        RpcError::new(
            ErrorKind::Invalid,
            format!("{source}: {e}"),
            "Fix voices.toml, or remove it to use the built-in voices.",
        )
    })?;
    let engine = |voice: &str| match voice.split_once(':') {
        Some((engine, name)) if ENGINES.contains(&engine) && !name.is_empty() => {
            Ok(engine == KOKORO)
        }
        _ => Err(RpcError::new(
            ErrorKind::Invalid,
            format!(
                "{source}: invalid voice \"{voice}\", expected <{}>:<voice>",
                ENGINES.join("|")
            ),
            "Fix voices.toml, or remove it to use the built-in voices. The pill does not speak until then.",
        )),
    };
    // The pill also falls back for a Kokoro voice name that its model does not have. Only the
    // model file knows those names, so the screen does not show that case.
    let builtin = || {
        toml::from_str::<VoicesFile>(VOICES)
            .map(|f| f.default)
            .unwrap_or_default()
    };
    let default_note = (!engine(&file.default)?).then(|| {
        format!(
            "Not a Kokoro voice. The pill speaks with the built-in default {}.",
            builtin()
        )
    });
    let fallback = if default_note.is_some() {
        builtin()
    } else {
        file.default.clone()
    };
    let mut roles = Vec::new();
    for (role, voice) in &file.roles {
        roles.push(RoleVoice {
            role: role.clone(),
            voice: voice.clone(),
            note: (!engine(voice)?).then(|| {
                format!("Not a Kokoro voice. The pill speaks with the default voice {fallback}.")
            }),
        });
    }
    Ok(VoicesResult {
        source: source.to_string(),
        lang: file.lang.clone().unwrap_or_else(|| "en-us".into()),
        default: file.default.clone(),
        default_note,
        roles,
    })
}

#[cfg(test)]
mod tests {
    use super::{Gate, PROGRESS_EVERY, VOICES, parse_voices};
    use std::time::{Duration, Instant};

    #[test]
    fn the_gate_lets_at_most_ten_progress_events_through_each_second() {
        let mut gate = Gate::default();
        let start = Instant::now();
        let passed = (0..1000)
            .filter(|i| gate.open(start + Duration::from_millis(*i), false))
            .count();
        assert_eq!(passed, 10);
        // A change of `verifying` passes at once, and starts a new interval.
        let at = start + Duration::from_millis(1001);
        assert!(gate.open(at, true));
        assert!(!gate.open(at + Duration::from_millis(1), true));
        assert!(gate.open(at + PROGRESS_EVERY, true));
    }

    #[test]
    fn the_built_in_voices_give_one_kokoro_voice_for_each_role() {
        let v = parse_voices("built-in", VOICES).unwrap();
        assert_eq!(v.default, "kokoro:af_heart");
        let pm = v.roles.iter().find(|r| r.role == "pm").unwrap();
        assert_eq!(pm.voice, "kokoro:af_heart");
        assert!(v.roles.iter().all(|r| r.note.is_none()));
    }

    #[test]
    fn a_voice_of_another_engine_has_a_note_and_a_bad_file_is_invalid() {
        let v = parse_voices(
            "x",
            "default = \"kokoro:af_heart\"\n[roles]\ndev = \"piper:amy\"\n",
        )
        .unwrap();
        assert_eq!(v.lang, "en-us");
        assert!(v.roles[0].note.as_deref().unwrap().contains("af_heart"));
        let e = parse_voices("x", "roles = 3").unwrap_err();
        assert_eq!(e.kind(), agentsi_proto::ErrorKind::Invalid);
    }

    #[test]
    fn a_voice_that_the_pill_rejects_makes_the_file_invalid() {
        for bad in ["foo:x", "piper", "kokoro:"] {
            let text = format!("default = \"kokoro:af_heart\"\n[roles]\ndev = \"{bad}\"\n");
            let e = parse_voices("x", &text).unwrap_err();
            assert_eq!(e.kind(), agentsi_proto::ErrorKind::Invalid, "{bad}");
        }
        let e = parse_voices("x", "default = \"nope\"\n").unwrap_err();
        assert_eq!(e.kind(), agentsi_proto::ErrorKind::Invalid);
    }

    #[test]
    fn a_default_of_another_engine_falls_back_to_the_built_in_default() {
        let v = parse_voices(
            "x",
            "default = \"piper:amy\"\n[roles]\ndev = \"chatterbox:bob\"\npm = \"kokoro:af_sky\"\n",
        )
        .unwrap();
        assert!(
            v.default_note
                .as_deref()
                .unwrap()
                .contains("kokoro:af_heart")
        );
        let dev = v.roles.iter().find(|r| r.role == "dev").unwrap();
        assert!(dev.note.as_deref().unwrap().contains("kokoro:af_heart"));
        let pm = v.roles.iter().find(|r| r.role == "pm").unwrap();
        assert_eq!(pm.note, None);
    }
}
