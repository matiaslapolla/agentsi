//! Text selection with the mouse. Rows are absolute: the row in the scrollback plus the screen,
//! so a selection stays on its text when the viewport moves.

use agentsi_vt::{Cell, Width};

/// A cell position. `row` counts from the first row of the scrollback.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct Point {
    pub row: u64,
    pub col: u16,
}

/// What one click selects, and what a drag adds: cells, words or lines.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Unit {
    Cell,
    Word,
    Line,
}

impl Unit {
    /// One click selects cells, two select words, three select lines.
    pub fn from_clicks(count: usize) -> Unit {
        match count {
            0 | 1 => Unit::Cell,
            2 => Unit::Word,
            _ => Unit::Line,
        }
    }
}

/// The characters that end a word, as in Ghostty's default `selection-word-chars`.
const WORD_STOPS: &str = " \t'\"│`|:;,()[]{}<>$";

/// A selection from `start` to `end`, both included, in reading order.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Selection {
    unit: Unit,
    /// The range of the first click, which a drag keeps.
    anchor: (Point, Point),
    pub start: Point,
    pub end: Point,
}

impl Selection {
    /// Starts a selection at `at`. `row` holds the cells of the row of `at`.
    pub fn new(at: Point, unit: Unit, row: &[Cell]) -> Selection {
        let anchor = expand(at, unit, row);
        Selection {
            unit,
            anchor,
            start: anchor.0,
            end: anchor.1,
        }
    }

    /// Moves the free end of the selection to `to`. `row` holds the cells of the row of `to`.
    pub fn extend(&mut self, to: Point, row: &[Cell]) {
        let (a, b) = expand(to, self.unit, row);
        self.start = self.anchor.0.min(a);
        self.end = self.anchor.1.max(b);
    }

    /// True when the selection holds only the cell of one click, with no drag.
    pub fn is_single_cell(&self) -> bool {
        self.unit == Unit::Cell && (self.start, self.end) == self.anchor
    }

    #[cfg(test)]
    pub fn contains(&self, p: Point) -> bool {
        self.start <= p && p <= self.end
    }

    /// The first and the last selected column of the absolute row `row`, in a row of `cols` cells.
    pub fn span(&self, row: u64, cols: u16) -> Option<(u16, u16)> {
        if row < self.start.row || row > self.end.row || cols == 0 {
            return None;
        }
        let from = if row == self.start.row {
            self.start.col
        } else {
            0
        };
        let to = if row == self.end.row {
            self.end.col
        } else {
            cols - 1
        };
        (from <= to).then_some((from, to))
    }

    /// The selected text of the rows in `rows`, as (absolute row, cells) pairs in order. Trailing
    /// blanks of each line go, and lines join with `\n`. Rows that `rows` lacks are left out.
    pub fn text<'a>(&self, rows: impl IntoIterator<Item = (u64, &'a [Cell])>) -> String {
        let mut lines = Vec::new();
        for (row, cells) in rows {
            if row < self.start.row || row > self.end.row {
                continue;
            }
            let from = if row == self.start.row {
                usize::from(self.start.col)
            } else {
                0
            };
            let to = if row == self.end.row {
                usize::from(self.end.col) + 1
            } else {
                cells.len()
            };
            let mut line = String::new();
            for cell in cells.iter().take(to.min(cells.len())).skip(from) {
                match cell.width {
                    Width::SpacerTail | Width::SpacerHead => {}
                    _ if cell.text.is_empty() => line.push(' '),
                    _ => line.push_str(&cell.text),
                }
            }
            line.truncate(line.trim_end().len());
            lines.push(line);
        }
        lines.join("\n")
    }
}

/// The range that one click of `unit` at `at` selects.
fn expand(at: Point, unit: Unit, row: &[Cell]) -> (Point, Point) {
    let last = row.len().saturating_sub(1) as u16;
    let col = head_of(at.col.min(last), row);
    let p = |col: u16| Point { row: at.row, col };
    match unit {
        Unit::Cell => {
            let end = if row
                .get(usize::from(col))
                .is_some_and(|c| c.width == Width::Wide)
            {
                col + 1
            } else {
                col
            };
            (p(col), p(end.min(last)))
        }
        Unit::Line => (p(0), p(last)),
        Unit::Word => {
            let class = |c: u16| class_of(&row[usize::from(c)]);
            if row.is_empty() {
                return (p(0), p(0));
            }
            let kind = class(col);
            let mut a = col;
            while a > 0 && same_class(class(a - 1), kind) {
                a -= 1;
            }
            let mut b = col;
            while b < last && same_class(class(b + 1), kind) {
                b += 1;
            }
            (p(a), p(b))
        }
    }
}

/// The first cell of the character at `col`: a wide character's tail maps to its head.
fn head_of(col: u16, row: &[Cell]) -> u16 {
    match row.get(usize::from(col)) {
        Some(c) if c.width == Width::SpacerTail && col > 0 => col - 1,
        _ => col,
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Class {
    Blank,
    Stop,
    Word,
    /// The tail of a wide character: it takes the class of its head.
    Tail,
}

fn class_of(cell: &Cell) -> Class {
    if cell.width == Width::SpacerTail {
        return Class::Tail;
    }
    match cell.text.chars().next() {
        None => Class::Blank,
        Some(c) if c.is_whitespace() => Class::Blank,
        Some(c) if WORD_STOPS.contains(c) => Class::Stop,
        Some(_) => Class::Word,
    }
}

fn same_class(a: Class, kind: Class) -> bool {
    match kind {
        // A click on a stop character selects only it.
        Class::Stop => false,
        _ => a == kind || a == Class::Tail,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn row(text: &str) -> Vec<Cell> {
        let mut cells = Vec::new();
        for c in text.chars() {
            let wide = c == '中';
            cells.push(Cell {
                text: if c == ' ' {
                    String::new()
                } else {
                    c.to_string()
                },
                width: if wide { Width::Wide } else { Width::Narrow },
                ..Default::default()
            });
            if wide {
                cells.push(Cell {
                    width: Width::SpacerTail,
                    ..Default::default()
                });
            }
        }
        cells
    }

    fn at(row: u64, col: u16) -> Point {
        Point { row, col }
    }

    #[test]
    fn a_drag_selects_in_reading_order() {
        let r = row("hello world");
        let mut s = Selection::new(at(5, 6), Unit::Cell, &r);
        assert!(s.is_single_cell());
        s.extend(at(5, 10), &r);
        assert_eq!((s.start, s.end), (at(5, 6), at(5, 10)));
        assert_eq!(s.text([(5, &r[..])]), "world");
        // Dragging back past the anchor flips the ends.
        s.extend(at(5, 0), &r);
        assert_eq!((s.start, s.end), (at(5, 0), at(5, 6)));
        assert_eq!(s.text([(5, &r[..])]), "hello w");
        assert!(s.contains(at(5, 3)) && !s.contains(at(5, 7)) && !s.contains(at(4, 3)));
    }

    #[test]
    fn the_span_of_a_row_holds_exactly_the_cells_that_contains_holds() {
        let r = row("one two three four");
        let mut s = Selection::new(at(3, 5), Unit::Cell, &r);
        s.extend(at(6, 2), &r);
        let cols = 10;
        for line in 0..9 {
            let span = s.span(line, cols);
            for col in 0..cols {
                let in_span = span.is_some_and(|(a, b)| a <= col && col <= b);
                assert_eq!(in_span, s.contains(at(line, col)), "row {line} col {col}");
            }
        }
        assert_eq!(s.span(4, cols), Some((0, 9)));
    }

    #[test]
    fn a_selection_over_rows_joins_lines_and_drops_trailing_blanks() {
        let rows = [row("one   "), row("two"), row("three")];
        let mut s = Selection::new(at(10, 1), Unit::Cell, &rows[0]);
        s.extend(at(12, 2), &rows[2]);
        let text = s.text(
            rows.iter()
                .enumerate()
                .map(|(i, r)| (10 + i as u64, &r[..])),
        );
        assert_eq!(text, "ne\ntwo\nthr");
        assert!(s.contains(at(11, 200)), "a middle row is whole");
    }

    #[test]
    fn a_double_click_selects_a_word_and_a_drag_adds_words() {
        let r = row("ls -la /tmp/x (done)");
        let s = Selection::new(at(0, 9), Unit::Word, &r);
        assert_eq!(s.text([(0, &r[..])]), "/tmp/x");
        let mut s = Selection::new(at(0, 0), Unit::Word, &r);
        assert_eq!(s.text([(0, &r[..])]), "ls");
        s.extend(at(0, 4), &r);
        assert_eq!(s.text([(0, &r[..])]), "ls -la");
        let paren = Selection::new(at(0, 14), Unit::Word, &r);
        assert_eq!(paren.text([(0, &r[..])]), "(");
    }

    #[test]
    fn a_triple_click_selects_the_line() {
        let r = row("  indented line  ");
        let s = Selection::new(at(3, 5), Unit::Line, &r);
        assert_eq!(s.text([(3, &r[..])]), "  indented line");
        assert_eq!(Unit::from_clicks(3), Unit::Line);
        assert_eq!(Unit::from_clicks(2), Unit::Word);
        assert_eq!(Unit::from_clicks(1), Unit::Cell);
    }

    #[test]
    fn wide_characters_select_whole() {
        let r = row("a中b");
        // Column 2 is the tail of 中.
        let s = Selection::new(at(0, 2), Unit::Cell, &r);
        assert_eq!((s.start.col, s.end.col), (1, 2));
        assert_eq!(s.text([(0, &r[..])]), "中");
        let w = Selection::new(at(0, 2), Unit::Word, &r);
        assert_eq!(w.text([(0, &r[..])]), "a中b");
    }

    #[test]
    fn a_click_on_a_wide_character_is_a_single_cell() {
        let r = row("a中b");
        let s = Selection::new(at(0, 1), Unit::Cell, &r);
        assert!(s.is_single_cell(), "a click with no drag selects nothing");
    }

    #[test]
    fn rows_outside_the_view_are_left_out() {
        let rows = [row("seen")];
        let mut s = Selection::new(at(0, 0), Unit::Cell, &rows[0]);
        s.extend(at(2, 3), &rows[0]);
        assert_eq!(s.text([(1, &rows[0][..])]), "seen");
    }
}
