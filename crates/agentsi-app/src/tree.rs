//! The item rows of the sidebar tree: what runs under one product.
//!
//! The sources are `terminal.list` and its events (the panes), `meeting.progress` (the
//! meeting), `story.list` (the story sessions) and the reflection that this window started.
//! This module only turns them into rows. The sidebar draws the rows.

use agentsi_proto::events::ProgramState;
use agentsi_ui::tokens::label::Hue;
use agentsi_ui::tokens::{Status, WorkState};

use crate::look::LookData;
use crate::model::{CoreState, StorySessionItem, Term};
use crate::pane::{glow_of, status_of};
use crate::screens::meeting::{LiveMeeting, Meetings};

/// What an item row opens.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ItemKey {
    /// A terminal, by terminal id.
    Pane(String),
    /// The meeting of the product.
    Meeting,
    /// A story with a session, by story id.
    Story(String),
    /// The reflection of the product.
    Reflection,
}

impl ItemKey {
    /// A piece of an element id.
    pub fn slug(&self) -> String {
        match self {
            ItemKey::Pane(id) => format!("pane-{id}"),
            ItemKey::Meeting => "meeting".into(),
            ItemKey::Story(id) => format!("story-{id}"),
            ItemKey::Reflection => "reflection".into(),
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct Item {
    pub key: ItemKey,
    pub status: Status,
    /// The work state that makes the row glow.
    pub work: Option<WorkState>,
    pub title: String,
    /// The program title that a pane label replaced.
    pub tooltip: Option<String>,
    /// The word on the right: the state of a pane, the phase of a meeting.
    pub meta: String,
    pub hue: Option<Hue>,
    /// The item counts as running in the chip of a closed product.
    pub running: bool,
}

/// The longest question that a meeting row shows.
const QUESTION_MAX: usize = 60;

/// The number in a terminal id (`t12-a` gives 12), so `t2` sorts before `t10`.
fn serial(id: &str) -> (usize, &str) {
    let n = id
        .strip_prefix('t')
        .and_then(|r| r.split('-').next())
        .and_then(|n| n.parse().ok())
        .unwrap_or(usize::MAX);
    (n, id)
}

fn state_word(t: &Term) -> &'static str {
    if !t.running {
        return "exited";
    }
    if t.attention().is_some() {
        return "asks";
    }
    match t.state {
        Some(ProgramState::Idle) => "idle",
        Some(ProgramState::Working) => "working",
        Some(ProgramState::Done) => "done",
        Some(ProgramState::Blocked) => "blocked",
        Some(ProgramState::Error) => "error",
        None => "",
    }
}

fn pane(t: &Term, look: &LookData, host: Option<&str>) -> Item {
    let style = look.pane(host, &t.id);
    let program = Some(t.title.clone()).filter(|p| !p.is_empty());
    let (title, tooltip) = match style.label {
        Some(label) => (label, program),
        None => (program.unwrap_or_else(|| t.id.clone()), None),
    };
    Item {
        key: ItemKey::Pane(t.id.clone()),
        status: status_of(Some(t)),
        work: glow_of(Some(t)),
        title,
        tooltip,
        meta: state_word(t).into(),
        hue: style.hue,
        running: t.running,
    }
}

fn meeting(m: &LiveMeeting) -> Item {
    let question: String = m.question.chars().take(QUESTION_MAX).collect();
    Item {
        key: ItemKey::Meeting,
        status: Status::Info,
        work: None,
        title: if question.is_empty() {
            "Meeting".into()
        } else {
            question
        },
        tooltip: None,
        meta: m.stage.clone(),
        hue: None,
        running: true,
    }
}

fn story(s: &StorySessionItem) -> Item {
    let status = match s.state.as_str() {
        "ready" => Status::Ok,
        "failed" => Status::Danger,
        _ => Status::Info,
    };
    Item {
        key: ItemKey::Story(s.id.clone()),
        status,
        work: None,
        title: format!("{} {}", s.id, s.title),
        tooltip: None,
        meta: s.state.clone(),
        hue: None,
        running: s.running(),
    }
}

/// The item rows of `product` on the core `core`: its panes, its running meeting, its story
/// sessions and its running reflection.
pub fn items(
    core: &CoreState,
    meetings: Option<&Meetings>,
    look: &LookData,
    product: &str,
) -> Vec<Item> {
    let host = core.remote.as_deref();
    let mut panes: Vec<&Term> = core
        .terminals
        .values()
        .filter(|t| t.product.as_deref() == Some(product))
        .collect();
    panes.sort_by_key(|t| serial(&t.id));
    let mut out: Vec<Item> = panes.into_iter().map(|t| pane(t, look, host)).collect();
    if let Some(m) = meetings
        .and_then(|m| m.live(product))
        .filter(|m| m.running())
    {
        out.push(meeting(m));
    }
    if let Some(sessions) = core.sessions.get(product) {
        out.extend(sessions.iter().map(story));
    }
    if core.reflecting.contains(product) {
        out.push(Item {
            key: ItemKey::Reflection,
            status: Status::Info,
            work: None,
            title: "Reflection".into(),
            tooltip: None,
            meta: "running".into(),
            hue: None,
            running: true,
        });
    }
    out
}

/// How many of `items` run, for the chip of a closed product.
pub fn running(items: &[Item]) -> usize {
    items.iter().filter(|i| i.running).count()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::look::Style;
    use serde_json::json;

    fn term(id: &str, product: &str, title: &str) -> Term {
        Term::from_info(
            serde_json::from_value(json!({
                "id": id, "argv": ["sh"], "cwd": "/", "product": product, "pid": 1,
                "cols": 80, "rows": 24, "title": title, "state": "working",
                "running": true, "code": null,
            }))
            .unwrap(),
        )
    }

    fn core() -> CoreState {
        let (a, _b) = std::os::unix::net::UnixStream::pair().unwrap();
        CoreState::new(std::sync::Arc::new(crate::link::Link::for_tests(a)))
    }

    #[test]
    fn the_panes_of_a_product_sort_by_number_and_a_label_replaces_the_program_title() {
        let mut core = core();
        for (id, p) in [("t10-a", "demo"), ("t2-a", "demo"), ("t3-a", "other")] {
            core.terminals.insert(id.into(), term(id, p, "claude"));
        }
        let mut look = LookData::default();
        look.panes.insert(
            "t2-a".into(),
            Style {
                label: Some("build".into()),
                hue: Some(Hue::Sky),
            },
        );
        let rows = items(&core, None, &look, "demo");
        let titles: Vec<_> = rows.iter().map(|r| r.title.as_str()).collect();
        assert_eq!(titles, ["build", "claude"]);
        assert_eq!(rows[0].tooltip.as_deref(), Some("claude"));
        assert_eq!(rows[0].hue, Some(Hue::Sky));
        assert_eq!(rows[1].tooltip, None);
        assert_eq!(rows[0].meta, "working");
        assert_eq!(rows[0].key, ItemKey::Pane("t2-a".into()));
    }

    #[test]
    fn a_story_session_and_a_reflection_are_items_and_a_ready_session_does_not_run() {
        let mut core = core();
        core.sessions.insert(
            "demo".into(),
            vec![
                StorySessionItem {
                    id: "S1".into(),
                    title: "Login".into(),
                    state: "work".into(),
                    owner_role: "dev".into(),
                },
                StorySessionItem {
                    id: "S2".into(),
                    title: "Logout".into(),
                    state: "ready".into(),
                    owner_role: "dev".into(),
                },
            ],
        );
        core.reflecting.insert("demo".into());
        let rows = items(&core, None, &LookData::default(), "demo");
        assert_eq!(rows.len(), 3);
        assert_eq!(rows[0].title, "S1 Login");
        assert_eq!(rows[1].status, Status::Ok);
        assert_eq!(rows[2].key, ItemKey::Reflection);
        assert_eq!(running(&rows), 2);
        assert!(items(&core, None, &LookData::default(), "other").is_empty());
    }
}
