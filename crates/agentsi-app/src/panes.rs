//! The panes: the tools outside the dock (Products, Files and Alerts), in two columns at the
//! sides of the window.
//!
//! A column holds its open panes top to bottom. A hidden pane takes no place, and a column with
//! no open pane takes no width. Each open pane has a share of the height of its column. The
//! panes are global: each space shows the same columns. `layout.json` keeps them in `panes`.

use agentsi_ui::tokens::control;
use gpui_kit::Pixels;
use serde::{Deserialize, Serialize};

/// One tool outside the dock.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum PaneKind {
    Products,
    Files,
    Alerts,
}

impl PaneKind {
    /// The order of the toggles in the pane bar.
    pub const ALL: [PaneKind; 3] = [PaneKind::Products, PaneKind::Files, PaneKind::Alerts];

    pub const fn name(self) -> &'static str {
        match self {
            PaneKind::Products => "Products",
            PaneKind::Files => "Files",
            PaneKind::Alerts => "Alerts",
        }
    }

    /// The id of its toggle, and of its key in `keys.toml`.
    pub const fn key_id(self) -> &'static str {
        match self {
            PaneKind::Products => "toggle-products",
            PaneKind::Files => "toggle-files",
            PaneKind::Alerts => "toggle-alerts",
        }
    }

    /// The test selectors of a pane use this word.
    pub const fn word(self) -> &'static str {
        match self {
            PaneKind::Products => "products",
            PaneKind::Files => "files",
            PaneKind::Alerts => "alerts",
        }
    }
}

/// A column of panes.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Side {
    Left,
    Right,
}

impl Side {
    pub const fn word(self) -> &'static str {
        match self {
            Side::Left => "left",
            Side::Right => "right",
        }
    }
}

/// One pane in `layout.json`.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct PaneSave {
    pub pane: PaneKind,
    pub column: Side,
    /// Its place in its column, from the top.
    pub order: u32,
    /// Its share of the height of its column, against the other open panes there.
    pub share: f32,
    pub open: bool,
}

/// The field `panes` of `layout.json`. A pane or a value that does not read is dropped, and
/// `Panes::load` fills its default: a file of a newer app, or one edited by hand, must not cost
/// the spaces of the rest of the file.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct PanesSave {
    #[serde(default, deserialize_with = "readable")]
    pub panes: Vec<PaneSave>,
    /// The widths of the two columns, in px. A missing width reads as 0, which `load` resets.
    #[serde(default)]
    pub left_width: f32,
    #[serde(default)]
    pub right_width: f32,
    /// The panes that "Toggle all panes" hid, to show again.
    #[serde(
        default,
        deserialize_with = "readable",
        skip_serializing_if = "Vec::is_empty"
    )]
    pub stashed: Vec<PaneKind>,
}

/// The items of a list that read as `T`.
fn readable<'de, D, T>(d: D) -> Result<Vec<T>, D::Error>
where
    D: serde::Deserializer<'de>,
    T: serde::de::DeserializeOwned,
{
    let values = Vec::<serde_json::Value>::deserialize(d)?;
    Ok(values
        .into_iter()
        .filter_map(|v| serde_json::from_value(v).ok())
        .collect())
}

/// The field `panes` of `layout.json`, or `None` when it does not read at all.
pub fn read_saved<'de, D>(d: D) -> Result<Option<PanesSave>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    let value = Option::<serde_json::Value>::deserialize(d)?;
    Ok(value.and_then(|v| serde_json::from_value(v).ok()))
}

/// The largest share that a file can give a pane. A larger one, or one that is not a positive
/// number, reads as 1: the sum of the shares of a column must stay finite.
const SHARE_MAX: f32 = 1e4;

#[derive(Clone, Debug, PartialEq)]
struct Entry {
    kind: PaneKind,
    side: Side,
    share: f32,
    open: bool,
}

/// Where a moved pane goes: a column, and the open pane that it goes above, or `None` for the
/// bottom of the column.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Place {
    pub side: Side,
    pub before: Option<PaneKind>,
}

/// The places, sizes and open state of the panes.
#[derive(Clone, Debug, PartialEq)]
pub struct Panes {
    /// Each pane once, the left column first. The order inside a column is the order of this
    /// list.
    entries: Vec<Entry>,
    left: f32,
    right: f32,
    stashed: Vec<PaneKind>,
}

impl Default for Panes {
    /// Products open on the left, Files hidden under it, Alerts hidden on the right.
    fn default() -> Panes {
        let entry = |kind, side, open| Entry {
            kind,
            side,
            share: 1.,
            open,
        };
        let width = f32::from(control::PANE_COLUMN_WIDTH);
        Panes {
            entries: vec![
                entry(PaneKind::Products, Side::Left, true),
                entry(PaneKind::Files, Side::Left, false),
                entry(PaneKind::Alerts, Side::Right, false),
            ],
            left: width,
            right: width,
            stashed: Vec::new(),
        }
    }
}

/// The width of a column in a window `window` px wide: between `PANE_COLUMN_MIN` and
/// `PANE_COLUMN_MAX` of the window. A window too narrow for both gives the half.
pub fn clamp_width(width: f32, window: f32) -> f32 {
    let max = window * control::PANE_COLUMN_MAX;
    width.max(f32::from(control::PANE_COLUMN_MIN)).min(max)
}

impl Panes {
    /// The panes of a file of before `panes`: its `sidebar_mode` shows Files, else Products.
    pub fn from_sidebar_mode(mode: Option<&str>) -> Panes {
        let mut p = Panes::default();
        if mode == Some("files") {
            p.set_open(PaneKind::Products, false);
            p.set_open(PaneKind::Files, true);
        }
        p
    }

    /// The panes of the file: `panes` when it has them, else the old sidebar mode. A pane that
    /// the file lacks keeps its default place, hidden.
    pub fn load(saved: Option<&PanesSave>, sidebar_mode: Option<&str>) -> Panes {
        let Some(saved) = saved else {
            return Panes::from_sidebar_mode(sidebar_mode);
        };
        let mut list: Vec<&PaneSave> = saved.panes.iter().collect();
        list.sort_by_key(|p| (p.column as u8, p.order, p.pane));
        let mut entries: Vec<Entry> = Vec::new();
        for p in list {
            if entries.iter().any(|e| e.kind == p.pane) {
                continue;
            }
            entries.push(Entry {
                kind: p.pane,
                side: p.column,
                share: if p.share > 0. && p.share <= SHARE_MAX {
                    p.share
                } else {
                    1.
                },
                open: p.open,
            });
        }
        for d in Panes::default().entries {
            if !entries.iter().any(|e| e.kind == d.kind) {
                entries.push(Entry { open: false, ..d });
            }
        }
        entries.sort_by_key(|e| e.side as u8);
        let width = |w: f32| {
            if w.is_finite() && w > 0. {
                w
            } else {
                f32::from(control::PANE_COLUMN_WIDTH)
            }
        };
        let mut stashed = saved.stashed.clone();
        stashed.dedup();
        Panes {
            entries,
            left: width(saved.left_width),
            right: width(saved.right_width),
            stashed,
        }
    }

    pub fn save(&self) -> PanesSave {
        let mut order = [0u32; 2];
        let panes = self
            .entries
            .iter()
            .map(|e| {
                let n = &mut order[e.side as usize];
                *n += 1;
                PaneSave {
                    pane: e.kind,
                    column: e.side,
                    order: *n - 1,
                    share: e.share,
                    open: e.open,
                }
            })
            .collect();
        PanesSave {
            panes,
            left_width: self.left,
            right_width: self.right,
            stashed: self.stashed.clone(),
        }
    }

    fn entry(&self, kind: PaneKind) -> &Entry {
        self.entries
            .iter()
            .find(|e| e.kind == kind)
            .expect("each pane has an entry")
    }

    fn entry_mut(&mut self, kind: PaneKind) -> &mut Entry {
        self.entries
            .iter_mut()
            .find(|e| e.kind == kind)
            .expect("each pane has an entry")
    }

    pub fn is_open(&self, kind: PaneKind) -> bool {
        self.entry(kind).open
    }

    pub fn side(&self, kind: PaneKind) -> Side {
        self.entry(kind).side
    }

    pub fn share(&self, kind: PaneKind) -> f32 {
        self.entry(kind).share
    }

    /// The open panes, in each column top to bottom.
    pub fn open(&self) -> Vec<PaneKind> {
        self.entries
            .iter()
            .filter(|e| e.open)
            .map(|e| e.kind)
            .collect()
    }

    /// The open panes of a column, top to bottom.
    pub fn column(&self, side: Side) -> Vec<PaneKind> {
        self.entries
            .iter()
            .filter(|e| e.open && e.side == side)
            .map(|e| e.kind)
            .collect()
    }

    /// The width that the owner gave the column, before the clamp to the window.
    pub fn width(&self, side: Side) -> f32 {
        match side {
            Side::Left => self.left,
            Side::Right => self.right,
        }
    }

    fn width_mut(&mut self, side: Side) -> &mut f32 {
        match side {
            Side::Left => &mut self.left,
            Side::Right => &mut self.right,
        }
    }

    /// The width that the column takes in a window `window` px wide: 0 with no open pane. When
    /// the two columns would leave the dock less than `DOCK_MIN_WIDTH`, both get narrower in
    /// the same ratio.
    pub fn shown_width(&self, side: Side, window: Pixels) -> Pixels {
        let window = f32::from(window);
        let clamped = |s: Side| {
            if self.column(s).is_empty() {
                0.
            } else {
                clamp_width(self.width(s), window)
            }
        };
        let (own, both) = (clamped(side), clamped(Side::Left) + clamped(Side::Right));
        let room = (window - f32::from(control::DOCK_MIN_WIDTH)).max(0.);
        if both > room {
            Pixels::from(own * room / both)
        } else {
            Pixels::from(own)
        }
    }

    /// Opens or hides a pane. Returns false when it was so already.
    pub fn set_open(&mut self, kind: PaneKind, open: bool) -> bool {
        let e = self.entry_mut(kind);
        if e.open == open {
            return false;
        }
        e.open = open;
        true
    }

    pub fn toggle(&mut self, kind: PaneKind) {
        let open = !self.is_open(kind);
        self.set_open(kind, open);
    }

    /// Hides each open pane and keeps them. With no open pane it shows the kept panes again,
    /// or Products when none is kept.
    pub fn toggle_all(&mut self) {
        let open = self.open();
        if !open.is_empty() {
            for k in &open {
                self.set_open(*k, false);
            }
            self.stashed = open;
            return;
        }
        let back = std::mem::take(&mut self.stashed);
        if back.is_empty() {
            self.set_open(PaneKind::Products, true);
        }
        for k in back {
            self.set_open(k, true);
        }
    }

    /// Sets the width of a column from the window width. Returns false when it stays.
    pub fn set_width(&mut self, side: Side, width: f32, window: f32) -> bool {
        self.put_width(side, clamp_width(width, window))
    }

    fn put_width(&mut self, side: Side, width: f32) -> bool {
        let slot = self.width_mut(side);
        if *slot == width {
            return false;
        }
        *slot = width;
        true
    }

    /// The double click on the edge of a column: back to the first width.
    pub fn reset_width(&mut self, side: Side) -> bool {
        self.put_width(side, f32::from(control::PANE_COLUMN_WIDTH))
    }

    /// The heights of the open panes of a column `height` px high, top to bottom.
    pub fn heights(&self, side: Side, height: f32) -> Vec<(PaneKind, f32)> {
        let panes = self.column(side);
        let total: f32 = panes.iter().map(|k| self.share(*k)).sum();
        panes
            .into_iter()
            .map(|k| (k, self.share(k) / total * height))
            .collect()
    }

    /// Moves the edge under the open pane `n` of a column to `y` px from the top of the
    /// column. The pane over the edge and the pane under it share their height, and each keeps
    /// `PANE_MIN_HEIGHT` at least. Returns false when nothing changed.
    pub fn drag_split(&mut self, side: Side, n: usize, y: f32, height: f32) -> bool {
        let heights = self.heights(side, height);
        let (Some(&(upper, hu)), Some(&(lower, hl))) = (heights.get(n), heights.get(n + 1)) else {
            return false;
        };
        let min = f32::from(control::PANE_MIN_HEIGHT);
        let pair = hu + hl;
        if pair < 2. * min {
            return false;
        }
        let top: f32 = heights[..n].iter().map(|(_, h)| h).sum();
        let new_upper = (y - top).max(min).min(pair - min);
        if (new_upper - hu).abs() < f32::EPSILON {
            return false;
        }
        let total: f32 = heights.iter().map(|(k, _)| self.share(*k)).sum();
        let per_px = total / height;
        self.entry_mut(upper).share = new_upper * per_px;
        self.entry_mut(lower).share = (pair - new_upper) * per_px;
        true
    }

    /// The place that a pane dragged to `y` px from the top of the column `side`, `height` px
    /// high, goes: above the first open pane whose middle is under `y`. The dragged pane
    /// itself does not count.
    pub fn place_at(&self, moved: PaneKind, side: Side, y: f32, height: f32) -> Place {
        let mut top = 0.;
        for (k, h) in self.heights(side, height) {
            if k != moved && y < top + h / 2. {
                return Place {
                    side,
                    before: Some(k),
                };
            }
            top += h;
        }
        Place { side, before: None }
    }

    /// True when a drop at `place` leaves `moved` where it is.
    pub fn is_same_place(&self, moved: PaneKind, place: Place) -> bool {
        if self.side(moved) != place.side {
            return false;
        }
        let column = self.column(place.side);
        let at = column.iter().position(|k| *k == moved);
        let after = at.and_then(|i| column.get(i + 1)).copied();
        at.is_some() && after == place.before
    }

    /// Puts `moved` at `place`. Returns false when it was there already.
    pub fn move_to(&mut self, moved: PaneKind, place: Place) -> bool {
        if self.is_same_place(moved, place) || place.before == Some(moved) {
            return false;
        }
        let at = self
            .entries
            .iter()
            .position(|e| e.kind == moved)
            .expect("each pane has an entry");
        let mut e = self.entries.remove(at);
        // A pane that joins another column takes the mean share there, so it does not crush
        // or fill the column.
        let others = self.column(place.side);
        if e.side != place.side && !others.is_empty() {
            let sum: f32 = others.iter().map(|k| self.share(*k)).sum();
            e.share = sum / others.len() as f32;
        }
        e.side = place.side;
        // The bottom of a column is under its last open pane.
        let to = match place.before {
            Some(b) => self.entries.iter().position(|x| x.kind == b),
            None => self
                .entries
                .iter()
                .rposition(|x| x.side == place.side && x.open)
                .map(|i| i + 1)
                .or_else(|| self.entries.iter().position(|x| x.side == place.side)),
        };
        match to {
            Some(i) => self.entries.insert(i, e),
            None => self.entries.push(e),
        }
        self.entries.sort_by_key(|e| e.side as u8);
        true
    }
}

#[cfg(test)]
mod tests {
    use super::{PaneKind, Panes, Place, Side, clamp_width};
    use agentsi_ui::tokens::control;

    const P: PaneKind = PaneKind::Products;
    const F: PaneKind = PaneKind::Files;
    const A: PaneKind = PaneKind::Alerts;

    #[test]
    fn a_file_of_before_the_panes_opens_with_its_sidebar_mode() {
        let none = Panes::load(None, None);
        assert_eq!(none.open(), [P]);
        assert_eq!(none.side(P), Side::Left);
        let files = Panes::load(None, Some("files"));
        assert_eq!(files.open(), [F]);
        assert_eq!(files.side(F), Side::Left);
        assert_eq!(Panes::load(None, Some("products")).open(), [P]);
    }

    #[test]
    fn the_saved_panes_come_back_with_their_column_order_size_and_state() {
        let mut p = Panes::default();
        p.set_open(F, true);
        p.set_open(A, true);
        p.move_to(
            F,
            Place {
                side: Side::Left,
                before: Some(P),
            },
        );
        assert!(p.drag_split(Side::Left, 0, 300., 700.));
        p.set_width(Side::Left, 320., 1280.);
        p.set_width(Side::Right, 200., 1280.);
        let text = serde_json::to_string(&p.save()).unwrap();
        let back = Panes::load(Some(&serde_json::from_str(&text).unwrap()), Some("files"));
        assert_eq!(back, p, "the saved panes win over the old sidebar mode");
        assert_eq!(back.column(Side::Left), [F, P]);
        assert_eq!(back.column(Side::Right), [A]);
        assert_eq!(back.width(Side::Left), 320.);
    }

    #[test]
    fn a_pane_that_the_file_lacks_gets_its_default_place_hidden() {
        let saved = super::PanesSave {
            panes: vec![super::PaneSave {
                pane: F,
                column: Side::Right,
                order: 0,
                share: f32::NAN,
                open: true,
            }],
            left_width: -1.,
            right_width: 300.,
            stashed: vec![],
        };
        let p = Panes::load(Some(&saved), None);
        assert_eq!(p.open(), [F]);
        assert_eq!(p.side(F), Side::Right);
        assert_eq!(p.share(F), 1.);
        assert!(!p.is_open(P) && !p.is_open(A));
        assert_eq!(p.width(Side::Left), f32::from(control::PANE_COLUMN_WIDTH));
    }

    #[test]
    fn toggle_all_hides_the_open_panes_and_shows_the_same_ones_again() {
        let mut p = Panes::default();
        p.set_open(A, true);
        p.toggle_all();
        assert!(p.open().is_empty());
        p.toggle_all();
        assert_eq!(p.open(), [P, A]);
        // A pane opened by hand in between is what the next use hides.
        p.toggle_all();
        p.toggle(F);
        p.toggle_all();
        assert!(p.open().is_empty());
        p.toggle_all();
        assert_eq!(p.open(), [F]);
    }

    #[test]
    fn a_column_stays_between_its_minimum_and_half_the_window() {
        let mut p = Panes::default();
        p.set_width(Side::Left, 40., 1280.);
        assert_eq!(p.width(Side::Left), 180.);
        p.set_width(Side::Left, 900., 1280.);
        assert_eq!(p.width(Side::Left), 640.);
        assert_eq!(
            clamp_width(248., 300.),
            150.,
            "a narrow window gives the half"
        );
        assert!(p.reset_width(Side::Left));
        assert_eq!(p.width(Side::Left), 248.);
        assert!(!p.reset_width(Side::Left));
        p.toggle(P);
        assert_eq!(
            p.shown_width(Side::Left, gpui_kit::Pixels::from(1280.)),
            gpui_kit::Pixels::ZERO,
            "a column with no open pane takes no width"
        );
    }

    #[test]
    fn two_wide_columns_leave_the_dock_its_minimum_width() {
        let window = gpui_kit::Pixels::from(1280.);
        let mut p = Panes::default();
        p.set_open(A, true);
        p.set_width(Side::Left, 640., 1280.);
        p.set_width(Side::Right, 640., 1280.);
        let (l, r) = (
            p.shown_width(Side::Left, window),
            p.shown_width(Side::Right, window),
        );
        assert_eq!(l, r, "both columns give up the same ratio");
        assert_eq!(window - l - r, control::DOCK_MIN_WIDTH);
        let narrow = gpui_kit::Pixels::from(300.);
        let both = p.shown_width(Side::Left, narrow) + p.shown_width(Side::Right, narrow);
        assert_eq!(narrow - both, control::DOCK_MIN_WIDTH);
    }

    #[test]
    fn a_pane_or_a_value_that_does_not_read_is_dropped_and_the_file_still_reads() {
        let text = r#"{"version":3,"spaces":{},"panes":{
            "panes":[
                {"pane":"git","column":"left","order":0,"share":1,"open":true},
                {"pane":"files","column":"right","order":-1,"share":1,"open":true},
                {"pane":"alerts","column":"right","order":0,"open":true},
                {"pane":"products","column":"right","order":1,"share":2,"open":true}
            ],
            "stashed":["git","files"]
        }}"#;
        let layout: crate::layout::Layout = serde_json::from_str(text).expect("the file reads");
        let saved = layout.panes.expect("the panes read");
        assert_eq!(saved.panes.len(), 1, "only the products entry reads");
        assert_eq!(saved.stashed, [F]);
        let p = Panes::load(Some(&saved), None);
        assert_eq!(p.column(Side::Right), [P]);
        assert_eq!(p.share(P), 2.);
        assert_eq!(p.width(Side::Left), f32::from(control::PANE_COLUMN_WIDTH));

        let text = r#"{"version":3,"panes":"newer"}"#;
        let layout: crate::layout::Layout = serde_json::from_str(text).expect("the file reads");
        assert_eq!(layout.panes, None);
    }

    #[test]
    fn a_huge_share_reads_as_one_so_the_sum_stays_finite() {
        let mut saved = Panes::default().save();
        for e in &mut saved.panes {
            e.share = 3e38;
            e.open = true;
            e.column = Side::Left;
        }
        let mut p = Panes::load(Some(&saved), None);
        assert!(p.heights(Side::Left, 600.).iter().all(|(_, h)| *h == 200.));
        p.move_to(
            A,
            Place {
                side: Side::Right,
                before: None,
            },
        );
        assert!(p.save().panes.iter().all(|e| e.share.is_finite()));
    }

    #[test]
    fn a_split_moves_and_no_pane_gets_under_its_minimum_height() {
        let mut p = Panes::default();
        p.set_open(F, true);
        assert_eq!(p.heights(Side::Left, 600.), [(P, 300.), (F, 300.)]);
        assert!(p.drag_split(Side::Left, 0, 200., 600.));
        assert_eq!(p.heights(Side::Left, 600.), [(P, 200.), (F, 400.)]);
        p.drag_split(Side::Left, 0, 10., 600.);
        assert_eq!(p.heights(Side::Left, 600.), [(P, 120.), (F, 480.)]);
        p.drag_split(Side::Left, 0, 590., 600.);
        assert_eq!(p.heights(Side::Left, 600.), [(P, 480.), (F, 120.)]);
        assert!(
            !p.drag_split(Side::Left, 1, 100., 600.),
            "no pane under the last"
        );
    }

    #[test]
    fn a_pane_moves_inside_its_column_and_to_the_other_one() {
        let mut p = Panes::default();
        p.set_open(F, true);
        let left = |before| Place {
            side: Side::Left,
            before,
        };
        assert!(p.is_same_place(P, left(Some(F))));
        assert!(!p.move_to(P, left(Some(F))), "a drop at its own place");
        assert!(p.move_to(P, left(None)));
        assert_eq!(p.column(Side::Left), [F, P]);
        assert_eq!(p.place_at(F, Side::Left, 10., 600.), left(Some(P)));
        assert_eq!(p.place_at(F, Side::Left, 500., 600.), left(None));
        let right = Place {
            side: Side::Right,
            before: None,
        };
        assert!(p.move_to(F, right));
        assert_eq!(p.column(Side::Left), [P]);
        assert_eq!(p.column(Side::Right), [F]);
        p.set_open(A, true);
        assert_eq!(p.column(Side::Right), [F, A]);
    }
}
