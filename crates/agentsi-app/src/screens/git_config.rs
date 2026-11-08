//! The editor of the git config file (`git.toml`). The git screen shows it in its Config tab
//! with the shared table, the table of the product and the result. The settings screen shows
//! it as the section "Git" with the shared table only.
//!
//! Each save sends one whole table to the core. A key that starts a program (`core.sshCommand`,
//! `credential.helper`, any `alias.*`) asks one Confirm first, because the agents then run that
//! program.

use agentsi_proto::git_api::{
    GitConfigEntry, GitConfigGet, GitConfigGetParams, GitConfigResult, GitConfigSet,
    GitConfigSetParams, starts_program,
};
use agentsi_ui::components::{Button, neutral_tag, status_tag};
use agentsi_ui::tokens::{Status, line, radius, size, space, surface, text};
use gpui_kit::component::input::{Input, InputEvent, InputState};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;

use super::parts::{Load, mono, section};
use crate::model::CoreState;

/// The table that an action writes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Table {
    Shared,
    Product,
}

/// A save that waits for the Confirm of the owner.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Save {
    pub table: Table,
    pub entries: Vec<GitConfigEntry>,
    /// The keys that start a program.
    pub programs: Vec<String>,
}

/// The table with `key` set to `value`. A key that git reads the same way replaces the old one.
pub fn with_key(entries: &[GitConfigEntry], key: &str, value: &str) -> Vec<GitConfigEntry> {
    let mut out = without_key(entries, key);
    out.push(GitConfigEntry {
        key: key.into(),
        value: value.into(),
    });
    out
}

pub fn without_key(entries: &[GitConfigEntry], key: &str) -> Vec<GitConfigEntry> {
    entries
        .iter()
        .filter(|e| !e.key.eq_ignore_ascii_case(key))
        .cloned()
        .collect()
}

/// The keys of `new` that start a program and are not in `old` with the same value.
pub fn programs_added(old: &[GitConfigEntry], new: &[GitConfigEntry]) -> Vec<String> {
    new.iter()
        .filter(|e| starts_program(&e.key) && !old.contains(e))
        .map(|e| e.key.clone())
        .collect()
}

/// The words under the editor. A pane keeps the values that it started with.
pub const PANE_NOTE: &str = "A pane that runs now keeps its old values. A new pane, story session or meeting gets the new ones.";

struct Form {
    key: Entity<InputState>,
    value: Entity<InputState>,
}

pub struct GitConfigEditor {
    /// The product of the second table, or none for the shared table only.
    product: Option<String>,
    core: Entity<CoreState>,
    data: Load<GitConfigResult>,
    form: Option<Form>,
    /// The table that the form adds to.
    target: Table,
    pending: Option<Save>,
    note: Option<String>,
    _inputs: Vec<Subscription>,
}

impl GitConfigEditor {
    pub fn new(product: Option<String>, core: Entity<CoreState>, cx: &mut Context<Self>) -> Self {
        let mut editor = GitConfigEditor {
            product,
            core,
            data: Load::Loading,
            form: None,
            target: Table::Shared,
            pending: None,
            note: None,
            _inputs: Vec::new(),
        };
        editor.reload(cx);
        editor
    }

    pub fn reload(&mut self, cx: &mut Context<Self>) {
        let call = self
            .core
            .read(cx)
            .link
            .call::<GitConfigGet>(&GitConfigGetParams {
                product: self.product.clone(),
            });
        cx.spawn(async move |this, cx| {
            let result = call.await.map_err(|e| e.to_string());
            let _ = this.update(cx, |this, cx| this.loaded(result, cx));
        })
        .detach();
    }

    /// The answer of `git.config.get`.
    pub fn loaded(&mut self, result: Result<GitConfigResult, String>, cx: &mut Context<Self>) {
        self.data.settle(result);
        cx.notify();
    }

    fn table(&self, table: Table) -> Vec<GitConfigEntry> {
        match (&self.data, table) {
            (Load::Ready(d), Table::Shared) => d.shared.clone(),
            (Load::Ready(d), Table::Product) => d.product.clone(),
            _ => Vec::new(),
        }
    }

    /// Asks for the Confirm when a key starts a program, else saves at once.
    pub fn request_save(
        &mut self,
        table: Table,
        entries: Vec<GitConfigEntry>,
        cx: &mut Context<Self>,
    ) {
        self.note = None;
        let programs = programs_added(&self.table(table), &entries);
        let save = Save {
            table,
            entries,
            programs,
        };
        if save.programs.is_empty() {
            self.pending = None;
            self.send(save, cx);
        } else {
            self.pending = Some(save);
            cx.notify();
        }
    }

    pub fn confirm(&mut self, cx: &mut Context<Self>) {
        if let Some(save) = self.pending.take() {
            self.send(save, cx);
        }
    }

    pub fn cancel(&mut self, cx: &mut Context<Self>) {
        self.pending = None;
        cx.notify();
    }

    #[cfg(test)]
    pub fn pending(&self) -> Option<&Save> {
        self.pending.as_ref()
    }

    fn send(&mut self, save: Save, cx: &mut Context<Self>) {
        let product = match save.table {
            Table::Shared => None,
            Table::Product => self.product.clone(),
        };
        let call = self
            .core
            .read(cx)
            .link
            .call::<GitConfigSet>(&GitConfigSetParams {
                product,
                entries: save.entries,
            });
        cx.spawn(async move |this, cx| {
            let result = call.await.map_err(|e| e.to_string());
            let _ = this.update(cx, |this, cx| this.saved(result, cx));
        })
        .detach();
    }

    /// The answer of `git.config.set`.
    pub fn saved(&mut self, result: Result<GitConfigResult, String>, cx: &mut Context<Self>) {
        match result {
            Ok(r) => {
                self.data = Load::Ready(r);
                self.note = None;
            }
            Err(e) => self.note = Some(format!("The git config was not saved: {e}")),
        }
        cx.notify();
    }

    fn remove(&mut self, table: Table, key: &str, cx: &mut Context<Self>) {
        let entries = without_key(&self.table(table), key);
        self.request_save(table, entries, cx);
    }

    fn edit(
        &mut self,
        table: Table,
        entry: &GitConfigEntry,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.target = table;
        if let Some(f) = &self.form {
            let (k, v) = (entry.key.clone(), entry.value.clone());
            f.key.update(cx, |i, cx| i.set_value(k, window, cx));
            f.value.update(cx, |i, cx| i.set_value(v, window, cx));
        }
        cx.notify();
    }

    /// Adds the key and value of the form to the target table.
    pub fn add_from_form(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some((key_input, value_input)) =
            self.form.as_ref().map(|f| (f.key.clone(), f.value.clone()))
        else {
            return;
        };
        let key = key_input.read(cx).value().trim().to_string();
        let value = value_input.read(cx).value().to_string();
        if key.is_empty() {
            self.note = Some("Write the key first, for example user.email.".into());
            cx.notify();
            return;
        }
        let table = if self.product.is_some() {
            self.target
        } else {
            Table::Shared
        };
        let entries = with_key(&self.table(table), &key, &value);
        self.request_save(table, entries, cx);
        key_input.update(cx, |i, cx| i.set_value("", window, cx));
        value_input.update(cx, |i, cx| i.set_value("", window, cx));
    }

    /// Sets the form fields, for the tests.
    #[cfg(test)]
    pub fn fill(&mut self, key: &str, value: &str, window: &mut Window, cx: &mut Context<Self>) {
        let (k, v) = self.form(window, cx);
        k.update(cx, |i, cx| i.set_value(key.to_string(), window, cx));
        v.update(cx, |i, cx| i.set_value(value.to_string(), window, cx));
    }

    fn form(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> (Entity<InputState>, Entity<InputState>) {
        if let Some(f) = &self.form {
            return (f.key.clone(), f.value.clone());
        }
        let key =
            cx.new(|cx| InputState::new(window, cx).placeholder("Key, for example user.email"));
        let value = cx.new(|cx| InputState::new(window, cx).placeholder("Value"));
        for input in [&key, &value] {
            self._inputs.push(cx.subscribe_in(
                input,
                window,
                |this, _, e: &InputEvent, window, cx| {
                    if let InputEvent::PressEnter { .. } = e {
                        this.add_from_form(window, cx);
                    }
                },
            ));
        }
        self.form = Some(Form {
            key: key.clone(),
            value: value.clone(),
        });
        (key, value)
    }
}

impl GitConfigEditor {
    /// One table: its rows with Edit and Remove.
    fn rows(
        &self,
        id: &'static str,
        table: Table,
        entries: &[GitConfigEntry],
        cx: &mut Context<Self>,
    ) -> Div {
        let col = div().flex().flex_col().gap(space(1));
        if entries.is_empty() {
            return col.child(mono("No keys."));
        }
        col.children(entries.iter().enumerate().map(|(i, e)| {
            let (edit, key) = (e.clone(), e.key.clone());
            let shown = if e.value.is_empty() {
                format!("{} = (removed for this product)", e.key)
            } else {
                format!("{} = {}", e.key, e.value)
            };
            div()
                .flex()
                .items_center()
                .gap(space(2))
                .child(div().flex_1().min_w_0().child(mono(shown)))
                .when(starts_program(&e.key), |el| {
                    el.child(status_tag("starts a program", Status::Warn))
                })
                .child(
                    Button::new((id, i * 2))
                        .ghost()
                        .small()
                        .label("Edit")
                        .on_click(cx.listener(move |this, _, window, cx| {
                            this.edit(table, &edit, window, cx)
                        })),
                )
                .child(
                    Button::new((id, i * 2 + 1))
                        .ghost()
                        .small()
                        .label("Remove")
                        .on_click(cx.listener(move |this, _, _, cx| this.remove(table, &key, cx))),
                )
        }))
    }

    fn confirm_card(&self, cx: &mut Context<Self>) -> Option<Div> {
        let save = self.pending.as_ref()?;
        let lines: Vec<String> = save
            .programs
            .iter()
            .map(|k| format!("{k} makes git start a program."))
            .chain(["AgentSI and the agents then run that program in this product.".to_string()])
            .collect();
        Some(
            div()
                .flex()
                .flex_col()
                .gap(space(3))
                .p(space(4))
                .rounded(radius::lg())
                .border_1()
                .border_color(line::BORDER)
                .bg(surface::RAISED)
                .child(section("Save a key that starts a program", None))
                .children(lines.into_iter().map(|l| {
                    div()
                        .text_size(size::sm())
                        .text_color(text::SECONDARY)
                        .child(l)
                }))
                .child(
                    div()
                        .flex()
                        .gap(space(2))
                        .child(
                            Button::new("git-config-confirm")
                                .danger()
                                .small()
                                .label("Confirm")
                                .on_click(cx.listener(|this, _, _, cx| this.confirm(cx))),
                        )
                        .child(
                            Button::new("git-config-cancel")
                                .secondary()
                                .small()
                                .label("Cancel")
                                .on_click(cx.listener(|this, _, _, cx| this.cancel(cx))),
                        ),
                ),
        )
    }

    fn target_button(
        &self,
        id: &'static str,
        table: Table,
        label: &'static str,
        cx: &mut Context<Self>,
    ) -> Button {
        let b = Button::new(id).small().label(label);
        let b = if self.target == table {
            b.primary()
        } else {
            b.secondary()
        };
        b.on_click(cx.listener(move |this, _, _, cx| {
            this.target = table;
            cx.notify();
        }))
    }
}

impl Render for GitConfigEditor {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let root = div().flex().flex_col().gap(space(3));
        let d = match &self.data {
            Load::Loading => return root.child(mono("Reading the git config…")),
            Load::Failed(e) => {
                return root.child(
                    div()
                        .text_size(size::sm())
                        .text_color(Status::Danger.color())
                        .child(format!("The git config was not read: {e}")),
                );
            }
            Load::Ready(d) => d.clone(),
        };
        let (key, value) = self.form(window, cx);
        let two = self.product.is_some();
        root.child(section("Shared for all products", Some(d.shared.len())))
            .child(self.rows("git-shared", Table::Shared, &d.shared, cx))
            .when(two, |el| {
                el.child(section("Override for this product", Some(d.product.len())))
                    .child(self.rows("git-product", Table::Product, &d.product, cx))
                    .child(section("Result for this product", Some(d.result.len())))
                    .child(
                        div()
                            .flex()
                            .flex_col()
                            .gap(space(1))
                            .children(d.result.iter().map(|r| {
                                div()
                                    .flex()
                                    .items_center()
                                    .gap(space(2))
                                    .child(mono(format!("{} = {}", r.key, r.value)))
                                    .child(neutral_tag(r.source.clone()))
                            })),
                    )
            })
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap(space(2))
                    .child(div().flex_1().min_w_0().child(Input::new(&key)))
                    .child(div().flex_1().min_w_0().child(Input::new(&value)))
                    .when(two, |el| {
                        el.child(self.target_button(
                            "git-target-shared",
                            Table::Shared,
                            "Shared",
                            cx,
                        ))
                        .child(self.target_button(
                            "git-target-product",
                            Table::Product,
                            "This product",
                            cx,
                        ))
                    })
                    .child(
                        Button::new("git-config-add")
                            .primary()
                            .small()
                            .label("Save key")
                            .on_click(
                                cx.listener(|this, _, window, cx| this.add_from_form(window, cx)),
                            ),
                    ),
            )
            .when(two, |el| {
                el.child(mono(
                    "An empty value in the override removes the shared key for this product.",
                ))
            })
            .children(self.confirm_card(cx))
            .when_some(self.note.clone(), |el, n| {
                el.child(
                    div()
                        .debug_selector(|| "git-config-note".into())
                        .text_size(size::sm())
                        .text_color(Status::Danger.color())
                        .child(n),
                )
            })
            .child(mono(format!("{} · {PANE_NOTE}", d.path)))
    }
}

#[cfg(test)]
mod tests {
    use super::{GitConfigEntry, programs_added, with_key, without_key};

    fn e(key: &str, value: &str) -> GitConfigEntry {
        GitConfigEntry {
            key: key.into(),
            value: value.into(),
        }
    }

    #[test]
    fn a_key_replaces_the_old_one_that_git_reads_the_same_way() {
        let t = vec![e("user.name", "A"), e("user.email", "a@x")];
        let got = with_key(&t, "User.Email", "b@x");
        assert_eq!(got, vec![e("user.name", "A"), e("User.Email", "b@x")]);
        assert_eq!(without_key(&got, "user.name"), vec![e("User.Email", "b@x")]);
    }

    #[test]
    fn only_a_new_or_changed_program_key_needs_a_confirm() {
        let old = vec![e("core.sshCommand", "ssh -i k"), e("user.name", "A")];
        // The same table again, and a plain key, need none.
        assert!(programs_added(&old, &old).is_empty());
        assert!(programs_added(&old, &with_key(&old, "user.email", "a@x")).is_empty());
        // A changed value of a program key and a new alias do.
        assert_eq!(
            programs_added(&old, &with_key(&old, "core.sshCommand", "ssh -i other")),
            ["core.sshCommand"]
        );
        assert_eq!(
            programs_added(&old, &with_key(&old, "alias.st", "!sh -c x")),
            ["alias.st"]
        );
        // Removing a program key needs none.
        assert!(programs_added(&old, &without_key(&old, "core.sshCommand")).is_empty());
    }
}

#[cfg(test)]
mod editor_tests {
    use super::{GitConfigEditor, Table};
    use crate::link::Link;
    use crate::model::{CoreState, two_cores::received};
    use agentsi_proto::git_api::GitConfigResult;
    use gpui::{AppContext as _, Entity, TestAppContext, VisualTestContext};
    use serde_json::json;
    use std::os::unix::net::UnixStream;
    use std::sync::Arc;

    struct Ui<'a> {
        editor: Entity<GitConfigEditor>,
        sock: UnixStream,
        cx: &'a mut VisualTestContext,
    }

    fn result(shared: serde_json::Value) -> GitConfigResult {
        serde_json::from_value(json!({
            "path": "/c/agentsi/git.toml", "shared": shared, "product": [], "result": [],
        }))
        .unwrap()
    }

    fn ui(cx: &mut TestAppContext) -> Ui<'_> {
        cx.update(agentsi_ui::init);
        let (end, sock) = UnixStream::pair().unwrap();
        sock.set_nonblocking(true).unwrap();
        let core = cx.new(|_| CoreState::new(Arc::new(Link::for_tests(end))));
        let (editor, cx) = cx.add_window_view(|_, cx| GitConfigEditor::new(None, core, cx));
        let mut ui = Ui { editor, sock, cx };
        assert_eq!(ui.sent(), ["git.config.get"]);
        ui.editor
            .update(ui.cx, |e, cx| e.loaded(Ok(result(json!([]))), cx));
        ui.cx.update(|window, cx| window.draw(cx).clear(cx));
        ui
    }

    impl Ui<'_> {
        fn sent(&mut self) -> Vec<String> {
            received(&mut self.sock)
                .iter()
                .map(|r| r["method"].as_str().unwrap_or_default().to_string())
                .collect()
        }

        fn add(&mut self, key: &str, value: &str) {
            let (key, value) = (key.to_string(), value.to_string());
            self.editor.update_in(self.cx, |e, window, cx| {
                e.fill(&key, &value, window, cx);
                e.add_from_form(window, cx);
            });
        }
    }

    #[gpui::test]
    fn a_plain_key_is_saved_at_once(cx: &mut TestAppContext) {
        let mut ui = ui(cx);
        ui.add("user.email", "me@example.invalid");
        let sent = received(&mut ui.sock);
        assert_eq!(sent.len(), 1);
        assert_eq!(sent[0]["method"], "git.config.set");
        assert_eq!(
            sent[0]["params"],
            json!({"entries": [{"key": "user.email", "value": "me@example.invalid"}]})
        );
        assert!(ui.editor.read_with(ui.cx, |e, _| e.pending().is_none()));
    }

    #[gpui::test]
    fn a_key_that_starts_a_program_asks_one_confirm_and_sends_nothing_before_it(
        cx: &mut TestAppContext,
    ) {
        let mut ui = ui(cx);
        ui.add("core.sshCommand", "ssh -i /k");
        assert_eq!(
            ui.sent(),
            Vec::<String>::new(),
            "nothing is sent before the Confirm"
        );
        ui.editor.read_with(ui.cx, |e, _| {
            let p = e.pending().unwrap();
            assert_eq!(
                (p.table, p.programs.clone()),
                (Table::Shared, vec!["core.sshCommand".to_string()])
            );
        });
        ui.cx.update(|window, cx| window.draw(cx).clear(cx));
        // Cancel drops it. Nothing is sent.
        ui.editor.update(ui.cx, |e, cx| e.cancel(cx));
        assert_eq!(ui.sent(), Vec::<String>::new());
        // Asked again, one Confirm sends it, one time.
        ui.add("core.sshCommand", "ssh -i /k");
        ui.editor.update(ui.cx, |e, cx| e.confirm(cx));
        let sent = received(&mut ui.sock);
        assert_eq!(sent.len(), 1);
        assert_eq!(sent[0]["method"], "git.config.set");
        assert_eq!(sent[0]["params"]["entries"][0]["key"], "core.sshCommand");
        assert!(ui.editor.read_with(ui.cx, |e, _| e.pending().is_none()));
    }

    #[gpui::test]
    fn a_refused_save_shows_its_error_and_keeps_the_table(cx: &mut TestAppContext) {
        let ui = ui(cx);
        ui.editor.update(ui.cx, |e, cx| {
            e.saved(Err("\"name\" is not a git config key".into()), cx)
        });
        ui.cx.update(|window, cx| window.draw(cx).clear(cx));
        assert!(ui.cx.debug_bounds("git-config-note").is_some());
    }
}
