//! The Data section: the owner selects what to delete, reviews the clear plan, and
//! deletes it after one Confirm. The core does the work (`data.clear_plan`, `data.clear`)
//! and runs a backup first. Spec: `docs/specs/agentsi-desktop-clear-data.md`.
//!
//! "Delete" stays off until a plan shows. A change of the selection, of a product or of
//! a date drops the plan, so the Confirm always names the counts of the plan that the
//! core will check.

use std::cell::Cell;
use std::rc::Rc;

use agentsi_proto::clear_api::{
    ClearItem, ClearItemPlan, ClearKind, DataClear, DataClearParams, DataClearPlan,
    DataClearPlanParams, DataClearPlanResult, DataClearResult,
};
use agentsi_proto::ops_api::{DataBackup, DataBackupParams};
use agentsi_proto::rpc::ErrorKind;
use agentsi_ui::components::Button;
use agentsi_ui::tokens::{Status, line, radius, size, space, surface, text, weight};
use gpui_kit::component::Disableable as _;
use gpui_kit::component::checkbox::Checkbox;
use gpui_kit::component::input::{Input, InputEvent, InputState};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;

use super::{SettingsScreen, bytes};
use crate::link::CallError;
use crate::screens::parts::{mono, section};

/// Which choices a row has.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Scope {
    /// One product, required.
    Product,
    /// One product or all of them.
    ProductOrAll,
    /// No product choice.
    Nothing,
}

struct Row {
    kind: ClearKind,
    id: &'static str,
    title: &'static str,
    note: &'static str,
    scope: Scope,
    /// The row takes a date: only what is older than it goes.
    date: bool,
}

/// The items of the spec, in the order of its table.
const ROWS: [Row; 9] = [
    Row {
        kind: ClearKind::Product,
        id: "product",
        title: "Product",
        note: "Every row, the workspace folder and the registry entry of one product. Repos stay.",
        scope: Scope::Product,
        date: false,
    },
    Row {
        kind: ClearKind::Meetings,
        id: "meetings",
        title: "Meetings",
        note: "Minutes, priorities, decisions, snapshots, listens and call folders. A date keeps the newer ones.",
        scope: Scope::Product,
        date: true,
    },
    Row {
        kind: ClearKind::Stories,
        id: "stories",
        title: "Stories",
        note: "Done and rejected stories with their logs. Worktrees and branches stay.",
        scope: Scope::Product,
        date: false,
    },
    Row {
        kind: ClearKind::RoleLearning,
        id: "role-learning",
        title: "Role learning",
        note: "Memory, overlays, briefs and reflections.",
        scope: Scope::Product,
        date: false,
    },
    Row {
        kind: ClearKind::CallAudio,
        id: "call-audio",
        title: "Call audio",
        note: "The audio folders and their listens. Minutes stay.",
        scope: Scope::ProductOrAll,
        date: false,
    },
    Row {
        kind: ClearKind::VoiceTranscripts,
        id: "voice-transcripts",
        title: "Voice transcripts",
        note: "The files of voice/transcripts. A date keeps the newer ones.",
        scope: Scope::Nothing,
        date: true,
    },
    Row {
        kind: ClearKind::Logs,
        id: "logs",
        title: "Logs",
        note: "logs/hook.log.",
        scope: Scope::Nothing,
        date: false,
    },
    Row {
        kind: ClearKind::Backups,
        id: "backups",
        title: "Backups",
        note: "Every backup but the newest, with the registry copies.",
        scope: Scope::Nothing,
        date: false,
    },
    Row {
        kind: ClearKind::Everything,
        id: "everything",
        title: "Everything",
        note: "All of the above for every product, and the products of the registry. The data root and the newest backup stay.",
        scope: Scope::Nothing,
        date: false,
    },
];

/// The product choice of a row.
#[derive(Clone, Default, PartialEq, Eq)]
enum Choice {
    /// The active product, or the first.
    #[default]
    Default,
    Product(String),
    /// Every product. Only `Scope::ProductOrAll` rows have it.
    All,
}

/// A plan the owner reviewed.
#[derive(Clone)]
pub(super) struct Reviewed {
    items: Vec<ClearItem>,
    plan: DataClearPlanResult,
}

pub(super) enum Phase {
    Select,
    /// A call to the core runs.
    Working,
    Review(Reviewed),
    Confirm(Reviewed),
    Done(DataClearResult),
}

/// The text fields of the dates.
struct Dates {
    meetings: Entity<InputState>,
    transcripts: Entity<InputState>,
}

pub(super) struct ClearState {
    /// The data root and its size, from the last read.
    root: Option<(String, u64)>,
    checked: Vec<ClearKind>,
    /// The product choice of each row, by row index.
    choice: Vec<Choice>,
    dates: Option<Dates>,
    pub(super) phase: Phase,
    error: Option<String>,
    /// A backup runs.
    backing_up: bool,
    /// The outcome of the last "Back up now": the lines to show, or the error.
    pub(super) backup: Option<Result<Vec<String>, String>>,
    /// Set by the palette: the next draw scrolls the section into view.
    reveal: Rc<Cell<bool>>,
}

impl Default for ClearState {
    fn default() -> Self {
        ClearState {
            root: None,
            checked: Vec::new(),
            choice: vec![Choice::Default; ROWS.len()],
            dates: None,
            phase: Phase::Select,
            error: None,
            backing_up: false,
            backup: None,
            reveal: Rc::default(),
        }
    }
}

fn row_of(kind: ClearKind) -> usize {
    ROWS.iter().position(|r| r.kind == kind).unwrap_or(0)
}

/// `Meetings of alpha before 2026-01-01`.
fn name(item: &ClearItem) -> String {
    let row = &ROWS[row_of(item.item)];
    let mut text = row.title.to_string();
    match (&item.product, row.scope) {
        (Some(p), _) => text.push_str(&format!(" of {p}")),
        (None, Scope::ProductOrAll) => text.push_str(" of all products"),
        _ => {}
    }
    if let Some(d) = &item.before {
        text.push_str(&format!(" before {d}"));
    }
    text
}

/// What one item removes, in plain words.
fn describe(p: &ClearItemPlan) -> String {
    let mut parts: Vec<String> = p
        .tables
        .iter()
        .map(|t| format!("{} {}", t.table, t.rows))
        .collect();
    if !p.files.is_empty() {
        parts.push(format!(
            "{} files or folders, {}",
            p.files.len(),
            bytes(p.bytes)
        ));
    }
    if p.registry {
        parts.push("registry entry".into());
    }
    if parts.is_empty() {
        "nothing to delete".into()
    } else {
        parts.join(" · ")
    }
}

/// The sentence of the Confirm: the counts of the plan.
pub(super) fn confirm_line(plan: &DataClearPlanResult) -> String {
    format!(
        "Delete {} rows and {} files or folders ({})? A backup runs first. After it, only the backup brings them back.",
        plan.rows,
        plan.files,
        bytes(plan.bytes)
    )
}

impl SettingsScreen {
    /// Reads the data root and its size. The plan of no item gives them.
    pub(super) fn load_root(&mut self, cx: &mut Context<Self>) {
        let call = self
            .core
            .read(cx)
            .link
            .call::<DataClearPlan>(&DataClearPlanParams { items: Vec::new() });
        cx.spawn(async move |this, cx| {
            let result = call.await;
            let _ = this.update(cx, |this, cx| this.finish_root(result, cx));
        })
        .detach();
    }

    fn finish_root(
        &mut self,
        result: Result<DataClearPlanResult, CallError>,
        cx: &mut Context<Self>,
    ) {
        if let Ok(plan) = result {
            self.clear.root = Some((plan.data_root, plan.root_bytes));
            cx.notify();
        }
    }

    /// The product ids of the core, in order. The choice of a row cycles through them.
    fn clear_products(&self, cx: &App) -> Vec<String> {
        self.core
            .read(cx)
            .products
            .iter()
            .map(|p| p.id.clone())
            .collect()
    }

    /// The product of a row: the choice while it names a product of the core, else the
    /// active product, else the first. None means all products.
    fn product_of(&self, row: usize, cx: &App) -> Option<String> {
        let all = self.clear_products(cx);
        match &self.clear.choice[row] {
            Choice::All => return None,
            Choice::Product(p) if all.contains(p) => return Some(p.clone()),
            _ => {}
        }
        self.core
            .read(cx)
            .active
            .as_ref()
            .filter(|a| a.host.is_none())
            .map(|a| a.id.clone())
            .filter(|p| all.contains(p))
            .or_else(|| all.first().cloned())
    }

    fn drop_plan(&mut self) {
        self.clear.phase = Phase::Select;
        self.clear.error = None;
    }

    fn toggle_clear(&mut self, kind: ClearKind, on: bool, cx: &mut Context<Self>) {
        self.clear.checked.retain(|k| *k != kind);
        if on {
            self.clear.checked.push(kind);
        }
        self.drop_plan();
        cx.notify();
    }

    fn cycle_product(&mut self, row: usize, cx: &mut Context<Self>) {
        let all = self.clear_products(cx);
        let now = self.product_of(row, cx);
        let at = now.as_ref().and_then(|p| all.iter().position(|a| a == p));
        // The slot after the last product is "all products" where the row has it.
        self.clear.choice[row] = match (at, now) {
            (Some(i), _) if i + 1 < all.len() => Choice::Product(all[i + 1].clone()),
            (Some(_), _) if ROWS[row].scope == Scope::ProductOrAll => Choice::All,
            _ => all
                .first()
                .cloned()
                .map_or(Choice::Default, Choice::Product),
        };
        self.drop_plan();
        cx.notify();
    }

    fn clear_dates(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> (Entity<InputState>, Entity<InputState>) {
        if let Some(d) = &self.clear.dates {
            return (d.meetings.clone(), d.transcripts.clone());
        }
        let make = |placeholder: &'static str, window: &mut Window, cx: &mut Context<Self>| {
            cx.new(|cx| InputState::new(window, cx).placeholder(placeholder))
        };
        let meetings = make("older than YYYY-MM-DD", window, cx);
        let transcripts = make("older than YYYY-MM-DD", window, cx);
        for input in [&meetings, &transcripts] {
            self._inputs.push(
                cx.subscribe_in(input, window, |this, _, e: &InputEvent, _, cx| {
                    if let InputEvent::Change = e {
                        this.drop_plan();
                        cx.notify();
                    }
                }),
            );
        }
        self.clear.dates = Some(Dates {
            meetings: meetings.clone(),
            transcripts: transcripts.clone(),
        });
        (meetings, transcripts)
    }

    /// The items the owner selected, in the order of the table.
    fn selected_items(&self, cx: &App) -> Result<Vec<ClearItem>, String> {
        let mut items = Vec::new();
        for (i, row) in ROWS.iter().enumerate() {
            if !self.clear.checked.contains(&row.kind) {
                continue;
            }
            let product = match row.scope {
                Scope::Nothing => None,
                _ => self.product_of(i, cx),
            };
            if row.scope == Scope::Product && product.is_none() {
                return Err(format!(
                    "{} needs a product. This core has none.",
                    row.title
                ));
            }
            let before = self.clear.dates.as_ref().and_then(|d| {
                let input = match row.kind {
                    ClearKind::Meetings => &d.meetings,
                    ClearKind::VoiceTranscripts => &d.transcripts,
                    _ => return None,
                };
                Some(input.read(cx).value().trim().to_string()).filter(|t| !t.is_empty())
            });
            items.push(ClearItem {
                item: row.kind,
                product,
                before,
            });
        }
        if items.is_empty() {
            return Err("Select at least one item.".into());
        }
        Ok(items)
    }

    /// Asks the core for the clear plan of the selection.
    pub(super) fn review(&mut self, cx: &mut Context<Self>) {
        if matches!(self.clear.phase, Phase::Working) {
            return;
        }
        let items = match self.selected_items(cx) {
            Ok(items) => items,
            Err(e) => {
                self.clear.error = Some(e);
                cx.notify();
                return;
            }
        };
        let call = self
            .core
            .read(cx)
            .link
            .call::<DataClearPlan>(&DataClearPlanParams {
                items: items.clone(),
            });
        self.clear.error = None;
        self.clear.phase = Phase::Working;
        cx.spawn(async move |this, cx| {
            let result = call.await;
            let _ = this.update(cx, |this, cx| this.finish_review(items, result, cx));
        })
        .detach();
        cx.notify();
    }

    fn finish_review(
        &mut self,
        items: Vec<ClearItem>,
        result: Result<DataClearPlanResult, CallError>,
        cx: &mut Context<Self>,
    ) {
        match result {
            Ok(plan) => {
                self.clear.root = Some((plan.data_root.clone(), plan.root_bytes));
                self.clear.phase = Phase::Review(Reviewed { items, plan });
            }
            Err(e) => {
                self.clear.error = Some(e.to_string());
                self.clear.phase = Phase::Select;
            }
        }
        cx.notify();
    }

    /// "Delete": shows the Confirm that names the counts.
    pub(super) fn ask_delete(&mut self, cx: &mut Context<Self>) {
        if let Phase::Review(r) = &self.clear.phase {
            self.clear.phase = Phase::Confirm(r.clone());
            cx.notify();
        }
    }

    pub(super) fn cancel_delete(&mut self, cx: &mut Context<Self>) {
        if let Phase::Confirm(r) = &self.clear.phase {
            self.clear.phase = Phase::Review(r.clone());
            cx.notify();
        }
    }

    /// "Confirm": deletes the reviewed plan.
    pub(super) fn confirm_delete(&mut self, cx: &mut Context<Self>) {
        let Phase::Confirm(r) = &self.clear.phase else {
            return;
        };
        let (items, plan_id) = (r.items.clone(), r.plan.plan_id.clone());
        let call = self
            .core
            .read(cx)
            .link
            .call::<DataClear>(&DataClearParams { items, plan_id });
        let back = r.clone();
        self.clear.phase = Phase::Working;
        self.clear.error = None;
        cx.spawn(async move |this, cx| {
            let result = call.await;
            let _ = this.update(cx, |this, cx| this.finish_delete(back, result, cx));
        })
        .detach();
        cx.notify();
    }

    fn finish_delete(
        &mut self,
        back: Reviewed,
        result: Result<DataClearResult, CallError>,
        cx: &mut Context<Self>,
    ) {
        match result {
            Ok(done) => {
                self.clear.phase = Phase::Done(done);
                self.load_root(cx);
            }
            Err(CallError::Rpc(e)) if e.kind() == ErrorKind::PlanChanged => {
                self.clear.error = Some(format!("{} Review the plan again.", e.message));
                self.clear.phase = Phase::Select;
            }
            Err(e) => {
                // A busy product or a failed backup deleted nothing: the plan holds.
                self.clear.error = Some(e.to_string());
                self.clear.phase = Phase::Review(back);
            }
        }
        cx.notify();
    }

    /// "Back up now": `data.backup` with the default number of backups to keep.
    pub fn back_up_now(&mut self, cx: &mut Context<Self>) {
        if self.clear.backing_up {
            return;
        }
        self.clear.backing_up = true;
        self.clear.backup = None;
        let call = self
            .core
            .read(cx)
            .link
            .call::<DataBackup>(&DataBackupParams::default());
        cx.spawn(async move |this, cx| {
            let result = call.await;
            let _ = this.update(cx, |this, cx| this.finish_backup(result, cx));
        })
        .detach();
        cx.notify();
    }

    pub(super) fn finish_backup(
        &mut self,
        result: Result<agentsi_proto::ops_api::DataBackupResult, CallError>,
        cx: &mut Context<Self>,
    ) {
        self.clear.backing_up = false;
        self.clear.backup = Some(match result {
            Ok(done) => {
                let mut lines = vec![format!("Backup: {}", done.file)];
                if let Some(r) = done.registry {
                    lines.push(format!("Registry: {r}"));
                }
                Ok(lines)
            }
            Err(e) => Err(e.to_string()),
        });
        cx.notify();
    }

    /// Scrolls the Data section into view on the next draw. With a product, it selects the
    /// "Product" row for it.
    pub fn reveal_data(&mut self, product: Option<String>, cx: &mut Context<Self>) {
        if let Some(p) = product {
            let row = row_of(ClearKind::Product);
            self.clear.choice[row] = Choice::Product(p);
            self.clear.checked.retain(|k| *k != ClearKind::Product);
            self.clear.checked.push(ClearKind::Product);
            self.drop_plan();
        }
        self.clear.reveal.set(true);
        cx.notify();
    }

    fn data_row(
        &self,
        i: usize,
        dates: &(Entity<InputState>, Entity<InputState>),
        cx: &mut Context<Self>,
    ) -> Div {
        let row = &ROWS[i];
        let kind = row.kind;
        let on = self.clear.checked.contains(&kind);
        let busy = matches!(self.clear.phase, Phase::Working);
        let left = div()
            .flex()
            .flex_col()
            .gap(space(1))
            .flex_1()
            .min_w_0()
            .child(
                div()
                    .debug_selector(move || format!("clear-check:{}", row.id))
                    .child(
                        Checkbox::new(("clear-check", i))
                            .label(row.title)
                            .checked(on)
                            .disabled(busy)
                            .on_click(cx.listener(move |this, checked: &bool, _, cx| {
                                this.toggle_clear(kind, *checked, cx)
                            })),
                    ),
            )
            .child(
                div()
                    .text_size(size::sm())
                    .text_color(text::secondary())
                    .child(row.note),
            );
        let mut right = div().flex().items_center().gap(space(2));
        if row.scope != Scope::Nothing {
            let label = match self.product_of(i, cx) {
                Some(p) => p,
                None if row.scope == Scope::ProductOrAll => "all products".to_string(),
                None => "no product".to_string(),
            };
            right = right.child(
                Button::new(("clear-product", i))
                    .secondary()
                    .small()
                    .label(label)
                    .disabled(busy)
                    .on_click(cx.listener(move |this, _, _, cx| this.cycle_product(i, cx))),
            );
        }
        if row.date {
            let input = if kind == ClearKind::Meetings {
                &dates.0
            } else {
                &dates.1
            };
            right = right.child(div().w(space(45)).child(Input::new(input)));
        }
        pair_row(left, right)
    }

    fn plan_card(&self, r: &Reviewed, confirming: bool, cx: &mut Context<Self>) -> Div {
        let lines = r.plan.items.iter().map(|p| {
            div()
                .flex()
                .flex_col()
                .child(
                    div()
                        .text_size(size::sm())
                        .font_weight(weight::MEDIUM)
                        .text_color(text::primary())
                        .child(name(&p.item)),
                )
                .child(mono(describe(p)))
        });
        div()
            .debug_selector(|| "clear-plan".into())
            .flex()
            .flex_col()
            .gap(space(3))
            .p(space(4))
            .rounded(radius::lg())
            .border_1()
            .border_color(line::border())
            .bg(surface::raised())
            .child(section("Clear plan", None))
            .children(lines)
            .child(
                div()
                    .text_size(size::sm())
                    .text_color(text::secondary())
                    .child(format!(
                        "Total: {} rows, {} files or folders, {}.",
                        r.plan.rows,
                        r.plan.files,
                        bytes(r.plan.bytes)
                    )),
            )
            .when(confirming, |el| {
                el.child(
                    div()
                        .debug_selector(|| "clear-confirm-text".into())
                        .text_size(size::sm())
                        .text_color(Status::Danger.color())
                        .child(confirm_line(&r.plan)),
                )
                .child(
                    div()
                        .flex()
                        .gap(space(2))
                        .child(
                            Button::new("clear-confirm")
                                .danger()
                                .small()
                                .label("Confirm")
                                .on_click(cx.listener(|this, _, _, cx| this.confirm_delete(cx))),
                        )
                        .child(
                            Button::new("clear-cancel")
                                .secondary()
                                .small()
                                .label("Cancel")
                                .on_click(cx.listener(|this, _, _, cx| this.cancel_delete(cx))),
                        ),
                )
            })
    }

    fn done_card(done: &DataClearResult) -> Div {
        div()
            .debug_selector(|| "clear-done".into())
            .flex()
            .flex_col()
            .gap(space(2))
            .p(space(4))
            .rounded(radius::lg())
            .border_1()
            .border_color(line::border())
            .bg(surface::raised())
            .child(section("Deleted", None))
            .child(
                div()
                    .text_size(size::sm())
                    .text_color(text::primary())
                    .child(format!(
                        "{} rows and {} files or folders ({}).",
                        done.rows,
                        done.files,
                        bytes(done.bytes)
                    )),
            )
            .child(
                div()
                    .debug_selector(|| "clear-backup".into())
                    .child(mono(format!("Backup: {}", done.backup))),
            )
            .when_some(done.registry_backup.clone(), |el, r| {
                el.child(mono(format!("Registry backup: {r}")))
            })
            .children(done.failed.iter().map(|f| {
                div()
                    .debug_selector(|| "clear-failed".into())
                    .text_size(size::sm())
                    .text_color(Status::Warn.color())
                    .child(format!("Could not delete {}: {}", f.path, f.error))
            }))
    }

    pub(super) fn data_section(&mut self, window: &mut Window, cx: &mut Context<Self>) -> Div {
        let dates = self.clear_dates(window, cx);
        let rows: Vec<Div> = (0..ROWS.len())
            .map(|i| self.data_row(i, &dates, cx))
            .collect();
        let busy = matches!(self.clear.phase, Phase::Working);
        let reviewed = matches!(self.clear.phase, Phase::Review(_));
        let backing_up = self.clear.backing_up;
        let head = match &self.clear.root {
            Some((root, used)) => format!("{root} · {}", bytes(*used)),
            None => "Reading the data root…".to_string(),
        };
        let card = match &self.clear.phase {
            Phase::Review(r) => Some(self.plan_card(&r.clone(), false, cx)),
            Phase::Confirm(r) => Some(self.plan_card(&r.clone(), true, cx)),
            Phase::Done(d) => Some(Self::done_card(d)),
            _ => None,
        };
        let reveal = self.clear.reveal.clone();
        let scroll = self.scroll.clone();
        div()
            .debug_selector(|| "clear-section".into())
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
                            scroll.set_offset(point(
                                Pixels::ZERO,
                                -(bounds.top() - content_top),
                            ));
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
            .child(section("Data", None))
            .child(
                div()
                    .text_size(size::sm())
                    .text_color(text::secondary())
                    .child("Select what to delete, review what the delete removes, then delete. A backup runs before each delete. Repos, branches and models stay."),
            )
            .child(div().debug_selector(|| "clear-root".into()).child(mono(head)))
            .children(rows)
            .child(
                div()
                    .flex()
                    .gap(space(2))
                    .child(
                        Button::new("clear-review")
                            .secondary()
                            .small()
                            .label("Review")
                            .disabled(busy)
                            .on_click(cx.listener(|this, _, _, cx| this.review(cx))),
                    )
                    .child(
                        Button::new("clear-delete")
                            .danger()
                            .small()
                            .label("Delete")
                            .disabled(!reviewed)
                            .on_click(cx.listener(|this, _, _, cx| this.ask_delete(cx))),
                    )
                    .child(
                        Button::new("backup-now")
                            .secondary()
                            .small()
                            .label(if backing_up { "Backing up…" } else { "Back up now" })
                            .disabled(backing_up)
                            .on_click(cx.listener(|this, _, _, cx| this.back_up_now(cx))),
                    ),
            )
            .when_some(self.clear.backup.clone(), |el, outcome| {
                let lines = match outcome {
                    Ok(lines) => lines,
                    Err(e) => vec![format!("The backup failed: {e}")],
                };
                el.child(div().debug_selector(|| "backup-outcome".into()).children(
                    lines.into_iter().map(mono),
                ))
            })
            .when_some(self.clear.error.clone(), |el, e| {
                el.child(
                    div()
                        .debug_selector(|| "clear-error".into())
                        .text_size(size::sm())
                        .text_color(Status::Danger.color())
                        .child(e),
                )
            })
            .when_some(card, |el, c| el.child(c))
    }
}

/// A row of the table: the choice on the left, the product and date on the right.
fn pair_row(left: Div, right: Div) -> Div {
    div()
        .flex()
        .items_start()
        .justify_between()
        .gap(space(4))
        .py(space(2))
        .border_b_1()
        .border_color(line::border())
        .child(left)
        .child(right)
}

#[cfg(test)]
mod tests {

    use agentsi_proto::clear_api::{DataClearPlanResult, DataClearResult};
    use agentsi_proto::desk_api::ProductInfo;
    use gpui::{Modifiers, TestAppContext, VisualTestContext};
    use serde_json::{Value, json};

    use super::{CallError, ClearItem, ClearKind, Phase, Reviewed, confirm_line};
    use crate::screens::settings::tests::support::{Setup, setup};

    fn draw(cx: &mut VisualTestContext) {
        cx.update(|window, cx| window.draw(cx).clear(cx));
    }

    /// The plan is the last section: the rows and the buttons need more than one screen,
    /// and a plan card grows the page.
    fn bottom(t: &Setup, cx: &mut VisualTestContext) {
        t.screen.update(cx, |s, _| s.scroll.scroll_to_bottom());
        draw(cx);
    }

    fn click(t: &Setup, cx: &mut VisualTestContext, selector: &'static str) {
        bottom(t, cx);
        let b = cx
            .debug_bounds(selector)
            .unwrap_or_else(|| panic!("{selector} is drawn"));
        cx.simulate_click(b.center(), Modifiers::none());
        draw(cx);
    }

    /// Reads the requests of the screen up to the next `method`.
    fn request_after(t: &mut Setup, method: &str) -> Value {
        t.core_end.set_nonblocking(true).unwrap();
        loop {
            let frame = agentsi_proto::frame::read_frame(&mut t.core_end)
                .ok()
                .flatten()
                .unwrap_or_else(|| panic!("the screen sent no {method}"));
            let v: Value = serde_json::from_slice(&frame.payload).unwrap();
            if v["method"] == method {
                return v;
            }
        }
    }

    fn plan_for(items: &[ClearItem], rows: u64) -> DataClearPlanResult {
        let listed: Vec<Value> = items
            .iter()
            .map(|i| {
                let mut p = serde_json::to_value(i).unwrap();
                p["tables"] = json!([{"table": "minutes", "rows": rows}]);
                p["files"] =
                    json!([{"path": "workspace/alpha/audio/m1", "bytes": 2048, "dir": true}]);
                p["rows"] = json!(rows);
                p["bytes"] = json!(2048);
                p["registry"] = json!(false);
                p
            })
            .collect();
        serde_json::from_value(json!({
            "plan_id": "plan-1", "data_root": "/data/root", "root_bytes": 2_500_000,
            "items": listed, "rows": rows * listed.len() as u64,
            "files": listed.len(), "bytes": 2048 * listed.len() as u64
        }))
        .unwrap()
    }

    fn with_products(t: &Setup, cx: &mut VisualTestContext) {
        let core = t.screen.read_with(cx, |s, _| s.core.clone());
        let info = |id: &str| ProductInfo {
            id: id.into(),
            name: id.into(),
            path: None,
            archived: false,
        };
        core.update(cx, |c, _| c.products = vec![info("alpha"), info("beta")]);
    }

    fn app_size(cx: &mut VisualTestContext) {
        cx.simulate_resize(gpui::size(
            agentsi_ui::tokens::control::WINDOW_WIDTH,
            agentsi_ui::tokens::control::WINDOW_HEIGHT,
        ));
        draw(cx);
    }

    fn phase(t: &Setup, cx: &mut VisualTestContext) -> &'static str {
        t.screen.read_with(cx, |s, _| match s.clear.phase {
            Phase::Select => "select",
            Phase::Working => "working",
            Phase::Review(_) => "review",
            Phase::Confirm(_) => "confirm",
            Phase::Done(_) => "done",
        })
    }

    /// The plan that the screen holds in its Confirm.
    fn reviewed(t: &Setup, cx: &mut VisualTestContext) -> Reviewed {
        t.screen.read_with(cx, |s, _| match &s.clear.phase {
            Phase::Working => panic!("a delete runs"),
            Phase::Confirm(r) | Phase::Review(r) => r.clone(),
            _ => panic!("no plan"),
        })
    }

    fn rpc_error(kind: &str, message: &str) -> CallError {
        CallError::Rpc(
            serde_json::from_value(json!({
                "code": -32000, "message": message, "data": {"kind": kind, "hint": "Try again."}}))
            .unwrap(),
        )
    }

    /// Selects `id`, asks for the plan and takes the plan the core would send.
    fn review(t: &mut Setup, cx: &mut VisualTestContext, check: &'static str, rows: u64) -> Value {
        click(t, cx, check);
        click(t, cx, "button:clear-review");
        let asked = request_after(t, "data.clear_plan");
        let items: Vec<ClearItem> =
            serde_json::from_value(asked["params"]["items"].clone()).unwrap();
        t.screen.update(cx, |s, cx| {
            s.finish_review(items.clone(), Ok(plan_for(&items, rows)), cx)
        });
        draw(cx);
        asked
    }

    #[gpui::test]
    fn the_section_selects_reviews_confirms_and_shows_the_backup(cx: &mut TestAppContext) {
        let (mut t, cx) = setup(cx, None);
        app_size(cx);
        with_products(&t, cx);
        draw(cx);

        // The section reads the data root and its size when it opens.
        let root = request_after(&mut t, "data.clear_plan");
        assert_eq!(root["params"]["items"], json!([]));
        t.screen
            .update(cx, |s, cx| s.finish_root(Ok(plan_for(&[], 0)), cx));
        draw(cx);
        assert_eq!(
            t.screen.read_with(cx, |s, _| s.clear.root.clone()),
            Some(("/data/root".to_string(), 2_500_000))
        );

        // Delete stays off until a plan shows, and a click on it sends nothing.
        click(&t, cx, "button:clear-delete");
        assert_eq!(phase(&t, cx), "select");

        let asked = review(&mut t, cx, "clear-check:meetings", 41);
        assert!(
            t.screen
                .read_with(cx, |s, _| s.clear.checked == [ClearKind::Meetings])
        );
        assert_eq!(
            asked["params"]["items"],
            json!([{"item": "meetings", "product": "alpha"}])
        );
        assert_eq!(phase(&t, cx), "review");
        assert!(cx.debug_bounds("clear-plan").is_some(), "the plan shows");

        click(&t, cx, "button:clear-delete");
        assert_eq!(phase(&t, cx), "confirm");
        assert!(cx.debug_bounds("clear-confirm-text").is_some());
        let plan = reviewed(&t, cx).plan;
        assert_eq!(
            confirm_line(&plan),
            "Delete 41 rows and 1 files or folders (2.0 KB)? A backup runs first. After it, only the backup brings them back."
        );

        // Confirm sends the items and the id of the plan that the owner saw.
        let seen = reviewed(&t, cx);
        click(&t, cx, "button:clear-confirm");
        assert_eq!(phase(&t, cx), "working");
        let del = request_after(&mut t, "data.clear");
        assert_eq!(del["params"]["plan_id"], "plan-1");
        assert_eq!(del["params"]["items"], asked["params"]["items"]);
        let backup = "/data/root/backups/agentsi-2026-11-07.db";
        let done: DataClearResult = serde_json::from_value(json!({
            "backup": backup, "registry_backup": null, "rows": 41, "files": 0, "bytes": 2048,
            "registry_removed": [],
            "failed": [{"path": "workspace/alpha/audio/m1", "error": "denied"}]}))
        .unwrap();
        t.screen
            .update(cx, |s, cx| s.finish_delete(seen, Ok(done), cx));
        draw(cx);
        assert_eq!(phase(&t, cx), "done");
        bottom(&t, cx);
        assert!(
            cx.debug_bounds("clear-backup").is_some(),
            "the backup path shows"
        );
        assert!(
            cx.debug_bounds("clear-failed").is_some(),
            "the file that stayed shows"
        );
        let shown = t.screen.read_with(cx, |s, _| match &s.clear.phase {
            Phase::Done(d) => d.backup.clone(),
            _ => String::new(),
        });
        assert_eq!(shown, backup);
        // The section reads the size again.
        request_after(&mut t, "data.clear_plan");
    }

    #[gpui::test]
    fn a_change_after_the_review_drops_the_plan_and_a_refusal_keeps_it(cx: &mut TestAppContext) {
        let (mut t, cx) = setup(cx, None);
        app_size(cx);
        with_products(&t, cx);
        draw(cx);
        review(&mut t, cx, "clear-check:logs", 0);

        // A busy product: the delete is refused, nothing went, and the plan holds.
        click(&t, cx, "button:clear-delete");
        let seen = reviewed(&t, cx);
        click(&t, cx, "button:clear-confirm");
        request_after(&mut t, "data.clear");
        t.screen.update(cx, |s, cx| {
            s.finish_delete(
                seen,
                Err(rpc_error(
                    "busy",
                    "Running work blocks the delete: terminal t1.",
                )),
                cx,
            )
        });
        draw(cx);
        assert_eq!(phase(&t, cx), "review");
        assert!(cx.debug_bounds("clear-error").is_some());

        // A plan that no longer matches needs a new review.
        click(&t, cx, "button:clear-delete");
        let seen = reviewed(&t, cx);
        click(&t, cx, "button:clear-confirm");
        request_after(&mut t, "data.clear");
        t.screen.update(cx, |s, cx| {
            s.finish_delete(
                seen,
                Err(rpc_error(
                    "plan_changed",
                    "The data changed since the plan.",
                )),
                cx,
            )
        });
        draw(cx);
        assert_eq!(phase(&t, cx), "select");

        // A new choice drops a shown plan, and Delete is off again.
        review(&mut t, cx, "clear-check:backups", 0);
        assert_eq!(phase(&t, cx), "review");
        click(&t, cx, "clear-check:stories");
        assert_eq!(phase(&t, cx), "select");
        click(&t, cx, "button:clear-delete");
        assert_eq!(phase(&t, cx), "select", "Delete is off again");
        assert!(quiet(&mut t), "no delete was sent");
    }

    /// True when the screen sent nothing more.
    fn quiet(t: &mut Setup) -> bool {
        t.core_end.set_nonblocking(true).unwrap();
        loop {
            match agentsi_proto::frame::read_frame(&mut t.core_end)
                .ok()
                .flatten()
            {
                None => return true,
                Some(f) => {
                    let v: Value = serde_json::from_slice(&f.payload).unwrap();
                    if v["method"] == "data.clear" {
                        return false;
                    }
                }
            }
        }
    }

    #[gpui::test]
    fn the_palette_entry_scrolls_to_the_data_section_and_a_product_selects_its_row(
        cx: &mut TestAppContext,
    ) {
        let (t, cx) = setup(cx, None);
        app_size(cx);
        with_products(&t, cx);
        draw(cx);
        let window = f32::from(agentsi_ui::tokens::control::WINDOW_HEIGHT);
        let top =
            |cx: &mut VisualTestContext| f32::from(cx.debug_bounds("clear-section").unwrap().top());
        assert!(
            top(cx) > window,
            "the Data section is below the first screen"
        );
        t.screen
            .update(cx, |s, cx| s.reveal_data(Some("beta".into()), cx));
        draw(cx);
        draw(cx);
        assert!(top(cx) < window / 2., "the section is in view: {}", top(cx));
        let (checked, choice) = t
            .screen
            .read_with(cx, |s, cx| (s.clear.checked.clone(), s.product_of(0, cx)));
        assert_eq!(checked, [ClearKind::Product]);
        assert_eq!(choice.as_deref(), Some("beta"));
    }
}
