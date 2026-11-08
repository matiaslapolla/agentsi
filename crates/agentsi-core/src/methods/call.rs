//! `call.*`: the calls of meetings (U26). The logic is in `agentsi_agency::call`.
//! A render takes minutes of CPU, so it runs off the event loop, sends
//! `meeting.render` events and can be canceled. A core that stops cancels it.

use super::{Ctx, Router};
use crate::state::Core;
use agentsi_agency::call::{self, Error as CallError, Progress, Stage, Synth};
use agentsi_agency::meeting::running::{self, Entry, Refused};
use agentsi_proto::RpcError;
use agentsi_proto::call_api::{
    CallAudio, CallAudioParams, CallAudioResult, CallCancel, CallCancelParams, CallCancelResult,
    CallGet, CallGetParams, CallGetResult, CallHeard, CallHeardParams, CallHeardResult, CallRender,
    CallRenderParams, CallRenderResult, MeetingRender, MeetingRenderParams, RenderInfo,
    SAMPLE_RATE, encode_pcm,
};
use agentsi_proto::events::{DataChanged, DataChangedParams};
use agentsi_proto::rpc::ErrorKind;
use agentsi_store::{Store, paths, read};
use agentsi_voice::tts::{Options, SENTENCE_GAP, Voice, audio};
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, LazyLock, Mutex, MutexGuard};

static NEXT_NAME: AtomicU64 = AtomicU64::new(1);
/// The most samples that one `call.audio` returns: two minutes.
const MAX_CHUNK: u64 = SAMPLE_RATE as u64 * 120;
/// The calls that render now, as `<product>/<minutes id>`: a call renders once at a time.
/// The value is the render with its progress.
static RENDERING: LazyLock<Mutex<HashMap<String, RenderInfo>>> = LazyLock::new(Mutex::default);

fn rendering() -> MutexGuard<'static, HashMap<String, RenderInfo>> {
    RENDERING.lock().unwrap_or_else(|p| p.into_inner())
}

/// Frees the call when the render ends, also when it panics.
struct Marker(String);

impl Marker {
    fn new(key: String, render: &str) -> Option<Marker> {
        let mut all = rendering();
        if all.contains_key(&key) {
            return None;
        }
        let info = RenderInfo {
            render: render.into(),
            done: 0,
            of: 0,
        };
        all.insert(key.clone(), info);
        Some(Marker(key))
    }

    fn progress(&self, done: u32, of: u32) {
        if let Some(info) = rendering().get_mut(&self.0) {
            info.done = done;
            info.of = of;
        }
    }
}

impl Drop for Marker {
    fn drop(&mut self) {
        rendering().remove(&self.0);
    }
}

pub fn register(r: Router) -> Router {
    r.add(CallGet, get)
        .add(CallAudio, audio_chunk)
        .add(CallRender, render)
        .add(CallCancel, cancel)
        .add(CallHeard, heard)
}

fn rpc(e: CallError) -> RpcError {
    match e {
        CallError::NotFound(m) => RpcError::new(
            ErrorKind::NotFound,
            m,
            "Use minutes.list for the ids that exist, and call.get to see if the call has audio.",
        ),
        CallError::Invalid(m) => RpcError::new(
            ErrorKind::Invalid,
            m,
            "These minutes have nothing to speak. A call needs a script.",
        ),
        CallError::Unavailable(m) => RpcError::new(
            ErrorKind::Unavailable,
            m,
            "Set AGENTSI_HOME to the data root. It holds agentsi.db and workspace/.",
        ),
        CallError::Canceled => RpcError::new(
            ErrorKind::Conflict,
            "The render was canceled.",
            "Nothing was written. Render the call again when you want it.",
        ),
        CallError::Failed(m) => RpcError::internal(m),
    }
}

fn store(core: &Core) -> Result<Arc<Store>, RpcError> {
    core.store().map_err(|e| rpc(e.into()))?.ok_or_else(|| {
        rpc(CallError::Unavailable(
            "the data root has no agentsi.db. Import the data first.".into(),
        ))
    })
}

fn root() -> Result<PathBuf, RpcError> {
    paths::data_root(None).map_err(|e| rpc(e.into()))
}

/// Runs `f` on a reader connection, off the event loop.
async fn reading<T: Send + 'static>(
    ctx: &Ctx,
    f: impl FnOnce(&rusqlite::Connection, &std::path::Path) -> call::Result<T> + Send + 'static,
) -> Result<T, RpcError> {
    let core = ctx.core.clone();
    crate::blocking::spawn(move || {
        let (store, root) = (store(&core)?, root()?);
        store
            .read(|c| Ok(f(c, &root)))
            .map_err(|e| rpc(e.into()))?
            .map_err(rpc)
    })
    .await
    .map_err(|e| RpcError::internal(e.to_string()))?
}

fn render_of(product: &str, minutes_id: &str) -> Option<RenderInfo> {
    rendering().get(&format!("{product}/{minutes_id}")).cloned()
}

async fn get(ctx: Ctx, p: CallGetParams) -> Result<CallGetResult, RpcError> {
    let view = {
        let (product, id) = (p.product.clone(), p.minutes_id.clone());
        reading(&ctx, move |c, root| call::get(c, root, &product, &id)).await?
    };
    Ok(CallGetResult {
        render: render_of(&p.product, &p.minutes_id),
        product: p.product,
        minutes_id: p.minutes_id,
        question: view.question,
        segments: view.segments,
        timed: view.timed,
        audio: view.audio,
        heard: view.heard,
    })
}

async fn audio_chunk(ctx: Ctx, p: CallAudioParams) -> Result<CallAudioResult, RpcError> {
    let count = p.count.min(MAX_CHUNK);
    let (samples, total) = reading(&ctx, move |c, root| {
        call::audio_chunk(c, root, &p.product, &p.minutes_id, p.from, count)
    })
    .await?;
    Ok(CallAudioResult {
        from: p.from.min(total),
        total,
        pcm: encode_pcm(&samples),
    })
}

async fn heard(ctx: Ctx, p: CallHeardParams) -> Result<CallHeardResult, RpcError> {
    let core = ctx.core.clone();
    let product = p.product.clone();
    let changed = crate::blocking::spawn(move || {
        let (store, root) = (store(&core)?, root()?);
        store
            .try_write(move |tx| call::set_heard(tx, &root, &p.product, &p.minutes_id, p.heard))
            .map_err(|e| rpc(e.into()))?
            .map_err(rpc)
    })
    .await
    .map_err(|e| RpcError::internal(e.to_string()))??;
    if changed {
        ctx.core.publish::<DataChanged>(&DataChangedParams {
            product,
            kind: "call".into(),
        });
    }
    Ok(CallHeardResult { changed })
}

/// Kokoro with the voice of each role. A segment is its sentences with the pause of the
/// pill between them, at one level, as `_segment_audio` of the Python package makes it.
struct KokoroSynth(Voice);

impl Synth for KokoroSynth {
    fn segment(&mut self, role: &str, text: &str) -> Result<Vec<f32>, String> {
        let (voice, _warnings) = self.0.voice_for(role);
        let mut out: Vec<f32> = Vec::new();
        for sentence in self.0.sentences(text) {
            if !out.is_empty() {
                out.extend(audio::silence(SENTENCE_GAP));
            }
            out.extend(
                self.0
                    .render(&voice, &sentence)
                    .map_err(|e| e.to_string())?,
            );
        }
        if out.is_empty() {
            return Ok(audio::silence(0.1));
        }
        audio::level(&mut out);
        Ok(out)
    }
}

fn open_voice() -> call::Result<KokoroSynth> {
    Voice::open(Options::standard())
        .map(KokoroSynth)
        .map_err(|e| match e {
            agentsi_voice::tts::Error::NotDownloaded(_) => CallError::Unavailable(
                "The Kokoro voice is not downloaded. Download it on the settings screen.".into(),
            ),
            agentsi_voice::tts::Error::NoEspeak => CallError::Unavailable(
                "espeak-ng is not installed. Install it, then render the call again.".into(),
            ),
            other => CallError::Failed(other.to_string()),
        })
}

/// Kokoro takes about 500 MB. glibc keeps most of it after the model drops, so the core would
/// idle at that size until it restarts.
fn give_back_memory() {
    #[cfg(all(target_os = "linux", target_env = "gnu"))]
    // SAFETY: `malloc_trim` takes a plain integer and touches only the allocator.
    unsafe {
        libc::malloc_trim(0);
    }
}

fn stage_name(s: Stage) -> &'static str {
    match s {
        Stage::Start => "start",
        Stage::Segment => "segment",
        Stage::Encode => "encode",
    }
}

async fn render(ctx: Ctx, p: CallRenderParams) -> Result<CallRenderResult, RpcError> {
    let key = format!("{}/{}", p.product, p.minutes_id);
    let name = p.render.clone().unwrap_or_else(|| {
        format!(
            "render-{}-{}",
            std::process::id(),
            NEXT_NAME.fetch_add(1, Ordering::Relaxed)
        )
    });
    let marker = Marker::new(key, &name).ok_or_else(|| {
        RpcError::new(
            ErrorKind::Conflict,
            format!("The call {} renders already.", p.minutes_id),
            "Wait for it to end, or cancel it.",
        )
    })?;
    let entry = Entry::for_product(name.clone(), &p.product).map_err(|r| match r {
        Refused::NameTaken => RpcError::new(
            ErrorKind::Conflict,
            format!("A render named {name} runs already."),
            "Pick another name, or leave the name out.",
        ),
        Refused::CoreStops => RpcError::new(
            ErrorKind::Unavailable,
            "The core is stopping.",
            "Render the call again when the core runs.",
        ),
    })?;
    let core = ctx.core.clone();
    let (product, minutes_id) = (p.product.clone(), p.minutes_id.clone());
    let done = crate::blocking::spawn(move || {
        let _marker = marker;
        let send =
            |stage: &str, done: u32, of: u32, error: Option<String>, note: Option<String>| {
                core.publish::<MeetingRender>(&MeetingRenderParams {
                    render: entry.name.clone(),
                    product: product.clone(),
                    minutes_id: minutes_id.clone(),
                    stage: stage.into(),
                    done,
                    of,
                    error,
                    note,
                });
            };
        let outcome = (|| {
            let (store, root) = (
                store(&core).map_err(|e| CallError::Unavailable(e.message))?,
                root().map_err(|e| CallError::Unavailable(e.message))?,
            );
            let minutes = store
                .read(|c| Ok(read::minutes(c, &minutes_id)))
                .map_err(CallError::from)?
                .map_err(CallError::from)?
                .filter(|m| m.get("product").and_then(|v| v.as_str()) == Some(product.as_str()))
                .ok_or_else(|| {
                    CallError::NotFound(format!("No minutes {minutes_id} in product {product}."))
                })?;
            if !call::safe_segment(&product) || !call::safe_segment(&minutes_id) {
                return Err(CallError::NotFound(format!(
                    "No minutes {minutes_id} in product {product}."
                )));
            }
            let script = call::script_of(&minutes)?;
            let mut synth = open_voice()?;
            let out = call::audio_dir(&root, &product, &minutes_id);
            let step = |p: Progress| {
                _marker.progress(p.done as u32, p.of as u32);
                send(stage_name(p.stage), p.done as u32, p.of as u32, None, None)
            };
            call::render(&script, &out, &mut synth, &step, &entry.cancel)
        })();
        give_back_memory();
        match &outcome {
            Ok(r) => send(
                "done",
                r.segments as u32,
                r.segments as u32,
                None,
                Some(r.out.display().to_string()),
            ),
            Err(e) => send("done", 0, 0, Some(e.to_string()), None),
        }
        outcome
    })
    .await
    .map_err(|e| RpcError::internal(e.to_string()))?
    .map_err(rpc)?;
    ctx.core.publish::<DataChanged>(&DataChangedParams {
        product: p.product,
        kind: "call".into(),
    });
    Ok(CallRenderResult {
        render: name,
        out: done.out.display().to_string(),
        duration: done.duration,
        bytes: done.bytes,
        segments: done.segments as u32,
    })
}

async fn cancel(_ctx: Ctx, p: CallCancelParams) -> Result<CallCancelResult, RpcError> {
    Ok(CallCancelResult {
        canceled: running::cancel(&p.render),
    })
}
