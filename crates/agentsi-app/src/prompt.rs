//! A small question over the window: one line of text, or one choice from a short list.
//!
//! The palette entries "Label the active product", "Color the active product", "Move the active
//! product to a group", "Rename the active pane" and "Color the active pane" use it. The context
//! menus of unit N8 use the same prompt, so a menu entry and its palette entry ask alike.
//!
//! A text prompt has an input, and an optional list of suggestions under it. `Enter` answers with
//! the text, or with the suggestion under the cursor after `Down`. A choice prompt has the list
//! only. `Escape` and a click outside cancel.

use agentsi_ui::style::StyleExt as _;
use agentsi_ui::tokens::label::Hue;
use agentsi_ui::tokens::{control, line, radius, size, space, surface, text, weight};
use gpui_kit::component::input::{Input, InputEvent, InputState};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;

/// One answer in the list of a prompt.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Choice {
    pub label: SharedString,
    /// The answer. An empty value clears: no label, no color, no group.
    pub value: String,
    pub hue: Option<Hue>,
}

impl Choice {
    pub fn new(label: impl Into<SharedString>, value: impl Into<String>) -> Choice {
        Choice {
            label: label.into(),
            value: value.into(),
            hue: None,
        }
    }

    pub fn hue(mut self, hue: Hue) -> Choice {
        self.hue = Some(hue);
        self
    }

    /// "No color" and the 8 colors, with `Hue::key` as the value.
    pub fn colors() -> Vec<Choice> {
        std::iter::once(Choice::new("No color", ""))
            .chain(Hue::ALL.map(|h| Choice::new(h.name(), h.key()).hue(h)))
            .collect()
    }
}

/// How the owner left the prompt.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PromptEvent {
    Answer(String),
    Cancel,
}

impl EventEmitter<PromptEvent> for Prompt {}

pub struct Prompt {
    title: SharedString,
    hint: Option<SharedString>,
    input: Option<Entity<InputState>>,
    choices: Vec<Choice>,
    /// The suggestion under the cursor, as an index into `choices`.
    cursor: Option<usize>,
    /// The owner typed: until then the list shows each suggestion, also with a value in the input.
    edited: bool,
    focus: FocusHandle,
    _subscriptions: Vec<Subscription>,
}

impl Prompt {
    /// A line of text, with `initial` in it and `suggestions` under it.
    pub fn text(
        title: impl Into<SharedString>,
        hint: impl Into<SharedString>,
        initial: &str,
        suggestions: Vec<Choice>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Prompt {
        let input = cx.new(|cx| InputState::new(window, cx));
        input.update(cx, |s, cx| s.set_value(initial.to_string(), window, cx));
        let subscriptions = vec![cx.subscribe(&input, |this, _, e: &InputEvent, cx| match e {
            InputEvent::PressEnter { .. } => this.answer(cx),
            InputEvent::Change => {
                this.edited = true;
                this.cursor = None;
                cx.notify();
            }
            _ => {}
        })];
        let focus = input.focus_handle(cx);
        window.defer(cx, {
            let focus = focus.clone();
            move |window, cx| window.focus(&focus, cx)
        });
        Prompt {
            title: title.into(),
            hint: Some(hint.into()),
            input: Some(input),
            choices: suggestions,
            cursor: None,
            edited: false,
            focus,
            _subscriptions: subscriptions,
        }
    }

    /// A list of choices, with the cursor on `current`.
    pub fn pick(
        title: impl Into<SharedString>,
        choices: Vec<Choice>,
        current: usize,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Prompt {
        let focus = cx.focus_handle();
        window.defer(cx, {
            let focus = focus.clone();
            move |window, cx| window.focus(&focus, cx)
        });
        Prompt {
            title: title.into(),
            hint: None,
            input: None,
            cursor: Some(current.min(choices.len().saturating_sub(1))),
            choices,
            edited: false,
            focus,
            _subscriptions: Vec::new(),
        }
    }

    /// The text in the input, or none for a choice prompt.
    pub fn text_value(&self, cx: &App) -> Option<String> {
        self.input.as_ref().map(|i| i.read(cx).value().to_string())
    }

    /// The suggestions that match the text, as indexes into `choices`.
    fn shown(&self, cx: &App) -> Vec<usize> {
        let typed = self
            .text_value(cx)
            .filter(|_| self.edited)
            .map(|t| t.trim().to_lowercase())
            .unwrap_or_default();
        (0..self.choices.len())
            .filter(|i| typed.is_empty() || self.choices[*i].label.to_lowercase().contains(&typed))
            .collect()
    }

    fn answer(&mut self, cx: &mut Context<Self>) {
        let value = match (self.cursor, &self.input) {
            (Some(i), _) => self.choices.get(i).map(|c| c.value.clone()),
            (None, Some(_)) => self.text_value(cx),
            (None, None) => None,
        };
        if let Some(v) = value {
            cx.emit(PromptEvent::Answer(v));
        }
    }

    fn key_down(&mut self, e: &KeyDownEvent, cx: &mut Context<Self>) {
        let k = &e.keystroke;
        if k.modifiers.modified() {
            return;
        }
        match k.key.as_str() {
            "escape" => {
                cx.emit(PromptEvent::Cancel);
                cx.stop_propagation();
            }
            "down" | "up" => {
                let shown = self.shown(cx);
                if shown.is_empty() {
                    return;
                }
                let at = self.cursor.and_then(|c| shown.iter().position(|s| *s == c));
                let to = match (at, k.key == "down") {
                    (None, true) => 0,
                    (None, false) => shown.len() - 1,
                    (Some(i), true) => (i + 1).min(shown.len() - 1),
                    (Some(i), false) => i.saturating_sub(1),
                };
                self.cursor = Some(shown[to]);
                cx.stop_propagation();
                cx.notify();
            }
            "enter" if self.input.is_none() => {
                self.answer(cx);
                cx.stop_propagation();
            }
            _ => {}
        }
    }
}

fn swatch(hue: Option<Hue>) -> Div {
    div()
        .flex_none()
        .size(control::icon_sm())
        .rounded(radius::sm())
        .when_some(hue, |el, h| el.bg(h.color()))
        .when(hue.is_none(), |el| {
            el.border_1().border_color(line::strong())
        })
}

impl Render for Prompt {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let shown = self.shown(cx);
        let rows: Vec<AnyElement> = shown
            .into_iter()
            .map(|i| {
                let c = &self.choices[i];
                let on = self.cursor == Some(i);
                let value = c.value.clone();
                div()
                    .id(("prompt-choice", i))
                    .debug_selector({
                        let key = if c.value.is_empty() {
                            "none".to_string()
                        } else {
                            c.value.clone()
                        };
                        move || format!("prompt-choice:{key}")
                    })
                    .flex()
                    .items_center()
                    .gap(space(2))
                    .h(control::height_md())
                    .pad_x(space(2))
                    .rounded(radius::md())
                    .cursor_pointer()
                    .text_size(size::base())
                    .text_color(text::primary())
                    .when(on, |el| el.bg(surface::raised()))
                    .hover(|el| el.bg(surface::raised()))
                    .when(c.hue.is_some() || self.input.is_none(), |el| {
                        el.child(swatch(c.hue))
                    })
                    .child(c.label.clone())
                    .on_click(
                        cx.listener(move |_, _, _, cx| cx.emit(PromptEvent::Answer(value.clone()))),
                    )
                    .into_any_element()
            })
            .collect();
        div()
            .id("prompt")
            .debug_selector(|| "prompt".into())
            .absolute()
            .top_0()
            .left_0()
            .size_full()
            .flex()
            .justify_center()
            .items_start()
            .pt(space(16))
            .bg(surface::base().alpha(control::SCRIM_ALPHA))
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(|_, _, _, cx| cx.emit(PromptEvent::Cancel)),
            )
            .child(
                div()
                    .track_focus(&self.focus)
                    .key_context("Prompt")
                    .on_key_down(cx.listener(|this, e: &KeyDownEvent, _, cx| this.key_down(e, cx)))
                    .w(control::PALETTE_WIDTH)
                    .flex()
                    .flex_col()
                    .gap(space(2))
                    .p(space(3))
                    .rounded(radius::lg())
                    .bg(surface::overlay())
                    .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                    .child(
                        div()
                            .text_size(size::md())
                            .font_weight(weight::SEMIBOLD)
                            .text_color(text::primary())
                            .child(self.title.clone()),
                    )
                    .when_some(self.input.clone(), |el, i| el.child(Input::new(&i)))
                    .children(rows)
                    .when_some(self.hint.clone(), |el, h| {
                        el.child(
                            div()
                                .text_size(size::xs())
                                .text_color(text::tertiary())
                                .child(h),
                        )
                    }),
            )
    }
}
