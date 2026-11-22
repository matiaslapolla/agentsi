use std::rc::Rc;

use gpui_kit::component::tooltip::Tooltip;
use gpui_kit::component::{Icon, IconName};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;

use super::{
    ClickHandler, Force, Look, fade, focus_handle, focus_ring, look, use_interaction, wire,
};
use crate::tokens::{Color, control, radius, size, state, surface, text, toggle, weight};

/// The fill and the icon color of a toggle in one look. An open pane is bright: primary text
/// on an accent fill. A hidden pane is muted: tertiary text with no fill at rest.
pub(crate) fn toggle_colors(open: bool, look: Look) -> (Hsla, Hsla) {
    let c = |c: Color| c.hsla();
    if open {
        let fill = match look {
            Look::Rest => toggle::open(),
            Look::Hover => toggle::open_hover(),
            Look::Pressed => state::pressed(toggle::open_hover()),
        };
        return (c(fill), c(text::primary()));
    }
    match look {
        Look::Rest => (surface::overlay().alpha(0.), c(text::tertiary())),
        Look::Hover => (c(surface::overlay()), c(text::secondary())),
        Look::Pressed => (c(state::pressed(surface::overlay())), c(text::secondary())),
    }
}

/// A toggle of the pane bar: an icon, an optional label, and the open state of its pane.
/// Hover and press move in 120 ms. The test selector is `pane-toggle:<id>:open` or
/// `pane-toggle:<id>:hidden`, and `pane-toggle-count:<id>:<n>` for the count.
#[derive(IntoElement)]
pub struct PaneToggle {
    id: SharedString,
    icon: IconName,
    label: Option<SharedString>,
    count: usize,
    open: bool,
    tooltip: Option<SharedString>,
    force: Force,
    on_click: Option<ClickHandler>,
}

impl PaneToggle {
    pub fn new(id: impl Into<SharedString>, icon: IconName, open: bool) -> Self {
        PaneToggle {
            id: id.into(),
            icon,
            label: None,
            count: 0,
            open,
            tooltip: None,
            force: Force::None,
            on_click: None,
        }
    }
    pub fn label(mut self, label: impl Into<SharedString>) -> Self {
        self.label = Some(label.into());
        self
    }
    /// A number after the label, for example the unseen alerts. 0 shows none.
    pub fn count(mut self, count: usize) -> Self {
        self.count = count;
        self
    }
    pub fn tooltip(mut self, tooltip: impl Into<SharedString>) -> Self {
        self.tooltip = Some(tooltip.into());
        self
    }
    /// Shows a state without input. For the gallery.
    pub fn force(mut self, force: Force) -> Self {
        self.force = force;
        self
    }
    pub fn on_click(mut self, f: impl Fn(&ClickEvent, &mut Window, &mut App) + 'static) -> Self {
        self.on_click = Some(Rc::new(f));
        self
    }
}

impl RenderOnce for PaneToggle {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let id = ElementId::Name(format!("pane-toggle-{}", self.id).into());
        let state = use_interaction(&id, true, window, cx);
        let (look, ring) = look(&state, self.force, false, window, cx);
        let (fill, fg) = toggle_colors(self.open, look);
        let fill = fade(&id, "fill", fill, window, cx);
        let fg = fade(&id, "fg", fg, window, cx);
        let h = control::height_sm();
        let selector = format!(
            "pane-toggle:{}:{}",
            self.id,
            if self.open { "open" } else { "hidden" }
        );
        let tooltip = self.tooltip.clone();
        let el = div()
            .id(id.clone())
            .debug_selector(|| selector)
            .when_some(focus_handle(&state, cx), |el, f| el.track_focus(&f))
            .relative()
            .flex()
            .flex_none()
            .items_center()
            .justify_center()
            .gap(control::ICON_GAP)
            .h(h)
            .min_w(h)
            .px(control::PAD_X_SM)
            .rounded(radius::md())
            .bg(fill)
            .text_size(size::sm())
            .font_weight(weight::MEDIUM)
            .text_color(fg)
            .children(focus_ring(ring, radius::md()))
            .child(Icon::new(self.icon).size(control::icon_sm()).text_color(fg))
            .when_some(self.label, |el, l| el.child(l))
            .when(self.count > 0, |el| {
                let (id, n) = (self.id.clone(), self.count);
                el.child(
                    div()
                        .debug_selector(move || format!("pane-toggle-count:{id}:{n}"))
                        .font_weight(weight::SEMIBOLD)
                        .child(n.to_string()),
                )
            })
            .when(self.open, |el| {
                el.child(
                    div()
                        .absolute()
                        .left(control::PAD_X_SM)
                        .right(control::PAD_X_SM)
                        .bottom_0()
                        .h(control::TOGGLE_LINE)
                        .rounded(control::TOGGLE_LINE)
                        .bg(toggle::line().hsla()),
                )
            })
            .when_some(tooltip, |el, t| {
                el.tooltip(move |window, cx| Tooltip::new(t.clone()).build(window, cx))
            });
        wire(el, &state, self.on_click)
    }
}

#[cfg(test)]
mod tests {
    use super::{Look, toggle_colors};
    use crate::tokens::{text, toggle};

    #[test]
    fn an_open_toggle_is_bright_and_a_hidden_one_is_muted() {
        let (fill, fg) = toggle_colors(true, Look::Rest);
        assert_eq!(fg, text::primary().hsla());
        assert_eq!(fill, toggle::open().hsla());
        let (fill, fg) = toggle_colors(false, Look::Rest);
        assert_eq!(fg, text::tertiary().hsla());
        assert_eq!(fill.a, 0., "a hidden toggle is ghost at rest");
    }
}
