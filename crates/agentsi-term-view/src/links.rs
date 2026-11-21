//! The links of a pane: OSC 8 hyperlinks of the grid, and the URLs and `path:line` targets that
//! the URL finder sees in the text. The finder reads only the line under the pointer, so a screen
//! that nobody points at costs nothing.

use std::ops::Range;

use agentsi_vt::{Grid, Width};

/// What a link opens.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Link {
    /// An OSC 8 target, or a `http`, `https`, `file` or `mailto` URL of the text.
    Url(String),
    /// A file path with a line, and maybe a column: `src/main.rs:12:5`. The path is as the text
    /// has it, relative to the folder of the pane or absolute.
    Path {
        path: String,
        line: u32,
        col: Option<u32>,
    },
}

impl Link {
    /// The text of the target, for the tooltip and the clipboard.
    pub fn target(&self) -> String {
        match self {
            Link::Url(u) => u.clone(),
            Link::Path {
                path,
                line,
                col: Some(c),
            } => format!("{path}:{line}:{c}"),
            Link::Path { path, line, .. } => format!("{path}:{line}"),
        }
    }
}

/// The cells of one grid row that a link covers.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Span {
    /// The row of the grid.
    pub row: usize,
    pub col: u16,
    pub cols: u16,
}

/// A link and the cells it covers, top to bottom.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Found {
    pub link: Link,
    pub spans: Vec<Span>,
}

/// One char of a line and the cells it stands on.
#[derive(Clone, Copy)]
struct Place {
    row: usize,
    col: u16,
    cols: u16,
}

/// The link at grid row `row`, column `col`: an OSC 8 hyperlink of the cell, else a link of the
/// text of the line, with its wrapped rows.
pub(crate) fn link_at(grid: &Grid, row: usize, col: u16) -> Option<Found> {
    let cells = grid.rows.get(row)?;
    let wrapped = |r: usize| grid.wrapped.get(r).copied().unwrap_or(false);
    let mut first = row;
    while first > 0 && wrapped(first - 1) {
        first -= 1;
    }
    let mut last = row;
    while wrapped(last) && last + 1 < grid.rows.len() {
        last += 1;
    }
    // The tail of a wide char belongs to the cell before it.
    let col = match cells.get(usize::from(col)) {
        Some(c) if c.width == Width::SpacerTail && col > 0 => col - 1,
        Some(_) => col,
        None => return None,
    };
    if let Some(id) = cells[usize::from(col)].link {
        let uri = grid.link(id)?;
        let mut spans = Vec::new();
        for (r, cells) in grid.rows.iter().enumerate().take(last + 1).skip(first) {
            let mut x = 0;
            while x < cells.len() {
                if cells[x].link != Some(id) {
                    x += 1;
                    continue;
                }
                let start = x;
                while x < cells.len()
                    && (cells[x].link == Some(id) || cells[x].width == Width::SpacerTail)
                {
                    x += 1;
                }
                spans.push(Span {
                    row: r,
                    col: start as u16,
                    cols: (x - start) as u16,
                });
            }
        }
        // Only the run of cells under the pointer: two runs of one target on other rows are
        // two links on screen.
        let spans = connected(spans, row, col, &grid.wrapped, grid.cols);
        return Some(Found {
            link: Link::Url(uri.to_string()),
            spans,
        });
    }
    let mut text = String::new();
    let mut places = Vec::new();
    for (r, cells) in grid.rows.iter().enumerate().take(last + 1).skip(first) {
        for (x, cell) in cells.iter().enumerate() {
            let cols = match cell.width {
                Width::SpacerTail | Width::SpacerHead => continue,
                Width::Wide => 2,
                Width::Narrow => 1,
            };
            let place = Place {
                row: r,
                col: x as u16,
                cols,
            };
            if cell.text.is_empty() {
                text.push(' ');
                places.push(place);
            } else {
                for ch in cell.text.chars() {
                    text.push(ch);
                    places.push(place);
                }
            }
        }
    }
    let at = places
        .iter()
        .position(|p| p.row == row && p.col <= col && col < p.col + p.cols)?;
    let (range, link) = find(&text).into_iter().find(|(r, _)| r.contains(&at))?;
    let mut spans: Vec<Span> = Vec::new();
    for p in &places[range] {
        match spans.last_mut() {
            Some(s) if s.row == p.row && s.col + s.cols == p.col => s.cols += p.cols,
            Some(s) if s.row == p.row && s.col == p.col => {}
            _ => spans.push(Span {
                row: p.row,
                col: p.col,
                cols: p.cols,
            }),
        }
    }
    Some(Found { link, spans })
}

/// The spans that join the span under (`row`, `col`) through the soft wraps.
fn connected(spans: Vec<Span>, row: usize, col: u16, wrapped: &[bool], cols: u16) -> Vec<Span> {
    let Some(at) = spans
        .iter()
        .position(|s| s.row == row && s.col <= col && col < s.col + s.cols)
    else {
        return spans;
    };
    let joins = |a: &Span, b: &Span| {
        b.row == a.row + 1
            && wrapped.get(a.row).copied().unwrap_or(false)
            && a.col + a.cols >= cols
            && b.col == 0
    };
    let mut start = at;
    while start > 0 && joins(&spans[start - 1], &spans[start]) {
        start -= 1;
    }
    let mut end = at;
    while end + 1 < spans.len() && joins(&spans[end], &spans[end + 1]) {
        end += 1;
    }
    spans[start..=end].to_vec()
}

const SCHEMES: [&str; 4] = ["https://", "http://", "file://", "mailto:"];

/// The links of a line of text, as ranges of char indices, left to right.
pub(crate) fn find(line: &str) -> Vec<(Range<usize>, Link)> {
    let chars: Vec<char> = line.chars().collect();
    let mut found = Vec::new();
    let mut i = 0;
    while i < chars.len() {
        if i > 0 && chars[i - 1].is_alphanumeric() {
            i += 1;
            continue;
        }
        let Some(scheme) = SCHEMES.iter().find(|s| starts_with_ci(&chars[i..], s)) else {
            i += 1;
            continue;
        };
        let body = i + scheme.chars().count();
        let mut end = body;
        while end < chars.len() && url_char(chars[end]) {
            end += 1;
        }
        let end = trim_url(&chars[i..end]) + i;
        if end > body {
            found.push((i..end, Link::Url(chars[i..end].iter().collect())));
            i = end;
        } else {
            i = body;
        }
    }
    let urls: Vec<Range<usize>> = found.iter().map(|(r, _)| r.clone()).collect();
    let mut i = 0;
    while i < chars.len() {
        let starts = path_char(chars[i])
            && (i == 0 || !(path_char(chars[i - 1]) || chars[i - 1] == ':'))
            && !urls.iter().any(|r| r.contains(&i));
        if !starts {
            i += 1;
            continue;
        }
        match path_at(&chars, i) {
            Some((end, link)) if !urls.iter().any(|r| r.start < end && i < r.end) => {
                found.push((i..end, link));
                i = end;
            }
            _ => i += 1,
        }
    }
    found.sort_by_key(|(r, _)| r.start);
    found
}

fn starts_with_ci(chars: &[char], prefix: &str) -> bool {
    let mut it = chars.iter();
    prefix
        .chars()
        .all(|p| it.next().is_some_and(|c| c.to_ascii_lowercase() == p))
}

fn url_char(c: char) -> bool {
    !c.is_whitespace()
        && !c.is_control()
        && !matches!(c, '"' | '<' | '>' | '`' | '\\' | '{' | '}' | '|' | '^')
}

/// The length of `url` without its trailing punctuation. A `)` or `]` stays when it closes an
/// opening one of the URL.
fn trim_url(url: &[char]) -> usize {
    let mut end = url.len();
    loop {
        let Some(&last) = url[..end].last() else {
            return end;
        };
        let count = |c: char| url[..end].iter().filter(|x| **x == c).count();
        let cut = match last {
            '.' | ',' | ';' | ':' | '!' | '?' | '\'' | '"' | '*' => true,
            ')' => count('(') < count(')'),
            ']' => count('[') < count(']'),
            _ => false,
        };
        if !cut {
            return end;
        }
        end -= 1;
    }
}

fn path_char(c: char) -> bool {
    c.is_alphanumeric() || matches!(c, '_' | '.' | '/' | '~' | '+' | '@' | '-')
}

/// A `path:line` or `path:line:col` that starts at `i`: its end and its link.
fn path_at(chars: &[char], i: usize) -> Option<(usize, Link)> {
    let mut end = i;
    while end < chars.len() && path_char(chars[end]) {
        end += 1;
    }
    let path: String = chars[i..end].iter().collect();
    let file = path.rsplit('/').next().unwrap_or_default();
    let ext = file.rsplit_once('.').map(|(_, e)| e);
    let named = path.contains('/')
        || ext.is_some_and(|e| e.chars().next().is_some_and(|c| c.is_ascii_alphabetic()));
    if !named || file.is_empty() || file.ends_with('.') {
        return None;
    }
    let number = |from: usize| -> Option<(usize, u32)> {
        if chars.get(from) != Some(&':') {
            return None;
        }
        let mut to = from + 1;
        while to < chars.len() && chars[to].is_ascii_digit() {
            to += 1;
        }
        let n: String = chars[from + 1..to].iter().collect();
        n.parse().ok().map(|n| (to, n))
    };
    let (mut end, line) = number(end)?;
    if line == 0 {
        return None;
    }
    let col = match number(end) {
        Some((to, c)) => {
            end = to;
            Some(c)
        }
        None => None,
    };
    if chars.get(end).is_some_and(|c| c.is_alphanumeric()) {
        return None;
    }
    Some((end, Link::Path { path, line, col }))
}

#[cfg(test)]
mod tests {
    use agentsi_vt::{Cell, Grid};

    use super::*;

    fn targets(line: &str) -> Vec<String> {
        find(line).into_iter().map(|(_, l)| l.target()).collect()
    }

    fn url(s: &str) -> Link {
        Link::Url(s.into())
    }

    #[test]
    fn trailing_punctuation_and_quotes_are_cut() {
        assert_eq!(
            targets("see https://github.com/x/y). and"),
            ["https://github.com/x/y"]
        );
        assert_eq!(
            targets("at https://example.com/a, https://example.com/b."),
            ["https://example.com/a", "https://example.com/b"]
        );
        assert_eq!(
            targets("'https://example.com/q?a=1' \"http://x.org\""),
            ["https://example.com/q?a=1", "http://x.org"]
        );
        assert_eq!(targets("mailto:me@example.com!"), ["mailto:me@example.com"]);
        assert_eq!(targets("file:///tmp/a.txt:"), ["file:///tmp/a.txt"]);
        assert!(targets("https:// nothing").is_empty());
        assert!(targets("xhttps://glued.example").is_empty());
    }

    #[test]
    fn a_closing_parenthesis_of_the_url_stays() {
        assert_eq!(
            targets("(https://en.wikipedia.org/wiki/Rust_(programming_language))"),
            ["https://en.wikipedia.org/wiki/Rust_(programming_language)"]
        );
        assert_eq!(
            targets("[link](https://example.com/a)"),
            ["https://example.com/a"]
        );
        assert_eq!(targets("http://[::1]:8080/x]"), ["http://[::1]:8080/x"]);
    }

    #[test]
    fn paths_with_a_line_and_a_column_are_links() {
        assert_eq!(
            find("  --> src/main.rs:12:5"),
            vec![(
                6..22,
                Link::Path {
                    path: "src/main.rs".into(),
                    line: 12,
                    col: Some(5)
                }
            )]
        );
        assert_eq!(targets("main.rs:3: error"), ["main.rs:3"]);
        assert_eq!(targets("at f (/srv/app/x.js:10:2)"), ["/srv/app/x.js:10:2"]);
        assert_eq!(targets("./a/b:7."), ["./a/b:7"]);
        for not in [
            "12:30:45",
            "127.0.0.1:8080",
            "main.rs",
            "main.rs:0",
            "main.rs:12abc",
            "v1.2:3",
            "a.:4",
        ] {
            assert!(targets(not).is_empty(), "{not} is no link");
        }
    }

    #[test]
    fn a_url_with_a_port_is_one_link_and_not_a_path() {
        assert_eq!(
            find("http://example.com:8080/x.rs:1"),
            vec![(0..30, url("http://example.com:8080/x.rs:1"))]
        );
    }

    fn grid(cols: u16, rows: &[&str], wrapped: &[bool]) -> Grid {
        let mut g = Grid::default();
        g.cols = cols;
        g.wrapped = wrapped.to_vec();
        for r in rows {
            let mut cells: Vec<Cell> = r
                .chars()
                .map(|c| Cell {
                    text: if c == ' ' { String::new() } else { c.into() },
                    ..Cell::default()
                })
                .collect();
            cells.resize(usize::from(cols), Cell::default());
            g.rows.push(cells);
        }
        g
    }

    #[test]
    fn a_url_across_a_soft_wrap_is_one_link_on_both_rows() {
        let g = grid(
            12,
            &["go https://e", "xample.com/a", "next"],
            &[true, false, false],
        );
        let want = Found {
            link: url("https://example.com/a"),
            spans: vec![
                Span {
                    row: 0,
                    col: 3,
                    cols: 9,
                },
                Span {
                    row: 1,
                    col: 0,
                    cols: 12,
                },
            ],
        };
        assert_eq!(link_at(&g, 1, 4), Some(want.clone()));
        assert_eq!(link_at(&g, 0, 3), Some(want));
        assert_eq!(link_at(&g, 0, 1), None);
        assert_eq!(link_at(&g, 2, 1), None);
    }

    #[test]
    fn a_row_without_a_wrap_ends_the_line() {
        let g = grid(12, &["go https://e", "xample.com/a"], &[false, false]);
        assert_eq!(link_at(&g, 1, 4), None);
        assert_eq!(link_at(&g, 0, 5).map(|f| f.link), Some(url("https://e")));
    }

    #[test]
    fn an_osc_8_cell_gives_its_target_and_its_run_of_cells() {
        let mut t = agentsi_vt::Terminal::new(agentsi_vt::Options {
            cols: 30,
            rows: 3,
            scrollback_bytes: 10_000,
        })
        .unwrap();
        t.write(b"a \x1b]8;;https://example.com\x1b\\link\x1b]8;;\x1b\\ b https://x.org");
        let g = t.grid().unwrap();
        assert_eq!(
            link_at(&g, 0, 3),
            Some(Found {
                link: url("https://example.com"),
                spans: vec![Span {
                    row: 0,
                    col: 2,
                    cols: 4
                }],
            })
        );
        assert_eq!(link_at(&g, 0, 1), None);
        assert_eq!(
            link_at(&g, 0, 12).map(|f| f.link),
            Some(url("https://x.org"))
        );
    }
}
