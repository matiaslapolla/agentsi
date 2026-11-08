//! The settings screen: the voice models with download and delete, the voice of each
//! role, the vocabulary, the keys of the app and the SSH hosts. It belongs to no product.
//! The owner edits the hosts in `hosts.toml`. The keys are saved in `keys.toml` (see `keys.rs`).
//!
//! The `models.*` events of the core reach the screen through the `ModelFeed` entity. They
//! skip `CoreState`, so a download redraws this screen only.

use std::collections::BTreeMap;
use std::time::Duration;

use agentsi_proto::Event as _;
use agentsi_proto::core_api::Empty;
use agentsi_proto::events::{Lagged, ModelsDone, ModelsError, ModelsProgress};
use agentsi_proto::models_api::{
    ModelInfo, ModelParams, ModelState, ModelsDelete, ModelsDownload, ModelsList, ModelsListResult,
    ModelsVoices, VoicesResult,
};
use agentsi_proto::rpc::Notification;
use agentsi_proto::vocabulary_api::{
    VocabularyEntry, VocabularyGet, VocabularyList, VocabularySet, VocabularySetParams,
};
use agentsi_ui::components::{Button, Card, neutral_tag, status_tag};
use agentsi_ui::tokens::{Status, accent, font, line, radius, size, space, surface, text, weight};
use gpui_kit::component::IconName;
use gpui_kit::component::input::{Input, InputEvent, InputState};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;

use super::git_config::GitConfigEditor;
use super::parts::{Load, column, mono, notice, section};
use crate::hosts::{Hosts, HostsGlobal, RemoteHost};
use crate::keys::{self, Captured};

mod appearance;
mod data;
mod dictation;
mod git;
mod guide;
use crate::model::{CoreState, HostState};
pub use dictation::DictationGlobal;
use dictation::DictationKey;

/// Two downloads at once send up to 20 events each second. The screen draws at most 10 times.
const REDRAW_EVERY: Duration = Duration::from_millis(100);

/// Carries the `models.*` events from the shell to the settings screens.
pub struct ModelFeed;

impl EventEmitter<Notification> for ModelFeed {}

/// The feed of this window, set by the shell.
pub struct ModelFeedGlobal(pub Entity<ModelFeed>);

impl Global for ModelFeedGlobal {}

/// Takes one `models.*` event into the rows. Returns false when it changed nothing.
pub fn apply(models: &mut [ModelInfo], n: &Notification) -> bool {
    let (id, state) = if let Some(p) = n.parse::<ModelsProgress>() {
        let state = ModelState::Downloading {
            bytes: p.bytes,
            total: p.total,
            verifying: p.verifying,
        };
        (p.id, state)
    } else if let Some(p) = n.parse::<ModelsDone>() {
        (p.id, ModelState::Downloaded)
    } else if let Some(p) = n.parse::<ModelsError>() {
        (p.id.clone(), ModelState::from(&p))
    } else {
        return false;
    };
    match models.iter_mut().find(|m| m.id == id) {
        Some(m) if m.state != state => {
            m.state = state;
            true
        }
        _ => false,
    }
}

/// Takes the events that arrived while the list was read. They are newer than the list
/// may be, so they come after it, in their order.
pub fn apply_all(models: &mut [ModelInfo], events: &[Notification]) {
    for n in events {
        apply(models, n);
    }
}

/// `1.2 GB`, `310.4 MB`, `12.0 KB`. Decimal units, as the download sizes of Hugging Face.
pub fn bytes(n: u64) -> String {
    const UNITS: [(u64, &str); 3] = [(1_000_000_000, "GB"), (1_000_000, "MB"), (1_000, "KB")];
    for (unit, name) in UNITS {
        if n >= unit {
            return format!("{:.1} {name}", n as f64 / unit as f64);
        }
    }
    format!("{n} B")
}

/// The tag and the detail line of a state.
fn state_view(m: &ModelInfo) -> (gpui_kit::component::tag::Tag, Option<String>) {
    match &m.state {
        ModelState::Missing => (neutral_tag("not downloaded"), None),
        ModelState::Partial { bytes: b } => (
            status_tag("partial", Status::Info),
            Some(format!(
                "{} of {} on disk. Download continues from there.",
                bytes(*b),
                bytes(m.size)
            )),
        ),
        ModelState::Downloading {
            bytes: b,
            total,
            verifying,
        } => (
            status_tag("downloading", Status::Info),
            Some(if *verifying {
                format!("{} of {}. Checking the hash…", bytes(*b), bytes(*total))
            } else {
                format!("{} of {}", bytes(*b), bytes(*total))
            }),
        ),
        ModelState::Downloaded => (status_tag("downloaded", Status::Ok), None),
        ModelState::Conflict { message, .. } => (
            status_tag("conflict", Status::Warn),
            Some(format!(
                "{message}. Move that file away, then download again. Delete removes only the files that AgentSI wrote."
            )),
        ),
        ModelState::Error { error, message } => (
            status_tag(error.replace('_', " "), Status::Danger),
            Some(message.clone()),
        ),
    }
}

/// The forms in a "heard as" field: separated by commas or lines.
pub fn parse_heard(text: &str) -> Vec<String> {
    text.split([',', '\n'])
        .map(|h| h.split_whitespace().collect::<Vec<_>>().join(" "))
        .filter(|h| !h.is_empty())
        .collect()
}

/// The list after the owner adds an entry, or replaces entry `editing`. An entry with
/// the term of another entry (ignoring case) joins that entry.
pub fn with_entry(
    entries: &[VocabularyEntry],
    editing: Option<usize>,
    entry: VocabularyEntry,
) -> Vec<VocabularyEntry> {
    let mut out = entries.to_vec();
    match editing.filter(|i| *i < out.len()) {
        Some(i) => out[i] = entry,
        None => out.push(entry),
    }
    out
}

/// The text fields of the vocabulary form.
struct VocabForm {
    term: Entity<InputState>,
    heard: Entity<InputState>,
}

/// A key capture and the ties that end it. The capture ends when this drops: the owner leaves
/// the screen or the window, or the screen loses the focus.
struct CaptureGuard {
    _ties: Vec<Subscription>,
}

/// The row of the Keys section that waits for a key press.
struct Capturing {
    id: &'static str,
    _guard: CaptureGuard,
}

pub struct SettingsScreen {
    core: Entity<CoreState>,
    /// Takes the focus while a key row waits, so no field gets the key.
    focus: FocusHandle,
    /// The filter field of the Keys section. The first render makes it.
    keys_filter: Option<Entity<InputState>>,
    capture: Option<Capturing>,
    /// The dictation key section. None where the app has no `DictationEnv`: macOS.
    dictation: Option<DictationKey>,
    /// The Data section: what the owner selected to delete, and the plan.
    clear: data::ClearState,
    /// The error of the last save of the Appearance section.
    appearance_error: Option<String>,
    /// The shared table of the git config file: the section "Git".
    git: Entity<GitConfigEditor>,
    /// The refusal of the last change, with the row it belongs to.
    keys_note: Option<(&'static str, String)>,
    models: Load<ModelsListResult>,
    voices: Load<VoicesResult>,
    vocab: Load<VocabularyList>,
    /// The form needs a window, so the first render makes it.
    vocab_form: Option<VocabForm>,
    /// The entry that the form replaces, or none for a new entry.
    editing: Option<usize>,
    /// The line under the form: a refused save.
    vocab_note: Option<String>,
    _inputs: Vec<Subscription>,
    /// The error of the last call of a row, by model id: a refused delete, for example.
    refused: BTreeMap<String, String>,
    scroll: ScrollHandle,
    _feed: Option<Subscription>,
    hosts: Option<Entity<Hosts>>,
    _hosts: Option<Subscription>,
    /// The draw that the events since the last draw wait for.
    redraw: Option<Task<()>>,
    /// The events that arrive while `models.list` runs. They apply to its answer, which
    /// may be older than they are.
    pending: Option<Vec<Notification>>,
}

/// The vertical offset after a key that scrolls the page, or `None` when the key does not scroll.
/// An offset is zero at the top and `-max` at the bottom.
fn scroll_after(key: &str, now: Pixels, page: Pixels, max: Pixels) -> Option<Pixels> {
    let to = match key {
        "pagedown" => now - page,
        "pageup" => now + page,
        "home" => px(0.),
        "end" => -max,
        _ => return None,
    };
    Some(to.max(-max).min(px(0.)))
}

impl SettingsScreen {
    /// The page takes the focus, so the page keys reach it.
    pub fn focus_handle(&self) -> FocusHandle {
        self.focus.clone()
    }

    /// Page Up, Page Down, Home and End scroll the page: the screen has no other way to the
    /// lower sections for a keyboard.
    fn scroll_key(&mut self, e: &KeyDownEvent, cx: &mut Context<Self>) {
        let k = &e.keystroke;
        if k.modifiers.modified() {
            return;
        }
        let now = self.scroll.offset();
        let to = scroll_after(
            k.key.as_str(),
            now.y,
            self.scroll.bounds().size.height,
            self.scroll.max_offset().y,
        );
        let Some(y) = to else {
            return;
        };
        self.scroll.set_offset(point(now.x, y));
        cx.stop_propagation();
        cx.notify();
    }

    pub fn new(core: Entity<CoreState>, cx: &mut Context<Self>) -> Self {
        let feed = cx.try_global::<ModelFeedGlobal>().map(|f| f.0.clone());
        let hosts = cx.try_global::<HostsGlobal>().map(|h| h.0.clone());
        let git = {
            let core = core.clone();
            cx.new(|cx| GitConfigEditor::new(None, core, cx))
        };
        let mut screen = SettingsScreen {
            core,
            git,
            focus: cx.focus_handle(),
            keys_filter: None,
            capture: None,
            dictation: cx
                .try_global::<DictationGlobal>()
                .cloned()
                .map(DictationKey::new),
            clear: data::ClearState::default(),
            appearance_error: None,
            keys_note: None,
            models: Load::Loading,
            voices: Load::Loading,
            vocab: Load::Loading,
            vocab_form: None,
            editing: None,
            vocab_note: None,
            _inputs: Vec::new(),
            refused: BTreeMap::new(),
            scroll: ScrollHandle::new(),
            _feed: feed
                .map(|f| cx.subscribe(&f, |this, _, n: &Notification, cx| this.on_event(n, cx))),
            _hosts: hosts
                .as_ref()
                .map(|h| cx.observe(h, |_, _, cx| cx.notify())),
            hosts,
            redraw: None,
            pending: None,
        };
        screen.reload(cx);
        screen.load_root(cx);
        screen.refresh_dictation(cx);
        screen
    }

    fn on_event(&mut self, n: &Notification, cx: &mut Context<Self>) {
        if n.method == Lagged::NAME {
            self.reload(cx);
            return;
        }
        if let Some(pending) = &mut self.pending {
            pending.push(n.clone());
            return;
        }
        if let Load::Ready(list) = &mut self.models
            && apply(&mut list.models, n)
            && self.redraw.is_none()
        {
            self.redraw = Some(cx.spawn(async move |this, cx| {
                cx.background_executor().timer(REDRAW_EVERY).await;
                let _ = this.update(cx, |this, cx| {
                    this.redraw = None;
                    cx.notify();
                });
            }));
        }
    }

    /// Reads the models and the voices again.
    pub fn reload(&mut self, cx: &mut Context<Self>) {
        let link = self.core.read(cx).link.clone();
        let models = link.call::<ModelsList>(&Empty {});
        let voices = link.call::<ModelsVoices>(&Empty {});
        let vocab = link.call::<VocabularyGet>(&Empty {});
        self.pending.get_or_insert_default();
        self.git.update(cx, |g, cx| g.reload(cx));
        cx.spawn(async move |this, cx| {
            let vocab = vocab.await.map_err(|e| e.to_string());
            let _ = this.update(cx, |this, cx| {
                this.vocab.settle(vocab);
                cx.notify();
            });
        })
        .detach();
        cx.spawn(async move |this, cx| {
            let models = models.await.map_err(|e| e.to_string());
            let _ = this.update(cx, |this, cx| {
                this.models.settle(models);
                let pending = this.pending.take().unwrap_or_default();
                if let Load::Ready(list) = &mut this.models {
                    apply_all(&mut list.models, &pending);
                }
                this.refused.clear();
                cx.notify();
            });
        })
        .detach();
        cx.spawn(async move |this, cx| {
            let voices = voices.await.map_err(|e| e.to_string());
            let _ = this.update(cx, |this, cx| {
                this.voices.settle(voices);
                cx.notify();
            });
        })
        .detach();
    }

    fn set_state(&mut self, id: &str, state: ModelState) {
        if let Load::Ready(list) = &mut self.models
            && let Some(m) = list.models.iter_mut().find(|m| m.id == id)
        {
            m.state = state;
        }
    }

    fn download(&mut self, id: String, cx: &mut Context<Self>) {
        let call = self
            .core
            .read(cx)
            .link
            .call::<ModelsDownload>(&ModelParams { id: id.clone() });
        self.refused.remove(&id);
        cx.spawn(async move |this, cx| {
            let result = call.await;
            let _ = this.update(cx, |this, cx| {
                match result {
                    // The events give the state. The first one comes at once, and it may
                    // come before this reply.
                    Ok(r) if r.started => {}
                    // It runs already, or it is downloaded: the list tells which.
                    Ok(_) => this.reload(cx),
                    Err(e) => {
                        this.refused.insert(id, e.to_string());
                    }
                }
                cx.notify();
            });
        })
        .detach();
    }

    fn delete(&mut self, id: String, cx: &mut Context<Self>) {
        let call = self
            .core
            .read(cx)
            .link
            .call::<ModelsDelete>(&ModelParams { id: id.clone() });
        self.refused.remove(&id);
        cx.spawn(async move |this, cx| {
            let result = call.await;
            let _ = this.update(cx, |this, cx| {
                match result {
                    Ok(info) => this.set_state(&id, info.state),
                    Err(e) => {
                        this.refused.insert(id, e.to_string());
                    }
                }
                cx.notify();
            });
        })
        .detach();
    }

    fn vocab_form(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> (Entity<InputState>, Entity<InputState>) {
        if let Some(f) = &self.vocab_form {
            return (f.term.clone(), f.heard.clone());
        }
        let term =
            cx.new(|cx| InputState::new(window, cx).placeholder("Term, for example AgentSI"));
        let heard = cx.new(|cx| {
            InputState::new(window, cx)
                .placeholder("Heard as, separated by commas: Agent C, Agent SI")
        });
        for input in [&term, &heard] {
            self._inputs.push(cx.subscribe_in(
                input,
                window,
                |this, _, e: &InputEvent, window, cx| {
                    if let InputEvent::PressEnter { .. } = e {
                        this.save_form(window, cx);
                    }
                },
            ));
        }
        self.vocab_form = Some(VocabForm {
            term: term.clone(),
            heard: heard.clone(),
        });
        (term, heard)
    }

    /// Sends the whole list to the core. The core answers with the cleaned list.
    fn save_entries(
        &mut self,
        entries: Vec<VocabularyEntry>,
        reset_form: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let call = self
            .core
            .read(cx)
            .link
            .call::<VocabularySet>(&VocabularySetParams { entries });
        self.vocab_note = None;
        cx.spawn_in(window, async move |this, cx| {
            let result = call.await;
            let _ = this.update_in(cx, |this, window, cx| {
                match result {
                    Ok(list) => {
                        this.vocab = Load::Ready(list);
                        if reset_form {
                            this.reset_form(window, cx);
                        }
                    }
                    Err(e) => this.vocab_note = Some(format!("The vocabulary was not saved: {e}")),
                }
                cx.notify();
            });
        })
        .detach();
    }

    fn reset_form(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.editing = None;
        if let Some(f) = &self.vocab_form {
            f.term.update(cx, |i, cx| i.set_value("", window, cx));
            f.heard.update(cx, |i, cx| i.set_value("", window, cx));
        }
    }

    fn save_form(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let (Some(form), Load::Ready(list)) = (&self.vocab_form, &self.vocab) else {
            return;
        };
        let term = form.term.read(cx).value().trim().to_string();
        if term.is_empty() {
            self.vocab_note = Some("Write the term first.".into());
            cx.notify();
            return;
        }
        let heard = parse_heard(form.heard.read(cx).value().as_ref());
        let entries = with_entry(&list.entries, self.editing, VocabularyEntry { term, heard });
        self.save_entries(entries, true, window, cx);
    }

    fn edit_entry(&mut self, i: usize, window: &mut Window, cx: &mut Context<Self>) {
        let (Some(form), Load::Ready(list)) = (&self.vocab_form, &self.vocab) else {
            return;
        };
        let Some(entry) = list.entries.get(i) else {
            return;
        };
        let (term, heard) = (entry.term.clone(), entry.heard.join(", "));
        form.term
            .update(cx, |input, cx| input.set_value(term, window, cx));
        form.heard
            .update(cx, |input, cx| input.set_value(heard, window, cx));
        self.editing = Some(i);
        self.vocab_note = None;
        cx.notify();
    }

    fn delete_entry(&mut self, i: usize, window: &mut Window, cx: &mut Context<Self>) {
        let Load::Ready(list) = &self.vocab else {
            return;
        };
        let mut entries = list.entries.clone();
        if i < entries.len() {
            entries.remove(i);
        }
        // A row removed below the one in the form moves the form's row up by one.
        match self.editing {
            Some(e) if e == i => self.reset_form(window, cx),
            Some(e) if e > i => self.editing = Some(e - 1),
            _ => {}
        }
        self.save_entries(entries, false, window, cx);
    }

    fn vocabulary_section(&mut self, window: &mut Window, cx: &mut Context<Self>) -> Div {
        let (term, heard) = self.vocab_form(window, cx);
        let rows: Vec<Div> = match &self.vocab {
            Load::Loading => vec![div().child(mono("Reading the vocabulary…"))],
            Load::Failed(e) => vec![
                div()
                    .text_size(size::sm())
                    .text_color(Status::Danger.color())
                    .child(format!("The vocabulary was not read: {e}")),
            ],
            Load::Ready(list) => list
                .entries
                .iter()
                .enumerate()
                .map(|(i, e)| {
                    pair(
                        e.term.clone(),
                        div()
                            .flex()
                            .items_center()
                            .gap(space(2))
                            .child(mono(if e.heard.is_empty() {
                                "no heard forms".to_string()
                            } else {
                                e.heard.join(" · ")
                            }))
                            .child(
                                Button::new(("vocab-edit", i))
                                    .secondary()
                                    .small()
                                    .label("Edit")
                                    .on_click(cx.listener(move |this, _, window, cx| {
                                        this.edit_entry(i, window, cx)
                                    })),
                            )
                            .child(
                                Button::new(("vocab-delete", i))
                                    .secondary()
                                    .small()
                                    .icon(IconName::Delete)
                                    .label("Delete")
                                    .on_click(cx.listener(move |this, _, window, cx| {
                                        this.delete_entry(i, window, cx)
                                    })),
                            ),
                    )
                })
                .collect(),
        };
        let path = match &self.vocab {
            Load::Ready(list) => Some(list.path.clone()),
            _ => None,
        };
        let editing = self.editing.is_some();
        div()
            .flex()
            .flex_col()
            .gap(space(3))
            .child(section(
                "Vocabulary",
                match &self.vocab {
                    Load::Ready(list) => Some(list.entries.len()),
                    _ => None,
                },
            ))
            .child(
                div()
                    .text_size(size::sm())
                    .text_color(text::SECONDARY)
                    .child("Speech models mishear names. When a dictation has a heard form, the app types the term instead. Whole words only, any case."),
            )
            .children(rows)
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap(space(2))
                    .child(div().w(space(45)).child(Input::new(&term)))
                    .child(div().flex_1().min_w_0().child(Input::new(&heard)))
                    .child(
                        Button::new("vocab-save")
                            .primary()
                            .small()
                            .label(if editing { "Update" } else { "Add" })
                            .on_click(cx.listener(|this, _, window, cx| this.save_form(window, cx))),
                    )
                    .when(editing, |el| {
                        el.child(
                            Button::new("vocab-cancel")
                                .secondary()
                                .small()
                                .label("Cancel")
                                .on_click(cx.listener(|this, _, window, cx| {
                                    this.reset_form(window, cx);
                                    cx.notify();
                                })),
                        )
                    }),
            )
            .when_some(self.vocab_note.clone(), |el, n| {
                el.child(
                    div()
                        .text_size(size::sm())
                        .text_color(Status::Danger.color())
                        .child(n),
                )
            })
            .when_some(path, |el, p| el.child(mono(format!("{p} · saved when you press Add or Delete"))))
    }

    fn filter_input(&mut self, window: &mut Window, cx: &mut Context<Self>) -> Entity<InputState> {
        if let Some(f) = &self.keys_filter {
            return f.clone();
        }
        let input = cx.new(|cx| InputState::new(window, cx).placeholder("Filter the keys"));
        self._inputs.push(
            cx.subscribe_in(&input, window, |_, _, e: &InputEvent, _, cx| {
                if let InputEvent::Change = e {
                    cx.notify();
                }
            }),
        );
        self.keys_filter = Some(input.clone());
        input
    }

    /// Waits for the next key press, for action `id`. No other action runs meanwhile.
    fn begin_change(&mut self, id: &'static str, window: &mut Window, cx: &mut Context<Self>) {
        self.keys_note = None;
        self.cancel_captures(cx);
        let guard = self.start_capture(window, cx, |this, captured, cx| {
            this.on_captured(captured, cx)
        });
        self.capture = Some(Capturing { id, _guard: guard });
        cx.notify();
    }

    /// Takes the focus and waits for the next key press. The wait also ends when the screen
    /// loses the focus, the window goes inactive, or the dock hides the screen (`show`).
    fn start_capture(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
        on: impl Fn(&mut Self, Captured, &mut Context<Self>) + 'static,
    ) -> CaptureGuard {
        window.focus(&self.focus, cx);
        let wait = keys::capture(cx.weak_entity(), cx, move |this, captured, _, cx| {
            on(this, captured, cx)
        });
        let blur = cx.on_focus_out(&self.focus, window, |this, _, _, cx| {
            this.cancel_captures(cx)
        });
        let inactive = cx.observe_window_activation(window, |this, window, cx| {
            if !window.is_window_active() {
                this.cancel_captures(cx);
            }
        });
        CaptureGuard {
            _ties: vec![wait, blur, inactive],
        }
    }

    /// Ends each key capture of the screen. The keys stay as they were.
    fn cancel_captures(&mut self, cx: &mut Context<Self>) {
        let waiting = self.capture.take().is_some();
        let dictating = self.dictation.as_mut().is_some_and(|d| {
            let was = d.waiting();
            d.stop_waiting();
            was
        });
        if waiting || dictating {
            cx.notify();
        }
    }

    /// The dock shows or hides the screen. A hidden screen takes no key.
    pub fn show(&mut self, shown: bool, cx: &mut Context<Self>) {
        if !shown {
            self.cancel_captures(cx);
        }
    }

    fn on_captured(&mut self, captured: Captured, cx: &mut Context<Self>) {
        let Some(Capturing { id, .. }) = self.capture.take() else {
            return;
        };
        if let Captured::Key(key) = captured
            && let Err(why) = keys::change(cx, id, &key)
        {
            self.keys_note = Some((id, why));
        }
        cx.notify();
    }

    fn reset_key(&mut self, id: &'static str, cx: &mut Context<Self>) {
        self.capture = None;
        self.keys_note = match keys::reset(cx, id) {
            Ok(()) => None,
            Err(why) => Some((id, why)),
        };
        cx.notify();
    }

    /// Whether a row shows for the lowercase filter text `query`: it names the action or the key.
    fn row_matches(r: &keys::Row, query: &str) -> bool {
        query.is_empty()
            || r.label.to_lowercase().contains(query)
            || r.id.contains(query)
            || keys::display(&r.keystroke).to_lowercase().contains(query)
    }

    fn keys_section(&mut self, window: &mut Window, cx: &mut Context<Self>) -> Div {
        let filter = self.filter_input(window, cx);
        let query = filter.read(cx).value().trim().to_lowercase();
        let (rows, problems, file) = keys::with(cx, |k| {
            (
                k.rows(),
                k.problems().to_vec(),
                k.path().map(|p| p.display().to_string()),
            )
        });
        let rows: Vec<_> = rows
            .into_iter()
            .filter(|r| Self::row_matches(r, &query))
            .collect();
        let waiting = self.capture.as_ref().map(|c| c.id);
        let lines: Vec<Div> = rows
            .iter()
            .enumerate()
            .map(|(i, r)| {
                let id = r.id;
                let note = self
                    .keys_note
                    .as_ref()
                    .filter(|(n, _)| *n == id)
                    .map(|(_, why)| why.clone());
                let shown = if waiting == Some(id) {
                    div()
                        .text_size(size::sm())
                        .text_color(accent::BASE)
                        .child("Press the new key. Escape cancels.")
                        .into_any_element()
                } else {
                    keys::hint_for(&r.keystroke).into_any_element()
                };
                div()
                    .flex()
                    .flex_col()
                    .when(waiting == Some(id), |el| {
                        // A click in the row that waits keeps the capture.
                        el.on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                    })
                    .child(pair(
                        r.label,
                        div()
                            .flex()
                            .items_center()
                            .gap(space(2))
                            .child(shown)
                            .child(
                                Button::new(("key-change", i))
                                    .secondary()
                                    .small()
                                    .label("Change")
                                    .on_click(cx.listener(move |this, _, window, cx| {
                                        this.begin_change(id, window, cx)
                                    })),
                            )
                            .when(r.changed, |el| {
                                el.child(
                                    Button::new(("key-reset", i))
                                        .secondary()
                                        .small()
                                        .icon(IconName::RotateCw)
                                        .label("Reset")
                                        .on_click(cx.listener(move |this, _, _, cx| {
                                            this.reset_key(id, cx)
                                        })),
                                )
                            }),
                    ))
                    .when_some(note, |el, why| {
                        el.child(
                            div()
                                .pt(space(1))
                                .text_size(size::sm())
                                .text_color(Status::Danger.color())
                                .child(why),
                        )
                    })
            })
            .collect();
        let empty = lines.is_empty();
        div()
            .flex()
            .flex_col()
            .gap(space(3))
            .child(section("Keys", Some(rows.len())))
            .child(
                div()
                    .text_size(size::sm())
                    .text_color(text::SECONDARY)
                    .child("Change a key and it works at once. A key needs Ctrl, Alt or Super, unless it is a function key."),
            )
            .children(problems.into_iter().map(|p| {
                div()
                    .text_size(size::sm())
                    .text_color(Status::Warn.color())
                    .child(p)
            }))
            .child(Input::new(&filter))
            .children(lines)
            .when(empty, |el| {
                el.child(
                    div()
                        .text_size(size::sm())
                        .text_color(text::SECONDARY)
                        .child("No key matches the filter."),
                )
            })
            .child(mono(match file {
                Some(f) => format!("{f} · holds only the keys that differ from the defaults"),
                None => "no config folder, so a key cannot be saved".to_string(),
            }))
    }

    fn model_card(&self, i: usize, m: &ModelInfo, cx: &mut Context<Self>) -> Card {
        let (tag, detail) = state_view(m);
        let failed = matches!(
            m.state,
            ModelState::Error { .. } | ModelState::Conflict { .. }
        );
        let can_download = matches!(
            m.state,
            ModelState::Missing
                | ModelState::Partial { .. }
                | ModelState::Error { .. }
                | ModelState::Conflict { .. }
        );
        let can_delete = matches!(
            m.state,
            ModelState::Downloaded
                | ModelState::Partial { .. }
                | ModelState::Error { .. }
                | ModelState::Conflict { .. }
        );
        let label = match m.state {
            ModelState::Partial { .. } => "Continue",
            ModelState::Error { .. } | ModelState::Conflict { .. } => "Try again",
            _ => "Download",
        };
        let (down_id, del_id) = (m.id.clone(), m.id.clone());
        let actions = div()
            .flex()
            .items_center()
            .gap(space(2))
            .when(can_download, |el| {
                el.child(
                    Button::new(("model-download", i))
                        .primary()
                        .small()
                        .icon(IconName::ArrowDown)
                        .label(label)
                        .on_click(
                            cx.listener(move |this, _, _, cx| this.download(down_id.clone(), cx)),
                        ),
                )
            })
            .when(can_delete, |el| {
                el.child(
                    Button::new(("model-delete", i))
                        .secondary()
                        .small()
                        .icon(IconName::Delete)
                        .label("Delete")
                        .on_click(
                            cx.listener(move |this, _, _, cx| this.delete(del_id.clone(), cx)),
                        ),
                )
            });
        let progress = match m.state {
            ModelState::Downloading { bytes, total, .. } if total > 0 => {
                Some((bytes as f32 / total as f32).clamp(0., 1.))
            }
            _ => None,
        };
        let refused = self.refused.get(&m.id).cloned();
        Card::new(("model", i))
            .child(
                div()
                    .flex()
                    .items_center()
                    .justify_between()
                    .gap(space(3))
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap(space(2))
                            .min_w_0()
                            .child(
                                div()
                                    .text_size(size::md())
                                    .font_weight(weight::SEMIBOLD)
                                    .text_color(text::PRIMARY)
                                    .child(m.name.clone()),
                            )
                            .child(tag),
                    )
                    .child(actions),
            )
            .child(
                div()
                    .text_size(size::sm())
                    .text_color(text::SECONDARY)
                    .child(m.description.clone()),
            )
            .child(mono(format!(
                "{} · {} · {} · {}",
                m.id,
                m.kind,
                m.license,
                bytes(m.size)
            )))
            .when_some(progress, |el, f| el.child(bar(f)))
            .when_some(detail, |el, d| {
                el.child(
                    div()
                        .text_size(size::sm())
                        .text_color(if failed {
                            Status::Danger.color()
                        } else {
                            text::SECONDARY
                        })
                        .child(d),
                )
            })
            .when_some(refused, |el, r| {
                el.child(
                    div()
                        .text_size(size::sm())
                        .text_color(Status::Danger.color())
                        .child(r),
                )
            })
    }
}

/// A thin bar filled to `fraction`. The `Progress` of gpui-kit animates each new value at
/// the full refresh rate, which breaks the cap of 10 frames each second during a download.
fn bar(fraction: f32) -> Div {
    div()
        .w_full()
        .h(space(1))
        .rounded(radius::FULL)
        .bg(line::BORDER)
        .child(
            div()
                .h_full()
                .w(relative(fraction))
                .rounded(radius::FULL)
                .bg(accent::BASE),
        )
}

/// One `label  value` line of the read-only rows.
fn pair(label: impl Into<SharedString>, value: impl IntoElement) -> Div {
    div()
        .flex()
        .items_center()
        .justify_between()
        .gap(space(4))
        .py(space(2))
        .border_b_1()
        .border_color(line::BORDER)
        .child(
            div()
                .text_size(size::base())
                .text_color(text::PRIMARY)
                .child(label.into()),
        )
        .child(value)
}

fn voice_rows(v: &VoicesResult) -> Div {
    let value = |voice: &str, note: Option<&String>| {
        div()
            .flex()
            .flex_col()
            .items_end()
            .gap(space(1))
            .child(
                div()
                    .font_family(font::MONO)
                    .text_size(size::sm())
                    .text_color(text::SECONDARY)
                    .child(voice.to_string()),
            )
            .when_some(note.cloned(), |el, n| {
                el.child(
                    div()
                        .text_size(size::xs())
                        .text_color(Status::Warn.color())
                        .child(n),
                )
            })
    };
    div()
        .flex()
        .flex_col()
        .children(v.roles.iter().map(|r| {
            pair(
                super::parts::role_label(&r.role),
                value(&r.voice, r.note.as_ref()),
            )
        }))
        .child(pair(
            "Any other role",
            value(&v.default, v.default_note.as_ref()),
        ))
        .child(
            div()
                .pt(space(2))
                .child(mono(format!("{} · language {}", v.source, v.lang))),
        )
}

/// The state tag and the detail lines of one SSH host.
fn host_value(h: &RemoteHost) -> Div {
    let (status, word) = crate::sidebar::host_status(&h.state);
    let tag = status_tag(word, status);
    let products = match h.products.len() {
        1 => "1 product".to_string(),
        n => format!("{n} products"),
    };
    let why = match &h.state {
        HostState::Off(why) => Some(format!("{why}. The app tries again.")),
        _ => None,
    };
    div()
        .flex()
        .flex_col()
        .items_end()
        .gap(space(1))
        .child(tag)
        .child(mono(format!("ssh {} · {products}", h.ssh)))
        .children(why.into_iter().chain(h.note.clone()).map(|t| {
            div()
                .text_size(size::xs())
                .text_color(text::TERTIARY)
                .child(t)
        }))
}

fn host_rows(h: &Hosts) -> Div {
    let file = h
        .file
        .as_ref()
        .map_or("no config folder".to_string(), |f| f.display().to_string());
    div()
        .flex()
        .flex_col()
        .children(h.hosts.iter().map(|r| pair(r.name.clone(), host_value(r))))
        .when(h.hosts.is_empty() && h.error.is_none(), |el| {
            el.child(
                div()
                    .py(space(2))
                    .text_size(size::sm())
                    .text_color(text::SECONDARY)
                    .child("No SSH hosts. Add a [[host]] table to the file below."),
            )
        })
        .when_some(h.error.clone(), |el, e| {
            el.child(
                div()
                    .py(space(2))
                    .text_size(size::sm())
                    .text_color(Status::Danger.color())
                    .child(format!("The hosts were not read: {e}")),
            )
        })
        .child(
            div()
                .pt(space(2))
                .child(mono(format!("{file} · read at start, edit it by hand"))),
        )
}

impl Render for SettingsScreen {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let root = div()
            .id("settings")
            .debug_selector(|| "settings-root".into())
            .size_full()
            .bg(surface::PANEL)
            // A click outside the row that waits ends the capture.
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(|this, _, _, cx| this.cancel_captures(cx)),
            );
        let models = match &self.models {
            Load::Loading => return root.child(notice("Reading the models…", None)),
            Load::Failed(e) => {
                return root.child(notice(
                    format!("The models were not read: {e}"),
                    Some("Check that the core runs, then reload the screen.".into()),
                ));
            }
            Load::Ready(list) => list.clone(),
        };
        let cards: Vec<Card> = models
            .models
            .iter()
            .enumerate()
            .map(|(i, m)| self.model_card(i, m, cx))
            .collect();
        let voices = match &self.voices {
            Load::Loading => div().child(mono("Reading the voices…")),
            Load::Failed(e) => div()
                .text_size(size::sm())
                .text_color(Status::Danger.color())
                .child(format!("The voices were not read: {e}")),
            Load::Ready(v) => voice_rows(v),
        };
        let vocabulary = self.vocabulary_section(window, cx);
        let keys_section = self.keys_section(window, cx);
        let dictation = self.dictation_section(cx);
        let appearance = self.appearance_section(cx);
        let data = self.data_section(window, cx);
        let git = self.git_section();
        let guide = self.guide_section(cx);
        let hosts = self.hosts.as_ref().map(|h| host_rows(h.read(cx)));
        root.overflow_y_scroll()
            .track_focus(&self.focus)
            .on_key_down(cx.listener(|this, e: &KeyDownEvent, _, cx| this.scroll_key(e, cx)))
            .track_scroll(&self.scroll)
            .child(
            column()
                .child(
                    div()
                        .debug_selector(|| "settings-header".into())
                        .flex()
                        .flex_col()
                        .gap(space(1))
                        .child(
                            div()
                                .text_size(size::lg())
                                .font_weight(weight::SEMIBOLD)
                                .text_color(text::PRIMARY)
                                .child("Settings"),
                        )
                        .child(
                            div()
                                .text_size(size::sm())
                                .text_color(text::SECONDARY)
                                .child("The look of the app, voice models, voices, the vocabulary and the keys of this device, its SSH hosts, and the data to delete."),
                        ),
                )
                .child(appearance)
                .child(
                    div()
                        .flex()
                        .flex_col()
                        .gap(space(3))
                        .child(section("Voice models", Some(cards.len())))
                        .child(mono(format!(
                            "{} · each file has a pinned SHA-256",
                            models.root
                        )))
                        .children(cards),
                )
                .child(vocabulary)
                .child(
                    div()
                        .flex()
                        .flex_col()
                        .gap(space(3))
                        .child(section("Voices", None))
                        .child(voices),
                )
                .child(keys_section)
                .when_some(dictation, |el, section| el.child(section))
                .child(git)
                .when_some(hosts, |el, rows| {
                    el.child(
                        div()
                            .flex()
                            .flex_col()
                            .gap(space(3))
                            .child(section("Hosts", None))
                            .child(rows),
                    )
                })
                .child(guide)
                .child(data),
        )
    }
}

#[cfg(test)]
mod tests {
    use super::{apply, bytes, scroll_after};
    use agentsi_proto::models_api::{ModelInfo, ModelState};
    use agentsi_proto::rpc::Notification;
    use serde_json::json;

    #[test]
    fn page_keys_scroll_the_page_and_stop_at_both_ends() {
        use gpui::px;
        let (page, max) = (px(700.), px(1500.));
        let after = |key: &str, now: f32| scroll_after(key, px(now), page, max);
        assert_eq!(after("pagedown", 0.), Some(px(-700.)));
        assert_eq!(
            after("pagedown", -1200.),
            Some(px(-1500.)),
            "the end stops it"
        );
        assert_eq!(after("pageup", -1000.), Some(px(-300.)));
        assert_eq!(after("pageup", -200.), Some(px(0.)), "the top stops it");
        assert_eq!(after("home", -900.), Some(px(0.)));
        assert_eq!(after("end", -100.), Some(px(-1500.)));
        assert_eq!(after("a", -100.), None);
    }

    fn model(id: &str) -> ModelInfo {
        ModelInfo {
            id: id.into(),
            name: id.into(),
            kind: "speech-out".into(),
            license: "Apache-2.0".into(),
            description: String::new(),
            size: 100,
            state: ModelState::Missing,
        }
    }

    fn note(method: &str, params: serde_json::Value) -> Notification {
        serde_json::from_value(json!({"jsonrpc": "2.0", "method": method, "params": params}))
            .unwrap()
    }

    #[test]
    fn a_hash_error_event_puts_the_error_of_the_core_in_its_row() {
        let mut rows = [model("kokoro"), model("other")];
        let message = "`a.onnx` failed the SHA-256 check (expected 1, got 2) and was deleted";
        assert!(apply(
            &mut rows,
            &note(
                "models.error",
                json!({"id": "kokoro", "error": "hash_mismatch", "message": message})
            )
        ));
        assert_eq!(
            rows[0].state,
            ModelState::Error {
                error: "hash_mismatch".into(),
                message: message.into()
            }
        );
        assert_eq!(rows[1].state, ModelState::Missing);
    }

    #[test]
    fn progress_done_and_conflict_events_change_the_row() {
        let mut rows = [model("kokoro")];
        let progress = note(
            "models.progress",
            json!({"id": "kokoro", "bytes": 40, "total": 100, "verifying": false}),
        );
        assert!(apply(&mut rows, &progress));
        assert!(
            !apply(&mut rows, &progress),
            "the same event changes nothing"
        );
        assert!(matches!(
            rows[0].state,
            ModelState::Downloading { bytes: 40, .. }
        ));
        assert!(apply(
            &mut rows,
            &note("models.done", json!({"id": "kokoro"}))
        ));
        assert_eq!(rows[0].state, ModelState::Downloaded);
        let conflict = note(
            "models.error",
            json!({"id": "kokoro", "error": "conflict", "message": "m", "file": "a.onnx"}),
        );
        assert!(apply(&mut rows, &conflict));
        assert!(matches!(&rows[0].state, ModelState::Conflict { file, .. } if file == "a.onnx"));
        // Another event or another model changes nothing.
        assert!(!apply(
            &mut rows,
            &note("terminal.title", json!({"id": "t1", "title": "x"}))
        ));
        assert!(!apply(
            &mut rows,
            &note("models.done", json!({"id": "nope"}))
        ));
    }

    #[test]
    fn events_of_a_running_reload_apply_after_its_older_list() {
        // The list was read while the download ran, and the done event came first.
        let mut rows = [model("kokoro")];
        rows[0].state = ModelState::Downloading {
            bytes: 10,
            total: 100,
            verifying: false,
        };
        super::apply_all(&mut rows, &[note("models.done", json!({"id": "kokoro"}))]);
        assert_eq!(rows[0].state, ModelState::Downloaded);
    }

    #[test]
    fn heard_forms_split_at_commas_and_lines() {
        assert_eq!(
            super::parse_heard("Agent C,  agent  SI\n, ,Agent see"),
            ["Agent C", "agent SI", "Agent see"]
        );
        assert!(super::parse_heard(" , \n").is_empty());
    }

    #[test]
    fn the_form_adds_an_entry_or_replaces_the_one_it_edits() {
        use agentsi_proto::vocabulary_api::VocabularyEntry;
        let entry = |t: &str| VocabularyEntry {
            term: t.into(),
            heard: vec![],
        };
        let list = [entry("a"), entry("b")];
        assert_eq!(super::with_entry(&list, None, entry("c")).len(), 3);
        let edited = super::with_entry(&list, Some(0), entry("z"));
        assert_eq!(edited[0].term, "z");
        assert_eq!(edited.len(), 2);
        // A row that no longer exists adds a new entry.
        assert_eq!(super::with_entry(&list, Some(9), entry("c")).len(), 3);
    }

    #[test]
    fn sizes_read_in_decimal_units() {
        assert_eq!(bytes(325_532_387), "325.5 MB");
        assert_eq!(bytes(1_200_000_000), "1.2 GB");
        assert_eq!(bytes(12_000), "12.0 KB");
        assert_eq!(bytes(999), "999 B");
    }

    pub(super) mod support {
        use std::cell::Cell;
        use std::path::PathBuf;
        use std::rc::Rc;
        use std::sync::Arc;

        use agentsi_proto::models_api::ModelsListResult;
        use gpui::{TestAppContext, VisualTestContext};
        use gpui_kit::{AppContext as _, Entity};

        use crate::actions::{NewTerminal, OpenRoles};
        use crate::link::Link;
        use crate::model::CoreState;
        use crate::screens::parts::Load;
        use crate::screens::settings::SettingsScreen;

        pub struct Setup {
            pub screen: Entity<SettingsScreen>,
            /// The panel of the dock, when the screen lives in one.
            pub panel: Option<Entity<crate::screens::ScreenPanel>>,
            pub file: PathBuf,
            pub terminals: Rc<Cell<usize>>,
            pub roles: Rc<Cell<usize>>,
            // The core side of the link: it must stay open.
            pub core_end: std::os::unix::net::UnixStream,
        }

        /// A settings screen in a window, on a key file in a folder of its own. `NewTerminal`
        /// and `OpenRoles` count their runs.
        pub fn setup<'a>(
            cx: &'a mut TestAppContext,
            file_text: Option<&str>,
        ) -> (Setup, &'a mut VisualTestContext) {
            setup_with(cx, file_text, None)
        }

        pub fn setup_with<'a>(
            cx: &'a mut TestAppContext,
            file_text: Option<&str>,
            dictation: Option<super::super::dictation::DictationEnv>,
        ) -> (Setup, &'a mut VisualTestContext) {
            let global = dictation.map(super::super::DictationGlobal::Ready);
            build(cx, file_text, global, false)
        }

        /// The same, with any value of the dictation global.
        pub fn setup_global(
            cx: &mut TestAppContext,
            global: super::super::DictationGlobal,
        ) -> (Setup, &mut VisualTestContext) {
            build(cx, None, Some(global), false)
        }

        /// The same, with the screen in a `ScreenPanel`, as the dock holds it.
        pub fn setup_panel<'a>(
            cx: &'a mut TestAppContext,
            file_text: Option<&str>,
            dictation: Option<super::super::dictation::DictationEnv>,
        ) -> (Setup, &'a mut VisualTestContext) {
            let global = dictation.map(super::super::DictationGlobal::Ready);
            build(cx, file_text, global, true)
        }

        fn build<'a>(
            cx: &'a mut TestAppContext,
            file_text: Option<&str>,
            dictation: Option<super::super::DictationGlobal>,
            in_panel: bool,
        ) -> (Setup, &'a mut VisualTestContext) {
            static N: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
            let dir = std::env::temp_dir().join(format!(
                "agentsi-settings-keys-{}-{}",
                std::process::id(),
                N.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
            ));
            std::fs::create_dir_all(&dir).unwrap();
            let file = dir.join("keys.toml");
            if let Some(text) = file_text {
                std::fs::write(&file, text).unwrap();
            }
            let (terminals, roles) = (Rc::new(Cell::new(0)), Rc::new(Cell::new(0)));
            cx.update(agentsi_ui::init);
            cx.update(|cx| {
                if let Some(env) = dictation {
                    cx.set_global(env);
                }
                crate::actions::bind_from(cx, Some(file.clone()));
                crate::appearance::install(
                    cx,
                    Some(file.with_file_name(crate::appearance::FILE_NAME)),
                );
                let t = terminals.clone();
                cx.on_action(move |_: &NewTerminal, _| t.set(t.get() + 1));
                let r = roles.clone();
                cx.on_action(move |_: &OpenRoles, _| r.set(r.get() + 1));
            });
            let (end, core_end) = std::os::unix::net::UnixStream::pair().unwrap();
            let core = cx.new(|_| CoreState::new(Arc::new(Link::for_tests(end))));
            let (panel, screen, cx) = if in_panel {
                let spec = crate::screens::ScreenSpec::settings();
                let (panel, cx) =
                    cx.add_window_view(|_, cx| crate::screens::ScreenPanel::new(spec, core, cx));
                let screen = panel.read_with(cx, |p, _| p.settings().unwrap());
                (Some(panel), screen, cx)
            } else {
                let (screen, cx) = cx.add_window_view(|_, cx| SettingsScreen::new(core, cx));
                (None, screen, cx)
            };
            screen.update(cx, |s, cx| {
                s.models = Load::Ready(ModelsListResult {
                    root: "/models".into(),
                    models: Vec::new(),
                });
                cx.notify();
            });
            cx.update(|window, cx| window.draw(cx).clear(cx));
            (
                Setup {
                    screen,
                    panel,
                    file,
                    terminals,
                    roles,
                    core_end,
                },
                cx,
            )
        }
    }

    mod keys_section {
        use gpui::{TestAppContext, VisualTestContext};
        use gpui_kit::AsKeystroke as _;
        use gpui_kit::component::kbd::Kbd;

        use super::support::{Setup, setup};
        use crate::actions::NewTerminal;
        use crate::keys;

        fn change(t: &Setup, id: &'static str, cx: &mut VisualTestContext) {
            t.screen
                .update_in(cx, |s, window, cx| s.begin_change(id, window, cx));
        }

        fn note(t: &Setup, cx: &mut VisualTestContext) -> Option<String> {
            t.screen
                .read_with(cx, |s, _| s.keys_note.as_ref().map(|(_, n)| n.clone()))
        }

        fn waiting(t: &Setup, cx: &mut VisualTestContext) -> bool {
            t.screen.read_with(cx, |s, _| s.capture.is_some())
        }

        fn file_text(t: &Setup) -> String {
            std::fs::read_to_string(&t.file).unwrap_or_default()
        }

        #[gpui::test]
        fn a_changed_key_works_at_once_and_the_old_key_does_nothing(cx: &mut TestAppContext) {
            let (t, cx) = setup(cx, None);
            cx.simulate_keystrokes("ctrl-t");
            assert_eq!(t.terminals.get(), 1, "the default key runs the action");

            change(&t, "new-terminal", cx);
            assert!(waiting(&t, cx));
            cx.simulate_keystrokes("ctrl-alt-y");
            assert!(!waiting(&t, cx));
            assert_eq!(note(&t, cx), None);
            assert_eq!(
                t.terminals.get(),
                1,
                "the key press that sets the key runs nothing"
            );
            assert!(
                file_text(&t).contains("new-terminal = \"ctrl-alt-y\""),
                "{}",
                file_text(&t)
            );

            cx.simulate_keystrokes("ctrl-t");
            assert_eq!(t.terminals.get(), 1, "the old key does nothing");
            cx.simulate_keystrokes("ctrl-alt-y");
            assert_eq!(t.terminals.get(), 2, "the new key runs the action");
        }

        #[gpui::test]
        fn the_palette_hint_and_the_settings_row_show_the_new_key(cx: &mut TestAppContext) {
            let (t, cx) = setup(cx, None);
            let hint = |cx: &mut VisualTestContext| {
                cx.update(|window, _| {
                    // The palette row of gpui-component shows this binding.
                    assert!(Kbd::binding_for_action(&NewTerminal, None, window).is_some());
                    window
                        .highest_precedence_binding_for_action(&NewTerminal)
                        .and_then(|b| {
                            b.keystrokes()
                                .first()
                                .map(|k| Kbd::format(k.as_keystroke()))
                        })
                })
            };
            assert_eq!(hint(cx).as_deref(), Some("Ctrl+T"));
            change(&t, "new-terminal", cx);
            cx.simulate_keystrokes("ctrl-alt-y");
            assert_eq!(hint(cx).as_deref(), Some("Ctrl+Alt+Y"));
            // The settings row and the sidebar hint read the keys in effect.
            let rows = cx.update(|_, cx| keys::with(cx, |k| k.rows()));
            let row = rows.iter().find(|r| r.id == "new-terminal").unwrap();
            assert_eq!(keys::display(&row.keystroke), "Ctrl+Alt+Y");
            assert!(row.changed);
            assert_eq!(
                cx.update(|_, cx| keys::text(cx, "new-terminal")),
                "Ctrl+Alt+Y"
            );
            t.screen.update_in(cx, |s, window, cx| {
                s.keys_section(window, cx);
            });
        }

        #[gpui::test]
        fn a_key_file_with_bad_entries_keeps_the_good_ones_and_shows_one_line_each(
            cx: &mut TestAppContext,
        ) {
            let (t, cx) = setup(
                cx,
                Some(
                    "[app]\nno-such-action = \"ctrl-alt-j\"\nroles = \"ctrl-alt-r\"\nmemory = \"???\"\n",
                ),
            );
            let lines = cx.update(|_, cx| keys::with(cx, |k| k.problems().to_vec()));
            assert_eq!(lines.len(), 2, "{lines:?}");
            assert!(lines[0].starts_with("[app] no-such-action"), "{lines:?}");
            assert!(lines[1].starts_with("[app] memory"), "{lines:?}");
            cx.simulate_keystrokes("ctrl-alt-r");
            assert_eq!(t.roles.get(), 1, "the good entry works");
            cx.simulate_keystrokes("ctrl-shift-r");
            assert_eq!(
                t.roles.get(),
                1,
                "the default key of the changed action is free"
            );
            // The screen draws the lines.
            t.screen.update_in(cx, |s, window, cx| {
                s.keys_section(window, cx);
            });
        }

        #[gpui::test]
        fn the_editor_refuses_each_kind_of_key_on_the_screen_with_its_reason(
            cx: &mut TestAppContext,
        ) {
            let (t, cx) = setup(cx, None);
            for (pressed, reason) in [
                ("y", "needs Ctrl, Alt"),
                ("shift-y", "needs Ctrl, Alt"),
                ("ctrl-shift-c", "key of the terminal pane"),
                ("ctrl-shift-v", "key of the terminal pane"),
                ("ctrl-c", "key of terminal programs"),
                ("ctrl-d", "key of terminal programs"),
                ("ctrl-w", "key of terminal programs"),
                ("ctrl-k", "is already the key of \"Command palette\""),
            ] {
                change(&t, "roles", cx);
                cx.simulate_keystrokes(pressed);
                let line = note(&t, cx).unwrap_or_else(|| panic!("{pressed} was not refused"));
                assert!(line.contains(reason), "{pressed}: {line}");
                assert!(!waiting(&t, cx), "{pressed}");
            }
            assert_eq!(file_text(&t), "");
            // A function key stands alone.
            change(&t, "roles", cx);
            cx.simulate_keystrokes("f9");
            assert_eq!(note(&t, cx), None);
            assert!(file_text(&t).contains("roles = \"f9\""));
            // The next change clears the line of the last refusal.
            change(&t, "roles", cx);
            assert_eq!(note(&t, cx), None);
        }

        #[gpui::test]
        fn a_capture_ends_when_the_tab_is_hidden_and_the_next_key_reaches_its_action(
            cx: &mut TestAppContext,
        ) {
            use gpui_kit::component::dock::BasePanel as _;
            let (t, cx) = super::support::setup_panel(cx, None, None);
            let panel = t.panel.clone().unwrap();
            panel.update_in(cx, |p, window, cx| p.set_active(true, window, cx));
            change(&t, "roles", cx);
            assert!(waiting(&t, cx));
            // The owner clicks a terminal tab: the dock hides the settings panel.
            panel.update_in(cx, |p, window, cx| p.set_active(false, window, cx));
            assert!(!waiting(&t, cx));
            cx.simulate_keystrokes("ctrl-t");
            assert_eq!(t.terminals.get(), 1, "the key reaches its action");
            assert_eq!(note(&t, cx), None);
            assert_eq!(file_text(&t), "");
        }

        #[gpui::test]
        fn a_capture_ends_when_the_screen_loses_the_focus(cx: &mut TestAppContext) {
            let (t, cx) = setup(cx, None);
            // The focus events reach a listener only in an active window.
            cx.update(|window, _| window.activate_window());
            cx.run_until_parked();
            cx.update(|window, cx| window.draw(cx).clear(cx));
            change(&t, "roles", cx);
            cx.update(|window, cx| window.draw(cx).clear(cx));
            assert!(waiting(&t, cx));
            cx.update(|window, cx| window.blur(cx));
            cx.update(|window, cx| window.draw(cx).clear(cx));
            assert!(!waiting(&t, cx));
            cx.simulate_keystrokes("ctrl-t");
            assert_eq!(t.terminals.get(), 1);
            assert_eq!(file_text(&t), "");
        }

        #[gpui::test]
        fn a_mouse_down_outside_the_row_ends_the_capture(cx: &mut TestAppContext) {
            let (t, cx) = setup(cx, None);
            change(&t, "roles", cx);
            cx.update(|window, cx| window.draw(cx).clear(cx));
            assert!(waiting(&t, cx));
            cx.simulate_mouse_down(
                gpui::point(gpui::Pixels::from(4.0_f32), gpui::Pixels::from(4.0_f32)),
                gpui::MouseButton::Left,
                gpui::Modifiers::none(),
            );
            assert!(!waiting(&t, cx));
            cx.simulate_keystrokes("ctrl-t");
            assert_eq!(t.terminals.get(), 1);
        }

        #[gpui::test]
        fn a_key_changed_back_to_its_default_leaves_no_entry_and_no_reset(cx: &mut TestAppContext) {
            let (t, cx) = setup(cx, None);
            change(&t, "roles", cx);
            cx.simulate_keystrokes("alt-y");
            assert!(
                file_text(&t).contains("roles = \"alt-y\""),
                "{}",
                file_text(&t)
            );
            change(&t, "roles", cx);
            cx.simulate_keystrokes("ctrl-shift-r");
            assert_eq!(note(&t, cx), None);
            assert!(!file_text(&t).contains("roles"), "{}", file_text(&t));
            let rows = cx.update(|_, cx| keys::with(cx, |k| k.rows()));
            assert!(!rows.iter().find(|r| r.id == "roles").unwrap().changed);
            cx.simulate_keystrokes("ctrl-shift-r");
            assert_eq!(t.roles.get(), 1);
        }

        #[gpui::test]
        fn escape_leaves_the_key_as_it_was_and_a_key_press_runs_no_action(cx: &mut TestAppContext) {
            let (t, cx) = setup(cx, None);
            change(&t, "roles", cx);
            cx.simulate_keystrokes("ctrl-t");
            assert_eq!(
                t.terminals.get(),
                0,
                "the key of an action runs nothing while a row waits"
            );
            assert!(!waiting(&t, cx));

            change(&t, "roles", cx);
            cx.simulate_keystrokes("escape");
            assert!(!waiting(&t, cx));
            assert_eq!(note(&t, cx), None);
            assert_eq!(file_text(&t), "");
            cx.simulate_keystrokes("ctrl-shift-r");
            assert_eq!(t.roles.get(), 1, "the key is as it was");
            cx.simulate_keystrokes("ctrl-t");
            assert_eq!(t.terminals.get(), 1, "the capture is over");
        }

        #[gpui::test]
        fn reset_removes_the_entry_from_the_file_and_the_default_key_is_back(
            cx: &mut TestAppContext,
        ) {
            let (t, cx) = setup(cx, Some("# mine\n[app]\nstories = \"ctrl-alt-s\"\n"));
            change(&t, "roles", cx);
            cx.simulate_keystrokes("ctrl-alt-r");
            assert!(file_text(&t).contains("roles = \"ctrl-alt-r\""));
            cx.simulate_keystrokes("ctrl-shift-r");
            assert_eq!(t.roles.get(), 0);

            t.screen.update(cx, |s, cx| s.reset_key("roles", cx));
            let text = file_text(&t);
            assert!(!text.contains("roles"), "{text}");
            assert!(
                text.contains("# mine") && text.contains("stories"),
                "{text}"
            );
            cx.simulate_keystrokes("ctrl-alt-r");
            assert_eq!(t.roles.get(), 0, "the changed key is gone");
            cx.simulate_keystrokes("ctrl-shift-r");
            assert_eq!(t.roles.get(), 1, "the default key is back");
        }

        #[gpui::test]
        fn the_filter_narrows_the_rows(cx: &mut TestAppContext) {
            let (t, cx) = setup(cx, None);
            let count = |t: &Setup, cx: &mut VisualTestContext| {
                t.screen.update_in(cx, |s, window, cx| {
                    s.keys_section(window, cx);
                    let q = s.keys_filter.as_ref().unwrap().read(cx).value().to_string();
                    keys::with(cx, |k| k.rows())
                        .iter()
                        .filter(|r| {
                            crate::screens::settings::SettingsScreen::row_matches(
                                r,
                                q.trim().to_lowercase().as_str(),
                            )
                        })
                        .count()
                })
            };
            assert_eq!(count(&t, cx), 23);
            t.screen.update_in(cx, |s, window, cx| {
                let f = s.keys_filter.clone().unwrap();
                f.update(cx, |i, cx| i.set_value("terminal", window, cx));
            });
            assert_eq!(count(&t, cx), 1);
            t.screen.update_in(cx, |s, window, cx| {
                let f = s.keys_filter.clone().unwrap();
                f.update(cx, |i, cx| i.set_value("ctrl+k", window, cx));
            });
            assert_eq!(count(&t, cx), 1);
        }
    }
}
