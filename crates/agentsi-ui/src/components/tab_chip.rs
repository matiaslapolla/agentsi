//! The chip of a tab group: it sits in the tab bar before the tabs of its group, with the color of
//! the group and its name. A collapsed group shows only the chip, with the number of its tabs.

use gpui_kit::component::{Icon, IconName};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;

use super::tree::Decorate;
use super::{ClickHandler, Force, Look, fade, focus_ring, look, use_interaction, wire};
use crate::tokens::label::Hue;
use crate::tokens::{Color, control, radius, size, space, state, surface, text, weight};

/// The widest a group name grows before it is cut.
const NAME_WIDTH: Pixels = px(160.);

/// The fill of the chip for the pointer look.
fn fill_of(look: Look, hue: Option<Hue>) -> Color {
    let (rest, hover) = match hue {
        Some(h) => (h.tint(), h.tint_over(surface::OVERLAY)),
        None => (surface::RAISED, surface::OVERLAY),
    };
    match look {
        Look::Rest => rest,
        Look::Hover => hover,
        Look::Pressed => state::pressed(hover),
    }
}

/// One tab group in the tab bar. A click collapses or opens the group.
#[derive(IntoElement)]
pub struct TabChip {
    id: ElementId,
    force: Force,
    decorate: Option<Decorate>,
    name: SharedString,
    hue: Option<Hue>,
    collapsed: bool,
    count: usize,
    active: bool,
    on_click: Option<ClickHandler>,
}

impl TabChip {
    pub fn new(id: impl Into<ElementId>, name: impl Into<SharedString>) -> Self {
        TabChip {
            id: id.into(),
            force: Force::None,
            decorate: None,
            name: name.into(),
            hue: None,
            collapsed: false,
            count: 0,
            active: false,
            on_click: None,
        }
    }
    /// Changes the element of the chip before the pointer listeners go on, for example to accept
    /// a dropped tab.
    pub fn decorate(mut self, f: impl FnOnce(Stateful<Div>) -> Stateful<Div> + 'static) -> Self {
        self.decorate = Some(Box::new(f));
        self
    }
    /// The gallery forces a state, so that one screenshot shows it.
    pub fn force(mut self, force: Force) -> Self {
        self.force = force;
        self
    }
    pub fn hue(mut self, hue: Option<Hue>) -> Self {
        self.hue = hue;
        self
    }
    pub fn collapsed(mut self, collapsed: bool) -> Self {
        self.collapsed = collapsed;
        self
    }
    /// The number of tabs in the group. A collapsed chip shows it.
    pub fn count(mut self, count: usize) -> Self {
        self.count = count;
        self
    }
    /// The tab on screen is hidden in this collapsed group: the chip stands for it.
    pub fn active(mut self, active: bool) -> Self {
        self.active = active;
        self
    }
    pub fn on_click(mut self, f: impl Fn(&ClickEvent, &mut Window, &mut App) + 'static) -> Self {
        self.on_click = Some(std::rc::Rc::new(f));
        self
    }
}

impl RenderOnce for TabChip {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let state = use_interaction(&self.id, false, window, cx);
        let (look, _) = look(&state, self.force, false, window, cx);
        let fill = fade(&self.id, "fill", fill_of(look, self.hue).hsla(), window, cx);
        let ink = self.hue.map_or(text::SECONDARY, Hue::color);
        let selector = format!("tab-chip:{}", self.id);
        let el = div()
            .id(self.id.clone())
            .debug_selector(|| selector.clone())
            .relative()
            .flex()
            .flex_none()
            .items_center()
            .gap(space(1))
            .h(control::height_sm())
            .px(space(2))
            .rounded(radius::md())
            .bg(fill)
            .text_size(size::xs())
            .font_weight(weight::SEMIBOLD)
            .text_color(ink.hsla())
            .when(self.active, |el| el.border_1().border_color(ink.alpha(0.6)))
            .children(focus_ring(false, radius::md()))
            .child(
                Icon::new(if self.collapsed {
                    IconName::ChevronRight
                } else {
                    IconName::ChevronDown
                })
                .size(control::icon_sm())
                .text_color(ink.hsla()),
            )
            .child(
                div()
                    .debug_selector(|| format!("{selector}:dot"))
                    .flex_none()
                    .size(control::icon_sm() / 2.)
                    .rounded(radius::sm())
                    .bg(ink.hsla()),
            )
            .child(div().max_w(NAME_WIDTH).truncate().child(self.name))
            .when(self.collapsed, |el| {
                el.child(
                    div()
                        .debug_selector(|| format!("{selector}:count"))
                        .flex_none()
                        .child(self.count.to_string()),
                )
            });
        let el = match self.decorate {
            Some(f) => f(el),
            None => el,
        };
        wire(el, &state, self.on_click)
    }
}
