//! The Owner section: the name and the language of the owner. `registry.get` reads them and
//! `registry.update` saves them in `agentsi.config.json`. The prompts of the roles use both.

use std::cell::Cell;
use std::rc::Rc;

use agentsi_proto::agency_api::Owner;
use agentsi_proto::registry_api::{
    OwnerChange, RegistryGet, RegistryResult, RegistryUpdate, RegistryUpdateParams,
};
use agentsi_ui::components::Button;
use agentsi_ui::tokens::{size, space, text, weight};
use gpui_kit::component::input::{Input, InputState};
use gpui_kit::*;

use super::SettingsScreen;
use crate::screens::parts::{Flash, Load, flash_line, mono, section};

#[derive(Default)]
pub(super) struct OwnerState {
    owner: Option<Load<Owner>>,
    /// The name field and the language field. The first draw makes them.
    inputs: Option<(Entity<InputState>, Entity<InputState>)>,
    /// The values for the fields, set at the next draw, which has the window.
    fill: Option<Owner>,
    note: Option<Flash>,
    saving: bool,
    reveal: Rc<Cell<bool>>,
}

impl SettingsScreen {
    pub(super) fn load_owner(&mut self, cx: &mut Context<Self>) {
        let call = self
            .core
            .read(cx)
            .link
            .call::<RegistryGet>(&Default::default());
        cx.spawn(async move |this, cx| {
            let result = call.await.map_err(|e| e.to_string());
            let _ = this.update(cx, |this, cx| this.owner_loaded(result, cx));
        })
        .detach();
    }

    pub(super) fn owner_loaded(
        &mut self,
        result: Result<RegistryResult, String>,
        cx: &mut Context<Self>,
    ) {
        let first = !matches!(self.owner_state.owner, Some(Load::Ready(_)));
        let mut load = Load::Loading;
        load.settle(result.map(|r| r.owner));
        // A reload keeps what the owner typed. The fields take the stored values at the first
        // read and after a save.
        if first && let Load::Ready(o) = &load {
            self.owner_state.fill = Some(o.clone());
        }
        self.owner_state.owner = Some(load);
        cx.notify();
    }

    pub fn save_owner(&mut self, cx: &mut Context<Self>) {
        let Some((name, language)) = self.owner_state.inputs.clone() else {
            return;
        };
        if self.owner_state.saving {
            return;
        }
        let (name, language) = (
            name.read(cx).value().trim().to_string(),
            language.read(cx).value().trim().to_string(),
        );
        if name.is_empty() || language.is_empty() {
            self.owner_state.note =
                Some(Flash::Failed("The name and the language need text.".into()));
            cx.notify();
            return;
        }
        self.owner_state.saving = true;
        self.owner_state.note = None;
        let call = self
            .core
            .read(cx)
            .link
            .call::<RegistryUpdate>(&RegistryUpdateParams {
                owner: Some(OwnerChange {
                    name: Some(name),
                    language: Some(language),
                }),
                product: None,
            });
        cx.spawn(async move |this, cx| {
            let result = call.await.map_err(|e| e.to_string());
            let _ = this.update(cx, |this, cx| this.owner_saved(result, cx));
        })
        .detach();
        cx.notify();
    }

    pub(super) fn owner_saved(
        &mut self,
        result: Result<RegistryResult, String>,
        cx: &mut Context<Self>,
    ) {
        self.owner_state.saving = false;
        self.owner_state.note = Some(match result {
            Ok(r) => {
                self.owner_state.fill = Some(r.owner.clone());
                self.owner_state.owner = Some(Load::Ready(r.owner));
                Flash::Done("Saved.".into())
            }
            Err(e) => Flash::Failed(e),
        });
        cx.notify();
    }

    /// Scrolls the Owner section into view on the next draw.
    pub fn reveal_owner(&mut self, cx: &mut Context<Self>) {
        self.owner_state.reveal.set(true);
        cx.notify();
    }

    pub(super) fn owner_section(&mut self, window: &mut Window, cx: &mut Context<Self>) -> Div {
        let (name, language) = self
            .owner_state
            .inputs
            .get_or_insert_with(|| {
                (
                    cx.new(|cx| InputState::new(window, cx).placeholder("Your name")),
                    cx.new(|cx| InputState::new(window, cx).placeholder("English")),
                )
            })
            .clone();
        if let Some(o) = self.owner_state.fill.take() {
            name.update(cx, |s, cx| s.set_value(o.name, window, cx));
            language.update(cx, |s, cx| s.set_value(o.language, window, cx));
        }
        let field = |label: &'static str, input: &Entity<InputState>| {
            div()
                .flex()
                .flex_col()
                .gap(space(1))
                .child(
                    div()
                        .text_size(size::xs())
                        .font_weight(weight::MEDIUM)
                        .text_color(text::tertiary())
                        .child(label),
                )
                .child(Input::new(input))
        };
        let state = match &self.owner_state.owner {
            None | Some(Load::Loading) => Some(mono("Reading the owner…")),
            Some(Load::Failed(e)) => Some(mono(format!("The owner did not load: {e}"))),
            Some(Load::Ready(_)) => None,
        };
        let reveal = self.owner_state.reveal.clone();
        let scroll = self.scroll.clone();
        let saving = self.owner_state.saving;
        div()
            .debug_selector(|| "owner-section".into())
            .relative()
            .flex()
            .flex_col()
            .gap(space(3))
            // Zero size, at the top of the section: its bounds tell where to scroll.
            .child(
                canvas(
                    move |bounds, window, _| {
                        if reveal.replace(false) {
                            let content_top = scroll.bounds().top() + scroll.offset().y;
                            scroll.set_offset(point(Pixels::ZERO, -(bounds.top() - content_top)));
                            window.refresh();
                        }
                    },
                    |_, _, _, _| {},
                )
                .absolute()
                .top_0()
                .left_0()
                .w_full()
                .h(space(1)),
            )
            .child(section("Owner", None))
            .child(
                div()
                    .text_size(size::sm())
                    .text_color(text::secondary())
                    .child("The name and the language that the roles use in their prompts. They are saved in agentsi.config.json on the host of this core."),
            )
            .children(state)
            .child(field("Name", &name))
            .child(field("Language", &language))
            .children(self.owner_state.note.as_ref().map(flash_line))
            .child(
                div().child(
                    Button::new("owner-save")
                        .primary()
                        .small()
                        .label(if saving { "Saving…" } else { "Save" })
                        .disabled(saving)
                        .on_click(cx.listener(|this, _, _, cx| this.save_owner(cx))),
                ),
            )
    }
}

#[cfg(test)]
mod tests {
    use agentsi_proto::registry_api::RegistryResult;
    use gpui::{Modifiers, TestAppContext, VisualTestContext};
    use serde_json::{Value, json};

    use crate::screens::settings::tests::support::{Setup, setup};

    fn draw(cx: &mut VisualTestContext) {
        cx.update(|window, cx| window.draw(cx).clear(cx));
    }

    fn sent(t: &mut Setup) -> Vec<Value> {
        t.core_end.set_nonblocking(true).unwrap();
        let mut out = Vec::new();
        while let Some(f) = agentsi_proto::frame::read_frame(&mut t.core_end)
            .ok()
            .flatten()
        {
            out.push(serde_json::from_slice(&f.payload).unwrap());
        }
        out
    }

    fn registry(name: &str, language: &str) -> RegistryResult {
        serde_json::from_value(json!({
            "owner": {"name": name, "language": language}, "products": []
        }))
        .unwrap()
    }

    #[gpui::test]
    fn the_owner_section_reads_the_owner_and_saves_a_change_with_registry_update(
        cx: &mut TestAppContext,
    ) {
        let (mut t, cx) = setup(cx, None);
        assert!(sent(&mut t).iter().any(|r| r["method"] == "registry.get"));
        t.screen.update(cx, |s, cx| {
            s.owner_loaded(Ok(registry("Ana", "English")), cx)
        });
        draw(cx);
        let fields = t.screen.read_with(cx, |s, cx| {
            let (n, l) = s.owner_state.inputs.clone().unwrap();
            (
                n.read(cx).value().to_string(),
                l.read(cx).value().to_string(),
            )
        });
        assert_eq!(fields, ("Ana".to_string(), "English".to_string()));

        t.screen.update_in(cx, |s, window, cx| {
            let (_, l) = s.owner_state.inputs.clone().unwrap();
            l.update(cx, |i, cx| i.set_value("Español", window, cx));
        });
        draw(cx);
        let b = cx
            .debug_bounds("button:owner-save")
            .expect("the Save button is drawn");
        cx.simulate_click(b.center(), Modifiers::none());
        let update: Vec<Value> = sent(&mut t)
            .into_iter()
            .filter(|r| r["method"] == "registry.update")
            .collect();
        assert_eq!(update.len(), 1);
        assert_eq!(
            update[0]["params"],
            json!({"owner": {"name": "Ana", "language": "Español"}})
        );
        t.screen.update(cx, |s, cx| {
            s.owner_saved(Ok(registry("Ana", "Español")), cx)
        });
        draw(cx);
    }

    #[gpui::test]
    fn an_empty_field_is_refused_and_sends_nothing(cx: &mut TestAppContext) {
        let (mut t, cx) = setup(cx, None);
        t.screen
            .update(cx, |s, cx| s.owner_loaded(Ok(registry("", "English")), cx));
        draw(cx);
        let _ = sent(&mut t);
        t.screen.update(cx, |s, cx| s.save_owner(cx));
        assert!(
            sent(&mut t)
                .iter()
                .all(|r| r["method"] != "registry.update")
        );
    }
}
