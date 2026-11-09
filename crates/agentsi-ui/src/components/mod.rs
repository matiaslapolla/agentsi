//! The components of the design system.
//!
//! `gpui-component` gives `Tag` and `Kbd`, themed from the tokens. This module
//! re-exports them. The own components are the ones that need a 120 ms hover and press
//! transition and a forced state for the gallery (`Button`, `Card`, `ListRow`, `Switch`),
//! and two markers that the library does not have.
mod button;
mod card;
mod display;
mod switch;
mod tab_chip;
#[cfg(test)]
mod tests;
mod tree;

pub use button::{Button, ButtonSize, ButtonVariant};
pub use card::{Card, ListRow};
pub use display::{
    AttentionMarker, StatusDot, key_hint, key_hint_for, label_tag, neutral_tag, status_tag,
};
pub use gpui_kit::component::{kbd::Kbd, tag::Tag};
pub use switch::Switch;
pub use tab_chip::TabChip;
pub use tree::{GroupHeader, ItemRow, ProductRow, running_chip};

use std::rc::Rc;

use gpui_kit::base::motion::{Interpolate, Transition, transition};
use gpui_kit::{
    App, ClickEvent, Div, ElementId, Entity, FocusHandle, InteractiveElement, MouseButton, Pixels,
    Stateful, StatefulInteractiveElement, Styled, Window, div,
};

use crate::tokens::{control, line, motion};

/// A state that the gallery forces on a component, so that one screenshot shows it.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Force {
    #[default]
    None,
    Hover,
    Pressed,
    Focused,
}

/// What the pointer does to one component.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct Pointer {
    hovered: bool,
    pressed: bool,
}

impl Pointer {
    /// GPUI reports `false` while a mouse button is down and the pointer moves.
    /// Thus the end of a hover does not end a press.
    pub(crate) fn hover(&mut self, on: bool) {
        self.hovered = on;
    }
    pub(crate) fn down(&mut self) {
        self.pressed = true;
    }
    /// The button is released, inside or outside the component.
    pub(crate) fn up(&mut self) {
        self.pressed = false;
    }
    pub(crate) fn look(self) -> Look {
        if self.pressed {
            Look::Pressed
        } else if self.hovered {
            Look::Hover
        } else {
            Look::Rest
        }
    }
}

/// The visual state that a component draws.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Look {
    Rest,
    Hover,
    Pressed,
}

/// The pointer state and the focus handle of one component, kept between frames.
pub(crate) struct Interaction {
    pointer: Pointer,
    /// `None` for a component that does not take focus, for example a list row.
    focus: Option<FocusHandle>,
}

pub(crate) type ClickHandler = Rc<dyn Fn(&ClickEvent, &mut Window, &mut App)>;

/// The `Interaction` of the component `id` in the current view. A `focusable`
/// component gets a focus handle that is a tab stop.
pub(crate) fn use_interaction(
    id: &ElementId,
    focusable: bool,
    window: &mut Window,
    cx: &mut App,
) -> Entity<Interaction> {
    window.use_keyed_state(id.clone(), cx, |_, cx| Interaction {
        pointer: Pointer::default(),
        focus: focusable.then(|| cx.focus_handle().tab_stop(true)),
    })
}

/// The look and the focus ring for this frame. A forced state replaces the real one.
pub(crate) fn look(
    state: &Entity<Interaction>,
    force: Force,
    disabled: bool,
    window: &Window,
    cx: &App,
) -> (Look, bool) {
    if disabled {
        return (Look::Rest, false);
    }
    match force {
        Force::Hover => return (Look::Hover, false),
        Force::Pressed => return (Look::Pressed, false),
        Force::Focused => return (Look::Rest, true),
        Force::None => {}
    }
    let s = state.read(cx);
    let ring =
        s.focus.as_ref().is_some_and(|f| f.is_focused(window)) && window.last_input_was_keyboard();
    (s.pointer.look(), ring)
}

pub(crate) fn focus_handle(state: &Entity<Interaction>, cx: &App) -> Option<FocusHandle> {
    state.read(cx).focus.clone()
}

/// Moves a value of the component `id` to `target` in 120 ms, with ease-out.
pub(crate) fn fade<T: Interpolate + PartialEq + 'static>(
    id: &ElementId,
    channel: &'static str,
    target: T,
    window: &mut Window,
    cx: &mut App,
) -> T {
    transition(
        ElementId::from((id.clone(), channel)),
        target,
        Transition::new(motion::HOVER),
        window,
        cx,
    )
}

/// Adds the pointer listeners and the click handler. GPUI also turns Enter and Space
/// on a focused element into a click, so the component adds no key listener.
pub(crate) fn wire(
    el: Stateful<Div>,
    state: &Entity<Interaction>,
    on_click: Option<ClickHandler>,
) -> Stateful<Div> {
    let update = |f: fn(&mut Pointer)| {
        let state = state.clone();
        move |cx: &mut App| {
            state.update(cx, |s, cx| {
                f(&mut s.pointer);
                cx.notify();
            })
        }
    };
    let enter = update(|p| p.hover(true));
    let leave = update(|p| p.hover(false));
    let down = update(Pointer::down);
    let up = update(Pointer::up);
    let up_out = update(Pointer::up);
    let el = el
        .cursor_pointer()
        .on_hover(move |on, _, cx| if *on { enter(cx) } else { leave(cx) })
        .on_mouse_down(MouseButton::Left, move |_, _, cx| down(cx))
        .on_mouse_up(MouseButton::Left, move |_, _, cx| up(cx))
        .on_mouse_up_out(MouseButton::Left, move |_, _, cx| up_out(cx));
    match on_click {
        Some(f) => el.on_click(move |e, w, cx| f(e, w, cx)),
        None => el,
    }
}

/// The 2 px focus ring, outside the edge of an element with the corner radius `r`.
/// The element must be `relative`. A shadow cannot draw it, because GPUI fills a
/// shadow under a transparent element.
pub(crate) fn focus_ring(on: bool, r: Pixels) -> Option<Div> {
    on.then(|| {
        let w = control::FOCUS_RING;
        div()
            .absolute()
            .top(-w)
            .left(-w)
            .right(-w)
            .bottom(-w)
            .rounded(r + w)
            .border(w)
            .border_color(line::focus().hsla())
    })
}
