//! The state of one core process, and the event queue of each subscriber.

use crate::methods::models::Models;
use crate::methods::{Router, router};
use crate::terminal::Terminals;
use agentsi_proto::Event;
use agentsi_proto::core_api::Topic;
use agentsi_proto::events::{DictationTarget, DictationTargetParams, Lagged, LaggedParams};
use agentsi_proto::rpc::Notification;
use agentsi_store::Store;
use std::collections::{BTreeSet, HashMap, VecDeque};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, Weak};
use tokio::sync::Notify;

/// The events that wait for one connection. Past this number, the core
/// drops the events of that connection and sends `core.lagged`.
pub const EVENT_QUEUE: usize = 1024;

pub(crate) fn lock<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    m.lock().unwrap_or_else(|p| p.into_inner())
}

fn line<E: Event>(params: &E::Params) -> Arc<str> {
    serde_json::to_string(&Notification::of::<E>(params))
        .unwrap_or_default()
        .into()
}

#[derive(Default)]
struct Queue {
    messages: VecDeque<Arc<str>>,
    /// The events that the core dropped since the last message in the queue.
    missed: u64,
}

/// The topics and the event queue of one connection.
#[derive(Default)]
pub struct Subscriber {
    topics: Mutex<BTreeSet<Topic>>,
    queue: Mutex<Queue>,
    ready: Notify,
}

impl Subscriber {
    /// Adds topics and gives all the topics of the subscriber.
    pub fn add_topics(&self, topics: &[Topic]) -> Vec<Topic> {
        let mut all = lock(&self.topics);
        all.extend(topics.iter().copied());
        all.iter().copied().collect()
    }

    fn wants(&self, topic: Option<Topic>) -> bool {
        topic.is_none_or(|t| lock(&self.topics).contains(&t))
    }

    fn push(&self, message: &Arc<str>) {
        let mut q = lock(&self.queue);
        if q.messages.len() >= EVENT_QUEUE {
            q.missed += 1;
            return;
        }
        // The notice goes where the events are missing: after the old
        // events and before the new one.
        if q.missed > 0 {
            let missed = std::mem::take(&mut q.missed);
            q.messages
                .push_back(line::<Lagged>(&LaggedParams { missed }));
        }
        q.messages.push_back(message.clone());
        drop(q);
        self.ready.notify_one();
    }

    /// The next notification. It waits until one comes.
    pub async fn next(&self) -> Arc<str> {
        loop {
            {
                let mut q = lock(&self.queue);
                if let Some(m) = q.messages.pop_front() {
                    return m;
                }
                if q.missed > 0 {
                    let missed = std::mem::take(&mut q.missed);
                    return line::<Lagged>(&LaggedParams { missed });
                }
            }
            self.ready.notified().await;
        }
    }
}

pub struct Core {
    pub router: Router,
    pub terminals: Terminals,
    /// The voice models and their downloads.
    pub models: Models,
    /// The database of the data root, opened at its first use.
    store: Mutex<Option<Arc<Store>>>,
    /// The terminal of the focused pane of the app, from `focus.report`.
    focus: Mutex<Option<String>>,
    /// A dictation runs: the last `voice.dictate` start has no stop yet. `toggle` reads it.
    dictating: AtomicBool,
    /// The desk session of each product.
    pub desks: crate::desk::Desks,
    /// The conversation mode that runs, if any.
    pub(crate) converse: Mutex<Option<crate::converse::Active>>,
    /// The prompt of the running turn of each session outside the app that has voice mode on.
    session_prompts: Mutex<HashMap<String, String>>,
    subscribers: Mutex<Vec<Weak<Subscriber>>>,
    stop: Notify,
    closing: AtomicBool,
    closed: Notify,
    links: AtomicUsize,
}

impl Core {
    pub fn new() -> Arc<Core> {
        Core::with_models(Models::standard())
    }

    /// A core with its own catalog, models folder and download server. Tests use it.
    pub fn with_models(models: Models) -> Arc<Core> {
        Arc::new(Core {
            router: router(),
            terminals: Terminals::default(),
            models,
            store: Mutex::default(),
            focus: Mutex::default(),
            dictating: AtomicBool::new(false),
            desks: crate::desk::Desks::default(),
            converse: Mutex::default(),
            session_prompts: Mutex::default(),
            subscribers: Mutex::default(),
            stop: Notify::new(),
            closing: AtomicBool::new(false),
            closed: Notify::new(),
            links: AtomicUsize::new(0),
        })
    }

    /// A new subscriber with no topics. It gets events while the caller keeps it.
    pub fn subscriber(&self) -> Arc<Subscriber> {
        let s = Arc::new(Subscriber::default());
        lock(&self.subscribers).push(Arc::downgrade(&s));
        s
    }

    /// Sends an event to each subscriber of its topic. A `core` event goes
    /// to each subscriber.
    pub fn publish<E: Event>(&self, params: &E::Params) {
        let topic = Topic::of_event(E::NAME);
        let message = line::<E>(params);
        let mut subscribers = lock(&self.subscribers);
        subscribers.retain(|weak| match weak.upgrade() {
            Some(s) => {
                if s.wants(topic) {
                    s.push(&message);
                }
                true
            }
            None => false,
        });
    }

    /// `framed`, and the namespace of each method.
    pub fn features(&self) -> Vec<String> {
        let mut features: Vec<String> = self
            .router
            .names()
            .filter_map(|n| n.split('.').next())
            .map(String::from)
            .collect();
        features.push("framed".into());
        features.sort();
        features.dedup();
        features
    }

    /// The store of the data root. `Ok(None)` while the database file does not
    /// exist: the core never makes it, so `import-postgres --fresh` can.
    /// It blocks: call it off the event loop.
    pub fn store(&self) -> agentsi_store::Result<Option<Arc<Store>>> {
        let mut store = lock(&self.store);
        if let Some(s) = &*store {
            return Ok(Some(s.clone()));
        }
        let path = agentsi_store::paths::db_path(&agentsi_store::paths::data_root(None)?);
        if !path.exists() {
            return Ok(None);
        }
        let opened = Arc::new(Store::open(&path)?);
        *store = Some(opened.clone());
        Ok(Some(opened))
    }

    /// Keeps the prompt of the turn that a session with voice mode on began.
    pub fn set_session_prompt(&self, session: &str, prompt: String) {
        let mut prompts = lock(&self.session_prompts);
        // A session that never reaches Stop leaves its prompt. The map stays small.
        if prompts.len() >= 256 {
            prompts.clear();
        }
        prompts.insert(session.to_string(), prompt);
    }

    /// The prompt of the turn that ends, or `None` when voice mode went on after the turn
    /// began. An empty string stays behind, so a forced second Stop of the turn is saved.
    pub fn take_session_prompt(&self, session: &str) -> Option<String> {
        lock(&self.session_prompts).insert(session.to_string(), String::new())
    }

    /// Forgets the prompt. Voice mode went on or off, so the turn in progress is not a turn of it.
    pub fn forget_session_prompt(&self, session: &str) {
        lock(&self.session_prompts).remove(session);
    }

    /// The terminal of the focused pane, from the last `focus.report`.
    pub fn focus(&self) -> Option<String> {
        lock(&self.focus).clone()
    }

    /// Whether a dictation runs, as far as the core sent `dictation.start` and `dictation.stop`.
    pub fn dictating(&self) -> bool {
        self.dictating.load(Ordering::SeqCst)
    }

    /// Records a dictation start or stop, and gives the state before.
    pub fn set_dictating(&self, on: bool) -> bool {
        self.dictating.swap(on, Ordering::SeqCst)
    }

    /// True while a connection is subscribed to `topic`.
    pub fn has_subscriber(&self, topic: Topic) -> bool {
        lock(&self.subscribers)
            .iter()
            .filter_map(Weak::upgrade)
            .any(|s| s.wants(Some(topic)))
    }

    /// Moves the dictation target to `terminal`, and sends `dictation.target` on a change.
    pub fn set_focus(&self, terminal: Option<String>) {
        let mut focus = lock(&self.focus);
        if *focus == terminal {
            return;
        }
        focus.clone_from(&terminal);
        drop(focus);
        self.publish::<DictationTarget>(&DictationTargetParams { terminal });
    }

    /// Clears the dictation target when it is `terminal`.
    pub fn forget_focus(&self, terminal: &str) {
        let is_target = lock(&self.focus).as_deref() == Some(terminal);
        if is_target {
            self.set_focus(None);
        }
    }

    pub fn ask_stop(&self) {
        self.stop.notify_one();
    }

    pub async fn stopped(&self) {
        self.stop.notified().await;
    }

    /// Closes each client link. A core that stops calls it first, so a client learns at once
    /// and does not wait for the work that still ends.
    pub fn close_links(&self) {
        self.closing.store(true, Ordering::SeqCst);
        self.closed.notify_waiters();
    }

    pub async fn links_closing(&self) {
        loop {
            let notified = self.closed.notified();
            if self.closing.load(Ordering::SeqCst) {
                return;
            }
            notified.await;
        }
    }

    /// Counts one client link while the guard lives.
    pub fn open_link(self: &Arc<Self>) -> LinkGuard {
        self.links.fetch_add(1, Ordering::SeqCst);
        LinkGuard(self.clone())
    }

    /// Waits until each client link has ended, or until `limit` is over. A core that stops
    /// calls it, so the replies of the requests in flight reach their clients before it exits.
    pub async fn links_ended(&self, limit: std::time::Duration) {
        let deadline = tokio::time::Instant::now() + limit;
        while self.links.load(Ordering::SeqCst) > 0 && tokio::time::Instant::now() < deadline {
            tokio::time::sleep(std::time::Duration::from_millis(5)).await;
        }
    }
}

pub struct LinkGuard(Arc<Core>);

impl Drop for LinkGuard {
    fn drop(&mut self) {
        self.0.links.fetch_sub(1, Ordering::SeqCst);
    }
}
