//! The screen columns: the side columns inside a screen whose width the owner drags.
//!
//! A drag works like the edge of a pane column: the same handle, the same limits and a double
//! click that gives the first width again. One width is kept for each screen column, in the
//! global [`ScreenColumns`], and `layout.json` saves it.

use std::collections::BTreeMap;

use agentsi_ui::tokens::control;
use gpui_kit::base::InteractiveElementExt as _;
use gpui_kit::component::resizable::{HandleEdge, resize_handle};
use gpui_kit::*;
use serde::Deserialize as _;

use crate::panes::clamp_width;

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum ScreenColumn {
    /// The section list of Settings.
    SettingsNav,
    /// The list of the Git screen.
    GitList,
    /// The commit list and the files of a commit, in the Git history view.
    GitHistory,
    /// The desk conversation of the front desk.
    DeskConversation,
}

impl ScreenColumn {
    pub const ALL: [ScreenColumn; 4] = [
        ScreenColumn::SettingsNav,
        ScreenColumn::GitList,
        ScreenColumn::GitHistory,
        ScreenColumn::DeskConversation,
    ];

    /// The key in the field `columns` of `layout.json`.
    pub fn key(self) -> &'static str {
        match self {
            ScreenColumn::SettingsNav => "settings-nav",
            ScreenColumn::GitList => "git-list",
            ScreenColumn::GitHistory => "git-history",
            ScreenColumn::DeskConversation => "desk-conversation",
        }
    }

    fn from_key(key: &str) -> Option<ScreenColumn> {
        ScreenColumn::ALL.into_iter().find(|c| c.key() == key)
    }

    /// The width before a drag, and the width that a double click on the edge gives.
    pub fn first_width(self) -> Pixels {
        match self {
            ScreenColumn::SettingsNav => control::SETTINGS_NAV_WIDTH,
            ScreenColumn::GitList | ScreenColumn::GitHistory => control::SIDEBAR_WIDTH,
            ScreenColumn::DeskConversation => control::DESK_CONVERSATION_WIDTH,
        }
    }

    /// The edge with the handle. A column at the left of its row has it on the right.
    fn edge(self) -> HandleEdge {
        match self {
            ScreenColumn::DeskConversation => HandleEdge::Leading,
            _ => HandleEdge::Trailing,
        }
    }
}

/// The widths that the owner gave the screen columns. A column that is not here has its first
/// width.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct ScreenColumns {
    widths: BTreeMap<ScreenColumn, f32>,
}

impl Global for ScreenColumns {}

impl ScreenColumns {
    /// The widths of `layout.json`. A key that is not a screen column, or a width that is not
    /// a positive number, is dropped.
    pub fn load(saved: &BTreeMap<String, f32>) -> ScreenColumns {
        let widths = saved
            .iter()
            .filter(|(_, w)| w.is_finite() && **w > 0.)
            .filter_map(|(k, w)| ScreenColumn::from_key(k).map(|c| (c, *w)))
            .collect();
        ScreenColumns { widths }
    }

    pub fn save(&self) -> BTreeMap<String, f32> {
        self.widths
            .iter()
            .map(|(c, w)| (c.key().to_string(), *w))
            .collect()
    }

    /// The width of the column, at least `PANE_COLUMN_MIN`. The element keeps it to
    /// `PANE_COLUMN_MAX` of its row.
    pub fn width(&self, column: ScreenColumn) -> Pixels {
        let w = self
            .widths
            .get(&column)
            .copied()
            .unwrap_or(f32::from(column.first_width()));
        Pixels::from(w.max(f32::from(control::PANE_COLUMN_MIN)))
    }

    /// Sets the width from the width of the row. Returns false when it stays. A row too narrow
    /// for the minimum stores the minimum, and the element shows the half of the row.
    pub fn set(&mut self, column: ScreenColumn, width: f32, row: f32) -> bool {
        let width = clamp_width(width, row).max(f32::from(control::PANE_COLUMN_MIN));
        if f32::from(self.width(column)) == width {
            return false;
        }
        self.widths.insert(column, width);
        true
    }

    /// The double click on the edge: back to the first width. Returns false when the column
    /// already has its first width.
    pub fn reset(&mut self, column: ScreenColumn) -> bool {
        self.widths.remove(&column).is_some()
    }
}

/// The field `columns` of `layout.json`. A value that is not a number is dropped, and a field
/// that is not a map reads as empty. An edit by hand costs only the widths, never the layout.
pub fn read_saved<'de, D>(d: D) -> Result<BTreeMap<String, f32>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    let value = Option::<serde_json::Value>::deserialize(d)?;
    let Some(serde_json::Value::Object(map)) = value else {
        return Ok(BTreeMap::new());
    };
    Ok(map
        .into_iter()
        .filter_map(|(k, v)| v.as_f64().map(|w| (k, w as f32)))
        .collect())
}

/// The drag of the edge of a screen column of the entity `owner`.
#[derive(Clone, Copy)]
pub struct ColumnDrag {
    column: ScreenColumn,
    owner: EntityId,
}

impl Render for ColumnDrag {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        div()
    }
}

fn change(cx: &mut App, f: impl FnOnce(&mut ScreenColumns) -> bool) {
    let mut next = cx
        .try_global::<ScreenColumns>()
        .cloned()
        .unwrap_or_default();
    if f(&mut next) {
        cx.set_global(next);
    }
}

/// The shown width of `column`.
pub fn width(column: ScreenColumn, cx: &App) -> Pixels {
    cx.try_global::<ScreenColumns>().map_or_else(
        || ScreenColumns::default().width(column),
        |c| c.width(column),
    )
}

/// `content` in a column of the width of `column`, with the handle on its edge. The row that
/// holds the column must have [`on_column_drag`], and the column must touch the row edge on
/// the side away from the handle.
pub fn screen_column<T: 'static>(
    column: ScreenColumn,
    content: impl IntoElement,
    cx: &Context<T>,
) -> Div {
    let owner = cx.entity_id();
    let key = column.key();
    let edge = resize_handle::<ColumnDrag, ColumnDrag>(
        ElementId::Name(format!("column-edge-{key}").into()),
        Axis::Horizontal,
    )
    .inside(column.edge())
    .debug_selector(move || format!("column-edge:{key}"))
    .on_drag(ColumnDrag { column, owner }, |d, _, _, cx| cx.new(|_| *d))
    .on_double_click(move |_, _, cx| change(cx, |c| c.reset(column)));
    div()
        .debug_selector(move || format!("screen-column:{key}"))
        .relative()
        .flex_none()
        .h_full()
        .w(width(column, cx))
        .max_w(relative(control::PANE_COLUMN_MAX))
        .child(content)
        .child(edge)
}

/// The listener of the drags of `column` on the row that holds it. The bounds of the row give
/// the new width.
pub fn on_column_drag<E: InteractiveElement, T: 'static>(
    row: E,
    column: ScreenColumn,
    cx: &mut Context<T>,
) -> E {
    let owner = cx.entity_id();
    row.on_drag_move::<ColumnDrag>(move |e, _, cx| {
        let drag = *e.drag(cx);
        if drag.owner != owner || drag.column != column {
            return;
        }
        let (x, row) = (e.event.position.x, e.bounds);
        let w = match column.edge() {
            HandleEdge::Trailing => x - row.left(),
            HandleEdge::Leading => row.right() - x,
        };
        let row = f32::from(row.size.width);
        change(cx, |c| c.set(column, f32::from(w), row));
    })
}

/// Draws the entity again when a width changes, also from a drag in another screen.
pub fn observe<T: 'static>(cx: &mut Context<T>) -> Subscription {
    cx.observe_global::<ScreenColumns>(|_, cx| cx.notify())
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use gpui_kit::Pixels;

    use agentsi_ui::tokens::control;

    use super::{ScreenColumn, ScreenColumns, read_saved};

    fn min() -> f32 {
        f32::from(control::PANE_COLUMN_MIN)
    }

    #[test]
    fn columns_width_starts_at_the_first_width_and_keeps_the_minimum() {
        let c = ScreenColumns::default();
        for column in ScreenColumn::ALL {
            assert_eq!(c.width(column), column.first_width());
        }
        let saved = BTreeMap::from([("git-list".to_string(), 40.)]);
        assert_eq!(
            ScreenColumns::load(&saved).width(ScreenColumn::GitList),
            Pixels::from(min())
        );
    }

    #[test]
    fn columns_set_clamps_to_the_min_and_the_half_of_the_row() {
        let mut c = ScreenColumns::default();
        let col = ScreenColumn::DeskConversation;
        assert!(c.set(col, 500., 1200.));
        assert_eq!(c.width(col), Pixels::from(500.));
        assert!(!c.set(col, 500., 1200.), "the same width changes nothing");
        assert!(c.set(col, 40., 1200.));
        assert_eq!(c.width(col), Pixels::from(min()));
        assert!(c.set(col, 1000., 1200.));
        assert_eq!(c.width(col), Pixels::from(600.));
        // A row narrower than two minimums stores the minimum once.
        assert!(c.set(col, 200., 300.));
        assert!(!c.set(col, 200., 300.), "a second move changes nothing");
        assert_eq!(c.save(), BTreeMap::from([(col.key().to_string(), min())]));
    }

    #[test]
    fn columns_reset_gives_the_first_width() {
        let mut c = ScreenColumns::default();
        let col = ScreenColumn::SettingsNav;
        assert!(!c.reset(col), "nothing to reset");
        c.set(col, 320., 1200.);
        assert!(c.reset(col));
        assert_eq!(c.width(col), col.first_width());
        assert!(c.save().is_empty());
    }

    #[test]
    fn columns_load_drops_a_bad_key_and_a_bad_width_and_save_reads_back() {
        let saved = BTreeMap::from([
            ("git-list".to_string(), 300.),
            ("git-history".to_string(), -2.),
            ("desk-conversation".to_string(), f32::NAN),
            ("nope".to_string(), 300.),
        ]);
        let c = ScreenColumns::load(&saved);
        assert_eq!(c.save(), BTreeMap::from([("git-list".to_string(), 300.)]));
        assert_eq!(ScreenColumns::load(&c.save()), c);
    }

    #[test]
    fn columns_read_saved_drops_a_value_that_is_not_a_number() {
        #[derive(serde::Deserialize)]
        struct F {
            #[serde(default, deserialize_with = "read_saved")]
            columns: BTreeMap<String, f32>,
        }
        let f: F = serde_json::from_str(r#"{"columns": {"git-list": 300, "git-history": "wide"}}"#)
            .unwrap();
        assert_eq!(f.columns, BTreeMap::from([("git-list".to_string(), 300.)]));
        for bad in ["null", "[300]", "5", "\"wide\""] {
            let f: F = serde_json::from_str(&format!(r#"{{"columns": {bad}}}"#)).unwrap();
            assert!(f.columns.is_empty(), "{bad} reads as empty");
        }
        let f: F = serde_json::from_str("{}").unwrap();
        assert!(f.columns.is_empty());
    }
}

/// A drag and a double click on the edge of each screen column, over the real screens.
#[cfg(test)]
mod screen_tests {
    use std::os::unix::net::UnixStream;
    use std::sync::Arc;

    use gpui::{
        AppContext as _, Modifiers, MouseButton, MouseDownEvent, MouseUpEvent, Pixels, Point,
        TestAppContext, VisualTestContext,
    };

    use super::ScreenColumn;
    use crate::link::Link;
    use crate::model::CoreState;
    use crate::screens::desk::DeskScreen;
    use crate::screens::git::{GitScreen, Tab};
    use crate::screens::settings::SettingsScreen;

    fn draw(cx: &mut VisualTestContext) {
        cx.run_until_parked();
        cx.update(|window, cx| window.draw(cx).clear(cx));
    }

    fn bounds(cx: &mut VisualTestContext, name: String) -> gpui::Bounds<Pixels> {
        let name: &'static str = Box::leak(name.into_boxed_str());
        cx.debug_bounds(name)
            .unwrap_or_else(|| panic!("{name} is drawn"))
    }

    fn width(cx: &mut VisualTestContext, column: ScreenColumn) -> Pixels {
        bounds(cx, format!("screen-column:{}", column.key()))
            .size
            .width
    }

    fn edge(cx: &mut VisualTestContext, column: ScreenColumn) -> Point<Pixels> {
        bounds(cx, format!("column-edge:{}", column.key())).center()
    }

    fn double_click(cx: &mut VisualTestContext, at: Point<Pixels>) {
        for count in [1, 2] {
            cx.simulate_event(MouseDownEvent {
                button: MouseButton::Left,
                position: at,
                modifiers: Modifiers::none(),
                click_count: count,
                first_mouse: false,
            });
            cx.simulate_event(MouseUpEvent {
                button: MouseButton::Left,
                position: at,
                modifiers: Modifiers::none(),
                click_count: count,
            });
        }
        draw(cx);
    }

    /// Drags the edge to 60 px past the first width, then double-clicks it.
    fn drag_and_reset(cx: &mut VisualTestContext, column: ScreenColumn) {
        cx.simulate_resize(gpui::size(Pixels::from(1600.), Pixels::from(900.)));
        draw(cx);
        assert_eq!(width(cx, column), column.first_width());
        let from = edge(cx, column);
        let shown = bounds(cx, format!("screen-column:{}", column.key()));
        let wide = column.first_width() + Pixels::from(60.);
        let x = match column {
            ScreenColumn::DeskConversation => shown.right() - wide,
            _ => shown.left() + wide,
        };
        let held = MouseButton::Left;
        cx.simulate_mouse_down(from, held, Modifiers::none());
        cx.simulate_mouse_move(
            Point::new(from.x, from.y + Pixels::from(8.)),
            held,
            Modifiers::none(),
        );
        draw(cx);
        let to = Point::new(x, from.y + Pixels::from(8.));
        cx.simulate_mouse_move(to, held, Modifiers::none());
        draw(cx);
        cx.simulate_mouse_up(to, held, Modifiers::none());
        draw(cx);
        assert_eq!(width(cx, column), wide, "{column:?} follows the drag");
        let at = edge(cx, column);
        double_click(cx, at);
        assert_eq!(width(cx, column), column.first_width(), "{column:?} resets");
    }

    fn core(cx: &mut TestAppContext) -> gpui::Entity<CoreState> {
        cx.update(agentsi_ui::init);
        let (end, other) = UnixStream::pair().unwrap();
        // The other end stays open, so a call of the screen waits and does not fail.
        std::mem::forget(other);
        cx.new(|_| CoreState::new(Arc::new(Link::for_tests(end))))
    }

    #[gpui::test]
    fn the_settings_nav_resizes_and_resets(cx: &mut TestAppContext) {
        let core = core(cx);
        let (_, cx) = cx.add_window_view(|_, cx| SettingsScreen::new(core, cx));
        drag_and_reset(cx, ScreenColumn::SettingsNav);
    }

    #[gpui::test]
    fn the_desk_conversation_resizes_and_resets(cx: &mut TestAppContext) {
        let core = core(cx);
        let (_, cx) = cx.add_window_view(|_, cx| DeskScreen::new("demo".into(), core, cx));
        drag_and_reset(cx, ScreenColumn::DeskConversation);
    }

    #[gpui::test]
    fn the_git_list_and_the_git_history_resize_and_reset(cx: &mut TestAppContext) {
        let core = core(cx);
        let (screen, cx) = cx.add_window_view(|_, cx| GitScreen::new("demo".into(), core, cx));
        let status = serde_json::from_value(serde_json::json!({
            "repo": "/r", "branch": "main", "head": "abc12345", "upstream": null,
            "ahead": 0, "behind": 0, "worktrees": [], "files": [],
        }))
        .unwrap();
        screen.update(cx, |s, cx| s.status_loaded(Ok(status), cx));
        drag_and_reset(cx, ScreenColumn::GitList);
        screen.update(cx, |s, cx| s.set_tab(Tab::History, cx));
        drag_and_reset(cx, ScreenColumn::GitHistory);
    }
}
