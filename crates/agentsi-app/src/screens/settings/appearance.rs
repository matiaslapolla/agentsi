//! The Appearance section: density, corners and text size, with a sample row.
//!
//! A click applies the choice to the whole app at once and saves it in `appearance.toml`
//! (see `crate::appearance`).

use agentsi_ui::appearance::{ColorTheme, Corners, Density, Motion, TextSize};
use agentsi_ui::components::{Button, ListRow, status_tag};
use agentsi_ui::tokens::{Status, control, size, space};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;

use super::{SettingsScreen, pair};
use crate::appearance::Look;
use crate::screens::parts::section;

/// The id of the element that holds the sample row, for the tests.
pub(super) const SAMPLE: &str = "appearance-sample";

impl SettingsScreen {
    /// One row of three buttons. The current choice has the accent fill.
    fn choices<T: Copy + PartialEq + 'static>(
        &self,
        group: &'static str,
        all: [T; 3],
        current: T,
        label: fn(T) -> &'static str,
        set: fn(&mut Look, T),
        cx: &mut Context<Self>,
    ) -> Div {
        div()
            .debug_selector(|| format!("appearance-group:{group}"))
            .flex()
            .items_center()
            .gap(space(2))
            .children(all.into_iter().enumerate().map(|(i, choice)| {
                let button = Button::new((group, i))
                    .small()
                    .label(label(choice))
                    .on_click(cx.listener(move |this, _, _, cx| {
                        let result = crate::appearance::change(cx, |a| set(a, choice));
                        this.appearance_error = result.err();
                        cx.notify();
                    }));
                if choice == current {
                    button.primary()
                } else {
                    button.secondary()
                }
            }))
    }

    pub(super) fn appearance_section(&self, cx: &mut Context<Self>) -> Div {
        let now = crate::appearance::current(cx);
        let note = self
            .appearance_error
            .clone()
            .or_else(|| crate::appearance::note(cx));
        div()
            .flex()
            .flex_col()
            .gap(space(3))
            .child(section("Appearance", None))
            .child(pair(
                "Density",
                self.choices(
                    "appearance-density",
                    Density::ALL,
                    now.appearance.density,
                    Density::label,
                    |a, v| a.appearance.density = v,
                    cx,
                ),
            ))
            .child(pair(
                "Corners",
                self.choices(
                    "appearance-corners",
                    Corners::ALL,
                    now.appearance.corners,
                    Corners::label,
                    |a, v| a.appearance.corners = v,
                    cx,
                ),
            ))
            .child(pair(
                "Text size",
                self.choices(
                    "appearance-text",
                    TextSize::ALL,
                    now.appearance.text,
                    TextSize::label,
                    |a, v| a.appearance.text = v,
                    cx,
                ),
            ))
            .child(pair(
                "Color theme",
                self.choices(
                    "appearance-theme",
                    ColorTheme::ALL,
                    now.theme,
                    ColorTheme::label,
                    |a, v| a.theme = v,
                    cx,
                ),
            ))
            .child(pair(
                "Motion",
                self.choices(
                    "appearance-motion",
                    Motion::ALL,
                    now.motion,
                    Motion::label,
                    |a, v| a.motion = v,
                    cx,
                ),
            ))
            .child(sample())
            .when_some(note, |el, n| {
                el.child(
                    div()
                        .debug_selector(|| "appearance-note".into())
                        .text_size(size::sm())
                        .text_color(Status::Warn.color())
                        .child(n),
                )
            })
    }
}

/// A sidebar row, a button and a tag in the look that the owner picked.
fn sample() -> Stateful<Div> {
    div()
        .id(SAMPLE)
        .debug_selector(|| SAMPLE.into())
        .flex()
        .items_center()
        .gap(space(3))
        .child(
            div().w(control::SIDEBAR_WIDTH).child(
                ListRow::new("appearance-sample-row", "Product name")
                    .meta("~/dev/product")
                    .selected(true),
            ),
        )
        .child(
            Button::new("appearance-sample-button")
                .secondary()
                .label("Button"),
        )
        .child(status_tag("tag", Status::Ok))
}

#[cfg(test)]
mod tests {
    use agentsi_ui::appearance::{Appearance, Corners, Density, INK_EM, TextSize};
    use agentsi_ui::tokens::{control, size};
    use gpui::{Bounds, Modifiers, Pixels, TestAppContext, VisualTestContext};

    use super::SAMPLE;
    use crate::screens::settings::tests::support::{Setup, setup};

    fn bounds(cx: &mut VisualTestContext, selector: &str) -> Bounds<Pixels> {
        // The tests are short-lived, so a leaked name costs nothing.
        let name: &'static str = Box::leak(selector.to_string().into_boxed_str());
        cx.debug_bounds(name)
            .unwrap_or_else(|| panic!("{selector} is drawn"))
    }

    fn draw(cx: &mut VisualTestContext) {
        cx.update(|window, cx| window.draw(cx).clear(cx));
    }

    fn app_size(cx: &mut VisualTestContext) {
        cx.simulate_resize(gpui::size(control::WINDOW_WIDTH, control::WINDOW_HEIGHT));
        draw(cx);
    }

    fn appearance_file(t: &Setup) -> std::path::PathBuf {
        t.file.with_file_name(crate::appearance::FILE_NAME)
    }

    fn click(cx: &mut VisualTestContext, selector: &str) {
        let b = bounds(cx, selector);
        cx.simulate_click(b.center(), Modifiers::none());
        draw(cx);
    }

    #[gpui::test]
    fn the_default_look_keeps_the_top_of_the_settings_screen_as_before(cx: &mut TestAppContext) {
        let (_t, cx) = setup(cx, None);
        app_size(cx);
        let got = |cx: &mut VisualTestContext, name: &str| {
            let b = bounds(cx, name);
            [
                f32::from(b.origin.x),
                f32::from(b.origin.y),
                f32::from(b.size.width),
            ]
        };
        // x, y and width, measured on the commit before the setting (59b4872). The section
        // comes after the header, so the header and the column keep their place.
        assert_eq!(got(cx, "settings-root"), [0., 0., 1280.]);
        assert_eq!(got(cx, "screen-column"), [200., 0., 880.]);
        assert_eq!(got(cx, "settings-header"), [232., 32., 816.]);
    }

    #[gpui::test]
    fn the_settings_screen_lays_out_in_each_of_the_27_combinations(cx: &mut TestAppContext) {
        let (_t, cx) = setup(cx, None);
        app_size(cx);
        let window = Bounds::new(
            gpui::point(Pixels::ZERO, Pixels::ZERO),
            gpui::size(control::WINDOW_WIDTH, control::WINDOW_HEIGHT),
        );
        let mut row_heights = std::collections::BTreeSet::new();
        for a in Appearance::all() {
            cx.update(|_, cx| a.apply(cx));
            draw(cx);
            let ctx = format!("{a:?}");
            let column = bounds(cx, "screen-column");
            assert!(
                column.left() >= window.left() && column.right() <= window.right(),
                "{ctx}: the column is cut at the side of the window"
            );
            assert!(column.size.width <= control::CONTENT_WIDTH, "{ctx}");
            // The padding of the column on each side.
            let pad = agentsi_ui::tokens::space(8);
            let inner = (column.left() + pad, column.right() - pad);

            // Three groups of three buttons, one under the other, inside the column.
            let mut tops = Vec::new();
            for group in ["density", "corners", "text"] {
                let g = bounds(cx, &format!("appearance-group:appearance-{group}"));
                assert!(
                    g.left() >= inner.0 && g.right() <= inner.1,
                    "{ctx}: {group} group"
                );
                let mut last_right = g.left();
                for i in 0..3 {
                    let b = bounds(cx, &format!("button:appearance-{group}-{i}"));
                    assert_eq!(b.size.height, control::height_sm(), "{ctx}: {group} {i}");
                    assert!(b.left() >= last_right, "{ctx}: two buttons overlap");
                    assert!(b.right() <= g.right(), "{ctx}: a button leaves its group");
                    // The label is a few characters wide: the button is wider than it is high.
                    assert!(b.size.width > b.size.height, "{ctx}: {group} {i}");
                    last_right = b.right();
                }
                tops.push(g.bottom());
            }
            assert!(
                tops.windows(2).all(|w| w[0] < w[1]),
                "{ctx}: groups overlap"
            );

            // The sample: a sidebar row, a button and a tag in one line.
            let sample = bounds(cx, SAMPLE);
            assert!(
                sample.left() >= inner.0 && sample.right() <= inner.1,
                "{ctx}: sample"
            );
            assert!(
                sample.top() >= *tops.last().unwrap(),
                "{ctx}: sample under the groups"
            );
            let row = bounds(cx, "list-row:appearance-sample-row");
            row_heights.insert(f32::from(row.size.height).to_bits());
            assert_eq!(row.size.height, control::row(), "{ctx}");
            assert_eq!(row.size.width, control::SIDEBAR_WIDTH, "{ctx}");
            let button = bounds(cx, "button:appearance-sample-button");
            assert_eq!(button.size.height, control::height_md(), "{ctx}");
            assert!(
                row.right() <= button.left(),
                "{ctx}: the sample row and button overlap"
            );
            assert!(button.right() <= sample.right(), "{ctx}: the sample is cut");
            // The two lines of the sample row stay inside it.
            let text = bounds(cx, "list-row:appearance-sample-row:text");
            let ink = (f32::from(size::sm()) + f32::from(size::xs())) * INK_EM;
            let mid = f32::from(text.top() + text.size.height / 2.);
            assert!(mid - ink / 2. >= f32::from(row.top()), "{ctx}");
            assert!(mid + ink / 2. <= f32::from(row.bottom()), "{ctx}");
        }
        assert_eq!(row_heights.len(), 3, "the sample followed the density");
    }

    #[gpui::test]
    fn a_click_applies_the_choice_at_once_and_a_restart_reads_it(cx: &mut TestAppContext) {
        let (t, cx) = setup(cx, None);
        app_size(cx);
        let file = appearance_file(&t);
        assert!(!file.exists(), "no file until the owner picks");
        let before = bounds(cx, "list-row:appearance-sample-row").size.height;
        assert_eq!(before, control::row());

        // Compact (button 0 of density), Round (2 of corners), Large (2 of text).
        click(cx, "button:appearance-density-0");
        click(cx, "button:appearance-corners-2");
        click(cx, "button:appearance-text-2");
        let want = Appearance {
            density: Density::Compact,
            corners: Corners::Round,
            text: TextSize::Large,
        };
        assert_eq!(cx.update(|_, cx| Appearance::get(cx)), want);
        assert_eq!(cx.update(|_, _| agentsi_ui::appearance::current()), want);
        let after = bounds(cx, "list-row:appearance-sample-row").size.height;
        assert_eq!(f32::from(after), 32., "the drawn row is compact at once");
        assert_eq!(
            f32::from(bounds(cx, "button:appearance-text-0").size.height),
            24.,
            "a small button is compact"
        );

        // The file holds the choice, and a new start reads it.
        assert_eq!(
            crate::appearance::read(&file),
            (
                crate::appearance::Look {
                    appearance: want,
                    ..Default::default()
                },
                None
            )
        );
        cx.update(|_, cx| {
            Appearance::default().apply(cx);
            assert_eq!(Appearance::get(cx), Appearance::default());
            crate::appearance::install(cx, Some(file.clone()));
            assert_eq!(Appearance::get(cx), want);
        });
        draw(cx);
        assert_eq!(
            f32::from(bounds(cx, "list-row:appearance-sample-row").size.height),
            32.
        );
    }

    #[gpui::test]
    fn the_theme_and_motion_rows_apply_save_and_read_back(cx: &mut TestAppContext) {
        use agentsi_ui::appearance::{ColorTheme, Motion};
        use agentsi_ui::tokens::{Palette, current_palette};
        let (t, cx) = setup(cx, None);
        app_size(cx);
        assert_eq!(current_palette(), Palette::DARK, "no file gives Dark");
        click(cx, "button:appearance-theme-2");
        assert_eq!(current_palette(), Palette::LIGHT);
        click(cx, "button:appearance-motion-1");
        assert_eq!(cx.update(|_, cx| Motion::get(cx)), Motion::Reduced);
        assert!(cx.update(|_, cx| cx.reduce_motion()));
        click(cx, "button:appearance-motion-2");
        assert!(!cx.update(|_, cx| cx.reduce_motion()));
        let file = appearance_file(&t);
        let (look, note) = crate::appearance::read(&file);
        assert_eq!(
            (look.theme, look.motion, note),
            (ColorTheme::Light, Motion::Full, None)
        );
        click(cx, "button:appearance-theme-1");
        assert_eq!(current_palette(), Palette::DARK);
    }

    #[gpui::test]
    fn an_invalid_file_gives_the_defaults_and_one_note(cx: &mut TestAppContext) {
        let (t, cx) = setup(cx, None);
        app_size(cx);
        assert!(
            cx.debug_bounds("appearance-note").is_none(),
            "no note at first"
        );
        let file = appearance_file(&t);
        std::fs::write(&file, "density = \"huge\"\ncorners = round\n").unwrap();
        cx.update(|_, cx| crate::appearance::install(cx, Some(file.clone())));
        t.screen.update(cx, |_, cx| cx.notify());
        draw(cx);
        assert_eq!(
            cx.update(|_, cx| Appearance::get(cx)),
            Appearance::default()
        );
        let note = cx
            .update(|_, cx| crate::appearance::note(cx))
            .expect("a note");
        assert!(note.contains("does not parse"), "{note}");
        assert!(
            cx.debug_bounds("appearance-note").is_some(),
            "the screen shows the note"
        );

        // A value that is not a choice gives the default for it, with one note, and the rest stays.
        std::fs::write(&file, "density = \"huge\"\ncorners = \"round\"\n").unwrap();
        cx.update(|_, cx| crate::appearance::install(cx, Some(file.clone())));
        let (a, note) = cx.update(|_, cx| (Appearance::get(cx), crate::appearance::note(cx)));
        assert_eq!((a.density, a.corners), (Density::Normal, Corners::Round));
        assert!(note.unwrap().contains("not a choice"));

        // A save on a file that does not parse is refused and keeps the file, and the note says why.
        std::fs::write(&file, "density = = x").unwrap();
        cx.update(|_, cx| crate::appearance::install(cx, Some(file.clone())));
        draw(cx);
        click(cx, "button:appearance-density-0");
        assert_eq!(std::fs::read_to_string(&file).unwrap(), "density = = x");
        assert_eq!(
            cx.update(|_, cx| Appearance::get(cx).density),
            Density::Compact,
            "the look still applies for this run"
        );
        assert!(cx.debug_bounds("appearance-note").is_some());
    }
}
