//! The panes in the window: their toggles, their columns, and the resize and the move of a
//! pane with the pointer.
//!
//! A drag moves a value only while it moves, and the layout file is written once, at the end of
//! the drag. The content of each pane is a cached view, so a change in one pane does not draw
//! the others.

use agentsi_ui::tokens::{accent, control, line, radius, size, space, surface, text, weight};
use gpui_kit::base::InteractiveElementExt as _;
use gpui_kit::component::resizable::{HandleEdge, resize_handle};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;

use super::Shell;
use crate::alerts::Alerts;
use crate::pane_bar::{self, OnToggle};
use crate::panes::{PaneKind, Panes, Place, Side};

/// What the pointer does to the panes now.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(super) enum Gesture {
    /// A column edge or a split moves. The end of the drag saves the layout.
    Resize,
    /// A pane moves, and drops at `to`. `None` is a drop that changes nothing.
    Move { pane: PaneKind, to: Option<Place> },
}

/// The drag of the inner edge of a column.
#[derive(Clone, Copy)]
pub(super) struct ColumnEdge(Side);

/// The drag of the edge under the open pane `n` of a column.
#[derive(Clone, Copy)]
pub(super) struct SplitEdge {
    side: Side,
    n: usize,
}

/// The drag of a pane by its header.
#[derive(Clone, Copy)]
pub(super) struct DraggedPane(PaneKind);

impl Render for ColumnEdge {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        div()
    }
}

impl Render for SplitEdge {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        div()
    }
}

impl Render for DraggedPane {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        div()
            .px(space(3))
            .py(space(1))
            .rounded(radius::md())
            .border_1()
            .border_color(line::strong())
            .bg(surface::overlay())
            .text_size(size::sm())
            .text_color(text::primary())
            .child(self.0.name())
    }
}

fn drop_line(id: String, bottom: bool, active: bool) -> Div {
    div()
        .debug_selector(move || id)
        .absolute()
        .left_0()
        .right_0()
        .when(bottom, |el| el.bottom_0())
        .when(!bottom, |el| el.top_0())
        .h(control::DROP_LINE)
        .bg(if active {
            accent::base()
        } else {
            line::strong()
        })
}

/// The test selector of the drop line of `place`.
pub(super) fn drop_selector(place: Place) -> String {
    format!(
        "drop-line:{}:{}",
        place.side.word(),
        place.before.map_or("end", PaneKind::word)
    )
}

impl Shell {
    /// Takes the new panes: the file tree reads while it shows, the log counts its alerts as
    /// seen when it opens, a hidden tree gives the focus back, and the layout saves.
    pub(super) fn set_panes(&mut self, panes: Panes, window: &mut Window, cx: &mut Context<Self>) {
        if panes == self.panes {
            return;
        }
        let before = std::mem::replace(&mut self.panes, panes);
        let files = self.panes.is_open(PaneKind::Files);
        if before.is_open(PaneKind::Files) != files {
            let tree = self.sidebar.read(cx).files.clone();
            tree.update(cx, |t, cx| t.set_shown(files, cx));
        }
        if self.panes.is_open(PaneKind::Alerts)
            && !before.is_open(PaneKind::Alerts)
            && let Some(center) = Alerts::try_get(cx)
        {
            center.update(cx, |c, cx| c.mark_seen(cx));
        }
        let hid_a_tree = [PaneKind::Products, PaneKind::Files]
            .into_iter()
            .any(|k| before.is_open(k) && !self.panes.is_open(k));
        if hid_a_tree && self.sidebar.read(cx).has_focus(window, cx) {
            self.refocus(window, cx);
        }
        self.schedule_save(cx);
        cx.notify();
    }

    pub(super) fn toggle_pane(
        &mut self,
        kind: PaneKind,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let mut panes = self.panes.clone();
        panes.toggle(kind);
        self.set_panes(panes, window, cx);
    }

    pub(super) fn toggle_all_panes(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let mut panes = self.panes.clone();
        panes.toggle_all();
        self.set_panes(panes, window, cx);
    }

    /// Opens the Files pane and puts the focus in the tree.
    pub(super) fn show_files(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let mut panes = self.panes.clone();
        panes.set_open(PaneKind::Files, true);
        self.set_panes(panes, window, cx);
        let tree = self.sidebar.read(cx).files.clone();
        tree.update(cx, |t, cx| t.take_focus(window, cx));
    }

    fn drag_column(
        &mut self,
        side: Side,
        at: Point<Pixels>,
        window: Bounds<Pixels>,
        cx: &mut Context<Self>,
    ) {
        self.gesture = Some(Gesture::Resize);
        let width = match side {
            Side::Left => at.x - window.left(),
            Side::Right => window.right() - at.x,
        };
        if self
            .panes
            .set_width(side, f32::from(width), f32::from(window.size.width))
        {
            cx.notify();
        }
    }

    fn drag_split(
        &mut self,
        edge: SplitEdge,
        at: Point<Pixels>,
        column: Bounds<Pixels>,
        cx: &mut Context<Self>,
    ) {
        self.gesture = Some(Gesture::Resize);
        let y = f32::from(at.y - column.top());
        if self
            .panes
            .drag_split(edge.side, edge.n, y, f32::from(column.size.height))
        {
            cx.notify();
        }
    }

    fn start_move(&mut self, pane: PaneKind, cx: &mut Context<Self>) {
        self.gesture = Some(Gesture::Move { pane, to: None });
        cx.notify();
    }

    /// The pointer of a pane drag is at `at` in the window `window`. Over a column the pane goes
    /// to that column, else to the column of the nearer edge of the window.
    fn drag_pane(
        &mut self,
        pane: PaneKind,
        at: Point<Pixels>,
        window: Bounds<Pixels>,
        cx: &mut Context<Self>,
    ) {
        let Some(Gesture::Move { to, .. }) = self.gesture else {
            return;
        };
        let left = self.panes.shown_width(Side::Left, window.size.width);
        let right = self.panes.shown_width(Side::Right, window.size.width);
        let side = if left > Pixels::ZERO && at.x < window.left() + left {
            Side::Left
        } else if right > Pixels::ZERO && at.x > window.right() - right {
            Side::Right
        } else if at.x < window.center().x {
            Side::Left
        } else {
            Side::Right
        };
        let top = window.top() + control::TOOLBAR_HEIGHT;
        let height = f32::from(window.size.height - control::TOOLBAR_HEIGHT);
        let place = self
            .panes
            .place_at(pane, side, f32::from(at.y - top), height);
        let new = (!self.panes.is_same_place(pane, place)).then_some(place);
        if new != to {
            self.gesture = Some(Gesture::Move { pane, to: new });
            cx.notify();
        }
    }

    /// The pointer went up: a resize saves, and a move drops its pane.
    fn end_gesture(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        match self.gesture.take() {
            None => {}
            Some(Gesture::Resize) => self.schedule_save(cx),
            Some(Gesture::Move { pane, to }) => {
                if let Some(place) = to {
                    let mut panes = self.panes.clone();
                    panes.move_to(pane, place);
                    self.set_panes(panes, window, cx);
                }
                cx.notify();
            }
        }
    }

    /// Escape during a move: the drag ends and nothing changes.
    fn cancel_move(&mut self, window: &mut Window, cx: &mut Context<Self>) -> bool {
        if !matches!(self.gesture, Some(Gesture::Move { .. })) {
            return false;
        }
        self.gesture = None;
        cx.stop_active_drag(window);
        cx.notify();
        true
    }

    /// The listeners of the gestures, on the root of the window.
    pub(super) fn pane_listeners(
        &self,
        el: Stateful<Div>,
        cx: &mut Context<Self>,
    ) -> Stateful<Div> {
        el.on_drag_move::<ColumnEdge>(cx.listener(|this, e: &DragMoveEvent<ColumnEdge>, _, cx| {
            let side = e.drag(cx).0;
            this.drag_column(side, e.event.position, e.bounds, cx)
        }))
        .on_drag_move::<DraggedPane>(cx.listener(|this, e: &DragMoveEvent<DraggedPane>, _, cx| {
            let pane = e.drag(cx).0;
            this.drag_pane(pane, e.event.position, e.bounds, cx)
        }))
        // One of the two runs for each release: the root is under the pointer or it is not.
        .capture_any_mouse_up(cx.listener(|this, _, window, cx| this.end_gesture(window, cx)))
        .on_mouse_up_out(
            MouseButton::Left,
            cx.listener(|this, _, window, cx| this.end_gesture(window, cx)),
        )
        .capture_key_down(cx.listener(|this, e: &KeyDownEvent, window, cx| {
            if e.keystroke.key == "escape" && this.cancel_move(window, cx) {
                cx.stop_propagation();
            }
        }))
    }

    pub(super) fn pane_bar(&self, cx: &mut Context<Self>) -> Div {
        let shell = cx.entity().downgrade();
        let on_toggle: OnToggle = std::rc::Rc::new(move |kind, window, cx| {
            let _ = shell.update(cx, |s, cx| s.toggle_pane(kind, window, cx));
        });
        let unseen = Alerts::try_get(cx).map_or(0, |c| c.read(cx).unseen());
        pane_bar::pane_bar(&self.panes, unseen, on_toggle, cx)
    }

    fn moving(&self) -> Option<(PaneKind, Option<Place>)> {
        match self.gesture {
            Some(Gesture::Move { pane, to }) => Some((pane, to)),
            _ => None,
        }
    }

    /// The drop line at the top of an empty column, during a move: at the edge of the dock.
    pub(super) fn empty_column_drops(&self) -> Vec<Div> {
        let Some((_, to)) = self.moving() else {
            return Vec::new();
        };
        [Side::Left, Side::Right]
            .into_iter()
            .filter(|s| self.panes.column(*s).is_empty())
            .map(|side| {
                let place = Place { side, before: None };
                let line = drop_line(drop_selector(place), false, to == Some(place));
                div()
                    .absolute()
                    .top_0()
                    .w(control::PANE_COLUMN_MIN)
                    .h(control::DROP_LINE)
                    .when(side == Side::Left, |el| el.left_0())
                    .when(side == Side::Right, |el| el.right_0())
                    .child(line)
            })
            .collect()
    }

    /// A column with its open panes, or `None` when it has none: it then takes no width.
    pub(super) fn pane_column(
        &self,
        side: Side,
        window: &Window,
        cx: &mut Context<Self>,
    ) -> Option<AnyElement> {
        let panes = self.panes.column(side);
        if panes.is_empty() {
            return None;
        }
        let width = self.panes.shown_width(side, window.viewport_size().width);
        let last = panes.len() - 1;
        let frames: Vec<AnyElement> = panes
            .iter()
            .enumerate()
            .map(|(i, &kind)| self.pane_frame(side, i, kind, i == last, cx))
            .collect();
        let edge = resize_handle::<ColumnEdge, ColumnEdge>(
            ElementId::Name(format!("pane-edge-{}", side.word()).into()),
            Axis::Horizontal,
        )
        .inside(match side {
            Side::Left => HandleEdge::Trailing,
            Side::Right => HandleEdge::Leading,
        })
        .debug_selector(move || format!("pane-edge:{}", side.word()))
        .on_drag(ColumnEdge(side), |d, _, _, cx| cx.new(|_| *d))
        .on_double_click(cx.listener(move |this, _, _, cx| {
            if this.panes.reset_width(side) {
                this.schedule_save(cx);
                cx.notify();
            }
        }));
        Some(
            div()
                .id(ElementId::Name(
                    format!("pane-column-{}", side.word()).into(),
                ))
                .debug_selector(move || format!("pane-column:{}", side.word()))
                .relative()
                .flex()
                .flex_col()
                .flex_none()
                .h_full()
                .w(width)
                .bg(surface::base())
                .map(|el| match side {
                    Side::Left => el.border_r_1(),
                    Side::Right => el.border_l_1(),
                })
                .border_color(line::border())
                .on_drag_move::<SplitEdge>(cx.listener(
                    move |this, e: &DragMoveEvent<SplitEdge>, _, cx| {
                        let edge = *e.drag(cx);
                        if edge.side == side {
                            this.drag_split(edge, e.event.position, e.bounds, cx)
                        }
                    },
                ))
                .children(frames)
                .child(edge)
                .into_any_element(),
        )
    }

    /// One open pane: a header that drags, the cached content, the drop lines of a move, and
    /// the edge to the pane under it.
    fn pane_frame(
        &self,
        side: Side,
        n: usize,
        kind: PaneKind,
        last: bool,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let content: AnyView = match kind {
            PaneKind::Products => self.sidebar.clone().into(),
            PaneKind::Files => self.sidebar.read(cx).files.clone().into(),
            PaneKind::Alerts => self.alert_log.clone().into(),
        };
        let shell = cx.entity().downgrade();
        let header = div()
            .id(ElementId::Name(
                format!("pane-header-{}", kind.word()).into(),
            ))
            .debug_selector(move || format!("pane-header:{}", kind.word()))
            .flex()
            .flex_none()
            .items_center()
            .h(control::height_sm())
            .px(space(3))
            .border_b_1()
            .border_color(line::border())
            .text_size(size::xs())
            .font_weight(weight::SEMIBOLD)
            .text_color(text::tertiary())
            .cursor_grab()
            .child(kind.name())
            .on_drag(DraggedPane(kind), move |d, _, _, cx| {
                let _ = shell.update(cx, |s, cx| s.start_move(d.0, cx));
                cx.new(|_| *d)
            });
        let mut frame = div()
            .debug_selector(move || format!("pane:{}", kind.word()))
            .relative()
            .flex()
            .flex_col()
            .w_full()
            .flex_basis(Pixels::ZERO)
            .min_h(control::PANE_MIN_HEIGHT)
            .child(header)
            .child(content.cached(StyleRefinement::default().flex_1().min_h_0().w_full()));
        frame.style().flex_grow = Some(self.panes.share(kind));
        // The line over a lower pane takes no height, so the heights follow the shares.
        if n > 0 {
            frame = frame.child(
                div()
                    .absolute()
                    .top_0()
                    .left_0()
                    .right_0()
                    .border_t_1()
                    .border_color(line::border()),
            );
        }
        if let Some((moved, to)) = self.moving() {
            let above = Place {
                side,
                before: Some(kind),
            };
            if kind != moved && !self.panes.is_same_place(moved, above) {
                frame = frame.child(drop_line(drop_selector(above), false, to == Some(above)));
            }
            let below = Place { side, before: None };
            if last && !self.panes.is_same_place(moved, below) {
                frame = frame.child(drop_line(drop_selector(below), true, to == Some(below)));
            }
        }
        if !last {
            frame = frame.child(
                resize_handle::<SplitEdge, SplitEdge>(
                    ElementId::Name(format!("pane-split-{}", kind.word()).into()),
                    Axis::Vertical,
                )
                .inside(HandleEdge::Trailing)
                .debug_selector(move || format!("pane-split:{}", kind.word()))
                .on_drag(SplitEdge { side, n }, |d, _, _, cx| cx.new(|_| *d)),
            );
        }
        frame.into_any_element()
    }
}
