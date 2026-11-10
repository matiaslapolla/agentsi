//! The row of agent cards ("live agents on screen") on the meeting and desk screens.
//!
//! A card shows a state only when an event set it: the stream-json of a meeting run, the state of
//! a story session, or the `voice.speak` of the pill. The row draws no animation. A screen that
//! shows it observes the meeting feed, the core and the alert center, so it repaints when one of
//! them changes and an idle window draws no frames.

use agentsi_ui::components::AgentCard;
use agentsi_ui::tokens::{WorkState, space};
use gpui_kit::*;

use super::meeting::LiveMeeting;
use super::parts::role_label;
use crate::alerts::Alerts;
use crate::model::{CoreState, StorySessionItem};

/// The word and state of a story session. It shows the state word only.
fn session_card(s: &StorySessionItem) -> Option<(WorkState, &'static str)> {
    Some(match s.state.as_str() {
        "queued" => (WorkState::Working, "Queued"),
        "tests" => (WorkState::Working, "Tests"),
        "work" => (WorkState::Working, "Work"),
        "checks" => (WorkState::Working, "Checks"),
        "ready" => (WorkState::Review, "Ready"),
        "failed" => (WorkState::Failed, "Failed"),
        _ => return None,
    })
}

/// While the pill speaks for the role, the card shows Speaking, unless a state of a higher rank
/// (Blocked, Failed, Review or Repairing) holds. Speaking shows no activity word.
pub fn with_speech(card: (WorkState, String), speaking: bool) -> (WorkState, String) {
    if speaking && card.0.rank() > WorkState::Speaking.rank() {
        (WorkState::Speaking, String::new())
    } else {
        card
    }
}

/// The card of `role` on the desk: a meeting that runs the role, else a story session that the
/// role owns, else Off.
pub fn desk_card(
    role: &str,
    live: Option<&LiveMeeting>,
    sessions: &[StorySessionItem],
) -> (WorkState, String) {
    if let Some(card) = live.filter(|m| m.running()).and_then(|m| m.card(role)) {
        return card;
    }
    sessions
        .iter()
        .filter(|s| s.owner_role == role)
        .filter_map(session_card)
        .min_by_key(|(state, _)| state.rank())
        .map_or((WorkState::Off, "Idle".into()), |(s, w)| (s, w.into()))
}

fn card(role: &str, state: WorkState, activity: String) -> AgentCard {
    AgentCard::new(role_label(role), state).activity(activity)
}

/// The cards of the roles of the desk, in the order of the registry.
pub fn desk_cards(
    roles: &[String],
    product: &str,
    core: &Entity<CoreState>,
    cx: &App,
) -> Vec<AgentCard> {
    let state = core.read(cx);
    let live = state
        .meetings
        .as_ref()
        .and_then(|f| f.read(cx).0.live(product));
    let sessions = state.sessions.get(product).map_or(&[][..], Vec::as_slice);
    let center = Alerts::try_get(cx);
    roles
        .iter()
        .map(|role| {
            let speaking = center
                .as_ref()
                .is_some_and(|c| c.read(cx).is_speaking(state.remote.as_deref(), role));
            let (s, word) = with_speech(desk_card(role, live, sessions), speaking);
            card(role, s, word)
        })
        .collect()
}

/// The cards of the roles that ran in `run`, in the order of `roster`. A role with no run shows
/// no card, because no event set its state.
pub fn meeting_cards(
    run: &LiveMeeting,
    roster: &[String],
    host: Option<&str>,
    cx: &App,
) -> Vec<AgentCard> {
    let center = Alerts::try_get(cx);
    let mut order: Vec<&str> = roster.iter().map(String::as_str).collect();
    for p in &run.panes {
        if !order.contains(&p.role.as_str()) {
            order.push(&p.role);
        }
    }
    order
        .into_iter()
        .filter_map(|role| {
            let speaking = center
                .as_ref()
                .is_some_and(|c| c.read(cx).is_speaking(host, role));
            let (s, word) = with_speech(run.card(role)?, speaking);
            Some(card(role, s, word))
        })
        .collect()
}

/// The row: the cards wrap when the screen is narrow.
pub fn row(cards: Vec<AgentCard>) -> Div {
    div()
        .debug_selector(|| "agent-row".into())
        .flex()
        .flex_row()
        .flex_wrap()
        .flex_none()
        .gap(space(1))
        .children(cards)
}

/// Repaints `T` when the core, its meetings or the alert center change.
pub fn watch<T: 'static>(core: &Entity<CoreState>, cx: &mut Context<T>) -> Vec<Subscription> {
    let mut subs = vec![cx.observe(core, |_, _, cx| cx.notify())];
    if let Some(feed) = core.read(cx).meetings.clone() {
        subs.push(cx.observe(&feed, |_, _, cx| cx.notify()));
    }
    if let Some(center) = Alerts::try_get(cx) {
        subs.push(cx.observe(&center, |_, _, cx| cx.notify()));
    }
    subs
}

#[cfg(test)]
mod tests {
    use super::{desk_card, with_speech};
    use crate::model::StorySessionItem;
    use agentsi_ui::tokens::WorkState;

    fn session(role: &str, state: &str) -> StorySessionItem {
        StorySessionItem {
            id: "S1".into(),
            title: "T".into(),
            state: state.into(),
            owner_role: role.into(),
        }
    }

    #[test]
    fn a_role_with_no_event_is_off_and_idle() {
        assert_eq!(
            desk_card("qa", None, &[session("dev", "work")]),
            (WorkState::Off, "Idle".to_string())
        );
    }

    #[test]
    fn a_story_session_shows_its_state_word_only() {
        let sessions = [session("dev", "tests"), session("qa", "ready")];
        assert_eq!(
            desk_card("dev", None, &sessions),
            (WorkState::Working, "Tests".into())
        );
        assert_eq!(
            desk_card("qa", None, &sessions),
            (WorkState::Review, "Ready".into())
        );
        let failed = [session("dev", "work"), session("dev", "failed")];
        assert_eq!(
            desk_card("dev", None, &failed),
            (WorkState::Failed, "Failed".into())
        );
    }

    #[test]
    fn speech_wins_over_working_and_done_but_not_over_failed() {
        let speak = |s, w: &str| with_speech((s, w.to_string()), true);
        assert_eq!(
            speak(WorkState::Working, "Writing"),
            (WorkState::Speaking, String::new())
        );
        assert_eq!(
            speak(WorkState::Off, "Idle"),
            (WorkState::Speaking, String::new())
        );
        assert_eq!(speak(WorkState::Failed, "Failed").0, WorkState::Failed);
        assert_eq!(
            with_speech((WorkState::Working, "Writing".into()), false).0,
            WorkState::Working
        );
    }
}
