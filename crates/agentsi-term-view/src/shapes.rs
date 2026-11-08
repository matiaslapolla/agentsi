//! Block elements, box drawing and braille characters as shapes, not as font glyphs.
//!
//! A font draws these glyphs with its own metrics, so they leave gaps between cells and rows.
//! Shapes fill the cell exactly. Coordinates are pixels from the top left corner of the cell.

/// One primitive of a character.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Shape {
    /// A filled rectangle. `alpha` multiplies the opacity of the color (the shades use it).
    Rect {
        x: f32,
        y: f32,
        w: f32,
        h: f32,
        alpha: f32,
    },
    /// A stroke along a quadratic curve, for the rounded corners.
    Curve {
        from: (f32, f32),
        ctrl: (f32, f32),
        to: (f32, f32),
        width: f32,
    },
    /// A straight stroke, for the diagonals.
    Line {
        from: (f32, f32),
        to: (f32, f32),
        width: f32,
    },
}

/// True when `c` is drawn with [`shapes`].
pub fn is_shape(c: char) -> bool {
    matches!(c as u32, 0x2500..=0x259F | 0x2800..=0x28FF)
}

/// The shapes of `c` in a cell of `w` × `h` pixels. Empty when `c` is not drawn as shapes.
pub fn shapes(c: char, w: f32, h: f32) -> Vec<Shape> {
    let mut out = Vec::new();
    let cp = c as u32;
    match cp {
        0x2500..=0x257F => box_drawing(cp, w, h, &mut out),
        0x2580..=0x259F => block(cp, w, h, &mut out),
        0x2800..=0x28FF => braille(cp, w, h, &mut out),
        _ => {}
    }
    out
}

/// A rectangle with its edges on whole pixels, so neighbouring cells meet without seams.
fn rect(out: &mut Vec<Shape>, x0: f32, y0: f32, x1: f32, y1: f32, alpha: f32) {
    let (x0, y0, x1, y1) = (x0.round(), y0.round(), x1.round(), y1.round());
    if x1 > x0 && y1 > y0 {
        out.push(Shape::Rect {
            x: x0,
            y: y0,
            w: x1 - x0,
            h: y1 - y0,
            alpha,
        });
    }
}

// ---- Block elements, U+2580 to U+259F.

fn block(cp: u32, w: f32, h: f32, out: &mut Vec<Shape>) {
    // Each part is (x0, y0, x1, y1) as fractions of the cell.
    let parts: &[(f32, f32, f32, f32)] = match cp {
        0x2580 => &[(0., 0., 1., 0.5)],
        0x2581..=0x2587 => {
            let top = 1. - (cp - 0x2580) as f32 / 8.;
            return rect(out, 0., h * top, w, h, 1.);
        }
        0x2588 => &[(0., 0., 1., 1.)],
        0x2589..=0x258F => {
            let right = (8 - (cp - 0x2588)) as f32 / 8.;
            return rect(out, 0., 0., w * right, h, 1.);
        }
        0x2590 => &[(0.5, 0., 1., 1.)],
        0x2591..=0x2593 => {
            let alpha = (cp - 0x2590) as f32 / 4.;
            return rect(out, 0., 0., w, h, alpha);
        }
        0x2594 => &[(0., 0., 1., 0.125)],
        0x2595 => &[(0.875, 0., 1., 1.)],
        0x2596 => &[(0., 0.5, 0.5, 1.)],
        0x2597 => &[(0.5, 0.5, 1., 1.)],
        0x2598 => &[(0., 0., 0.5, 0.5)],
        0x2599 => &[(0., 0., 0.5, 1.), (0.5, 0.5, 1., 1.)],
        0x259A => &[(0., 0., 0.5, 0.5), (0.5, 0.5, 1., 1.)],
        0x259B => &[(0., 0., 1., 0.5), (0., 0.5, 0.5, 1.)],
        0x259C => &[(0., 0., 1., 0.5), (0.5, 0.5, 1., 1.)],
        0x259D => &[(0.5, 0., 1., 0.5)],
        0x259E => &[(0.5, 0., 1., 0.5), (0., 0.5, 0.5, 1.)],
        _ => &[(0.5, 0., 1., 1.), (0., 0.5, 0.5, 1.)], // U+259F
    };
    for &(x0, y0, x1, y1) in parts {
        rect(out, w * x0, h * y0, w * x1, h * y1, 1.);
    }
}

// ---- Braille, U+2800 to U+28FF.

fn braille(cp: u32, w: f32, h: f32, out: &mut Vec<Shape>) {
    // Bit order of the Unicode block: dots 1-3 down the left column, 4-6 down the right, then 7 and 8.
    const DOTS: [(u8, u8); 8] = [
        (0, 0),
        (0, 1),
        (0, 2),
        (1, 0),
        (1, 1),
        (1, 2),
        (0, 3),
        (1, 3),
    ];
    let bits = cp - 0x2800;
    let size = (w / 4.).round().max(1.);
    for (i, &(col, row)) in DOTS.iter().enumerate() {
        if bits & (1 << i) != 0 {
            let cx = w * (0.25 + 0.5 * f32::from(col));
            let cy = h * (0.125 + 0.25 * f32::from(row));
            let (x0, y0) = ((cx - size / 2.).round(), (cy - size / 2.).round());
            rect(out, x0, y0, x0 + size, y0 + size, 1.);
        }
    }
}

// ---- Box drawing, U+2500 to U+257F.

/// The weight of one arm of a box drawing character.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Arm {
    None,
    Light,
    Heavy,
    Double,
}

/// The arms of U+2500 to U+257F: up, right, down, left. `l` light, `h` heavy, `d` double, `.` none.
/// The dashes, the arcs and the diagonals have `....` here and their own code.
const ARMS: [&str; 128] = [
    ".l.l", ".h.h", "l.l.", "h.h.", "....", "....", "....", "....", // 2500
    "....", "....", "....", "....", ".ll.", ".hl.", ".lh.", ".hh.", // 2508
    "..ll", "..lh", "..hl", "..hh", "ll..", "lh..", "hl..", "hh..", // 2510
    "l..l", "l..h", "h..l", "h..h", "lll.", "lhl.", "hll.", "llh.", // 2518
    "hlh.", "hhl.", "lhh.", "hhh.", "l.ll", "l.lh", "h.ll", "l.hl", // 2520
    "h.hl", "h.lh", "l.hh", "h.hh", ".lll", ".llh", ".hll", ".hlh", // 2528
    ".lhl", ".lhh", ".hhl", ".hhh", "ll.l", "ll.h", "lh.l", "lh.h", // 2530
    "hl.l", "hl.h", "hh.l", "hh.h", "llll", "lllh", "lhll", "lhlh", // 2538
    "hlll", "llhl", "hlhl", "hllh", "hhll", "llhh", "lhhl", "hhlh", // 2540
    "lhhh", "hlhh", "hhhl", "hhhh", "....", "....", "....", "....", // 2548
    ".d.d", "d.d.", ".dl.", ".ld.", ".dd.", "..ld", "..dl", "..dd", // 2550
    "ld..", "dl..", "dd..", "l..d", "d..l", "d..d", "ldl.", "dld.", // 2558
    "ddd.", "l.ld", "d.dl", "d.dd", ".dld", ".ldl", ".ddd", "ld.d", // 2560
    "dl.l", "dd.d", "ldld", "dldl", "dddd", "....", "....", "....", // 2568
    "....", "....", "....", "....", "...l", "l...", ".l..", "..l.", // 2570
    "...h", "h...", ".h..", "..h.", ".h.l", "l.h.", ".l.h", "h.l.", // 2578
];

fn arms_of(cp: u32) -> [Arm; 4] {
    let mut arms = [Arm::None; 4];
    for (slot, b) in arms.iter_mut().zip(ARMS[(cp - 0x2500) as usize].bytes()) {
        *slot = match b {
            b'l' => Arm::Light,
            b'h' => Arm::Heavy,
            b'd' => Arm::Double,
            _ => Arm::None,
        };
    }
    arms
}

/// The width of a light line in a cell of width `w`. Underlines use it too.
pub fn line_width(w: f32) -> f32 {
    widths(w).0
}

/// Line widths for a cell of width `w`: the light line, and the heavy line.
fn widths(w: f32) -> (f32, f32) {
    let light = (w / 8.).round().max(1.);
    (light, light * 2.)
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Dir {
    Up,
    Right,
    Down,
    Left,
}

impl Dir {
    const ALL: [Dir; 4] = [Dir::Up, Dir::Right, Dir::Down, Dir::Left];
    fn vertical(self) -> bool {
        matches!(self, Dir::Up | Dir::Down)
    }
    fn opposite(self) -> Dir {
        match self {
            Dir::Up => Dir::Down,
            Dir::Right => Dir::Left,
            Dir::Down => Dir::Up,
            Dir::Left => Dir::Right,
        }
    }
    /// The two arms across this one: the one on the negative side first (left, or up).
    fn across(self) -> (Dir, Dir) {
        if self.vertical() {
            (Dir::Left, Dir::Right)
        } else {
            (Dir::Up, Dir::Down)
        }
    }
}

struct Geometry {
    w: f32,
    h: f32,
    light: f32,
    heavy: f32,
    /// The center of a light line, on each axis, on whole pixels.
    cx: f32,
    cy: f32,
}

impl Geometry {
    fn new(w: f32, h: f32) -> Geometry {
        let (light, heavy) = widths(w);
        Geometry {
            w,
            h,
            light,
            heavy,
            cx: ((w - light) / 2.).round() + light / 2.,
            cy: ((h - light) / 2.).round() + light / 2.,
        }
    }

    /// Half the width that an arm takes across its axis.
    fn half(&self, arm: Arm) -> f32 {
        match arm {
            Arm::None => 0.,
            Arm::Light => self.light / 2.,
            Arm::Heavy => self.heavy / 2.,
            Arm::Double => self.light * 1.5,
        }
    }

    /// A line of width `width` along the arm `dir`, from `start` past the center to the cell edge.
    /// `offset` moves it across the axis. A negative `start` begins before the center.
    fn arm_line(&self, out: &mut Vec<Shape>, dir: Dir, start: f32, offset: f32, width: f32) {
        let (cx, cy) = (self.cx, self.cy);
        let half = width / 2.;
        match dir {
            Dir::Up => rect(
                out,
                cx + offset - half,
                0.,
                cx + offset + half,
                cy - start,
                1.,
            ),
            Dir::Down => rect(
                out,
                cx + offset - half,
                cy + start,
                cx + offset + half,
                self.h,
                1.,
            ),
            Dir::Left => rect(
                out,
                0.,
                cy + offset - half,
                cx - start,
                cy + offset + half,
                1.,
            ),
            Dir::Right => rect(
                out,
                cx + start,
                cy + offset - half,
                self.w,
                cy + offset + half,
                1.,
            ),
        }
    }
}

fn box_drawing(cp: u32, w: f32, h: f32, out: &mut Vec<Shape>) {
    let g = Geometry::new(w, h);
    match cp {
        0x2504..=0x250B => {
            let i = cp - 0x2504;
            let heavy = i % 2 == 1;
            let vertical = (i / 2) % 2 == 1;
            let n = if i < 4 { 3 } else { 4 };
            return dashes(&g, out, n, vertical, heavy);
        }
        0x254C..=0x254F => {
            let i = cp - 0x254C;
            return dashes(&g, out, 2, i >= 2, i % 2 == 1);
        }
        0x256D..=0x2570 => return arc(&g, out, cp),
        0x2571..=0x2573 => {
            let width = g.light;
            if cp != 0x2572 {
                out.push(Shape::Line {
                    from: (w, 0.),
                    to: (0., h),
                    width,
                });
            }
            if cp != 0x2571 {
                out.push(Shape::Line {
                    from: (0., 0.),
                    to: (w, h),
                    width,
                });
            }
            return;
        }
        _ => {}
    }
    let arms = arms_of(cp);
    let arm = |d: Dir| arms[d as usize];
    for dir in Dir::ALL {
        let (neg, pos) = dir.across();
        match arm(dir) {
            Arm::None => {}
            a @ (Arm::Light | Arm::Heavy) => {
                // Reach back over the arms across, so the joint has no gap.
                let start = -g.half(arm(neg)).max(g.half(arm(pos)));
                let width = if a == Arm::Heavy { g.heavy } else { g.light };
                g.arm_line(out, dir, start, 0., width);
            }
            Arm::Double => {
                for (side, other, offset) in [(neg, pos, -g.light), (pos, neg, g.light)] {
                    let start = match (arm(side), arm(dir.opposite()), arm(other)) {
                        // A double arm on this side: meet its near line in a square corner.
                        (Arm::Double, _, _) => g.light / 2.,
                        // A single arm on this side: end on it.
                        (Arm::Light | Arm::Heavy, _, _) => 0.,
                        // A double arm straight on: run on into its line.
                        (Arm::None, Arm::Double, _) => 0.,
                        // Only the other side has an arm: this is the outer line of a corner.
                        (Arm::None, _, o) if o != Arm::None => -g.half(o),
                        _ => 0.,
                    };
                    g.arm_line(out, dir, start, offset, g.light);
                }
            }
        }
    }
}

fn dashes(g: &Geometry, out: &mut Vec<Shape>, n: u32, vertical: bool, heavy: bool) {
    let width = if heavy { g.heavy } else { g.light };
    let len = if vertical { g.h } else { g.w };
    let seg = len / n as f32;
    let gap = (seg / 3.).round().max(1.);
    for i in 0..n {
        let a = i as f32 * seg + gap / 2.;
        let b = (i + 1) as f32 * seg - gap / 2.;
        if vertical {
            rect(out, g.cx - width / 2., a, g.cx + width / 2., b, 1.);
        } else {
            rect(out, a, g.cy - width / 2., b, g.cy + width / 2., 1.);
        }
    }
}

/// The rounded corners U+256D to U+2570: a straight piece on each arm and a curve between them.
fn arc(g: &Geometry, out: &mut Vec<Shape>, cp: u32) {
    let (down, right) = match cp {
        0x256D => (true, true),
        0x256E => (true, false),
        0x256F => (false, false),
        _ => (false, true),
    };
    let (cx, cy, t) = (g.cx, g.cy, g.light);
    // The radius that reaches the cell edge on the horizontal arm, and fits on the vertical arm.
    let r = (if right { g.w - cx } else { cx }).min(if down { g.h - cy } else { cy });
    let vy = if down { cy + r } else { cy - r };
    let hx = if right { cx + r } else { cx - r };
    if down {
        rect(out, cx - t / 2., vy, cx + t / 2., g.h, 1.);
    } else {
        rect(out, cx - t / 2., 0., cx + t / 2., vy, 1.);
    }
    if right {
        rect(out, hx, cy - t / 2., g.w, cy + t / 2., 1.);
    } else {
        rect(out, 0., cy - t / 2., hx, cy + t / 2., 1.);
    }
    out.push(Shape::Curve {
        from: (cx, vy),
        ctrl: (cx, cy),
        to: (hx, cy),
        width: t,
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    const W: f32 = 8.;
    const H: f32 = 16.;

    fn rects(c: char) -> Vec<(f32, f32, f32, f32, f32)> {
        shapes(c, W, H)
            .into_iter()
            .map(|s| match s {
                Shape::Rect { x, y, w, h, alpha } => (x, y, w, h, alpha),
                other => panic!("{c:?} gave {other:?}"),
            })
            .collect()
    }

    /// The pixels that the rectangles of `c` cover, as a W × H mask.
    fn mask(c: char) -> Vec<Vec<bool>> {
        let mut m = vec![vec![false; W as usize]; H as usize];
        for (x, y, w, h, _) in rects(c) {
            for row in m.iter_mut().skip(y as usize).take(h as usize) {
                for px in row.iter_mut().skip(x as usize).take(w as usize) {
                    *px = true;
                }
            }
        }
        m
    }

    #[test]
    fn the_shape_ranges_are_block_box_and_braille() {
        assert!(is_shape('─') && is_shape('█') && is_shape('⣿') && is_shape('╭'));
        assert!(!is_shape('a') && !is_shape('→') && !is_shape('⏺'));
        assert!(shapes('a', W, H).is_empty());
        for cp in (0x2500..=0x259F).chain(0x2801..=0x28FF) {
            let c = char::from_u32(cp).unwrap();
            assert!(!shapes(c, W, H).is_empty(), "U+{cp:04X} draws nothing");
        }
        assert!(
            shapes('\u{2800}', W, H).is_empty(),
            "the blank braille cell"
        );
    }

    #[test]
    fn blocks_fill_their_part_of_the_cell() {
        assert_eq!(rects('█'), vec![(0., 0., W, H, 1.)]);
        assert_eq!(rects('▀'), vec![(0., 0., W, H / 2., 1.)]);
        assert_eq!(rects('▄'), vec![(0., H / 2., W, H / 2., 1.)]);
        assert_eq!(rects('▌'), vec![(0., 0., W / 2., H, 1.)]);
        assert_eq!(rects('▐'), vec![(W / 2., 0., W / 2., H, 1.)]);
        assert_eq!(rects('▁'), vec![(0., 14., W, 2., 1.)]);
        assert_eq!(rects('▏'), vec![(0., 0., 1., H, 1.)]);
        assert_eq!(rects('▒'), vec![(0., 0., W, H, 0.5)]);
        assert_eq!(rects('▚').len(), 2);
    }

    #[test]
    fn eighth_blocks_grow_one_step_at_a_time() {
        let heights: Vec<f32> = ('▁'..='█').map(|c| rects(c)[0].3).collect();
        assert_eq!(heights, vec![2., 4., 6., 8., 10., 12., 14., 16.]);
    }

    #[test]
    fn braille_dots_follow_the_unicode_bit_order() {
        // Dot 1 is top left, dot 4 top right, dot 7 bottom left, dot 8 bottom right.
        let one = rects('\u{2801}');
        assert_eq!(one.len(), 1);
        let (x, y, w, h, _) = one[0];
        assert!(x < W / 2. && y < H / 4., "{one:?}");
        assert_eq!((w, h), (2., 2.));
        let four = rects('\u{2808}')[0];
        assert!(four.0 >= W / 2. && four.1 < H / 4.);
        let seven = rects('\u{2840}')[0];
        assert!(seven.0 < W / 2. && seven.1 >= 3. * H / 4.);
        let eight = rects('\u{2880}')[0];
        assert!(eight.0 >= W / 2. && eight.1 >= 3. * H / 4.);
        assert_eq!(rects('⣿').len(), 8);
        // The dots do not touch each other.
        let all = rects('⣿');
        for (i, a) in all.iter().enumerate() {
            for b in &all[i + 1..] {
                let apart =
                    a.0 + a.2 <= b.0 || b.0 + b.2 <= a.0 || a.1 + a.3 <= b.1 || b.1 + b.3 <= a.1;
                assert!(apart, "{a:?} touches {b:?}");
            }
        }
    }

    #[test]
    fn straight_lines_cross_the_whole_cell() {
        let h = mask('─');
        assert!(h.iter().filter(|r| r.iter().all(|p| *p)).count() == 1);
        let v = mask('│');
        let cols: Vec<usize> = (0..W as usize)
            .filter(|&x| v.iter().all(|r| r[x]))
            .collect();
        assert_eq!(cols.len(), 1);
        // A heavy line is twice as wide as a light line.
        assert_eq!(rects('━')[0].3, 2. * rects('─')[0].3);
    }

    #[test]
    fn neighbouring_cells_join_without_a_gap() {
        // The right edge of ─ meets the left edge of the next ─ on the same row, and │ meets │ below.
        let row = |c: char| -> Vec<usize> {
            let m = mask(c);
            (0..H as usize)
                .filter(|&y| m[y][0] || m[y][W as usize - 1])
                .collect()
        };
        assert_eq!(row('─'), row('┼'));
        assert_eq!(row('┬'), row('─'));
        let col = |c: char| -> Vec<usize> {
            let m = mask(c);
            (0..W as usize)
                .filter(|&x| m[0][x] || m[H as usize - 1][x])
                .collect()
        };
        assert_eq!(col('│'), col('┼'));
        assert_eq!(col('├'), col('│'));
    }

    #[test]
    fn a_corner_has_no_gap_at_the_joint() {
        let m = mask('┌');
        // The cells right of the corner and below it are lit, and they meet at one pixel.
        let row = (0..H as usize).find(|&y| m[y][W as usize - 1]).unwrap();
        let col = (0..W as usize).find(|&x| m[H as usize - 1][x]).unwrap();
        assert!(m[row][col], "the joint pixel is lit");
        assert!(
            !m[row - 1][col] && !m[row][col - 1],
            "nothing pokes out past the corner"
        );
        // ┐, └ and ┘ also reach their two edges.
        for c in ['┐', '└', '┘', '╋', '┣'] {
            assert!(!rects(c).is_empty());
        }
    }

    #[test]
    fn double_lines_are_two_lines_and_corners_close() {
        let m = mask('═');
        let rows: Vec<usize> = (0..H as usize)
            .filter(|&y| m[y].iter().all(|p| *p))
            .collect();
        assert_eq!(rows.len(), 2);
        assert!(rows[1] - rows[0] >= 2, "a gap between the two lines");
        // ╔: the outer lines meet, and the inner lines meet, at the left and top of the cell.
        let c = mask('╔');
        let (outer, inner) = (rows[0], rows[1]);
        // The first pixel of the lit run that reaches the right edge.
        let run_start = |y: usize| {
            (0..W as usize)
                .rev()
                .take_while(|&x| c[y][x])
                .last()
                .unwrap()
        };
        let (outer_x, inner_x) = (run_start(outer), run_start(inner));
        assert!(outer_x < inner_x);
        assert!(c[H as usize - 1][outer_x] && c[H as usize - 1][inner_x]);
        assert!(
            (outer..H as usize).all(|y| c[y][outer_x]),
            "the outer corner is closed"
        );
        assert!(
            (inner..H as usize).all(|y| c[y][inner_x]),
            "the inner corner is closed"
        );
    }

    #[test]
    fn rounded_corners_are_curves_that_reach_two_edges() {
        // Each corner reaches its vertical edge with a rect and its horizontal edge with a rect or the
        // curve, and no part leaves the cell.
        let reaches = |c: char, x_edge: f32, y_edge: f32| {
            let s = shapes(c, W, H);
            assert!(s.iter().any(|s| matches!(s, Shape::Curve { .. })), "{c:?}");
            let vertical = s.iter().any(|s| match *s {
                Shape::Rect { y, h, .. } => y == y_edge || y + h == y_edge,
                _ => false,
            });
            let horizontal = s.iter().any(|s| match *s {
                Shape::Rect { x, w, .. } => x == x_edge || x + w == x_edge,
                Shape::Curve { to, .. } => to.0 == x_edge,
                _ => false,
            });
            assert!(vertical && horizontal, "{c:?} {s:?}");
            for p in &s {
                if let Shape::Curve { from, to, .. } = *p {
                    for (x, y) in [from, to] {
                        assert!(
                            (0. ..=W).contains(&x) && (0. ..=H).contains(&y),
                            "{c:?} {p:?}"
                        );
                    }
                }
            }
        };
        reaches('╭', W, H);
        reaches('╮', 0., H);
        reaches('╯', 0., 0.);
        reaches('╰', W, 0.);
    }

    #[test]
    fn dashes_and_diagonals() {
        assert_eq!(rects('┄').len(), 3);
        assert_eq!(rects('┊').len(), 4);
        assert_eq!(rects('╌').len(), 2);
        assert!(
            rects('╏').iter().all(|r| r.2 == 2.),
            "heavy vertical dashes"
        );
        assert_eq!(shapes('╳', W, H).len(), 2);
        assert!(matches!(shapes('╱', W, H)[0], Shape::Line { .. }));
    }
}
