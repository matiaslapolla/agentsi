//! One story (U23): its criteria, its session state, the diff of its branch, its log,
//! and the review actions that are legal now. Approve writes at once. Changes, Discard,
//! Run, Accept and Reject show what they will do first, and a click on Confirm does it.
//! There is no text field: Changes takes its note from the clipboard, as the roles
//! screen takes its stage folder.

use agentsi_proto::story_api::{
    Story, StoryAccept, StoryApprove, StoryChanges, StoryChangesParams, StoryConfig,
    StoryConfigGet, StoryConfigParams, StoryDiscard, StoryGet, StoryGetResult, StoryIdParams,
    StoryReject, StoryRejectParams, StoryReview, StoryRun,
};
use agentsi_ui::components::{Button, Card, neutral_tag, status_tag};
use agentsi_ui::tokens::{Status, line, radius, size, space, surface, text, weight};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;

use super::parts::{Load, column, first_screen_ready, mono, notice, section};
use super::stories::story_tag;
use crate::menus::{self, Target};
use crate::model::{CoreState, ProductRef};

/// The lines of a diff that the screen draws. The rest is counted.
const DIFF_LINES: usize = 400;
/// The last lines of the log that the screen draws.
const LOG_LINES: usize = 80;

/// What the screen shows of a story.
#[derive(Clone)]
pub struct StoryData {
    pub get: StoryGetResult,
    pub config: Option<StoryConfig>,
    pub stat: String,
    pub diff: String,
}

/// An action that waits for the owner to confirm it.
#[derive(Clone, Debug, PartialEq)]
pub enum Pending {
    Changes(String),
    Discard,
    Run,
    Accept,
    Reject { force: bool },
}

pub struct StoryScreen {
    product: String,
    id: String,
    core: Entity<CoreState>,
    data: Load<StoryData>,
    pending: Option<Pending>,
    message: Option<Result<String, String>>,
    busy: bool,
    scroll: ScrollHandle,
}

/// What a pending action will do, in plain lines.
pub fn pending_lines(
    p: &Pending,
    s: &Story,
    config: Option<&StoryConfig>,
) -> (String, Vec<String>) {
    let branch = s
        .session
        .as_ref()
        .map(|x| x.branch.clone())
        .unwrap_or_else(|| format!("story/{}-{}", s.id, s.slug));
    match p {
        Pending::Changes(note) => (
            "Request changes".into(),
            vec![
                format!("Note: {note}"),
                "The approval becomes changes_requested. Run agentsi story changes to let the PM rewrite it.".into(),
            ],
        ),
        Pending::Discard => (
            "Discard the story".into(),
            vec!["The status becomes discarded. No session runs for it again.".into()],
        ),
        Pending::Run => (
            "Start the story session".into(),
            vec![
                format!("A worktree on {branch} from the main branch."),
                "QA writes the criterion tests, then the owner does the work. Each test and gate runs in bubblewrap.".into(),
                match config {
                    Some(c) => format!("Each Claude session stops at USD {:.2}.", c.budget_usd),
                    None => "Each Claude session stops at the budget of the stories block.".into(),
                },
            ],
        ),
        Pending::Accept => (
            "Merge the story".into(),
            vec![
                format!("Merge {branch} with --no-ff into the main branch."),
                "Then remove the worktree and the branch. The status becomes implemented.".into(),
            ],
        ),
        Pending::Reject { force } => (
            "Reject the session".into(),
            [
                (*force).then(|| "Stop the session that still runs.".to_string()),
                Some(format!("Remove the worktree and the branch {branch}.")),
                Some("Record a discarded result. The story can run again.".into()),
            ]
            .into_iter()
            .flatten()
            .collect(),
        ),
    }
}

/// The last `n` lines of `text`, and how many come before them.
pub fn last_lines(text: &str, n: usize) -> (usize, Vec<String>) {
    let all: Vec<&str> = text.lines().collect();
    let skip = all.len().saturating_sub(n);
    (skip, all[skip..].iter().map(|l| l.to_string()).collect())
}

impl StoryScreen {
    pub fn new(
        product: String,
        id: String,
        core: Entity<CoreState>,
        cx: &mut Context<Self>,
    ) -> Self {
        let mut screen = StoryScreen {
            product,
            id,
            core,
            data: Load::Loading,
            pending: None,
            message: None,
            busy: false,
            scroll: ScrollHandle::new(),
        };
        screen.reload(cx);
        screen
    }

    fn product(&self, cx: &App) -> ProductRef {
        ProductRef {
            host: self.core.read(cx).remote.clone(),
            id: self.product.clone(),
        }
    }

    fn params(&self) -> StoryIdParams {
        StoryIdParams {
            product: self.product.clone(),
            id: self.id.clone(),
        }
    }

    /// Reads the story, the stories block and, while it has a session, its diff.
    pub fn reload(&mut self, cx: &mut Context<Self>) {
        let link = self.core.read(cx).link.clone();
        let params = self.params();
        let get = link.call::<StoryGet>(&params);
        let config = link.call::<StoryConfigGet>(&StoryConfigParams {
            product: self.product.clone(),
            run: false,
        });
        let review = link.call::<StoryReview>(&params);
        cx.spawn(async move |this, cx| {
            let result = match get.await {
                Ok(get) => {
                    let config = config.await.ok();
                    let (stat, diff) = if get.story.session.is_some() {
                        review.await.map(|r| (r.stat, r.diff)).unwrap_or_default()
                    } else {
                        Default::default()
                    };
                    Ok(StoryData {
                        get,
                        config,
                        stat,
                        diff,
                    })
                }
                Err(e) => Err(e.to_string()),
            };
            let _ = this.update(cx, |this, cx| {
                this.data.settle(result);
                first_screen_ready();
                cx.notify();
            });
        })
        .detach();
    }

    fn story(&self) -> Option<&Story> {
        match &self.data {
            Load::Ready(d) => Some(&d.get.story),
            _ => None,
        }
    }

    /// An action waits for the Confirm of the owner.
    #[cfg(test)]
    pub fn has_pending(&self) -> bool {
        self.pending.is_some()
    }

    /// Starts an action: Approve at once, the others after a confirm.
    pub fn act(&mut self, action: &str, cx: &mut Context<Self>) {
        self.message = None;
        let running = self.story().is_some_and(|s| {
            s.session
                .as_ref()
                .is_some_and(|x| x.state != "ready" && x.state != "failed")
        });
        self.pending = match action {
            "approve" => {
                self.pending = None;
                self.send(None, cx);
                return;
            }
            "changes" => {
                let note = cx
                    .read_from_clipboard()
                    .and_then(|c| c.text())
                    .map(|t| t.trim().to_string())
                    .unwrap_or_default();
                if note.is_empty() {
                    self.message = Some(Err("Copy the note of the change request first.".into()));
                    cx.notify();
                    return;
                }
                Some(Pending::Changes(note))
            }
            "discard" => Some(Pending::Discard),
            "run" => Some(Pending::Run),
            "accept" => Some(Pending::Accept),
            "reject" => Some(Pending::Reject { force: running }),
            _ => None,
        };
        cx.notify();
    }

    /// Sends the pending action, or Approve when `pending` is `None`.
    fn send(&mut self, pending: Option<Pending>, cx: &mut Context<Self>) {
        let link = self.core.read(cx).link.clone();
        let p = self.params();
        let id = self.id.clone();
        let done: Task<Result<String, String>> = match pending {
            None => {
                let call = link.call::<StoryApprove>(&p);
                cx.background_spawn(async move {
                    call.await
                        .map(|_| format!("Approved {id}."))
                        .map_err(|e| e.to_string())
                })
            }
            Some(Pending::Changes(note)) => {
                let call = link.call::<StoryChanges>(&StoryChangesParams {
                    product: p.product,
                    id: p.id,
                    note,
                    rewrite: false,
                });
                cx.background_spawn(async move {
                    call.await
                        .map(|_| format!("Changes requested for {id}."))
                        .map_err(|e| e.to_string())
                })
            }
            Some(Pending::Discard) => {
                let call = link.call::<StoryDiscard>(&p);
                cx.background_spawn(async move {
                    call.await
                        .map(|_| format!("Discarded {id}."))
                        .map_err(|e| e.to_string())
                })
            }
            Some(Pending::Run) => {
                let call = link.call::<StoryRun>(&p);
                cx.background_spawn(async move {
                    call.await
                        .map(|r| {
                            let branch = r.story.session.map(|s| s.branch).unwrap_or_default();
                            format!("The story session of {id} started on {branch}. Reload to follow it.")
                        })
                        .map_err(|e| e.to_string())
                })
            }
            Some(Pending::Accept) => {
                let call = link.call::<StoryAccept>(&p);
                cx.background_spawn(async move {
                    call.await
                        .map(|r| {
                            format!(
                                "Story {id} merged into the main branch as {} ({}).",
                                r.commit, r.result
                            )
                        })
                        .map_err(|e| e.to_string())
                })
            }
            Some(Pending::Reject { force }) => {
                let call = link.call::<StoryReject>(&StoryRejectParams {
                    product: p.product,
                    id: p.id,
                    force,
                });
                cx.background_spawn(async move {
                    call.await
                        .map(|_| format!("Story {id} discarded: worktree and branch removed. It can run again."))
                        .map_err(|e| e.to_string())
                })
            }
        };
        self.busy = true;
        cx.spawn(async move |this, cx| {
            let result = done.await;
            let _ = this.update(cx, |this, cx| {
                this.busy = false;
                this.message = Some(result);
                this.reload(cx);
                cx.notify();
            });
        })
        .detach();
        cx.notify();
    }

    fn confirm(&mut self, cx: &mut Context<Self>) {
        if let Some(p) = self.pending.take() {
            self.send(Some(p), cx);
        }
    }

    fn cancel(&mut self, cx: &mut Context<Self>) {
        self.pending = None;
        cx.notify();
    }
}

fn label(action: &str) -> &'static str {
    match action {
        "approve" => "Approve",
        "changes" => "Request changes (note from the clipboard)",
        "discard" => "Discard",
        "run" => "Run the story session",
        "accept" => "Accept and merge",
        _ => "Reject the session",
    }
}

fn text_block(s: impl Into<SharedString>) -> Div {
    div()
        .text_size(size::sm())
        .text_color(text::secondary())
        .child(s.into())
}

fn lines_block(lines: &[String]) -> Div {
    div()
        .flex()
        .flex_col()
        .gap(space(1))
        .children(lines.iter().map(|l| mono(l.clone())))
}

impl StoryScreen {
    fn header(&self, d: &StoryData, cx: &mut Context<Self>) -> Div {
        let s = &d.get.story;
        let mut tags = vec![neutral_tag(s.kind.clone()), story_tag(s)];
        if d.get.archived {
            tags.push(status_tag("archived", Status::Warn));
        }
        let busy = self.busy;
        div()
            .flex()
            .flex_col()
            .gap(space(3))
            .child(
                div()
                    .flex()
                    .flex_col()
                    .gap(space(1))
                    .child(
                        div()
                            .text_size(size::xl())
                            .font_weight(weight::SEMIBOLD)
                            .text_color(text::primary())
                            .child(s.title.clone()),
                    )
                    .child(mono(format!(
                        "{} · owner {} · {} · approval {} · proposed by {}{}",
                        s.id,
                        s.owner_role,
                        s.status,
                        s.approval,
                        s.proposed_by,
                        s.minutes
                            .as_ref()
                            .map_or(String::new(), |m| format!(" in minutes {m}"))
                    ))),
            )
            .child(div().flex().gap(space(2)).children(tags))
            .child(
                div()
                    .flex()
                    .flex_wrap()
                    .gap(space(2))
                    .children(d.get.actions.iter().enumerate().map(|(i, a)| {
                        let action = a.clone();
                        let button = Button::new(("story-action", i)).small().label(label(a));
                        let button = match a.as_str() {
                            "approve" | "accept" => button.primary(),
                            "discard" | "reject" => button.danger(),
                            _ => button.secondary(),
                        };
                        button
                            .disabled(busy)
                            .on_click(cx.listener(move |this, _, _, cx| this.act(&action, cx)))
                    }))
                    .child(
                        Button::new("story-reload")
                            .ghost()
                            .small()
                            .label("Reload")
                            .on_click(cx.listener(|this, _, _, cx| this.reload(cx))),
                    ),
            )
    }

    fn pending_card(&self, d: &StoryData, cx: &mut Context<Self>) -> Option<Div> {
        let p = self.pending.as_ref()?;
        let (title, lines) = pending_lines(p, &d.get.story, d.config.as_ref());
        Some(
            div()
                .flex()
                .flex_col()
                .gap(space(3))
                .p(space(4))
                .rounded(radius::lg())
                .border_1()
                .border_color(line::border())
                .bg(surface::raised())
                .child(section(&title, None))
                .child(lines_block(&lines))
                .child(
                    div()
                        .flex()
                        .gap(space(2))
                        .child(
                            Button::new("story-confirm")
                                .danger()
                                .small()
                                .label("Confirm")
                                .on_click(cx.listener(|this, _, _, cx| this.confirm(cx))),
                        )
                        .child(
                            Button::new("story-cancel")
                                .secondary()
                                .small()
                                .label("Cancel")
                                .on_click(cx.listener(|this, _, _, cx| this.cancel(cx))),
                        ),
                ),
        )
    }
}

fn want(s: &Story) -> Div {
    let mut lines = Vec::new();
    if let Some(a) = s.as_a.as_deref().filter(|a| !a.is_empty()) {
        lines.push(format!("As a {a},"));
    }
    lines.push(format!("I want {},", s.i_want));
    lines.push(format!("so that {}.", s.so_that));
    if let Some(note) = s
        .changes_note
        .as_deref()
        .filter(|_| s.approval == "changes_requested")
    {
        lines.push(format!("Changes requested: {note}"));
    }
    div()
        .flex()
        .flex_col()
        .gap(space(1))
        .children(lines.into_iter().map(text_block))
}

fn criteria(s: &Story) -> Div {
    div()
        .flex()
        .flex_col()
        .gap(space(3))
        .child(section("Criteria", Some(s.criteria.len())))
        .children(s.criteria.iter().enumerate().map(|(i, c)| {
            Card::new(("story-criterion", i))
                .child(text_block(format!("Given {}", c.given)))
                .child(text_block(format!("When {}", c.when)))
                .child(text_block(format!("Then {}", c.then)))
                .child(mono(format!(
                    "Test: {} · {}",
                    c.test.as_deref().unwrap_or("none"),
                    c.checks
                )))
        }))
}

fn session_block(d: &StoryData) -> Option<Div> {
    let s = &d.get.story;
    let x = s.session.as_ref()?;
    let mut lines = vec![
        format!("State: {} · attempt {}", x.state, x.attempts),
        format!("Branch: {}", x.branch),
        format!("Worktree: {}", x.worktree),
    ];
    if let Some(c) = &x.tests_commit {
        lines.push(format!("Tests commit: {c}"));
    }
    if let Some(c) = &x.final_commit {
        lines.push(format!("Final commit: {c}"));
    }
    if let Some(Some(stop)) = &x.stop {
        lines.push(format!(
            "Stopped: {}, cap USD {:.2} for each session.",
            stop.reason, stop.cap_usd
        ));
    }
    let failure = x
        .failure
        .as_deref()
        .map(|f| f.lines().map(String::from).collect::<Vec<_>>());
    Some(
        div()
            .flex()
            .flex_col()
            .gap(space(2))
            .child(section("Session", None))
            .child(lines_block(&lines))
            .when_some(failure, |el, f| {
                el.child(text_block("Failure")).child(lines_block(&f))
            }),
    )
}

fn diff_block(d: &StoryData) -> Option<Div> {
    if d.stat.is_empty() && d.diff.is_empty() {
        return None;
    }
    let all: Vec<String> = d.diff.lines().map(String::from).collect();
    let shown = &all[..all.len().min(DIFF_LINES)];
    let more = all.len().saturating_sub(DIFF_LINES);
    Some(
        div()
            .flex()
            .flex_col()
            .gap(space(2))
            .child(section("Diff", None))
            .child(lines_block(
                &d.stat.lines().map(String::from).collect::<Vec<_>>(),
            ))
            .child(
                div()
                    .p(space(3))
                    .rounded(radius::lg())
                    .border_1()
                    .border_color(line::border())
                    .bg(surface::raised())
                    .child(lines_block(shown)),
            )
            .when(more > 0, |el| {
                el.child(text_block(format!(
                    "{more} more lines. Read them with git diff in the repo."
                )))
            }),
    )
}

fn log_block(d: &StoryData) -> Option<Div> {
    if d.get.log.is_empty() {
        return None;
    }
    let (skipped, lines) = last_lines(&d.get.log, LOG_LINES);
    Some(
        div()
            .flex()
            .flex_col()
            .gap(space(2))
            .child(section("Log", None))
            .when(skipped > 0, |el| {
                el.child(text_block(format!("{skipped} older lines are not shown.")))
            })
            .child(lines_block(&lines)),
    )
}

fn results_block(s: &Story) -> Option<Div> {
    if s.results.is_empty() {
        return None;
    }
    let lines: Vec<String> = s
        .results
        .iter()
        .map(|r| match &r.commit {
            Some(c) => format!("{}: {} ({c})", r.at, r.result),
            None => format!("{}: {}", r.at, r.result),
        })
        .collect();
    Some(
        div()
            .flex()
            .flex_col()
            .gap(space(2))
            .child(section("Results", Some(lines.len())))
            .child(lines_block(&lines)),
    )
}

impl Render for StoryScreen {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let (product, id) = (self.product(cx), self.id.clone());
        let root = div()
            .id("story-screen")
            .debug_selector(|| "story-screen".into())
            .size_full()
            .bg(surface::panel())
            .on_mouse_down(MouseButton::Right, move |e, _, cx| {
                menus::request(cx, Target::Story(product.clone(), id.clone()), e.position)
            });
        let d = match &self.data {
            Load::Loading => return root.child(notice("Reading the story…", None)),
            Load::Failed(e) => {
                return root.child(notice(
                    format!("The story did not load: {e}"),
                    Some(super::parts::reload_hint(cx)),
                ));
            }
            Load::Ready(d) => d.clone(),
        };
        let message = self.message.as_ref().map(|m| match m {
            Ok(t) => mono(t.clone()),
            Err(e) => mono(format!("Error: {e}")),
        });
        let s = &d.get.story;
        root.track_scroll(&self.scroll).overflow_y_scroll().child(
            column()
                .child(self.header(&d, cx))
                .children(message)
                .children(self.pending_card(&d, cx))
                .child(want(s))
                .child(criteria(s))
                .children(session_block(&d))
                .children(diff_block(&d))
                .children(results_block(s))
                .children(log_block(&d)),
        )
    }
}

#[cfg(test)]
mod tests {
    use super::{Pending, last_lines, pending_lines};
    use agentsi_proto::story_api::{Story, StoryConfig};

    fn story() -> Story {
        serde_json::from_value(serde_json::json!({
            "id": "S001", "type": "story", "slug": "filter-mail", "title": "Filter mail", "owner_role": "dev",
            "status": "not_implemented", "approval": "approved", "proposed_by": "pm",
            "created_at": "2026-10-01T10:00:00.000Z", "updated_at": "2026-10-01T10:00:00.000Z"
        }))
        .unwrap()
    }

    #[test]
    fn the_run_confirm_names_the_branch_and_the_budget() {
        let config = StoryConfig {
            repo: "/r".into(),
            test: "t {files}".into(),
            gate: vec![],
            budget_usd: 2.0,
        };
        let (title, lines) = pending_lines(&Pending::Run, &story(), Some(&config));
        assert_eq!(title, "Start the story session");
        assert_eq!(
            lines[0],
            "A worktree on story/S001-filter-mail from the main branch."
        );
        assert_eq!(lines[2], "Each Claude session stops at USD 2.00.");
        let (_, reject) = pending_lines(&Pending::Reject { force: true }, &story(), None);
        assert_eq!(reject[0], "Stop the session that still runs.");
    }

    #[gpui::test]
    fn a_story_action_lands_on_the_core_of_the_screen_when_two_cores_have_the_same_story(
        cx: &mut gpui::TestAppContext,
    ) {
        use crate::model::two_cores::{methods, two_cores};
        use gpui_kit::AppContext as _;
        cx.update(agentsi_ui::init);
        let mut t = two_cores(cx);
        let studio = t.cores.of(Some("studio")).unwrap().clone();
        let local = t.cores.local.clone();
        let new = |core| {
            move |cx: &mut gpui_kit::Context<super::StoryScreen>| {
                super::StoryScreen::new("demo".into(), "S001".into(), core, cx)
            }
        };
        let _local_screen = cx.new(new(local));
        let screen = cx.new(new(studio));
        let reads = ["story.get", "story.config", "story.review"];
        assert_eq!(methods(&mut t.local), reads);
        assert_eq!(methods(&mut t.studio), reads);
        screen.update(cx, |s, cx| s.send(None, cx));
        assert_eq!(methods(&mut t.studio), ["story.approve"]);
        assert_eq!(methods(&mut t.local), Vec::<String>::new());
    }

    #[test]
    fn the_log_keeps_its_last_lines() {
        let (skipped, lines) = last_lines("a\nb\nc", 2);
        assert_eq!(
            (skipped, lines),
            (1, vec!["b".to_string(), "c".to_string()])
        );
    }
}
