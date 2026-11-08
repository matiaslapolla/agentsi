use std::rc::Rc;

use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;

use super::{
    ClickHandler, Force, Look, fade, focus_handle, focus_ring, look, use_interaction, wire,
};
use crate::tokens::{Color, accent, control, grid, radius, state, switch};

type ChangeHandler = Rc<dyn Fn(&bool, &mut Window, &mut App)>;

/// The track color of a switch in one look.
pub(crate) fn track(checked: bool, look: Look) -> Color {
    match (checked, look) {
        (true, Look::Rest) => accent::BASE,
        (true, Look::Hover) => accent::HOVER,
        (true, Look::Pressed) => accent::PRESSED,
        (false, Look::Rest) => switch::TRACK,
        (false, Look::Hover) => switch::TRACK_HOVER,
        (false, Look::Pressed) => switch::TRACK_PRESSED,
    }
}

/// An on and off switch. Hover, press and the thumb move in 120 ms.
/// Enter and Space toggle a focused switch. The owner keeps the value: `on_click`
/// gets the requested value, and the owner renders it.
#[derive(IntoElement)]
pub struct Switch {
    id: ElementId,
    checked: bool,
    disabled: bool,
    force: Force,
    label: Option<SharedString>,
    on_click: Option<ChangeHandler>,
}

impl Switch {
    pub fn new(id: impl Into<ElementId>) -> Self {
        Switch {
            id: id.into(),
            checked: false,
            disabled: false,
            force: Force::None,
            label: None,
            on_click: None,
        }
    }
    pub fn checked(mut self, checked: bool) -> Self {
        self.checked = checked;
        self
    }
    pub fn disabled(mut self, disabled: bool) -> Self {
        self.disabled = disabled;
        self
    }
    /// Shows a state without input. For the gallery.
    pub fn force(mut self, force: Force) -> Self {
        self.force = force;
        self
    }
    /// The name that a screen reader announces. The switch draws no text.
    pub fn accessibility_label(mut self, label: impl Into<SharedString>) -> Self {
        self.label = Some(label.into());
        self
    }
    pub fn on_click(mut self, f: impl Fn(&bool, &mut Window, &mut App) + 'static) -> Self {
        self.on_click = Some(Rc::new(f));
        self
    }
}

impl RenderOnce for Switch {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let state = use_interaction(&self.id, true, window, cx);
        let (look, ring) = look(&state, self.force, self.disabled, window, cx);
        let fill = fade(
            &self.id,
            "track",
            track(self.checked, look).hsla(),
            window,
            cx,
        );
        let travel = control::SWITCH_WIDTH - control::SWITCH_THUMB - control::SWITCH_INSET * 2.;
        let x = fade(
            &self.id,
            "thumb",
            if self.checked { travel } else { grid(0) },
            window,
            cx,
        );
        let el = div()
            .id(self.id.clone())
            .role(Role::Switch)
            .aria_toggled(if self.checked {
                Toggled::True
            } else {
                Toggled::False
            })
            .when_some(self.label, |el, l| el.aria_label(l))
            .when_some(
                focus_handle(&state, cx).filter(|_| !self.disabled),
                |el, f| el.track_focus(&f),
            )
            .relative()
            .flex_none()
            .w(control::SWITCH_WIDTH)
            .h(control::SWITCH_HEIGHT)
            .p(control::SWITCH_INSET)
            .rounded(radius::FULL)
            .bg(fill)
            .child(
                div()
                    .ml(x)
                    .size(control::SWITCH_THUMB)
                    .rounded(radius::FULL)
                    .bg(switch::THUMB.hsla()),
            )
            .children(focus_ring(ring, radius::FULL))
            .when(self.disabled, |el| el.opacity(state::DISABLED_ALPHA));
        if self.disabled {
            return el;
        }
        let checked = self.checked;
        let on_click = self
            .on_click
            .map(|f| -> ClickHandler { Rc::new(move |_, w, cx| f(&!checked, w, cx)) });
        wire(el, &state, on_click)
    }
}
