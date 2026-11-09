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
