//! The terminal panel: one pane of the dock, attached to one terminal of the core.
//!
//! The panel attaches with `terminal.attach`, feeds the stream into a
//! `TerminalPane` and returns a CREDIT for each DATA frame. The pane's input,
//! replies, size and focus go back as frames on the same channel. The header shows
//! the program state, the attention marker and the voice switch.
//!
//! The panel holds the core of its terminal's host. A stream of an SSH host ends when the
//! link breaks. The panel then keeps its last screen and says so in the header. When the
//! host answers again, the panel attaches again: the terminal may have outlived the link.

use std::cell::RefCell;
use std::rc::Rc;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::time::Instant;

use agentsi_proto::ErrorKind;
use agentsi_proto::events::ProgramState;
use agentsi_proto::frame::{Frame, Kind};
use agentsi_proto::terminal_api::{CloseReason, Kill, TerminalId};
use agentsi_proto::voice_api::{
    Converse, ConverseAction, ConverseParams, FocusReport, FocusReportParams, Mode, ModeParams,
};
use agentsi_term_view::{PaneEvent, PaneOptions, PaneOutput, TerminalPane};
use agentsi_ui::bench::Bench;
use agentsi_ui::components::{StateMark, StatusDot, Switch, state_icon, status_tag};
use agentsi_ui::style::StyleExt as _;
use agentsi_ui::tokens::WorkState;
use agentsi_ui::tokens::label::Hue;
use agentsi_ui::tokens::{Status, control, line, radius, size, space, surface, text, weight};
use futures::StreamExt;
use gpui_kit::component::Icon;
use gpui_kit::component::dock::{BasePanel, Panel, PanelEvent, PanelState};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;

use crate::layout::{TERMINAL_PANEL, terminal_info};
use crate::link::{CallError, Link, StreamItem};
use crate::look::LookGlobal;
use crate::model::{CoreEvent, CoreState, DEVICE_OFF, HostState, Term};

fn resize_of(item: &StreamItem) -> Option<(u16, u16)> {
    match item {
        StreamItem::Resize(cols, rows) => Some((*cols, *rows)),
        _ => None,
    }
}

/// With this variable set, each panel writes the time of each feed and of the frame after it
/// to stderr, from the call of `terminal.attach`. It measures the cost of the first paint.
const TRACE_ATTACH: &str = "AGENTSI_TRACE_ATTACH";

fn trace_feed(id: &str, started: Instant, bytes: usize, cx: &mut AsyncWindowContext) {
    let fed = started.elapsed();
    let id = id.to_string();
    let _ = cx.update(|window, _| {
        window.on_next_frame(move |_, _| {
            eprintln!(
                "attach {id}: fed {bytes} bytes at {:.1} ms, next frame at {:.1} ms",
                fed.as_secs_f64() * 1000.,
                started.elapsed().as_secs_f64() * 1000.
            );
        });
    });
}

/// True while the app quits: then a panel that leaves the dock keeps its terminal.
pub static QUITTING: AtomicBool = AtomicBool::new(false);

/// The order of use of the panels, across all hosts.
static LAST_USE: AtomicU64 = AtomicU64::new(0);

/// The work state of a program state.
pub fn work_state_of(t: Option<&Term>) -> WorkState {
    match t {
        None => WorkState::Off,
        Some(t) if !t.running => WorkState::Off,
        Some(t) if t.attention().is_some() => WorkState::Blocked,
        Some(t) => match t.state {
            Some(ProgramState::Working) => WorkState::Working,
            Some(ProgramState::Done) => WorkState::Done,
            Some(ProgramState::Blocked) => WorkState::Blocked,
            Some(ProgramState::Error) => WorkState::Failed,
            Some(ProgramState::Idle) | None => WorkState::Off,
        },
    }
}

/// The status color of a program state.
pub fn status_of(t: Option<&Term>) -> Status {
    work_state_of(t).status()
}

/// The state that makes a pane glow: only a program that asks for the owner.
pub fn glow_of(t: Option<&Term>) -> Option<WorkState> {
    t.and_then(Term::attention).map(|_| WorkState::Blocked)
}

/// What the tab and the header of a pane show: the label of the owner, else the program title.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Shown {
    pub title: String,
    /// The program title that a label replaced.
    pub tooltip: Option<String>,
    /// The color of the pane, else the color of its product.
    pub hue: Option<Hue>,
}

/// Names terminal `id` of `core` for the tab, the header and the tree.
pub fn shown(core: &CoreState, id: &str, cx: &App) -> Shown {
    let term = core.terminals.get(id);
    let program = term
        .map(|t| t.title.clone())
        .filter(|t| !t.is_empty())
        .unwrap_or_else(|| id.to_string());
    let host = core.remote.as_deref();
    let product = term.and_then(|t| t.product.clone());
    let (label, pane_hue, product_hue) = LookGlobal::with(cx, |look| {
        let pane = look.pane(host, id);
        let product = product.map(|p| look.product(host, &p).hue);
        (pane.label, pane.hue, product.flatten())
    })
    .unwrap_or((None, None, None));
    match label {
        Some(label) => Shown {
            title: label,
            tooltip: Some(program),
            hue: pane_hue.or(product_hue),
        },
        None => Shown {
            title: program,
            tooltip: None,
            hue: pane_hue.or(product_hue),
        },
    }
}

/// The 3 px bar of a color at the start of a tab title.
pub fn tab_bar(hue: Hue, selector: String) -> impl IntoElement {
    div()
        .debug_selector(|| selector)
        .flex_none()
        .w(control::COLOR_BAR)
        .h(control::icon_sm())
        .rounded(radius::FULL)
        .bg(hue.color())
}

/// The attention bench: the time when the bench asked for a `blocked` state.
/// The pane marker and the product marker record their paint. The sample is the time
/// to the later of the two, or to the pane marker for a terminal with no product.
#[derive(Clone, Default)]
pub struct AttentionProbe(Rc<RefCell<Probe>>);

#[derive(Default)]
struct Probe {
    armed: Option<(Instant, Bench)>,
    with_product: bool,
    pane: bool,
    product: bool,
}

/// A marker that the attention bench times.
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Marker {
    Pane,
    Product,
}

impl AttentionProbe {
    pub fn arm(&self, at: Instant, bench: Bench, with_product: bool) {
        *self.0.borrow_mut() = Probe {
            armed: Some((at, bench)),
            with_product,
            pane: false,
            product: false,
        };
    }

    pub fn painted(&self, marker: Marker) {
        let mut p = self.0.borrow_mut();
        if p.armed.is_none() {
            return;
        }
        match marker {
            Marker::Pane => p.pane = true,
            Marker::Product => p.product = true,
        }
        if p.pane
            && (p.product || !p.with_product)
            && let Some((at, bench)) = p.armed.take()
        {
            bench.record_latency(at.elapsed().as_secs_f64() * 1000.);
        }
    }

    /// An element that tells the probe that `marker` painted.
    pub fn element(&self, marker: Marker) -> impl IntoElement + use<> {
        let probe = self.clone();
        canvas(|_, _, _| (), move |_, _, _, _| probe.painted(marker)).size_0()
    }
}

enum Stream {
    /// `terminal.attach` has not answered yet.
    Attaching,
    /// A channel belongs to one link: the panel never sends it on the next link of its host.
    Open(u32, Arc<Link>),
    /// The stream ended. The header shows the reason.
    Closed,
    /// The link of an SSH host broke. The panel attaches again when the host answers.
    Lost,
}

pub struct TerminalPanel {
    pub id: String,
    core: Entity<CoreState>,
    pane: Entity<TerminalPane>,
    stream: Stream,
    /// The last size of the pane, sent again after the attach.
    size: Option<(u16, u16)>,
    focused: bool,
    /// When this panel last took the focus, or 0. The palette takes the focus while it is
    /// open, and its actions act on the panel with the highest number.
    pub last_use: u64,
    /// True while the dock displays this tab. A hidden pane takes no keys.
    pub shown: bool,
    header: Entity<PaneHeader>,
    _subscriptions: Vec<Subscription>,
    _stream: Option<Task<()>>,
}

impl TerminalPanel {
    pub fn new(
        id: String,
        core: Entity<CoreState>,
        bench: Option<Bench>,
        probe: Option<AttentionProbe>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let pane = cx.new(|cx| {
            TerminalPane::new(
                PaneOptions {
                    bench,
                    ..Default::default()
                },
                window,
                cx,
            )
        });
        let subscriptions = vec![
            cx.subscribe_in(&pane, window, |this, _, event, window, cx| {
                this.on_pane(event, window, cx)
            }),
            cx.subscribe_in(&core, window, |this, _, event, window, cx| {
                if *event == CoreEvent::Up && matches!(this.stream, Stream::Lost) {
                    this.attach(window, cx);
                    this.read_voice(cx);
                }
            }),
        ];
        let header = cx.new(|cx| PaneHeader::new(id.clone(), core.clone(), probe, cx));
        let mut panel = TerminalPanel {
            id,
            core,
            pane,
            stream: Stream::Attaching,
            size: None,
            focused: false,
            last_use: 0,
            shown: false,
            header,
            _subscriptions: subscriptions,
            _stream: None,
        };
        panel.attach(window, cx);
        panel.read_voice(cx);
        panel
    }

    /// This panel shows terminal `id` of `host`.
    pub fn is(&self, host: Option<&str>, id: &str, cx: &App) -> bool {
        self.id == id && self.core.read(cx).remote.as_deref() == host
    }

    /// The SSH host of this terminal, or `None` for this device.
    pub fn host(&self, cx: &App) -> Option<String> {
        self.core.read(cx).remote.clone()
    }

    /// A program runs in the terminal: closing the tab stops it.
    pub fn running(&self, cx: &App) -> bool {
        self.core
            .read(cx)
            .terminals
            .get(&self.id)
            .is_some_and(|t| t.running)
    }

    fn attach(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let link = self.core.read(cx).link.clone();
        self.stream = Stream::Attaching;
        self.header.update(cx, |h, cx| {
            h.closed = None;
            h.lost = false;
            cx.notify();
        });
        let started = Instant::now();
        let opened = link.attach(&self.id);
        let pane = self.pane.clone();
        let trace = std::env::var_os(TRACE_ATTACH)
            .is_some()
            .then(|| self.id.clone());
        self._stream = Some(cx.spawn_in(window, async move |this, cx| {
            let (channel, mut stream) = match opened.await {
                Ok(s) => s,
                Err(e) => {
                    let _ =
                        this.update_in(cx, |p, window, cx| p.attach_failed(e, &link, window, cx));
                    return;
                }
            };
            let open = link.clone();
            let _ = this.update(cx, |p, cx| {
                p.stream = Stream::Open(channel, open);
                if let Some((cols, rows)) = p.size {
                    link.send(&Frame::resize(channel, cols, rows));
                }
                if p.focused {
                    link.send(&Frame::focus(channel, true));
                }
                cx.notify();
            });
            // The core sends the size of its screen before the bytes at that size, for
            // example before a first paint. The pane takes it with the next feed.
            let mut source = None;
            while let Some(item) = stream.next().await {
                match item {
                    StreamItem::Data(at, mut bytes) => {
                        // Join the frames that already wait into one feed, as the pane wants.
                        let mut pending = None;
                        while let Ok(next) = stream.try_recv() {
                            match next {
                                StreamItem::Data(_, more) => bytes.extend(more),
                                other => {
                                    pending = Some(other);
                                    break;
                                }
                            }
                        }
                        let n = bytes.len();
                        pane.update(cx, |p, cx| {
                            if let Some((cols, rows)) = source.take() {
                                p.source_size(cols, rows);
                            }
                            p.feed(&bytes, at, cx)
                        });
                        link.send(&Frame::credit(channel, n as u32));
                        if let Some(id) = &trace {
                            trace_feed(id, started, n, cx);
                        }
                        source = pending.as_ref().and_then(resize_of).or(source);
                        if let Some(item) = pending
                            && !this
                                .update(cx, |p, cx| p.on_stream(item, cx))
                                .unwrap_or(false)
                        {
                            return;
                        }
                    }
                    other => {
                        source = resize_of(&other).or(source);
                        if !this
                            .update(cx, |p, cx| p.on_stream(other, cx))
                            .unwrap_or(false)
                        {
                            return;
                        }
                    }
                }
            }
        }));
    }

    /// A RESIZE or a CLOSE of the stream. Returns false when the stream ended.
    /// The stream loop keeps the size of a RESIZE for the next feed. The pane sends its own size
    /// after the attach, and the core then sends the bytes at that size.
    fn on_stream(&mut self, item: StreamItem, cx: &mut Context<Self>) -> bool {
        match item {
            StreamItem::Data(..) | StreamItem::Resize(..) => true,
            StreamItem::Close(reason) => {
                let why: String = match reason {
                    Some(CloseReason::Exit { code: Some(c) }) => format!("exited with code {c}"),
                    Some(CloseReason::Exit { code: None }) => "stopped by a signal".into(),
                    None if self.core.read(cx).remote.is_some() => {
                        self.set_lost(cx);
                        return false;
                    }
                    None => "the core is gone".into(),
                };
                self.set_closed(why, cx);
                false
            }
        }
    }

    /// The attach on `link` failed. On a host, a broken link waits for the host, unless the
    /// host came back while this attach ran: then no other event follows. A terminal that
    /// the core no longer knows has ended.
    fn attach_failed(
        &mut self,
        e: CallError,
        link: &Arc<Link>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        match e {
            CallError::Link(_) if self.core.read(cx).remote.is_some() => {
                let core = self.core.read(cx);
                let newer = core.host == HostState::Up && !Arc::ptr_eq(&core.link, link);
                if newer {
                    self.attach(window, cx);
                } else {
                    self.set_lost(cx);
                }
            }
            CallError::Rpc(e)
                if e.kind() == ErrorKind::NotFound && self.core.read(cx).remote.is_some() =>
            {
                self.set_closed("the terminal ended".into(), cx)
            }
            e => self.set_closed(format!("no attach: {e}"), cx),
        }
    }

    /// The link of the host broke. The header follows the state of the host.
    fn set_lost(&mut self, cx: &mut Context<Self>) {
        self.stream = Stream::Lost;
        self.header.update(cx, |h, cx| {
            h.lost = true;
            cx.notify();
        });
    }

    fn set_closed(&mut self, why: String, cx: &mut Context<Self>) {
        self.stream = Stream::Closed;
        self.header.update(cx, |h, cx| {
            h.closed = Some(why);
            cx.notify();
        });
    }

    /// The open channel, with the link that it belongs to.
    fn channel(&self) -> Option<(u32, Arc<Link>)> {
        match &self.stream {
            Stream::Open(c, link) => Some((*c, link.clone())),
            _ => None,
        }
    }

    fn on_pane(&mut self, event: &PaneEvent, window: &mut Window, cx: &mut Context<Self>) {
        let channel = self.channel();
        match event {
            PaneEvent::Output(PaneOutput::Input(b)) => {
                if let Some((c, link)) = channel {
                    link.send(&Frame::new(c, Kind::DATA, b.clone()));
                }
            }
            PaneEvent::Output(PaneOutput::Auto(b)) => {
                if let Some((c, link)) = channel {
                    link.send(&Frame::new(c, Kind::AUTO, b.clone()));
                }
            }
            PaneEvent::Output(PaneOutput::Resize { cols, rows }) => {
                self.size = Some((*cols, *rows));
                if let Some((c, link)) = channel {
                    link.send(&Frame::resize(c, *cols, *rows));
                }
            }
            PaneEvent::Output(PaneOutput::Focus(f)) => {
                self.focused = *f;
                if let Some((c, link)) = channel {
                    link.send(&Frame::focus(c, *f));
                }
                self.report_focus(*f, window, cx);
            }
            PaneEvent::Title(_) | PaneEvent::Bell => {}
        }
    }

    /// `focus.report`: dictated text goes to the focused pane. A window that loses
    /// the focus keeps its target, so the owner can dictate from another window.
    fn report_focus(&mut self, focused: bool, window: &mut Window, cx: &mut Context<Self>) {
        let id = self.id.clone();
        let core = self.core.clone();
        if focused {
            self.last_use = LAST_USE.fetch_add(1, Ordering::Relaxed) + 1;
            core.update(cx, |s, _| s.focused = Some(id.clone()));
            let call = core
                .read(cx)
                .link
                .call::<FocusReport>(&FocusReportParams { terminal: Some(id) });
            cx.background_spawn(async move {
                let _ = call.await;
            })
            .detach();
            return;
        }
        if !window.is_window_active() {
            return;
        }
        // Another pane can take the focus in the same frame: report none only if none did.
        cx.spawn_in(window, async move |_, cx| {
            let _ = cx.update(|_, cx| {
                let still = core.read(cx).focused.as_deref() == Some(id.as_str());
                if still {
                    core.update(cx, |s, _| s.focused = None);
                    let call = core
                        .read(cx)
                        .link
                        .call::<FocusReport>(&FocusReportParams::default());
                    cx.background_spawn(async move {
                        let _ = call.await;
                    })
                    .detach();
                }
            });
        })
        .detach();
    }

    fn read_voice(&self, cx: &mut Context<Self>) {
        self.voice(None, cx);
    }

    pub fn toggle_voice(&self, cx: &mut Context<Self>) {
        let on = self
            .core
            .read(cx)
            .terminals
            .get(&self.id)
            .is_some_and(|t| t.voice);
        self.voice(Some(!on), cx);
    }

    /// Starts conversation mode with this pane, or stops it (`voice.converse` toggle). The
    /// core turns voice mode of the pane on with the mode and puts it back at the stop, so the
    /// switch of the header reads the state again when the call ends.
    pub fn talk(&self, cx: &mut Context<Self>) {
        let call = self.core.read(cx).link.call::<Converse>(&ConverseParams {
            action: ConverseAction::Toggle,
            terminal: Some(self.id.clone()),
        });
        let (core, id) = (self.core.clone(), self.id.clone());
        cx.spawn(async move |_, cx| {
            if let Err(e) = call.await {
                eprintln!("agentsi-app: conversation mode: {e}");
            }
            cx.update(|cx| voice_mode(&core, &id, None, cx));
        })
        .detach();
    }

    /// Calls `voice.mode`, and shows the state from its reply.
    pub fn voice(&self, on: Option<bool>, cx: &mut Context<Self>) {
        voice_mode(&self.core, &self.id, on, cx);
    }
}

/// `voice.mode {terminal, on}`. The state of the reply goes into `CoreState`.
pub fn voice_mode(core: &Entity<CoreState>, id: &str, on: Option<bool>, cx: &mut App) {
    let call = core.read(cx).link.call::<Mode>(&ModeParams {
        terminal: id.to_string(),
        on,
    });
    let core = core.clone();
    cx.spawn(async move |cx| {
        if let Ok(r) = call.await {
            core.update(cx, |s, cx| {
                if let Some(t) = s.terminals.get_mut(&r.terminal)
                    && t.voice != r.on
                {
                    t.voice = r.on;
                    cx.notify();
                }
            });
        }
    })
    .detach();
}

/// The header of a terminal panel, in its own cached view: a frame of the pane
/// does not draw it again.
pub struct PaneHeader {
    id: String,
    core: Entity<CoreState>,
    /// Why the stream ended, once it ended.
    closed: Option<String>,
    /// The link of the host broke. The state of the host gives the word, and the link can
    /// break before the host thread reports it.
    lost: bool,
    probe: Option<AttentionProbe>,
    _observe: Subscription,
}

impl PaneHeader {
    fn new(
        id: String,
        core: Entity<CoreState>,
        probe: Option<AttentionProbe>,
        cx: &mut Context<Self>,
    ) -> Self {
        PaneHeader {
            _observe: cx.observe(&core, |_, _, cx| cx.notify()),
            id,
            core,
            closed: None,
            lost: false,
            probe,
        }
    }
}

impl PaneHeader {
    /// The tag of a stream that ended.
    fn shown_state(&self, host: &HostState) -> Option<String> {
        match (host, self.lost) {
            (HostState::Connecting, true) => Some(HostState::Connecting.word().to_string()),
            (_, true) => Some(DEVICE_OFF.to_string()),
            (_, false) => self.closed.clone(),
        }
    }
}

impl Render for PaneHeader {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let core = self.core.read(cx);
        let term = core.terminals.get(&self.id);
        let shown = shown(core, &self.id, cx);
        let (title, hue) = (shown.title, shown.hue);
        let product = term
            .and_then(|t| t.product.clone())
            .and_then(|p| core.product(&p).map(|p| p.name.clone()).or(Some(p)));
        let place = match (product, &core.remote) {
            (Some(p), Some(host)) => Some(format!("{p} · {host}")),
            (None, Some(host)) => Some(host.clone()),
            (p, None) => p,
        };
        let attention = term.and_then(Term::attention);
        let voice = term.is_some_and(|t| t.voice);
        let closed = self.shown_state(&core.host);
        let probe = self.probe.clone();
        div()
            .size_full()
            .flex()
            .flex_none()
            .items_center()
            .gap(space(2))
            .h(control::height_md())
            .pad_x(space(2))
            .bg(surface::panel())
            .border_b_1()
            .border_color(line::border())
            .text_size(size::sm())
            .when_some(hue, |el, h| {
                el.child(tab_bar(h, format!("header-color:{}", self.id)))
            })
            .child(StatusDot(status_of(term)))
            .child(
                div()
                    .min_w_0()
                    .truncate()
                    .text_color(text::primary())
                    .font_weight(weight::MEDIUM)
                    .child(title),
            )
            .when_some(place, |el, p| {
                el.child(div().flex_none().text_color(text::tertiary()).child(p))
            })
            .child(div().flex_1())
            .when_some(closed, |el, why| el.child(status_tag(why, Status::Off)))
            .when_some(attention, |el, msg| {
                el.child(StateMark::new(WorkState::Blocked).reason(msg))
                    .when_some(probe, |el, probe| el.child(probe.element(Marker::Pane)))
            })
            .child(
                div()
                    .flex()
                    .flex_none()
                    .items_center()
                    .gap(space(2))
                    .child(
                        Switch::new(ElementId::Name(format!("voice-{}", self.id).into()))
                            .checked(voice)
                            .accessibility_label("Voice mode")
                            .on_click(cx.listener(|this, on: &bool, _, cx| {
                                voice_mode(&this.core, &this.id, Some(*on), cx)
                            })),
                    )
                    .child(div().text_color(text::secondary()).child(if voice {
                        "Voice on"
                    } else {
                        "Voice off"
                    })),
            )
    }
}

impl EventEmitter<PanelEvent> for TerminalPanel {}

impl Focusable for TerminalPanel {
    fn focus_handle(&self, cx: &App) -> FocusHandle {
        self.pane.focus_handle(cx)
    }
}

impl BasePanel for TerminalPanel {
    fn panel_name(&self) -> &'static str {
        TERMINAL_PANEL
    }

    fn zoomable(&self, _: &App) -> bool {
        true
    }

    /// The pane of the displayed tab takes the keys, also after a restore.
    fn set_active(&mut self, active: bool, window: &mut Window, cx: &mut Context<Self>) {
        self.shown = active;
        if active {
            window.focus(&self.pane.focus_handle(cx), cx);
        }
    }

    /// The owner closed the tab: the terminal ends too. A quit keeps it.
    fn on_removed(&mut self, _: &mut Window, cx: &mut Context<Self>) {
        let link = self.core.read(cx).link.clone();
        if let Some((c, open)) = self.channel() {
            open.send(&Frame::close(c, ""));
        }
        self.stream = Stream::Closed;
        if QUITTING.load(Ordering::Relaxed) {
            return;
        }
        let id = self.id.clone();
        let was_focused = self.core.update(cx, |s, cx| {
            s.close(&id);
            cx.notify();
            let was = s.focused.as_deref() == Some(id.as_str());
            if was {
                s.focused = None;
            }
            was
        });
        // The pane goes away with no Focus(false): report the end of its focus here.
        if was_focused {
            drop(link.call::<FocusReport>(&FocusReportParams::default()));
        }
        drop(link.call::<Kill>(&TerminalId { id }));
    }

    fn dump(&self, cx: &App) -> PanelState {
        PanelState {
            info: terminal_info(&self.id, self.core.read(cx).remote.as_deref()),
            ..PanelState::new(TERMINAL_PANEL)
        }
    }
}

impl Panel for TerminalPanel {
    fn title(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let core = self.core.read(cx);
        let term = core.terminals.get(&self.id);
        let shown = shown(core, &self.id, cx);
        // The same title on two hosts must not read as one terminal.
        let title = match &core.remote {
            Some(host) => format!("{host} · {}", shown.title),
            None => shown.title,
        };
        let work = work_state_of(term);
        div()
            .flex()
            .items_center()
            .gap(space(2))
            .min_w_0()
            .glow(format!("tab-glow:{}", self.id), glow_of(term))
            .when_some(shown.hue, |el, h| {
                el.child(tab_bar(h, format!("tab-color:{}", self.id)))
            })
            .child(StatusDot(status_of(term)))
            .when_some(
                matches!(work, WorkState::Blocked | WorkState::Failed)
                    .then(|| state_icon(work))
                    .flatten(),
                |el, icon| {
                    el.child(
                        Icon::new(icon)
                            .flex_none()
                            .size(control::icon_sm())
                            .text_color(work.color().hsla()),
                    )
                },
            )
            .child(
                div()
                    .id(SharedString::from(format!("tab-title:{}", self.id)))
                    .min_w_0()
                    .truncate()
                    .when_some(shown.tooltip, |el, program| {
                        el.tooltip(move |window, cx| {
                            gpui_kit::component::tooltip::Tooltip::new(program.clone())
                                .build(window, cx)
                        })
                    })
                    .child(title),
            )
    }

    fn inner_padding(&self, _: &App) -> bool {
        false
    }
}

impl Render for TerminalPanel {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let blocked = glow_of(self.core.read(cx).terminals.get(&self.id));
        div()
            .size_full()
            .flex()
            .flex_col()
            .bg(surface::base())
            .glow(format!("pane-glow:{}", self.id), blocked)
            .child(
                self.header.clone().cached(
                    StyleRefinement::default()
                        .w_full()
                        .h(control::height_md())
                        .flex_none(),
                ),
            )
            .child(div().flex_1().min_h_0().child(self.pane.clone()))
    }
}

#[cfg(test)]
mod tests {
    use std::os::unix::net::UnixStream;
    use std::sync::Arc;

    use gpui::TestAppContext;
    use gpui_kit::{AppContext as _, Entity};

    use super::{Stream, TerminalPanel};
    use crate::link::Link;
    use crate::model::two_cores::methods;
    use crate::model::{CoreEvent, CoreState, HostState};

    /// A new link of the host, and the socket of its core.
    fn host_link() -> (Arc<Link>, UnixStream) {
        let (end, other) = UnixStream::pair().unwrap();
        other.set_nonblocking(true).unwrap();
        (Arc::new(Link::for_tests(end)), other)
    }

    fn panel(
        cx: &mut TestAppContext,
    ) -> (
        Entity<CoreState>,
        Entity<TerminalPanel>,
        &mut gpui::VisualTestContext,
    ) {
        cx.update(agentsi_ui::init);
        let core = cx.new(|_| CoreState::remote("studio", Vec::new()));
        let c = core.clone();
        let (panel, cx) = cx.add_window_view(|window, cx| {
            TerminalPanel::new("t1-a".into(), c, None, None, window, cx)
        });
        (core, panel, cx)
    }

    fn word(panel: &Entity<TerminalPanel>, cx: &mut gpui::VisualTestContext) -> Option<String> {
        panel.read_with(cx, |p, cx| {
            let h = p.header.read(cx);
            h.shown_state(&p.core.read(cx).host)
        })
    }

    #[gpui::test]
    fn the_header_of_a_lost_pane_follows_the_state_of_the_host(cx: &mut TestAppContext) {
        let (core, panel, cx) = panel(cx);
        cx.run_until_parked();
        assert!(panel.read_with(cx, |p, _| matches!(p.stream, Stream::Lost)));
        assert_eq!(word(&panel, cx).as_deref(), Some("connecting"));
        core.update(cx, |s, cx| {
            s.host = HostState::Off("refused".into());
            cx.notify();
        });
        assert_eq!(word(&panel, cx).as_deref(), Some("device off"));
    }

    #[gpui::test]
    fn a_lost_pane_attaches_on_the_new_link_when_the_host_is_up(cx: &mut TestAppContext) {
        let (core, panel, cx) = panel(cx);
        cx.run_until_parked();
        let (link, mut fake) = host_link();
        core.update(cx, |s, cx| {
            s.link = link;
            s.host = HostState::Up;
            cx.emit(CoreEvent::Up);
        });
        cx.run_until_parked();
        assert_eq!(methods(&mut fake), ["terminal.attach", "voice.mode"]);
        assert_eq!(word(&panel, cx), None);
    }

    #[gpui::test]
    fn an_attach_that_fails_after_the_host_came_back_attaches_again(cx: &mut TestAppContext) {
        cx.update(agentsi_ui::init);
        let core = cx.new(|_| CoreState::remote("studio", Vec::new()));
        let (link, mut fake) = host_link();
        // The host comes back before the failed attach of the old link runs, so the panel
        // takes the event while it attaches.
        let (_panel, cx) = cx.add_window_view(|window, cx| {
            let panel = TerminalPanel::new("t1-a".into(), core.clone(), None, None, window, cx);
            core.update(cx, |s, cx| {
                s.link = link;
                s.host = HostState::Up;
                cx.emit(CoreEvent::Up);
            });
            panel
        });
        cx.run_until_parked();
        assert_eq!(methods(&mut fake), ["terminal.attach"]);
    }
}
