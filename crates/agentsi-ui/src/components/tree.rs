//! The rows of the sidebar tree: a product row with an arrow, an item row under it, and the
//! header of a product group.
//!
//! Like `ListRow`, these rows are cheap and take no focus handle: the sidebar takes focus as
//! one element and sets `focused` on the row of its cursor.

use gpui_kit::component::tooltip::Tooltip;
use gpui_kit::component::{Icon, IconName};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;

use super::{
    ClickHandler, Force, Look, fade, focus_ring, look, neutral_tag, use_interaction, wire,
};
use crate::components::label_tag;
use crate::tokens::label::Hue;
use crate::tokens::{accent, control, radius, size, space, state, surface, text, weight};

/// A change to the element of a row, for example drag and drop. The sidebar knows its
/// products, so it adds the handlers here and the component stays free of them.
pub type Decorate = Box<dyn FnOnce(Stateful<Div>) -> Stateful<Div>>;

/// The box of the arrow of a product row. An item row indents by the same width.
fn arrow_box() -> Pixels {
    control::icon_sm() + space(2)
}

/// The fill of a row for the pointer look.
fn fill_of(look: Look, selected: bool) -> crate::tokens::Color {
    let rest = if selected {
        accent::SUBTLE
    } else {
        surface::PANEL
    };
    match look {
        Look::Rest => rest,
        Look::Hover if selected => accent::SUBTLE,
        Look::Hover => surface::OVERLAY,
        Look::Pressed if selected => accent::SUBTLE,
        Look::Pressed => state::pressed(surface::OVERLAY),
    }
}

/// The 3 px bar of a color at the left edge of a row.
fn color_bar(h: Hue, selector: String) -> Div {
    div()
        .debug_selector(|| selector)
        .absolute()
        .left_0()
        .top(control::SELECTION_INSET / 2.)
        .bottom(control::SELECTION_INSET / 2.)
        .w(control::COLOR_BAR)
        .rounded(radius::FULL)
        .bg(h.color().hsla())
}

/// One product: an arrow, its name, its label, its folder and a trailing element. A product
/// color shows as a bar at the left edge.
#[derive(IntoElement)]
pub struct ProductRow {
    id: ElementId,
    force: Force,
    decorate: Option<Decorate>,
    title: SharedString,
    label: Option<SharedString>,
    hue: Option<Hue>,
    meta: Option<SharedString>,
    open: bool,
    trailing: Option<AnyElement>,
    selected: bool,
    focused: bool,
    on_click: Option<ClickHandler>,
    on_toggle: Option<ClickHandler>,
}

impl ProductRow {
    pub fn new(id: impl Into<ElementId>, title: impl Into<SharedString>) -> Self {
        ProductRow {
            id: id.into(),
            force: Force::None,
            decorate: None,
            title: title.into(),
            label: None,
            hue: None,
            meta: None,
            open: false,
            trailing: None,
            selected: false,
            focused: false,
            on_click: None,
            on_toggle: None,
        }
    }
    /// Changes the element of the row before the pointer listeners go on.
    pub fn decorate(mut self, f: impl FnOnce(Stateful<Div>) -> Stateful<Div> + 'static) -> Self {
        self.decorate = Some(Box::new(f));
        self
    }
    /// The gallery forces a state, so that one screenshot shows it.
    pub fn force(mut self, force: Force) -> Self {
        self.force = force;
        self
    }
    pub fn label(mut self, label: Option<SharedString>) -> Self {
        self.label = label;
        self
    }
    pub fn hue(mut self, hue: Option<Hue>) -> Self {
        self.hue = hue;
        self
    }
    pub fn meta(mut self, meta: impl Into<SharedString>) -> Self {
        self.meta = Some(meta.into());
        self
    }
    pub fn open(mut self, open: bool) -> Self {
        self.open = open;
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
    /// The keyboard cursor of the sidebar is on this row.
    pub fn focused(mut self, focused: bool) -> Self {
        self.focused = focused;
        self
    }
    pub fn on_click(mut self, f: impl Fn(&ClickEvent, &mut Window, &mut App) + 'static) -> Self {
        self.on_click = Some(std::rc::Rc::new(f));
        self
    }
    /// A click on the arrow. It does not reach `on_click`.
    pub fn on_toggle(mut self, f: impl Fn(&ClickEvent, &mut Window, &mut App) + 'static) -> Self {
        self.on_toggle = Some(std::rc::Rc::new(f));
        self
    }
}

impl RenderOnce for ProductRow {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let state = use_interaction(&self.id, false, window, cx);
        let (look, ring) = look(&state, self.force, false, window, cx);
        let ring = ring || self.focused;
        let fill = fade(
            &self.id,
            "fill",
            fill_of(look, self.selected).hsla(),
            window,
            cx,
        );
        let selector = format!("tree-row:{}", self.id);
        let arrow_id = ElementId::from((self.id.clone(), "arrow"));
        let arrow_selector = format!("{selector}:arrow");
        let on_toggle = self.on_toggle.clone();
        let arrow = div()
            .id(arrow_id)
            .debug_selector(|| arrow_selector)
            .flex()
            .flex_none()
            .items_center()
            .justify_center()
            .size(arrow_box())
            .cursor_pointer()
            .child(
                Icon::new(if self.open {
                    IconName::ChevronDown
                } else {
                    IconName::ChevronRight
                })
                .size(control::icon_sm())
                .text_color(text::TERTIARY.hsla()),
            )
            .on_click(move |e, w, cx| {
                cx.stop_propagation();
                if let Some(f) = &on_toggle {
                    f(e, w, cx);
                }
            });
        let el = div()
            .id(self.id.clone())
            .debug_selector(|| selector.clone())
            .relative()
            .flex()
            .items_center()
            .gap(space(2))
            .w_full()
            .h(control::row())
            .pl(space(2))
            .pr(space(3))
            .rounded(radius::md())
            .bg(fill)
            .children(focus_ring(ring, radius::md()))
            .when_some(self.hue, |el, h| {
                el.child(color_bar(h, format!("{selector}:bar")))
            })
            .when(self.selected && self.hue.is_none(), |el| {
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
            .child(arrow)
            .child(
                div()
                    .debug_selector(|| format!("{selector}:text"))
                    .flex()
                    .flex_col()
                    .flex_1()
                    .min_w_0()
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap(space(2))
                            .min_w_0()
                            .child(
                                div()
                                    .debug_selector(|| format!("{selector}:title"))
                                    .min_w_0()
                                    .flex_shrink(1.)
                                    .text_size(size::sm())
                                    .font_weight(weight::MEDIUM)
                                    .text_color(text::PRIMARY.hsla())
                                    .truncate()
                                    .child(self.title),
                            )
                            .when_some(self.label, |el, l| {
                                el.child(
                                    div()
                                        .debug_selector(|| format!("{selector}:label"))
                                        .flex()
                                        .flex_shrink(1.)
                                        .min_w_0()
                                        .child(label_tag(l, self.hue)),
                                )
                            }),
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
        let el = match self.decorate {
            Some(f) => f(el),
            None => el,
        };
        wire(el, &state, self.on_click)
    }
}

/// One running thing under a product: a status dot or an icon, a title and a meta on the right.
#[derive(IntoElement)]
pub struct ItemRow {
    id: ElementId,
    force: Force,
    decorate: Option<Decorate>,
    leading: Option<AnyElement>,
    title: SharedString,
    meta: Option<SharedString>,
    hue: Option<Hue>,
    tooltip: Option<SharedString>,
    selected: bool,
    focused: bool,
    on_click: Option<ClickHandler>,
}

impl ItemRow {
    pub fn new(id: impl Into<ElementId>, title: impl Into<SharedString>) -> Self {
        ItemRow {
            id: id.into(),
            force: Force::None,
            decorate: None,
            leading: None,
            title: title.into(),
            meta: None,
            hue: None,
            tooltip: None,
            selected: false,
            focused: false,
            on_click: None,
        }
    }
    /// Changes the element of the row before the pointer listeners go on.
    pub fn decorate(mut self, f: impl FnOnce(Stateful<Div>) -> Stateful<Div> + 'static) -> Self {
        self.decorate = Some(Box::new(f));
        self
    }
    /// The gallery forces a state, so that one screenshot shows it.
    pub fn force(mut self, force: Force) -> Self {
        self.force = force;
        self
    }
    /// A `StatusDot` or a small icon, before the title.
    pub fn leading(mut self, el: impl IntoElement) -> Self {
        self.leading = Some(el.into_any_element());
        self
    }
    pub fn meta(mut self, meta: impl Into<SharedString>) -> Self {
        self.meta = Some(meta.into());
        self
    }
    pub fn hue(mut self, hue: Option<Hue>) -> Self {
        self.hue = hue;
        self
    }
    /// The text of a hover, for example the program title under a label.
    pub fn tooltip(mut self, tooltip: Option<SharedString>) -> Self {
        self.tooltip = tooltip;
        self
    }
    pub fn selected(mut self, selected: bool) -> Self {
        self.selected = selected;
        self
    }
    pub fn focused(mut self, focused: bool) -> Self {
        self.focused = focused;
        self
    }
    pub fn on_click(mut self, f: impl Fn(&ClickEvent, &mut Window, &mut App) + 'static) -> Self {
        self.on_click = Some(std::rc::Rc::new(f));
        self
    }
}

impl RenderOnce for ItemRow {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let state = use_interaction(&self.id, false, window, cx);
        let (look, ring) = look(&state, self.force, false, window, cx);
        let ring = ring || self.focused;
        let fill = fade(
            &self.id,
            "fill",
            fill_of(look, self.selected).hsla(),
            window,
            cx,
        );
        let selector = format!("tree-row:{}", self.id);
        let tooltip = self.tooltip.clone();
        let el = div()
            .id(self.id.clone())
            .debug_selector(|| selector.clone())
            .relative()
            .flex()
            .items_center()
            .gap(space(2))
            .w_full()
            .py(control::row_pad())
            .pl(space(2) + arrow_box() + space(2))
            .pr(space(3))
            .rounded(radius::md())
            .bg(fill)
            .children(focus_ring(ring, radius::md()))
            .when_some(self.hue, |el, h| {
                el.child(color_bar(h, format!("{selector}:bar")))
            })
            .when_some(self.leading, |el, l| el.child(div().flex_none().child(l)))
            .child(
                div()
                    .debug_selector(|| format!("{selector}:title"))
                    .flex_1()
                    .min_w_0()
                    .text_size(size::sm())
                    .text_color(text::PRIMARY.hsla())
                    .truncate()
                    .child(self.title),
            )
            .when_some(self.meta, |el, m| {
                el.child(
                    div()
                        .flex_shrink(1.)
                        .min_w_0()
                        .max_w(control::TRAILING_MAX)
                        .text_size(size::xs())
                        .text_color(text::TERTIARY.hsla())
                        .truncate()
                        .child(m),
                )
            })
            .when_some(tooltip, |el, t| {
                el.tooltip(move |window, cx| Tooltip::new(t.clone()).build(window, cx))
            });
        let el = match self.decorate {
            Some(f) => f(el),
            None => el,
        };
        wire(el, &state, self.on_click)
    }
}

/// The header of a product group: an arrow, a color square, the name and the number of products.
#[derive(IntoElement)]
pub struct GroupHeader {
    id: ElementId,
    force: Force,
    decorate: Option<Decorate>,
    name: SharedString,
    hue: Option<Hue>,
    collapsed: bool,
    count: usize,
    focused: bool,
    on_click: Option<ClickHandler>,
}

impl GroupHeader {
    pub fn new(id: impl Into<ElementId>, name: impl Into<SharedString>) -> Self {
        GroupHeader {
            id: id.into(),
            force: Force::None,
            decorate: None,
            name: name.into(),
            hue: None,
            collapsed: false,
            count: 0,
            focused: false,
            on_click: None,
        }
    }
    /// Changes the element of the row before the pointer listeners go on.
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
    pub fn count(mut self, count: usize) -> Self {
        self.count = count;
        self
    }
    pub fn focused(mut self, focused: bool) -> Self {
        self.focused = focused;
        self
    }
    /// A click on the header collapses or opens the group.
    pub fn on_click(mut self, f: impl Fn(&ClickEvent, &mut Window, &mut App) + 'static) -> Self {
        self.on_click = Some(std::rc::Rc::new(f));
        self
    }
}

impl RenderOnce for GroupHeader {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let state = use_interaction(&self.id, false, window, cx);
        let (look, ring) = look(&state, self.force, false, window, cx);
        let ring = ring || self.focused;
        let fill = fade(&self.id, "fill", fill_of(look, false).hsla(), window, cx);
        let selector = format!("tree-row:{}", self.id);
        let el = div()
            .id(self.id.clone())
            .debug_selector(|| selector.clone())
            .relative()
            .flex()
            .items_center()
            .gap(space(2))
            .w_full()
            .py(control::row_pad())
            .pl(space(2))
            .pr(space(3))
            .rounded(radius::md())
            .bg(fill)
            .children(focus_ring(ring, radius::md()))
            .child(
                div()
                    .flex()
                    .flex_none()
                    .items_center()
                    .justify_center()
                    .size(arrow_box())
                    .child(
                        Icon::new(if self.collapsed {
                            IconName::ChevronRight
                        } else {
                            IconName::ChevronDown
                        })
                        .size(control::icon_sm())
                        .text_color(text::TERTIARY.hsla()),
                    ),
            )
            .child(
                div()
                    .debug_selector(|| format!("{selector}:square"))
                    .flex_none()
                    .size(control::icon_sm() / 2.)
                    .rounded(radius::sm())
                    .bg(self.hue.map_or(text::TERTIARY.hsla(), |h| h.color().hsla())),
            )
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .text_size(size::xs())
                    .font_weight(weight::SEMIBOLD)
                    .text_color(text::SECONDARY.hsla())
                    .truncate()
                    .child(self.name),
            )
            .child(
                div()
                    .flex_none()
                    .text_size(size::xs())
                    .text_color(text::TERTIARY.hsla())
                    .child(self.count.to_string()),
            );
        let el = match self.decorate {
            Some(f) => f(el),
            None => el,
        };
        wire(el, &state, self.on_click)
    }
}

/// The chip of a closed product: how many of its items run.
pub fn running_chip(count: usize) -> Tag {
    neutral_tag(format!("{count} running"))
}

use gpui_kit::component::tag::Tag;
