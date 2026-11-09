//! The desk sessions of the core: at most one `claude` process for each product, its state,
//! the login check and the desk notes. Spec: `docs/specs/agentsi-desktop-conversation.md`.
//!
//! The core never restarts a process by itself. A crash, a hang or a lost session gives a
//! state with a message, and only the next owner message starts a new process.

mod actor;

use crate::state::{Core, lock};
use agentsi_agency::desk::args::{DeskCall, DeskContext, desk_args, owner_text, system_prompt};
use agentsi_agency::desk::auth::{self, AUTH_TIMEOUT, Auth};
use agentsi_agency::registry::Registry;
use agentsi_agency::roles::files::RoleFiles;
use agentsi_agency::roles::load::load_role;
use agentsi_proto::desk_session_api::{
    AuthState, DeskItem, DeskItemEvent, DeskItemParams, DeskState, DeskStateEvent, DeskStateParams,
};
use agentsi_proto::rpc::{ErrorKind, RpcError};
use agentsi_store::{Store, desk};
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, mpsc};
use std::time::Duration;

/// The program of the desk session. The core finds it on its PATH.
pub const CLAUDE: &str = "claude";

/// What the actor of a process gets.
pub(crate) enum Cmd {
    /// One owner message, with its desk notes.
    Send(String),
    Stop,
}

/// The time limits of a desk process.
#[derive(Debug, Clone, Copy)]
pub struct Limits {
    /// With no owner message for this long, the core stops the process.
    pub idle: Duration,
    /// A reply that runs this long stops the process, with an `error` item.
    pub reply: Duration,
}

impl Limits {
    /// 30 min and 10 min. `AGENTSI_DESK_IDLE_SECS` and `AGENTSI_DESK_REPLY_SECS` change them
    /// for a test.
    pub fn from_env() -> Limits {
        let secs = |name: &str, default: u64| {
            std::env::var(name)
                .ok()
                .and_then(|v| v.parse::<u64>().ok())
                .map_or(Duration::from_secs(default), Duration::from_secs)
        };
        Limits {
            idle: secs("AGENTSI_DESK_IDLE_SECS", 30 * 60),
            reply: secs("AGENTSI_DESK_REPLY_SECS", 10 * 60),
        }
    }
}

struct Slot {
    state: DeskState,
    message: Option<String>,
    /// The process that the slot holds now. An actor with another number is stale.
    generation: u64,
    commands: Option<mpsc::Sender<Cmd>>,
    /// A `desk.send` holds the slot, between its check and its delivery.
    sending: bool,
}

impl Default for Slot {
    fn default() -> Slot {
        Slot {
            state: DeskState::Stopped,
            message: None,
            generation: 0,
            commands: None,
            sending: false,
        }
    }
}

#[derive(Default)]
pub struct Desks {
    slots: Mutex<HashMap<String, Slot>>,
    auth: Mutex<Option<Auth>>,
    generations: AtomicU64,
}

/// The actor threads that run.
static RUNNING: AtomicUsize = AtomicUsize::new(0);

/// True when no desk process runs.
pub fn idle() -> bool {
    RUNNING.load(Ordering::SeqCst) == 0
}

impl Desks {
    /// The state of the desk session of a product, and its message.
    pub fn status(&self, product: &str) -> (DeskState, Option<String>) {
        lock(&self.slots)
            .get(product)
            .map_or((DeskState::Stopped, None), |s| (s.state, s.message.clone()))
    }

    /// The last result of the login check, if any ran.
    pub fn auth(&self) -> Option<Auth> {
        lock(&self.auth).clone()
    }

    fn next_generation(&self) -> u64 {
        self.generations.fetch_add(1, Ordering::SeqCst) + 1
    }

    /// True while `generation` is the process of the product.
    fn current(&self, product: &str, generation: u64) -> bool {
        lock(&self.slots)
            .get(product)
            .is_some_and(|s| s.generation == generation)
    }

    /// Sends Stop to each process. The core calls it when it stops.
    pub fn stop_all(&self) {
        for slot in lock(&self.slots).values_mut() {
            if let Some(c) = slot.commands.take() {
                let _ = c.send(Cmd::Stop);
            }
        }
    }
}

/// Sets the state of a product and sends `desk.state` when it changed. With `generation`,
/// a stale actor changes nothing.
fn set_state(
    core: &Core,
    product: &str,
    generation: Option<u64>,
    state: DeskState,
    message: Option<String>,
) {
    let mut slots = lock(&core.desks.slots);
    let slot = slots.entry(product.to_string()).or_default();
    if generation.is_some_and(|g| g != slot.generation) {
        return;
    }
    if slot.state == state && slot.message == message {
        return;
    }
    slot.state = state;
    slot.message.clone_from(&message);
    drop(slots);
    core.publish::<DeskStateEvent>(&DeskStateParams {
        product: product.to_string(),
        state,
        message,
    });
}

/// Runs `claude auth status --json`, keeps the result and gives it.
pub fn check_auth(core: &Core) -> Auth {
    let result = auth::check(CLAUDE, AUTH_TIMEOUT);
    set_auth(core, result.clone());
    result
}

fn set_auth(core: &Core, result: Auth) {
    *lock(&core.desks.auth) = Some(result);
}

fn rpc_store(e: agentsi_store::Error) -> RpcError {
    RpcError::internal(e.to_string())
}

fn no_database() -> RpcError {
    RpcError::new(
        ErrorKind::Unavailable,
        "The data root has no agentsi.db.",
        "Import the data first, or set AGENTSI_HOME to the data root.",
    )
}

pub(crate) fn open_store(core: &Core) -> Result<Arc<Store>, RpcError> {
    core.store().map_err(rpc_store)?.ok_or_else(no_database)
}

pub(crate) fn registry() -> Result<Registry, RpcError> {
    let root = agentsi_store::paths::data_root(None).map_err(rpc_store)?;
    Registry::load(&root).map_err(|e| RpcError::new(ErrorKind::Unavailable, e.to_string(), ""))
}

pub(crate) fn unknown_product(id: &str) -> RpcError {
    RpcError::new(
        ErrorKind::NotFound,
        format!("Unknown product \"{id}\"."),
        "Use product.list for the products.",
    )
}

/// Stores an item of a product and sends `desk.item`. Gives its `seq`.
pub(crate) fn store_item(core: &Core, product: &str, item: DeskItem) -> Result<i64, RpcError> {
    let store = open_store(core)?;
    let body = serde_json::to_string(&item).map_err(|e| RpcError::internal(e.to_string()))?;
    let at = crate::clock::iso_now();
    let (p, kind, stamp) = (product.to_string(), item.kind(), at.clone());
    let seq = store
        .write(move |tx| desk::add_item(tx, &p, kind, &body, &stamp))
        .map_err(rpc_store)?;
    core.publish::<DeskItemEvent>(&DeskItemParams {
        product: product.to_string(),
        seq,
        at,
        item,
    });
    Ok(seq)
}

/// Stores a desk note when the product has a desk session: one line about a write of
/// product data. The next owner message carries it. A failure is only logged, so a note
/// never fails the write of the owner.
pub fn note(core: &Core, product: &str, text: &str) {
    let Ok(Some(store)) = core.store() else {
        return;
    };
    let item = DeskItem::Note {
        text: text.to_string(),
    };
    let body = serde_json::to_string(&item).unwrap_or_default();
    let at = crate::clock::iso_now();
    let (p, stamp) = (product.to_string(), at.clone());
    let stored = store.write(move |tx| {
        if desk::session(tx, &p)?.is_none() {
            return Ok(None);
        }
        desk::add_item(tx, &p, "note", &body, &stamp).map(Some)
    });
    match stored {
        Ok(Some(seq)) => core.publish::<DeskItemEvent>(&DeskItemParams {
            product: product.to_string(),
            seq,
            at,
            item,
        }),
        Ok(None) => {}
        Err(e) => eprintln!("core: desk note of {product} failed: {e}"),
    }
}

/// `AGENTSI_CLI_BIN`, else `agentsi` next to this program, else `agentsi` on the PATH.
fn agentsi_program() -> String {
    if let Some(bin) = std::env::var_os("AGENTSI_CLI_BIN").filter(|v| !v.is_empty()) {
        return bin.to_string_lossy().into_owned();
    }
    std::env::current_exe()
        .ok()
        .and_then(|exe| exe.parent().map(|d| d.join("agentsi")))
        .filter(|p| p.is_file())
        .map_or_else(|| "agentsi".into(), |p| p.to_string_lossy().into_owned())
}

fn socket(core: &Core) -> String {
    core.terminals
        .socket()
        .or_else(|| agentsi_proto::socket::socket_path().ok().map(|s| s.path))
        .unwrap_or_default()
        .to_string_lossy()
        .into_owned()
}

/// What a start of the process needs, apart from the session.
struct Plan {
    call: DeskCall,
    cwd: PathBuf,
}

/// The `pm` agent of the product, with its model, effort and persona, and the folder.
fn plan(core: &Core, product: &str) -> Result<Plan, RpcError> {
    let registry = registry()?;
    let p = registry
        .product(product)
        .map_err(|_| unknown_product(product))?
        .clone();
    let cwd = p.paths.first().map(PathBuf::from).ok_or_else(|| {
        RpcError::new(
            ErrorKind::Unavailable,
            format!("The product \"{product}\" has no folder."),
            "Add a path to the product in the registry.",
        )
    })?;
    let store = open_store(core)?;
    let id = p.id.clone();
    let overlays = store
        .read(move |c| {
            agentsi_agency::roster::read_overlays(c, &id)
                .map_err(|e| agentsi_store::Error::Other(e.to_string()))
        })
        .map_err(rpc_store)?;
    let root = agentsi_store::paths::data_root(None).map_err(rpc_store)?;
    let def = load_role(
        "pm",
        product,
        &RoleFiles::at(&root),
        &overlays,
        &mut Vec::new(),
    )
    .map_err(RpcError::internal)?;
    let cwd_text = cwd.to_string_lossy().into_owned();
    let system = system_prompt(
        &def.body,
        &DeskContext {
            product: &p.name,
            owner: &registry.owner.name,
            language: &registry.owner.language,
            cwd: &cwd_text,
        },
    );
    Ok(Plan {
        call: DeskCall {
            model: def.model,
            effort: def.effort,
            system,
            agentsi: agentsi_program(),
            socket: socket(core),
            session: String::new(),
            resume: false,
        },
        cwd,
    })
}

fn busy() -> RpcError {
    RpcError::new(
        ErrorKind::Busy,
        "A reply of the desk session runs.",
        "Wait for its end, or call desk.stop.",
    )
}

/// The login must be `ready`. A state that is not `ready` is checked again first.
fn require_auth(core: &Core, product: &str) -> Result<(), RpcError> {
    let known = core.desks.auth();
    let result = match known {
        Some(a) if a.state == AuthState::Ready => a,
        _ => check_auth(core),
    };
    if result.state == AuthState::Ready {
        return Ok(());
    }
    let message = result
        .message
        .unwrap_or_else(|| "The login of Claude Code is not usable.".into());
    set_state(core, product, None, DeskState::Auth, Some(message.clone()));
    Err(RpcError::new(
        ErrorKind::Unavailable,
        message,
        "Fix the login of Claude Code, then call desk.auth with recheck.",
    ))
}

/// `desk.send`: stores the owner message and gives it to the process of the product. It
/// starts the process when none runs. It blocks: call it off the event loop.
pub fn send(core: &Arc<Core>, product: &str, text: &str) -> Result<i64, RpcError> {
    let text = text.trim();
    if text.is_empty() {
        return Err(RpcError::invalid_params("The message is empty."));
    }
    let plan = plan(core, product)?;
    require_auth(core, product)?;
    {
        let mut slots = lock(&core.desks.slots);
        let slot = slots.entry(product.to_string()).or_default();
        if slot.sending || matches!(slot.state, DeskState::Starting | DeskState::Working) {
            return Err(busy());
        }
        slot.sending = true;
    }
    let stored = record_owner(core, product, text);
    let mut slots = lock(&core.desks.slots);
    let slot = slots.entry(product.to_string()).or_default();
    slot.sending = false;
    let (seq, session, resume, notes) = stored?;
    let message = owner_text(&notes, text);
    if let Some(c) = &slot.commands
        && c.send(Cmd::Send(message.clone())).is_ok()
    {
        drop(slots);
        set_state(core, product, None, DeskState::Working, None);
        return Ok(seq);
    }
    let generation = core.desks.next_generation();
    let (commands, inbox) = mpsc::channel();
    slot.generation = generation;
    slot.commands = Some(commands);
    drop(slots);
    set_state(core, product, None, DeskState::Starting, None);
    let mut call = plan.call;
    call.session = session;
    call.resume = resume;
    actor::spawn(
        core.clone(),
        actor::Start {
            product: product.to_string(),
            generation,
            args: desk_args(&call),
            cwd: plan.cwd,
            first: message,
            inbox,
            limits: Limits::from_env(),
        },
    );
    Ok(seq)
}

/// Stores the owner item and marks the unsent notes sent, in one transaction. Makes the
/// session when the product has none. Gives the seq, the session, whether to resume it, and
/// the texts of the notes.
fn record_owner(
    core: &Core,
    product: &str,
    text: &str,
) -> Result<(i64, String, bool, Vec<String>), RpcError> {
    let store = open_store(core)?;
    let fresh =
        agentsi_agency::desk::new_session_id().map_err(|e| RpcError::internal(e.to_string()))?;
    let item = DeskItem::Owner {
        text: text.to_string(),
    };
    let body = serde_json::to_string(&item).map_err(|e| RpcError::internal(e.to_string()))?;
    let at = crate::clock::iso_now();
    let (p, stamp) = (product.to_string(), at.clone());
    let (seq, session, resume, notes) = store
        .write(move |tx| {
            let session = match desk::session(tx, &p)? {
                Some(s) => s,
                None => {
                    desk::set_session(tx, &p, &fresh, &stamp)?;
                    fresh
                }
            };
            // Claude Code saves the session at the end of a reply. Before one, a resume
            // would find no session.
            let resume = desk::has_kind(tx, &p, "end")?;
            let notes = desk::take_notes(tx, &p)?;
            let seq = desk::add_item(tx, &p, "owner", &body, &stamp)?;
            Ok((seq, session, resume, notes))
        })
        .map_err(rpc_store)?;
    core.publish::<DeskItemEvent>(&DeskItemParams {
        product: product.to_string(),
        seq,
        at,
        item,
    });
    let notes = notes
        .iter()
        .filter_map(|body| match serde_json::from_str(body) {
            Ok(DeskItem::Note { text }) => Some(text),
            _ => None,
        })
        .collect();
    Ok((seq, session, resume, notes))
}

/// `desk.stop`: stops the process of a product. False when none ran.
pub fn stop(core: &Core, product: &str) -> bool {
    let commands = {
        let mut slots = lock(&core.desks.slots);
        let Some(slot) = slots.get_mut(product) else {
            return false;
        };
        let commands = slot.commands.take();
        if commands.is_some() {
            slot.generation = core.desks.next_generation();
        }
        commands
    };
    let Some(commands) = commands else {
        return false;
    };
    let _ = commands.send(Cmd::Stop);
    set_state(core, product, None, DeskState::Stopped, None);
    true
}

/// `desk.reset`: stops the process, deletes the items and gives the product a new session.
pub fn reset(core: &Core, product: &str) -> Result<String, RpcError> {
    registry()?
        .product(product)
        .map_err(|_| unknown_product(product))?;
    stop(core, product);
    let store = open_store(core)?;
    let session =
        agentsi_agency::desk::new_session_id().map_err(|e| RpcError::internal(e.to_string()))?;
    let (p, s, at) = (
        product.to_string(),
        session.clone(),
        crate::clock::iso_now(),
    );
    store
        .write(move |tx| desk::reset(tx, &p, &s, &at))
        .map_err(rpc_store)?;
    set_state(core, product, None, DeskState::Stopped, None);
    Ok(session)
}
