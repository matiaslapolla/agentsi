//! A form: text fields, choices and check boxes inside a screen, with one button that
//! writes and one Cancel button. The screen that holds the form listens to `FormEvent`.
//!
//! `Enter` in the last field writes. `Escape` cancels. An empty required field disables the
//! write button, and `Enter` then does nothing.

use agentsi_ui::components::Button;
use agentsi_ui::tokens::{line, radius, size, space, surface, text, weight};
use gpui_kit::component::input::{Input, InputEvent, InputState};
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

pub struct Form {
    id: &'static str,
    title: &'static str,
    submit: &'static str,
    fields: Vec<Field>,
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
            fields: Vec::new(),
            focus: cx.focus_handle(),
            _subs: Vec::new(),
        }
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
                InputEvent::Change => cx.notify(),
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
        if self.ready(cx) {
            cx.emit(FormEvent::Submit);
        }
    }

    pub fn cancel(&mut self, cx: &mut Context<Self>) {
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
            .child(
                div()
                    .flex()
                    .gap(space(2))
                    .child(
                        Button::new(SharedString::from(format!("{id}-submit")))
                            .primary()
                            .small()
                            .label(self.submit)
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

        cx.update(|window, cx| {
            let focus = form.read(cx).focus.clone();
            window.focus(&focus, cx)
        });
        cx.simulate_keystrokes("escape");
        let _ = Modifiers::none();
        assert_eq!(events.borrow().last(), Some(&FormEvent::Cancel));
    }
}
