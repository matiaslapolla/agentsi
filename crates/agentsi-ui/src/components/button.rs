use std::rc::Rc;

use gpui_kit::component::{Icon, IconName};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;

use super::{
    ClickHandler, Force, Look, fade, focus_handle, focus_ring, look, use_interaction, wire,
};
use crate::tokens::{
    Color, Status, accent, control, danger_fill_hover, danger_fill_press, line, radius, size,
    state, status, surface, text, weight,
};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum ButtonVariant {
    /// The one main action of an area. Accent fill.
    #[default]
    Primary,
    /// A normal action. Raised fill and a border.
    Secondary,
    /// A quiet action. No fill at rest. A button with only an icon is usually ghost.
    Ghost,
    /// An action that deletes or stops. Danger tint.
    Danger,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum ButtonSize {
    Sm,
    #[default]
    Md,
}

/// The fill and the text color of a variant in one look.
pub(crate) fn colors(v: ButtonVariant, look: Look) -> (Hsla, Hsla) {
    let c = |c: Color| c.hsla();
    let (rest, hover, press, fg_rest, fg_hover) = match v {
        ButtonVariant::Primary => (
            c(accent::base()),
            c(accent::hover()),
            c(accent::pressed()),
            accent::on(),
            accent::on(),
        ),
        ButtonVariant::Secondary => (
            c(surface::raised()),
            c(surface::overlay()),
            c(state::pressed(surface::overlay())),
            text::primary(),
            text::primary(),
        ),
        ButtonVariant::Ghost => (
            surface::overlay().alpha(0.),
            c(surface::overlay()),
            c(state::pressed(surface::overlay())),
            text::secondary(),
            text::primary(),
        ),
        ButtonVariant::Danger => (
            c(Status::Danger.tint()),
            c(danger_fill_hover()),
            c(danger_fill_press()),
            status::danger(),
            status::danger(),
        ),
    };
    match look {
        Look::Rest => (rest, c(fg_rest)),
        Look::Hover => (hover, c(fg_hover)),
        Look::Pressed => (press, c(fg_hover)),
    }
}

/// A button with a label, an icon, or both. Hover and press move in 120 ms.
/// Enter and Space activate a focused button.
#[derive(IntoElement)]
pub struct Button {
    id: ElementId,
    label: Option<SharedString>,
    icon: Option<IconName>,
    variant: ButtonVariant,
    size: ButtonSize,
    disabled: bool,
    force: Force,
    on_click: Option<ClickHandler>,
}

impl Button {
    pub fn new(id: impl Into<ElementId>) -> Self {
        Button {
            id: id.into(),
            label: None,
            icon: None,
            variant: ButtonVariant::Primary,
            size: ButtonSize::Md,
            disabled: false,
            force: Force::None,
            on_click: None,
        }
    }
    pub fn label(mut self, label: impl Into<SharedString>) -> Self {
        self.label = Some(label.into());
        self
    }
    pub fn icon(mut self, icon: IconName) -> Self {
        self.icon = Some(icon);
        self
    }
    pub fn variant(mut self, variant: ButtonVariant) -> Self {
        self.variant = variant;
        self
    }
    pub fn primary(self) -> Self {
        self.variant(ButtonVariant::Primary)
    }
    pub fn secondary(self) -> Self {
        self.variant(ButtonVariant::Secondary)
    }
    pub fn ghost(self) -> Self {
        self.variant(ButtonVariant::Ghost)
    }
    pub fn danger(self) -> Self {
        self.variant(ButtonVariant::Danger)
    }
    pub fn size(mut self, size: ButtonSize) -> Self {
        self.size = size;
        self
    }
    pub fn small(self) -> Self {
        self.size(ButtonSize::Sm)
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
    pub fn on_click(mut self, f: impl Fn(&ClickEvent, &mut Window, &mut App) + 'static) -> Self {
        self.on_click = Some(Rc::new(f));
        self
    }
}

impl RenderOnce for Button {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let state = use_interaction(&self.id, true, window, cx);
        let (look, ring) = look(&state, self.force, self.disabled, window, cx);
        let (fill, fg) = colors(self.variant, look);
        let fill = fade(&self.id, "fill", fill, window, cx);
        let fg = fade(&self.id, "fg", fg, window, cx);
        let (h, pad_x, text_size, icon_size) = match self.size {
            ButtonSize::Sm => (
                control::height_sm(),
                control::PAD_X_SM,
                size::sm(),
                control::icon_sm(),
            ),
            ButtonSize::Md => (
                control::height_md(),
                control::PAD_X_MD,
                size::base(),
                control::icon_md(),
            ),
        };
        let icon_only = self.label.is_none();
        let el = div()
            .id(self.id.clone())
            .debug_selector(|| format!("button:{}", self.id))
            .when_some(
                focus_handle(&state, cx).filter(|_| !self.disabled),
                |el, f| el.track_focus(&f),
            )
            .relative()
            .flex()
            .flex_none()
            .items_center()
            .justify_center()
            .gap(control::ICON_GAP)
            .h(h)
            .when(icon_only, |el| el.w(h))
            .when(!icon_only, |el| el.px(pad_x))
            .rounded(radius::md())
            .bg(fill)
            .when(self.variant == ButtonVariant::Secondary, |el| {
                el.border_1().border_color(line::strong().hsla())
            })
            .text_size(text_size)
            .font_weight(weight::MEDIUM)
            .text_color(fg)
            .children(focus_ring(ring, radius::md()))
            .when(self.disabled, |el| el.opacity(state::DISABLED_ALPHA))
            .when_some(self.icon, |el, icon| {
                el.child(Icon::new(icon).size(icon_size).text_color(fg))
            })
            .when_some(self.label, |el, l| el.child(l));
        if self.disabled {
            el
        } else {
            wire(el, &state, self.on_click)
        }
    }
}
