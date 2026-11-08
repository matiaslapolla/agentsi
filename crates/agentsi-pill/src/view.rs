//! The pill window on Linux: a layer-shell surface on the overlay layer.
//!
//! The window exists only while the pill shows something. Hidden is no surface
//! and no frame. A phase that animates draws on a 30 Hz timer, the others draw once.

use crate::link::{self, LinkEvent, LinkSender};
use crate::state::{Effect, Input, Mark, Model, Phase};
use agentsi_ui::tokens::{self, font, radius, size, space, status, surface, text};
use futures::StreamExt;
use futures::channel::mpsc::UnboundedReceiver;
use gpui::layer_shell::{Anchor, KeyboardInteractivity, Layer, LayerShellOptions};
use gpui::{
    App, Bounds, Context, Entity, MouseButton, Pixels, Render, Size, Window,
    WindowBackgroundAppearance, WindowBounds, WindowHandle, WindowKind, WindowOptions, div, point,
    prelude::*,
};
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant};

const FRAME: Duration = Duration::from_millis(33);
const BARS: usize = 9;
/// The pill hangs this many steps of `space` under the top edge of the output.
const TOP_MARGIN_STEPS: u16 = 3;

/// Counts the frames that the view draws. `AGENTSI_PILL_STATS` prints it.
pub static FRAMES: AtomicU64 = AtomicU64::new(0);

/// Called for each effect, on the window thread. The speech code (U12) plugs in here.
pub type EffectHook = Arc<dyn Fn(&Effect) + Send + Sync>;

pub struct Options {
    pub start: Model,
    pub link: LinkSender,
    pub events: UnboundedReceiver<LinkEvent>,
    pub hook: EffectHook,
    /// Stop the program after this time. `None` runs until the process ends.
    pub lifetime: Option<Duration>,
}

fn wide(phase: Phase) -> bool {
    phase == Phase::Error
}

fn window_size(phase: Phase) -> Size<Pixels> {
    Size::new(if wide(phase) { space(80) } else { space(42) }, space(10))
}

pub fn run(options: Options) {
    gpui_platform::application().run(move |cx: &mut App| {
        agentsi_ui::init(cx);
        cx.set_quit_mode(gpui::QuitMode::Explicit);
        let Options {
            start,
            link,
            mut events,
            hook,
            lifetime,
        } = options;
        let app = cx.new(|_| PillApp {
            model: start,
            window: None,
            link,
            hook,
            armed: 0,
            ticking: false,
            since: Instant::now(),
        });
        app.update(cx, |app, cx| app.sync(cx));
        // This task owns the app. With no window, nothing else does.
        cx.spawn(async move |cx| {
            while let Some(event) = events.next().await {
                app.update(cx, |app, cx| match event {
                    LinkEvent::Input(input) => app.input(input, cx),
                    LinkEvent::DictationTarget(_) => {}
                });
            }
        })
        .detach();
        if let Some(lifetime) = lifetime {
            cx.spawn(async move |cx| {
                cx.background_executor().timer(lifetime).await;
                cx.update(|cx| cx.quit());
            })
            .detach();
        }
    });
}

/// The window that shows now, and the width class it opened with.
struct Open {
    handle: WindowHandle<PillView>,
    wide: bool,
}

struct PillApp {
    model: Model,
    window: Option<Open>,
    link: LinkSender,
    hook: EffectHook,
    /// The generation whose timeout is armed.
    armed: u64,
    ticking: bool,
    /// When the phase started, for the animation clock.
    since: Instant,
}

impl PillApp {
    fn snap(&self) -> Snap {
        Snap {
            phase: self.model.phase(),
            mark: self.model.mark(),
            error: self.model.error().map(str::to_owned),
            since: self.since,
        }
    }

    /// Pushes the model to the window. The view never reads the app: the window
    /// renders inside the update that opens it.
    fn redraw(&self, cx: &mut Context<Self>) {
        if let Some(open) = &self.window {
            let snap = self.snap();
            let _ = open.handle.update(cx, |view, _, cx| {
                view.snap = snap;
                cx.notify();
            });
        }
    }

    fn input(&mut self, input: Input, cx: &mut Context<Self>) {
        let before = self.model.generation();
        let effects = self.model.apply(input);
        for effect in &effects {
            match effect {
                Effect::StopRecording => link::stop_recording(self.link.clone()),
                Effect::EndConversation => link::stop_conversation(self.link.clone()),
                _ => {}
            }
            (self.hook)(effect);
        }
        if self.model.generation() != before {
            self.since = Instant::now();
        }
        self.sync(cx);
    }

    /// Makes the window, the limit timer and the frame timer match the model.
    fn sync(&mut self, cx: &mut Context<Self>) {
        let phase = self.model.phase();
        let width_class = wide(phase);
        if let Some(open) = &self.window
            && (phase == Phase::Hidden || open.wide != width_class)
        {
            let _ = open
                .handle
                .update(cx, |_, window, _| window.remove_window());
            self.window = None;
        }
        if phase != Phase::Hidden && self.window.is_none() {
            let app = cx.entity();
            let snap = self.snap();
            let opened = Instant::now();
            let bounds = Bounds {
                origin: point(Pixels::ZERO, Pixels::ZERO),
                size: window_size(phase),
            };
            match cx.open_window(
                WindowOptions {
                    titlebar: None,
                    window_bounds: Some(WindowBounds::Windowed(bounds)),
                    app_id: Some("agentsi-pill".to_string()),
                    window_background: WindowBackgroundAppearance::Transparent,
                    kind: WindowKind::LayerShell(LayerShellOptions {
                        namespace: "agentsi-pill".to_string(),
                        layer: Layer::Overlay,
                        anchor: Anchor::TOP,
                        margin: Some((
                            space(TOP_MARGIN_STEPS),
                            Pixels::ZERO,
                            Pixels::ZERO,
                            Pixels::ZERO,
                        )),
                        keyboard_interactivity: KeyboardInteractivity::None,
                        ..Default::default()
                    }),
                    focus: false,
                    ..Default::default()
                },
                move |_, cx| cx.new(|_| PillView { app, snap }),
            ) {
                Ok(handle) => {
                    self.window = Some(Open {
                        handle,
                        wide: width_class,
                    });
                    if crate::stats_enabled() {
                        eprintln!("agentsi-pill: window opened in {:?}", opened.elapsed());
                    }
                }
                Err(e) => eprintln!("agentsi-pill: cannot open the window: {e}"),
            }
        }
        self.arm_limit(cx);
        self.arm_frames(cx);
        self.redraw(cx);
    }

    fn arm_limit(&mut self, cx: &mut Context<Self>) {
        let generation = self.model.generation();
        if self.armed == generation {
            return;
        }
        self.armed = generation;
        if let Some(limit) = self.model.limit() {
            cx.spawn(async move |this, cx| {
                cx.background_executor().timer(limit).await;
                let _ = this.update(cx, |app, cx| app.input(Input::Timeout { generation }, cx));
            })
            .detach();
        }
    }

    fn arm_frames(&mut self, cx: &mut Context<Self>) {
        if self.ticking || !self.model.phase().animated() {
            return;
        }
        self.ticking = true;
        cx.spawn(async move |this, cx| {
            loop {
                cx.background_executor().timer(FRAME).await;
                let keep = this
                    .update(cx, |app, cx| {
                        if !app.model.phase().animated() {
                            app.ticking = false;
                            return false;
                        }
                        app.redraw(cx);
                        true
                    })
                    .unwrap_or(false);
                if !keep {
                    break;
                }
            }
        })
        .detach();
    }
}

/// What the view draws.
struct Snap {
    phase: Phase,
    /// Toggle dictation or conversation mode: the microphone is open with no key held.
    mark: Option<Mark>,
    error: Option<String>,
    since: Instant,
}

struct PillView {
    app: Entity<PillApp>,
    snap: Snap,
}

impl Render for PillView {
    fn render(&mut self, _window: &mut Window, _cx: &mut Context<Self>) -> impl IntoElement {
        FRAMES.fetch_add(1, Ordering::Relaxed);
        let phase = self.snap.phase;
        let t = self.snap.since.elapsed().as_secs_f32();
        let content = match phase {
            Phase::Error => {
                error_line(self.snap.error.as_deref().unwrap_or("Error")).into_any_element()
            }
            Phase::Listening => {
                listening(t, self.snap.mark == Some(Mark::Toggle)).into_any_element()
            }
            Phase::Transcribing => dots(t).into_any_element(),
            Phase::Waiting => waiting().into_any_element(),
            Phase::Preparing | Phase::Speaking | Phase::Hidden => bars(phase, t).into_any_element(),
        };
        let conversing = self.snap.mark == Some(Mark::Conversation);
        let app = self.app.clone();
        div()
            .size_full()
            .flex()
            .items_center()
            .justify_center()
            .gap(space(1))
            .rounded(radius::FULL)
            .bg(surface::OVERLAY.alpha(0.92))
            .border_1()
            .border_color(tokens::line::STRONG.hsla())
            .cursor_pointer()
            .when(conversing, |el| el.child(conversation_mark()))
            .child(content)
            .on_mouse_down(MouseButton::Left, move |_, _, cx| {
                // The click handler runs inside an update of this window. Closing the window
                // there fails, so the input runs after the update ends.
                let app = app.clone();
                cx.defer(move |cx| app.update(cx, |app, cx| app.input(Input::Click, cx)));
            })
    }
}

fn bar(height: Pixels, color: tokens::Color, alpha: f32) -> impl IntoElement {
    div()
        .w(space(1))
        .h(height)
        .rounded(radius::FULL)
        .bg(color.alpha(alpha))
}

/// Preparing: a slow wave. Speaking: loud and quiet syllables, 4 each second.
fn bars(phase: Phase, t: f32) -> impl IntoElement {
    let speaking = phase == Phase::Speaking;
    div()
        .flex()
        .items_center()
        .gap(space(1))
        .children((0..BARS).map(move |i| {
            let i = i as f32;
            let level = if speaking {
                let syllable = ((t * 4.0).sin() * 0.5 + 0.5) * (0.6 + 0.4 * (i * 1.7).sin().abs());
                1.0 + 6.0 * syllable
            } else {
                1.0 + 2.5 * (0.5 + 0.5 * (t * 5.5 + i * 0.8).sin())
            };
            bar(
                space(1) * level.round(),
                tokens::accent::BASE,
                if speaking { 1.0 } else { 0.6 },
            )
        }))
}

/// Listening: a red dot that pulses, and bars that follow the voice. Toggle dictation puts a
/// ring around the dot: the microphone stays open with no key held.
/// The level is a placeholder until the speech-in code reports the microphone level.
fn listening(t: f32, toggle: bool) -> impl IntoElement {
    let pulse = 0.55 + 0.45 * (t * 4.0).sin().abs();
    let dot = div()
        .size(space(3))
        .rounded(radius::FULL)
        .bg(status::DANGER.alpha(pulse));
    div()
        .flex()
        .items_center()
        .gap(space(3))
        .child(if toggle {
            div()
                .size(space(5))
                .flex()
                .items_center()
                .justify_center()
                .rounded(radius::FULL)
                .border_1()
                .border_color(status::DANGER.alpha(0.8))
                .child(dot)
                .into_any_element()
        } else {
            dot.into_any_element()
        })
        .child(
            div()
                .flex()
                .items_center()
                .gap(space(1))
                .children((0..7).map(move |i| {
                    let i = i as f32;
                    let level = 1.0
                        + 4.0
                            * (0.5 + 0.5 * (t * 6.0 + i * 1.3).sin())
                            * (0.6 + 0.4 * (t * 2.3 + i).sin().abs());
                    bar(space(1) * level.round(), text::PRIMARY, 0.85)
                })),
        )
}

/// Transcribing: three dots in a row, one lit at a time.
fn dots(t: f32) -> impl IntoElement {
    let lit = ((t * 3.0) as usize) % 3;
    div()
        .flex()
        .items_center()
        .gap(space(2))
        .children((0..3).map(move |i| {
            div()
                .size(space(2))
                .rounded(radius::FULL)
                .bg(tokens::accent::BASE.alpha(if i == lit { 1.0 } else { 0.35 }))
        }))
}

/// Waiting: three dots, all lit at half strength. It does not move, so the pill draws no frame
/// while the pane works.
fn waiting() -> impl IntoElement {
    div()
        .flex()
        .items_center()
        .gap(space(2))
        .children((0..3).map(|_| {
            div()
                .size(space(2))
                .rounded(radius::FULL)
                .bg(tokens::accent::BASE.alpha(0.5))
        }))
}

/// Conversation mode: a small speech bubble before the phase. The microphone is open or ready.
fn conversation_mark() -> impl IntoElement {
    div()
        .w(space(5))
        .h(space(4))
        .flex_none()
        .flex()
        .items_center()
        .justify_center()
        .gap(space(1))
        .rounded(radius::sm())
        .border_1()
        .border_color(tokens::accent::BASE.alpha(0.9))
        .children((0..2).map(|_| {
            div()
                .size(space(1))
                .rounded(radius::FULL)
                .bg(tokens::accent::BASE.alpha(0.9))
        }))
}

fn error_line(message: &str) -> impl IntoElement {
    div()
        .flex()
        .items_center()
        .gap(space(2))
        .px(space(4))
        .child(
            div()
                .size(space(2))
                .flex_none()
                .rounded(radius::FULL)
                .bg(status::DANGER.hsla()),
        )
        .child(
            div()
                .text_size(size::sm())
                .font_family(font::UI)
                .text_color(text::PRIMARY.hsla())
                .overflow_hidden()
                .text_ellipsis()
                .whitespace_nowrap()
                .child(message.to_owned()),
        )
}
