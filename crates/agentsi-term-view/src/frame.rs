//! The draw lists of one screen, row by row: backgrounds, text runs, shape cells, and the cursor.
//! A row holds no row number, so a row that moves on the screen keeps its lists.
//! [`FrameCache`] builds again only the rows whose cells or selection changed.
//! Building a frame needs no window, so tests can check it.

use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;

use agentsi_ui::tokens::{Color, terminal};
use agentsi_vt::{Cell, Colors, CursorShape, Grid, Rgb, RowId, Terminal, Underline, Width};
use gpui_kit::{Hsla, Pixels, ShapedLine};

use crate::selection::Selection;
use crate::shapes::is_shape;

fn hsla(c: Rgb) -> Hsla {
    Color((u32::from(c.r) << 16) | (u32::from(c.g) << 8) | u32::from(c.b)).hsla()
}

/// A run of cells with one background color.
#[derive(Clone, Debug, PartialEq)]
pub struct Back {
    pub col: u16,
    pub cols: u16,
    pub color: Hsla,
}

/// Cells with one look, drawn as one shaped line with a fixed advance per cell.
#[derive(Clone, Debug, PartialEq)]
pub struct Run {
    pub col: u16,
    /// The cells the run covers. A wide character is one run of 2 cells.
    pub cols: u16,
    /// A run of one wide cell, which gets an advance of 2 cells. Its text can hold several
    /// codepoints, such as a flag or an emoji with a variation selector.
    pub wide: bool,
    pub text: String,
    pub color: Hsla,
    pub bold: bool,
    pub italic: bool,
    pub underline: Underline,
    pub strike: bool,
}

impl Run {
    pub fn is_wide(&self) -> bool {
        self.wide
    }
}

/// A cell drawn with [`crate::shapes::shapes`].
#[derive(Clone, Debug, PartialEq)]
pub struct ShapeCell {
    pub col: u16,
    pub ch: char,
    pub color: Hsla,
}

#[derive(Clone, Debug, PartialEq)]
pub struct CursorCell {
    pub row: u16,
    pub col: u16,
    pub wide: bool,
    pub shape: CursorShape,
    pub blinking: bool,
    pub color: Hsla,
    /// The character under the cursor, drawn in `text_color` on a block cursor.
    pub text: String,
    pub text_color: Hsla,
    pub bold: bool,
    pub italic: bool,
}

/// The shaped lines of a row, with the font size and the cell width that shaped them.
pub type Shaped = ((Pixels, Pixels), Vec<ShapedLine>);

/// The draw lists of one row.
#[derive(Debug, Default)]
pub struct Row {
    /// The cells that the lists come from.
    cells: Vec<Cell>,
    /// The first and the last selected column.
    selected: Option<(u16, u16)>,
    pub backs: Vec<Back>,
    pub runs: Vec<Run>,
    pub shapes: Vec<ShapeCell>,
    /// The shaped lines of `runs`. The first paint of the row makes them.
    pub lines: RefCell<Option<Shaped>>,
}

impl PartialEq for Row {
    /// Shaped lines come from the lists, so they take no part.
    fn eq(&self, other: &Row) -> bool {
        self.cells == other.cells
            && self.selected == other.selected
            && self.backs == other.backs
            && self.runs == other.runs
            && self.shapes == other.shapes
    }
}

impl Row {
    /// True when the row draws `cells` with this selection.
    fn draws(&self, cells: &[Cell], selected: Option<(u16, u16)>) -> bool {
        self.selected == selected && self.cells == cells
    }
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Frame {
    pub cols: u16,
    pub background: Hsla,
    /// The rows from the top of the viewport.
    pub rows: Vec<Rc<Row>>,
    pub cursor: Option<CursorCell>,
}

/// Builds the frame of `grid`, every row anew. `top` is the absolute row of the first row of the
/// grid, for the selection. Tests compare a kept frame with it.
#[cfg(test)]
pub fn build(grid: &Grid, top: u64, selection: Option<&Selection>) -> Frame {
    Frame {
        cols: grid.cols,
        background: hsla(grid.colors.background),
        rows: grid
            .rows
            .iter()
            .enumerate()
            .map(|(y, cells)| {
                let selected = selection.and_then(|s| s.span(top + y as u64, grid.cols));
                Rc::new(build_row(cells, &grid.colors, selected))
            })
            .collect(),
        cursor: cursor(grid),
    }
}

/// The grid and the frame of one terminal, kept from one update to the next.
#[derive(Default)]
pub struct FrameCache {
    grid: Grid,
    /// The row ids of `grid`.
    ids: Vec<RowId>,
    frame: Rc<Frame>,
    /// The default colors that built `frame`. `None` before the first frame.
    colors: Option<Colors>,
}

impl FrameCache {
    /// Reads the rows of `term` that changed and builds the rows whose cells or selection changed.
    /// A row that only moved keeps its lists and its shaped lines. On an error the frame stays,
    /// and the next update builds every row that the failed one read.
    pub fn update(
        &mut self,
        term: &mut Terminal,
        top: u64,
        selection: Option<&Selection>,
    ) -> agentsi_vt::Result<()> {
        let damage = term.update_grid(&mut self.grid)?;
        let grid = &self.grid;
        let old = &self.frame;
        // A cell of the default colors stores no color, so new default colors change every row.
        let same_colors = self.colors == Some(grid.colors) && old.rows.len() == grid.rows.len();
        let mut read = vec![damage.all; grid.rows.len()];
        for y in &damage.rows {
            if let Some(r) = read.get_mut(usize::from(*y)) {
                *r = true;
            }
        }
        let index: HashMap<RowId, usize> = if same_colors {
            self.ids
                .iter()
                .enumerate()
                .map(|(i, id)| (*id, i))
                .collect()
        } else {
            HashMap::new()
        };
        let rows = grid
            .rows
            .iter()
            .enumerate()
            .map(|(y, cells)| {
                let selected = selection.and_then(|s| s.span(top + y as u64, grid.cols));
                let kept = if !same_colors {
                    None
                } else if !read[y] {
                    // libghostty-vt did not change the cells of this row.
                    old.rows.get(y).filter(|row| row.selected == selected)
                } else {
                    let moved = damage
                        .ids
                        .get(y)
                        .and_then(|id| index.get(id))
                        .and_then(|&i| old.rows.get(i));
                    [moved, old.rows.get(y)]
                        .into_iter()
                        .flatten()
                        .find(|row| row.draws(cells, selected))
                };
                kept.cloned()
                    .unwrap_or_else(|| Rc::new(build_row(cells, &grid.colors, selected)))
            })
            .collect();
        self.frame = Rc::new(Frame {
            cols: grid.cols,
            background: hsla(grid.colors.background),
            rows,
            cursor: cursor(grid),
        });
        self.colors = Some(grid.colors);
        self.ids = damage.ids;
        Ok(())
    }

    pub fn grid(&self) -> &Grid {
        &self.grid
    }

    pub fn frame(&self) -> &Rc<Frame> {
        &self.frame
    }
}

/// The background of a cell after inverse and the selection. `None` is the default background.
fn background(cell: &Cell, colors: &Colors, selected: bool) -> Option<Hsla> {
    if selected {
        Some(terminal::SELECTION.hsla())
    } else if cell.style.inverse {
        Some(hsla(cell.fg.unwrap_or(colors.foreground)))
    } else {
        cell.bg.map(hsla)
    }
}

/// The text color of a cell after inverse and faint.
fn foreground(cell: &Cell, colors: &Colors) -> Hsla {
    let fg = if cell.style.inverse {
        cell.bg.unwrap_or(colors.background)
    } else {
        cell.fg.unwrap_or(colors.foreground)
    };
    let mut fg = hsla(fg);
    if cell.style.faint {
        fg.a *= terminal::FAINT_ALPHA;
    }
    fg
}

fn build_row(cells: &[Cell], colors: &Colors, selected: Option<(u16, u16)>) -> Row {
    let mut out = Row {
        cells: cells.to_vec(),
        selected,
        ..Default::default()
    };
    let mut back: Option<Back> = None;
    let mut run: Option<Run> = None;
    // Blank cells after a run wait here. They join the run only if more text of its look follows.
    let mut gap: u16 = 0;
    for (x, cell) in cells.iter().enumerate() {
        let x = x as u16;
        let is_selected = selected.is_some_and(|(a, b)| a <= x && x <= b);
        match (&mut back, background(cell, colors, is_selected)) {
            (Some(b), Some(c)) if b.color == c && b.col + b.cols == x => b.cols += 1,
            (slot, c) => {
                if let Some(b) = slot.take() {
                    out.backs.push(b);
                }
                *slot = c.map(|color| Back {
                    col: x,
                    cols: 1,
                    color,
                });
            }
        }
        if matches!(cell.width, Width::SpacerTail | Width::SpacerHead) {
            continue;
        }
        let ch = single_char(&cell.text);
        let blank = cell.text.is_empty() || cell.text == " " || cell.style.invisible;
        if let Some(c) = ch.filter(|c| is_shape(*c) && !cell.style.invisible) {
            if let Some(run) = run.take() {
                out.runs.push(run);
            }
            gap = 0;
            out.shapes.push(ShapeCell {
                col: x,
                ch: c,
                color: foreground(cell, colors),
            });
            continue;
        }
        if blank {
            if run.is_some() {
                gap += 1;
            }
            continue;
        }
        let fg = foreground(cell, colors);
        let wide = cell.width == Width::Wide;
        let joins = !wide
            && run.as_ref().is_some_and(|r| {
                r.color == fg
                    && r.bold == cell.style.bold
                    && r.italic == cell.style.italic
                    && r.underline == cell.style.underline
                    && r.strike == cell.style.strikethrough
                    && r.col + r.cols + gap == x
                    && !r.is_wide()
            });
        if joins {
            let run = run.as_mut().expect("a run");
            run.text.extend(std::iter::repeat_n(' ', usize::from(gap)));
            run.text.push_str(&cell.text);
            run.cols += gap + 1;
        } else {
            if let Some(run) = run.take() {
                out.runs.push(run);
            }
            run = Some(Run {
                col: x,
                cols: if wide { 2 } else { 1 },
                wide,
                text: cell.text.clone(),
                color: fg,
                bold: cell.style.bold,
                italic: cell.style.italic,
                underline: cell.style.underline,
                strike: cell.style.strikethrough,
            });
        }
        gap = 0;
    }
    out.runs.extend(run);
    out.backs.extend(back);
    out
}

fn single_char(s: &str) -> Option<char> {
    let mut it = s.chars();
    match (it.next(), it.next()) {
        (Some(c), None) => Some(c),
        _ => None,
    }
}

fn cursor(grid: &Grid) -> Option<CursorCell> {
    let c = grid.cursor.filter(|c| c.visible)?;
    let mut col = c.x;
    // On the tail of a wide character, the cursor covers the whole character.
    let wide = c.on_wide_tail
        || grid
            .rows
            .get(usize::from(c.y))
            .and_then(|r| r.get(usize::from(c.x)))
            .is_some_and(|cell| cell.width == Width::Wide);
    if c.on_wide_tail && col > 0 {
        col -= 1;
    }
    let cell = grid
        .rows
        .get(usize::from(c.y))
        .and_then(|r| r.get(usize::from(col)));
    let under_bg = cell
        .and_then(|cell| cell.bg)
        .unwrap_or(grid.colors.background);
    Some(CursorCell {
        row: c.y,
        col,
        wide,
        shape: c.shape,
        blinking: c.blinking,
        color: grid
            .colors
            .cursor
            .map(hsla)
            .unwrap_or_else(|| terminal::CURSOR.hsla()),
        text: cell
            .filter(|cell| !cell.style.invisible)
            .map(|cell| cell.text.clone())
            .unwrap_or_default(),
        text_color: hsla(under_bg),
        bold: cell.is_some_and(|c| c.style.bold),
        italic: cell.is_some_and(|c| c.style.italic),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::selection::{Point, Unit};
    use agentsi_vt::{Options, Palette, Scroll};

    fn term(cols: u16, rows: u16) -> Terminal {
        Terminal::new(Options {
            cols,
            rows,
            scrollback_bytes: 100_000,
        })
        .unwrap()
    }

    fn grid(bytes: &[u8]) -> Grid {
        let mut t = term(20, 3);
        t.write(bytes);
        t.grid().unwrap()
    }

    fn runs(f: &Frame) -> Vec<&Run> {
        f.rows.iter().flat_map(|r| &r.runs).collect()
    }

    fn backs(f: &Frame) -> Vec<&Back> {
        f.rows.iter().flat_map(|r| &r.backs).collect()
    }

    #[test]
    fn text_of_one_look_is_one_run_with_its_inner_blanks() {
        let f = build(&grid(b"ls -la  x"), 0, None);
        let runs = runs(&f);
        assert_eq!(runs.len(), 1);
        assert_eq!(runs[0].text, "ls -la  x");
        assert_eq!((runs[0].col, runs[0].cols), (0, 9));
        assert!(
            backs(&f).is_empty(),
            "the default background is not drawn per cell"
        );
    }

    #[test]
    fn a_change_of_color_or_style_starts_a_new_run() {
        let f = build(&grid(b"a\x1b[31mb\x1b[1mc\x1b[0m d"), 0, None);
        let runs = runs(&f);
        let texts: Vec<&str> = runs.iter().map(|r| r.text.as_str()).collect();
        assert_eq!(texts, ["a", "b", "c", "d"]);
        assert!(runs[2].bold);
        assert_ne!(runs[0].color, runs[1].color);
    }

    #[test]
    fn backgrounds_merge_along_the_row() {
        let f = build(&grid(b"\x1b[44m   \x1b[0m \x1b[41m \x1b[0m"), 0, None);
        let backs = backs(&f);
        assert_eq!(backs.len(), 2);
        assert_eq!((backs[0].col, backs[0].cols), (0, 3));
        assert_eq!((backs[1].col, backs[1].cols), (4, 1));
    }

    #[test]
    fn inverse_swaps_the_colors() {
        let g = grid(b"\x1b[7mx");
        let f = build(&g, 0, None);
        assert_eq!(backs(&f)[0].color, hsla(g.colors.foreground));
        assert_eq!(runs(&f)[0].color, hsla(g.colors.background));
    }

    #[test]
    fn box_block_and_braille_cells_become_shapes() {
        let f = build(&grid("╭─╮ █▌ ⣿ ab".as_bytes()), 0, None);
        let chars: String = f.rows[0].shapes.iter().map(|s| s.ch).collect();
        assert_eq!(chars, "╭─╮█▌⣿");
        let runs = runs(&f);
        assert_eq!(runs.len(), 1);
        assert_eq!(runs[0].text, "ab");
        assert_eq!(runs[0].col, 9);
    }

    #[test]
    fn a_wide_character_is_a_run_of_two_cells() {
        let f = build(&grid("a中b".as_bytes()), 0, None);
        let runs = runs(&f);
        let wide = runs.iter().find(|r| r.text == "中").unwrap();
        assert!(wide.is_wide());
        assert_eq!(wide.col, 1);
        let b = runs.iter().find(|r| r.text == "b").unwrap();
        assert_eq!(b.col, 3);
    }

    #[test]
    fn text_after_a_wide_cluster_of_several_codepoints_keeps_its_column() {
        // A flag is two codepoints in one wide cell, and a heart with VS16 is two codepoints too.
        for text in ["\u{1F1E6}\u{1F1F7}hi", "\u{2764}\u{FE0F}hi"] {
            let f = build(&grid(text.as_bytes()), 0, None);
            let runs = runs(&f);
            let hi = runs.iter().find(|r| r.text.contains('h')).unwrap();
            assert_eq!((hi.text.as_str(), hi.col), ("hi", 2), "{text:?}: {runs:?}");
            assert!(runs[0].is_wide(), "{text:?}: {runs:?}");
        }
    }

    #[test]
    fn the_selection_paints_its_cells() {
        let g = grid(b"hello world");
        let mut s = Selection::new(Point { row: 7, col: 0 }, Unit::Cell, &g.rows[0]);
        s.extend(Point { row: 7, col: 4 }, &g.rows[0]);
        let f = build(&g, 7, Some(&s));
        let backs = backs(&f);
        assert_eq!(backs.len(), 1);
        assert_eq!((backs[0].col, backs[0].cols), (0, 5));
        assert_eq!(backs[0].color, terminal::SELECTION.hsla());
        // The same selection on other absolute rows paints nothing here.
        assert!(super::tests::backs(&build(&g, 100, Some(&s))).is_empty());
    }

    #[test]
    fn the_cursor_carries_the_cell_under_it() {
        let f = build(&grid(b"ab\x1b[D"), 0, None);
        let c = f.cursor.unwrap();
        assert_eq!((c.row, c.col), (0, 1));
        assert_eq!(c.text, "b");
        assert!(!c.wide);
        let hidden = build(&grid(b"x\x1b[?25l"), 0, None);
        assert!(hidden.cursor.is_none());
    }

    #[test]
    fn faint_text_is_see_through() {
        let f = build(&grid(b"\x1b[2mdim"), 0, None);
        assert!(runs(&f)[0].color.a < 1.);
    }

    /// A terminal with a kept frame. Each step updates the kept frame and checks it against a
    /// frame built anew from a full copy of the same state.
    struct KeptFrame {
        term: Terminal,
        cache: FrameCache,
        selection: Option<Selection>,
    }

    impl KeptFrame {
        fn new(cols: u16, rows: u16) -> KeptFrame {
            let mut k = KeptFrame {
                term: term(cols, rows),
                cache: FrameCache::default(),
                selection: None,
            };
            k.check();
            k
        }

        fn top(&self) -> u64 {
            self.term.scrollbar().map(|b| b.offset).unwrap_or(0)
        }

        /// Updates the kept frame, checks it, and returns the rows that it built anew.
        fn check(&mut self) -> Vec<usize> {
            let before = self.cache.frame().clone();
            let top = self.top();
            self.cache
                .update(&mut self.term, top, self.selection.as_ref())
                .unwrap();
            let full = build(&self.term.grid().unwrap(), top, self.selection.as_ref());
            let kept = self.cache.frame();
            assert_eq!(**kept, full, "the kept frame differs from a full build");
            assert_eq!(*self.cache.grid(), self.term.grid().unwrap());
            kept.rows
                .iter()
                .enumerate()
                .filter(|(_, r)| !before.rows.iter().any(|b| Rc::ptr_eq(b, r)))
                .map(|(y, _)| y)
                .collect()
        }

        fn write(&mut self, bytes: &[u8]) -> Vec<usize> {
            self.term.write(bytes);
            self.check()
        }

        fn select(&mut self, from: (u64, u16), to: (u64, u16)) -> Vec<usize> {
            let top = self.top();
            let rows = self.cache.grid().rows.clone();
            let row = |r: u64| rows[(r - top) as usize].clone();
            let mut s = Selection::new(
                Point {
                    row: from.0,
                    col: from.1,
                },
                Unit::Cell,
                &row(from.0),
            );
            s.extend(
                Point {
                    row: to.0,
                    col: to.1,
                },
                &row(to.0),
            );
            self.selection = Some(s);
            self.check()
        }
    }

    fn lines(k: &mut KeptFrame, from: usize, to: usize) {
        for i in from..to {
            k.write(format!("\x1b[3{}mline {i}\x1b[0m ╭─╮ █ ⣿\r\n", i % 7 + 1).as_bytes());
        }
    }

    #[test]
    fn a_quiet_update_builds_no_row() {
        let mut k = KeptFrame::new(30, 6);
        lines(&mut k, 0, 3);
        assert!(k.check().is_empty());
    }

    #[test]
    fn a_scroll_builds_only_the_new_rows() {
        let mut k = KeptFrame::new(30, 6);
        lines(&mut k, 0, 10);
        let built = k.write(b"\x1b[32mnew line\x1b[0m\r\n");
        // The rows moved up one place. Only the new line is built. The empty last row takes the
        // lists of the empty row that was there before.
        assert_eq!(built, [4]);
    }

    #[test]
    fn an_edit_in_place_builds_only_its_row() {
        let mut k = KeptFrame::new(30, 6);
        lines(&mut k, 0, 4);
        assert_eq!(k.write(b"\x1b[2;3H\x1b[41mX\x1b[0m"), [1]);
        assert_eq!(k.write(b"\r\x1b[K"), [1]);
    }

    #[test]
    fn a_scroll_of_the_viewport_and_back_is_drawn_right() {
        let mut k = KeptFrame::new(30, 6);
        lines(&mut k, 0, 20);
        for scroll in [
            Scroll::Lines(-2),
            Scroll::Lines(-3),
            Scroll::Top,
            Scroll::Lines(4),
            Scroll::Bottom,
        ] {
            k.term.scroll(scroll);
            k.check();
        }
        // Output while the viewport is in the scrollback, then the way back.
        k.term.scroll(Scroll::Lines(-5));
        k.check();
        lines(&mut k, 20, 23);
        k.term.scroll(Scroll::Bottom);
        k.check();
    }

    #[test]
    fn a_resize_is_drawn_right() {
        let mut k = KeptFrame::new(30, 6);
        lines(&mut k, 0, 8);
        for (cols, rows) in [(20, 6), (20, 9), (40, 4), (30, 6)] {
            k.term.resize(cols, rows, 8, 16).unwrap();
            k.check();
        }
    }

    #[test]
    fn new_colors_build_every_row() {
        let mut k = KeptFrame::new(30, 6);
        lines(&mut k, 0, 3);
        // The palette of the pane, then the program's own colors: OSC 4, then OSC 10 and 11.
        k.term
            .set_palette(&Palette {
                foreground: Rgb { r: 1, g: 2, b: 3 },
                background: Rgb { r: 4, g: 5, b: 6 },
                cursor: None,
                ansi: [Rgb { r: 7, g: 8, b: 9 }; 16],
            })
            .unwrap();
        assert_eq!(k.check().len(), 6);
        // A cell stores its palette color as RGB, so OSC 4 changes only the rows of color 1.
        assert_eq!(k.write(b"\x1b]4;1;#00ff00\x07"), [0]);
        assert_eq!(k.write(b"\x1b]10;#ff0000\x07\x1b]11;#000080\x07").len(), 6);
    }

    #[test]
    fn a_selection_change_builds_only_the_rows_it_touches() {
        let mut k = KeptFrame::new(30, 6);
        lines(&mut k, 0, 4);
        let top = k.top();
        assert_eq!(k.select((top + 1, 2), (top + 2, 4)), [1, 2]);
        assert_eq!(k.select((top + 1, 2), (top + 3, 1)), [2, 3]);
        k.selection = None;
        assert_eq!(k.check(), [1, 2, 3]);
        // A selection that the scroll moves keeps its text.
        assert_eq!(k.select((top + 1, 0), (top + 1, 5)), [1]);
        lines(&mut k, 4, 9);
    }

    #[test]
    fn a_cursor_change_keeps_every_row() {
        let mut k = KeptFrame::new(30, 6);
        lines(&mut k, 0, 3);
        for bytes in [
            &b"\x1b[1;1H"[..],
            b"\x1b[?25l",
            b"\x1b[?25h",
            b"\x1b[5 q",
            b"\x1b[2 q",
        ] {
            let built = k.write(bytes);
            assert!(built.is_empty(), "{bytes:?} built rows {built:?}");
            assert_eq!(k.cache.frame().cursor, cursor(&k.term.grid().unwrap()));
        }
    }

    #[test]
    fn new_ansi_colors_alone_build_the_rows_that_use_them() {
        let mut k = KeptFrame::new(30, 6);
        k.write(b"plain\r\n\x1b[34mblue\x1b[0m\r\n");
        let mut palette = Palette {
            foreground: Rgb {
                r: 0xee,
                g: 0xee,
                b: 0xee,
            },
            background: Rgb { r: 0, g: 0, b: 0 },
            cursor: None,
            ansi: [Rgb { r: 9, g: 9, b: 9 }; 16],
        };
        k.term.set_palette(&palette).unwrap();
        k.check();
        palette.ansi[4] = Rgb { r: 1, g: 2, b: 200 };
        k.term.set_palette(&palette).unwrap();
        assert_eq!(k.check(), [1]);
    }

    #[test]
    fn a_failed_read_with_new_colors_still_builds_every_row() {
        let mut k = KeptFrame::new(30, 6);
        lines(&mut k, 0, 3);
        // A read that fails after it took the new default colors leaves them in the grid, and the
        // frame of the old colors.
        let mut palette = Palette {
            foreground: Rgb { r: 200, g: 0, b: 0 },
            ..Palette::default()
        };
        palette.ansi = [Rgb { r: 9, g: 9, b: 9 }; 16];
        k.term.set_palette(&palette).unwrap();
        let colors = k.term.grid().unwrap().colors;
        assert_ne!(colors, k.cache.grid.colors);
        k.cache.grid.colors = colors;
        assert_eq!(k.check().len(), 6);
    }

    #[test]
    fn programs_that_redraw_parts_of_the_screen_are_drawn_right() {
        let mut k = KeptFrame::new(30, 8);
        lines(&mut k, 0, 12);
        for bytes in [
            // A scroll region, inserted and deleted lines, an erase with a background.
            &b"\x1b[2;5r\x1b[5;1H\nregion\x1b[r"[..],
            b"\x1b[3;1H\x1b[2L",
            b"\x1b[4;1H\x1b[M",
            b"\x1b[44m\x1b[6;1H\x1b[J\x1b[0m",
            // Wide characters, an emoji with VS16, a combining mark, and a reverse index.
            "\x1b[1;1H中文 \u{2764}\u{FE0F} e\u{301}".as_bytes(),
            b"\x1b[1;1H\x1bM\x1bM",
            // The alternate screen, a draw on it, and the way back.
            b"\x1b[?1049h\x1b[2J\x1b[3;3Htui",
            b"\x1b[3;3H\x1b[7mTUI\x1b[0m",
            b"\x1b[?1049l",
            // A full reset.
            b"\x1bc",
        ] {
            k.write(bytes);
        }
    }

    #[test]
    fn a_random_mix_of_output_and_moves_is_always_drawn_right() {
        let mut k = KeptFrame::new(24, 7);
        let mut seed = 0x2545_f491_4f6c_dd1du64;
        let mut next = |n: u64| {
            seed ^= seed << 13;
            seed ^= seed >> 7;
            seed ^= seed << 17;
            seed % n
        };
        let pieces: [&[u8]; 14] = [
            b"word ",
            b"\r\n",
            b"\x1b[31mred\x1b[0m",
            b"\x1b[7m inv \x1b[0m",
            "╭─╮⣿█".as_bytes(),
            "中".as_bytes(),
            b"\x1b[A",
            b"\x1b[2K",
            b"\x1b[H",
            b"\x1b[3;6r\x1b[6;1H\n\x1b[r",
            b"\x1b[L",
            b"\x1b[P",
            b"\x1b[44m\x1b[K\x1b[0m",
            b"\x1b[?1049h",
        ];
        for step in 0..400 {
            match next(10) {
                0 => k.term.scroll(Scroll::Lines(next(5) as isize - 2)),
                1 => k.term.scroll(Scroll::Bottom),
                2 if step % 50 == 0 => k
                    .term
                    .resize(20 + next(10) as u16, 5 + next(4) as u16, 8, 16)
                    .unwrap(),
                3 => {
                    let rows = k.cache.grid().rows.len() as u64;
                    let top = k.top();
                    let a = top + next(rows);
                    let b = top + next(rows);
                    k.select((a.min(b), next(8) as u16), (a.max(b), next(8) as u16));
                }
                4 => k.selection = None,
                5 if step % 40 == 0 => k.term.write(b"\x1b[?1049l"),
                _ => {
                    let piece = pieces[next(pieces.len() as u64) as usize];
                    k.term.write(piece);
                }
            }
            k.check();
        }
    }
}
