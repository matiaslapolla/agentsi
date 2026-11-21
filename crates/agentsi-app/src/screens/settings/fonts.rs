//! The Fonts section: the family of the text of the app and the family of the terminals.
//!
//! Each picker lists the bundled families first, then the families of the system, with a
//! search field. The monospace filter of the terminal picker reads the family names, because a
//! measure of each family would load each font file. A click applies the family to the whole
//! app at once and saves it in `appearance.toml` (see `crate::appearance`).

use agentsi_ui::appearance::Fonts;
use agentsi_ui::components::{ItemRow, Switch};
use agentsi_ui::tokens::{Status, control, font, line, radius, size, space, text};
use gpui_kit::component::input::{Input, InputEvent, InputState};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;

use super::{SettingsScreen, pair};
use crate::screens::parts::section;

/// The rows that a picker draws at most. The search field finds the others.
const SHOWN: usize = 200;

/// Words that name a monospace family. The list is short on purpose: a family that it misses
/// shows with the filter off.
const MONO_WORDS: [&str; 10] = [
    "mono", "code", "courier", "consol", "menlo", "monaco", "fixed", "term", "hack", "iosevka",
];

/// Whether the name of a family says that it is monospace.
pub(super) fn looks_monospace(name: &str) -> bool {
    let name = name.to_lowercase();
    MONO_WORDS.iter().any(|w| name.contains(w))
}

/// The families of `all` that match the lowercase `query`, in their order, and with
/// `mono_only` only the monospace ones.
pub(super) fn matching<'a>(
    all: &'a [SharedString],
    query: &str,
    mono_only: bool,
) -> impl Iterator<Item = &'a SharedString> {
    all.iter().filter(move |name| {
        (!mono_only || looks_monospace(name)) && name.to_lowercase().contains(query)
    })
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Which {
    Ui,
    Mono,
}

impl Which {
    fn key(self) -> &'static str {
        match self {
            Which::Ui => "ui",
            Which::Mono => "mono",
        }
    }
}

/// What the section keeps between draws.
pub(super) struct FontsState {
    /// The families of the text system. The first draw of the section reads them.
    names: Option<Vec<SharedString>>,
    /// The search fields, made at the first draw, which has the window.
    queries: Option<(Entity<InputState>, Entity<InputState>)>,
    mono_only: bool,
    /// The error of the last save.
    error: Option<String>,
}

impl Default for FontsState {
    fn default() -> Self {
        FontsState {
            names: None,
            queries: None,
            mono_only: true,
            error: None,
        }
    }
}

impl SettingsScreen {
    fn font_queries(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> (Entity<InputState>, Entity<InputState>) {
        if let Some(q) = &self.fonts.queries {
            return q.clone();
        }
        let mut make = |placeholder: &'static str, this: &mut Self| {
            let input = cx.new(|cx| InputState::new(window, cx).placeholder(placeholder));
            this._inputs.push(
                cx.subscribe_in(&input, window, |_, _, e: &InputEvent, _, cx| {
                    if let InputEvent::Change = e {
                        cx.notify();
                    }
                }),
            );
            input
        };
        let ui = make("Search the families", self);
        let mono = make("Search the families", self);
        self.fonts.queries = Some((ui.clone(), mono.clone()));
        (ui, mono)
    }

    fn pick_font(&mut self, which: Which, name: SharedString, cx: &mut Context<Self>) {
        let result = crate::appearance::change(cx, |look| match which {
            Which::Ui => look.fonts.ui = name,
            Which::Mono => look.fonts.mono = name,
        });
        self.fonts.error = result.err();
        cx.notify();
    }

    fn font_picker(
        &self,
        which: Which,
        current: &SharedString,
        query: &Entity<InputState>,
        cx: &mut Context<Self>,
    ) -> Div {
        let names = self.fonts.names.as_deref().unwrap_or_default();
        let q = query.read(cx).value().trim().to_lowercase();
        let mono_only = which == Which::Mono && self.fonts.mono_only;
        let found: Vec<SharedString> = matching(names, &q, mono_only).cloned().collect();
        let more = found.len().saturating_sub(SHOWN);
        let key = which.key();
        let rows = found.into_iter().take(SHOWN).map(|name| {
            let id = SharedString::from(format!("font-{key}-{name}"));
            let picked = name == *current;
            ItemRow::new(id, name.clone()).selected(picked).on_click(
                cx.listener(move |this, _, _, cx| this.pick_font(which, name.clone(), cx)),
            )
        });
        div()
            .flex()
            .flex_col()
            .gap(space(2))
            .child(Input::new(query))
            .child(
                div()
                    .id(SharedString::from(format!("font-list-{key}")))
                    .debug_selector(move || format!("font-list:{key}"))
                    .h(control::DESK_CHART_HEIGHT)
                    .overflow_y_scroll()
                    .p(space(1))
                    .rounded(radius::md())
                    .border_1()
                    .border_color(line::border())
                    .children(rows)
                    .when(more > 0, |el| {
                        el.child(
                            div()
                                .p(space(2))
                                .text_size(size::xs())
                                .text_color(text::tertiary())
                                .child(format!("{more} more. Type to narrow the list.")),
                        )
                    }),
            )
    }

    pub(super) fn fonts_section(&mut self, window: &mut Window, cx: &mut Context<Self>) -> Div {
        if self.fonts.names.is_none() {
            self.fonts.names = Some(crate::appearance::families(cx));
        }
        let (ui_query, mono_query) = self.font_queries(window, cx);
        let now = Fonts::get(cx);
        let note = self
            .fonts
            .error
            .clone()
            .or_else(|| crate::appearance::note(cx));
        let sample = |family: SharedString, selector: &'static str| {
            div()
                .debug_selector(move || selector.into())
                .font_family(family)
                .text_size(size::base())
                .text_color(text::primary())
                .child("The quick brown fox jumps over 0123456789 {}[]")
        };
        div()
            .flex()
            .flex_col()
            .gap(space(3))
            .child(section("Fonts", None))
            .child(
                div()
                    .text_size(size::sm())
                    .text_color(text::secondary())
                    .child("A click applies the family to the whole app at once. The list shows the bundled families first, then the families of this system."),
            )
            .child(pair("Text of the app", div().child(now.ui.clone())))
            .child(self.font_picker(Which::Ui, &now.ui, &ui_query, cx))
            .child(sample(font::ui(), "font-sample:ui"))
            .child(pair(
                "Terminals and code",
                div()
                    .flex()
                    .items_center()
                    .gap(space(3))
                    .child(now.mono.clone())
                    .child(
                        Switch::new("font-mono-only")
                            .checked(self.fonts.mono_only)
                            .accessibility_label("Monospace only")
                            .on_click(cx.listener(|this, on: &bool, _, cx| {
                                this.fonts.mono_only = *on;
                                cx.notify();
                            })),
                    )
                    .child(
                        div()
                            .text_size(size::sm())
                            .text_color(text::secondary())
                            .child("Monospace only"),
                    ),
            ))
            .child(self.font_picker(Which::Mono, &now.mono, &mono_query, cx))
            .child(sample(font::mono(), "font-sample:mono"))
            .when_some(note, |el, n| {
                el.child(
                    div()
                        .debug_selector(|| "fonts-note".into())
                        .text_size(size::sm())
                        .text_color(Status::Warn.color())
                        .child(n),
                )
            })
    }
}

#[cfg(test)]
mod tests {
    use gpui::{Modifiers, TestAppContext};
    use gpui_kit::SharedString;

    use super::{looks_monospace, matching};
    use crate::screens::settings::Section;
    use crate::screens::settings::tests::support::setup;

    #[test]
    fn the_monospace_filter_reads_the_family_names() {
        for name in [
            "JetBrains Mono",
            "Fira Code",
            "Courier New",
            "Inconsolata",
            "Hack",
        ] {
            assert!(looks_monospace(name), "{name}");
        }
        for name in ["Inter", "Noto Sans", "Cantarell"] {
            assert!(!looks_monospace(name), "{name}");
        }
        let all: Vec<SharedString> = ["Inter", "JetBrains Mono", "Noto Sans", "Noto Sans Mono"]
            .map(SharedString::from)
            .to_vec();
        let names = |q, mono| {
            matching(&all, q, mono)
                .map(|s| s.to_string())
                .collect::<Vec<_>>()
        };
        assert_eq!(names("noto", false), ["Noto Sans", "Noto Sans Mono"]);
        assert_eq!(names("noto", true), ["Noto Sans Mono"]);
        assert_eq!(names("", true), ["JetBrains Mono", "Noto Sans Mono"]);
    }

    #[gpui::test]
    fn a_click_on_a_family_applies_it_and_saves_it(cx: &mut TestAppContext) {
        use agentsi_ui::appearance::Fonts;
        let (t, cx) = setup(cx, None);
        t.open(cx, Section::Fonts);
        // The test text system lists no family of its own: the bundled ones show.
        let row = cx
            .debug_bounds("tree-row:font-ui-JetBrains Mono")
            .expect("the bundled mono family is in the list of the text");
        assert!(
            cx.debug_bounds("tree-row:font-mono-Inter").is_none(),
            "the monospace filter hides Inter"
        );
        cx.simulate_click(row.center(), Modifiers::none());
        cx.update(|window, cx| window.draw(cx).clear(cx));
        assert_eq!(
            cx.update(|_, cx| Fonts::get(cx).ui.to_string()),
            "JetBrains Mono"
        );
        assert_eq!(agentsi_ui::tokens::font::ui().as_ref(), "JetBrains Mono");
        let file = t.file.with_file_name(crate::appearance::FILE_NAME);
        let (look, note) = crate::appearance::read(&file);
        assert_eq!((look.fonts.ui.as_ref(), note), ("JetBrains Mono", None));
    }
}
