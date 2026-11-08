//! The call of one minutes (U26): a player that shows which role speaks, the listened
//! mark, and the render of a call that has no audio yet.
//!
//! The core decodes the file, old `call.webm` or new `call.opus`, and sends 24 kHz samples
//! in chunks of about a minute (`call.audio`). The screen plays them through `player`. The
//! samples load when the owner first presses play, so an open screen costs one `call.get`.
//!
//! Keys of the focused screen: `Space` plays or pauses, `Left` and `Right` move to the
//! previous and the next segment, `L` marks the call as listened or removes the mark. The
//! palette has an entry for each action, and `Ctrl+Shift+P` opens the call of the open
//! minutes. A call that plays to its end gets the listened mark.

mod player;

use agentsi_proto::call_api::{
    CallAudio, CallAudioParams, CallCancel, CallCancelParams, CallGet, CallGetParams,
    CallGetResult, CallHeard, CallHeardParams, CallRender, CallRenderParams, Segment, decode_pcm,
};
use agentsi_ui::components::{Button, Card, Force, neutral_tag, status_tag};
use agentsi_ui::tokens::{
    Status, accent, control, line, radius, size, space, status, surface, text, weight,
};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;
use std::time::Duration;

use super::ScreenSpec;
use super::parts::{
    Flash, Live, Load, column, first_screen_ready, flash_line, mono, notice, render_name,
    role_label, stat, stats,
};
use crate::model::CoreState;
use crate::nav::Nav;
use player::{Player, SPEEDS, Snapshot};

/// How often the position and the speaking role are read while a call plays.
const TICK: Duration = Duration::from_millis(50);
/// How often the screen reads a call that renders.
const POLL: Duration = Duration::from_secs(1);
/// The samples of one `call.audio` request: one minute.
const CHUNK: u64 = 60 * player::RATE as u64;
/// The role that speaks stays lit this long after its segment ends, as in the web player.
const HOLD: f64 = 0.15;

pub struct CallScreen {
    product: String,
    id: String,
    core: Entity<CoreState>,
    data: Load<Box<CallGetResult>>,
    player: Player,
    /// The file whose samples the tape holds: its format and its size.
    tape_of: Option<(String, u64)>,
    loading: bool,
    speed: usize,
    position: f64,
    ticker: Option<Task<()>>,
    poll: Option<Task<()>>,
    focus: FocusHandle,
    shown: bool,
    stamp: u64,
    busy: bool,
    flash: Option<Flash>,
    /// The name of the render that this screen started.
    render_name: Option<String>,
}

/// The line that tells what a listened mark did. A mark that exists already is no error.
fn heard_flash(result: Result<bool, String>, heard: bool, has_audio: bool) -> Flash {
    match result {
        Ok(true) if heard => Flash::Done("Marked as listened.".into()),
        Ok(true) => Flash::Done("Removed the listened mark.".into()),
        Ok(false) if heard && !has_audio => Flash::Failed("This call has no audio to mark.".into()),
        Ok(false) if heard => Flash::Done("The call was marked as listened already.".into()),
        Ok(false) => Flash::Done("The call had no listened mark.".into()),
        Err(e) => Flash::Failed(e),
    }
}

/// `m:ss`, or `h:mm:ss` from one hour.
fn clock(seconds: f64) -> String {
    let s = seconds.max(0.0) as u64;
    if s >= 3600 {
        format!("{}:{:02}:{:02}", s / 3600, s / 60 % 60, s % 60)
    } else {
        format!("{}:{:02}", s / 60, s % 60)
    }
}

/// The segment that holds time `t`: the last one that has started, with 50 ms of slack.
fn active_segment(segments: &[Segment], t: f64) -> Option<usize> {
    segments.iter().rposition(|s| s.start <= t + 0.05)
}

/// The role that speaks at `t` while the call plays.
fn speaking(segments: &[Segment], t: f64, playing: bool) -> Option<&str> {
    let s = &segments[active_segment(segments, t)?];
    (playing && t < s.end + HOLD).then_some(s.role.as_str())
}

/// The roles of the call in the order of their first segment.
fn participants(segments: &[Segment]) -> Vec<&str> {
    let mut roles: Vec<&str> = Vec::new();
    for s in segments {
        if !roles.contains(&s.role.as_str()) {
            roles.push(&s.role);
        }
    }
    roles
}

impl CallScreen {
    pub fn new(
        product: String,
        id: String,
        core: Entity<CoreState>,
        cx: &mut Context<Self>,
    ) -> Self {
        let mut screen = CallScreen {
            product,
            id,
            core,
            data: Load::Loading,
            player: Player::device(),
            tape_of: None,
            loading: false,
            speed: 0,
            position: 0.0,
            ticker: None,
            poll: None,
            focus: cx.focus_handle().tab_stop(true),
            shown: true,
            stamp: 0,
            busy: false,
            flash: None,
            render_name: None,
        };
        screen.stamp = Live::tick(cx);
        let me = cx.weak_entity();
        Calls::add(cx, me);
        screen.reload(cx);
        screen
    }

    pub fn focus_handle(&self) -> FocusHandle {
        self.focus.clone()
    }

    pub fn stamp(&self) -> u64 {
        self.stamp
    }

    pub fn shown(&self) -> bool {
        self.shown
    }

    pub fn show(&mut self, shown: bool, cx: &mut Context<Self>) {
        self.shown = shown;
        if shown {
            self.touch(cx);
        } else {
            self.pause(cx);
        }
    }

    fn touch(&mut self, cx: &mut Context<Self>) {
        self.stamp = Live::tick(cx);
    }

    fn call(&self) -> Option<&CallGetResult> {
        match &self.data {
            Load::Ready(c) => Some(c),
            _ => None,
        }
    }

    fn playing(&self) -> bool {
        self.player.shared.snapshot().playing
    }

    fn say(&mut self, flash: Flash, cx: &mut Context<Self>) {
        self.flash = Some(flash);
        cx.notify();
    }

    fn fail(&mut self, message: impl Into<String>, cx: &mut Context<Self>) {
        self.say(Flash::Failed(message.into()), cx);
    }

    /// Reads the call again. A file that changed since the samples loaded drops the tape.
    pub fn reload(&mut self, cx: &mut Context<Self>) {
        let call = self.core.read(cx).link.call::<CallGet>(&CallGetParams {
            product: self.product.clone(),
            minutes_id: self.id.clone(),
        });
        cx.spawn(async move |this, cx| {
            let result = call.await.map(Box::new).map_err(|e| e.to_string());
            let _ = this.update(cx, |this, cx| {
                this.busy = false;
                if let Ok(c) = &result {
                    this.sync_tape(c.audio.as_ref().map(|a| (a.format.clone(), a.bytes)), cx);
                }
                this.data.settle(result);
                this.poll_while_rendering(cx);
                first_screen_ready();
                cx.notify();
            });
        })
        .detach();
    }

    /// The tape belongs to the file that it was loaded from, or that is loading now. When the call
    /// has another file, the samples go and the owner starts again.
    fn sync_tape(&mut self, file: Option<(String, u64)>, cx: &mut Context<Self>) {
        if file == self.tape_of {
            return;
        }
        self.pause(cx);
        self.player.shared.clear();
        self.tape_of = None;
        self.position = 0.0;
    }

    /// While a render runs, reads the call every second to show its progress.
    fn poll_while_rendering(&mut self, cx: &mut Context<Self>) {
        let rendering = self.call().is_some_and(|c| c.render.is_some());
        if !rendering && self.render_name.is_none() {
            self.poll = None;
            return;
        }
        self.poll = Some(cx.spawn(async move |this, cx| {
            cx.background_executor().timer(POLL).await;
            let _ = this.update(cx, |this, cx| {
                this.poll = None;
                this.reload(cx);
            });
        }));
    }

    /// Loads the samples of the call, chunk by chunk, once.
    fn load_tape(&mut self, cx: &mut Context<Self>) {
        let Some(audio) = self.call().and_then(|c| c.audio.clone()) else {
            return;
        };
        if self.loading || self.tape_of.is_some() {
            return;
        }
        self.loading = true;
        // Set at the start: a reload while the samples load finds the same file and keeps them.
        let key = (audio.format, audio.bytes);
        self.tape_of = Some(key.clone());
        let (link, product, id) = (
            self.core.read(cx).link.clone(),
            self.product.clone(),
            self.id.clone(),
        );
        let shared = self.player.shared.clone();
        cx.spawn(async move |this, cx| {
            let mut from = 0u64;
            let outcome = loop {
                let reply = link
                    .call::<CallAudio>(&CallAudioParams {
                        product: product.clone(),
                        minutes_id: id.clone(),
                        from,
                        count: CHUNK,
                    })
                    .await;
                let chunk = match reply {
                    Ok(c) => c,
                    Err(e) => break Err(e.to_string()),
                };
                let Some(samples) = decode_pcm(&chunk.pcm) else {
                    break Err("The core sent audio that cannot be read.".to_string());
                };
                // A tape that was cleared meanwhile belongs to another file: stop.
                if shared.len() as u64 != from {
                    break Ok(false);
                }
                shared.append(&samples);
                from += samples.len() as u64;
                if samples.is_empty() || from >= chunk.total {
                    shared.finish_loading();
                    break Ok(true);
                }
            };
            let _ = this.update(cx, |this, cx| {
                this.loading = false;
                match outcome {
                    Ok(_) => {}
                    Err(e) => {
                        // Half a tape would play wrong: drop it, and the next play loads again.
                        if this.tape_of.as_ref() == Some(&key) {
                            this.player.shared.clear();
                            this.tape_of = None;
                        }
                        this.fail(format!("The audio did not load: {e}"), cx)
                    }
                }
                cx.notify();
            });
        })
        .detach();
    }

    /// Plays from the position, or pauses.
    pub fn toggle_play(&mut self, cx: &mut Context<Self>) {
        self.touch(cx);
        if self.playing() {
            self.pause(cx);
            return;
        }
        let Some(c) = self.call() else { return };
        if c.audio.is_none() {
            self.fail("This call has no audio. Render it first.", cx);
            return;
        }
        if !c.timed {
            self.fail(
                "This call has no timings, so it cannot play. Render it again.",
                cx,
            );
            return;
        }
        self.load_tape(cx);
        self.flash = None;
        match self.player.play() {
            Ok(()) => self.start_ticker(cx),
            Err(e) => self.fail(e, cx),
        }
        cx.notify();
    }

    pub fn pause(&mut self, cx: &mut Context<Self>) {
        self.player.pause();
        self.ticker = None;
        self.position = self.player.shared.snapshot().position;
        cx.notify();
    }

    fn start_ticker(&mut self, cx: &mut Context<Self>) {
        self.ticker = Some(cx.spawn(async move |this, cx| {
            loop {
                cx.background_executor().timer(TICK).await;
                let go = this.update(cx, |this, cx| this.tick(cx)).unwrap_or(false);
                if !go {
                    break;
                }
            }
        }));
    }

    /// Reads the player. Returns false when the ticker can stop.
    fn tick(&mut self, cx: &mut Context<Self>) -> bool {
        let snap: Snapshot = self.player.shared.snapshot();
        self.position = snap.position;
        cx.notify();
        if self.player.shared.take_ended() {
            self.player.pause();
            self.ticker = None;
            self.finished(cx);
            return false;
        }
        snap.playing
    }

    /// The call played to its end: it gets the listened mark.
    fn finished(&mut self, cx: &mut Context<Self>) {
        self.position = 0.0;
        self.player.shared.seek(0.0);
        if self.call().is_some_and(|c| !c.heard) {
            self.set_heard(true, cx);
        }
    }

    pub fn set_heard(&mut self, heard: bool, cx: &mut Context<Self>) {
        self.touch(cx);
        if self.busy {
            return;
        }
        self.busy = true;
        self.flash = None;
        let call = self.core.read(cx).link.call::<CallHeard>(&CallHeardParams {
            product: self.product.clone(),
            minutes_id: self.id.clone(),
            heard,
        });
        cx.spawn(async move |this, cx| {
            let result = call.await;
            let _ = this.update(cx, |this, cx| {
                this.busy = false;
                let has_audio = this.call().is_some_and(|c| c.audio.is_some());
                this.flash = Some(heard_flash(
                    result.map(|r| r.changed).map_err(|e| e.to_string()),
                    heard,
                    has_audio,
                ));
                this.reload(cx);
            });
        })
        .detach();
        cx.notify();
    }

    pub fn toggle_heard(&mut self, cx: &mut Context<Self>) {
        let heard = self.call().is_some_and(|c| c.heard);
        self.set_heard(!heard, cx);
    }

    /// Moves to the start of segment `idx`, and plays from there when `play` is set.
    fn seek_segment(&mut self, idx: usize, play: bool, cx: &mut Context<Self>) {
        self.touch(cx);
        let Some(start) = self
            .call()
            .and_then(|c| c.segments.get(idx))
            .map(|s| s.start)
        else {
            return;
        };
        self.player.shared.seek(start);
        self.position = start;
        if play && !self.playing() {
            self.toggle_play(cx);
        }
        cx.notify();
    }

    /// The previous (-1) or the next (1) segment. A step back inside a segment goes to its start.
    pub fn step(&mut self, delta: isize, cx: &mut Context<Self>) {
        let Some(c) = self.call().filter(|c| c.timed) else {
            return;
        };
        let now = active_segment(&c.segments, self.position).unwrap_or(0);
        let inside = self.position - c.segments[now].start;
        let to = if delta < 0 && inside > 1.0 {
            now
        } else {
            now.saturating_add_signed(delta).min(c.segments.len() - 1)
        };
        let play = self.playing();
        self.seek_segment(to, play, cx);
    }

    pub fn cycle_speed(&mut self, cx: &mut Context<Self>) {
        self.touch(cx);
        self.speed = (self.speed + 1) % SPEEDS.len();
        self.player.shared.set_speed(SPEEDS[self.speed]);
        cx.notify();
    }

    /// Renders the call with the voices of the core. It takes minutes.
    pub fn render_call(&mut self, cx: &mut Context<Self>) {
        self.touch(cx);
        if self.render_name.is_some() || self.call().is_some_and(|c| c.render.is_some()) {
            return;
        }
        let name = render_name();
        self.render_name = Some(name.clone());
        self.flash = None;
        let call = self
            .core
            .read(cx)
            .link
            .call::<CallRender>(&CallRenderParams {
                product: self.product.clone(),
                minutes_id: self.id.clone(),
                render: Some(name),
            });
        cx.spawn(async move |this, cx| {
            let result = call.await;
            let _ = this.update(cx, |this, cx| {
                this.render_name = None;
                this.flash = Some(match result {
                    Ok(r) => Flash::Done(format!(
                        "Rendered {} segments, {}.",
                        r.segments,
                        clock(r.duration)
                    )),
                    Err(e) => Flash::Failed(e.to_string()),
                });
                this.reload(cx);
            });
        })
        .detach();
        self.poll_while_rendering(cx);
        cx.notify();
    }

    /// Stops the render of this call, also one that the CLI started.
    pub fn cancel_render(&mut self, cx: &mut Context<Self>) {
        self.touch(cx);
        let name = self
            .render_name
            .clone()
            .or_else(|| self.call()?.render.as_ref().map(|r| r.render.clone()));
        let Some(render) = name else { return };
        let call = self
            .core
            .read(cx)
            .link
            .call::<CallCancel>(&CallCancelParams { render });
        cx.spawn(async move |this, cx| {
            let result = call.await;
            let _ = this.update(cx, |this, cx| {
                if let Err(e) = result {
                    this.fail(e.to_string(), cx);
                }
                this.reload(cx);
            });
        })
        .detach();
    }

    fn open_minutes(&self, cx: &mut Context<Self>) {
        Nav::open(cx, &self.core, ScreenSpec::minute(&self.product, &self.id));
    }

    fn key_down(&mut self, e: &KeyDownEvent, cx: &mut Context<Self>) {
        let k = &e.keystroke;
        if k.modifiers.modified() {
            return;
        }
        match k.key.as_str() {
            "space" => self.toggle_play(cx),
            "left" => self.step(-1, cx),
            "right" => self.step(1, cx),
            "l" => self.toggle_heard(cx),
            _ => return,
        }
        cx.stop_propagation();
    }
}

/// The call screens that are open, so a palette action finds the one the owner used last.
#[derive(Default)]
pub struct Calls(Vec<WeakEntity<CallScreen>>);

impl Global for Calls {}

impl Calls {
    fn add(cx: &mut App, screen: WeakEntity<CallScreen>) {
        let calls = cx.default_global::<Calls>();
        calls.0.retain(|w| w.upgrade().is_some());
        calls.0.push(screen);
    }

    /// The call screen in view that the owner used last.
    pub fn last(cx: &App) -> Option<Entity<CallScreen>> {
        cx.try_global::<Calls>()?
            .0
            .iter()
            .filter_map(WeakEntity::upgrade)
            .filter(|c| c.read(cx).shown())
            .max_by_key(|c| c.read(cx).stamp())
    }
}

fn speaker(role: &str, lit: bool) -> Div {
    let initial = role_label(role).chars().next().unwrap_or('?').to_string();
    div()
        .flex()
        .flex_col()
        .items_center()
        .gap(space(2))
        .child(
            div()
                .size(control::row())
                .flex()
                .items_center()
                .justify_center()
                .rounded(radius::FULL)
                .border_2()
                .border_color(if lit { status::OK } else { line::BORDER })
                .bg(if lit {
                    Status::Ok.tint()
                } else {
                    surface::RAISED
                })
                .text_size(size::lg())
                .font_weight(weight::SEMIBOLD)
                .text_color(if lit { status::OK } else { text::SECONDARY })
                .child(initial),
        )
        .child(
            div()
                .text_size(size::sm())
                .text_color(if lit { text::PRIMARY } else { text::TERTIARY })
                .child(role_label(role)),
        )
}

impl Focusable for CallScreen {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus.clone()
    }
}

impl Render for CallScreen {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let root = div()
            .id("call")
            .track_focus(&self.focus)
            .key_context("Call")
            .on_key_down(cx.listener(|this, e: &KeyDownEvent, _, cx| this.key_down(e, cx)))
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(|this, _, _, cx| this.touch(cx)),
            )
            .size_full()
            .bg(surface::PANEL);
        let c = match &self.data {
            Load::Loading => return root.child(notice("Reading the call…", None)),
            Load::Failed(e) => {
                return root.child(notice(
                    format!("This call did not load: {e}"),
                    Some(super::parts::reload_hint(cx)),
                ));
            }
            Load::Ready(c) => c,
        };
        let snap = self.player.shared.snapshot();
        let playing = snap.playing;
        let t = if playing {
            snap.position
        } else {
            self.position
        };
        let rendering = c.render.clone();
        let own_render = self.render_name.is_some();
        let render_busy = rendering.is_some() || own_render;
        let total = c
            .segments
            .iter()
            .fold(0.0f64, |m, s| m.max(s.end))
            .max(snap.loaded);
        let active = if c.timed {
            active_segment(&c.segments, t)
        } else {
            None
        };
        let lit = c
            .timed
            .then(|| speaking(&c.segments, t, playing))
            .flatten()
            .map(String::from);
        let can_play = c.audio.is_some() && c.timed && !render_busy;

        let state_line = if let Some(r) = &rendering {
            format!("Rendering: segment {} of {}…", r.done, r.of)
        } else if own_render {
            "Rendering…".to_string()
        } else if c.audio.is_none() {
            "No audio".to_string()
        } else if !c.timed {
            "No timings".to_string()
        } else if snap.buffering {
            "Loading the audio…".to_string()
        } else if playing {
            "In call".to_string()
        } else {
            "Paused".to_string()
        };
        let place = match active {
            Some(i) if c.timed => format!(
                "segment {} of {} · {} of {}",
                i + 1,
                c.segments.len(),
                clock(t),
                clock(total)
            ),
            _ => format!("{} segments · {}", c.segments.len(), clock(total)),
        };

        let toolbar = div()
            .flex()
            .flex_wrap()
            .items_center()
            .gap(space(2))
            .child(
                Button::new("call-play")
                    .primary()
                    .label(if playing { "Pause" } else { "Play" })
                    .disabled(!can_play)
                    .on_click(cx.listener(|this, _, _, cx| this.toggle_play(cx))),
            )
            .child(
                Button::new("call-back")
                    .ghost()
                    .small()
                    .label("Previous")
                    .disabled(!can_play)
                    .on_click(cx.listener(|this, _, _, cx| this.step(-1, cx))),
            )
            .child(
                Button::new("call-next")
                    .ghost()
                    .small()
                    .label("Next")
                    .disabled(!can_play)
                    .on_click(cx.listener(|this, _, _, cx| this.step(1, cx))),
            )
            .child(
                Button::new("call-speed")
                    .ghost()
                    .small()
                    .label(format!("Speed {}×", SPEEDS[self.speed]))
                    .on_click(cx.listener(|this, _, _, cx| this.cycle_speed(cx))),
            )
            .child(
                Button::new("call-heard")
                    .secondary()
                    .small()
                    .label(if c.heard {
                        "Remove the listened mark"
                    } else {
                        "Mark as listened"
                    })
                    .disabled(self.busy || c.audio.is_none())
                    .on_click(cx.listener(|this, _, _, cx| this.toggle_heard(cx))),
            )
            .child(if render_busy {
                Button::new("call-render")
                    .danger()
                    .small()
                    .label("Cancel the render")
                    .on_click(cx.listener(|this, _, _, cx| this.cancel_render(cx)))
            } else {
                Button::new("call-render")
                    .ghost()
                    .small()
                    .label(if c.audio.is_some() {
                        "Render again"
                    } else {
                        "Render the call"
                    })
                    .disabled(c.segments.is_empty())
                    .on_click(cx.listener(|this, _, _, cx| this.render_call(cx)))
            })
            .child(
                Button::new("call-minutes")
                    .ghost()
                    .small()
                    .label("Open the minutes")
                    .on_click(cx.listener(|this, _, _, cx| this.open_minutes(cx))),
            );

        let header = div()
            .flex()
            .flex_col()
            .gap(space(3))
            .child(mono(c.minutes_id.clone()).truncate())
            .child(
                div()
                    .text_size(size::lg())
                    .font_weight(weight::SEMIBOLD)
                    .text_color(text::PRIMARY)
                    .child(c.question.clone()),
            )
            .child(
                div()
                    .flex()
                    .flex_wrap()
                    .items_center()
                    .gap(space(3))
                    .child(if playing {
                        status_tag(state_line.clone(), Status::Ok)
                    } else {
                        neutral_tag(state_line.clone())
                    })
                    .when(c.heard, |el| el.child(status_tag("listened", Status::Ok)))
                    .when_some(c.audio.as_ref(), |el, a| {
                        el.child(neutral_tag(format!("{} file", a.format)))
                    })
                    .child(
                        div()
                            .text_size(size::sm())
                            .text_color(text::TERTIARY)
                            .child(place),
                    ),
            );

        let stage = div()
            .flex()
            .flex_wrap()
            .justify_center()
            .gap_x(space(8))
            .gap_y(space(4))
            .p(space(6))
            .rounded(radius::lg())
            .border_1()
            .border_color(line::BORDER)
            .bg(surface::RAISED)
            .children(
                participants(&c.segments)
                    .into_iter()
                    .map(|r| speaker(r, lit.as_deref() == Some(r))),
            );

        let segments =
            div()
                .flex()
                .flex_col()
                .gap(space(2))
                .children(c.segments.iter().enumerate().map(|(i, s)| {
                    let now = active == Some(i) && c.timed;
                    Card::new(("segment", i))
                        .force(if now { Force::Hover } else { Force::None })
                        .on_click(cx.listener(move |this, _, _, cx| {
                            this.seek_segment(i, true, cx);
                        }))
                        .child(
                            div()
                                .flex()
                                .items_baseline()
                                .gap(space(3))
                                .child(
                                    div()
                                        .text_size(size::sm())
                                        .font_weight(weight::SEMIBOLD)
                                        .text_color(if now { accent::BASE } else { text::PRIMARY })
                                        .child(role_label(&s.role)),
                                )
                                .when(c.timed, |el| el.child(mono(clock(s.start)))),
                        )
                        .child(
                            div()
                                .text_size(size::base())
                                .text_color(text::SECONDARY)
                                .child(s.text.clone()),
                        )
                }));

        let figures = stats(vec![
            stat("Segments", c.segments.len().to_string()),
            stat("Length", clock(total)),
            stat("Roles", participants(&c.segments).len().to_string()),
            stat("Listened", if c.heard { "yes" } else { "no" }),
        ]);

        root.overflow_y_scroll().child(
            column()
                .child(header)
                .children(self.flash.as_ref().map(flash_line))
                .when(c.segments.is_empty(), |el| {
                    el.child(notice(
                        "These minutes have no call script.",
                        Some("A meeting writes a script with its synthesis.".into()),
                    ))
                })
                .child(toolbar)
                .child(stage)
                .child(figures)
                .child(segments),
        )
    }
}

#[cfg(test)]
mod tests {
    // Not `use super::*`: it would bring in the `test` macro of `gpui_kit`.
    use super::player::{Output, Player};
    use super::{CallScreen, Load, Segment, active_segment, clock, participants, speaking};
    use agentsi_proto::call_api::CallGetResult;
    use serde_json::{Value, json};
    use std::sync::{Arc, Mutex};

    fn segments() -> Vec<Segment> {
        let at = |idx, role: &str, start, end| Segment {
            idx,
            role: role.into(),
            text: format!("Segment {idx}."),
            start,
            end,
        };
        vec![
            at(0, "pm", 0.0, 3.0),
            at(1, "dev", 3.35, 6.0),
            at(2, "pm", 6.35, 9.0),
        ]
    }

    #[test]
    fn a_mark_that_exists_already_is_no_error_and_a_call_with_no_audio_is() {
        use super::{Flash, heard_flash};
        assert_eq!(
            heard_flash(Ok(true), true, true),
            Flash::Done("Marked as listened.".into())
        );
        assert!(
            matches!(heard_flash(Ok(false), true, true), Flash::Done(m) if m.contains("already"))
        );
        assert!(
            matches!(heard_flash(Ok(false), true, false), Flash::Failed(m) if m.contains("no audio"))
        );
        assert!(matches!(
            heard_flash(Ok(false), false, true),
            Flash::Done(_)
        ));
        assert_eq!(
            heard_flash(Err("x".into()), true, true),
            Flash::Failed("x".into())
        );
    }

    #[gpui::test]
    fn a_reload_while_the_audio_loads_keeps_the_play_going(cx: &mut gpui::TestAppContext) {
        let ui = ui(cx, call("{}"));
        ui.screen.update(ui.cx, |s, cx| s.toggle_play(cx));
        ui.cx.run_until_parked();
        // The loader runs: the reload of a poll or of a data change finds the same file.
        let (loading, tape_of) = ui
            .screen
            .read_with(ui.cx, |s, _| (s.loading, s.tape_of.clone()));
        assert!(loading);
        assert_eq!(tape_of, Some(("webm".into(), 37995)));
        ui.screen
            .update(ui.cx, |s, cx| s.sync_tape(Some(("webm".into(), 37995)), cx));
        assert!(
            ui.screen.read_with(ui.cx, |s, _| s.playing()),
            "still playing"
        );
    }

    #[gpui::test]
    fn a_step_before_the_first_play_makes_the_call_start_at_that_segment(
        cx: &mut gpui::TestAppContext,
    ) {
        let mut ui = ui(cx, call("{}"));
        ui.screen.update(ui.cx, |s, cx| s.step(1, cx));
        assert_eq!(ui.screen.read_with(ui.cx, |s, _| s.position), 3.35);
        // The samples load after the seek. The device then reads from the segment.
        ui.fill_tape(10);
        ui.screen.update(ui.cx, |s, cx| s.toggle_play(cx));
        ui.device(24_000);
        ui.tick();
        let pos = ui.screen.read_with(ui.cx, |s, _| s.position);
        assert!((pos - 4.35).abs() < 1e-6, "{pos}");
    }

    #[test]
    fn a_clock_reads_minutes_and_hours() {
        assert_eq!(clock(0.0), "0:00");
        assert_eq!(clock(61.9), "1:01");
        assert_eq!(clock(3600.0 + 125.0), "1:02:05");
        assert_eq!(clock(-4.0), "0:00");
    }

    #[test]
    fn the_active_segment_is_the_last_one_that_started() {
        let s = segments();
        assert_eq!(active_segment(&s, 0.0), Some(0));
        assert_eq!(active_segment(&s, 3.31), Some(1), "50 ms of slack");
        assert_eq!(active_segment(&s, 3.2), Some(0));
        assert_eq!(active_segment(&s, 99.0), Some(2));
        assert_eq!(active_segment(&[], 1.0), None);
    }

    #[test]
    fn a_role_speaks_during_its_segment_and_a_moment_after_and_only_while_playing() {
        let s = segments();
        assert_eq!(speaking(&s, 1.0, true), Some("pm"));
        assert_eq!(speaking(&s, 1.0, false), None);
        assert_eq!(speaking(&s, 3.1, true), Some("pm"), "the hold of 150 ms");
        assert_eq!(
            speaking(&s, 3.2, true),
            None,
            "the pause between two segments"
        );
        assert_eq!(speaking(&s, 4.0, true), Some("dev"));
        // The pause after the last segment is silent.
        assert_eq!(speaking(&s, 9.2, true), None);
    }

    #[test]
    fn the_roles_of_a_call_come_in_the_order_of_their_first_segment() {
        assert_eq!(participants(&segments()), ["pm", "dev"]);
    }

    struct Silent(Arc<Mutex<Vec<&'static str>>>);

    impl Output for Silent {
        fn play(&mut self) -> Result<(), String> {
            self.0.lock().unwrap().push("play");
            Ok(())
        }
        fn pause(&mut self) {
            self.0.lock().unwrap().push("pause");
        }
    }

    fn call(extra: &str) -> CallGetResult {
        let mut v: Value = json!({
            "product": "demo", "minutes_id": "m1", "question": "Does it play?",
            "segments": segments(), "timed": true,
            "audio": {"format": "webm", "bytes": 37995}, "heard": false
        });
        let extra: Value = serde_json::from_str(extra).unwrap();
        v.as_object_mut()
            .unwrap()
            .extend(extra.as_object().unwrap().clone());
        serde_json::from_value(v).unwrap()
    }

    fn sent(other: &mut std::os::unix::net::UnixStream) -> Option<Value> {
        other.set_nonblocking(true).unwrap();
        let frame = agentsi_proto::frame::read_frame(other).ok().flatten()?;
        serde_json::from_slice(&frame.payload).ok()
    }

    struct Ui<'a> {
        screen: gpui::Entity<CallScreen>,
        other: std::os::unix::net::UnixStream,
        log: Arc<Mutex<Vec<&'static str>>>,
        cx: &'a mut gpui::VisualTestContext,
    }

    /// A call screen on a fake core, with an output that makes no sound.
    fn ui(cx: &mut gpui::TestAppContext, data: CallGetResult) -> Ui<'_> {
        use crate::link::Link;
        use crate::model::CoreState;
        use gpui_kit::AppContext as _;
        cx.update(agentsi_ui::init);
        let (end, mut other) = std::os::unix::net::UnixStream::pair().unwrap();
        let core = cx.new(|_| CoreState::new(Arc::new(Link::for_tests(end))));
        let (screen, cx) =
            cx.add_window_view(|_, cx| CallScreen::new("demo".into(), "m1".into(), core, cx));
        assert_eq!(sent(&mut other).unwrap()["method"], "call.get");
        let log = Arc::new(Mutex::new(Vec::new()));
        let l = log.clone();
        screen.update(cx, |s, cx| {
            s.player = Player::new(Box::new(move |_| Ok(Box::new(Silent(l.clone())))));
            s.data = Load::Ready(Box::new(data));
            cx.notify();
        });
        cx.update(|window, cx| window.draw(cx).clear(cx));
        Ui {
            screen,
            other,
            log,
            cx,
        }
    }

    impl Ui<'_> {
        fn draw(&mut self) {
            self.cx.update(|window, cx| window.draw(cx).clear(cx));
        }

        /// The samples of the call, as the chunks of `call.audio` would have loaded them.
        fn load_tape(&mut self, seconds: usize) {
            self.screen.update(self.cx, |s, _| {
                s.player.shared.append(&vec![3_000i16; seconds * 24_000]);
                s.player.shared.finish_loading();
                s.tape_of = Some(("webm".into(), 37995));
            });
        }

        /// Samples that a chunk of `call.audio` has loaded, before the screen knows the file.
        fn fill_tape(&mut self, seconds: usize) {
            self.screen.update(self.cx, |s, _| {
                s.player.shared.append(&vec![3_000i16; seconds * 24_000]);
                s.player.shared.finish_loading();
            });
        }

        /// The device asks for `frames` frames at 24 kHz.
        fn device(&mut self, frames: usize) {
            self.screen.update(self.cx, |s, _| {
                s.player.shared.fill(&mut vec![0.0; frames], 1, 24_000)
            });
        }

        fn tick(&mut self) -> bool {
            self.screen.update(self.cx, |s, cx| s.tick(cx))
        }

        fn flash(&mut self) -> Option<String> {
            self.screen.read_with(self.cx, |s, _| match &s.flash {
                Some(super::Flash::Done(m)) | Some(super::Flash::Failed(m)) => Some(m.clone()),
                None => None,
            })
        }
    }

    #[gpui::test]
    fn an_old_call_plays_and_the_speaking_role_follows_the_audio(cx: &mut gpui::TestAppContext) {
        let mut ui = ui(cx, call("{}"));
        ui.fill_tape(10);
        ui.screen.update(ui.cx, |s, cx| s.toggle_play(cx));
        assert_eq!(*ui.log.lock().unwrap(), ["play"]);
        // The first play asks the core for the samples.
        ui.cx.run_until_parked();
        let req = sent(&mut ui.other).expect("a request");
        assert_eq!(req["method"], "call.audio");
        assert_eq!(
            req["params"],
            json!({"product": "demo", "minutes_id": "m1", "from": 0, "count": 1_440_000})
        );
        assert!(ui.screen.read_with(ui.cx, |s, _| s.playing()));

        ui.device(24_000);
        assert!(ui.tick());
        let (pos, who) = ui.screen.read_with(ui.cx, |s, _| {
            let c = s.call().unwrap();
            (
                s.position,
                speaking(&c.segments, s.position, true).map(String::from),
            )
        });
        assert!((pos - 1.0).abs() < 1e-6, "{pos}");
        assert_eq!(who.as_deref(), Some("pm"));
        ui.draw();

        ui.device(24_000 * 3);
        ui.tick();
        let who = ui.screen.read_with(ui.cx, |s, _| {
            speaking(&s.call().unwrap().segments, s.position, true).map(String::from)
        });
        assert_eq!(
            who.as_deref(),
            Some("dev"),
            "4 s is inside the second segment"
        );
        ui.draw();

        ui.screen.update(ui.cx, |s, cx| s.toggle_play(cx));
        assert_eq!(*ui.log.lock().unwrap(), ["play", "pause"]);
        assert!(!ui.screen.read_with(ui.cx, |s, _| s.playing()));
    }

    #[gpui::test]
    fn a_call_that_plays_to_its_end_gets_the_listened_mark(cx: &mut gpui::TestAppContext) {
        let mut ui = ui(cx, call("{}"));
        ui.load_tape(2);
        ui.screen.update(ui.cx, |s, cx| s.toggle_play(cx));
        sent(&mut ui.other);
        ui.device(24_000 * 3);
        assert!(!ui.tick(), "the ticker stops at the end");
        let req = sent(&mut ui.other).expect("a request");
        assert_eq!(req["method"], "call.heard");
        assert_eq!(
            req["params"],
            json!({"product": "demo", "minutes_id": "m1", "heard": true})
        );
        // The call is back at its start, ready to play again.
        assert_eq!(ui.screen.read_with(ui.cx, |s, _| s.position), 0.0);
        assert_eq!(*ui.log.lock().unwrap(), ["play", "pause"]);

        // A call that was heard before sends no second mark.
        let mut heard = ui_heard(ui.cx);
        sent(&mut heard.1);
        heard.0.tick();
        assert!(sent(&mut heard.1).is_none());
    }

    /// A heard call that plays to its end. Returns the screen and its fake core.
    fn ui_heard(
        cx: &mut gpui::VisualTestContext,
    ) -> (HeardCall<'_>, std::os::unix::net::UnixStream) {
        use crate::link::Link;
        use crate::model::CoreState;
        use gpui_kit::AppContext as _;
        let (end, other) = std::os::unix::net::UnixStream::pair().unwrap();
        let core = cx.new(|_| CoreState::new(Arc::new(Link::for_tests(end))));
        let screen =
            cx.update(|_, cx| cx.new(|cx| CallScreen::new("demo".into(), "m1".into(), core, cx)));
        screen.update(cx, |s, _| {
            s.data = Load::Ready(Box::new(call(r#"{"heard": true}"#)));
            s.player = Player::new(Box::new(|_| Ok(Box::new(Silent(Default::default())))));
            s.player.shared.append(&vec![1_000i16; 4_800]);
            s.player.shared.finish_loading();
            s.player.play().unwrap();
            s.player.shared.fill(&mut vec![0.0; 9_600], 1, 24_000);
        });
        (HeardCall { screen, cx }, other)
    }

    struct HeardCall<'a> {
        screen: gpui::Entity<CallScreen>,
        cx: &'a mut gpui::VisualTestContext,
    }

    impl HeardCall<'_> {
        fn tick(&mut self) {
            self.screen.update(self.cx, |s, cx| {
                s.tick(cx);
            });
        }
    }

    #[gpui::test]
    fn keys_play_step_and_mark(cx: &mut gpui::TestAppContext) {
        let mut ui = ui(cx, call("{}"));
        ui.load_tape(10);
        ui.cx.update(|window, cx| window.focus_next(cx));
        ui.cx.simulate_keystrokes("space");
        assert_eq!(*ui.log.lock().unwrap(), ["play"]);
        sent(&mut ui.other);
        // Right goes to the next segment while it plays, Left back to the start of this one.
        ui.cx.simulate_keystrokes("right");
        assert_eq!(ui.screen.read_with(ui.cx, |s, _| s.position), 3.35);
        ui.cx.simulate_keystrokes("right");
        assert_eq!(ui.screen.read_with(ui.cx, |s, _| s.position), 6.35);
        ui.cx.simulate_keystrokes("right");
        assert_eq!(
            ui.screen.read_with(ui.cx, |s, _| s.position),
            6.35,
            "no segment after the last"
        );
        ui.device(24_000 * 2);
        ui.tick();
        ui.cx.simulate_keystrokes("left");
        assert_eq!(
            ui.screen.read_with(ui.cx, |s, _| s.position),
            6.35,
            "inside a segment, left restarts it"
        );
        ui.cx.simulate_keystrokes("space");
        assert_eq!(*ui.log.lock().unwrap(), ["play", "pause"]);

        ui.cx.simulate_keystrokes("l");
        let req = sent(&mut ui.other).expect("a request");
        assert_eq!(
            (req["method"].clone(), req["params"]["heard"].clone()),
            (json!("call.heard"), json!(true))
        );
        // A second press waits for the reply of the first.
        ui.cx.simulate_keystrokes("l");
        assert!(sent(&mut ui.other).is_none());
    }

    #[gpui::test]
    fn a_call_with_no_audio_cannot_play_and_offers_the_render(cx: &mut gpui::TestAppContext) {
        let mut ui = ui(cx, call(r#"{"audio": null, "timed": false}"#));
        ui.screen.update(ui.cx, |s, cx| s.toggle_play(cx));
        assert!(ui.flash().unwrap().contains("no audio"));
        assert!(ui.log.lock().unwrap().is_empty(), "the device never opened");
        ui.draw();
        ui.screen.update(ui.cx, |s, cx| s.render_call(cx));
        let req = sent(&mut ui.other).expect("a request");
        assert_eq!(req["method"], "call.render");
        let name = req["params"]["render"].as_str().unwrap().to_string();
        assert!(name.starts_with("app-"), "{name}");
        assert_eq!(req["params"]["minutes_id"], "m1");
        // A second press waits for the render.
        ui.screen.update(ui.cx, |s, cx| s.render_call(cx));
        assert!(sent(&mut ui.other).is_none());
        // Cancel names the render of this screen.
        ui.screen.update(ui.cx, |s, cx| s.cancel_render(cx));
        let req = sent(&mut ui.other).expect("a request");
        assert_eq!(req["method"], "call.cancel");
        assert_eq!(req["params"]["render"], json!(name));
        ui.draw();
    }

    #[gpui::test]
    fn a_render_that_runs_in_the_core_shows_its_progress_and_can_be_canceled(
        cx: &mut gpui::TestAppContext,
    ) {
        let running = call(r#"{"audio": null, "render": {"render": "cli-7", "done": 2, "of": 5}}"#);
        let mut ui = ui(cx, running);
        ui.draw();
        ui.screen.update(ui.cx, |s, cx| s.render_call(cx));
        assert!(sent(&mut ui.other).is_none(), "a render runs already");
        ui.screen.update(ui.cx, |s, cx| s.cancel_render(cx));
        let req = sent(&mut ui.other).expect("a request");
        assert_eq!(req["params"]["render"], "cli-7");
    }

    #[gpui::test]
    fn a_call_with_no_timings_does_not_play(cx: &mut gpui::TestAppContext) {
        let mut ui = ui(cx, call(r#"{"timed": false}"#));
        ui.screen.update(ui.cx, |s, cx| s.toggle_play(cx));
        assert!(ui.flash().unwrap().contains("no timings"));
        assert!(ui.log.lock().unwrap().is_empty());
        ui.draw();
    }

    #[gpui::test]
    fn a_failing_device_shows_its_error_and_the_call_stays_paused(cx: &mut gpui::TestAppContext) {
        let mut ui = ui(cx, call("{}"));
        ui.screen.update(ui.cx, |s, cx| {
            s.player = Player::new(Box::new(|_| Err("No audio output device".into())));
            s.toggle_play(cx);
        });
        assert_eq!(ui.flash().as_deref(), Some("No audio output device"));
        assert!(!ui.screen.read_with(ui.cx, |s, _| s.playing()));
        ui.draw();
    }

    #[gpui::test]
    fn a_file_that_changed_drops_the_samples(cx: &mut gpui::TestAppContext) {
        let mut ui = ui(cx, call("{}"));
        ui.load_tape(2);
        assert_eq!(
            ui.screen.read_with(ui.cx, |s, _| s.player.shared.len()),
            48_000
        );
        // A new render replaced the file: the screen reads the call and finds another size.
        ui.screen.update(ui.cx, |s, cx| {
            let fresh = call(r#"{"audio": {"format": "opus", "bytes": 120000}}"#);
            s.data.settle(Ok(Box::new(fresh)));
            let file = s
                .call()
                .and_then(|c| c.audio.as_ref())
                .map(|a| (a.format.clone(), a.bytes));
            if file != s.tape_of {
                s.player.shared.clear();
                s.tape_of = None;
            }
            cx.notify();
        });
        assert_eq!(ui.screen.read_with(ui.cx, |s, _| s.player.shared.len()), 0);
    }
}
