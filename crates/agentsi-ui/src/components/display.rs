//! Components that show state and take no input.
use gpui_kit::component::kbd::Kbd;
use gpui_kit::component::tag::Tag;
use gpui_kit::component::{Icon, IconName};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;

use crate::tokens::label::Hue;
use crate::tokens::{
    Color, Status, WorkState, control, grid, radius, size, space, status, surface, text, weight,
};

/// The `gpui-component` tag in a status color on its tint. Long text ends with an ellipsis.
pub fn status_tag(label: impl Into<SharedString>, s: Status) -> Tag {
    tag(label.into(), s.tint(), s.color())
}

/// The `gpui-component` tag in neutral colors.
pub fn neutral_tag(label: impl Into<SharedString>) -> Tag {
    tag(label.into(), surface::overlay(), text::secondary())
}

/// The tag of a label in a color of `tokens::label`, or neutral with none.
pub fn label_tag(label: impl Into<SharedString>, hue: Option<Hue>) -> Tag {
    match hue {
        Some(h) => tag(label.into(), h.tint(), h.color()),
        None => tag(label.into(), surface::overlay(), text::secondary()),
    }
}

fn tag(label: SharedString, fill: Color, fg: Color) -> Tag {
    Tag::custom(fill.hsla(), fg.hsla(), fill.hsla())
        .rounded(radius::sm())
        .h(control::TAG_HEIGHT)
        .px(space(2))
        .text_size(size::xs())
        .font_weight(weight::MEDIUM)
        .min_w_0()
        .truncate()
        .child(label)
}

/// The `gpui-component` key hint for `keystroke`, for example `ctrl-k`, at the meta text size.
/// Panics on a keystroke that GPUI cannot parse, because each hint is a constant.
pub fn key_hint(keystroke: &'static str) -> Kbd {
    key_hint_for(Keystroke::parse(keystroke).expect("a valid keystroke"))
}

/// The key hint for a keystroke that is known only at run time, such as a key of `keys.toml`.
pub fn key_hint_for(keystroke: Keystroke) -> Kbd {
    Kbd::new(keystroke)
        .outline()
        .text_size(size::xs())
        .px(control::KBD_PAD_X)
}

/// An 8 px dot in a status color. Always put a word or an icon next to it.
#[derive(IntoElement)]
pub struct StatusDot(pub Status);

impl RenderOnce for StatusDot {
    fn render(self, _: &mut Window, _: &mut App) -> impl IntoElement {
        div()
            .flex_none()
            .size(grid(2))
            .rounded(radius::FULL)
            .bg(self.0.color().hsla())
    }
}

/// The icon, the word and the color of a work state. The word may carry a reason, for example
/// "Blocked: Bash". It does not pulse, so an idle window draws no frames. Long text ends with an
/// ellipsis.
#[derive(IntoElement)]
pub struct StateMark {
    state: WorkState,
    reason: SharedString,
    compact: bool,
}

impl StateMark {
    pub fn new(state: WorkState) -> Self {
        StateMark {
            state,
            reason: SharedString::default(),
            compact: false,
        }
    }
    /// The text after the word of the state.
    pub fn reason(mut self, reason: impl Into<SharedString>) -> Self {
        self.reason = reason.into();
        self
    }
    /// Only the icon, for the sidebar.
    pub fn compact(mut self) -> Self {
        self.compact = true;
        self
    }
}

/// The icon of a state. Working and Off use a status dot, drawn by [`StateMark`].
pub fn state_icon(state: WorkState) -> Option<IconName> {
    Some(match state {
        WorkState::Blocked => IconName::TriangleAlert,
        WorkState::Failed => IconName::CircleX,
        WorkState::Review => IconName::Eye,
        WorkState::Repairing => IconName::RefreshCw,
        WorkState::Speaking => IconName::Play,
        WorkState::Listening => IconName::Mic,
        WorkState::Done => IconName::CircleCheck,
        WorkState::Working | WorkState::Off => return None,
    })
}

impl RenderOnce for StateMark {
    fn render(self, _: &mut Window, _: &mut App) -> impl IntoElement {
        let s = self.state;
        let word: SharedString = if self.reason.is_empty() {
            s.word().into()
        } else {
            format!("{}: {}", s.word(), self.reason).into()
        };
        div()
            .flex()
            .flex_shrink(1.)
            .min_w_0()
            .items_center()
            .gap(control::ICON_GAP)
            .h(control::MARKER_HEIGHT)
            .when(self.compact, |el| {
                el.flex_none().w(control::MARKER_HEIGHT).justify_center()
            })
            .when(!self.compact, |el| el.px(space(2)))
            .rounded(radius::FULL)
            .bg(s.tint().hsla())
            .border_1()
            .border_color(s.color().alpha(status::BORDER_ALPHA))
            .text_size(size::xs())
            .font_weight(weight::MEDIUM)
            .text_color(s.color().hsla())
            .child(match state_icon(s) {
                Some(icon) => Icon::new(icon)
                    .flex_none()
                    .size(control::icon_sm())
                    .text_color(s.color().hsla())
                    .into_any_element(),
                None => div()
                    .flex_none()
                    .size(grid(2))
                    .rounded(radius::FULL)
                    .bg(s.color().hsla())
                    .into_any_element(),
            })
            .when(!self.compact, |el| {
                el.child(div().min_w_0().truncate().child(word))
            })
    }
}

/// The widest spread of the speaking ring, at a full audio level.
const LEVEL_SPREAD: f32 = 4.;

/// The small view of one agent: the initial in a ring in the color of its work state, the name,
/// the state mark and the activity word. The state mark carries the word, so the ring never
/// shows a state alone. A speaking card with a `level` (0 to 1) grows a shadow of up to 4 px that
/// takes no layout space. The card draws no animation: its owner repaints it when the level
/// changes, so an idle window draws no frames.
#[derive(IntoElement)]
pub struct AgentCard {
    name: SharedString,
    state: WorkState,
    activity: SharedString,
    level: Option<f32>,
}

impl AgentCard {
    pub fn new(name: impl Into<SharedString>, state: WorkState) -> Self {
        AgentCard {
            name: name.into(),
            state,
            activity: SharedString::default(),
            level: None,
        }
    }
    /// The words of the work now, for example "Reading plan.md".
    pub fn activity(mut self, activity: impl Into<SharedString>) -> Self {
        self.activity = activity.into();
        self
    }
    /// The peak audio level, 0 to 1. It only matters while the state is Speaking.
    pub fn level(mut self, level: f32) -> Self {
        self.level = Some(level.clamp(0., 1.));
        self
    }
}

/// The spread in pixels of the speaking ring shadow for an audio level.
pub fn ring_spread(state: WorkState, level: Option<f32>) -> f32 {
    match (state, level) {
        (WorkState::Speaking, Some(l)) => l.clamp(0., 1.) * LEVEL_SPREAD,
        _ => 0.,
    }
}

impl RenderOnce for AgentCard {
    fn render(self, _: &mut Window, _: &mut App) -> impl IntoElement {
        let s = self.state;
        let on = s != WorkState::Off;
        let initial = self
            .name
            .chars()
            .next()
            .map(|c| c.to_uppercase().to_string())
            .unwrap_or_else(|| "?".into());
        let spread = ring_spread(s, self.level);
        let ring = div()
            .flex_none()
            .size(control::row())
            .flex()
            .items_center()
            .justify_center()
            .rounded(radius::FULL)
            .border_2()
            .border_color(s.color().hsla())
            .bg(if on { s.tint() } else { surface::raised() }.hsla())
            .text_size(size::lg())
            .font_weight(weight::SEMIBOLD)
            .text_color(if on { s.color() } else { text::secondary() }.hsla())
            .when(spread > 0., |el| {
                el.shadow(vec![BoxShadow {
                    color: s.color().alpha(status::BORDER_ALPHA),
                    offset: point(px(0.), px(0.)),
                    blur_radius: px(0.),
                    spread_radius: px(spread),
                    inset: false,
                }])
            })
            .child(initial);
        div()
            .flex()
            .flex_col()
            .items_center()
            .gap(space(1))
            .flex_none()
            .w(grid(36))
            .p(space(2))
            .child(ring)
            .child(
                div()
                    .max_w_full()
                    .truncate()
                    .text_size(size::sm())
                    .font_weight(weight::MEDIUM)
                    .text_color(if on { text::primary() } else { text::tertiary() }.hsla())
                    .child(self.name),
            )
            .child(StateMark::new(s))
            .child(
                div()
                    .max_w_full()
                    .h(size::xs())
                    .truncate()
                    .text_size(size::xs())
                    .text_color(text::tertiary().hsla())
                    .child(self.activity),
            )
    }
}
