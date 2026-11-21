//! Fetch, Pull and Push in the header of the git screen, with ahead and behind. A push first
//! reads `git.outgoing` and shows a confirm with the branch, the remote and the commits that
//! go out. A pull that cannot fast-forward says to use a terminal. While a call runs, the
//! last `git.progress` line of the product shows under the header.

use super::GitScreen;
use crate::screens::parts::{Load, mono, section};
use agentsi_proto::events::GitProgressParams;
use agentsi_proto::git_api::{
    GitFetch, GitOutcome, GitOutgoing, GitOutgoingResult, GitPull, GitPush, GitPushParams,
    GitSyncResult,
};
use agentsi_ui::components::Button;
use agentsi_ui::style::StyleExt as _;
use agentsi_ui::tokens::{Status, line, radius, size, space, surface, text};
use gpui_kit::*;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Op {
    Fetch,
    Pull,
    Push,
}

impl Op {
    fn word(self) -> &'static str {
        match self {
            Op::Fetch => "fetch",
            Op::Pull => "pull",
            Op::Push => "push",
        }
    }
}

#[derive(Default)]
pub(super) struct Sync {
    running: Option<Op>,
    pub(super) progress: Option<String>,
    /// The confirm of a push: what goes out.
    confirm: Option<Load<GitOutgoingResult>>,
    /// The line of the last fetch, pull, push or branch action: a result or an error.
    pub(super) note: Option<Result<String, String>>,
}

impl GitScreen {
    pub fn fetch(&mut self, cx: &mut Context<Self>) {
        self.run(Op::Fetch, None, cx);
    }

    pub fn pull(&mut self, cx: &mut Context<Self>) {
        self.run(Op::Pull, None, cx);
    }

    /// Reads what a push sends and asks the confirm. Nothing is pushed before the confirm.
    pub fn ask_push(&mut self, cx: &mut Context<Self>) {
        if self.sync.running.is_some() {
            return;
        }
        let call = self.core.read(cx).link.call::<GitOutgoing>(&self.repo());
        self.sync.confirm = Some(Load::Loading);
        self.sync.note = None;
        cx.spawn(async move |this, cx| {
            let result = call.await.map_err(|e| e.to_string());
            let _ = this.update(cx, |this, cx| this.outgoing_loaded(result, cx));
        })
        .detach();
        cx.notify();
    }

    pub fn outgoing_loaded(
        &mut self,
        result: Result<GitOutgoingResult, String>,
        cx: &mut Context<Self>,
    ) {
        if self.sync.confirm.is_none() {
            return;
        }
        let mut load = Load::Loading;
        load.settle(result);
        self.sync.confirm = Some(load);
        cx.notify();
    }

    /// Pushes what the confirm listed: the core refuses the push when the branch or its head
    /// moved since.
    pub fn confirm_push(&mut self, cx: &mut Context<Self>) {
        if self.sync.running.is_some() || self.busy {
            return;
        }
        let Some(Load::Ready(o)) = &self.sync.confirm else {
            return;
        };
        if o.count == 0 {
            return;
        }
        let expect = (o.branch.clone(), o.head.clone());
        self.sync.confirm = None;
        self.run(Op::Push, Some(expect), cx);
    }

    pub fn cancel_push(&mut self, cx: &mut Context<Self>) {
        self.sync.confirm = None;
        cx.notify();
    }

    fn run(&mut self, op: Op, expect: Option<(String, String)>, cx: &mut Context<Self>) {
        if self.sync.running.is_some() || self.busy {
            return;
        }
        let p = self.repo();
        let link = self.core.read(cx).link.clone();
        let call = match op {
            Op::Fetch => {
                let call = link.call::<GitFetch>(&p);
                cx.background_spawn(async move { call.await.map_err(|e| e.to_string()) })
            }
            Op::Pull => {
                let call = link.call::<GitPull>(&p);
                cx.background_spawn(async move { call.await.map_err(|e| e.to_string()) })
            }
            Op::Push => {
                let (branch, head) = expect.unzip();
                let call = link.call::<GitPush>(&GitPushParams {
                    product: p.product,
                    repo: p.repo,
                    branch,
                    head,
                });
                cx.background_spawn(async move { call.await.map_err(|e| e.to_string()) })
            }
        };
        self.sync.running = Some(op);
        self.sync.progress = None;
        self.sync.note = None;
        cx.spawn(async move |this, cx| {
            let result = call.await;
            let _ = this.update(cx, |this, cx| this.synced(op, result, cx));
        })
        .detach();
        cx.notify();
    }

    /// The answer of a fetch, a pull or a push.
    pub fn synced(
        &mut self,
        op: Op,
        result: Result<GitSyncResult, String>,
        cx: &mut Context<Self>,
    ) {
        self.sync.running = None;
        self.sync.progress = None;
        self.sync.note = Some(match result {
            Ok(r) => match r.outcome {
                GitOutcome::Done => Ok(match op {
                    Op::Fetch => "Fetched.".into(),
                    Op::Pull => "Pulled.".into(),
                    Op::Push => "Pushed.".into(),
                }),
                GitOutcome::Diverged => Err("The branch and its upstream both have new \
                    commits, so a fast-forward pull cannot run. The tree did not change. \
                    Merge or rebase in a terminal."
                    .into()),
                _ if r.output.is_empty() => Err(format!("The {} failed.", op.word())),
                _ => Err(r.output),
            },
            Err(e) => Err(e),
        });
        self.reload(cx);
        cx.notify();
    }

    pub(super) fn take_progress(&mut self, p: GitProgressParams, cx: &mut Context<Self>) {
        if self.sync.running.is_some_and(|op| op.word() == p.op) {
            self.sync.progress = Some(p.line);
            cx.notify();
        }
    }

    pub(super) fn sync_buttons(&self, cx: &mut Context<Self>) -> Vec<Button> {
        let (ahead, behind, upstream, branch) = match &self.status {
            Load::Ready(s) => (
                s.ahead.unwrap_or(0),
                s.behind.unwrap_or(0),
                s.upstream.is_some(),
                s.branch.is_some(),
            ),
            _ => (0, 0, false, false),
        };
        let busy = self.sync.running.is_some() || self.busy;
        let count = |label: &str, arrow: &str, n: u32| match n {
            0 => label.to_string(),
            n => format!("{label} {arrow}{n}"),
        };
        vec![
            Button::new("git-fetch")
                .secondary()
                .small()
                .label("Fetch")
                .disabled(busy)
                .on_click(cx.listener(|this, _, _, cx| this.fetch(cx))),
            Button::new("git-pull")
                .secondary()
                .small()
                .label(count("Pull", "↓", behind))
                .disabled(busy || !upstream)
                .on_click(cx.listener(|this, _, _, cx| this.pull(cx))),
            Button::new("git-push")
                .secondary()
                .small()
                .label(count("Push", "↑", ahead))
                .disabled(busy || !branch)
                .on_click(cx.listener(|this, _, _, cx| this.ask_push(cx))),
        ]
    }

    fn push_card(&self, cx: &mut Context<Self>) -> Option<Div> {
        let card = div()
            .debug_selector(|| "git-push-confirm".into())
            .flex()
            .flex_col()
            .gap(space(2))
            .m(space(3))
            .p(space(3))
            .rounded(radius::lg())
            .border_1()
            .border_color(line::border())
            .bg(surface::raised());
        let muted = |t: String| {
            div()
                .text_size(size::sm())
                .text_color(text::secondary())
                .child(t)
        };
        let cancel = Button::new("git-push-cancel")
            .secondary()
            .small()
            .label("Cancel")
            .on_click(cx.listener(|this, _, _, cx| this.cancel_push(cx)));
        Some(match self.sync.confirm.as_ref()? {
            Load::Loading => card.child(muted("Reading the commits that go out…".into())),
            Load::Failed(e) => card
                .child(muted(format!("The push cannot run: {e}")))
                .child(div().flex().child(cancel)),
            Load::Ready(o) => {
                let target = format!("{}/{}", o.remote, o.remote_branch);
                let head = if o.set_upstream {
                    format!("Push {} to {target}, and set it as the upstream", o.branch)
                } else {
                    format!("Push {} to {target}", o.branch)
                };
                let more = o.count as usize - o.commits.len().min(o.count as usize);
                card.child(section(&head, Some(o.count as usize)))
                    .children(o.commits.iter().map(|c| {
                        mono(format!(
                            "{} {}",
                            c.id.chars().take(8).collect::<String>(),
                            c.subject
                        ))
                    }))
                    .children((more > 0).then(|| muted(format!("and {more} more"))))
                    .children((o.count == 0).then(|| muted("No commit goes out.".to_string())))
                    .child(
                        div()
                            .flex()
                            .gap(space(2))
                            .child(
                                Button::new("git-push-confirm-button")
                                    .primary()
                                    .small()
                                    .label("Push")
                                    .disabled(o.count == 0 || self.sync.running.is_some())
                                    .on_click(cx.listener(|this, _, _, cx| this.confirm_push(cx))),
                            )
                            .child(cancel),
                    )
            }
        })
    }

    /// The progress, the confirm of a push and the line of the last action, under the header.
    pub(super) fn sync_strip(&self, cx: &mut Context<Self>) -> Vec<Div> {
        let mut out = Vec::new();
        if let Some(op) = self.sync.running {
            let line = self
                .sync
                .progress
                .clone()
                .unwrap_or_else(|| format!("The {} runs…", op.word()));
            out.push(
                div()
                    .debug_selector(|| "git-progress".into())
                    .pad_x(space(4))
                    .py(space(1))
                    .border_b_1()
                    .border_color(line::border())
                    .child(mono(line)),
            );
        }
        out.extend(self.push_card(cx));
        if let Some(note) = &self.sync.note {
            let (color, t) = match note {
                Ok(t) => (text::secondary(), t.clone()),
                Err(e) => (Status::Danger.color(), e.clone()),
            };
            out.push(
                div()
                    .debug_selector(|| "git-sync-note".into())
                    .pad_x(space(4))
                    .py(space(1))
                    .border_b_1()
                    .border_color(line::border())
                    .text_size(size::sm())
                    .text_color(color)
                    .children(t.lines().take(20).map(|l| div().child(l.to_string()))),
            );
        }
        out
    }
}
