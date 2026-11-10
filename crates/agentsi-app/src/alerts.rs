//! The alert center: the work state of each target, and the alerts that a change makes.
//!
//! [`apply`] and its two siblings are pure: they take one input from the core and give the
//! alerts that it makes. [`AlertCenter`] keeps the active alerts, which make the glow, and
//! the log. The gpui global [`Alerts`] holds the center. Spec:
//! `docs/specs/agentsi-desktop-immersive.md`, "Alert center".

use std::collections::{HashMap, HashSet, VecDeque};
use std::time::SystemTime;

use agentsi_proto::desk_api::PendingDecision;
use agentsi_proto::events::{ModelsError, ProgramState, TerminalExit, TerminalState};
use agentsi_proto::meeting_api::MeetingProgress;
use agentsi_proto::rpc::Notification;
use agentsi_ui::tokens::WorkState;
use gpui_kit::{App, AppContext as _, Context, Entity, EventEmitter, Global};

use crate::model::{StorySessionItem, attention_text};
use crate::tree::ItemKey;

/// The alerts that the log keeps.
pub const LOG_MAX: usize = 200;

/// What an alert is about. A host is `None` for the core of this device.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum AlertTarget {
    Pane {
        host: Option<String>,
        id: String,
    },
    Meeting {
        host: Option<String>,
        product: String,
    },
    Story {
        host: Option<String>,
        product: String,
        id: String,
    },
    /// The decisions that wait for the owner, for one product.
    Decisions {
        host: Option<String>,
        product: String,
    },
    Model {
        id: String,
    },
}

impl AlertTarget {
    /// The target that an item row of the tree opens.
    pub fn of_item(host: Option<&str>, product: &str, key: &ItemKey) -> Option<AlertTarget> {
        let host = host.map(str::to_string);
        let product = product.to_string();
        match key {
            ItemKey::Pane(id) => Some(AlertTarget::Pane {
                host,
                id: id.clone(),
            }),
            ItemKey::Meeting => Some(AlertTarget::Meeting { host, product }),
            ItemKey::Story(id) => Some(AlertTarget::Story {
                host,
                product,
                id: id.clone(),
            }),
            ItemKey::Reflection => None,
        }
    }

    pub fn host(&self) -> Option<&str> {
        match self {
            AlertTarget::Pane { host, .. }
            | AlertTarget::Meeting { host, .. }
            | AlertTarget::Story { host, .. }
            | AlertTarget::Decisions { host, .. } => host.as_deref(),
            AlertTarget::Model { .. } => None,
        }
    }

    pub fn product(&self) -> Option<&str> {
        match self {
            AlertTarget::Meeting { product, .. }
            | AlertTarget::Story { product, .. }
            | AlertTarget::Decisions { product, .. } => Some(product),
            AlertTarget::Pane { .. } | AlertTarget::Model { .. } => None,
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct Alert {
    pub id: u64,
    pub target: AlertTarget,
    pub state: WorkState,
    /// The reason, without the word of the state.
    pub text: String,
    /// The pane title for a pane, else the name of the agent or the story. The center fills it.
    pub label: String,
    /// The product of the target. The center fills it for a pane.
    pub product: Option<String>,
    /// The alert comes from a terminal bell: its glow ends when the pane gets the focus.
    pub bell: bool,
    pub at: SystemTime,
}

impl Alert {
    /// The word of the state and the reason: "Blocked: Claude asks for a permission".
    pub fn line(&self) -> String {
        format!("{}: {}", self.state.word(), self.text)
    }

    /// Only Blocked, Failed and Review make an alert.
    pub fn is_alertable(state: WorkState) -> bool {
        matches!(
            state,
            WorkState::Blocked | WorkState::Failed | WorkState::Review
        )
    }
}

/// The work state of each target of one core, and what the last read showed.
#[derive(Debug, Default)]
pub struct AlertState {
    host: Option<String>,
    states: HashMap<AlertTarget, WorkState>,
    /// The story lists that were read, by product: the first read makes no alert.
    stories: HashSet<String>,
    decisions: HashMap<String, HashSet<i64>>,
    next: u64,
    ended: Vec<AlertTarget>,
}

impl AlertState {
    pub fn new(host: Option<String>) -> AlertState {
        AlertState {
            host,
            ..AlertState::default()
        }
    }

    /// The work state that the alert center knows for `target`.
    #[cfg(test)]
    pub fn state_of(&self, target: &AlertTarget) -> Option<WorkState> {
        self.states.get(target).copied()
    }

    /// The targets whose alert state ended since the last call.
    pub fn take_ended(&mut self) -> Vec<AlertTarget> {
        std::mem::take(&mut self.ended)
    }

    fn alert(
        &mut self,
        target: AlertTarget,
        state: WorkState,
        text: String,
        label: String,
    ) -> Alert {
        self.next += 1;
        let product = target.product().map(str::to_string);
        Alert {
            id: self.next,
            target,
            state,
            text,
            label,
            product,
            bell: false,
            at: SystemTime::now(),
        }
    }

    /// Moves `target` to `state`. A move into Blocked, Failed or Review makes an alert, and a
    /// move out of one of them ends its alert. `force` makes an alert for the same state.
    fn move_to(
        &mut self,
        target: AlertTarget,
        state: WorkState,
        text: impl FnOnce() -> String,
        label: String,
        force: bool,
    ) -> Option<Alert> {
        let old = self.states.insert(target.clone(), state);
        if old != Some(state) && old.is_some_and(Alert::is_alertable) {
            self.ended.push(target.clone());
        }
        if Alert::is_alertable(state) && (force || old != Some(state)) {
            return Some(self.alert(target, state, text(), label));
        }
        // A pane that finishes its work makes a Done alert, for the tone only.
        if state == WorkState::Done
            && old == Some(WorkState::Working)
            && matches!(target, AlertTarget::Pane { .. })
        {
            return Some(self.alert(target, state, "Done".into(), label));
        }
        None
    }

    fn pane(&self, id: &str) -> AlertTarget {
        AlertTarget::Pane {
            host: self.host.clone(),
            id: id.to_string(),
        }
    }

    fn meeting(&self, product: &str) -> AlertTarget {
        AlertTarget::Meeting {
            host: self.host.clone(),
            product: product.to_string(),
        }
    }

    /// A bell of the terminal `id`: event 21. It always makes an alert.
    pub fn bell(&mut self, id: &str) -> Alert {
        let target = self.pane(id);
        let mut alert = self.alert(target, WorkState::Blocked, "Bell".into(), String::new());
        alert.bell = true;
        alert
    }

    /// The story sessions that `story.list` gave for `product`. The first list of a product
    /// makes no alert, so a start of the app is quiet.
    pub fn stories(&mut self, product: &str, items: &[StorySessionItem]) -> Vec<Alert> {
        let first = self.stories.insert(product.to_string());
        let mut out = Vec::new();
        let seen: HashSet<&str> = items.iter().map(|i| i.id.as_str()).collect();
        let gone: Vec<AlertTarget> = self
            .states
            .keys()
            .filter(|t| matches!(t, AlertTarget::Story { product: p, id, .. } if p == product && !seen.contains(id.as_str())))
            .cloned()
            .collect();
        for target in gone {
            if self.states.remove(&target).is_some_and(Alert::is_alertable) {
                self.ended.push(target);
            }
        }
        for item in items {
            let state = match item.state.as_str() {
                "ready" => WorkState::Review,
                "failed" => WorkState::Failed,
                _ => WorkState::Working,
            };
            let target = AlertTarget::Story {
                host: self.host.clone(),
                product: product.to_string(),
                id: item.id.clone(),
            };
            let name = format!("{} {}", item.id, item.title);
            let text = match state {
                WorkState::Review => format!("{name} is ready for review"),
                _ => format!("{name} failed"),
            };
            let made = self.move_to(target, state, || text, name.clone(), false);
            if !first {
                out.extend(made);
            }
        }
        out
    }

    /// The pending decisions that `product.desk` gave for `product`. A decision that the
    /// last read did not show makes one alert for the new ones. The first read makes none.
    pub fn desk(&mut self, product: &str, pending: &[PendingDecision]) -> Vec<Alert> {
        let now: HashSet<i64> = pending.iter().map(|d| d.id).collect();
        let before = self.decisions.insert(product.to_string(), now.clone());
        let target = AlertTarget::Decisions {
            host: self.host.clone(),
            product: product.to_string(),
        };
        let new: Vec<&PendingDecision> = match &before {
            Some(b) => pending.iter().filter(|d| !b.contains(&d.id)).collect(),
            None => Vec::new(),
        };
        if now.is_empty() {
            if self.states.remove(&target).is_some() {
                self.ended.push(target);
            }
            return Vec::new();
        }
        if new.is_empty() {
            return Vec::new();
        }
        let text = match new.as_slice() {
            [one] => one.text.clone(),
            many => format!("{} decisions wait for you", many.len()),
        };
        self.states.insert(target.clone(), WorkState::Review);
        vec![self.alert(target, WorkState::Review, text, "Decisions".into())]
    }
}

/// Does the `note` of a finished check say that some checks failed: "3 failures: ..."?
fn check_failed(note: &str) -> bool {
    let Some((count, _)) = note.split_once(" failures:") else {
        return false;
    };
    count.parse::<u32>().is_ok_and(|n| n > 0)
}

/// Takes one event of the core and gives the alerts that it makes.
pub fn apply(st: &mut AlertState, n: &Notification) -> Vec<Alert> {
    if let Some(p) = n.parse::<TerminalState>() {
        let target = st.pane(&p.id);
        let made = match p.state {
            ProgramState::Blocked => {
                match attention_text(p.kind.as_deref().unwrap_or(""), p.msg.as_deref()) {
                    Some(text) => {
                        st.move_to(target, WorkState::Blocked, || text, String::new(), false)
                    }
                    None => st.move_to(
                        target,
                        WorkState::Working,
                        String::new,
                        String::new(),
                        false,
                    ),
                }
            }
            ProgramState::Error => st.move_to(
                target,
                WorkState::Failed,
                || {
                    p.msg
                        .unwrap_or_else(|| "The program reported an error".into())
                },
                String::new(),
                false,
            ),
            ProgramState::Working => st.move_to(
                target,
                WorkState::Working,
                String::new,
                String::new(),
                false,
            ),
            ProgramState::Done => {
                st.move_to(target, WorkState::Done, String::new, String::new(), false)
            }
            ProgramState::Idle => {
                st.move_to(target, WorkState::Off, String::new, String::new(), false)
            }
        };
        return made.into_iter().collect();
    }
    if let Some(p) = n.parse::<TerminalExit>() {
        let target = st.pane(&p.id);
        let state = if p.code == Some(0) {
            WorkState::Done
        } else {
            WorkState::Off
        };
        st.move_to(target, state, String::new, String::new(), false);
        return Vec::new();
    }
    if let Some(p) = n.parse::<MeetingProgress>() {
        let target = st.meeting(&p.product);
        let role = p.role.clone().unwrap_or_default();
        let made = match (p.stage.as_str(), p.phase.as_str()) {
            ("start", _) => st.move_to(
                target,
                WorkState::Working,
                String::new,
                String::new(),
                false,
            ),
            ("role", "end") if p.error.is_some() => {
                let why = p.error.clone().unwrap_or_default();
                st.move_to(
                    target,
                    WorkState::Failed,
                    || format!("{role}: {why}"),
                    role.clone(),
                    false,
                )
            }
            ("check", "end") if p.note.as_deref().is_some_and(check_failed) => st.move_to(
                target,
                WorkState::Repairing,
                String::new,
                String::new(),
                false,
            ),
            ("done", _) => match (&p.error, &p.note) {
                (Some(why), _) => st.move_to(
                    target,
                    WorkState::Failed,
                    || format!("The meeting ended: {why}"),
                    String::new(),
                    true,
                ),
                (None, note) => {
                    let text = match note {
                        Some(id) => format!("Minutes {id} written"),
                        None => "Minutes written".into(),
                    };
                    st.move_to(target, WorkState::Review, || text, String::new(), true)
                }
            },
            _ => None,
        };
        return made.into_iter().collect();
    }
    if let Some(p) = n.parse::<ModelsError>() {
        let target = AlertTarget::Model { id: p.id.clone() };
        return st
            .move_to(target, WorkState::Failed, || p.message, p.id, true)
            .into_iter()
            .collect();
    }
    Vec::new()
}

/// The center: the active alerts, which make the glow, and the log.
#[derive(Default)]
pub struct AlertCenter {
    states: HashMap<Option<String>, AlertState>,
    /// The alerts whose glow runs: the last alert of each target that the owner did not open.
    active: HashMap<AlertTarget, Alert>,
    /// The newest alert first.
    log: VecDeque<Alert>,
    unseen: usize,
    /// The alert that the owner just opened, for the fade of the tint. The id is the alert id.
    fades: HashMap<AlertTarget, (WorkState, u64)>,
    /// The pane that the microphone or the conversation listens to.
    listening: Option<(Option<String>, String)>,
    /// The role whose speech the pill plays, and the terminal that asked for it.
    speaking: Option<Speech>,
}

/// One speech of the pill, as far as the events tell it.
#[derive(Debug, Clone, PartialEq)]
struct Speech {
    host: Option<String>,
    terminal: String,
    role: String,
    /// The guessed end: no event tells it.
    until: std::time::Instant,
}

/// What the core knows about a pane: the center cannot read it.
#[derive(Debug, Clone, Default)]
pub struct Name {
    pub title: String,
    pub product: Option<String>,
}

/// A new alert, for the views that show it: the toast, the tone and the system notice.
#[derive(Debug, Clone)]
pub struct Raised(pub Alert);

impl EventEmitter<Raised> for AlertCenter {}

impl AlertCenter {
    fn state(&mut self, host: Option<&str>) -> &mut AlertState {
        self.states
            .entry(host.map(str::to_string))
            .or_insert_with(|| AlertState::new(host.map(str::to_string)))
    }

    /// Takes one event of the core of `host`.
    pub fn take(
        &mut self,
        host: Option<&str>,
        n: &Notification,
        names: &dyn Fn(&str) -> Name,
        cx: &mut Context<Self>,
    ) {
        let alerts = apply(self.state(host), n);
        self.finish(host, alerts, names, cx);
    }

    pub fn take_stories(
        &mut self,
        host: Option<&str>,
        product: &str,
        items: &[StorySessionItem],
        cx: &mut Context<Self>,
    ) {
        let alerts = self.state(host).stories(product, items);
        self.finish(host, alerts, &|_| Name::default(), cx);
    }

    pub fn take_desk(
        &mut self,
        host: Option<&str>,
        product: &str,
        pending: &[PendingDecision],
        cx: &mut Context<Self>,
    ) {
        let alerts = self.state(host).desk(product, pending);
        self.finish(host, alerts, &|_| Name::default(), cx);
    }

    /// A terminal bell in pane `id`.
    pub fn bell(
        &mut self,
        host: Option<&str>,
        id: &str,
        names: &dyn Fn(&str) -> Name,
        cx: &mut Context<Self>,
    ) {
        let alert = self.state(host).bell(id);
        self.finish(host, vec![alert], names, cx);
    }

    fn finish(
        &mut self,
        host: Option<&str>,
        alerts: Vec<Alert>,
        names: &dyn Fn(&str) -> Name,
        cx: &mut Context<Self>,
    ) {
        let ended = self.state(host).take_ended();
        let mut changed = false;
        for target in ended {
            changed |= self.active.remove(&target).is_some();
        }
        for mut alert in alerts {
            let silent = alert.state == WorkState::Done;
            if let AlertTarget::Pane { id, .. } = &alert.target {
                let name = names(id);
                alert.label = name.title;
                alert.product = name.product;
            }
            if !silent {
                self.active.insert(alert.target.clone(), alert.clone());
                self.log.push_front(alert.clone());
                self.log.truncate(LOG_MAX);
                self.unseen += 1;
                changed = true;
            }
            cx.emit(Raised(alert));
        }
        if changed {
            cx.notify();
        }
    }

    /// The state that makes `target` glow because of an alert, else none.
    pub fn glow(&self, target: &AlertTarget) -> Option<WorkState> {
        self.active.get(target).map(|a| a.state)
    }

    /// [`glow`](Self::glow) and the id of the alert. A new alert has a new id, so its glow
    /// pulses again.
    pub fn glow_id(&self, target: &AlertTarget) -> Option<(WorkState, u64)> {
        self.active.get(target).map(|a| (a.state, a.id))
    }

    /// The alert that the owner opened last on `target`, while its tint fades.
    pub fn fade(&self, target: &AlertTarget) -> Option<(WorkState, u64)> {
        self.fades.get(target).copied()
    }

    /// The owner opened or focused `target`: the glow of its alert ends and its tint fades.
    pub fn opened(&mut self, target: &AlertTarget, cx: &mut Context<Self>) {
        if let Some(alert) = self.active.get(target) {
            self.fades.insert(target.clone(), (alert.state, alert.id));
            // The glow of Blocked ends when the program state changes, not when the owner looks.
            if alert.bell || alert.state != WorkState::Blocked {
                self.active.remove(target);
            }
            cx.notify();
        }
    }

    /// True while the microphone or the conversation listens to pane `id`.
    pub fn is_listening(&self, host: Option<&str>, id: &str) -> bool {
        self.listening
            .as_ref()
            .is_some_and(|(h, p)| h.as_deref() == host && p == id)
    }

    /// True while the pill speaks with the voice of `role` for the core of `host`.
    pub fn is_speaking(&self, host: Option<&str>, role: &str) -> bool {
        self.speaking.as_ref().is_some_and(|s| {
            s.host.as_deref() == host && s.role == role && std::time::Instant::now() < s.until
        })
    }

    /// Reads `voice.speak` and `voice.stop`. The speech ends at `voice.stop`, or after the
    /// guess of the chime, and the center then repaints once.
    fn hear_speech(&mut self, host: Option<&str>, n: &Notification, cx: &mut Context<Self>) {
        let text = |k: &str| n.params.get(k).and_then(|v| v.as_str()).map(str::to_string);
        match n.method.as_str() {
            "voice.speak" => {
                let (Some(terminal), Some(role)) = (text("terminal"), text("role")) else {
                    return;
                };
                let wait = crate::chime::speech_guess(n);
                let until = std::time::Instant::now() + wait;
                self.speaking = Some(Speech {
                    host: host.map(str::to_string),
                    terminal,
                    role,
                    until,
                });
                cx.spawn(async move |this, cx| {
                    cx.background_executor().timer(wait).await;
                    let _ = this.update(cx, |c, cx| {
                        if c.speaking.as_ref().is_some_and(|s| s.until == until) {
                            c.speaking = None;
                            cx.notify();
                        }
                    });
                })
                .detach();
                cx.notify();
            }
            "voice.stop" => {
                let same = self.speaking.as_ref().is_some_and(|s| {
                    s.host.as_deref() == host && Some(&s.terminal) == text("terminal").as_ref()
                });
                if same {
                    self.speaking = None;
                    cx.notify();
                }
            }
            _ => {}
        }
    }

    /// Reads the events that open or close the microphone. `focused` is the pane that the
    /// owner uses, which `voice.mic` does not name.
    pub fn hear(
        &mut self,
        host: Option<&str>,
        n: &Notification,
        focused: Option<&str>,
        cx: &mut Context<Self>,
    ) {
        self.hear_speech(host, n, cx);
        let now = match n.method.as_str() {
            "voice.mic" => match n.params.get("open").and_then(|o| o.as_bool()) {
                Some(true) => focused.map(str::to_string),
                _ => None,
            },
            "converse.state" => match n.params.get("state").and_then(|s| s.as_str()) {
                Some("started") => n
                    .params
                    .get("terminal")
                    .and_then(|t| t.as_str())
                    .map(str::to_string),
                Some("stopped") => None,
                _ => return,
            },
            _ => return,
        };
        let next = now.map(|id| (host.map(str::to_string), id));
        if self.listening != next {
            self.listening = next;
            cx.notify();
        }
    }

    /// The owner focused pane `id`: the glow of its alert ends.
    pub fn focused(&mut self, host: Option<&str>, id: &str, cx: &mut Context<Self>) {
        let target = AlertTarget::Pane {
            host: host.map(str::to_string),
            id: id.to_string(),
        };
        self.opened(&target, cx);
    }

    pub fn log(&self) -> impl Iterator<Item = &Alert> {
        self.log.iter()
    }

    pub fn unseen(&self) -> usize {
        self.unseen
    }

    pub fn mark_seen(&mut self, cx: &mut Context<Self>) {
        if self.unseen != 0 {
            self.unseen = 0;
            cx.notify();
        }
    }
}

/// The center of this run.
#[derive(Clone)]
pub struct Alerts(pub Entity<AlertCenter>);

impl Global for Alerts {}

impl Alerts {
    pub fn install(cx: &mut App) -> Entity<AlertCenter> {
        let center = cx.new(|_| AlertCenter::default());
        cx.set_global(Alerts(center.clone()));
        center
    }

    pub fn try_get(cx: &App) -> Option<Entity<AlertCenter>> {
        cx.try_global::<Alerts>().map(|a| a.0.clone())
    }

    /// The glow of an alert on `target`.
    pub fn glow(cx: &App, target: &AlertTarget) -> Option<WorkState> {
        Self::try_get(cx).and_then(|c| c.read(cx).glow(target))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use agentsi_ui::components::state_icon;
    use gpui_kit::assets::IconNamed as _;
    use serde_json::{Value, json};

    const REPLAY: &str = include_str!("../tests/fixtures/alerts-replay.jsonl");

    fn item(v: &Value) -> StorySessionItem {
        let s = |k: &str| v[k].as_str().unwrap_or_default().to_string();
        StorySessionItem {
            id: s("id"),
            title: s("title"),
            state: s("state"),
            owner_role: String::new(),
        }
    }

    fn note(method: &str, params: Value) -> Notification {
        serde_json::from_value(json!({"jsonrpc": "2.0", "method": method, "params": params}))
            .unwrap()
    }

    #[test]
    fn the_replay_gives_the_alerts_of_the_state_map() {
        let mut st = AlertState::new(None);
        for (n, line) in REPLAY.lines().enumerate() {
            let step: Value = serde_json::from_str(line).unwrap();
            let got = if let Some(s) = step.get("stories") {
                let items: Vec<_> = s["items"].as_array().unwrap().iter().map(item).collect();
                st.stories(s["product"].as_str().unwrap(), &items)
            } else if let Some(id) = step.get("bell").and_then(Value::as_str) {
                vec![st.bell(id)]
            } else {
                let m = &step["notify"];
                apply(
                    &mut st,
                    &note(m["method"].as_str().unwrap(), m["params"].clone()),
                )
            };
            let got: Vec<Value> = got
                .iter()
                .map(|a| {
                    json!({
                        "state": format!("{:?}", a.state),
                        "icon": state_icon(a.state).unwrap().path().to_string(),
                        "line": a.line(),
                    })
                })
                .collect();
            assert_eq!(Value::Array(got), step["alerts"], "line {}: {line}", n + 1);
        }
    }

    #[test]
    fn a_check_with_failures_moves_the_meeting_to_repairing_and_makes_no_alert() {
        let mut st = AlertState::new(None);
        let n = note(
            "meeting.progress",
            json!({"meeting": "m", "product": "p", "stage": "check", "phase": "end",
                   "note": "3 failures: a; b; c"}),
        );
        assert!(apply(&mut st, &n).is_empty());
        let target = AlertTarget::Meeting {
            host: None,
            product: "p".into(),
        };
        assert_eq!(st.state_of(&target), Some(WorkState::Repairing));
        assert!(!check_failed("0 failures: none"));
        assert!(!check_failed("all checks passed"));
    }

    #[test]
    fn the_first_story_list_and_the_first_desk_read_make_no_alert() {
        let mut st = AlertState::new(None);
        let ready = StorySessionItem {
            id: "S1".into(),
            title: "A".into(),
            state: "ready".into(),
            owner_role: "dev".into(),
        };
        assert!(st.stories("p", &[ready]).is_empty());
        let decision = |id| PendingDecision {
            id,
            kind: "minutes".into(),
            minutes_id: None,
            reflection_id: None,
            text: format!("Pick {id}"),
            why: None,
            action: None,
            priorities: Vec::new(),
        };
        assert!(st.desk("p", &[decision(1)]).is_empty());
        let made = st.desk("p", &[decision(1), decision(2)]);
        assert_eq!(made.len(), 1);
        assert_eq!(made[0].line(), "Review: Pick 2");
        assert!(st.desk("p", &[decision(1), decision(2)]).is_empty());
        assert!(st.desk("p", &[]).is_empty());
    }

    #[test]
    fn a_blocked_pane_that_works_again_ends_its_alert() {
        let mut st = AlertState::new(None);
        let blocked = note(
            "terminal.state",
            json!({"id": "t1", "state": "blocked", "kind": "question"}),
        );
        assert_eq!(apply(&mut st, &blocked).len(), 1);
        assert!(st.take_ended().is_empty());
        let working = note("terminal.state", json!({"id": "t1", "state": "working"}));
        apply(&mut st, &working);
        assert_eq!(
            st.take_ended(),
            [AlertTarget::Pane {
                host: None,
                id: "t1".into()
            }]
        );
    }

    #[test]
    fn an_unknown_kind_of_blocked_is_not_an_alert() {
        let mut st = AlertState::new(None);
        let n = note(
            "terminal.state",
            json!({"id": "t1", "state": "blocked", "kind": "other"}),
        );
        assert!(apply(&mut st, &n).is_empty());
    }

    #[gpui::test]
    fn a_bell_glows_until_its_pane_gets_the_focus_and_a_failed_target_until_it_opens(
        cx: &mut gpui::TestAppContext,
    ) {
        let center = cx.new(|_| AlertCenter::default());
        let pane = AlertTarget::Pane {
            host: None,
            id: "t1".into(),
        };
        let meeting = AlertTarget::Meeting {
            host: None,
            product: "p".into(),
        };
        let none = |_: &str| Name::default();
        center.update(cx, |c, cx| {
            c.bell(None, "t1", &none, cx);
            let failed = note(
                "meeting.progress",
                json!({"meeting": "m", "product": "p", "stage": "done", "phase": "end",
                       "error": "boom"}),
            );
            c.take(None, &failed, &none, cx);
        });
        center.read_with(cx, |c, _| {
            assert_eq!(c.glow(&pane), Some(WorkState::Blocked));
            assert_eq!(c.glow(&meeting), Some(WorkState::Failed));
            assert_eq!(c.unseen(), 2);
        });
        center.update(cx, |c, cx| {
            c.focused(None, "t1", cx);
            c.opened(&meeting, cx);
            c.mark_seen(cx);
        });
        center.read_with(cx, |c, _| {
            assert_eq!(
                (c.glow(&pane), c.glow(&meeting), c.unseen()),
                (None, None, 0)
            );
            assert_eq!(c.log().count(), 2, "the log keeps both");
        });
    }

    #[gpui::test]
    fn the_log_keeps_200_alerts_newest_first(cx: &mut gpui::TestAppContext) {
        let center = cx.new(|_| AlertCenter::default());
        center.update(cx, |c, cx| {
            for i in 0..LOG_MAX + 5 {
                c.bell(None, &format!("t{i}"), &|_| Name::default(), cx);
            }
        });
        center.read_with(cx, |c, _| {
            assert_eq!(c.log().count(), LOG_MAX);
            let first = c.log().next().unwrap();
            assert_eq!(
                first.target,
                AlertTarget::Pane {
                    host: None,
                    id: format!("t{}", LOG_MAX + 4)
                }
            );
        });
    }

    #[gpui::test]
    fn opening_ends_the_glow_of_a_bell_and_fades_the_tint_but_a_blocked_glow_stays(
        cx: &mut gpui::TestAppContext,
    ) {
        let center = cx.new(|_| AlertCenter::default());
        let none = |_: &str| Name::default();
        let bell = AlertTarget::Pane {
            host: None,
            id: "bell".into(),
        };
        let blocked = AlertTarget::Pane {
            host: None,
            id: "t1".into(),
        };
        center.update(cx, |c, cx| {
            c.bell(None, "bell", &none, cx);
            c.take(
                None,
                &note(
                    "terminal.state",
                    json!({"id": "t1", "state": "blocked", "kind": "auth"}),
                ),
                &none,
                cx,
            );
        });
        let (bell_id, blocked_id) = center.read_with(cx, |c, _| {
            (c.glow_id(&bell).unwrap().1, c.glow_id(&blocked).unwrap().1)
        });
        assert_ne!(bell_id, blocked_id);
        center.update(cx, |c, cx| {
            c.opened(&bell, cx);
            c.opened(&blocked, cx);
        });
        center.read_with(cx, |c, _| {
            assert_eq!(c.glow(&bell), None, "a bell glow ends");
            assert_eq!(
                c.glow(&blocked),
                Some(WorkState::Blocked),
                "Blocked ends with the state"
            );
            assert_eq!(c.fade(&bell), Some((WorkState::Blocked, bell_id)));
            assert_eq!(c.fade(&blocked), Some((WorkState::Blocked, blocked_id)));
        });
        // A second bell has a new id, so its glow pulses again.
        center.update(cx, |c, cx| c.bell(None, "bell", &none, cx));
        center.read_with(cx, |c, _| assert_ne!(c.glow_id(&bell).unwrap().1, bell_id));
    }

    #[gpui::test]
    fn the_microphone_and_the_conversation_mark_the_pane_that_listens(
        cx: &mut gpui::TestAppContext,
    ) {
        let center = cx.new(|_| AlertCenter::default());
        let hear = |cx: &mut gpui::TestAppContext, n: Notification, focused: Option<&str>| {
            center.update(cx, |c, cx| c.hear(None, &n, focused, cx));
        };
        hear(cx, note("voice.mic", json!({"open": true})), Some("t1"));
        center.read_with(cx, |c, _| assert!(c.is_listening(None, "t1")));
        hear(cx, note("voice.mic", json!({"open": false})), Some("t1"));
        center.read_with(cx, |c, _| assert!(!c.is_listening(None, "t1")));
        hear(
            cx,
            note(
                "converse.state",
                json!({"state": "started", "terminal": "t2"}),
            ),
            None,
        );
        center.read_with(cx, |c, _| {
            assert!(c.is_listening(None, "t2") && !c.is_listening(None, "t1"))
        });
        hear(
            cx,
            note(
                "converse.state",
                json!({"state": "silent_reply", "terminal": "t2"}),
            ),
            None,
        );
        center.read_with(cx, |c, _| {
            assert!(c.is_listening(None, "t2"), "a silent reply keeps listening")
        });
        hear(
            cx,
            note(
                "converse.state",
                json!({"state": "stopped", "terminal": null}),
            ),
            None,
        );
        center.read_with(cx, |c, _| assert!(!c.is_listening(None, "t2")));
        // A mic with no focused pane marks nothing.
        hear(cx, note("voice.mic", json!({"open": true})), None);
        center.read_with(cx, |c, _| assert!(!c.is_listening(None, "t2")));
    }

    #[gpui::test]
    fn the_pill_speech_marks_the_card_of_its_role_until_it_stops(cx: &mut gpui::TestAppContext) {
        let center = cx.new(|_| AlertCenter::default());
        let hear = |cx: &mut gpui::TestAppContext, n: Notification| {
            center.update(cx, |c, cx| c.hear(None, &n, None, cx));
        };
        let speak = note(
            "voice.speak",
            json!({"terminal": "t1", "role": "dev", "text": "one two three"}),
        );
        hear(cx, speak);
        center.read_with(cx, |c, _| {
            assert!(c.is_speaking(None, "dev"));
            assert!(!c.is_speaking(None, "qa"));
            assert!(!c.is_speaking(Some("host"), "dev"), "another core");
        });
        hear(cx, note("voice.stop", json!({"terminal": "t2"})));
        center.read_with(cx, |c, _| assert!(c.is_speaking(None, "dev"), "another terminal"));
        hear(cx, note("voice.stop", json!({"terminal": "t1"})));
        center.read_with(cx, |c, _| assert!(!c.is_speaking(None, "dev")));
    }

    #[test]
    fn a_pane_that_finishes_work_makes_a_done_alert_for_the_tone_only() {
        let mut st = AlertState::new(None);
        apply(
            &mut st,
            &note("terminal.state", json!({"id": "t1", "state": "working"})),
        );
        let made = apply(
            &mut st,
            &note("terminal.state", json!({"id": "t1", "state": "done"})),
        );
        assert_eq!(made.len(), 1);
        assert_eq!(made[0].state, WorkState::Done);
        assert!(!Alert::is_alertable(made[0].state));
    }

    #[gpui::test]
    fn a_done_alert_reaches_no_log_and_no_glow(cx: &mut gpui::TestAppContext) {
        let center = cx.new(|_| AlertCenter::default());
        let none = |_: &str| Name::default();
        center.update(cx, |c, cx| {
            for state in ["working", "done"] {
                c.take(
                    None,
                    &note("terminal.state", json!({"id": "t1", "state": state})),
                    &none,
                    cx,
                );
            }
        });
        center.read_with(cx, |c, _| {
            assert_eq!((c.log().count(), c.unseen()), (0, 0));
            assert_eq!(
                c.glow(&AlertTarget::Pane {
                    host: None,
                    id: "t1".into()
                }),
                None
            );
        });
    }
}
