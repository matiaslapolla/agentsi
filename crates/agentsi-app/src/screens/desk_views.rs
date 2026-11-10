//! The views of the desk conversation other than the decisions view.
//!
//! An object view keeps only the id that Claude gave. It reads the stored object from the core
//! and reads it again on `data.changed` of its kind: `minutes` for priorities, `stories` for a
//! story and a diff, `tickets` for tickets. A button that writes asks a Confirm first. A button that would run a
//! story or spend money only opens the story screen.
//!
//! A generic view (table, chart, checklist, page) draws the small data that Claude gave. The
//! core checked its limits. A check in a checklist calls `desk.check`, which stores a desk note
//! and writes no product data. A page is never drawn here: "Open in browser" writes it to a
//! file with mode 0600 and opens the file.
//!
//! Spec: `docs/specs/agentsi-desktop-conversation-views.md`.

use std::collections::HashMap;
use std::io::Write as _;
use std::os::unix::fs::{OpenOptionsExt as _, PermissionsExt as _};
use std::path::{Path, PathBuf};

use agentsi_proto::desk_api::{MinutesDetail, MinutesGet, MinutesGetParams, PriorityInfo};
use agentsi_proto::desk_session_api::{
    ChartKind, DeskCheck, DeskCheckParams, DeskPage, DeskPageParams, DeskPageResult, DeskSeqResult,
    View,
};
use agentsi_proto::marks_api::{Mark, MarkEntry, MarkSet, MarkSetParams, MarkSetResult};
use agentsi_proto::story_api::{
    StoryGet, StoryGetResult, StoryIdParams, StoryReview, StoryReviewResult,
};
use agentsi_proto::ticket_api::{Ticket, TicketGet, TicketGetParams, TicketGetResult};
use agentsi_ui::components::{Button, Card, ListRow, neutral_tag};
use agentsi_ui::tokens::{WorkState, control, line, radius, size, space, surface, text, weight};
use gpui_kit::component::ActiveTheme as _;
use gpui_kit::component::chart::{AreaChart, BarChart, LineChart, PieChart};
use gpui_kit::component::checkbox::Checkbox;
use gpui_kit::component::diff::{Diff, DiffFile, DiffParseOptions, DiffState};
use gpui_kit::component::table::{Table, TableBody, TableCell, TableHead, TableHeader, TableRow};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;

use super::{Confirm, DeskConversation, Row};
use crate::link::CallError;
use crate::nav::{Nav, ScreenRequest};
use crate::screens::parts::{Flash, Live, Load, mark_tag, mono, role_label};
use crate::screens::stories::story_tag;
use crate::screens::story::DIFF_LINES;

/// What a view row knows of its stored item, apart from the view.
#[derive(Clone, Debug, Default, PartialEq)]
pub(super) struct Shown {
    pub(super) seq: i64,
    /// A checklist: the items that the owner checked.
    pub(super) checked: Vec<u32>,
    /// A page: the characters of its HTML. The app reads the HTML only to open it.
    pub(super) html_len: Option<u64>,
}

/// The folder of the page files of this device: `$XDG_RUNTIME_DIR/agentsi/pages`
/// (`$TMPDIR` on macOS). `None` when the variable is not set.
pub(super) fn pages_dir() -> Option<PathBuf> {
    let var = if cfg!(target_os = "macos") {
        "TMPDIR"
    } else {
        "XDG_RUNTIME_DIR"
    };
    std::env::var_os(var)
        .filter(|v| !v.is_empty())
        .map(|base| PathBuf::from(base).join("agentsi").join("pages"))
}

/// Writes a page to `<dir>/<product>-<seq>.html`. Only the owner can read the folder and the
/// file, because the page can hold product data.
pub(super) fn write_page(
    dir: &Path,
    product: &str,
    seq: i64,
    html: &str,
) -> std::io::Result<PathBuf> {
    std::fs::create_dir_all(dir)?;
    std::fs::set_permissions(dir, std::fs::Permissions::from_mode(0o700))?;
    let name: String = product
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '-' || c == '_' {
                c
            } else {
                '_'
            }
        })
        .collect();
    let path = dir.join(format!("{name}-{seq}.html"));
    let mut file = std::fs::OpenOptions::new()
        .write(true)
        .create(true)
        .truncate(true)
        .mode(0o600)
        .open(&path)?;
    // A file from an older run keeps its mode on open.
    file.set_permissions(std::fs::Permissions::from_mode(0o600))?;
    file.write_all(html.as_bytes())?;
    Ok(path)
}

/// The stored objects of the object views, by id.
#[derive(Default)]
pub(super) struct Objects {
    pub(super) minutes: HashMap<String, Load<MinutesDetail>>,
    pub(super) stories: HashMap<String, Load<StoryGetResult>>,
    pub(super) diffs: HashMap<String, Load<DiffShown>>,
    pub(super) tickets: HashMap<String, Load<Ticket>>,
}

/// The diff of a story as the view draws it: at most `DIFF_LINES` lines.
pub(super) struct DiffShown {
    pub(super) story: StoryReviewResult,
    /// The parsed patch. `None` when the patch does not parse: then the view draws the lines.
    pub(super) state: Option<Entity<DiffState>>,
    pub(super) lines: Vec<String>,
    /// The lines after `DIFF_LINES`.
    pub(super) more: usize,
}

/// The views that a read covers.
#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) enum Reads {
    All,
    /// The priorities views: `data.changed` of kind `minutes`.
    Minutes,
    /// The story and diff views: `data.changed` of kind `stories`.
    Stories,
    /// The tickets views: `data.changed` of kind `tickets`.
    Tickets,
}

impl Reads {
    fn covers(self, view: &View) -> bool {
        matches!(
            (self, view),
            (Reads::All, _)
                | (Reads::Minutes, View::Priorities { .. })
                | (Reads::Stories, View::Story { .. } | View::Diff { .. })
                | (Reads::Tickets, View::Tickets { .. })
        )
    }
}

impl DeskConversation {
    // ---- Reading ----------------------------------------------------------------------

    /// Reads the objects of each view that `which` covers, one read for each object.
    pub(super) fn read_objects(&mut self, which: Reads, cx: &mut Context<Self>) {
        let mut seen = Vec::new();
        let views: Vec<View> = self
            .rows
            .iter()
            .filter_map(|r| match r {
                Row::View(v, _) if which.covers(v) && !seen.contains(v) => {
                    seen.push(v.clone());
                    Some(v.clone())
                }
                _ => None,
            })
            .collect();
        for v in views {
            self.read_view(&v, cx);
        }
    }

    /// Reads the object of one view. A view that holds its data reads nothing.
    pub(super) fn read_view(&mut self, view: &View, cx: &mut Context<Self>) {
        let link = self.core.read(cx).link.clone();
        match view {
            View::Decisions { .. }
            | View::Table { .. }
            | View::Chart { .. }
            | View::Checklist { .. }
            | View::Page { .. } => {}
            View::Tickets { ids } => {
                for id in ids {
                    let id = id.clone();
                    self.objects
                        .tickets
                        .entry(id.clone())
                        .or_insert(Load::Loading);
                    let call = link.call::<TicketGet>(&TicketGetParams {
                        product: self.product.clone(),
                        id: id.clone(),
                    });
                    cx.spawn(async move |this, cx| {
                        let result = call.await;
                        let _ = this.update(cx, |this, cx| this.ticket_done(id, result, cx));
                    })
                    .detach();
                }
            }
            View::Priorities { minutes_id } => {
                let id = minutes_id.clone();
                self.objects
                    .minutes
                    .entry(id.clone())
                    .or_insert(Load::Loading);
                let call = link.call::<MinutesGet>(&MinutesGetParams { id: id.clone() });
                cx.spawn(async move |this, cx| {
                    let result = call.await;
                    let _ = this.update(cx, |this, cx| this.minutes_done(id, result, cx));
                })
                .detach();
            }
            View::Story { id } => {
                let id = id.clone();
                self.objects
                    .stories
                    .entry(id.clone())
                    .or_insert(Load::Loading);
                let call = link.call::<StoryGet>(&StoryIdParams {
                    product: self.product.clone(),
                    id: id.clone(),
                });
                cx.spawn(async move |this, cx| {
                    let result = call.await;
                    let _ = this.update(cx, |this, cx| this.story_done(id, result, cx));
                })
                .detach();
            }
            View::Diff { story_id } => {
                let id = story_id.clone();
                self.objects
                    .diffs
                    .entry(id.clone())
                    .or_insert(Load::Loading);
                let call = link.call::<StoryReview>(&StoryIdParams {
                    product: self.product.clone(),
                    id: id.clone(),
                });
                cx.spawn(async move |this, cx| {
                    let result = call.await;
                    let _ = this.update(cx, |this, cx| this.review_done(id, result, cx));
                })
                .detach();
            }
        }
    }

    pub(crate) fn minutes_done(
        &mut self,
        id: String,
        result: Result<MinutesDetail, CallError>,
        cx: &mut Context<Self>,
    ) {
        self.objects
            .minutes
            .entry(id)
            .or_insert(Load::Loading)
            .settle(result.map_err(|e| e.to_string()));
        self.views_changed(cx);
    }

    pub(crate) fn ticket_done(
        &mut self,
        id: String,
        result: Result<TicketGetResult, CallError>,
        cx: &mut Context<Self>,
    ) {
        self.objects
            .tickets
            .entry(id)
            .or_insert(Load::Loading)
            .settle(result.map(|r| r.ticket).map_err(|e| e.to_string()));
        self.views_changed(cx);
    }

    pub(crate) fn story_done(
        &mut self,
        id: String,
        result: Result<StoryGetResult, CallError>,
        cx: &mut Context<Self>,
    ) {
        self.objects
            .stories
            .entry(id)
            .or_insert(Load::Loading)
            .settle(result.map_err(|e| e.to_string()));
        self.views_changed(cx);
    }

    pub(crate) fn review_done(
        &mut self,
        id: String,
        result: Result<StoryReviewResult, CallError>,
        cx: &mut Context<Self>,
    ) {
        let old = self.objects.diffs.remove(&id);
        let state = match &old {
            Some(Load::Ready(d)) => d.state.clone(),
            _ => None,
        };
        let shown = result.map_err(|e| e.to_string()).map(|story| {
            let all: Vec<&str> = story.diff.lines().collect();
            let more = all.len().saturating_sub(DIFF_LINES);
            let lines: Vec<String> = all[..all.len().min(DIFF_LINES)]
                .iter()
                .map(|l| l.to_string())
                .collect();
            let mut patch = lines.join("\n");
            patch.push('\n');
            let options = DiffParseOptions::new().truncated_hunks(more > 0);
            let state = match DiffFile::parse_with(&patch, options) {
                Ok(files) if !files.is_empty() => Some(match state {
                    Some(s) => {
                        s.update(cx, |s, cx| s.set_files(files, cx));
                        s
                    }
                    None => cx.new(|cx| DiffState::new(files, cx)),
                }),
                _ => None,
            };
            DiffShown {
                story,
                state,
                lines,
                more,
            }
        });
        let mut entry = old.unwrap_or(Load::Loading);
        entry.settle(shown);
        self.objects.diffs.insert(id, entry);
        self.views_changed(cx);
    }

    /// The rows of the views changed height.
    fn views_changed(&mut self, cx: &mut Context<Self>) {
        let n = self.rows.len();
        self.scroller.update(cx, |s, cx| {
            s.remeasure_items(0..n, cx);
        });
        cx.notify();
    }

    // ---- Writing ----------------------------------------------------------------------

    /// Asks the Confirm of a mark on a priority of a priorities view.
    pub fn ask_mark(&mut self, minutes_id: &str, idx: u32, mark: Mark, cx: &mut Context<Self>) {
        self.confirm = Some(Confirm::Mark(minutes_id.to_string(), idx, mark));
        cx.notify();
    }

    /// Sets the mark with `unmarked_only`: a priority that the owner marked elsewhere since
    /// the view read it is refused, not changed.
    pub(super) fn set_mark(
        &mut self,
        minutes_id: String,
        idx: u32,
        mark: Mark,
        cx: &mut Context<Self>,
    ) {
        if self.busy {
            return;
        }
        self.busy = true;
        self.flash = None;
        let call = self.core.read(cx).link.call::<MarkSet>(&MarkSetParams {
            entries: vec![MarkEntry {
                minutes_id: minutes_id.clone(),
                idx,
                mark,
                is_new: false,
            }],
            unmarked_only: true,
        });
        let core = self.core.clone();
        cx.spawn(async move |this, cx| {
            let result = call.await;
            let _ = this.update(cx, |this, cx| {
                this.mark_done(minutes_id, idx, mark, result, cx)
            });
            cx.update(|cx| Live::reload_core(&core, cx));
        })
        .detach();
    }

    pub(crate) fn mark_done(
        &mut self,
        minutes_id: String,
        idx: u32,
        mark: Mark,
        result: Result<MarkSetResult, CallError>,
        cx: &mut Context<Self>,
    ) {
        self.busy = false;
        self.flash = Some(match result {
            Ok(_) => Flash::Done(format!(
                "Marked priority {} of {minutes_id} as {}.",
                idx + 1,
                mark.as_str()
            )),
            Err(e) => Flash::Failed(e.to_string()),
        });
        // The view shows the stored mark. The owner needs no new message for it.
        self.read_view(&View::Priorities { minutes_id }, cx);
        cx.notify();
    }

    /// Checks or unchecks one item of a checklist. The row shows it at once. A refusal of the
    /// core puts the row back.
    pub fn toggle(&mut self, seq: i64, item: u32, cx: &mut Context<Self>) {
        let Some(shown) = self.rows.iter_mut().find_map(|r| match r {
            Row::View(View::Checklist { .. }, shown) if shown.seq == seq => Some(shown),
            _ => None,
        }) else {
            return;
        };
        let before = shown.checked.clone();
        match shown.checked.iter().position(|c| *c == item) {
            Some(i) => {
                shown.checked.remove(i);
            }
            None => {
                shown.checked.push(item);
                shown.checked.sort_unstable();
            }
        }
        let call = self.core.read(cx).link.call::<DeskCheck>(&DeskCheckParams {
            product: self.product.clone(),
            seq,
            checked: shown.checked.clone(),
        });
        cx.spawn(async move |this, cx| {
            let result = call.await;
            let _ = this.update(cx, |this, cx| this.check_done(seq, before, result, cx));
        })
        .detach();
        cx.notify();
    }

    pub(crate) fn check_done(
        &mut self,
        seq: i64,
        before: Vec<u32>,
        result: Result<DeskSeqResult, CallError>,
        cx: &mut Context<Self>,
    ) {
        if let Err(e) = result {
            self.flash = Some(Flash::Failed(e.to_string()));
            for r in &mut self.rows {
                if let Row::View(_, shown) = r
                    && shown.seq == seq
                {
                    shown.checked.clone_from(&before);
                }
            }
        }
        cx.notify();
    }

    /// "Open in browser": reads the HTML of the page from the core, then writes and opens it.
    pub fn open_page(&mut self, seq: i64, cx: &mut Context<Self>) {
        let call = self.core.read(cx).link.call::<DeskPage>(&DeskPageParams {
            product: self.product.clone(),
            seq,
        });
        cx.spawn(async move |this, cx| {
            let result = call.await;
            let _ = this.update(cx, |this, cx| this.page_done(seq, result, cx));
        })
        .detach();
    }

    pub(crate) fn page_done(
        &mut self,
        seq: i64,
        result: Result<DeskPageResult, CallError>,
        cx: &mut Context<Self>,
    ) {
        let written = result.map_err(|e| e.to_string()).and_then(|page| {
            let dir = self.pages.clone().ok_or_else(|| {
                "XDG_RUNTIME_DIR is not set, so the app has no folder for the page.".to_string()
            })?;
            write_page(&dir, &self.product, seq, &page.html)
                .map_err(|e| format!("The page file was not written: {e}"))
        });
        match written {
            Ok(path) => cx.open_url(&crate::format::file_url(&path)),
            Err(e) => self.flash = Some(Flash::Failed(e)),
        }
        cx.notify();
    }

    /// Opens the Ticket screen. A view never changes a ticket itself.
    pub fn open_ticket(&mut self, id: &str, cx: &mut Context<Self>) {
        Nav::open(cx, &self.core, ScreenRequest::ticket(&self.product, id));
    }

    /// Opens the story screen. A view never runs a story itself.
    pub fn open_story(&mut self, id: &str, cx: &mut Context<Self>) {
        Nav::open(cx, &self.core, ScreenRequest::story(&self.product, id));
    }

    // ---- Drawing ----------------------------------------------------------------------

    pub(super) fn view_element(
        &self,
        row: usize,
        view: &View,
        shown: &Shown,
        weak: &WeakEntity<Self>,
        cx: &App,
    ) -> AnyElement {
        match view {
            // The conversation draws the decisions view itself.
            View::Decisions { .. } => div().into_any_element(),
            View::Priorities { minutes_id } => self.priorities_view(row, minutes_id, weak),
            View::Story { id } => self.story_view(row, id, weak),
            View::Diff { story_id } => self.diff_view(row, story_id, weak),
            View::Table {
                title,
                columns,
                rows,
            } => table_view(row, title, columns, rows),
            View::Chart {
                chart,
                title,
                labels,
                values,
            } => chart_view(row, *chart, title, labels, values, cx),
            View::Checklist { title, items } => checklist_view(row, title, items, shown, weak),
            View::Page { title, .. } => page_view(row, title, shown, weak),
            View::Tickets { ids } => self.tickets_view(row, ids, weak),
        }
    }

    fn tickets_view(&self, row: usize, ids: &[String], weak: &WeakEntity<Self>) -> AnyElement {
        let now = crate::format::now_secs();
        div()
            .flex()
            .flex_col()
            .gap(space(1))
            .child(view_title(
                "Tickets",
                &format!(
                    "{} ticket{}",
                    ids.len(),
                    if ids.len() == 1 { "" } else { "s" }
                ),
            ))
            .children(ids.iter().enumerate().map(|(n, id)| {
                let t = match self.objects.tickets.get(id) {
                    Some(Load::Ready(t)) => t,
                    other => return pending_line(other, &format!("the ticket {id}")),
                };
                let (weak, id) = (weak.clone(), id.clone());
                ListRow::new(
                    SharedString::from(format!("deskv-{row}-ticket-{n}")),
                    t.title.clone(),
                )
                .meta(crate::screens::tickets::meta(t, now))
                .trailing(
                    div()
                        .flex()
                        .gap(space(2))
                        .child(crate::screens::ticket::kind_tag(t.kind))
                        .child(crate::screens::ticket::state_tag(t)),
                )
                .on_click(move |_, _, cx| {
                    let _ = weak.update(cx, |this, cx| this.open_ticket(&id, cx));
                })
                .into_any_element()
            }))
            .into_any_element()
    }

    fn priorities_view(&self, row: usize, minutes_id: &str, weak: &WeakEntity<Self>) -> AnyElement {
        let m = match self.objects.minutes.get(minutes_id) {
            Some(Load::Ready(m)) => m,
            other => return pending_line(other, &format!("the minutes {minutes_id}")),
        };
        div()
            .flex()
            .flex_col()
            .gap(space(2))
            .child(view_title(
                "Priorities",
                &format!("{} · {}", m.id, m.question),
            ))
            .when(m.priorities.is_empty(), |el| {
                el.child(muted("These minutes have no priorities."))
            })
            .children(
                m.priorities
                    .iter()
                    .map(|p| self.priority_card(row, &m.id, p, weak)),
            )
            .into_any_element()
    }

    fn priority_card(
        &self,
        row: usize,
        minutes_id: &str,
        p: &PriorityInfo,
        weak: &WeakEntity<Self>,
    ) -> AnyElement {
        let key = format!("deskv-{row}-p{}", p.idx);
        let mut card = Card::new(SharedString::from(key.clone()))
            .child(
                div()
                    .flex()
                    .flex_wrap()
                    .items_center()
                    .gap(space(2))
                    .child(neutral_tag("Priority"))
                    .child(mono(format!(
                        "{} · {}",
                        p.idx + 1,
                        role_label(p.owner_role.as_deref().unwrap_or("pm"))
                    )))
                    .child(mark_tag(p.mark.as_deref())),
            )
            .child(
                div()
                    .text_size(size::md())
                    .font_weight(weight::SEMIBOLD)
                    .text_color(text::primary())
                    .child(p.title.clone()),
            )
            .child(
                div()
                    .text_size(size::sm())
                    .text_color(text::secondary())
                    .child(p.why.clone()),
            );
        if p.mark.is_some() {
            return card.into_any_element();
        }
        let busy = self.busy;
        let asked = match &self.confirm {
            Some(Confirm::Mark(m, idx, mark)) if m == minutes_id && *idx == p.idx => Some(*mark),
            _ => None,
        };
        if let Some(mark) = asked {
            card = card
                .child(muted(&format!(
                    "Mark priority {} as {}?",
                    p.idx + 1,
                    mark_label(mark)
                )))
                .child(confirm_buttons(&key, busy, weak));
            return card.into_any_element();
        }
        let ask = |mark: Mark| {
            let (weak, id, idx) = (weak.clone(), minutes_id.to_string(), p.idx);
            move |_: &ClickEvent, _: &mut Window, cx: &mut App| {
                let _ = weak.update(cx, |this, cx| this.ask_mark(&id, idx, mark, cx));
            }
        };
        card.child(
            div()
                .flex()
                .flex_wrap()
                .gap(space(2))
                .child(
                    Button::new(SharedString::from(format!("{key}-accept")))
                        .primary()
                        .small()
                        .label("Accept")
                        .disabled(busy)
                        .on_click(ask(Mark::Accepted)),
                )
                .child(
                    Button::new(SharedString::from(format!("{key}-change")))
                        .secondary()
                        .small()
                        .label("Accept with changes")
                        .disabled(busy)
                        .on_click(ask(Mark::Changed)),
                )
                .child(
                    Button::new(SharedString::from(format!("{key}-reject")))
                        .secondary()
                        .small()
                        .label("Reject")
                        .disabled(busy)
                        .on_click(ask(Mark::Rejected)),
                ),
        )
        .into_any_element()
    }

    fn story_view(&self, row: usize, id: &str, weak: &WeakEntity<Self>) -> AnyElement {
        let got = match self.objects.stories.get(id) {
            Some(Load::Ready(g)) => g,
            other => return pending_line(other, &format!("the story {id}")),
        };
        let s = &got.story;
        Card::new(SharedString::from(format!("deskv-{row}-story")))
            .child(
                div()
                    .flex()
                    .flex_wrap()
                    .items_center()
                    .gap(space(2))
                    .child(neutral_tag(if s.kind == "task" { "Task" } else { "Story" }))
                    .child(mono(s.id.clone()))
                    .child(story_tag(s))
                    .when(got.archived, |el| el.child(neutral_tag("archived"))),
            )
            .child(
                div()
                    .text_size(size::md())
                    .font_weight(weight::SEMIBOLD)
                    .text_color(text::primary())
                    .child(s.title.clone()),
            )
            .when(!s.i_want.is_empty(), |el| {
                el.child(muted(&format!("I want {}", s.i_want)))
            })
            .child(
                div()
                    .text_size(size::sm())
                    .font_weight(weight::SEMIBOLD)
                    .text_color(text::secondary())
                    .child(format!("Criteria ({})", s.criteria.len())),
            )
            .children(s.criteria.iter().map(|c| {
                div()
                    .text_size(size::sm())
                    .text_color(text::secondary())
                    .child(format!(
                        "Given {}, when {}, then {}.",
                        c.given, c.when, c.then
                    ))
            }))
            .child(open_button(row, &s.id, weak))
            .into_any_element()
    }

    fn diff_view(&self, row: usize, story_id: &str, weak: &WeakEntity<Self>) -> AnyElement {
        let d = match self.objects.diffs.get(story_id) {
            Some(Load::Ready(d)) => d,
            other => return pending_line(other, &format!("the diff of {story_id}")),
        };
        let body = if d.lines.is_empty() {
            muted("The branch of this story has no diff.").into_any_element()
        } else {
            match &d.state {
                Some(state) => div()
                    .h(control::DESK_VIEW_HEIGHT)
                    .rounded(radius::md())
                    .border_1()
                    .border_color(line::border())
                    .overflow_hidden()
                    .child(Diff::new(state).size_full())
                    .into_any_element(),
                None => div()
                    .flex()
                    .flex_col()
                    .p(space(2))
                    .rounded(radius::md())
                    .bg(surface::raised())
                    .children(
                        d.lines
                            .iter()
                            .take(DIFF_TEXT_LINES)
                            .map(|l| mono(l.clone())),
                    )
                    .into_any_element(),
            }
        };
        div()
            .flex()
            .flex_col()
            .gap(space(2))
            .child(view_title(
                "Diff",
                &format!("{} · {}", d.story.story.id, d.story.story.title),
            ))
            .children(
                d.story
                    .stat
                    .lines()
                    .last()
                    .filter(|l| !l.trim().is_empty())
                    .map(|l| mono(l.trim().to_string())),
            )
            .child(body)
            .when(d.more > 0, |el| {
                el.child(muted(&format!(
                    "{} more lines. Open the story for the whole diff.",
                    d.more
                )))
            })
            .child(open_button(row, &d.story.story.id, weak))
            .into_any_element()
    }
}

fn table_view(row: usize, title: &str, columns: &[String], rows: &[Vec<String>]) -> AnyElement {
    let table = Table::new()
        .child(TableHeader::new().child(
            TableRow::new().children(columns.iter().map(|c| TableHead::new().child(c.clone()))),
        ))
        .child(TableBody::new().children(rows.iter().map(|r| {
            TableRow::new().children(r.iter().map(|cell| TableCell::new().child(cell.clone())))
        })));
    div()
        .flex()
        .flex_col()
        .gap(space(2))
        .child(view_title("Table", title))
        .child(
            div()
                .id(SharedString::from(format!("deskv-{row}-table")))
                .max_h(control::DESK_VIEW_HEIGHT)
                .overflow_scroll()
                .rounded(radius::md())
                .border_1()
                .border_color(line::border())
                .child(table),
        )
        .when(rows.is_empty(), |el| {
            el.child(muted("The table has no rows."))
        })
        .into_any_element()
}

/// One point of a chart: its label, its value and, for a pie, its color.
#[derive(Clone)]
struct Point {
    label: String,
    value: f64,
    color: Hsla,
}

fn chart_view(
    row: usize,
    kind: ChartKind,
    title: &str,
    labels: &[String],
    values: &[f64],
    cx: &App,
) -> AnyElement {
    let theme = cx.theme();
    let palette = [
        theme.chart_1,
        theme.chart_2,
        theme.chart_3,
        theme.chart_4,
        theme.chart_5,
    ];
    let points: Vec<Point> = labels
        .iter()
        .zip(values)
        .enumerate()
        .map(|(n, (label, value))| Point {
            label: label.clone(),
            value: *value,
            color: palette[n % palette.len()],
        })
        .collect();
    let id = SharedString::from(format!("deskv-{row}-chart"));
    let chart = match kind {
        ChartKind::Bar => BarChart::new(points)
            .id(id)
            .band(|p: &Point| p.label.clone())
            .value(|p: &Point| p.value)
            .into_any_element(),
        ChartKind::Line => LineChart::new(points)
            .id(id)
            .x(|p: &Point| p.label.clone())
            .y(|p: &Point| p.value)
            .dot()
            .into_any_element(),
        ChartKind::Area => AreaChart::new(points)
            .id(id)
            .x(|p: &Point| p.label.clone())
            .y(|p: &Point| p.value)
            .into_any_element(),
        ChartKind::Pie => PieChart::new(points)
            .id(id)
            .value(|p: &Point| p.value as f32)
            .color(|p: &Point| p.color)
            .label(|p: &Point| SharedString::from(p.label.clone()))
            .into_any_element(),
    };
    div()
        .flex()
        .flex_col()
        .gap(space(2))
        .child(view_title("Chart", title))
        .child(div().h(control::DESK_CHART_HEIGHT).w_full().child(chart))
        .into_any_element()
}

fn checklist_view(
    row: usize,
    title: &str,
    items: &[String],
    shown: &Shown,
    weak: &WeakEntity<DeskConversation>,
) -> AnyElement {
    let seq = shown.seq;
    div()
        .flex()
        .flex_col()
        .gap(space(2))
        .child(view_title(
            "Checklist",
            &format!("{title} · {} of {}", shown.checked.len(), items.len()),
        ))
        .children(items.iter().enumerate().map(|(n, item)| {
            let weak = weak.clone();
            let idx = n as u32;
            Checkbox::new(SharedString::from(format!("deskv-{row}-check-{n}")))
                .label(item.clone())
                .checked(shown.checked.contains(&idx))
                .on_click(move |_, _, cx| {
                    let _ = weak.update(cx, |this, cx| this.toggle(seq, idx, cx));
                })
        }))
        .into_any_element()
}

fn page_view(
    row: usize,
    title: &str,
    shown: &Shown,
    weak: &WeakEntity<DeskConversation>,
) -> AnyElement {
    let (weak, seq) = (weak.clone(), shown.seq);
    Card::new(SharedString::from(format!("deskv-{row}-page")))
        .child(view_title("Page", title))
        .children(shown.html_len.map(|n| {
            muted(&format!(
                "{n} characters of HTML. The app does not draw it."
            ))
        }))
        .child(
            div().flex().child(
                Button::new(SharedString::from(format!("deskv-{row}-open-page")))
                    .secondary()
                    .small()
                    .label("Open in browser")
                    .on_click(move |_, _, cx| {
                        let _ = weak.update(cx, |this, cx| this.open_page(seq, cx));
                    }),
            ),
        )
        .into_any_element()
}

/// The lines that a diff with no parsed patch draws as text.
const DIFF_TEXT_LINES: usize = 40;

fn mark_label(mark: Mark) -> &'static str {
    match mark {
        Mark::Accepted => "accepted",
        Mark::Changed => "accepted with changes",
        Mark::Rejected => "rejected",
    }
}

fn muted(s: &str) -> Div {
    div()
        .text_size(size::sm())
        .text_color(text::secondary())
        .child(s.to_string())
}

/// The line of a view whose object has not come, or did not load.
fn pending_line<T>(load: Option<&Load<T>>, what: &str) -> AnyElement {
    match load {
        Some(Load::Failed(e)) => div()
            .text_size(size::sm())
            .text_color(WorkState::Failed.color())
            .child(format!("Cannot read {what}: {e}"))
            .into_any_element(),
        _ => div()
            .text_size(size::sm())
            .text_color(text::tertiary())
            .child(format!("Reading {what}…"))
            .into_any_element(),
    }
}

fn view_title(kind: &str, title: &str) -> Div {
    div()
        .flex()
        .items_center()
        .gap(space(2))
        .min_w_0()
        .child(neutral_tag(kind.to_string()))
        .child(
            div()
                .min_w_0()
                .truncate()
                .text_size(size::sm())
                .font_weight(weight::SEMIBOLD)
                .text_color(text::primary())
                .child(title.to_string()),
        )
}

fn open_button(row: usize, id: &str, weak: &WeakEntity<DeskConversation>) -> impl IntoElement {
    let (weak, id) = (weak.clone(), id.to_string());
    div().flex().child(
        Button::new(SharedString::from(format!("deskv-{row}-open")))
            .secondary()
            .small()
            .label("Open the story")
            .on_click(move |_, _, cx| {
                let _ = weak.update(cx, |this, cx| this.open_story(&id, cx));
            }),
    )
}

fn confirm_buttons(key: &str, busy: bool, weak: &WeakEntity<DeskConversation>) -> Div {
    let call = |f: fn(&mut DeskConversation, &mut Context<DeskConversation>)| {
        let weak = weak.clone();
        move |_: &ClickEvent, _: &mut Window, cx: &mut App| {
            let _ = weak.update(cx, f);
        }
    };
    div()
        .flex()
        .gap(space(2))
        .child(
            Button::new(SharedString::from(format!("{key}-confirm")))
                .primary()
                .small()
                .label("Confirm")
                .disabled(busy)
                .on_click(call(|t, cx| t.confirmed(cx))),
        )
        .child(
            Button::new(SharedString::from(format!("{key}-cancel")))
                .ghost()
                .small()
                .label("Cancel")
                .on_click(call(|t, cx| t.cancel(cx))),
        )
}

#[cfg(test)]
mod tests {
    use agentsi_proto::desk_api::MinutesDetail;
    use agentsi_proto::events::DataChanged;
    use agentsi_proto::marks_api::{Mark, MarkSetResult};
    use agentsi_proto::rpc::Notification;
    use agentsi_proto::story_api::{StoryGetResult, StoryReviewResult};
    use gpui::{TestAppContext, VisualTestContext};
    use gpui_kit::Entity;
    use serde_json::{Value, json};

    // No `use super::*`: see the tests of the conversation.
    use super::super::tests::{draw, entry, history, methods_sent, setup};
    use super::{DeskConversation, Load};
    use crate::link::CallError;
    use crate::nav::{Nav, ScreenRequest};
    use crate::screens::parts::Flash;
    use agentsi_proto::desk_session_api::{DeskPageResult, DeskSeqResult};

    pub(in crate::screens) fn minutes(mark: Option<&str>) -> MinutesDetail {
        serde_json::from_value(json!({
            "id": "m1", "product": "demo", "scope": null, "question": "What now?",
            "status": "valid", "created_at": null, "archived": false, "heard": false,
            "check": null,
            "priorities": [
                {"idx": 0, "title": "Ship the beta", "why": "Users wait.", "owner_role": "pm",
                 "objective": null, "mark": mark, "is_new": false, "sources": []},
                {"idx": 1, "title": "Fix the login", "why": "It fails.", "owner_role": "dev",
                 "objective": null, "mark": "rejected", "is_new": false, "sources": []}
            ],
            "disagreements": [], "decisions": [], "roles": [], "usage": [],
            "usage_totals": {"rows": 0, "input_tokens": 0, "output_tokens": 0,
                             "cache_read": 0, "cache_write": 0, "cost_usd": 0.0}
        }))
        .unwrap()
    }

    fn story_json() -> Value {
        json!({
            "id": "S001", "type": "story", "slug": "login", "title": "Add a login",
            "i_want": "to sign in", "so_that": "my data is mine",
            "criteria": [{"given": "a user", "when": "they sign in", "then": "they see the home",
                          "test": "tests/login.rs", "checks": "the home shows"}],
            "owner_role": "dev", "status": "not_implemented", "approval": "approved",
            "proposed_by": "pm", "created_at": "2026-11-01", "updated_at": "2026-11-02"
        })
    }

    pub(in crate::screens) fn story() -> StoryGetResult {
        serde_json::from_value(json!({
            "story": story_json(), "markdown": "# S001", "actions": ["run"], "log": "",
            "archived": false
        }))
        .unwrap()
    }

    pub(in crate::screens) fn review(diff: &str) -> StoryReviewResult {
        serde_json::from_value(json!({
            "story": story_json(), "stat": " src/a.rs | 2 +-\n 1 file changed",
            "diff": diff, "repo": "/src/demo"
        }))
        .unwrap()
    }

    pub(in crate::screens) const PATCH: &str = "diff --git a/src/a.rs b/src/a.rs\n\
        index 1111111..2222222 100644\n--- a/src/a.rs\n+++ b/src/a.rs\n\
        @@ -1,2 +1,2 @@\n fn main() {\n-    old();\n+    new();\n";

    fn views_history(cx: &mut VisualTestContext, conv: &Entity<DeskConversation>) {
        conv.update(cx, |c, cx| {
            c.history_done(
                Ok(history(
                    "idle",
                    None,
                    json!([
                        entry(1, json!({"kind": "owner", "text": "Show me"})),
                        entry(2, json!({"kind": "view", "view": {"kind": "priorities", "minutes_id": "m1"}})),
                        entry(3, json!({"kind": "view", "view": {"kind": "story", "id": "S001"}})),
                        entry(4, json!({"kind": "view", "view": {"kind": "diff", "story_id": "S001"}})),
                    ]),
                )),
                cx,
            )
        });
    }

    fn changed(kind: &str) -> Notification {
        Notification::of::<DataChanged>(
            &serde_json::from_value(json!({"product": "demo", "kind": kind})).unwrap(),
        )
    }

    #[gpui::test]
    fn the_priorities_story_and_diff_views_read_their_objects_and_draw_them(
        cx: &mut TestAppContext,
    ) {
        let (conv, mut other, cx) = setup(cx);
        methods_sent(&mut other);
        views_history(cx, &conv);
        let sent = methods_sent(&mut other);
        assert_eq!(
            sent,
            [
                ("minutes.get".to_string(), json!({"id": "m1"})),
                (
                    "story.get".to_string(),
                    json!({"product": "demo", "id": "S001"})
                ),
                (
                    "story.review".to_string(),
                    json!({"product": "demo", "id": "S001"})
                ),
            ]
        );
        // Before the objects come, each view draws a reading line.
        draw(cx);
        conv.update(cx, |c, cx| {
            c.minutes_done("m1".into(), Ok(minutes(None)), cx);
            c.story_done("S001".into(), Ok(story()), cx);
            c.review_done("S001".into(), Ok(review(PATCH)), cx);
        });
        draw(cx);
        draw_rows(cx, &conv);
        conv.read_with(cx, |c, _| {
            assert!(matches!(c.objects.minutes["m1"], Load::Ready(_)));
            assert!(matches!(c.objects.stories["S001"], Load::Ready(_)));
            let Load::Ready(d) = &c.objects.diffs["S001"] else {
                panic!("the diff is not ready")
            };
            assert!(d.state.is_some(), "the patch parses");
            assert_eq!(d.more, 0);
        });

        // A failed read is a line. It does not stop the other views.
        conv.update(cx, |c, cx| {
            c.story_done("S001".into(), Err(CallError::Link("gone".into())), cx)
        });
        draw(cx);
    }

    #[gpui::test]
    fn a_long_diff_shows_the_line_limit_of_the_story_screen(cx: &mut TestAppContext) {
        let (conv, mut other, cx) = setup(cx);
        views_history(cx, &conv);
        methods_sent(&mut other);
        let mut long = String::from(PATCH);
        for n in 0..500 {
            long.push_str(&format!("+line {n}\n"));
        }
        conv.update(cx, |c, cx| {
            c.review_done("S001".into(), Ok(review(&long)), cx)
        });
        draw(cx);
        conv.read_with(cx, |c, _| {
            let Load::Ready(d) = &c.objects.diffs["S001"] else {
                panic!("the diff is not ready")
            };
            assert_eq!(d.lines.len(), crate::screens::story::DIFF_LINES);
            assert_eq!(
                d.more,
                long.lines().count() - crate::screens::story::DIFF_LINES
            );
        });
        // An empty branch is a line, not an error.
        conv.update(cx, |c, cx| c.review_done("S001".into(), Ok(review("")), cx));
        draw(cx);
    }

    #[gpui::test]
    fn accept_on_a_priority_asks_a_confirm_then_sets_the_mark_and_reads_it_again(
        cx: &mut TestAppContext,
    ) {
        let (conv, mut other, cx) = setup(cx);
        views_history(cx, &conv);
        conv.update(cx, |c, cx| {
            c.minutes_done("m1".into(), Ok(minutes(None)), cx)
        });
        methods_sent(&mut other);

        // Accept only asks. It writes nothing.
        conv.update(cx, |c, cx| c.ask_mark("m1", 0, Mark::Accepted, cx));
        draw(cx);
        assert!(methods_sent(&mut other).is_empty());
        conv.update(cx, |c, cx| c.cancel(cx));
        conv.update(cx, |c, cx| c.confirmed(cx));
        assert!(methods_sent(&mut other).is_empty());

        conv.update(cx, |c, cx| c.ask_mark("m1", 0, Mark::Accepted, cx));
        conv.update(cx, |c, cx| c.confirmed(cx));
        let sent = methods_sent(&mut other);
        assert_eq!(
            sent,
            [(
                "mark.set".to_string(),
                json!({"entries": [{"minutes_id": "m1", "idx": 0, "mark": "accepted",
                                    "is_new": false}], "unmarked_only": true})
            )]
        );

        // The core accepted. The view reads the minutes again. No owner message is sent.
        conv.update(cx, |c, cx| {
            c.mark_done(
                "m1".into(),
                0,
                Mark::Accepted,
                Ok(MarkSetResult { marked: 1 }),
                cx,
            )
        });
        let sent = methods_sent(&mut other);
        assert_eq!(
            sent.iter().map(|s| s.0.as_str()).collect::<Vec<_>>(),
            ["minutes.get"]
        );
        conv.update(cx, |c, cx| {
            c.minutes_done("m1".into(), Ok(minutes(Some("accepted"))), cx)
        });
        draw(cx);
        conv.read_with(cx, |c, _| {
            let Load::Ready(m) = &c.objects.minutes["m1"] else {
                panic!("the minutes are not ready")
            };
            assert_eq!(m.priorities[0].mark.as_deref(), Some("accepted"));
            assert!(matches!(c.flash, Some(Flash::Done(_))));
            assert!(!c.busy);
        });
        assert!(methods_sent(&mut other).is_empty());

        // A refusal of the core shows as a line.
        conv.update(cx, |c, cx| {
            c.mark_done(
                "m1".into(),
                0,
                Mark::Rejected,
                Err(CallError::Link("conflict".into())),
                cx,
            )
        });
        conv.read_with(cx, |c, _| {
            assert!(matches!(c.flash, Some(Flash::Failed(_))))
        });
    }

    #[gpui::test]
    fn each_object_view_reads_again_after_data_changed_of_its_kind(cx: &mut TestAppContext) {
        let (conv, mut other, cx) = setup(cx);
        views_history(cx, &conv);
        methods_sent(&mut other);
        conv.update(cx, |c, cx| c.take(&changed("tickets"), cx));
        assert!(methods_sent(&mut other).is_empty());
        conv.update(cx, |c, cx| c.take(&changed("minutes"), cx));
        let names: Vec<String> = methods_sent(&mut other).into_iter().map(|m| m.0).collect();
        assert_eq!(names, ["minutes.get"]);
        conv.update(cx, |c, cx| c.take(&changed("stories"), cx));
        let names: Vec<String> = methods_sent(&mut other).into_iter().map(|m| m.0).collect();
        assert_eq!(names, ["story.get", "story.review"]);
    }

    #[gpui::test]
    fn open_the_story_asks_the_shell_for_the_story_screen(cx: &mut TestAppContext) {
        let (nav, mut requests) = Nav::channel();
        cx.update(|cx| cx.set_global(nav));
        let (conv, _other, cx) = setup(cx);
        conv.update(cx, |c, cx| c.open_story("S001", cx));
        assert_eq!(
            requests.try_recv().unwrap(),
            ScreenRequest::story("demo", "S001")
        );
    }

    /// All the rows of a conversation in one column, so that a test draws each row, also
    /// the rows that the scroller does not show.
    struct AllRows(Entity<DeskConversation>);

    impl gpui::Render for AllRows {
        fn render(
            &mut self,
            _: &mut gpui::Window,
            cx: &mut gpui::Context<Self>,
        ) -> impl gpui::IntoElement {
            use gpui::{ParentElement as _, Styled as _};
            let weak = self.0.downgrade();
            let conv = self.0.read(cx);
            gpui::div()
                .flex()
                .flex_col()
                .w(gpui::px(380.))
                .children((0..conv.rows.len()).map(|i| conv.row_element(i, &weak, cx)))
        }
    }

    fn draw_rows(cx: &mut VisualTestContext, conv: &Entity<DeskConversation>) {
        let conv = conv.clone();
        cx.draw(
            gpui::point(gpui::px(0.), gpui::px(0.)),
            gpui::size(gpui::px(380.), gpui::px(4000.)),
            |_, cx| {
                use gpui::{AppContext as _, IntoElement as _};
                gpui::AnyView::from(cx.new(|_| AllRows(conv))).into_any_element()
            },
        );
    }

    /// An empty folder of this test process.
    fn scratch(name: &str) -> std::path::PathBuf {
        let dir =
            std::env::temp_dir().join(format!("agentsi-desk-pages-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        dir
    }

    fn generic_history(cx: &mut VisualTestContext, conv: &Entity<DeskConversation>) {
        let chart = |kind: &str| {
            json!({"kind": "view", "view": {"kind": "chart", "chart": kind, "title": "Cost",
                   "labels": ["w1", "w2", "w3"], "values": [1.5, 3.0, 2.0]}})
        };
        conv.update(cx, |c, cx| {
            c.history_done(
                Ok(history(
                    "idle",
                    None,
                    json!([
                        entry(
                            1,
                            json!({"kind": "view", "view": {"kind": "table", "title": "Roles",
                            "columns": ["role", "cost"], "rows": [["pm", "1"], ["dev", "2"]]}})
                        ),
                        entry(2, chart("bar")),
                        entry(3, chart("line")),
                        entry(4, chart("area")),
                        entry(5, chart("pie")),
                        entry(
                            6,
                            json!({"kind": "view", "view": {"kind": "checklist",
                            "title": "Launch", "items": ["tag", "post", "mail"]},
                            "checked": [2]})
                        ),
                        entry(
                            7,
                            json!({"kind": "view", "view": {"kind": "page",
                            "title": "Report", "html": ""}, "html_len": 15})
                        ),
                    ]),
                )),
                cx,
            )
        });
    }

    fn checked(c: &DeskConversation, seq: i64) -> Vec<u32> {
        c.rows
            .iter()
            .find_map(|r| match r {
                super::Row::View(_, shown) if shown.seq == seq => Some(shown.checked.clone()),
                _ => None,
            })
            .unwrap()
    }

    #[gpui::test]
    fn a_table_each_chart_a_checklist_and_a_page_draw_from_their_data_with_no_read(
        cx: &mut TestAppContext,
    ) {
        let (conv, mut other, cx) = setup(cx);
        methods_sent(&mut other);
        generic_history(cx, &conv);
        draw(cx);
        draw_rows(cx, &conv);
        // A generic view holds its data. It reads nothing from the core.
        assert!(methods_sent(&mut other).is_empty());
        conv.read_with(cx, |c, _| {
            assert_eq!(c.kinds(), ["view"; 7]);
            assert_eq!(checked(c, 6), [2]);
        });
    }

    #[gpui::test]
    fn a_check_calls_desk_check_with_all_checked_items_and_a_refusal_puts_it_back(
        cx: &mut TestAppContext,
    ) {
        let (conv, mut other, cx) = setup(cx);
        generic_history(cx, &conv);
        methods_sent(&mut other);
        conv.update(cx, |c, cx| c.toggle(6, 0, cx));
        assert_eq!(
            methods_sent(&mut other),
            [(
                "desk.check".to_string(),
                json!({"product": "demo", "seq": 6, "checked": [0, 2]})
            )]
        );
        conv.read_with(cx, |c, _| assert_eq!(checked(c, 6), [0, 2]));
        conv.update(cx, |c, cx| {
            c.check_done(6, vec![2], Ok(DeskSeqResult { seq: 6 }), cx)
        });
        conv.read_with(cx, |c, _| assert_eq!(checked(c, 6), [0, 2]));

        conv.update(cx, |c, cx| c.toggle(6, 2, cx));
        assert_eq!(methods_sent(&mut other)[0].1["checked"], json!([0]));
        conv.update(cx, |c, cx| {
            c.check_done(6, vec![0, 2], Err(CallError::Link("gone".into())), cx)
        });
        draw(cx);
        conv.read_with(cx, |c, _| {
            assert_eq!(checked(c, 6), [0, 2]);
            assert!(matches!(c.flash, Some(Flash::Failed(_))));
        });
        // A seq that is not a checklist sends nothing.
        conv.update(cx, |c, cx| c.toggle(7, 0, cx));
        assert!(methods_sent(&mut other).is_empty());
    }

    #[gpui::test]
    fn open_in_browser_writes_the_page_with_mode_0600_and_opens_the_file(cx: &mut TestAppContext) {
        use std::os::unix::fs::PermissionsExt as _;
        let (conv, mut other, cx) = setup(cx);
        generic_history(cx, &conv);
        methods_sent(&mut other);
        let dir = scratch("open");
        let pages = dir.join("agentsi").join("pages");
        conv.update(cx, |c, _| c.pages = Some(pages.clone()));
        conv.update(cx, |c, cx| c.open_page(7, cx));
        assert_eq!(
            methods_sent(&mut other),
            [(
                "desk.page".to_string(),
                json!({"product": "demo", "seq": 7})
            )]
        );
        let html = "<h1>Report</h1>";
        conv.update(cx, |c, cx| {
            c.page_done(
                7,
                Ok(DeskPageResult {
                    title: "Report".into(),
                    html: html.into(),
                }),
                cx,
            )
        });
        let file = pages.join("demo-7.html");
        assert_eq!(std::fs::read_to_string(&file).unwrap(), html);
        let mode = std::fs::metadata(&file).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode, 0o600);
        let dir_mode = std::fs::metadata(&pages).unwrap().permissions().mode() & 0o777;
        assert_eq!(dir_mode, 0o700);
        assert_eq!(cx.opened_url(), Some(crate::format::file_url(&file)));

        // With no runtime folder, the page is not written and a line says why.
        conv.update(cx, |c, cx| {
            c.pages = None;
            c.page_done(
                7,
                Ok(DeskPageResult {
                    title: "Report".into(),
                    html: html.into(),
                }),
                cx,
            )
        });
        conv.read_with(cx, |c, _| {
            assert!(matches!(c.flash, Some(Flash::Failed(_))))
        });
    }

    #[test]
    fn an_old_page_file_gets_mode_0600_again() {
        use std::os::unix::fs::PermissionsExt as _;
        let dir = scratch("old");
        std::fs::create_dir_all(&dir).unwrap();
        let old = dir.join("a_b-3.html");
        std::fs::write(&old, "old").unwrap();
        std::fs::set_permissions(&old, std::fs::Permissions::from_mode(0o644)).unwrap();
        let path = super::write_page(&dir, "a/b", 3, "new").unwrap();
        assert_eq!(path, old);
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "new");
        let mode = std::fs::metadata(&path).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode, 0o600);
    }

    fn ticket(id: &str, state: &str) -> agentsi_proto::ticket_api::TicketGetResult {
        serde_json::from_value(json!({
            "ticket": {
                "product": "demo", "id": id, "scope": null, "kind": "todo", "state": state,
                "title": "Tag the release", "body": "", "waiting_on": null, "assignee": "owner",
                "author": {"by": "owner", "id": ""}, "updated_by": {"by": "owner", "id": ""},
                "links": [], "objective": null, "created_at": "2026-11-09T10:00:00.000Z",
                "updated_at": "2026-11-09T10:00:00.000Z", "closed_at": null
            },
            "markdown": ""
        }))
        .unwrap()
    }

    #[gpui::test]
    fn a_tickets_view_reads_each_ticket_and_reads_again_at_once_after_a_ticket_write(
        cx: &mut TestAppContext,
    ) {
        let (nav, mut requests) = Nav::channel();
        cx.update(|cx| cx.set_global(nav));
        let (conv, mut other, cx) = setup(cx);
        methods_sent(&mut other);
        conv.update(cx, |c, cx| {
            c.history_done(
                Ok(history(
                    "idle",
                    None,
                    json!([entry(
                        1,
                        json!({"kind": "view",
                        "view": {"kind": "tickets", "ids": ["C001", "C002"]}})
                    )]),
                )),
                cx,
            )
        });
        assert_eq!(
            methods_sent(&mut other),
            [
                (
                    "ticket.get".to_string(),
                    json!({"product": "demo", "id": "C001"})
                ),
                (
                    "ticket.get".to_string(),
                    json!({"product": "demo", "id": "C002"})
                ),
            ]
        );
        conv.update(cx, |c, cx| {
            c.ticket_done("C001".into(), Ok(ticket("C001", "open")), cx);
            c.ticket_done("C002".into(), Err(CallError::Link("gone".into())), cx);
        });
        draw(cx);
        draw_rows(cx, &conv);

        // A ticket write sends data.changed of kind tickets. The view reads at once, with no
        // timer, so it changes as soon as the core answers.
        conv.update(cx, |c, cx| c.take(&changed("stories"), cx));
        assert!(methods_sent(&mut other).is_empty());
        conv.update(cx, |c, cx| c.take(&changed("tickets"), cx));
        let names: Vec<String> = methods_sent(&mut other).into_iter().map(|m| m.0).collect();
        assert_eq!(names, ["ticket.get", "ticket.get"]);
        conv.update(cx, |c, cx| {
            c.ticket_done("C001".into(), Ok(ticket("C001", "done")), cx)
        });
        draw_rows(cx, &conv);
        conv.read_with(cx, |c, _| {
            let Load::Ready(t) = &c.objects.tickets["C001"] else {
                panic!("the ticket is not ready")
            };
            assert_eq!(t.state, agentsi_proto::ticket_api::TicketState::Done);
        });

        conv.update(cx, |c, cx| c.open_ticket("C001", cx));
        assert_eq!(
            requests.try_recv().unwrap(),
            ScreenRequest::ticket("demo", "C001")
        );
    }
}
