//! A copy of the visible screen, for drawing.

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Rgb {
    pub r: u8,
    pub g: u8,
    pub b: u8,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Underline {
    #[default]
    None,
    Single,
    Double,
    Curly,
    Dotted,
    Dashed,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct CellStyle {
    pub bold: bool,
    pub italic: bool,
    pub faint: bool,
    pub blink: bool,
    pub inverse: bool,
    pub invisible: bool,
    pub strikethrough: bool,
    pub overline: bool,
    pub underline: Underline,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Width {
    #[default]
    Narrow,
    /// The first of two cells. The next cell is `SpacerTail`.
    Wide,
    SpacerTail,
    SpacerHead,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Cell {
    /// The grapheme cluster. Empty for a blank cell.
    pub text: String,
    pub width: Width,
    /// `None` means the default foreground of [`Colors`].
    pub fg: Option<Rgb>,
    /// `None` means the default background of [`Colors`].
    pub bg: Option<Rgb>,
    pub style: CellStyle,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum CursorShape {
    Bar,
    #[default]
    Block,
    Underline,
    BlockHollow,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Cursor {
    pub x: u16,
    pub y: u16,
    pub on_wide_tail: bool,
    pub visible: bool,
    pub blinking: bool,
    pub shape: CursorShape,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Colors {
    pub background: Rgb,
    pub foreground: Rgb,
    pub cursor: Option<Rgb>,
}

/// The visible rows of the terminal at one moment.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Grid {
    pub cols: u16,
    pub rows: Vec<Vec<Cell>>,
    pub colors: Colors,
    /// `None` when the cursor is outside the viewport.
    pub cursor: Option<Cursor>,
}

impl Grid {
    /// The text of each row, without trailing blanks. A wide cell counts once.
    pub fn text_lines(&self) -> Vec<String> {
        self.rows
            .iter()
            .map(|row| {
                let mut line = String::new();
                for cell in row {
                    match cell.width {
                        Width::SpacerTail | Width::SpacerHead => {}
                        _ if cell.text.is_empty() => line.push(' '),
                        _ => line.push_str(&cell.text),
                    }
                }
                line.truncate(line.trim_end().len());
                line
            })
            .collect()
    }
}

/// The default colors of a terminal: what SGR 39 and 49 and the 16 ANSI colors give.
/// Colors 16 to 255 keep the standard cube and grey ramp.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Palette {
    pub foreground: Rgb,
    pub background: Rgb,
    pub cursor: Option<Rgb>,
    /// Normal colors 0 to 7, then bright colors 8 to 15.
    pub ansi: [Rgb; 16],
}

/// A move of the viewport over the scrollback.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Scroll {
    Top,
    /// Back to the live screen.
    Bottom,
    /// Rows to move. Negative moves up into the history.
    Lines(isize),
}

/// Where the viewport is, in rows.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Scrollbar {
    /// All rows: the scrollback and the screen.
    pub total: u64,
    /// The rows above the top of the viewport.
    pub offset: u64,
    /// The rows of the viewport.
    pub len: u64,
}

impl Scrollbar {
    /// True when the viewport shows the live screen.
    pub fn at_bottom(&self) -> bool {
        self.offset + self.len >= self.total
    }
}
