//! A form: text fields, choices and check boxes inside a screen, with one button that
//! writes and one Cancel button. The screen that holds the form listens to `FormEvent`.
//!
//! `Enter` in the last field writes. `Escape` cancels. An empty required field disables the
//! write button, and `Enter` then does nothing.

use agentsi_ui::components::Button;
use agentsi_ui::tokens::{line, radius, size, space, surface, text, weight};
use gpui_kit::component::checkbox::Checkbox;
use gpui_kit::component::input::{Input, InputEvent, InputState};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;

/// What the form tells its screen.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum FormEvent {
    Submit,
    Cancel,
}

struct Field {
    label: &'static str,
    required: bool,
    input: Entity<InputState>,
}

struct Pick {
    label: &'static str,
    options: Vec<&'static str>,
    selected: usize,
}

struct Check {
    label: &'static str,
    checked: bool,
}

pub struct Form {
    id: &'static str,
    title: &'static str,
    submit: &'static str,
    /// A line above the buttons, for example the cost of a model call.
    note: Option<String>,
    /// The write needs a Confirm, and `armed` says the owner clicked once.
    confirm: bool,
    armed: bool,
    fields: Vec<Field>,
    picks: Vec<Pick>,
    checks: Vec<Check>,
    focus: FocusHandle,
    _subs: Vec<Subscription>,
}

impl EventEmitter<FormEvent> for Form {}

impl Form {
    pub fn new(
        id: &'static str,
        title: &'static str,
        submit: &'static str,
        cx: &mut Context<Self>,
    ) -> Self {
        Form {
            id,
            title,
            submit,
            note: None,
            confirm: false,
            armed: false,
            fields: Vec::new(),
            picks: Vec::new(),
            checks: Vec::new(),
            focus: cx.focus_handle(),
            _subs: Vec::new(),
        }
    }

    /// A line above the buttons. A form that starts a model call says that it costs money. The
    /// first click on the write button then asks for a Confirm, and the second one writes.
    pub fn confirm(mut self, note: impl Into<String>) -> Self {
        self.note = Some(note.into());
        self.confirm = true;
        self
    }

    pub fn field(
        mut self,
        label: &'static str,
        placeholder: &'static str,
        required: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let input = cx.new(|cx| InputState::new(window, cx).placeholder(placeholder));
        self._subs
            .push(cx.subscribe(&input, |this, source, e: &InputEvent, cx| match e {
                InputEvent::Change => {
                    this.armed = false;
                    cx.notify()
                }
                InputEvent::PressEnter { .. }
                    if this.fields.last().is_some_and(|f| f.input == source) =>
                {
                    this.submit(cx)
                }
                _ => {}
            }));
        self.fields.push(Field {
            label,
            required,
            input,
        });
        self
    }

    pub fn pick(mut self, label: &'static str, options: &[&'static str]) -> Self {
        self.picks.push(Pick {
            label,
            options: options.to_vec(),
            selected: 0,
        });
        self
    }

    pub fn check(mut self, label: &'static str, checked: bool) -> Self {
        self.checks.push(Check { label, checked });
        self
    }

    /// The trimmed text of field `i`.
    pub fn value(&self, i: usize, cx: &App) -> String {
        self.fields
            .get(i)
            .map(|f| f.input.read(cx).value().trim().to_string())
            .unwrap_or_default()
    }

    #[cfg(test)]
    pub fn set_value(&mut self, i: usize, value: &str, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(f) = self.fields.get(i) {
            f.input.update(cx, |s, cx| s.set_value(value, window, cx));
        }
    }

    /// The option of choice `i`.
    pub fn picked(&self, i: usize) -> &'static str {
        self.picks
            .get(i)
            .and_then(|p| p.options.get(p.selected).copied())
            .unwrap_or("")
    }

    pub fn select(&mut self, i: usize, option: usize, cx: &mut Context<Self>) {
        if let Some(p) = self.picks.get_mut(i)
            && option < p.options.len()
        {
            p.selected = option;
            cx.notify();
        }
    }

    pub fn has_picks(&self) -> bool {
        !self.picks.is_empty()
    }

    pub fn has_checks(&self) -> bool {
        !self.checks.is_empty()
    }

    pub fn checked(&self, i: usize) -> bool {
        self.checks.get(i).is_some_and(|c| c.checked)
    }

    pub fn set_checked(&mut self, i: usize, on: bool, cx: &mut Context<Self>) {
        if let Some(c) = self.checks.get_mut(i) {
            c.checked = on;
            cx.notify();
        }
    }

    /// True when each required field has text.
    pub fn ready(&self, cx: &App) -> bool {
        self.fields
            .iter()
            .all(|f| !f.required || !f.input.read(cx).value().trim().is_empty())
    }

    pub fn focus_first(&self, window: &mut Window, cx: &mut App) {
        if let Some(f) = self.fields.first() {
            let focus = f.input.focus_handle(cx);
            window.defer(cx, move |window, cx| window.focus(&focus, cx));
        }
    }

    pub fn submit(&mut self, cx: &mut Context<Self>) {
        if !self.ready(cx) {
            return;
        }
        if self.confirm && !self.armed {
            self.armed = true;
            cx.notify();
            return;
        }
        self.armed = false;
        cx.emit(FormEvent::Submit);
    }

    pub fn cancel(&mut self, cx: &mut Context<Self>) {
        self.armed = false;
        cx.emit(FormEvent::Cancel);
    }
}

impl Render for Form {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let id = self.id;
        let ready = self.ready(cx);
        let fields: Vec<Div> = self
            .fields
            .iter()
            .map(|f| {
                div()
                    .flex()
                    .flex_col()
                    .gap(space(1))
                    .child(
                        div()
                            .text_size(size::xs())
                            .font_weight(weight::MEDIUM)
                            .text_color(text::tertiary())
                            .child(f.label),
                    )
                    .child(Input::new(&f.input))
            })
            .collect();
        let picks: Vec<Div> = self
            .picks
            .iter()
            .enumerate()
            .map(|(i, p)| {
                div()
                    .flex()
                    .items_center()
                    .gap(space(2))
                    .child(
                        div()
                            .text_size(size::sm())
                            .text_color(text::secondary())
                            .child(p.label),
                    )
                    .children(p.options.iter().enumerate().map(|(o, name)| {
                        let button = Button::new((SharedString::from(format!("{id}-pick-{i}")), o))
                            .small()
                            .label(*name)
                            .on_click(cx.listener(move |this, _, _, cx| this.select(i, o, cx)));
                        if o == p.selected {
                            button.primary()
                        } else {
                            button.secondary()
                        }
                    }))
            })
            .collect();
        let checks: Vec<Div> = self
            .checks
            .iter()
            .enumerate()
            .map(|(i, c)| {
                div().child(
                    Checkbox::new((SharedString::from(format!("{id}-check")), i))
                        .label(c.label)
                        .checked(c.checked)
                        .on_click(cx.listener(move |this, on: &bool, _, cx| {
                            this.set_checked(i, *on, cx)
                        })),
                )
            })
            .collect();
        div()
            .debug_selector(move || format!("form:{id}"))
            .track_focus(&self.focus)
            .on_key_down(cx.listener(|this, e: &KeyDownEvent, _, cx| {
                if e.keystroke.key == "escape" {
                    this.cancel(cx);
                    cx.stop_propagation();
                }
            }))
            .flex()
            .flex_col()
            .gap(space(3))
            .p(space(4))
            .rounded(radius::lg())
            .border_1()
            .border_color(line::border())
            .bg(surface::raised())
            .child(
                div()
                    .text_size(size::md())
                    .font_weight(weight::SEMIBOLD)
                    .text_color(text::primary())
                    .child(self.title),
            )
            .children(fields)
            .children(picks)
            .children(checks)
            .when_some(self.note.clone(), |el, n| {
                el.child(
                    div()
                        .text_size(size::sm())
                        .text_color(text::tertiary())
                        .child(n),
                )
            })
            .child(
                div()
                    .flex()
                    .gap(space(2))
                    .child(
                        Button::new(SharedString::from(format!("{id}-submit")))
                            .primary()
                            .small()
                            .label(if self.armed { "Confirm" } else { self.submit })
                            .disabled(!ready)
                            .on_click(cx.listener(|this, _, _, cx| this.submit(cx))),
                    )
                    .child(
                        Button::new(SharedString::from(format!("{id}-cancel")))
                            .secondary()
                            .small()
                            .label("Cancel")
                            .on_click(cx.listener(|this, _, _, cx| this.cancel(cx))),
                    ),
            )
    }
}

#[cfg(test)]
mod tests {
    use std::cell::RefCell;
    use std::rc::Rc;

    use gpui::{Modifiers, TestAppContext};

    use super::{Form, FormEvent};

    #[gpui::test]
    fn enter_in_the_last_field_writes_escape_cancels_and_an_empty_required_field_blocks(
        cx: &mut TestAppContext,
    ) {
        cx.update(agentsi_ui::init);
        let (form, cx) = cx.add_window_view(|window, cx| {
            Form::new("t", "Test", "Save", cx)
                .field("Text", "text", true, window, cx)
                .field("Why", "why", false, window, cx)
                .pick("Kind", &["A", "B"])
                .check("Box", false)
        });
        let events = Rc::new(RefCell::new(Vec::new()));
        let log = events.clone();
        let _sub = cx.update(|_, cx| {
            cx.subscribe(&form, move |_, e: &FormEvent, _| log.borrow_mut().push(e.clone()))
        });
        cx.update(|window, cx| window.draw(cx).clear(cx));
        assert!(!form.read_with(cx, |f, cx| f.ready(cx)));
        form.update(cx, |f, cx| f.submit(cx));
        assert!(events.borrow().is_empty(), "an empty required field blocks");

        form.update_in(cx, |f, window, cx| f.set_value(0, "hello", window, cx));
        assert!(form.read_with(cx, |f, cx| f.ready(cx)));
        form.update(cx, |f, cx| f.submit(cx));
        assert_eq!(*events.borrow(), [FormEvent::Submit]);

        form.update(cx, |f, cx| {
            f.select(0, 1, cx);
            f.set_checked(0, true, cx);
        });
        assert_eq!(form.read_with(cx, |f, _| f.picked(0)), "B");
        assert!(form.read_with(cx, |f, _| f.checked(0)));

        cx.update(|window, cx| {
            let focus = form.read(cx).focus.clone();
            window.focus(&focus, cx)
        });
        cx.simulate_keystrokes("escape");
        let _ = Modifiers::none();
        assert_eq!(events.borrow().last(), Some(&FormEvent::Cancel));
    }
}
