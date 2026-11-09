use std::rc::Rc;

use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;

use super::{
    ClickHandler, Force, Look, fade, focus_handle, focus_ring, look, use_interaction, wire,
};
use crate::tokens::{accent, control, line, radius, size, space, state, surface, text, weight};

/// A raised card. With a click handler it takes focus, Enter and Space activate it,
/// and its border moves to the accent on hover. Without one, it takes no input.
#[derive(IntoElement)]
pub struct Card {
    id: ElementId,
    children: Vec<AnyElement>,
    disabled: bool,
    force: Force,
    on_click: Option<ClickHandler>,
}

impl Card {
    pub fn new(id: impl Into<ElementId>) -> Self {
        Card {
            id: id.into(),
            children: Vec::new(),
            disabled: false,
            force: Force::None,
            on_click: None,
        }
    }
    pub fn disabled(mut self, disabled: bool) -> Self {
        self.disabled = disabled;
        self
    }
    pub fn force(mut self, force: Force) -> Self {
        self.force = force;
        self
    }
    pub fn on_click(mut self, f: impl Fn(&ClickEvent, &mut Window, &mut App) + 'static) -> Self {
        self.on_click = Some(Rc::new(f));
        self
    }
}

impl ParentElement for Card {
    fn extend(&mut self, elements: impl IntoIterator<Item = AnyElement>) {
        self.children.extend(elements);
    }
}

impl RenderOnce for Card {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let clickable = self.on_click.is_some();
        let state = use_interaction(&self.id, clickable, window, cx);
        let (look, ring) = look(&state, self.force, self.disabled, window, cx);
        let accent_border = accent::BASE.alpha(accent::HOVER_BORDER_ALPHA);
        let (border, fill) = match look {
            Look::Rest => (line::BORDER.hsla(), surface::RAISED.hsla()),
            Look::Hover => (accent_border, surface::RAISED.hsla()),
            Look::Pressed => (accent_border, state::pressed(surface::RAISED).hsla()),
        };
        let border = fade(&self.id, "border", border, window, cx);
        let fill = fade(&self.id, "fill", fill, window, cx);
        let el = div()
            .id(self.id.clone())
            .when_some(
                focus_handle(&state, cx).filter(|_| !self.disabled),
                |el, f| el.track_focus(&f),
            )
            .relative()
            .flex()
            .flex_col()
            .gap(space(3))
            .p(space(4))
            .rounded(radius::lg())
            .border_1()
            .border_color(border)
            .bg(fill)
            .text_size(size::base())
            .text_color(text::PRIMARY.hsla())
            .children(focus_ring(ring, radius::lg()))
            .when(self.disabled, |el| el.opacity(state::DISABLED_ALPHA))
            .children(self.children);
        if clickable && !self.disabled {
            wire(el, &state, self.on_click)
        } else {
            el
        }
    }
}

/// One row of a list: a title, a meta line and a trailing element. 44 px high.
/// It is cheap, because a virtual list builds some rows in each frame of a scroll.
/// A row has no focus handle: the list takes focus and sets `focused` on the row of
/// its cursor. A trailing element is at most `control::TRAILING_MAX` wide.
#[derive(IntoElement)]
pub struct ListRow {
    id: ElementId,
    title: SharedString,
    meta: Option<SharedString>,
    trailing: Option<AnyElement>,
    selected: bool,
    focused: bool,
    disabled: bool,
    force: Force,
    on_click: Option<ClickHandler>,
}

impl ListRow {
    pub fn new(id: impl Into<ElementId>, title: impl Into<SharedString>) -> Self {
        ListRow {
            id: id.into(),
            title: title.into(),
            meta: None,
            trailing: None,
            selected: false,
            focused: false,
            disabled: false,
            force: Force::None,
            on_click: None,
        }
    }
    pub fn meta(mut self, meta: impl Into<SharedString>) -> Self {
        self.meta = Some(meta.into());
        self
    }
    pub fn trailing(mut self, el: impl IntoElement) -> Self {
        self.trailing = Some(el.into_any_element());
        self
    }
    pub fn selected(mut self, selected: bool) -> Self {
        self.selected = selected;
        self
    }
    /// The keyboard cursor of the list is on this row.
    pub fn focused(mut self, focused: bool) -> Self {
        self.focused = focused;
        self
    }
    pub fn disabled(mut self, disabled: bool) -> Self {
        self.disabled = disabled;
        self
    }
    pub fn force(mut self, force: Force) -> Self {
        self.force = force;
        self
    }
    pub fn on_click(mut self, f: impl Fn(&ClickEvent, &mut Window, &mut App) + 'static) -> Self {
        self.on_click = Some(Rc::new(f));
        self
    }
}

impl RenderOnce for ListRow {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let state = use_interaction(&self.id, false, window, cx);
        let (look, ring) = look(&state, self.force, self.disabled, window, cx);
        let ring = ring || (self.focused && !self.disabled);
        let rest = if self.selected {
            accent::subtle()
        } else {
            surface::PANEL
        };
        let fill = match look {
            Look::Rest => rest,
            Look::Hover if self.selected => accent::subtle(),
            Look::Hover => surface::OVERLAY,
            // A selected row keeps its fill: the meta text fails 4.5:1 on a pressed accent fill.
            Look::Pressed if self.selected => accent::subtle(),
            Look::Pressed => state::pressed(surface::OVERLAY),
        };
        let fill = fade(&self.id, "fill", fill.hsla(), window, cx);
        let selector = format!("list-row:{}", self.id);
        let el = div()
            .id(self.id.clone())
            .debug_selector(|| selector.clone())
            .relative()
            .flex()
            .items_center()
            .justify_between()
            .gap(space(3))
            .w_full()
            .h(control::row())
            .px(space(4))
            .rounded(radius::md())
            .bg(fill)
            .children(focus_ring(ring, radius::md()))
            .when(self.disabled, |el| el.opacity(state::DISABLED_ALPHA))
            .when(self.selected, |el| {
                el.child(
                    div()
                        .absolute()
                        .left_0()
                        .top(control::SELECTION_INSET)
                        .bottom(control::SELECTION_INSET)
                        .w(control::SELECTION_BAR)
                        .rounded(radius::FULL)
                        .bg(accent::BASE.hsla()),
                )
            })
            .child(
                div()
                    .debug_selector(|| format!("{selector}:text"))
                    .flex()
                    .flex_col()
                    .flex_1()
                    .min_w_0()
                    .child(
                        div()
                            .debug_selector(|| "list-row-title".into())
                            .text_size(size::sm())
                            .font_weight(weight::MEDIUM)
                            .text_color(text::PRIMARY.hsla())
                            .truncate()
                            .child(self.title),
                    )
                    .when_some(self.meta, |el, m| {
                        el.child(
                            div()
                                .text_size(size::xs())
                                .text_color(text::TERTIARY.hsla())
                                .truncate()
                                .child(m),
                        )
                    }),
            )
            .when_some(self.trailing, |el, t| {
                el.child(
                    div()
                        .flex()
                        .flex_shrink(1.)
                        .min_w_0()
                        .max_w(control::TRAILING_MAX)
                        .overflow_hidden()
                        .child(t),
                )
            });
        if self.disabled {
            el
        } else {
            wire(el, &state, self.on_click)
        }
    }
}
