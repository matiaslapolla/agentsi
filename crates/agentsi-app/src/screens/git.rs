//! The git screen of a product: the branch, the changed files and their diff, stage and
//! unstage, and the commit. A History tab lists the last commits. A Config tab edits the git
//! config of the product (see `git_config`). Spec: `docs/specs/agentsi-desktop-git.md`.
//!
//! The screen reads `git.status` when it opens, when it gets the focus (`show`), after each
//! action, and when the core sends `data.changed` (the shell reloads each screen of the host).
//! It has no push, pull or fetch: the owner does them in a pane.

use agentsi_proto::git_api::{
    GitCommit, GitCommitInfo, GitCommitParams, GitCommitResult, GitDiff, GitDiffParams,
    GitDiffResult, GitDone, GitLog, GitLogParams, GitLogResult, GitPathsParams, GitRepoParams,
    GitStage, GitStatus, GitStatusResult, GitUnstage,
};
use agentsi_ui::components::{Button, ListRow, neutral_tag, status_tag};
use agentsi_ui::style::StyleExt as _;
use agentsi_ui::tokens::{Status, control, line, size, space, surface, text, weight};
use gpui_kit::component::diff::{Diff, DiffFile, DiffParseOptions, DiffState};
use gpui_kit::component::input::{Input, InputEvent, InputState};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;

use super::git_config::GitConfigEditor;
use super::minutes::cursor_after;
use super::parts::{Load, first_screen_ready, mono, notice};
use crate::model::CoreState;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Tab {
    Changes,
    History,
    Config,
}

/// One file in one of the two lists.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Row {
    pub path: String,
    pub orig_path: Option<String>,
    /// True in the "Staged" list, false in "Changes".
    pub staged: bool,
    /// The state letter: `M` `A` `D` `R` `C` `T` `U` or `?`.
    pub letter: String,
}

/// The rows of the screen: the staged files first, then the changes. A file with both states
/// is in both lists.
pub fn rows_of(status: &GitStatusResult) -> Vec<Row> {
    let side = |staged: bool| {
        status.files.iter().filter_map(move |f| {
            let letter = if staged { &f.staged } else { &f.unstaged };
            Some(Row {
                path: f.path.clone(),
                orig_path: f.orig_path.clone(),
                staged,
                letter: letter.clone()?,
            })
        })
    };
    side(true).chain(side(false)).collect()
}

/// What the file list draws: a title with its count, or a file row.
#[derive(Clone, Debug, PartialEq, Eq)]
enum Item {
    Head { staged: bool, count: usize },
    File(usize),
}

fn items_of(rows: &[Row]) -> Vec<Item> {
    let staged = rows.iter().filter(|r| r.staged).count();
    let mut out = vec![Item::Head {
        staged: true,
        count: staged,
    }];
    out.extend((0..staged).map(Item::File));
    out.push(Item::Head {
        staged: false,
        count: rows.len() - staged,
    });
    out.extend((staged..rows.len()).map(Item::File));
    out
}

/// What the right side shows.
enum DiffView {
    Loading,
    /// A patch that the viewer did not take, or a line to say why there is none.
    Text(Vec<String>),
    Patch,
}

struct Shown {
    path: String,
    staged: bool,
    truncated: bool,
    view: DiffView,
}

/// The lines of a fallback patch that the screen draws.
const TEXT_LINES: usize = 400;

pub struct GitScreen {
    product: String,
    core: Entity<CoreState>,
    status: Load<GitStatusResult>,
    log: Load<Vec<GitCommitInfo>>,
    tab: Tab,
    cursor: usize,
    shown: Option<Shown>,
    diff: Option<Entity<DiffState>>,
    message: Option<Entity<InputState>>,
    /// The line of the last action: a commit id or an error.
    note: Option<Result<String, String>>,
    /// The output of a hook that refused the last commit.
    hook: Option<String>,
    busy: bool,
    visible: bool,
    config: Entity<GitConfigEditor>,
    focus: FocusHandle,
    scroll: UniformListScrollHandle,
    _inputs: Vec<Subscription>,
}

impl GitScreen {
    pub fn new(product: String, core: Entity<CoreState>, cx: &mut Context<Self>) -> Self {
        let config = {
            let (product, core) = (product.clone(), core.clone());
            cx.new(|cx| GitConfigEditor::new(Some(product), core, cx))
        };
        let mut screen = GitScreen {
            product,
            core,
            status: Load::Loading,
            log: Load::Loading,
            tab: Tab::Changes,
            cursor: 0,
            shown: None,
            diff: None,
            message: None,
            note: None,
            hook: None,
            busy: false,
            visible: false,
            config,
            focus: cx.focus_handle().tab_stop(true),
            scroll: UniformListScrollHandle::new(),
            _inputs: Vec::new(),
        };
        screen.reload(cx);
        screen
    }

    pub fn focus_handle(&self) -> FocusHandle {
        self.focus.clone()
    }

    /// The dock shows or hides this screen. A screen that comes into view reads again.
    pub fn show(&mut self, shown: bool, cx: &mut Context<Self>) {
        let came = shown && !self.visible;
        self.visible = shown;
        if came {
            self.reload(cx);
        }
    }

    fn repo(&self) -> GitRepoParams {
        GitRepoParams {
            product: self.product.clone(),
            repo: None,
        }
    }

    /// Reads the status, and the tab that is open.
    pub fn reload(&mut self, cx: &mut Context<Self>) {
        let call = self.core.read(cx).link.call::<GitStatus>(&self.repo());
        cx.spawn(async move |this, cx| {
            let result = call.await.map_err(|e| e.to_string());
            let _ = this.update(cx, |this, cx| this.status_loaded(result, cx));
        })
        .detach();
        match self.tab {
            Tab::History => self.load_log(cx),
            Tab::Config => self.config.update(cx, |c, cx| c.reload(cx)),
            Tab::Changes => {}
        }
    }

    /// The answer of `git.status`. The cursor stays on its file when the file is still there.
    pub fn status_loaded(
        &mut self,
        result: Result<GitStatusResult, String>,
        cx: &mut Context<Self>,
    ) {
        let was = self.row().map(|r| (r.path, r.staged));
        self.status.settle(result);
        let rows = self.rows();
        self.cursor = was
            .and_then(|(path, staged)| {
                rows.iter()
                    .position(|r| r.path == path && r.staged == staged)
            })
            .unwrap_or_else(|| self.cursor.min(rows.len().saturating_sub(1)));
        first_screen_ready();
        self.select(cx);
        cx.notify();
    }

    pub fn rows(&self) -> Vec<Row> {
        match &self.status {
            Load::Ready(s) => rows_of(s),
            _ => Vec::new(),
        }
    }

    fn row(&self) -> Option<Row> {
        self.rows().into_iter().nth(self.cursor)
    }

    /// Shows the diff of the file under the cursor. It reads the diff again when the
    /// file or its side changed, or when the status is new.
    fn select(&mut self, cx: &mut Context<Self>) {
        let Some(row) = self.row() else {
            self.shown = None;
            return;
        };
        let same = self
            .shown
            .as_ref()
            .is_some_and(|s| s.path == row.path && s.staged == row.staged);
        if !same {
            self.shown = Some(Shown {
                path: row.path.clone(),
                staged: row.staged,
                truncated: false,
                view: DiffView::Loading,
            });
        }
        let call = self.core.read(cx).link.call::<GitDiff>(&GitDiffParams {
            product: self.product.clone(),
            repo: None,
            path: row.path.clone(),
            staged: row.staged,
        });
        cx.spawn(async move |this, cx| {
            let result = call.await.map_err(|e| e.to_string());
            let _ = this.update(cx, |this, cx| {
                this.diff_loaded(&row.path, row.staged, result, cx)
            });
        })
        .detach();
    }

    /// The answer of `git.diff`. It counts only while the file is still the selected one.
    pub fn diff_loaded(
        &mut self,
        path: &str,
        staged: bool,
        result: Result<GitDiffResult, String>,
        cx: &mut Context<Self>,
    ) {
        let Some(shown) = self.shown.as_mut() else {
            return;
        };
        if shown.path != path || shown.staged != staged {
            return;
        }
        match result {
            Err(e) => {
                shown.truncated = false;
                shown.view = DiffView::Text(vec![format!("The diff did not load: {e}")]);
            }
            Ok(d) => {
                shown.truncated = d.truncated;
                shown.view = if d.patch.trim().is_empty() {
                    DiffView::Text(vec!["This side has no text change to show.".into()])
                } else {
                    let options = DiffParseOptions::new().truncated_hunks(d.truncated);
                    match DiffFile::parse_with(&d.patch, options) {
                        Ok(files) if !files.is_empty() => {
                            match &self.diff {
                                Some(state) => state.update(cx, |s, cx| s.set_files(files, cx)),
                                None => self.diff = Some(cx.new(|cx| DiffState::new(files, cx))),
                            }
                            DiffView::Patch
                        }
                        _ => DiffView::Text(
                            d.patch.lines().take(TEXT_LINES).map(String::from).collect(),
                        ),
                    }
                };
            }
        }
        cx.notify();
    }

    fn load_log(&mut self, cx: &mut Context<Self>) {
        let call = self.core.read(cx).link.call::<GitLog>(&GitLogParams {
            product: self.product.clone(),
            repo: None,
            limit: 50,
        });
        cx.spawn(async move |this, cx| {
            let result = call.await.map_err(|e| e.to_string());
            let _ = this.update(cx, |this, cx| this.log_loaded(result, cx));
        })
        .detach();
    }

    pub fn log_loaded(&mut self, result: Result<GitLogResult, String>, cx: &mut Context<Self>) {
        self.log.settle(result.map(|r| r.commits));
        cx.notify();
    }

    fn set_tab(&mut self, tab: Tab, cx: &mut Context<Self>) {
        self.tab = tab;
        match tab {
            Tab::History => self.load_log(cx),
            Tab::Config => self.config.update(cx, |c, cx| c.reload(cx)),
            Tab::Changes => {}
        }
        cx.notify();
    }

    // The actions.

    /// Stages the file under the cursor, or unstages it.
    pub fn toggle_selected(&mut self, cx: &mut Context<Self>) {
        if let Some(row) = self.row() {
            self.move_files(vec![row.path], !row.staged, cx);
        }
    }

    pub fn stage_all(&mut self, cx: &mut Context<Self>) {
        let paths = self.paths(false);
        self.move_files(paths, true, cx);
    }

    pub fn unstage_all(&mut self, cx: &mut Context<Self>) {
        let paths = self.paths(true);
        self.move_files(paths, false, cx);
    }

    fn paths(&self, staged: bool) -> Vec<String> {
        self.rows()
            .into_iter()
            .filter(|r| r.staged == staged)
            .map(|r| r.path)
            .collect()
    }

    fn move_files(&mut self, paths: Vec<String>, stage: bool, cx: &mut Context<Self>) {
        if paths.is_empty() || self.busy {
            return;
        }
        let p = GitPathsParams {
            product: self.product.clone(),
            repo: None,
            paths,
        };
        let link = self.core.read(cx).link.clone();
        let call = if stage {
            let call = link.call::<GitStage>(&p);
            cx.background_spawn(async move { call.await.map_err(|e| e.to_string()) })
        } else {
            let call = link.call::<GitUnstage>(&p);
            cx.background_spawn(async move { call.await.map_err(|e| e.to_string()) })
        };
        self.busy = true;
        self.note = None;
        cx.spawn(async move |this, cx| {
            let result = call.await;
            let _ = this.update(cx, |this, cx| this.moved(result, cx));
        })
        .detach();
        cx.notify();
    }

    /// The answer of `git.stage` or `git.unstage`. The status is read again in both cases.
    pub fn moved(&mut self, result: Result<GitDone, String>, cx: &mut Context<Self>) {
        self.busy = false;
        if let Err(e) = result {
            self.note = Some(Err(e));
        }
        self.reload(cx);
        cx.notify();
    }

    fn message(&mut self, window: &mut Window, cx: &mut Context<Self>) -> Entity<InputState> {
        if let Some(m) = &self.message {
            return m.clone();
        }
        let input = cx.new(|cx| InputState::new(window, cx).placeholder("Commit message"));
        self._inputs.push(cx.subscribe_in(
            &input,
            window,
            |this, _, e: &InputEvent, window, cx| match e {
                InputEvent::PressEnter { .. } => this.commit(window, cx),
                InputEvent::Change => cx.notify(),
                _ => {}
            },
        ));
        self.message = Some(input.clone());
        input
    }

    fn text(&self, cx: &App) -> String {
        self.message
            .as_ref()
            .map(|m| m.read(cx).value().trim().to_string())
            .unwrap_or_default()
    }

    /// The button is off with no staged file, an empty message, or an action in flight.
    pub fn can_commit(&self, cx: &App) -> bool {
        !self.busy && self.rows().iter().any(|r| r.staged) && !self.text(cx).is_empty()
    }

    pub fn commit(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if !self.can_commit(cx) {
            return;
        }
        let call = self.core.read(cx).link.call::<GitCommit>(&GitCommitParams {
            product: self.product.clone(),
            repo: None,
            message: self.text(cx),
        });
        self.busy = true;
        self.note = None;
        cx.spawn_in(window, async move |this, cx| {
            let result = call.await.map_err(|e| e.to_string());
            let _ = this.update_in(cx, |this, window, cx| this.committed(result, window, cx));
        })
        .detach();
        cx.notify();
    }

    /// The answer of `git.commit`. A refused commit keeps the message and shows the output.
    pub fn committed(
        &mut self,
        result: Result<GitCommitResult, String>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.busy = false;
        match result {
            Ok(r) if r.committed => {
                self.hook = None;
                let id: String = r.commit.unwrap_or_default().chars().take(8).collect();
                self.note = Some(Ok(format!("Committed {id}.")));
                if let Some(m) = &self.message {
                    m.update(cx, |i, cx| i.set_value("", window, cx));
                }
            }
            Ok(r) => {
                self.hook = Some(if r.output.is_empty() {
                    "Git refused the commit and gave no output.".into()
                } else {
                    r.output
                });
            }
            Err(e) => self.note = Some(Err(e)),
        }
        self.reload(cx);
        cx.notify();
    }

    fn key_down(&mut self, e: &KeyDownEvent, cx: &mut Context<Self>) {
        let k = &e.keystroke;
        if k.modifiers.modified() {
            return;
        }
        if k.key == "space" {
            self.toggle_selected(cx);
            cx.stop_propagation();
        } else if let Some(to) = cursor_after(&k.key, self.cursor, self.rows().len()) {
            self.cursor = to;
            if let Some(i) = items_of(&self.rows())
                .iter()
                .position(|it| *it == Item::File(to))
            {
                self.scroll.scroll_to_item(i, ScrollStrategy::Nearest);
            }
            self.select(cx);
            cx.stop_propagation();
            cx.notify();
        }
    }
}

/// `date` of a commit time.
fn day(secs: i64) -> String {
    crate::format::date_from_secs(secs)
}

fn state_tag(letter: &str) -> gpui_kit::component::tag::Tag {
    match letter {
        "A" | "?" => status_tag(letter.to_string(), Status::Ok),
        "D" | "U" => status_tag(letter.to_string(), Status::Danger),
        "M" | "T" => status_tag(letter.to_string(), Status::Warn),
        _ => status_tag(letter.to_string(), Status::Info),
    }
}

impl GitScreen {
    fn toolbar(&self, cx: &mut Context<Self>) -> Div {
        let head = match &self.status {
            Load::Ready(s) => {
                let name = s.branch.clone().unwrap_or_else(|| {
                    format!("detached at {}", s.head.clone().unwrap_or_default())
                });
                let mut tags = Vec::new();
                if let (Some(a), Some(b)) = (s.ahead, s.behind) {
                    tags.push(neutral_tag(format!("ahead {a} · behind {b}")));
                }
                if s.worktrees.len() > 1 {
                    tags.push(neutral_tag(format!("{} worktrees", s.worktrees.len())));
                }
                (name, s.upstream.clone(), tags)
            }
            _ => ("Git".into(), None, Vec::new()),
        };
        let tab = |id: &'static str, label: &'static str, t: Tab, cx: &mut Context<Self>| {
            let b = Button::new(id).small().label(label);
            let b = if self.tab == t {
                b.primary()
            } else {
                b.ghost()
            };
            b.on_click(cx.listener(move |this, _, _, cx| this.set_tab(t, cx)))
        };
        div()
            .flex()
            .items_center()
            .gap(space(3))
            .h(control::TOOLBAR_HEIGHT)
            .flex_none()
            .pad_x(space(4))
            .border_b_1()
            .border_color(line::BORDER)
            .child(
                div()
                    .debug_selector(|| "git-branch".into())
                    .text_size(size::md())
                    .font_weight(weight::SEMIBOLD)
                    .text_color(text::PRIMARY)
                    .child(head.0),
            )
            .children(head.2)
            .when_some(head.1, |el, up| el.child(mono(up)))
            .child(div().flex_1())
            .child(tab("git-tab-changes", "Changes", Tab::Changes, cx))
            .child(tab("git-tab-history", "History", Tab::History, cx))
            .child(tab("git-tab-config", "Config", Tab::Config, cx))
            .child(
                Button::new("git-refresh")
                    .secondary()
                    .small()
                    .label("Refresh")
                    .on_click(cx.listener(|this, _, _, cx| this.reload(cx))),
            )
    }

    fn file_list(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let items = items_of(&self.rows());
        let focused = self.focus.is_focused(window);
        let count = items.len();
        uniform_list(
            "git-files",
            count,
            cx.processor(move |this, range: std::ops::Range<usize>, _, cx| {
                let rows = this.rows();
                let items = items_of(&rows);
                range
                    .filter_map(|i| {
                        let item = items.get(i)?.clone();
                        Some(match item {
                            Item::Head { staged, count } => {
                                this.head(i, staged, count, cx).into_any_element()
                            }
                            Item::File(n) => {
                                let r = rows.get(n)?;
                                let meta = r.orig_path.as_ref().map(|o| format!("from {o}"));
                                let row = ListRow::new(("git-file", i), r.path.clone())
                                    .selected(this.cursor == n)
                                    .focused(this.cursor == n && focused)
                                    .trailing(state_tag(&r.letter))
                                    .on_click(cx.listener(move |this, _, window, cx| {
                                        this.cursor = n;
                                        window.focus(&this.focus, cx);
                                        this.select(cx);
                                        cx.notify();
                                    }));
                                let row = match meta {
                                    Some(m) => row.meta(m),
                                    None => row,
                                };
                                div().pad_x(space(1)).child(row).into_any_element()
                            }
                        })
                    })
                    .collect::<Vec<_>>()
            }),
        )
        .track_scroll(&self.scroll)
        .flex_1()
        .min_h_0()
        .p(space(1))
    }

    fn head(&self, i: usize, staged: bool, count: usize, cx: &mut Context<Self>) -> Div {
        let (title, label) = if staged {
            ("Staged", "Unstage all")
        } else {
            ("Changes", "Stage all")
        };
        div()
            .flex()
            .items_center()
            .justify_between()
            .pad_x(space(3))
            .pt(space(2))
            .child(
                div()
                    .text_size(size::sm())
                    .font_weight(weight::SEMIBOLD)
                    .text_color(text::SECONDARY)
                    .child(format!("{title}  {count}")),
            )
            .child(
                Button::new(("git-all", i))
                    .ghost()
                    .small()
                    .label(label)
                    .disabled(count == 0 || self.busy)
                    .on_click(cx.listener(move |this, _, _, cx| {
                        if staged {
                            this.unstage_all(cx)
                        } else {
                            this.stage_all(cx)
                        }
                    })),
            )
    }

    fn diff_pane(&self, cx: &mut Context<Self>) -> Div {
        let pane = div().flex_1().min_w_0().flex().flex_col();
        let Some(shown) = &self.shown else {
            return pane.child(notice(
                "No changed file.",
                Some("Edit a file, or press Refresh.".into()),
            ));
        };
        let toggle = if shown.staged {
            "Unstage this file"
        } else {
            "Stage this file"
        };
        let header = div()
            .flex()
            .items_center()
            .justify_between()
            .gap(space(2))
            .h(control::row())
            .flex_none()
            .pad_x(space(3))
            .border_b_1()
            .border_color(line::BORDER)
            .child(mono(format!(
                "{} · {}",
                shown.path,
                if shown.staged { "staged" } else { "not staged" }
            )))
            .child(
                Button::new("git-toggle")
                    .secondary()
                    .small()
                    .label(toggle)
                    .disabled(self.busy)
                    .on_click(cx.listener(|this, _, _, cx| this.toggle_selected(cx))),
            );
        let body = match (&shown.view, &self.diff) {
            (DiffView::Loading, _) => notice("Reading the diff…", None).into_any_element(),
            (DiffView::Text(lines), _) => div()
                .id("git-text")
                .size_full()
                .overflow_y_scroll()
                .p(space(3))
                .flex()
                .flex_col()
                .gap(space(1))
                .children(lines.iter().map(|l| mono(l.clone())))
                .into_any_element(),
            (DiffView::Patch, Some(state)) => div()
                .size_full()
                .child(Diff::new(state).header_visible(false).size_full())
                .into_any_element(),
            (DiffView::Patch, None) => div().into_any_element(),
        };
        pane.child(header)
            .when(shown.truncated, |el| {
                el.child(
                    div()
                        .debug_selector(|| "git-cut".into())
                        .pad_x(space(3))
                        .py(space(1))
                        .text_size(size::sm())
                        .text_color(Status::Warn.color())
                        .child(
                            "The diff is cut at 512 KiB. Read the rest with git diff in a pane.",
                        ),
                )
            })
            .child(
                div()
                    .debug_selector(|| "git-diff".into())
                    .flex_1()
                    .min_h_0()
                    .child(body),
            )
    }

    fn commit_bar(&mut self, window: &mut Window, cx: &mut Context<Self>) -> Div {
        let input = self.message(window, cx);
        let can = self.can_commit(cx);
        div()
            .flex()
            .flex_col()
            .gap(space(2))
            .debug_selector(|| "git-bar".into())
            .flex_none()
            .p(space(3))
            .border_t_1()
            .border_color(line::BORDER)
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap(space(2))
                    .child(div().flex_1().min_w_0().child(Input::new(&input)))
                    .child(
                        Button::new("git-commit")
                            .primary()
                            .small()
                            .label("Commit")
                            .disabled(!can)
                            .on_click(cx.listener(|this, _, window, cx| this.commit(window, cx))),
                    ),
            )
            .when_some(self.hook.clone(), |el, out| {
                el.child(
                    div()
                        .debug_selector(|| "git-hook".into())
                        .flex()
                        .flex_col()
                        .gap(space(1))
                        .child(
                            div()
                                .text_size(size::sm())
                                .text_color(Status::Danger.color())
                                .child("A hook refused the commit. The message stays."),
                        )
                        .children(out.lines().take(40).map(|l| mono(l.to_string()))),
                )
            })
            .when_some(self.note.clone(), |el, n| {
                el.child(match n {
                    Ok(t) => mono(t),
                    Err(e) => div()
                        .text_size(size::sm())
                        .text_color(Status::Danger.color())
                        .child(format!("Error: {e}")),
                })
            })
    }

    fn history(&self) -> AnyElement {
        match &self.log {
            Load::Loading => notice("Reading the history…", None).into_any_element(),
            Load::Failed(e) => {
                notice(format!("The history did not load: {e}"), None).into_any_element()
            }
            Load::Ready(commits) if commits.is_empty() => {
                notice("No commit yet.", None).into_any_element()
            }
            Load::Ready(commits) => div()
                .id("git-history")
                .size_full()
                .overflow_y_scroll()
                .p(space(1))
                .children(commits.iter().enumerate().map(|(i, c)| {
                    div().pad_x(space(1)).child(
                        ListRow::new(("git-commit-row", i), c.subject.clone()).meta(format!(
                            "{} · {} · {}",
                            c.id.chars().take(8).collect::<String>(),
                            c.author,
                            day(c.time)
                        )),
                    )
                }))
                .into_any_element(),
        }
    }
}

impl Render for GitScreen {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let root = div().size_full().flex().flex_col().bg(surface::PANEL);
        let body = match (&self.status, self.tab) {
            (Load::Loading, Tab::Changes) => notice("Reading the repo…", None).into_any_element(),
            (Load::Failed(e), Tab::Changes) => notice(
                format!("The repo did not load: {e}"),
                Some("The folder of the product must be a git repo.".into()),
            )
            .into_any_element(),
            (_, Tab::Changes) => div()
                .size_full()
                .flex()
                .flex_col()
                .child(
                    div()
                        .id("git-body")
                        .track_focus(&self.focus)
                        .key_context("GitList")
                        .on_key_down(
                            cx.listener(|this, e: &KeyDownEvent, _, cx| this.key_down(e, cx)),
                        )
                        .flex_1()
                        .min_h_0()
                        .flex()
                        .child(
                            div()
                                .debug_selector(|| "git-list".into())
                                .w(control::SIDEBAR_WIDTH)
                                .flex_none()
                                .flex()
                                .flex_col()
                                .border_r_1()
                                .border_color(line::BORDER)
                                .child(self.file_list(window, cx)),
                        )
                        .child(self.diff_pane(cx)),
                )
                .child(self.commit_bar(window, cx))
                .into_any_element(),
            (_, Tab::History) => self.history(),
            (_, Tab::Config) => div()
                .id("git-config")
                .size_full()
                .overflow_y_scroll()
                .p(space(4))
                .child(self.config.clone())
                .into_any_element(),
        };
        root.child(self.toolbar(cx))
            .child(div().flex_1().min_h_0().child(body))
    }
}

#[cfg(test)]
mod tests {
    use super::{GitScreen, Item, Row, Tab, items_of, rows_of};
    use crate::link::Link;
    use crate::link::tests::FakeCore;
    use crate::model::CoreState;
    use agentsi_proto::git_api::{GitCommitResult, GitDiffResult, GitDone, GitStatusResult};
    use gpui::{AppContext as _, Entity, TestAppContext, VisualTestContext};
    use serde_json::{Value, json};
    use std::os::unix::net::UnixStream;
    use std::sync::Arc;

    fn status(files: &[(&str, Option<&str>, Option<&str>)]) -> GitStatusResult {
        let files: Vec<Value> = files
            .iter()
            .map(|(p, s, u)| json!({"path": p, "staged": s, "unstaged": u}))
            .collect();
        serde_json::from_value(json!({
            "repo": "/r", "branch": "main", "head": "abc12345", "upstream": "origin/main",
            "ahead": 1, "behind": 0, "worktrees": [], "files": files,
        }))
        .unwrap()
    }

    fn row(path: &str, staged: bool, letter: &str) -> Row {
        Row {
            path: path.into(),
            orig_path: None,
            staged,
            letter: letter.into(),
        }
    }

    #[test]
    fn the_lists_hold_the_staged_files_first_and_a_file_with_two_states_is_in_both() {
        let s = status(&[
            ("a", None, Some("M")),
            ("b", Some("A"), None),
            ("c", Some("M"), Some("M")),
        ]);
        assert_eq!(
            rows_of(&s),
            vec![
                row("b", true, "A"),
                row("c", true, "M"),
                row("a", false, "M"),
                row("c", false, "M")
            ]
        );
        assert_eq!(
            items_of(&rows_of(&s)),
            vec![
                Item::Head {
                    staged: true,
                    count: 2
                },
                Item::File(0),
                Item::File(1),
                Item::Head {
                    staged: false,
                    count: 2
                },
                Item::File(2),
                Item::File(3),
            ]
        );
        assert_eq!(items_of(&[]).len(), 2);
    }

    struct Ui<'a> {
        screen: Entity<GitScreen>,
        fake: FakeCore,
        cx: &'a mut VisualTestContext,
    }

    /// A link with no reader thread: a thread of its own would wake a task under the test
    /// scheduler. The test hands each reply to the screen, as the task of the call does.
    fn ui(cx: &mut TestAppContext) -> Ui<'_> {
        cx.update(agentsi_ui::init);
        let (end, other) = UnixStream::pair().unwrap();
        let core = cx.new(|_| CoreState::new(Arc::new(Link::for_tests(end))));
        let (screen, cx) = cx.add_window_view(|_, cx| GitScreen::new("alpha".into(), core, cx));
        let mut ui = Ui {
            screen,
            fake: FakeCore(other),
            cx,
        };
        // The Config tab reads its file at once, then the screen reads the status.
        assert_eq!(ui.methods(2), ["git.config.get", "git.status"]);
        ui
    }

    impl Ui<'_> {
        /// The methods of the next `n` requests.
        fn methods(&mut self, n: usize) -> Vec<String> {
            (0..n)
                .map(|_| self.fake.request()["method"].as_str().unwrap().to_string())
                .collect()
        }

        fn draw(&mut self) {
            self.cx.update(|window, cx| window.draw(cx).clear(cx));
        }

        fn status(&mut self, s: GitStatusResult) {
            self.screen
                .update(self.cx, |s_, cx| s_.status_loaded(Ok(s), cx));
            self.draw();
        }

        fn rows(&mut self) -> Vec<Row> {
            self.screen.read_with(self.cx, |s, _| s.rows())
        }

        fn message(&mut self, text: &str) {
            let text = text.to_string();
            self.screen.update_in(self.cx, |s, window, cx| {
                let input = s.message(window, cx);
                input.update(cx, |i, cx| i.set_value(text, window, cx));
            });
        }

        fn can_commit(&mut self) -> bool {
            self.screen.read_with(self.cx, |s, cx| s.can_commit(cx))
        }
    }

    #[gpui::test]
    fn the_screen_stages_a_file_commits_it_with_a_message_and_shows_the_new_state(
        cx: &mut TestAppContext,
    ) {
        let mut ui = ui(cx);
        ui.status(status(&[("a.txt", None, Some("?"))]));
        let req = ui.fake.request();
        assert_eq!(req["method"], "git.diff");
        assert_eq!(
            req["params"],
            json!({"product": "alpha", "path": "a.txt", "staged": false})
        );
        let patch = "diff --git a/a.txt b/a.txt\nnew file mode 100644\nindex 0000000..e69de29\n--- /dev/null\n+++ b/a.txt\n@@ -0,0 +1 @@\n+hello\n";
        ui.screen.update(ui.cx, |s, cx| {
            s.diff_loaded(
                "a.txt",
                false,
                Ok(GitDiffResult {
                    patch: patch.into(),
                    truncated: false,
                }),
                cx,
            )
        });
        ui.draw();
        assert_eq!(ui.rows(), vec![row("a.txt", false, "?")]);
        // The button stays off: nothing is staged and there is no message.
        assert!(!ui.can_commit());

        // Space stages the file under the cursor.
        ui.screen.update(ui.cx, |s, cx| s.toggle_selected(cx));
        let req = ui.fake.request();
        assert_eq!(req["method"], "git.stage");
        assert_eq!(
            req["params"],
            json!({"product": "alpha", "paths": ["a.txt"]})
        );
        ui.screen.update(ui.cx, |s, cx| s.moved(Ok(GitDone {}), cx));
        assert_eq!(ui.methods(1), ["git.status"]);
        ui.status(status(&[("a.txt", Some("A"), None)]));
        let req = ui.fake.request();
        assert_eq!(
            (req["method"].as_str(), req["params"]["staged"].as_bool()),
            (Some("git.diff"), Some(true))
        );
        assert_eq!(ui.rows(), vec![row("a.txt", true, "A")]);

        // A staged file alone is not enough: the message is empty.
        assert!(!ui.can_commit());
        ui.message("feat: add a");
        assert!(ui.can_commit());
        ui.screen
            .update_in(ui.cx, |s, window, cx| s.commit(window, cx));
        let req = ui.fake.request();
        assert_eq!(req["method"], "git.commit");
        assert_eq!(
            req["params"],
            json!({"product": "alpha", "message": "feat: add a"})
        );
        let done = GitCommitResult {
            committed: true,
            commit: Some("abcdef0123456789".into()),
            output: String::new(),
        };
        ui.screen
            .update_in(ui.cx, |s, window, cx| s.committed(Ok(done), window, cx));
        assert_eq!(ui.methods(1), ["git.status"]);
        ui.status(status(&[]));
        ui.draw();

        // The new state: no file, the note with the id, and the message is empty again.
        assert!(ui.rows().is_empty());
        ui.screen.read_with(ui.cx, |s, cx| {
            assert_eq!(s.note, Some(Ok("Committed abcdef01.".to_string())));
            assert_eq!(s.text(cx), "");
            assert!(s.hook.is_none());
        });
        assert!(!ui.can_commit());
    }

    #[gpui::test]
    fn the_lists_the_diff_and_the_commit_bar_each_get_room(cx: &mut TestAppContext) {
        let mut ui = ui(cx);
        ui.status(status(&[
            ("a.txt", None, Some("M")),
            ("b.txt", Some("A"), None),
        ]));
        let patch = "diff --git a/a.txt b/a.txt\n--- a/a.txt\n+++ b/a.txt\n@@ -1 +1 @@\n-a\n+b\n";
        ui.screen.update(ui.cx, |s, cx| {
            s.diff_loaded(
                "b.txt",
                true,
                Ok(GitDiffResult {
                    patch: patch.into(),
                    truncated: false,
                }),
                cx,
            )
        });
        ui.draw();
        let (list, diff, bar) = (
            ui.cx.debug_bounds("git-list").unwrap(),
            ui.cx.debug_bounds("git-diff").unwrap(),
            ui.cx.debug_bounds("git-bar").unwrap(),
        );
        assert!(f32::from(list.size.height) > 100., "{list:?}");
        assert!(f32::from(diff.size.height) > 100., "{diff:?}");
        assert!(f32::from(diff.size.width) > 200., "{diff:?}");
        // The list is left of the diff, and the bar is under both.
        assert!(f32::from(list.right()) <= f32::from(diff.left()) + 1.);
        assert!(
            f32::from(bar.top()) >= f32::from(diff.bottom()) - 1.
                && f32::from(bar.top()) >= f32::from(list.bottom()) - 1.
        );
    }

    #[gpui::test]
    fn a_hook_that_refuses_the_commit_shows_its_output_and_keeps_the_message(
        cx: &mut TestAppContext,
    ) {
        let mut ui = ui(cx);
        ui.status(status(&[("a.txt", Some("A"), None)]));
        ui.message("feat: a");
        ui.screen
            .update_in(ui.cx, |s, window, cx| s.commit(window, cx));
        let refused = GitCommitResult {
            committed: false,
            commit: None,
            output: "lint failed: a.txt".into(),
        };
        ui.screen
            .update_in(ui.cx, |s, window, cx| s.committed(Ok(refused), window, cx));
        ui.draw();
        ui.screen.read_with(ui.cx, |s, cx| {
            assert_eq!(s.hook.as_deref(), Some("lint failed: a.txt"));
            assert_eq!(s.text(cx), "feat: a");
        });
        assert!(ui.cx.debug_bounds("git-hook").is_some());
        assert!(
            ui.can_commit(),
            "the owner fixes the hook and commits again"
        );
    }

    #[gpui::test]
    fn a_cut_diff_shows_one_line_that_says_so(cx: &mut TestAppContext) {
        let mut ui = ui(cx);
        ui.status(status(&[("big.txt", None, Some("M"))]));
        let patch =
            "diff --git a/big.txt b/big.txt\n--- a/big.txt\n+++ b/big.txt\n@@ -1 +1 @@\n-a\n+b\n";
        ui.screen.update(ui.cx, |s, cx| {
            s.diff_loaded(
                "big.txt",
                false,
                Ok(GitDiffResult {
                    patch: patch.into(),
                    truncated: true,
                }),
                cx,
            )
        });
        ui.draw();
        assert!(ui.cx.debug_bounds("git-cut").is_some());
        // A diff of a file that is no longer selected changes nothing.
        ui.screen.update(ui.cx, |s, cx| {
            s.diff_loaded(
                "other.txt",
                false,
                Ok(GitDiffResult {
                    patch: String::new(),
                    truncated: false,
                }),
                cx,
            )
        });
        ui.draw();
        assert!(ui.cx.debug_bounds("git-cut").is_some());
    }

    #[gpui::test]
    fn the_screen_reads_again_when_it_gets_the_focus_and_the_history_tab_reads_the_log(
        cx: &mut TestAppContext,
    ) {
        let mut ui = ui(cx);
        ui.screen.update(ui.cx, |s, cx| s.show(false, cx));
        ui.screen.update(ui.cx, |s, cx| s.show(true, cx));
        assert_eq!(ui.methods(1), ["git.status"]);
        // Shown again while shown: no new read.
        ui.screen.update(ui.cx, |s, cx| s.show(true, cx));
        ui.screen.update(ui.cx, |s, cx| s.set_tab(Tab::History, cx));
        let req = ui.fake.request();
        assert_eq!(req["method"], "git.log");
        assert_eq!(req["params"]["limit"], 50);
        let log = serde_json::from_value(json!({"commits": [
            {"id": "0123456789abcdef", "subject": "feat: a", "author": "Ada", "time": 1_700_000_000}]})).unwrap();
        ui.screen.update(ui.cx, |s, cx| s.log_loaded(Ok(log), cx));
        ui.draw();
        ui.screen.update(ui.cx, |s, cx| s.set_tab(Tab::Config, cx));
        ui.draw();
    }
}
