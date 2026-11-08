//! The "Dictation key" section of the settings screen.
//!
//! The compositor holds the key, so the section writes the bind file of `dictation_bind` and
//! keeps the key in `[dictation] key` of `keys.toml`. Each read of `hyprctl` and each write of
//! a file runs on a background thread. The section reads at the start of the screen, after a
//! change and on a click on "Check again", so an idle screen draws no frame for it.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use agentsi_ui::components::{Button, neutral_tag, status_tag};
use agentsi_ui::tokens::{Status, accent, size, space, text};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;

use super::{CaptureGuard, SettingsScreen, pair};
use crate::dictation_bind::{self as bind, CoreCall, Form, Hyprctl, Mode, SaveError, Setup, State};
use crate::keys::{self, Captured, KeyFile};
use crate::screens::parts::{mono, section};

/// What the section needs from the machine.
#[derive(Clone)]
pub struct DictationEnv {
    pub setup: Setup,
    pub hyprctl: Arc<dyn Hyprctl + Send + Sync>,
}

/// The app sets this at start, except on macOS, where the section does not show. A screen test
/// sets it with a fake `hyprctl`.
#[derive(Clone)]
pub enum DictationGlobal {
    Ready(DictationEnv),
    /// The machine cannot write the bind file. The text says why, as one line.
    Missing(String),
}

impl Global for DictationGlobal {}

impl DictationGlobal {
    /// The setup of this session with the real `hyprctl`. `None` on macOS.
    pub fn detect() -> Option<DictationGlobal> {
        if cfg!(target_os = "macos") {
            return None;
        }
        let (Some(hypr), Some(dir)) = (bind::hypr_dir(), bind::agentsi_dir()) else {
            return Some(DictationGlobal::Missing(
                "AgentSI found no config folder, so it cannot write the dictation key.".into(),
            ));
        };
        let Some(core) = CoreCall::from_env() else {
            return Some(DictationGlobal::Missing(
                "AgentSI found no agentsi-core program, so it cannot write the dictation key. Put agentsi-core next to agentsi-app, on the PATH, or in AGENTSI_CORE_BIN.".into(),
            ));
        };
        let setup = Setup {
            form: bind::detect(&hypr, bind::in_session()),
            dir,
            core,
            mode: Mode::Hold,
            converse: None,
        };
        Some(DictationGlobal::Ready(DictationEnv {
            setup,
            hyprctl: Arc::new(bind::RealHyprctl),
        }))
    }
}

/// What the section shows for the key.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum View {
    Loading,
    NoKey,
    State(State),
    Failed(String),
}

pub struct DictationKey {
    env: Result<DictationEnv, String>,
    pub(super) view: View,
    /// The last refusal or failure, under the key.
    pub(super) note: Option<String>,
    /// A thread works. The buttons wait.
    pub(super) busy: bool,
    /// The bind file holds a bind.
    bound: bool,
    /// What the section shows for the conversation key.
    pub(super) converse_view: View,
    /// The bind file holds a conversation bind.
    converse_bound: bool,
    /// The capture ends when this drops.
    capture: Option<CaptureGuard>,
    /// The capture of the conversation key ends when this drops.
    converse_capture: Option<CaptureGuard>,
}

impl DictationKey {
    pub fn new(global: DictationGlobal) -> Self {
        DictationKey {
            env: match global {
                DictationGlobal::Ready(env) => Ok(env),
                DictationGlobal::Missing(why) => Err(why),
            },
            view: View::Loading,
            note: None,
            busy: false,
            bound: false,
            converse_view: View::Loading,
            converse_bound: false,
            capture: None,
            converse_capture: None,
        }
    }

    pub(super) fn stop_waiting(&mut self) {
        self.capture = None;
        self.converse_capture = None;
    }

    pub(super) fn waiting(&self) -> bool {
        self.capture.is_some() || self.converse_capture.is_some()
    }

    fn waiting_converse(&self) -> bool {
        self.converse_capture.is_some()
    }

    fn ready(&self) -> Option<&DictationEnv> {
        self.env
            .as_ref()
            .ok()
            .filter(|e| e.setup.form != Form::CommandsOnly)
    }
}

/// What a read of the machine gives: the views and the stray binds of both keys.
struct Read {
    dictation: View,
    bound: bool,
    converse: View,
    converse_bound: bool,
}

/// Writes the empty bind file when there is none, then reads the state of both keys.
fn read_view(env: &DictationEnv, key: Option<String>, converse: Option<String>) -> Read {
    if let Err(e) = env.setup.ensure_file() {
        let failed = View::Failed(e.to_string());
        return Read {
            dictation: failed.clone(),
            bound: false,
            converse: failed,
            converse_bound: false,
        };
    }
    let gpui = |k: &str| bind::from_hypr(k);
    let dictation_gpui = key.as_deref().and_then(|k| gpui(k).ok());
    let view_of = |key: Option<String>, read: &dyn Fn(&str) -> Result<State, String>| match key {
        None => View::NoKey,
        Some(key) => match gpui(&key) {
            Err(e) => View::Failed(format!("The key in keys.toml cannot be used: {e}")),
            Ok(g) => match read(&g) {
                Ok(state) => View::State(state),
                Err(e) => View::Failed(format!("Cannot read the binds: {e}")),
            },
        },
    };
    let dictation = view_of(key, &|g| env.setup.state(&*env.hyprctl, g));
    let converse = view_of(converse, &|g| {
        env.setup
            .converse_state(&*env.hyprctl, g, dictation_gpui.as_deref())
    });
    Read {
        dictation,
        bound: env.setup.bound(),
        converse,
        converse_bound: env.setup.converse_bound(),
    }
}

/// What a change of the key needs, off the UI thread.
/// The conf form cannot remove one bind of the owner with `hyprctl keyword`, so it reloads.
const CONF_RELOADS: &str = "The conf form reloads Hyprland on each change. On Omarchy, a reload moves the active workspace.";

struct Job {
    env: DictationEnv,
    keys_file: Option<PathBuf>,
}

impl Job {
    /// The key file must parse, or nothing is written.
    fn keys_file(&self) -> Result<&Path, String> {
        let path = self
            .keys_file
            .as_deref()
            .ok_or("There is no config folder, so the key cannot be saved.")?;
        KeyFile::read(path)?;
        Ok(path)
    }

    /// Writes the bind file for the GPUI key `gpui`, then the key file. If the key file cannot
    /// be written, the old bind file comes back. `Ok(Some(line))` says the key is saved and
    /// Hyprland did not apply it.
    fn save(&self, gpui: &str) -> Result<Option<String>, String> {
        let keys_file = self.keys_file()?;
        let hypr = bind::to_hypr(gpui)?;
        let before = self.bind_file();
        let warning = Self::apply_warning(self.env.setup.save(gpui, &*self.env.hyprctl))?;
        if let Err(e) = KeyFile::write_dictation(keys_file, Some(&hypr)) {
            self.restore(before);
            return Err(format!("{e}. The old bind file is back."));
        }
        Ok(warning)
    }

    /// Writes the bind file for the GPUI conversation key `gpui`, with the dictation key
    /// `dictation`, then the key file. If the key file cannot be written, the old bind file
    /// comes back.
    fn save_converse(&self, gpui: &str, dictation: Option<&str>) -> Result<Option<String>, String> {
        let keys_file = self.keys_file()?;
        let hypr = bind::to_hypr(gpui)?;
        let before = self.bind_file();
        let warning = Self::apply_warning(self.env.setup.save_converse(
            gpui,
            dictation,
            &*self.env.hyprctl,
        ))?;
        if let Err(e) = KeyFile::write_conversation(keys_file, Some(&hypr)) {
            self.restore(before);
            return Err(format!("{e}. The old bind file is back."));
        }
        Ok(warning)
    }

    /// Writes the bind file with no conversation bind, then drops the key from the key file.
    fn remove_converse(&self, dictation: Option<&str>) -> Result<Option<String>, String> {
        let keys_file = self.keys_file()?;
        let before = self.bind_file();
        let warning = Self::apply_warning(
            self.env
                .setup
                .remove_converse(dictation, &*self.env.hyprctl),
        )?;
        if let Err(e) = KeyFile::write_conversation(keys_file, None) {
            self.restore(before);
            return Err(format!("{e}. The old bind file is back."));
        }
        Ok(warning)
    }

    /// Writes the empty bind file, then drops the key from the key file.
    fn remove(&self) -> Result<Option<String>, String> {
        let keys_file = self.keys_file()?;
        let before = self.bind_file();
        let warning = Self::apply_warning(self.env.setup.remove(&*self.env.hyprctl))?;
        if let Err(e) = KeyFile::write_dictation(keys_file, None) {
            self.restore(before);
            return Err(format!("{e}. The old bind file is back."));
        }
        Ok(warning)
    }

    /// Writes the mode into the key file, then the bind file in the form of the mode for the
    /// GPUI key `gpui`, if there is one. If the bind file cannot be written, the key file keeps
    /// the `previous` mode.
    fn set_mode(
        &self,
        mode: Mode,
        previous: Mode,
        gpui: Option<&str>,
    ) -> Result<Option<String>, String> {
        let keys_file = self.keys_file()?;
        KeyFile::write_dictation_mode(keys_file, mode)?;
        let Some(gpui) = gpui else {
            return Ok(None);
        };
        let setup = Setup {
            mode,
            ..self.env.setup.clone()
        };
        Self::apply_warning(setup.save(gpui, &*self.env.hyprctl)).inspect_err(|_| {
            let _ = KeyFile::write_dictation_mode(keys_file, previous);
        })
    }

    fn apply_warning(saved: Result<(), SaveError>) -> Result<Option<String>, String> {
        match saved {
            Ok(()) => Ok(None),
            Err(e @ SaveError::Apply(_)) => Ok(Some(e.to_string())),
            Err(e) => Err(e.to_string()),
        }
    }

    fn bind_file(&self) -> Option<String> {
        std::fs::read_to_string(self.env.setup.bind_path()?).ok()
    }

    /// Puts the bind file back as it was and applies it, so the compositor matches the file. A
    /// file that did not exist comes back as the empty bind file, which removes the new bind.
    fn restore(&self, before: Option<String>) {
        let Some(path) = self.env.setup.bind_path() else {
            return;
        };
        let text = before.unwrap_or_else(|| bind::render_empty(self.env.setup.form));
        let _ = crate::atomic::write(&path, &text);
        let _ = self.env.setup.apply(&*self.env.hyprctl);
    }
}

impl SettingsScreen {
    fn dictation_job(&self, cx: &App) -> Option<Job> {
        let env = self.dictation.as_ref()?.env.clone().ok()?;
        let keys_file = keys::with(cx, |k| k.path().map(Path::to_path_buf));
        let mut job = Job { env, keys_file };
        job.env.setup.mode = keys::dictation_mode(cx);
        job.env.setup.converse = keys::conversation(cx).and_then(|k| bind::from_hypr(&k).ok());
        Some(job)
    }

    /// Reads the state again, off the UI thread.
    pub(super) fn refresh_dictation(&mut self, cx: &mut Context<Self>) {
        let Some(job) = self.dictation_job(cx) else {
            return;
        };
        let Some(d) = &mut self.dictation else {
            return;
        };
        if d.ready().is_none() {
            return;
        }
        d.busy = true;
        d.view = View::Loading;
        d.converse_view = View::Loading;
        let key = keys::dictation(cx);
        let converse = keys::conversation(cx);
        cx.spawn(async move |this, cx| {
            let read = cx
                .background_executor()
                .spawn(async move { read_view(&job.env, key, converse) })
                .await;
            let _ = this.update(cx, |this, cx| {
                if let Some(d) = &mut this.dictation {
                    d.view = read.dictation;
                    d.bound = read.bound;
                    d.converse_view = read.converse;
                    d.converse_bound = read.converse_bound;
                    d.busy = false;
                }
                cx.notify();
            });
        })
        .detach();
        cx.notify();
    }

    /// Waits for the next key press. Unlike the app keys, a key with `Super` is fine here.
    pub(super) fn begin_dictation_change(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self
            .dictation
            .as_ref()
            .is_none_or(|d| d.busy || d.ready().is_none())
        {
            return;
        }
        self.cancel_captures(cx);
        let guard = self.start_capture(window, cx, |this, captured, cx| {
            this.on_dictation_captured(captured, cx)
        });
        if let Some(d) = &mut self.dictation {
            d.note = None;
            d.capture = Some(guard);
        }
        cx.notify();
    }

    fn on_dictation_captured(&mut self, captured: Captured, cx: &mut Context<Self>) {
        let Some(d) = &mut self.dictation else {
            return;
        };
        d.capture = None;
        let Captured::Key(ks) = captured else {
            cx.notify();
            return;
        };
        if let Err(refusal) = keys::with(cx, |k| k.check_dictation(&ks)) {
            d.note = Some(refusal.to_string());
            cx.notify();
            return;
        }
        let gpui = keys::key_text(&ks, cx.platform_info().os);
        self.save_dictation_key(gpui, cx);
    }

    /// Waits for the next key press, for the conversation key.
    pub(super) fn begin_converse_change(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self
            .dictation
            .as_ref()
            .is_none_or(|d| d.busy || d.ready().is_none())
        {
            return;
        }
        self.cancel_captures(cx);
        let guard = self.start_capture(window, cx, |this, captured, cx| {
            this.on_converse_captured(captured, cx)
        });
        if let Some(d) = &mut self.dictation {
            d.note = None;
            d.converse_capture = Some(guard);
        }
        cx.notify();
    }

    fn on_converse_captured(&mut self, captured: Captured, cx: &mut Context<Self>) {
        let Some(d) = &mut self.dictation else {
            return;
        };
        d.converse_capture = None;
        let Captured::Key(ks) = captured else {
            cx.notify();
            return;
        };
        if let Err(refusal) = keys::with(cx, |k| k.check_conversation(&ks)) {
            d.note = Some(refusal.to_string());
            cx.notify();
            return;
        }
        let gpui = keys::key_text(&ks, cx.platform_info().os);
        self.save_converse_key(gpui, cx);
    }

    /// The dictation key of `keys.toml` in GPUI form, when it parses.
    fn dictation_gpui(cx: &App) -> Option<String> {
        keys::dictation(cx).and_then(|k| bind::from_hypr(&k).ok())
    }

    /// Saves the GPUI key `gpui` as the conversation key, off the UI thread.
    fn save_converse_key(&mut self, gpui: String, cx: &mut Context<Self>) {
        let Some(job) = self.dictation_job(cx) else {
            return;
        };
        let dictation = Self::dictation_gpui(cx);
        self.run_dictation_job(cx, move || job.save_converse(&gpui, dictation.as_deref()));
    }

    /// Writes the bind file again for the conversation key of `keys.toml`, with no capture.
    pub(super) fn rewrite_converse_key(&mut self, cx: &mut Context<Self>) {
        let Some(Ok(gpui)) = keys::conversation(cx).map(|k| bind::from_hypr(&k)) else {
            return;
        };
        self.save_converse_key(gpui, cx);
    }

    /// Writes the bind file with no conversation bind and drops the key from the key file.
    pub(super) fn remove_converse_key(&mut self, cx: &mut Context<Self>) {
        let Some(job) = self.dictation_job(cx) else {
            return;
        };
        if self.dictation.as_ref().is_some_and(|d| d.busy) {
            return;
        }
        if let Some(d) = &mut self.dictation {
            d.converse_capture = None;
        }
        let dictation = Self::dictation_gpui(cx);
        self.run_dictation_job(cx, move || job.remove_converse(dictation.as_deref()));
    }

    /// Saves the GPUI key `gpui` as the dictation key, off the UI thread.
    fn save_dictation_key(&mut self, gpui: String, cx: &mut Context<Self>) {
        let Some(job) = self.dictation_job(cx) else {
            return;
        };
        self.run_dictation_job(cx, move || job.save(&gpui));
    }

    /// Writes the bind file again for the key of `keys.toml`, with no capture.
    pub(super) fn rewrite_dictation_key(&mut self, cx: &mut Context<Self>) {
        let Some(Ok(gpui)) = keys::dictation(cx).map(|k| bind::from_hypr(&k)) else {
            return;
        };
        self.save_dictation_key(gpui, cx);
    }

    /// Saves the mode and writes the bind file again in its form.
    pub(super) fn set_dictation_mode(&mut self, mode: Mode, cx: &mut Context<Self>) {
        let Some(job) = self.dictation_job(cx) else {
            return;
        };
        let previous = job.env.setup.mode;
        let Some(d) = &self.dictation else {
            return;
        };
        if d.busy || previous == mode {
            return;
        }
        let gpui = keys::dictation(cx)
            .and_then(|k| bind::from_hypr(&k).ok())
            .filter(|_| d.ready().is_some());
        self.run_dictation_job(cx, move || job.set_mode(mode, previous, gpui.as_deref()));
    }

    /// Writes an empty bind file and drops the key from the key file.
    pub(super) fn remove_dictation_key(&mut self, cx: &mut Context<Self>) {
        let Some(job) = self.dictation_job(cx) else {
            return;
        };
        if self.dictation.as_ref().is_some_and(|d| d.busy) {
            return;
        }
        if let Some(d) = &mut self.dictation {
            d.capture = None;
        }
        self.run_dictation_job(cx, move || job.remove());
    }

    /// Runs a change on a background thread, then shows its result and reads the state.
    fn run_dictation_job(
        &mut self,
        cx: &mut Context<Self>,
        work: impl FnOnce() -> Result<Option<String>, String> + Send + 'static,
    ) {
        let Some(d) = &mut self.dictation else {
            return;
        };
        d.busy = true;
        d.note = None;
        cx.spawn(async move |this, cx| {
            let done = cx.background_executor().spawn(async move { work() }).await;
            let _ = this.update(cx, |this, cx| {
                let (written, note) = match done {
                    Ok(warning) => (true, warning),
                    Err(e) => (false, Some(e)),
                };
                if written {
                    keys::reload(cx);
                }
                if let Some(d) = &mut this.dictation {
                    d.note = note;
                    d.busy = false;
                }
                if written {
                    this.refresh_dictation(cx);
                }
                cx.notify();
            });
        })
        .detach();
        cx.notify();
    }

    /// Remove shows when the key file has a key, or the bind file holds a bind that the key
    /// file does not list.
    pub(super) fn dictation_can_remove(&self, cx: &App) -> bool {
        keys::dictation(cx).is_some() || self.dictation.as_ref().is_some_and(|d| d.bound)
    }

    pub(super) fn copy_include_line(&self, cx: &mut Context<Self>) {
        let line = self
            .dictation
            .as_ref()
            .and_then(|d| d.ready())
            .and_then(|e| e.setup.include_line());
        if let Some(line) = line {
            cx.write_to_clipboard(ClipboardItem::new_string(line));
        }
    }

    /// The choice between the hold form and the toggle form.
    fn dictation_mode_row(&self, busy: bool, cx: &mut Context<Self>) -> Div {
        let current = keys::dictation_mode(cx);
        let button = |id: &'static str, label: &'static str, mode: Mode, cx: &mut Context<Self>| {
            let b = Button::new(id).small().disabled(busy).label(label);
            let b = if current == mode {
                b.primary()
            } else {
                b.secondary()
            };
            b.on_click(cx.listener(move |this, _, _, cx| this.set_dictation_mode(mode, cx)))
        };
        pair(
            "Mode",
            div()
                .flex()
                .items_center()
                .gap(space(2))
                .child(button(
                    "dictation-mode-hold",
                    "Hold to dictate",
                    Mode::Hold,
                    cx,
                ))
                .child(button(
                    "dictation-mode-toggle",
                    "Press to start, press to stop",
                    Mode::Toggle,
                    cx,
                )),
        )
    }

    /// The section, or `None` when the screen has no dictation global (macOS).
    pub(super) fn dictation_section(&self, cx: &mut Context<Self>) -> Option<Div> {
        let d = self.dictation.as_ref()?;
        let head = div()
            .flex()
            .flex_col()
            .gap(space(3))
            .child(section("Dictation key", None));
        let line = |why: String| {
            div()
                .text_size(size::sm())
                .text_color(text::SECONDARY)
                .child(why)
        };
        let env = match &d.env {
            Err(why) => return Some(head.child(line(why.clone()))),
            Ok(env) => env,
        };
        let setup = &env.setup;
        if setup.form == Form::CommandsOnly {
            let mode_row = self.dictation_mode_row(d.busy, cx);
            let head = match keys::dictation_mode(cx) {
                Mode::Hold => {
                    let [start, stop] = setup.core.commands();
                    head.child(line("AgentSI found no Hyprland config. Bind these two commands in your compositor: the first on key press, the second on key release.".into()))
                        .child(mono(start))
                        .child(mono(stop))
                }
                Mode::Toggle => head
                    .child(line("AgentSI found no Hyprland config. Bind this command in your compositor, on key press only.".into()))
                    .child(mono(setup.core.command("toggle"))),
            };
            return Some(head.child(mode_row));
        }
        let key = keys::dictation(cx);
        let shown = if d.waiting() {
            div()
                .text_size(size::sm())
                .text_color(accent::BASE)
                .child("Press the new key. Escape cancels.")
                .into_any_element()
        } else {
            match key.as_deref().map(bind::from_hypr) {
                None => line("No key".into()).into_any_element(),
                Some(Ok(g)) => match keys::parse_key(&g, cx.platform_info().os) {
                    Ok(ks) => keys::hint_for(&ks).into_any_element(),
                    Err(_) => mono(key.clone().unwrap_or_default()).into_any_element(),
                },
                Some(Err(_)) => mono(key.clone().unwrap_or_default()).into_any_element(),
            }
        };
        let busy = d.busy;
        let tag_of = |view: &View| match view {
            View::Loading => neutral_tag("Reading the binds…"),
            View::NoKey => neutral_tag("No key"),
            View::State(State::Active) => status_tag("Active", Status::Ok),
            View::State(State::IncludeMissing) => status_tag("Add the include line", Status::Warn),
            View::State(State::OutOfDate) => {
                status_tag("The bind file is out of date", Status::Warn)
            }
            View::State(State::Taken(by)) => {
                status_tag(format!("The key is taken by {by}"), Status::Danger)
            }
            View::Failed(_) => status_tag("Not read", Status::Danger),
        };
        let state = tag_of(&d.view);
        let out_of_date = d.view == View::State(State::OutOfDate);
        let converse_key = keys::conversation(cx);
        let converse_waiting = d.waiting_converse();
        let converse_state = tag_of(&d.converse_view);
        let converse_out_of_date = d.converse_view == View::State(State::OutOfDate);
        let converse_shown = if converse_waiting {
            div()
                .text_size(size::sm())
                .text_color(accent::BASE)
                .child("Press the new key. Escape cancels.")
                .into_any_element()
        } else {
            match converse_key.as_deref().map(bind::from_hypr) {
                None => line("No key".into()).into_any_element(),
                Some(Ok(g)) => match keys::parse_key(&g, cx.platform_info().os) {
                    Ok(ks) => keys::hint_for(&ks).into_any_element(),
                    Err(_) => mono(converse_key.clone().unwrap_or_default()).into_any_element(),
                },
                Some(Err(_)) => mono(converse_key.clone().unwrap_or_default()).into_any_element(),
            }
        };
        let converse_can_remove = converse_key.is_some() || d.converse_bound;
        let converse_stray = converse_key.is_none() && d.converse_bound;
        let converse_row = pair(
            "Conversation key",
            div()
                .flex()
                .items_center()
                .gap(space(2))
                .child(converse_shown)
                .child(
                    Button::new("conversation-change")
                        .secondary()
                        .small()
                        .disabled(busy)
                        .label("Change")
                        .on_click(cx.listener(|this, _, window, cx| {
                            this.begin_converse_change(window, cx)
                        })),
                )
                .when(converse_can_remove, |el| {
                    el.child(
                        Button::new("conversation-remove")
                            .secondary()
                            .small()
                            .disabled(busy)
                            .label("Remove")
                            .on_click(cx.listener(|this, _, _, cx| this.remove_converse_key(cx))),
                    )
                })
                .child(converse_state)
                .when(converse_out_of_date, |el| {
                    el.child(
                        Button::new("conversation-rewrite")
                            .primary()
                            .small()
                            .disabled(busy)
                            .label("Write again")
                            .on_click(cx.listener(|this, _, _, cx| this.rewrite_converse_key(cx))),
                    )
                }),
        );
        let failed = match &d.view {
            View::Failed(e) => Some(e.clone()),
            _ => None,
        };
        let stray = key.is_none() && d.bound;
        let waiting = d.waiting();
        let can_remove = self.dictation_can_remove(cx);
        let include = setup.include_line();
        let key_row = pair(
            "Key",
            div()
                .flex()
                .items_center()
                .gap(space(2))
                .child(shown)
                .child(
                    Button::new("dictation-change")
                        .secondary()
                        .small()
                        .disabled(busy)
                        .label("Change")
                        .on_click(cx.listener(|this, _, window, cx| {
                            this.begin_dictation_change(window, cx)
                        })),
                )
                .when(can_remove, |el| {
                    el.child(
                        Button::new("dictation-remove")
                            .secondary()
                            .small()
                            .disabled(busy)
                            .label("Remove")
                            .on_click(cx.listener(|this, _, _, cx| this.remove_dictation_key(cx))),
                    )
                }),
        );
        let intro = match keys::dictation_mode(cx) {
            Mode::Hold => "Hold the key to dictate in any window.",
            Mode::Toggle => {
                "Press the key to start to dictate in any window, and press it again to stop."
            }
        };
        let mode_row = self.dictation_mode_row(busy, cx);
        Some(
            head.child(line(format!("{intro} Add the include line to your Hyprland config one time. After that, a new key needs no edit.")))
                .when(setup.form == Form::Conf, |el| el.child(line(CONF_RELOADS.into())))
                .child(mode_row)
                .child(div().when(waiting, |el| {
                    // A click in the row that waits keeps the capture.
                    el.on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                }).child(key_row))
                .child(pair(
                    "State",
                    div()
                        .flex()
                        .items_center()
                        .gap(space(2))
                        .child(state)
                        .when(out_of_date, |el| {
                            el.child(
                                Button::new("dictation-rewrite")
                                    .primary()
                                    .small()
                                    .disabled(busy)
                                    .label("Write again")
                                    .on_click(cx.listener(|this, _, _, cx| {
                                        this.rewrite_dictation_key(cx)
                                    })),
                            )
                        })
                        .child(
                            Button::new("dictation-check")
                                .secondary()
                                .small()
                                .disabled(busy)
                                .label("Check again")
                                .on_click(cx.listener(|this, _, _, cx| this.refresh_dictation(cx))),
                        ),
                ))
                .child(line("One press of the conversation key starts a talk with the focused pane, and the next press stops it.".into()))
                .child(div().when(converse_waiting, |el| {
                    el.on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                }).child(converse_row))
                .when(converse_stray, |el| {
                    el.child(line("The bind file holds a conversation key that keys.toml does not list. Remove clears it.".into()))
                })
                .when(stray, |el| {
                    el.child(line("The bind file holds a key that keys.toml does not list. Remove clears it.".into()))
                })
                .when_some(failed.or_else(|| d.note.clone()), |el, why| {
                    el.child(
                        div()
                            .text_size(size::sm())
                            .text_color(Status::Danger.color())
                            .child(why),
                    )
                })
                .when_some(include, |el, include| {
                    el.child(pair(
                        "Include line",
                        Button::new("dictation-copy")
                            .secondary()
                            .small()
                            .label("Copy")
                            .on_click(cx.listener(|this, _, _, cx| this.copy_include_line(cx))),
                    ))
                    .child(mono(include))
                }),
        )
    }
}

#[cfg(test)]
mod tests {
    use std::path::Path;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::{Arc, Mutex};

    use gpui::{TestAppContext, VisualTestContext};

    use super::super::tests::support::{Setup, setup_global, setup_panel, setup_with};
    use super::{DictationEnv, View};
    use crate::dictation_bind::{
        Bind, CoreCall, DESCRIPTION, Form, Hyprctl, Mode, Setup as BindSetup, State,
    };

    /// The fake `hyprctl`: no test runs the real one.
    #[derive(Default)]
    struct Fake {
        binds: Mutex<Vec<Bind>>,
        reads: AtomicUsize,
        reloads: AtomicUsize,
        /// The calls that change the compositor, in order: `reload` and `eval <code>`.
        log: Mutex<Vec<String>>,
    }

    impl Hyprctl for Fake {
        fn binds(&self) -> Result<Vec<Bind>, String> {
            self.reads.fetch_add(1, Ordering::SeqCst);
            Ok(self.binds.lock().unwrap().clone())
        }

        fn reload(&self) -> Result<(), String> {
            self.reloads.fetch_add(1, Ordering::SeqCst);
            self.log.lock().unwrap().push("reload".into());
            Ok(())
        }

        fn eval(&self, code: &str) -> Result<(), String> {
            self.log.lock().unwrap().push(format!("eval {code}"));
            Ok(())
        }
    }

    fn bind(key: &str, modmask: u32, description: &str) -> Bind {
        Bind {
            key: key.into(),
            modmask,
            description: description.into(),
            dispatcher: "__lua".into(),
            arg: "1".into(),
            mouse: false,
        }
    }

    /// The `Super` mask of `hyprctl binds -j`.
    const SUPER: u32 = 64;

    struct Rig {
        t: Setup,
        fake: Arc<Fake>,
        /// The folder of the bind file.
        dir: std::path::PathBuf,
    }

    /// What a rig does besides the defaults.
    #[derive(Default)]
    struct Opts {
        /// The screen lives in a `ScreenPanel`, as the dock holds it.
        panel: bool,
        /// The bind file holds this key before the screen exists.
        saved: Option<&'static str>,
    }

    fn rig<'a>(
        cx: &'a mut TestAppContext,
        form: Form,
        key_file: Option<&str>,
        binds: Vec<Bind>,
    ) -> (Rig, &'a mut VisualTestContext) {
        rig_in(cx, form, key_file, binds, Opts::default())
    }

    fn rig_in<'a>(
        cx: &'a mut TestAppContext,
        form: Form,
        key_file: Option<&str>,
        binds: Vec<Bind>,
        opts: Opts,
    ) -> (Rig, &'a mut VisualTestContext) {
        static N: AtomicUsize = AtomicUsize::new(0);
        let dir = std::env::temp_dir()
            .join(format!(
                "agentsi-dictation-{}-{}",
                std::process::id(),
                N.fetch_add(1, Ordering::Relaxed)
            ))
            .join("agentsi");
        let fake = Arc::new(Fake {
            binds: Mutex::new(binds),
            ..Default::default()
        });
        let env = DictationEnv {
            setup: BindSetup {
                mode: Mode::Hold,
                converse: None,
                form,
                dir: dir.clone(),
                core: CoreCall {
                    bin: "/opt/agentsi/agentsi-core".into(),
                    socket: None,
                },
            },
            hyprctl: fake.clone(),
        };
        if let Some(key) = opts.saved {
            env.setup.save(key, &*fake).unwrap();
            fake.reloads.store(0, Ordering::SeqCst);
            fake.log.lock().unwrap().clear();
        }
        let (t, cx) = if opts.panel {
            setup_panel(cx, key_file, Some(env))
        } else {
            setup_with(cx, key_file, Some(env))
        };
        (Rig { t, fake, dir }, cx)
    }

    fn view(r: &Rig, cx: &mut VisualTestContext) -> View {
        r.t.screen
            .read_with(cx, |s, _| s.dictation.as_ref().unwrap().view.clone())
    }

    fn note(r: &Rig, cx: &mut VisualTestContext) -> Option<String> {
        r.t.screen
            .read_with(cx, |s, _| s.dictation.as_ref().unwrap().note.clone())
    }

    /// The calls that changed the compositor, in order.
    fn applies(r: &Rig) -> Vec<String> {
        r.fake.log.lock().unwrap().clone()
    }

    fn bind_file(r: &Rig) -> Option<String> {
        std::fs::read_to_string(r.dir.join("hypr-dictation.lua")).ok()
    }

    fn read(path: &Path) -> String {
        std::fs::read_to_string(path).unwrap_or_default()
    }

    const KEY_F10: &str = "[dictation]\nkey = \"SUPER + F10\"\n";

    #[gpui::test]
    fn the_state_is_active_when_the_compositor_holds_the_bind(cx: &mut TestAppContext) {
        let ours = bind("F10", SUPER, DESCRIPTION);
        let opts = Opts {
            saved: Some("super-f10"),
            ..Opts::default()
        };
        let (r, cx) = rig_in(cx, Form::Lua, Some(KEY_F10), vec![ours], opts);
        cx.run_until_parked();
        assert_eq!(view(&r, cx), View::State(State::Active));
        assert_eq!(
            r.fake.reloads.load(Ordering::SeqCst),
            0,
            "a read never reloads"
        );
    }

    #[gpui::test]
    fn the_state_asks_for_the_include_line_when_the_bind_is_not_loaded(cx: &mut TestAppContext) {
        let (r, cx) = rig(cx, Form::Lua, Some(KEY_F10), vec![]);
        cx.run_until_parked();
        assert_eq!(view(&r, cx), View::State(State::IncludeMissing));
        // The empty bind file exists, so the include line is valid at once.
        assert_eq!(
            bind_file(&r).as_deref().map(|t| t.contains("hl.bind")),
            Some(false)
        );
    }

    #[gpui::test]
    fn the_state_names_the_bind_that_holds_the_key(cx: &mut TestAppContext) {
        let foreign = bind("F10", SUPER, "Launch the foreign tool");
        let (r, cx) = rig(cx, Form::Lua, Some(KEY_F10), vec![foreign]);
        cx.run_until_parked();
        assert_eq!(
            view(&r, cx),
            View::State(State::Taken("Launch the foreign tool".into()))
        );
    }

    #[gpui::test]
    fn the_binds_are_read_off_the_ui_thread(cx: &mut TestAppContext) {
        let (r, cx) = rig(cx, Form::Lua, Some(KEY_F10), vec![]);
        cx.run_until_parked();
        let reads = r.fake.reads.load(Ordering::SeqCst);
        assert_eq!(reads, 1);
        // The call returns with the view on "Reading", and the read has not run on this stack.
        r.t.screen.update(cx, |s, cx| s.refresh_dictation(cx));
        assert_eq!(r.fake.reads.load(Ordering::SeqCst), reads);
        assert_eq!(view(&r, cx), View::Loading);
        cx.run_until_parked();
        assert_eq!(r.fake.reads.load(Ordering::SeqCst), reads + 1);
        assert_ne!(view(&r, cx), View::Loading);
        // Nothing reads again while the screen is idle.
        cx.update(|window, cx| window.draw(cx).clear(cx));
        cx.run_until_parked();
        assert_eq!(r.fake.reads.load(Ordering::SeqCst), reads + 1);
    }

    #[gpui::test]
    fn a_new_key_with_super_is_saved_in_the_bind_file_and_the_key_file(cx: &mut TestAppContext) {
        let (r, cx) = rig(cx, Form::Lua, None, vec![]);
        cx.run_until_parked();
        assert_eq!(view(&r, cx), View::NoKey);
        r.t.screen
            .update_in(cx, |s, window, cx| s.begin_dictation_change(window, cx));
        cx.simulate_keystrokes("super-f10");
        cx.run_until_parked();
        assert_eq!(r.fake.reloads.load(Ordering::SeqCst), 0, "no reload");
        assert_eq!(applies(&r).len(), 1, "one eval");
        assert!(bind_file(&r).unwrap().contains("hl.bind(\"SUPER + F10\""));
        assert!(read(&r.t.file).contains("key = \"SUPER + F10\""));
        assert_eq!(note(&r, cx), None);
        assert_eq!(view(&r, cx), View::State(State::IncludeMissing));
    }

    fn converse_view(r: &Rig, cx: &mut VisualTestContext) -> View {
        r.t.screen.read_with(cx, |s, _| {
            s.dictation.as_ref().unwrap().converse_view.clone()
        })
    }

    #[gpui::test]
    fn the_conversation_key_is_saved_beside_the_dictation_key_and_removed_alone(
        cx: &mut TestAppContext,
    ) {
        let opts = Opts {
            saved: Some("super-f10"),
            ..Opts::default()
        };
        let (r, cx) = rig_in(cx, Form::Lua, Some(KEY_F10), vec![], opts);
        cx.run_until_parked();
        assert_eq!(converse_view(&r, cx), View::NoKey);
        r.t.screen
            .update_in(cx, |s, window, cx| s.begin_converse_change(window, cx));
        assert!(
            r.t.screen
                .read_with(cx, |s, _| s.dictation.as_ref().unwrap().waiting())
        );
        cx.simulate_keystrokes("super-f9");
        cx.run_until_parked();
        assert_eq!(note(&r, cx), None);
        // One file holds both binds, and the key file holds both keys.
        let file = bind_file(&r).unwrap();
        assert!(file.contains("hl.bind(\"SUPER + F10\""), "{file}");
        assert!(
            file.contains("state.converse = hl.bind(\"SUPER + F9\""),
            "{file}"
        );
        assert!(file.contains("converse toggle"), "{file}");
        let keys = read(&r.t.file);
        assert!(keys.contains("[conversation]"), "{keys}");
        assert!(keys.contains("key = \"SUPER + F9\""), "{keys}");
        assert!(keys.contains("key = \"SUPER + F10\""), "{keys}");
        assert_eq!(converse_view(&r, cx), View::State(State::IncludeMissing));

        // A new dictation key keeps the conversation bind.
        r.t.screen
            .update_in(cx, |s, window, cx| s.begin_dictation_change(window, cx));
        cx.simulate_keystrokes("super-f8");
        cx.run_until_parked();
        let file = bind_file(&r).unwrap();
        assert!(file.contains("hl.bind(\"SUPER + F8\""), "{file}");
        assert!(
            file.contains("state.converse = hl.bind(\"SUPER + F9\""),
            "{file}"
        );

        // The dictation key cannot take the conversation key, and the reverse.
        r.t.screen
            .update_in(cx, |s, window, cx| s.begin_dictation_change(window, cx));
        cx.simulate_keystrokes("super-f9");
        cx.run_until_parked();
        assert!(note(&r, cx).unwrap().contains("Conversation key"));
        r.t.screen
            .update_in(cx, |s, window, cx| s.begin_converse_change(window, cx));
        cx.simulate_keystrokes("super-f8");
        cx.run_until_parked();
        assert!(note(&r, cx).unwrap().contains("Dictation key"));

        // A remove drops the conversation bind and keeps the dictation bind.
        r.t.screen.update(cx, |s, cx| s.remove_converse_key(cx));
        cx.run_until_parked();
        let file = bind_file(&r).unwrap();
        assert!(file.contains("hl.bind(\"SUPER + F8\""), "{file}");
        assert!(!file.contains("converse toggle"), "{file}");
        let keys = read(&r.t.file);
        assert!(!keys.contains("conversation"), "{keys}");
        assert!(keys.contains("SUPER + F8"), "{keys}");
        assert_eq!(converse_view(&r, cx), View::NoKey);
        assert_eq!(r.fake.reloads.load(Ordering::SeqCst), 0, "eval only");
    }

    #[gpui::test]
    fn the_lua_form_applies_a_save_and_a_remove_by_eval_only(cx: &mut TestAppContext) {
        let (r, cx) = rig(cx, Form::Lua, None, vec![]);
        cx.run_until_parked();
        begin(&r, cx);
        cx.simulate_keystrokes("super-f10");
        cx.run_until_parked();
        r.t.screen.update(cx, |s, cx| s.remove_dictation_key(cx));
        cx.run_until_parked();
        let eval = format!(
            "eval dofile(\"{}\")",
            r.dir.join("hypr-dictation.lua").display()
        );
        assert_eq!(applies(&r), [eval.clone(), eval]);
        assert_eq!(r.fake.reloads.load(Ordering::SeqCst), 0);
    }

    #[gpui::test]
    fn the_conf_form_still_reloads_and_never_evals(cx: &mut TestAppContext) {
        let (r, cx) = rig(cx, Form::Conf, None, vec![]);
        cx.run_until_parked();
        begin(&r, cx);
        cx.simulate_keystrokes("super-f10");
        cx.run_until_parked();
        assert_eq!(applies(&r), ["reload"]);
        let conf = read(&r.dir.join("hypr-dictation.conf"));
        assert!(conf.contains("bindd = SUPER, F10"), "{conf}");
        assert!(super::CONF_RELOADS.contains("reloads Hyprland"));
    }

    #[gpui::test]
    fn a_taken_key_leaves_the_bind_file_and_the_key_file_as_they_were(cx: &mut TestAppContext) {
        let foreign = bind("F11", 0, "Launch the foreign tool");
        let (r, cx) = rig(cx, Form::Lua, Some(KEY_F10), vec![foreign]);
        cx.run_until_parked();
        let (bind_before, keys_before) = (bind_file(&r), read(&r.t.file));
        assert!(bind_before.is_some());
        r.t.screen
            .update_in(cx, |s, window, cx| s.begin_dictation_change(window, cx));
        cx.simulate_keystrokes("f11");
        cx.run_until_parked();
        let line = note(&r, cx).expect("a refusal");
        assert!(line.contains("taken by Launch the foreign tool"), "{line}");
        assert_eq!(bind_file(&r), bind_before);
        assert_eq!(read(&r.t.file), keys_before);
        assert_eq!(r.fake.reloads.load(Ordering::SeqCst), 0);
    }

    #[gpui::test]
    fn a_key_without_a_modifier_is_refused_and_escape_changes_nothing(cx: &mut TestAppContext) {
        let (r, cx) = rig(cx, Form::Lua, Some(KEY_F10), vec![]);
        cx.run_until_parked();
        let keys_before = read(&r.t.file);
        r.t.screen
            .update_in(cx, |s, window, cx| s.begin_dictation_change(window, cx));
        cx.simulate_keystrokes("y");
        cx.run_until_parked();
        assert!(note(&r, cx).unwrap().contains("Ctrl, Alt or Super"));
        r.t.screen
            .update_in(cx, |s, window, cx| s.begin_dictation_change(window, cx));
        assert!(
            r.t.screen
                .read_with(cx, |s, _| s.dictation.as_ref().unwrap().waiting())
        );
        cx.simulate_keystrokes("escape");
        cx.run_until_parked();
        assert!(
            !r.t.screen
                .read_with(cx, |s, _| s.dictation.as_ref().unwrap().waiting())
        );
        assert_eq!(read(&r.t.file), keys_before);
        assert_eq!(r.fake.reloads.load(Ordering::SeqCst), 0);
    }

    #[gpui::test]
    fn remove_writes_an_empty_bind_file_and_drops_the_key(cx: &mut TestAppContext) {
        let ours = bind("F10", SUPER, DESCRIPTION);
        let (r, cx) = rig(cx, Form::Lua, Some(KEY_F10), vec![ours]);
        cx.run_until_parked();
        r.t.screen.update(cx, |s, cx| s.remove_dictation_key(cx));
        cx.run_until_parked();
        assert_eq!(r.fake.reloads.load(Ordering::SeqCst), 0, "no reload");
        assert_eq!(applies(&r).len(), 1, "one eval");
        assert!(!bind_file(&r).unwrap().contains("hl.bind"));
        assert!(!read(&r.t.file).contains("dictation"));
        assert_eq!(view(&r, cx), View::NoKey);
    }

    #[gpui::test]
    fn the_press_to_start_form_writes_one_bind_with_no_release_and_hold_writes_it_back(
        cx: &mut TestAppContext,
    ) {
        let ours = bind("F10", SUPER, DESCRIPTION);
        let opts = Opts {
            saved: Some("super-f10"),
            ..Opts::default()
        };
        let (r, cx) = rig_in(cx, Form::Lua, Some(KEY_F10), vec![ours], opts);
        cx.run_until_parked();
        assert!(bind_file(&r).unwrap().contains("dictate stop"));

        r.t.screen
            .update(cx, |s, cx| s.set_dictation_mode(Mode::Toggle, cx));
        cx.run_until_parked();
        assert_eq!(applies(&r).len(), 1, "one eval");
        assert_eq!(r.fake.reloads.load(Ordering::SeqCst), 0, "no reload");
        let file = bind_file(&r).unwrap();
        assert!(file.contains("dictate toggle") && file.contains("AgentSI: press to dictate"));
        assert!(
            !file.contains("hl.on") && !file.contains("dictate start"),
            "{file}"
        );
        let keys = read(&r.t.file);
        assert!(
            keys.contains("mode = \"toggle\"") && keys.contains("key = \"SUPER + F10\""),
            "{keys}"
        );
        assert_eq!(note(&r, cx), None);
        assert_eq!(view(&r, cx), View::State(State::Active));

        // The same mode again changes nothing.
        r.t.screen
            .update(cx, |s, cx| s.set_dictation_mode(Mode::Toggle, cx));
        cx.run_until_parked();
        assert_eq!(applies(&r).len(), 1);

        r.t.screen
            .update(cx, |s, cx| s.set_dictation_mode(Mode::Hold, cx));
        cx.run_until_parked();
        assert_eq!(applies(&r).len(), 2);
        assert_eq!(r.fake.reloads.load(Ordering::SeqCst), 0);
        let file = bind_file(&r).unwrap();
        assert!(
            file.contains("dictate start") && file.contains("dictate stop"),
            "{file}"
        );
        assert!(!read(&r.t.file).contains("mode"));
        assert_eq!(view(&r, cx), View::State(State::Active));
    }

    #[gpui::test]
    fn with_no_key_the_mode_is_saved_and_no_bind_is_written(cx: &mut TestAppContext) {
        let (r, cx) = rig(cx, Form::Lua, None, vec![]);
        cx.run_until_parked();
        r.t.screen
            .update(cx, |s, cx| s.set_dictation_mode(Mode::Toggle, cx));
        cx.run_until_parked();
        assert_eq!(r.fake.reloads.load(Ordering::SeqCst), 0);
        assert!(read(&r.t.file).contains("mode = \"toggle\""));
        assert!(!bind_file(&r).unwrap().contains("hl.bind"));
        // The next key goes to the bind file in the toggle form.
        r.t.screen
            .update_in(cx, |s, window, cx| s.begin_dictation_change(window, cx));
        cx.simulate_keystrokes("super-f10");
        cx.run_until_parked();
        assert!(bind_file(&r).unwrap().contains("dictate toggle"));
    }

    #[gpui::test]
    fn a_bind_file_that_cannot_be_written_keeps_the_old_mode(cx: &mut TestAppContext) {
        let foreign = bind("F10", SUPER, "Launch the foreign tool");
        let (r, cx) = rig(cx, Form::Lua, Some(KEY_F10), vec![foreign]);
        cx.run_until_parked();
        let (bind_before, keys_before) = (bind_file(&r), read(&r.t.file));
        r.t.screen
            .update(cx, |s, cx| s.set_dictation_mode(Mode::Toggle, cx));
        cx.run_until_parked();
        assert!(
            note(&r, cx)
                .unwrap()
                .contains("taken by Launch the foreign tool")
        );
        assert_eq!(bind_file(&r), bind_before);
        assert_eq!(read(&r.t.file), keys_before);
        assert_eq!(r.fake.reloads.load(Ordering::SeqCst), 0);
    }

    #[gpui::test]
    fn without_hyprland_toggle_mode_shows_the_one_command(cx: &mut TestAppContext) {
        let keys = "[dictation]\nmode = \"toggle\"\n";
        let (r, cx) = rig(cx, Form::CommandsOnly, Some(keys), vec![]);
        cx.run_until_parked();
        r.t.screen.update(cx, |s, cx| {
            assert!(s.dictation_section(cx).is_some());
        });
        let command = r.t.screen.read_with(cx, |s, _| {
            s.dictation
                .as_ref()
                .unwrap()
                .env
                .as_ref()
                .unwrap()
                .setup
                .core
                .command("toggle")
        });
        assert!(command.ends_with("dictate toggle"));
        // The mode is saved with no bind file to write.
        r.t.screen
            .update(cx, |s, cx| s.set_dictation_mode(Mode::Hold, cx));
        cx.run_until_parked();
        assert!(!read(&r.t.file).contains("mode"));
        assert_eq!(r.fake.reloads.load(Ordering::SeqCst), 0);
    }

    #[gpui::test]
    fn the_copy_button_puts_the_include_line_on_the_clipboard(cx: &mut TestAppContext) {
        let (r, cx) = rig(cx, Form::Lua, None, vec![]);
        cx.run_until_parked();
        r.t.screen.update(cx, |s, cx| s.copy_include_line(cx));
        let want = format!(
            "pcall(dofile, \"{}\")",
            r.dir.join("hypr-dictation.lua").display()
        );
        let got = cx.read_from_clipboard().and_then(|c| c.text());
        assert_eq!(got.as_deref(), Some(want.as_str()));
    }

    #[gpui::test]
    fn without_hyprland_the_section_writes_nothing_and_shows_the_two_commands(
        cx: &mut TestAppContext,
    ) {
        let (r, cx) = rig(cx, Form::CommandsOnly, None, vec![]);
        cx.run_until_parked();
        assert_eq!(r.fake.reads.load(Ordering::SeqCst), 0);
        assert_eq!(r.fake.reloads.load(Ordering::SeqCst), 0);
        assert!(bind_file(&r).is_none());
        let [start, stop] = r.t.screen.read_with(cx, |s, _| {
            s.dictation
                .as_ref()
                .unwrap()
                .env
                .as_ref()
                .unwrap()
                .setup
                .core
                .commands()
        });
        assert!(start.ends_with("dictate start") && stop.ends_with("dictate stop"));
        r.t.screen.update(cx, |s, cx| {
            assert!(s.dictation_section(cx).is_some());
        });
    }

    #[gpui::test]
    fn a_screen_with_no_environment_has_no_section(cx: &mut TestAppContext) {
        let (t, cx) = setup_with(cx, None, None);
        t.screen.update(cx, |s, cx| {
            assert!(s.dictation.is_none());
            assert!(s.dictation_section(cx).is_none());
        });
    }

    fn begin(r: &Rig, cx: &mut VisualTestContext) {
        r.t.screen
            .update_in(cx, |s, window, cx| s.begin_dictation_change(window, cx));
    }

    #[gpui::test]
    fn a_dictation_capture_ends_when_the_tab_is_hidden_and_writes_nothing(cx: &mut TestAppContext) {
        use gpui_kit::component::dock::BasePanel as _;
        let opts = Opts {
            panel: true,
            ..Opts::default()
        };
        let (r, cx) = rig_in(cx, Form::Lua, Some(KEY_F10), vec![], opts);
        cx.run_until_parked();
        let (bind_before, keys_before) = (bind_file(&r), read(&r.t.file));
        let panel = r.t.panel.clone().unwrap();
        panel.update_in(cx, |p, window, cx| p.set_active(true, window, cx));
        begin(&r, cx);
        assert!(
            r.t.screen
                .read_with(cx, |s, _| s.dictation.as_ref().unwrap().waiting())
        );
        panel.update_in(cx, |p, window, cx| p.set_active(false, window, cx));
        assert!(
            !r.t.screen
                .read_with(cx, |s, _| s.dictation.as_ref().unwrap().waiting())
        );
        cx.simulate_keystrokes("super-f11");
        cx.run_until_parked();
        assert_eq!(bind_file(&r), bind_before);
        assert_eq!(read(&r.t.file), keys_before);
        assert_eq!(r.fake.reloads.load(Ordering::SeqCst), 0);
    }

    #[gpui::test]
    fn the_dictation_capture_refuses_program_keys_pane_keys_and_keys_of_the_app(
        cx: &mut TestAppContext,
    ) {
        let (r, cx) = rig(cx, Form::Lua, Some(KEY_F10), vec![]);
        cx.run_until_parked();
        let (bind_before, keys_before) = (bind_file(&r), read(&r.t.file));
        for (pressed, reason) in [
            ("ctrl-c", "key of terminal programs"),
            ("ctrl-shift-c", "key of the terminal pane"),
            ("ctrl-shift-d", "already the key of \"Front desk\""),
            ("ctrl-k", "already the key of \"Command palette\""),
        ] {
            begin(&r, cx);
            cx.simulate_keystrokes(pressed);
            cx.run_until_parked();
            let line = note(&r, cx).unwrap_or_else(|| panic!("{pressed} was not refused"));
            assert!(line.contains(reason), "{pressed}: {line}");
            assert_eq!(bind_file(&r), bind_before, "{pressed}");
            assert_eq!(read(&r.t.file), keys_before, "{pressed}");
        }
        assert_eq!(r.fake.reloads.load(Ordering::SeqCst), 0);
    }

    #[gpui::test]
    fn a_key_file_that_does_not_parse_stops_the_change_before_any_write(cx: &mut TestAppContext) {
        let (r, cx) = rig(cx, Form::Lua, Some("[dictation\nkey ="), vec![]);
        cx.run_until_parked();
        let (bind_before, keys_before) = (bind_file(&r), read(&r.t.file));
        begin(&r, cx);
        cx.simulate_keystrokes("super-f10");
        cx.run_until_parked();
        assert!(note(&r, cx).unwrap().contains("does not parse"));
        assert_eq!(bind_file(&r), bind_before);
        assert_eq!(read(&r.t.file), keys_before);
        assert_eq!(r.fake.reloads.load(Ordering::SeqCst), 0);
    }

    #[gpui::test]
    fn a_failed_write_of_the_key_file_puts_the_bind_file_back(cx: &mut TestAppContext) {
        use std::os::unix::fs::PermissionsExt as _;
        let ours = bind("F10", SUPER, DESCRIPTION);
        let opts = Opts {
            saved: Some("super-f10"),
            ..Opts::default()
        };
        let (r, cx) = rig_in(cx, Form::Lua, Some(KEY_F10), vec![ours], opts);
        cx.run_until_parked();
        let bind_before = bind_file(&r);
        assert!(bind_before.as_deref().unwrap().contains("SUPER + F10"));
        // The folder of the key file takes no new file, so the write fails after the check.
        let folder = r.t.file.parent().unwrap().to_path_buf();
        let open = std::fs::metadata(&folder).unwrap().permissions();
        std::fs::set_permissions(&folder, std::fs::Permissions::from_mode(0o555)).unwrap();
        let writable = std::fs::File::create(folder.join("probe")).is_ok();
        if !writable {
            begin(&r, cx);
            cx.simulate_keystrokes("super-f9");
            cx.run_until_parked();
        }
        std::fs::set_permissions(&folder, open).unwrap();
        if writable {
            return;
        }
        assert!(note(&r, cx).is_some(), "the failure shows");
        assert_eq!(bind_file(&r), bind_before, "the old state is back");
        assert!(read(&r.t.file).contains("SUPER + F10"));
    }

    #[gpui::test]
    fn remove_stays_reachable_when_the_bind_file_holds_a_key_and_the_key_file_has_none(
        cx: &mut TestAppContext,
    ) {
        let opts = Opts {
            saved: Some("super-f10"),
            ..Opts::default()
        };
        let (r, cx) = rig_in(cx, Form::Lua, None, vec![], opts);
        cx.run_until_parked();
        assert!(r.t.screen.update(cx, |s, cx| s.dictation_can_remove(cx)));
        r.t.screen.update(cx, |s, cx| s.remove_dictation_key(cx));
        cx.run_until_parked();
        assert!(!bind_file(&r).unwrap().contains("hl.bind"));
        assert!(!r.t.screen.update(cx, |s, cx| s.dictation_can_remove(cx)));
    }

    #[gpui::test]
    fn write_again_saves_the_same_key_with_no_capture_when_the_bind_file_is_out_of_date(
        cx: &mut TestAppContext,
    ) {
        let ours = bind("F10", SUPER, DESCRIPTION);
        let opts = Opts {
            saved: Some("super-f10"),
            ..Opts::default()
        };
        let (r, cx) = rig_in(cx, Form::Lua, Some(KEY_F10), vec![ours], opts);
        cx.run_until_parked();
        assert_eq!(view(&r, cx), View::State(State::Active));
        // The program that the bind file names moved since the file was written.
        let file = r.dir.join("hypr-dictation.lua");
        let old = std::fs::read_to_string(&file).unwrap();
        std::fs::write(
            &file,
            old.replace("/opt/agentsi/agentsi-core", "/gone/agentsi-core"),
        )
        .unwrap();
        r.t.screen.update(cx, |s, cx| s.refresh_dictation(cx));
        cx.run_until_parked();
        assert_eq!(view(&r, cx), View::State(State::OutOfDate));
        r.t.screen.update(cx, |s, cx| s.rewrite_dictation_key(cx));
        cx.run_until_parked();
        assert_eq!(std::fs::read_to_string(&file).unwrap(), old);
        assert_eq!(view(&r, cx), View::State(State::Active));
        assert_eq!(r.fake.reloads.load(Ordering::SeqCst), 0, "no reload");
        assert_eq!(applies(&r).len(), 1, "one eval");
        assert_eq!(note(&r, cx), None);
    }

    #[gpui::test]
    fn a_missing_core_program_leaves_one_line_in_the_section(cx: &mut TestAppContext) {
        let why = "AgentSI found no agentsi-core program, so it cannot write the dictation key.";
        let (t, cx) = setup_global(cx, super::DictationGlobal::Missing(why.into()));
        cx.run_until_parked();
        t.screen.update(cx, |s, cx| {
            assert!(s.dictation_section(cx).is_some());
            assert_eq!(
                s.dictation.as_ref().unwrap().env.as_ref().err(),
                Some(&why.to_string())
            );
        });
        // Nothing runs on a machine that cannot write the file.
        t.screen.update(cx, |s, cx| s.refresh_dictation(cx));
        cx.run_until_parked();
    }
}
