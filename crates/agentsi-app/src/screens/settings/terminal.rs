//! The Terminal section: the font size, the line height, the cursor, the scrollback, the shell,
//! the palette, the two switches and the link key of `terminal.toml`, with a live sample pane.
//!
//! A change applies to each open pane at once and saves the file (see
//! `crate::terminal_settings`). The scrollback and the shell are read when a pane opens.

use std::time::Instant;

use agentsi_term_view::{PaneOptions, TerminalPane};
use agentsi_ui::appearance::{
    CursorBlink, CursorStyle, LinkModifier, TerminalPalette, TerminalSettings,
};
use agentsi_ui::components::{Button, Switch};
use agentsi_ui::tokens::{Status, control, radius, size, space, text};
use gpui_kit::component::IconName;
use gpui_kit::component::input::{Input, InputEvent, InputState};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;

use super::{SettingsScreen, pair};
use crate::screens::parts::section;

/// The 4 lines of the sample pane: the text styles and the 16 colors.
const SAMPLE: &[u8] = b"\x1b[1mAgentSI\x1b[0m terminal \x1b[2m(faint)\x1b[0m \x1b[3mitalic\x1b[0m\r\n\
\x1b[31mred \x1b[32mgreen \x1b[33myellow \x1b[34mblue \x1b[35mmagenta \x1b[36mcyan \x1b[37mwhite\x1b[0m\r\n\
\x1b[91mred \x1b[92mgreen \x1b[93myellow \x1b[94mblue \x1b[95mmagenta \x1b[96mcyan \x1b[97mwhite\x1b[0m\r\n\
\x1b[32m~/dev/product\x1b[0m $ ";

/// The steps of the scrollback buttons, in MB. A value of the file between two steps stays.
const SCROLLBACK_STEPS: [u16; 6] = [1, 5, 10, 25, 50, 100];

/// The value of a stepper after one step down (`up` false) or up, inside `range`.
pub(super) fn step_font_size(now: u8, up: bool) -> u8 {
    let next = if up {
        now.saturating_add(1)
    } else {
        now.saturating_sub(1)
    };
    next.clamp(
        *TerminalSettings::FONT_SIZES.start(),
        *TerminalSettings::FONT_SIZES.end(),
    )
}

/// The line height one step of 0.05 down or up, inside the range.
pub(super) fn step_line_height(now: f32, up: bool) -> f32 {
    let steps = (now * 20.).round() + if up { 1. } else { -1. };
    (steps / 20.).clamp(
        *TerminalSettings::LINE_HEIGHTS.start(),
        *TerminalSettings::LINE_HEIGHTS.end(),
    )
}

/// The next step of [`SCROLLBACK_STEPS`] below or above `now`. At an end, `now`.
pub(super) fn step_scrollback(now: u16, up: bool) -> u16 {
    if up {
        SCROLLBACK_STEPS
            .into_iter()
            .find(|s| *s > now)
            .unwrap_or(now)
    } else {
        SCROLLBACK_STEPS
            .into_iter()
            .rev()
            .find(|s| *s < now)
            .unwrap_or(now)
    }
}

/// What the section keeps between draws.
#[derive(Default)]
pub(super) struct TerminalState {
    /// The live sample. The first draw of the section makes it, with the window.
    sample: Option<Entity<TerminalPane>>,
    /// The shell field.
    shell: Option<Entity<InputState>>,
    /// The error of the last save.
    error: Option<String>,
}

impl SettingsScreen {
    fn change_terminal(&mut self, f: impl FnOnce(&mut TerminalSettings), cx: &mut Context<Self>) {
        self.terminal.error = crate::terminal_settings::change(cx, f).err();
        cx.notify();
    }

    /// One row of buttons, one for each choice. The current choice has the accent fill.
    fn terminal_choices<T: Copy + PartialEq + 'static>(
        &self,
        group: &'static str,
        all: impl IntoIterator<Item = T>,
        current: T,
        label: fn(T) -> &'static str,
        set: fn(&mut TerminalSettings, T),
        cx: &mut Context<Self>,
    ) -> Div {
        div()
            .debug_selector(move || format!("terminal-group:{group}"))
            .flex()
            .flex_wrap()
            .justify_end()
            .gap(space(2))
            .children(all.into_iter().enumerate().map(|(i, choice)| {
                let button = Button::new((group, i))
                    .small()
                    .label(label(choice))
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.change_terminal(|t| set(t, choice), cx)
                    }));
                if choice == current {
                    button.primary()
                } else {
                    button.secondary()
                }
            }))
    }

    /// A value with a button down and a button up.
    fn stepper(
        &self,
        id: &'static str,
        value: String,
        step: fn(&mut TerminalSettings, bool),
        cx: &mut Context<Self>,
    ) -> Div {
        let button = |up: bool, cx: &mut Context<Self>| {
            Button::new((id, usize::from(up)))
                .small()
                .secondary()
                .icon(if up { IconName::Plus } else { IconName::Minus })
                .on_click(
                    cx.listener(move |this, _, _, cx| this.change_terminal(|t| step(t, up), cx)),
                )
        };
        div()
            .flex()
            .items_center()
            .gap(space(2))
            .child(button(false, cx))
            .child(
                div()
                    .debug_selector(move || format!("terminal-value:{id}"))
                    .min_w(control::NUM_COLUMN)
                    .flex()
                    .justify_center()
                    .text_size(size::base())
                    .text_color(text::primary())
                    .child(value),
            )
            .child(button(true, cx))
    }

    fn shell_input(&mut self, window: &mut Window, cx: &mut Context<Self>) -> Entity<InputState> {
        if let Some(input) = &self.terminal.shell {
            return input.clone();
        }
        let now = TerminalSettings::get(cx).shell;
        let input = cx.new(|cx| {
            let mut input = InputState::new(window, cx).placeholder("Empty: $SHELL");
            input.set_value(now, window, cx);
            input
        });
        self._inputs.push(
            cx.subscribe_in(&input, window, |this, input, e: &InputEvent, _, cx| {
                if let InputEvent::PressEnter { .. } = e {
                    let shell = input.read(cx).value().trim().to_string();
                    this.change_terminal(|t| t.shell = shell.into(), cx);
                }
            }),
        );
        self.terminal.shell = Some(input.clone());
        input
    }

    fn sample_pane(&mut self, window: &mut Window, cx: &mut Context<Self>) -> Entity<TerminalPane> {
        if let Some(p) = &self.terminal.sample {
            return p.clone();
        }
        let pane = cx.new(|cx| {
            let mut pane = TerminalPane::new(PaneOptions::default(), window, cx);
            pane.feed(SAMPLE, Instant::now(), cx);
            pane
        });
        self.terminal.sample = Some(pane.clone());
        pane
    }

    pub(super) fn terminal_section(&mut self, window: &mut Window, cx: &mut Context<Self>) -> Div {
        let t = TerminalSettings::get(cx);
        let shell = self.shell_input(window, cx);
        let sample = self.sample_pane(window, cx);
        let note = self
            .terminal
            .error
            .clone()
            .or_else(|| crate::terminal_settings::note(cx));
        let hint = |s: &'static str| {
            div()
                .text_size(size::xs())
                .text_color(text::tertiary())
                .child(s)
        };
        div()
            .flex()
            .flex_col()
            .gap(space(3))
            .child(section("Terminal", None))
            .child(
                div()
                    .text_size(size::sm())
                    .text_color(text::secondary())
                    .child("A change applies to each open pane at once. The font family is in the section Fonts."),
            )
            .child(
                div()
                    .id("terminal-sample")
                    .debug_selector(|| "terminal-sample".into())
                    .h(control::TERMINAL_SAMPLE_HEIGHT)
                    .rounded(radius::md())
                    .overflow_hidden()
                    .child(sample),
            )
            .child(pair(
                "Font size",
                self.stepper(
                    "terminal-font-size",
                    format!("{} px", t.font_size),
                    |t, up| t.font_size = step_font_size(t.font_size, up),
                    cx,
                ),
            ))
            .child(pair(
                "Line height",
                self.stepper(
                    "terminal-line-height",
                    format!("{:.2}", t.line_height),
                    |t, up| t.line_height = step_line_height(t.line_height, up),
                    cx,
                ),
            ))
            .child(pair(
                "Cursor",
                self.terminal_choices(
                    "terminal-cursor",
                    CursorStyle::ALL,
                    t.cursor_style,
                    CursorStyle::label,
                    |t, v| t.cursor_style = v,
                    cx,
                ),
            ))
            .child(pair(
                "Cursor blink",
                self.terminal_choices(
                    "terminal-blink",
                    CursorBlink::ALL,
                    t.cursor_blink,
                    CursorBlink::label,
                    |t, v| t.cursor_blink = v,
                    cx,
                ),
            ))
            .child(pair(
                "Colors",
                self.terminal_choices(
                    "terminal-palette",
                    TerminalPalette::ALL,
                    t.palette,
                    TerminalPalette::label,
                    |t, v| t.palette = v,
                    cx,
                ),
            ))
            .child(pair(
                "Scrollback",
                self.stepper(
                    "terminal-scrollback",
                    format!("{} MB", t.scrollback_mb),
                    |t, up| t.scrollback_mb = step_scrollback(t.scrollback_mb, up),
                    cx,
                ),
            ))
            .child(hint("New panes use it. Open panes keep their scrollback."))
            .child(pair(
                "Shell",
                div()
                    .debug_selector(|| "terminal-shell".into())
                    .w(control::TRAILING_MAX * 2.)
                    .child(Input::new(&shell)),
            ))
            .child(hint("Enter saves. New panes of this device use it. Open panes keep their shell, and a pane of an SSH host uses the shell of the host."))
            .child(pair(
                "Copy on select",
                Switch::new("terminal-copy-on-select")
                    .checked(t.copy_on_select)
                    .accessibility_label("Copy on select")
                    .on_click(cx.listener(|this, on: &bool, _, cx| {
                        let on = *on;
                        this.change_terminal(|t| t.copy_on_select = on, cx)
                    })),
            ))
            .child(hint("A selection with the mouse also goes to the clipboard."))
            .child(pair(
                "Option as Meta",
                Switch::new("terminal-option-as-meta")
                    .checked(t.option_as_meta)
                    .accessibility_label("Option as Meta")
                    .on_click(cx.listener(|this, on: &bool, _, cx| {
                        let on = *on;
                        this.change_terminal(|t| t.option_as_meta = on, cx)
                    })),
            ))
            .child(hint("On macOS, Option sends Alt to the program. Other systems always send Alt."))
            .child(pair(
                "Open links",
                self.terminal_choices(
                    "terminal-link",
                    LinkModifier::ALL,
                    t.link_modifier,
                    LinkModifier::label,
                    |t, v| t.link_modifier = v,
                    cx,
                ),
            ))
            .child(hint("A URL opens in the browser, and a path with a line in $EDITOR. On macOS, Ctrl is Cmd."))
            .when_some(note, |el, n| {
                el.child(
                    div()
                        .debug_selector(|| "terminal-note".into())
                        .text_size(size::sm())
                        .text_color(Status::Warn.color())
                        .child(n),
                )
            })
    }
}

#[cfg(test)]
mod tests {
    use agentsi_ui::appearance::{CursorStyle, LinkModifier, TerminalSettings};
    use gpui::{Modifiers, TestAppContext, VisualTestContext};

    use super::{step_font_size, step_line_height, step_scrollback};
    use crate::screens::settings::Section;
    use crate::screens::settings::tests::support::setup;

    #[test]
    fn the_steppers_stay_inside_their_ranges() {
        assert_eq!(step_font_size(13, true), 14);
        assert_eq!(step_font_size(24, true), 24);
        assert_eq!(step_font_size(9, false), 9);
        assert_eq!(step_line_height(1.25, true), 1.3);
        assert_eq!(step_line_height(1.6, true), 1.6);
        assert_eq!(step_line_height(1.0, false), 1.0);
        assert_eq!(step_line_height(1.05, false), 1.0);
        assert_eq!(step_scrollback(10, true), 25);
        assert_eq!(step_scrollback(37, false), 25);
        assert_eq!(step_scrollback(37, true), 50);
        assert_eq!(step_scrollback(100, true), 100);
        assert_eq!(step_scrollback(1, false), 1);
    }

    fn click(cx: &mut VisualTestContext, selector: &'static str) {
        let b = cx
            .debug_bounds(selector)
            .unwrap_or_else(|| panic!("{selector} is drawn"));
        cx.simulate_click(b.center(), Modifiers::none());
        cx.update(|window, cx| window.draw(cx).clear(cx));
    }

    #[gpui::test]
    fn the_section_applies_each_change_at_once_and_saves_it(cx: &mut TestAppContext) {
        let (t, cx) = setup(cx, None);
        let file = t.file.with_file_name(crate::terminal_settings::FILE_NAME);
        cx.update(|_, cx| crate::terminal_settings::install(cx, Some(file)));
        t.open(cx, Section::Terminal);
        assert!(
            cx.debug_bounds("terminal-sample").is_some(),
            "the live sample"
        );
        click(cx, "button:terminal-font-size-1");
        click(cx, "button:terminal-font-size-1");
        click(cx, "button:terminal-cursor-2");
        click(cx, "button:terminal-link-1");
        let now = cx.update(|_, cx| TerminalSettings::get(cx));
        assert_eq!((now.font_size, now.cursor_style), (15, CursorStyle::Bar));
        assert_eq!(now.link_modifier, LinkModifier::None);
        assert_eq!(agentsi_ui::tokens::size::mono(), gpui::px(15.));
        let file = t.file.with_file_name(crate::terminal_settings::FILE_NAME);
        assert_eq!(crate::terminal_settings::read(&file), (now, None));
    }

    #[gpui::test]
    fn a_bad_value_in_the_file_shows_one_note_line(cx: &mut TestAppContext) {
        let (t, cx) = setup(cx, None);
        let file = t.file.with_file_name(crate::terminal_settings::FILE_NAME);
        std::fs::write(&file, "font_size = 99\n").unwrap();
        cx.update(|_, cx| crate::terminal_settings::install(cx, Some(file)));
        t.open(cx, Section::Terminal);
        assert_eq!(cx.update(|_, cx| TerminalSettings::get(cx).font_size), 13);
        let note = cx.debug_bounds("terminal-note").expect("the note shows");
        assert!(note.size.height > gpui::px(0.));
        let text = cx
            .update(|_, cx| crate::terminal_settings::note(cx))
            .unwrap();
        assert_eq!(text.lines().count(), 1, "{text}");
    }
}
