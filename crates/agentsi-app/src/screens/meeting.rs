//! The meeting of a product (U22): one live pane for each role.
//!
//! Each core has its own `MeetingFeed` entity in `CoreState`, and takes each `meeting.*` event
//! of that core. So a local product and a product of a host with the same id share nothing. The
//! feed keeps the last meeting of each product, so a meeting that the CLI started shows
//! here too, and a screen that opens late shows what the feed already has. A screen
//! draws only when the feed changes.

use std::collections::{HashMap, VecDeque};

use agentsi_proto::call_api::{CallGet, CallGetParams, CallGetResult};
use agentsi_proto::events::{Lagged, MeetingEvent};
use agentsi_proto::meeting_api::{
    MeetingCancel, MeetingCancelParams, MeetingProgress, MeetingProgressParams, MeetingStart,
    MeetingStartParams, MeetingStartResult,
};
use agentsi_proto::rpc::Notification;
use agentsi_ui::components::{Button, Tag, status_tag};
use agentsi_ui::style::StyleExt as _;
use agentsi_ui::tokens::{Status, control, font, line, radius, size, space, surface, text, weight};
use gpui_kit::component::IconName;
use gpui_kit::component::input::{Input, InputEvent, InputState};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;
use serde_json::Value;

use super::parts::{CallInfo, first_screen_ready, notice};
use crate::link::CallError;
use crate::menus::{self, Target};
use crate::model::{CoreState, ProductRef};
use crate::nav::{Nav, ScreenRequest};

/// The lines that a pane keeps. A role that writes more loses its oldest lines.
const PANE_LINES: usize = 400;
/// Panes in one row of the screen.
const PANE_COLUMNS: usize = 3;

#[derive(Clone, Debug, PartialEq)]
pub enum PaneState {
    Running,
    Done,
    Failed(String),
}

/// One Claude run of a meeting: a role, or the synthesis.
#[derive(Clone, Debug, PartialEq)]
pub struct Pane {
    pub role: String,
    pub state: PaneState,
    pub lines: VecDeque<String>,
    pub seconds: Option<f64>,
    pub cost_usd: Option<f64>,
}

impl Pane {
    fn new(role: &str) -> Pane {
        Pane {
            role: role.into(),
            state: PaneState::Running,
            lines: VecDeque::new(),
            seconds: None,
            cost_usd: None,
        }
    }

    fn push(&mut self, new: Vec<String>) {
        self.lines.extend(new);
        while self.lines.len() > PANE_LINES {
            self.lines.pop_front();
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub enum MeetingState {
    Running,
    /// The id of the minutes.
    Done(String),
    Failed(String),
}

/// The last meeting of one product.
#[derive(Clone, Debug, PartialEq)]
pub struct LiveMeeting {
    pub meeting: String,
    pub question: String,
    pub stage: String,
    pub state: MeetingState,
    pub panes: Vec<Pane>,
    pub warnings: Vec<String>,
    /// The cost at the end of the meeting.
    pub cost_usd: Option<f64>,
    /// The core dropped events for this app, so a pane can miss lines.
    pub lost_events: bool,
}

impl LiveMeeting {
    fn new(meeting: &str) -> LiveMeeting {
        LiveMeeting {
            meeting: meeting.into(),
            question: String::new(),
            stage: "Starting".into(),
            state: MeetingState::Running,
            panes: Vec::new(),
            warnings: Vec::new(),
            cost_usd: None,
            lost_events: false,
        }
    }

    fn pane(&mut self, role: &str) -> &mut Pane {
        let at = match self.panes.iter().position(|p| p.role == role) {
            Some(at) => at,
            None => {
                self.panes.push(Pane::new(role));
                self.panes.len() - 1
            }
        };
        &mut self.panes[at]
    }

    pub fn running(&self) -> bool {
        self.state == MeetingState::Running
    }

    /// The cost so far: the total at the end, else the sum of the finished runs.
    pub fn cost(&self) -> f64 {
        self.cost_usd
            .unwrap_or_else(|| self.panes.iter().filter_map(|p| p.cost_usd).sum())
    }

    fn progress(&mut self, p: &MeetingProgressParams) {
        let ended = p.phase == "end";
        match p.stage.as_str() {
            "start" => self.question = p.note.clone().unwrap_or_default(),
            "index" if !ended => self.stage = "Reading the product".into(),
            "check" if !ended => self.stage = "Checking the minutes".into(),
            "warning" => self.warnings.extend(p.note.clone()),
            "role" | "synthesis" => {
                if p.stage == "role" {
                    self.stage = "The roles answer".into();
                } else {
                    self.stage = "The PM writes the minutes".into();
                }
                if let Some(role) = &p.role {
                    let pane = self.pane(role);
                    if ended {
                        pane.seconds = p.seconds;
                        pane.cost_usd = p.cost_usd;
                        pane.state = match &p.error {
                            Some(e) => PaneState::Failed(e.clone()),
                            None => PaneState::Done,
                        };
                    } else {
                        pane.state = PaneState::Running;
                    }
                }
            }
            "done" => {
                self.cost_usd = p.cost_usd;
                self.state = match (&p.error, &p.note) {
                    (Some(e), _) => MeetingState::Failed(e.clone()),
                    (None, Some(id)) => MeetingState::Done(id.clone()),
                    (None, None) => MeetingState::Failed("The meeting gave no minutes.".into()),
                };
                self.end();
            }
            _ => {}
        }
    }

    /// The meeting stopped, so no pane runs any more.
    fn end(&mut self) {
        self.stage = "Finished".into();
        for pane in &mut self.panes {
            if pane.state == PaneState::Running {
                pane.state = PaneState::Failed("Stopped with the meeting.".into());
            }
        }
    }
}

fn tool_line(name: &str, input: &Value) -> String {
    let arg = ["file_path", "pattern", "query", "path"]
        .iter()
        .find_map(|k| input.get(*k).and_then(Value::as_str));
    match arg {
        Some(arg) => format!("{name}: {arg}"),
        None => name.to_string(),
    }
}

/// What the structured answer of a run shows: the view of a role, or the priorities of the minutes.
fn answer_lines(input: &Value) -> Vec<String> {
    let mut out: Vec<String> = input
        .get("answer")
        .and_then(Value::as_str)
        .map(|a| a.lines().map(String::from).collect())
        .unwrap_or_default();
    let titles = input
        .get("priorities")
        .and_then(Value::as_array)
        .map_or(&[][..], Vec::as_slice)
        .iter()
        .filter_map(|p| p.get("title").and_then(Value::as_str));
    out.extend(titles.enumerate().map(|(i, t)| format!("{}. {t}", i + 1)));
    out
}

/// The lines of a pane for one stream-json line. A line that is not an assistant
/// message shows nothing: the progress events tell the start and the end of a run.
pub fn pane_lines(stream_line: &str) -> Vec<String> {
    let Ok(v) = serde_json::from_str::<Value>(stream_line) else {
        return Vec::new();
    };
    if v.get("type").and_then(Value::as_str) != Some("assistant") {
        return Vec::new();
    }
    let blocks = v
        .pointer("/message/content")
        .and_then(Value::as_array)
        .map_or(&[][..], Vec::as_slice);
    let mut out = Vec::new();
    for b in blocks {
        match b.get("type").and_then(Value::as_str) {
            Some("text") => {
                let body = b.get("text").and_then(Value::as_str).unwrap_or_default();
                out.extend(body.lines().filter(|l| !l.is_empty()).map(String::from));
            }
            Some("tool_use") => {
                let name = b.get("name").and_then(Value::as_str).unwrap_or("tool");
                let input = b.get("input").unwrap_or(&Value::Null);
                if name == "StructuredOutput" {
                    out.extend(answer_lines(input));
                } else {
                    out.push(tool_line(name, input));
                }
            }
            _ => {}
        }
    }
    out
}

/// The last meeting of each product.
#[derive(Default)]
pub struct Meetings {
    live: HashMap<String, LiveMeeting>,
}

impl Meetings {
    pub fn live(&self, product: &str) -> Option<&LiveMeeting> {
        self.live.get(product)
    }

    /// Takes one event. Returns false when nothing changed.
    pub fn apply(&mut self, n: &Notification) -> bool {
        if let Some(p) = n.parse::<MeetingProgress>() {
            let shown = self.live.get(&p.product);
            if !shown.is_some_and(|r| r.meeting == p.meeting) {
                // The newest meeting of a product owns the screen, so a late event of an older one is dropped.
                if p.stage != "start" && shown.is_some_and(LiveMeeting::running) {
                    return false;
                }
                self.live
                    .insert(p.product.clone(), LiveMeeting::new(&p.meeting));
            }
            if let Some(run) = self.live.get_mut(&p.product) {
                run.progress(&p);
            }
            return true;
        }
        if let Some(e) = n.parse::<MeetingEvent>() {
            let lines = pane_lines(&e.line);
            let run = self.live.values_mut().find(|r| r.meeting == e.meeting);
            return match run {
                Some(run) if !lines.is_empty() => {
                    run.pane(&e.role).push(lines);
                    true
                }
                _ => false,
            };
        }
        if n.parse::<Lagged>().is_some() {
            let mut changed = false;
            for run in self.live.values_mut().filter(|r| r.running()) {
                changed |= !run.lost_events;
                run.lost_events = true;
            }
            return changed;
        }
        false
    }

    /// The link to the core broke, so no meeting can report its end.
    pub fn link_lost(&mut self) -> bool {
        let mut changed = false;
        for run in self.live.values_mut().filter(|r| r.running()) {
            run.state = MeetingState::Failed("The link to the core broke.".into());
            run.end();
            changed = true;
        }
        changed
    }

    /// Ends `meeting` when its `done` event was lost. Returns false when nothing changed.
    pub fn finish(&mut self, product: &str, meeting: &str, state: MeetingState) -> bool {
        match self.live.get_mut(product) {
            Some(run) if run.meeting == meeting && run.running() => {
                run.state = state;
                run.end();
                true
            }
            _ => false,
        }
    }

    /// Takes the reply of `meeting.cancel`. With no such meeting in the core, its `done`
    /// event was lost, and only this ends it here. Returns false when nothing changed.
    pub fn cancel_replied(&mut self, product: &str, meeting: &str, canceled: bool) -> bool {
        let ended = MeetingState::Failed("The meeting no longer runs.".into());
        !canceled && self.finish(product, meeting, ended)
    }

    /// The id of the minutes of the last meeting of `product`, when it ended with them.
    pub fn minutes_of(&self, product: &str) -> Option<&str> {
        match &self.live.get(product)?.state {
            MeetingState::Done(id) => Some(id),
            _ => None,
        }
    }
}

/// The meetings of this window, as an entity that the screens observe.
pub struct MeetingFeed(pub Meetings);

/// Stops the meeting that runs on `product`. With none, nothing happens.
pub fn cancel(product: &str, core: &Entity<CoreState>, cx: &mut App) {
    let Some(feed) = core.read(cx).meetings.clone() else {
        return;
    };
    let Some(meeting) = feed
        .read(cx)
        .0
        .live(product)
        .filter(|r| r.running())
        .map(|r| r.meeting.clone())
    else {
        return;
    };
    let call = core
        .read(cx)
        .link
        .call::<MeetingCancel>(&MeetingCancelParams {
            meeting: meeting.clone(),
        });
    let product = product.to_string();
    cx.spawn(async move |cx| {
        if let Ok(reply) = call.await {
            feed.update(cx, |f, cx| {
                if f.0.cancel_replied(&product, &meeting, reply.canceled) {
                    cx.notify();
                }
            });
        }
    })
    .detach();
}

/// The id of the minutes of the last meeting of `product`.
pub fn minutes_of(product: &str, core: &Entity<CoreState>, cx: &App) -> Option<String> {
    let feed = core.read(cx).meetings.as_ref()?.read(cx);
    feed.0.minutes_of(product).map(String::from)
}

fn meeting_name(product: &str) -> String {
    let ms = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_millis());
    format!("app-{product}-{ms}")
}

pub fn pane_tag(state: &PaneState) -> Tag {
    match state {
        PaneState::Running => status_tag("running", Status::Info),
        PaneState::Done => status_tag("done", Status::Ok),
        PaneState::Failed(_) => status_tag("failed", Status::Danger),
    }
}

/// The figures of a finished run: `12 s · $0.0312`.
pub fn pane_figures(p: &Pane) -> String {
    let mut parts = Vec::new();
    if let Some(s) = p.seconds {
        parts.push(format!("{s:.0} s"));
    }
    if let Some(c) = p.cost_usd {
        parts.push(format!("${c:.4}"));
    }
    parts.join(" · ")
}

/// The line of the toolbar.
pub fn status_line(run: &LiveMeeting) -> String {
    let cost = run.cost();
    match &run.state {
        MeetingState::Running if cost > 0.0 => format!("{} · ${cost:.4} so far", run.stage),
        MeetingState::Running => run.stage.clone(),
        MeetingState::Done(id) => format!("Minutes {id} · ${cost:.4}"),
        MeetingState::Failed(_) => "The meeting stopped".into(),
    }
}

pub struct MeetingScreen {
    product: String,
    core: Entity<CoreState>,
    feed: Option<Entity<MeetingFeed>>,
    input: Option<Entity<InputState>>,
    /// The focus of the screen while the question field is hidden. Without it, the focus stays
    /// on the hidden field and no key reaches the window.
    focus: FocusHandle,
    /// The name of the meeting that this screen started and that has no reply yet.
    starting: Option<String>,
    /// A failed `meeting.start` that reached no run.
    message: Option<String>,
    scrolls: HashMap<String, ScrollHandle>,
    /// What `call.get` said about the minutes of the last meeting that ended.
    call: Option<(String, CallInfo)>,
    /// The minutes whose call is being read.
    asking: Option<String>,
    _subscriptions: Vec<Subscription>,
}

impl MeetingScreen {
    pub fn new(product: String, core: Entity<CoreState>, cx: &mut Context<Self>) -> Self {
        let feed = core.read(cx).meetings.clone();
        let mut subscriptions = Vec::new();
        if let Some(feed) = &feed {
            subscriptions.push(cx.observe(feed, |this, _, cx| {
                this.follow(cx);
                this.check_call(cx);
                cx.notify();
            }));
        }
        first_screen_ready();
        let mut screen = MeetingScreen {
            product,
            core,
            feed,
            input: None,
            focus: cx.focus_handle(),
            starting: None,
            message: None,
            scrolls: HashMap::new(),
            call: None,
            asking: None,
            _subscriptions: subscriptions,
        };
        screen.check_call(cx);
        screen
    }

    pub fn focus_handle(&self) -> FocusHandle {
        self.focus.clone()
    }

    /// The minutes of the meeting on screen, once it is done.
    fn minutes_id(&self, cx: &App) -> Option<String> {
        match &self.live(cx)?.state {
            MeetingState::Done(id) => Some(id.clone()),
            _ => None,
        }
    }

    /// When the meeting on screen has ended, asks the core whether its minutes have a call.
    fn check_call(&mut self, cx: &mut Context<Self>) {
        let Some(id) = self.minutes_id(cx) else {
            return;
        };
        let known = self.call.as_ref().is_some_and(|(known, _)| *known == id);
        if known || self.asking.as_ref() == Some(&id) {
            return;
        }
        self.asking = Some(id.clone());
        let reply = self.core.read(cx).link.call::<CallGet>(&CallGetParams {
            product: self.product.clone(),
            minutes_id: id.clone(),
        });
        cx.spawn(async move |this, cx| {
            let result = reply.await.map_err(|e| e.to_string());
            let _ = this.update(cx, |this, cx| this.call_loaded(id, result, cx));
        })
        .detach();
    }

    /// Keeps what `call.get` said. A call that cannot be read is no call to listen to.
    fn call_loaded(
        &mut self,
        id: String,
        result: Result<CallGetResult, String>,
        cx: &mut Context<Self>,
    ) {
        if self.asking.as_ref() == Some(&id) {
            self.asking = None;
        }
        let info = result.as_ref().map(CallInfo::from).unwrap_or_default();
        self.call = Some((id, info));
        cx.notify();
    }

    fn live<'a>(&self, cx: &'a App) -> Option<&'a LiveMeeting> {
        self.feed.as_ref()?.read(cx).0.live(&self.product)
    }

    /// A pane that shows its last line keeps showing it as lines come.
    fn follow(&self, cx: &App) {
        let Some(run) = self.live(cx) else {
            return;
        };
        for pane in &run.panes {
            if let Some(scroll) = self.scrolls.get(&pane.role)
                && scroll.offset().y <= -scroll.max_offset().y
            {
                scroll.scroll_to_bottom();
            }
        }
    }

    fn input(&mut self, window: &mut Window, cx: &mut Context<Self>) -> Entity<InputState> {
        if let Some(input) = &self.input {
            return input.clone();
        }
        let input =
            cx.new(|cx| InputState::new(window, cx).placeholder("What should the roles answer?"));
        self._subscriptions
            .push(cx.subscribe(&input, |this, _, e: &InputEvent, cx| {
                if let InputEvent::PressEnter { .. } = e {
                    this.start(cx);
                }
            }));
        let focus = input.focus_handle(cx);
        window.defer(cx, move |window, cx| window.focus(&focus, cx));
        self.input = Some(input.clone());
        input
    }

    pub fn start(&mut self, cx: &mut Context<Self>) {
        let Some(input) = &self.input else {
            return;
        };
        let question = input.read(cx).value().trim().to_string();
        if question.is_empty() {
            self.message = Some("Write the question first.".into());
            cx.notify();
            return;
        }
        if self.starting.is_some() || self.live(cx).is_some_and(LiveMeeting::running) {
            return;
        }
        let name = meeting_name(&self.product);
        let call = self
            .core
            .read(cx)
            .link
            .call::<MeetingStart>(&MeetingStartParams {
                product: self.product.clone(),
                question,
                meeting: Some(name.clone()),
                ..MeetingStartParams::default()
            });
        self.starting = Some(name.clone());
        self.message = None;
        cx.notify();
        cx.spawn(async move |this, cx| {
            let result = call.await;
            let _ = this.update(cx, |this, cx| this.start_replied(&name, result, cx));
        })
        .detach();
    }

    /// Takes the reply of `meeting.start`, which also ends a meeting whose `done` event was lost.
    fn start_replied(
        &mut self,
        name: &str,
        result: Result<MeetingStartResult, CallError>,
        cx: &mut Context<Self>,
    ) {
        self.starting = None;
        let shown = self.live(cx).is_some_and(|r| r.meeting == name);
        let ended = match result {
            Ok(r) => r.minutes["id"].as_str().map_or(
                MeetingState::Failed("The meeting gave no minutes.".into()),
                |id| MeetingState::Done(id.to_string()),
            ),
            Err(e) => {
                if !shown {
                    self.message = Some(e.to_string());
                }
                MeetingState::Failed(e.to_string())
            }
        };
        if let Some(feed) = self.feed.clone() {
            let product = self.product.clone();
            feed.update(cx, |f, cx| {
                if f.0.finish(&product, name, ended) {
                    cx.notify();
                }
            });
        }
        cx.notify();
    }

    fn toolbar(&self, run: Option<&LiveMeeting>, cx: &mut Context<Self>) -> Div {
        let running = run.is_some_and(LiveMeeting::running);
        let minutes = run.and_then(|r| match &r.state {
            MeetingState::Done(id) => Some(id.clone()),
            _ => None,
        });
        div()
            .flex()
            .items_center()
            .gap(space(3))
            .h(control::TOOLBAR_HEIGHT)
            .flex_none()
            .pad_x(space(4))
            .border_b_1()
            .border_color(line::border())
            .child(
                div()
                    .text_size(size::md())
                    .font_weight(weight::SEMIBOLD)
                    .text_color(text::primary())
                    .child("Meeting"),
            )
            .when_some(run, |el, r| {
                el.child(
                    div()
                        .flex_1()
                        .min_w_0()
                        .truncate()
                        .text_size(size::sm())
                        .text_color(text::tertiary())
                        .child(status_line(r)),
                )
            })
            .when(run.is_none(), |el| el.child(div().flex_1()))
            .when(running, |el| {
                el.child(
                    Button::new("meeting-cancel")
                        .danger()
                        .small()
                        .label("Cancel")
                        .on_click(cx.listener(|this, _, _, cx| {
                            cancel(&this.product, &this.core, cx);
                        })),
                )
            })
            .when_some(minutes, |el, id| {
                let listen = self
                    .call
                    .as_ref()
                    .is_some_and(|(known, info)| *known == id && info.audio);
                let call_id = id.clone();
                el.child(
                    Button::new("meeting-minutes")
                        .secondary()
                        .small()
                        .label("Open the minutes")
                        .on_click(cx.listener(move |this, _, _, cx| {
                            Nav::open(cx, &this.core, ScreenRequest::minute(&this.product, &id));
                        })),
                )
                .when(listen, |el| {
                    el.child(
                        Button::new("meeting-listen")
                            .secondary()
                            .small()
                            .icon(IconName::Play)
                            .label("Listen")
                            .on_click(cx.listener(move |this, _, _, cx| {
                                Nav::open(
                                    cx,
                                    &this.core,
                                    ScreenRequest::call(&this.product, &call_id),
                                );
                            })),
                    )
                })
            })
    }

    fn form(&self, input: &Entity<InputState>, cx: &mut Context<Self>) -> Div {
        let starting = self.starting.is_some();
        div()
            .flex()
            .flex_col()
            .flex_none()
            .gap(space(2))
            .p(space(4))
            .border_b_1()
            .border_color(line::border())
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap(space(2))
                    .child(div().flex_1().min_w_0().child(Input::new(input)))
                    .child(
                        Button::new("meeting-start")
                            .primary()
                            .small()
                            .label("Start the meeting")
                            .disabled(starting)
                            .on_click(cx.listener(|this, _, _, cx| this.start(cx))),
                    ),
            )
            .child(
                div()
                    .text_size(size::sm())
                    .text_color(text::tertiary())
                    .child("A meeting runs one Claude call for each role, then the synthesis. It costs money and takes minutes."),
            )
            .when_some(self.message.clone(), |el, m| {
                el.child(div().text_size(size::sm()).text_color(text::secondary()).child(m))
            })
    }

    fn pane(&mut self, pane: &Pane, i: usize) -> Stateful<Div> {
        let scroll = self.scrolls.entry(pane.role.clone()).or_default().clone();
        let failure = match &pane.state {
            PaneState::Failed(e) => Some(e.clone()),
            _ => None,
        };
        let empty = pane.lines.is_empty() && failure.is_none();
        div()
            .id(("meeting-pane", i))
            .flex()
            .flex_col()
            .min_h_0()
            .min_w_0()
            .rounded(radius::lg())
            .border_1()
            .border_color(line::border())
            .bg(surface::raised())
            .child(
                div()
                    .flex()
                    .items_center()
                    .flex_none()
                    .gap(space(2))
                    .p(space(2))
                    .border_b_1()
                    .border_color(line::border())
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .truncate()
                            .font_family(font::MONO)
                            .text_size(size::sm())
                            .text_color(text::primary())
                            .child(pane.role.clone()),
                    )
                    .child(
                        div()
                            .text_size(size::xs())
                            .text_color(text::tertiary())
                            .child(pane_figures(pane)),
                    )
                    .child(pane_tag(&pane.state)),
            )
            .child(
                div()
                    .id(("meeting-pane-lines", i))
                    .track_scroll(&scroll)
                    .overflow_y_scroll()
                    .flex_1()
                    .min_h_0()
                    .flex()
                    .flex_col()
                    .gap(space(1))
                    .p(space(2))
                    .text_size(size::sm())
                    .text_color(text::secondary())
                    .children(pane.lines.iter().map(|l| div().child(l.clone())))
                    .when(empty, |el| {
                        el.child(
                            div()
                                .text_color(text::tertiary())
                                .child("Waiting for the first line…"),
                        )
                    })
                    .when_some(failure, |el, e| el.child(div().child(e))),
            )
    }

    fn panes(&mut self, run: &LiveMeeting) -> Div {
        let columns = run.panes.len().clamp(1, PANE_COLUMNS);
        let rows = run.panes.len().div_ceil(columns).max(1);
        let cells: Vec<_> = run
            .panes
            .iter()
            .enumerate()
            .map(|(i, p)| self.pane(p, i))
            .collect();
        div()
            .grid()
            .grid_cols(columns as u16)
            .grid_rows(rows as u16)
            .gap(space(2))
            .flex_1()
            .min_h_0()
            .p(space(2))
            .children(cells)
    }
}

impl Render for MeetingScreen {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let run = self.live(cx).cloned();
        let input = self.input(window, cx);
        let running = run.as_ref().is_some_and(LiveMeeting::running);
        if running {
            // The form hides while a meeting runs. A focused field that is not drawn takes every
            // key, so the screen takes the focus back. The defer runs after the one that
            // focuses a new field.
            let (field, screen) = (input.focus_handle(cx), self.focus.clone());
            window.defer(cx, move |window, cx| {
                if field.is_focused(window) || window.focused(cx).is_none() {
                    window.focus(&screen, cx);
                }
            });
        }
        let product = ProductRef {
            host: self.core.read(cx).remote.clone(),
            id: self.product.clone(),
        };
        let root = div()
            .debug_selector(|| "meeting-screen".into())
            .track_focus(&self.focus)
            .size_full()
            .flex()
            .flex_col()
            .bg(surface::panel())
            .on_mouse_down(MouseButton::Right, move |e, _, cx| {
                menus::request(cx, Target::Meeting(product.clone()), e.position)
            })
            .child(self.toolbar(run.as_ref(), cx))
            .when(!running, |el| el.child(self.form(&input, cx)));
        let Some(run) = run else {
            return root.child(notice(
                "No meeting on this product since the app started.",
                Some("Write a question above, or run agentsi meet in a terminal.".into()),
            ));
        };
        let mut notes: Vec<String> = Vec::new();
        if !run.question.is_empty() {
            notes.push(run.question.clone());
        }
        if let MeetingState::Failed(e) = &run.state {
            notes.push(e.clone());
        }
        notes.extend(run.warnings.iter().cloned());
        if run.lost_events {
            notes.push("The core dropped some events, so a pane can miss lines.".into());
        }
        root.child(
            div()
                .flex()
                .flex_col()
                .flex_none()
                .gap(space(1))
                .pad_x(space(4))
                .pt(space(2))
                .text_size(size::sm())
                .text_color(text::secondary())
                .children(notes.into_iter().map(|n| div().child(n))),
        )
        .child(self.panes(&run))
    }
}

#[cfg(test)]
mod tests {
    use super::CallError;
    use super::{
        Lagged, MeetingEvent, MeetingFeed, MeetingProgress, MeetingProgressParams, MeetingScreen,
        MeetingState, Meetings, Notification, PANE_LINES, Pane, PaneState, cancel, pane_figures,
        pane_lines, status_line,
    };
    use crate::link::Link;
    use crate::link::tests::FakeCore;
    use crate::model::CoreState;
    use agentsi_proto::Event as _;
    use agentsi_proto::call_api::CallGetResult;
    use agentsi_proto::events::MeetingEventParams;
    use agentsi_proto::meeting_api::MeetingStartResult;
    use agentsi_proto::rpc::V2;
    use gpui::{AppContext as _, Entity, Focusable as _, TestAppContext, VisualTestContext};
    use serde_json::json;
    use std::os::unix::net::UnixStream;
    use std::sync::Arc;

    fn note<E: agentsi_proto::Event>(params: &E::Params) -> Notification {
        Notification {
            jsonrpc: V2,
            method: E::NAME.into(),
            params: serde_json::to_value(params).unwrap(),
        }
    }

    fn progress(stage: &str, phase: &str, f: impl Fn(&mut MeetingProgressParams)) -> Notification {
        let mut p = MeetingProgressParams {
            meeting: "m1".into(),
            product: "alpha".into(),
            stage: stage.into(),
            phase: phase.into(),
            ..MeetingProgressParams::default()
        };
        f(&mut p);
        note::<MeetingProgress>(&p)
    }

    fn line(role: &str, text: &str) -> Notification {
        let body =
            json!({"type": "assistant", "message": {"content": [{"type": "text", "text": text}]}});
        note::<MeetingEvent>(&MeetingEventParams {
            meeting: "m1".into(),
            role: role.into(),
            line: body.to_string(),
        })
    }

    struct Ui<'a> {
        screen: Entity<MeetingScreen>,
        core: Entity<CoreState>,
        feed: Entity<MeetingFeed>,
        fake: FakeCore,
        cx: &'a mut VisualTestContext,
    }

    /// A link with no reader thread: a thread of its own would wake a task under the test
    /// scheduler, which aborts the test binary. A test hands each reply to the screen itself.
    fn ui(cx: &mut TestAppContext) -> Ui<'_> {
        cx.update(agentsi_ui::init);
        let (end, other) = UnixStream::pair().unwrap();
        let fake = FakeCore(other);
        let feed = cx.new(|_| MeetingFeed(Meetings::default()));
        let core = cx.new(|_| CoreState {
            meetings: Some(feed.clone()),
            ..CoreState::new(Arc::new(Link::for_tests(end)))
        });
        let for_view = core.clone();
        let (screen, cx) =
            cx.add_window_view(|_, cx| MeetingScreen::new("alpha".into(), for_view, cx));
        cx.update(|window, cx| window.draw(cx).clear(cx));
        Ui {
            screen,
            core,
            feed,
            fake,
            cx,
        }
    }

    impl Ui<'_> {
        fn ask(&mut self, question: &str) {
            let question = question.to_string();
            self.screen.update_in(self.cx, |s, window, cx| {
                let input = s.input.clone().unwrap();
                input.update(cx, |i, cx| i.set_value(question, window, cx));
                s.start(cx);
            });
        }

        fn feed(&mut self, n: &Notification) {
            self.feed.update(self.cx, |f, cx| {
                f.0.apply(n);
                cx.notify();
            });
            self.cx.update(|window, cx| window.draw(cx).clear(cx));
        }

        /// Gives the screen the reply of `meeting.start`, as the task of `start` does.
        fn start_reply(&mut self, name: &str, result: Result<MeetingStartResult, CallError>) {
            self.screen
                .update(self.cx, |s, cx| s.start_replied(name, result, cx));
        }
    }

    #[gpui::test]
    fn a_running_meeting_moves_the_focus_from_the_hidden_field_to_the_screen(
        cx: &mut TestAppContext,
    ) {
        let mut ui = ui(cx);
        ui.cx.run_until_parked();
        let (field, screen) = ui.screen.read_with(ui.cx, |s, cx| {
            (s.input.clone().unwrap().focus_handle(cx), s.focus.clone())
        });
        ui.cx
            .update(|window, _| assert!(field.is_focused(window), "the field starts focused"));
        ui.feed(&progress("start", "start", |p| p.meeting = "m1".into()));
        ui.cx.run_until_parked();
        ui.cx.update(|window, _| {
            assert!(
                !field.is_focused(window),
                "the hidden field keeps the focus"
            );
            assert!(screen.is_focused(window), "the screen takes the focus");
        });
    }

    #[gpui::test]
    fn an_empty_question_starts_nothing(cx: &mut TestAppContext) {
        let mut ui = ui(cx);
        ui.ask("   ");
        ui.screen.read_with(ui.cx, |s, _| {
            assert_eq!(s.message.as_deref(), Some("Write the question first."));
            assert!(s.starting.is_none());
        });
    }

    #[gpui::test]
    fn start_sends_the_question_and_a_second_start_waits(cx: &mut TestAppContext) {
        let mut ui = ui(cx);
        ui.ask("  Why now?  ");
        let req = ui.fake.request();
        assert_eq!(req["method"], "meeting.start");
        assert_eq!(req["params"]["product"], "alpha");
        assert_eq!(req["params"]["question"], "Why now?");
        assert!(req["params"].get("roles").is_none());
        let name = req["params"]["meeting"].as_str().unwrap().to_string();
        assert!(name.starts_with("app-alpha-"), "{name}");

        ui.ask("Again?");
        let started = ui.screen.read_with(ui.cx, |s, _| s.starting.clone());
        assert_eq!(started.as_deref(), Some(name.as_str()));

        let shown = name.clone();
        ui.feed(&progress("start", "start", move |p| {
            p.meeting = shown.clone()
        }));
        let reply = MeetingStartResult {
            meeting: name.clone(),
            minutes: json!({"id": "2026-a"}),
            usage: Vec::new(),
            total_cost_usd: 0.0,
        };
        ui.start_reply(&name, Ok(reply));
        ui.screen.read_with(ui.cx, |s, _| {
            assert!(s.message.is_none());
            assert!(s.starting.is_none());
        });
        // No `done` event came, so the reply ended the meeting.
        ui.feed.read_with(ui.cx, |f, _| {
            assert_eq!(f.0.minutes_of("alpha"), Some("2026-a"));
        });
    }

    #[gpui::test]
    fn listen_shows_after_the_minutes_of_the_meeting_have_a_call(cx: &mut TestAppContext) {
        let (nav, mut requests) = crate::nav::Nav::channel();
        cx.update(|cx| cx.set_global(nav));
        let mut ui = ui(cx);
        ui.ask("Why now?");
        let req = ui.fake.request();
        let name = req["params"]["meeting"].as_str().unwrap().to_string();
        let shown = name.clone();
        ui.feed(&progress("start", "start", move |p| {
            p.meeting = shown.clone()
        }));
        let reply = MeetingStartResult {
            meeting: name.clone(),
            minutes: json!({"id": "2026-a"}),
            usage: Vec::new(),
            total_cost_usd: 0.0,
        };
        ui.start_reply(&name, Ok(reply));
        ui.cx.update(|window, cx| window.draw(cx).clear(cx));
        // The meeting ended: the screen asks whether the minutes have a call.
        let ask = ui.fake.request();
        assert_eq!(ask["method"], "call.get");
        assert_eq!(
            ask["params"],
            json!({"product": "alpha", "minutes_id": "2026-a"})
        );
        assert!(ui.cx.debug_bounds("button:meeting-minutes").is_some());
        assert!(ui.cx.debug_bounds("button:meeting-listen").is_none());

        let call = |extra: serde_json::Value| -> CallGetResult {
            let mut v = json!({"product": "alpha", "minutes_id": "2026-a", "question": "Q?",
                "segments": [], "timed": true, "heard": false});
            v.as_object_mut()
                .unwrap()
                .extend(extra.as_object().unwrap().clone());
            serde_json::from_value(v).unwrap()
        };
        ui.screen.update(ui.cx, |s, cx| {
            s.call_loaded("2026-a".into(), Ok(call(json!({}))), cx)
        });
        ui.cx.update(|window, cx| window.draw(cx).clear(cx));
        assert!(ui.cx.debug_bounds("button:meeting-listen").is_none());

        ui.screen.update(ui.cx, |s, cx| {
            let audio = json!({"audio": {"format": "opus", "bytes": 10}});
            s.call_loaded("2026-a".into(), Ok(call(audio)), cx)
        });
        ui.cx.update(|window, cx| window.draw(cx).clear(cx));
        let listen = ui
            .cx
            .debug_bounds("button:meeting-listen")
            .expect("a Listen button");
        ui.cx
            .simulate_click(listen.center(), gpui::Modifiers::none());
        assert_eq!(
            requests.try_recv().unwrap(),
            crate::nav::ScreenRequest::call("alpha", "2026-a")
        );
    }

    #[test]
    fn a_cancel_that_finds_no_meeting_ends_it_here() {
        let mut m = Meetings::default();
        m.apply(&progress("start", "start", |_| {}));
        assert!(!m.cancel_replied("alpha", "m1", true));
        assert!(m.live("alpha").unwrap().running());
        assert!(m.cancel_replied("alpha", "m1", false));
        assert!(!m.live("alpha").unwrap().running());
    }

    #[gpui::test]
    fn cancel_names_the_meeting_that_runs(cx: &mut TestAppContext) {
        let mut ui = ui(cx);
        ui.ask("Why now?");
        let start = ui.fake.request();
        let name = start["params"]["meeting"].as_str().unwrap().to_string();
        let first = name.clone();
        ui.feed(&progress("start", "start", move |p| {
            p.meeting = first.clone();
            p.note = Some("Why now?".into());
        }));
        let (core, cx) = (ui.core.clone(), &mut *ui.cx);
        cx.update(|_, cx| cancel("alpha", &core, cx));
        let req = ui.fake.request();
        assert_eq!(req["method"], "meeting.cancel");
        assert_eq!(req["params"]["meeting"], json!(name));

        // The run shows the reason, so the error of the reply adds no second line.
        let last = name.clone();
        ui.feed(&progress("done", "end", move |p| {
            p.meeting = last.clone();
            p.error = Some("Canceled.".into());
        }));
        ui.start_reply(&name, Err(CallError::Link("Canceled.".into())));
        ui.screen
            .read_with(ui.cx, |s, _| assert!(s.message.is_none()));
    }

    #[gpui::test]
    fn a_start_that_reaches_no_run_shows_its_error(cx: &mut TestAppContext) {
        let mut ui = ui(cx);
        ui.ask("Why now?");
        let req = ui.fake.request();
        let name = req["params"]["meeting"].as_str().unwrap().to_string();
        ui.start_reply(&name, Err(CallError::Link("Unknown product".into())));
        let message = ui.screen.read_with(ui.cx, |s, _| s.message.clone());
        assert!(message.unwrap().contains("Unknown product"));
    }

    #[gpui::test]
    fn two_cores_with_the_same_product_id_share_no_meeting(cx: &mut TestAppContext) {
        use crate::model::take_event;
        use crate::model::two_cores::{methods, two_cores};
        cx.update(agentsi_ui::init);
        let mut t = two_cores(cx);
        let local = t.cores.local.clone();
        let studio = t.cores.of(Some("studio")).unwrap().clone();
        for core in [&local, &studio] {
            let feed = cx.new(|_| MeetingFeed(Meetings::default()));
            core.update(cx, |s, _| s.meetings = Some(feed));
        }
        let shown = |core: &Entity<CoreState>, cx: &mut TestAppContext| {
            core.read_with(cx, |s, cx| {
                let feed = s.meetings.as_ref().unwrap().read(cx);
                feed.0.live("alpha").map(|r| r.meeting.clone())
            })
        };
        let mut a = cx.to_async();
        take_event(&studio, &progress("start", "start", |_| {}), &mut a);
        assert_eq!(shown(&studio, cx).as_deref(), Some("m1"));
        assert_eq!(shown(&local, cx), None);

        // A local meeting of the same product and the same name stays on its own feed.
        take_event(&local, &progress("start", "start", |_| {}), &mut a);
        take_event(&studio, &line("dev", "On the host"), &mut a);
        let lines = |core: &Entity<CoreState>, cx: &mut TestAppContext| {
            core.read_with(cx, |s, cx| {
                let feed = s.meetings.as_ref().unwrap().read(cx);
                feed.0.live("alpha").unwrap().panes.len()
            })
        };
        assert_eq!(lines(&studio, cx), 1);
        assert_eq!(lines(&local, cx), 0);

        // A cancel of the host reaches its core only, and a lost host ends only its meeting.
        cx.update(|cx| cancel("alpha", &studio, cx));
        assert_eq!(methods(&mut t.studio), ["meeting.cancel"]);
        assert_eq!(methods(&mut t.local), Vec::<String>::new());
        studio.update(cx, |s, cx| s.meetings_lost(cx));
        let running = |core: &Entity<CoreState>, cx: &mut TestAppContext| {
            core.read_with(cx, |s, cx| {
                let feed = s.meetings.as_ref().unwrap().read(cx);
                feed.0.live("alpha").unwrap().running()
            })
        };
        assert!(!running(&studio, cx));
        assert!(running(&local, cx));
    }

    #[gpui::test]
    fn a_meeting_screen_of_a_host_starts_its_meeting_on_that_core(cx: &mut TestAppContext) {
        use crate::model::two_cores::{methods, two_cores};
        cx.update(agentsi_ui::init);
        let mut t = two_cores(cx);
        let studio = t.cores.of(Some("studio")).unwrap().clone();
        let (screen, cx) =
            cx.add_window_view(|_, cx| MeetingScreen::new("alpha".into(), studio, cx));
        cx.update(|window, cx| window.draw(cx).clear(cx));
        screen.update_in(cx, |s, window, cx| {
            let input = s.input.clone().unwrap();
            input.update(cx, |i, cx| i.set_value("What next?", window, cx));
            s.start(cx);
        });
        assert_eq!(methods(&mut t.studio), ["meeting.start"]);
        assert_eq!(methods(&mut t.local), Vec::<String>::new());
    }

    #[gpui::test]
    fn cancel_with_no_running_meeting_sends_nothing(cx: &mut TestAppContext) {
        let ui = ui(cx);
        let core = ui.core.clone();
        ui.cx.update(|_, cx| cancel("alpha", &core, cx));
        ui.cx.update(|_, cx| cancel("other", &core, cx));
        ui.fake.0.set_nonblocking(true).unwrap();
        let mut byte = [0u8; 1];
        let read = std::io::Read::read(&mut &ui.fake.0, &mut byte);
        assert_eq!(read.unwrap_err().kind(), std::io::ErrorKind::WouldBlock);
    }

    #[gpui::test]
    fn the_screen_draws_in_each_state(cx: &mut TestAppContext) {
        let mut ui = ui(cx);
        ui.feed(&progress("start", "start", |p| {
            p.note = Some("Why?".into())
        }));
        ui.feed(&progress("role", "start", |p| p.role = Some("dev".into())));
        ui.feed(&line("dev", "Hello"));
        ui.feed(&progress("warning", "end", |p| {
            p.note = Some("Careful.".into())
        }));
        ui.feed(&progress("role", "end", |p| {
            p.role = Some("qa".into());
            p.error = Some("boom".into());
        }));
        ui.feed(&progress("done", "end", |p| p.note = Some("2026-a".into())));
        ui.feed(&progress("start", "start", |p| p.meeting = "m2".into()));
        ui.feed(&progress("done", "end", |p| {
            p.meeting = "m2".into();
            p.error = Some("Canceled.".into());
        }));
    }

    #[test]
    fn the_stream_shows_text_tools_and_the_structured_answer() {
        let text = json!({"type": "assistant", "message": {"content": [
            {"type": "text", "text": "First\n\nSecond"},
            {"type": "tool_use", "name": "Read", "input": {"file_path": "docs/a.md"}},
            {"type": "tool_use", "name": "Other", "input": {}},
        ]}});
        assert_eq!(
            pane_lines(&text.to_string()),
            ["First", "Second", "Read: docs/a.md", "Other"]
        );
        let answer = json!({"type": "assistant", "message": {"content": [
            {"type": "tool_use", "name": "StructuredOutput", "input": {
                "answer": "My view", "priorities": [{"title": "Ship"}, {"title": "Test"}]}},
        ]}});
        assert_eq!(
            pane_lines(&answer.to_string()),
            ["My view", "1. Ship", "2. Test"]
        );
        assert!(pane_lines("not json").is_empty());
        assert!(pane_lines(r#"{"type":"result","result":"x"}"#).is_empty());
    }

    #[test]
    fn a_meeting_fills_its_panes_and_ends_with_its_minutes() {
        let mut m = Meetings::default();
        assert!(m.apply(&progress("start", "start", |p| p.note = Some("Why?".into()))));
        m.apply(&progress("index", "start", |_| {}));
        assert_eq!(m.live("alpha").unwrap().stage, "Reading the product");
        m.apply(&progress("role", "start", |p| p.role = Some("dev".into())));
        assert!(m.apply(&line("dev", "Hello")));
        m.apply(&progress("role", "end", |p| {
            p.role = Some("dev".into());
            p.seconds = Some(12.4);
            p.cost_usd = Some(0.03);
        }));
        m.apply(&progress("role", "end", |p| {
            p.role = Some("qa".into());
            p.error = Some("boom".into());
        }));
        let run = m.live("alpha").unwrap();
        assert_eq!(run.question, "Why?");
        assert_eq!(run.panes[0].lines, ["Hello"]);
        assert_eq!(run.panes[0].state, PaneState::Done);
        assert_eq!(pane_figures(&run.panes[0]), "12 s · $0.0300");
        assert_eq!(run.panes[1].state, PaneState::Failed("boom".into()));
        assert_eq!(status_line(run), "The roles answer · $0.0300 so far");
        m.apply(&progress("done", "end", |p| {
            p.note = Some("2026-a".into());
            p.cost_usd = Some(0.5);
        }));
        let run = m.live("alpha").unwrap();
        assert_eq!(run.state, MeetingState::Done("2026-a".into()));
        assert_eq!(status_line(run), "Minutes 2026-a · $0.5000");
    }

    #[test]
    fn a_late_event_of_an_older_meeting_changes_nothing() {
        let mut m = Meetings::default();
        m.apply(&progress("start", "start", |p| p.meeting = "new".into()));
        m.apply(&progress("role", "start", |p| {
            p.meeting = "new".into();
            p.role = Some("dev".into());
        }));
        let old = progress("role", "end", |p| p.role = Some("qa".into()));
        assert!(!m.apply(&old));
        let run = m.live("alpha").unwrap();
        assert_eq!((run.meeting.as_str(), run.panes.len()), ("new", 1));
    }

    #[test]
    fn a_meeting_that_ended_gives_way_to_any_event_of_another() {
        let mut m = Meetings::default();
        m.apply(&progress("done", "end", |p| p.note = Some("2026-a".into())));
        assert_eq!(m.minutes_of("alpha"), Some("2026-a"));
        assert!(m.apply(&progress("role", "start", |p| {
            p.meeting = "m2".into();
            p.role = Some("dev".into());
        })));
        assert_eq!(m.live("alpha").unwrap().meeting, "m2");
        assert_eq!(m.minutes_of("alpha"), None);
    }

    #[test]
    fn finish_ends_a_meeting_whose_done_event_was_lost() {
        let mut m = Meetings::default();
        m.apply(&progress("role", "start", |p| p.role = Some("dev".into())));
        assert!(!m.finish("alpha", "other", MeetingState::Done("x".into())));
        assert!(m.finish("alpha", "m1", MeetingState::Done("2026-a".into())));
        let run = m.live("alpha").unwrap();
        assert_eq!(run.state, MeetingState::Done("2026-a".into()));
        assert!(matches!(run.panes[0].state, PaneState::Failed(_)));
        assert!(!m.finish("alpha", "m1", MeetingState::Done("again".into())));
    }

    #[test]
    fn a_new_meeting_of_a_product_replaces_the_last_one() {
        let mut m = Meetings::default();
        m.apply(&progress("role", "start", |p| p.role = Some("dev".into())));
        m.apply(&progress("start", "start", |p| p.meeting = "m2".into()));
        let run = m.live("alpha").unwrap();
        assert_eq!((run.meeting.as_str(), run.panes.len()), ("m2", 0));
    }

    #[test]
    fn a_line_of_an_unknown_meeting_changes_nothing() {
        let mut m = Meetings::default();
        assert!(!m.apply(&line("dev", "Hello")));
    }

    #[test]
    fn a_pane_keeps_only_its_last_lines() {
        let mut p = Pane::new("dev");
        p.push((0..PANE_LINES + 5).map(|i| i.to_string()).collect());
        assert_eq!(p.lines.len(), PANE_LINES);
        assert_eq!(p.lines[0], "5");
    }

    #[test]
    fn a_canceled_meeting_stops_its_running_panes() {
        let mut m = Meetings::default();
        m.apply(&progress("role", "start", |p| p.role = Some("dev".into())));
        m.apply(&progress("done", "end", |p| {
            p.error = Some("Canceled.".into())
        }));
        let run = m.live("alpha").unwrap();
        assert_eq!(run.state, MeetingState::Failed("Canceled.".into()));
        assert!(matches!(run.panes[0].state, PaneState::Failed(_)));
    }

    #[test]
    fn lost_events_and_a_lost_link_mark_only_a_running_meeting() {
        let mut m = Meetings::default();
        m.apply(&progress("role", "start", |p| p.role = Some("dev".into())));
        let lagged = Notification {
            jsonrpc: V2,
            method: Lagged::NAME.into(),
            params: json!({"missed": 2}),
        };
        assert!(m.apply(&lagged));
        assert!(m.live("alpha").unwrap().lost_events);
        assert!(!m.apply(&lagged));
        assert!(m.link_lost());
        assert!(!m.link_lost());
        let run = m.live("alpha").unwrap();
        assert!(!run.running());
        assert!(matches!(run.panes[0].state, PaneState::Failed(_)));
    }
}
