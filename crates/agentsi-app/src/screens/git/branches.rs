//! The Branches tab: the local branches with the current one first, the remote branches and
//! the worktrees. The selected branch has Switch, New branch from here and Copy name. A
//! switch with changes in the tree shows `dirty` and the files.

use super::GitScreen;
use crate::screens::parts::{Load, mono, notice, section};
use agentsi_proto::git_api::{
    GitBranch, GitBranchCreate, GitBranchCreateParams, GitBranches, GitBranchesResult, GitDone,
    GitFile, GitOutcome, GitSwitch, GitSwitchParams, GitSwitchResult,
};
use agentsi_ui::components::{Button, ListRow};
use agentsi_ui::style::StyleExt as _;
use agentsi_ui::tokens::{Status, line, radius, size, space, surface, text};
use gpui_kit::component::input::{Input, InputEvent, InputState};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;

#[derive(Default)]
pub(super) struct Branches {
    /// The name of the selected branch.
    pub(super) selected: Option<String>,
    pub(super) form: Option<NewBranch>,
    /// The branch of a refused switch, and the changed files that held it.
    dirty: Option<(String, Vec<GitFile>)>,
}

pub(super) struct NewBranch {
    /// The start of the new branch. `None` starts from the head.
    from: Option<String>,
    pub(super) input: Entity<InputState>,
    _events: Subscription,
}

fn ahead_behind(b: &GitBranch) -> Option<String> {
    match (b.ahead, b.behind) {
        (Some(0), Some(0)) => None,
        (Some(a), Some(z)) => Some(format!("↑{a} ↓{z}")),
        _ => None,
    }
}

fn meta(b: &GitBranch) -> String {
    let mut parts = vec![b.head.clone()];
    if let Some(up) = &b.upstream {
        parts.push(up.clone());
    }
    parts.extend(ahead_behind(b));
    parts.push(b.subject.clone());
    parts.join(" · ")
}

impl GitScreen {
    pub(super) fn load_branches(&mut self, cx: &mut Context<Self>) {
        let call = self.core.read(cx).link.call::<GitBranches>(&self.repo());
        cx.spawn(async move |this, cx| {
            let result = call.await.map_err(|e| e.to_string());
            let _ = this.update(cx, |this, cx| this.branches_loaded(result, cx));
        })
        .detach();
    }

    pub fn branches_loaded(
        &mut self,
        result: Result<GitBranchesResult, String>,
        cx: &mut Context<Self>,
    ) {
        self.branches.settle(result);
        if let Load::Ready(b) = &self.branches {
            let known = |name: &str| b.local.iter().chain(&b.remote).any(|x| x.name == name);
            if !self.branch.selected.as_deref().is_some_and(known) {
                self.branch.selected = b.local.first().map(|x| x.name.clone());
            }
        }
        cx.notify();
    }

    pub fn select_branch(&mut self, name: &str, cx: &mut Context<Self>) {
        self.branch.selected = Some(name.to_string());
        cx.notify();
    }

    pub fn switch_to(&mut self, name: String, cx: &mut Context<Self>) {
        if self.busy {
            return;
        }
        let call = self.core.read(cx).link.call::<GitSwitch>(&GitSwitchParams {
            product: self.product.clone(),
            repo: None,
            name: name.clone(),
        });
        self.busy = true;
        self.branch.dirty = None;
        self.sync.note = None;
        cx.spawn(async move |this, cx| {
            let result = call.await.map_err(|e| e.to_string());
            let _ = this.update(cx, |this, cx| this.switched(&name, result, cx));
        })
        .detach();
        cx.notify();
    }

    /// The answer of `git.switch`.
    pub fn switched(
        &mut self,
        name: &str,
        result: Result<GitSwitchResult, String>,
        cx: &mut Context<Self>,
    ) {
        self.busy = false;
        self.branch.dirty = None;
        match result {
            Ok(r) => match r.outcome {
                GitOutcome::Dirty => self.branch.dirty = Some((name.to_string(), r.files)),
                GitOutcome::Done => self.sync.note = Some(Ok(format!("Switched to {name}."))),
                _ => self.sync.note = Some(Err(r.output)),
            },
            Err(e) => self.sync.note = Some(Err(e)),
        }
        self.reload(cx);
        cx.notify();
    }

    /// Opens the form of a new branch that starts at `from`, or at the head.
    pub fn open_new_branch(
        &mut self,
        from: Option<String>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let input = cx.new(|cx| InputState::new(window, cx).placeholder("New branch name"));
        let events = cx.subscribe_in(&input, window, |this, _, e: &InputEvent, _, cx| match e {
            InputEvent::PressEnter { .. } => this.create_branch(cx),
            InputEvent::Change => cx.notify(),
            _ => {}
        });
        input.update(cx, |i, cx| i.focus(window, cx));
        self.branch.form = Some(NewBranch {
            from,
            input,
            _events: events,
        });
        cx.notify();
    }

    pub fn close_new_branch(&mut self, cx: &mut Context<Self>) {
        self.branch.form = None;
        cx.notify();
    }

    pub fn create_branch(&mut self, cx: &mut Context<Self>) {
        let Some(form) = &self.branch.form else {
            return;
        };
        let name = form.input.read(cx).value().trim().to_string();
        if name.is_empty() || self.busy {
            return;
        }
        let call = self
            .core
            .read(cx)
            .link
            .call::<GitBranchCreate>(&GitBranchCreateParams {
                product: self.product.clone(),
                repo: None,
                name: name.clone(),
                from: form.from.clone(),
            });
        self.busy = true;
        cx.spawn(async move |this, cx| {
            let result = call.await.map_err(|e| e.to_string());
            let _ = this.update(cx, |this, cx| this.branch_created(&name, result, cx));
        })
        .detach();
        cx.notify();
    }

    /// The answer of `git.branch.create`. A refused name keeps the form.
    pub fn branch_created(
        &mut self,
        name: &str,
        result: Result<GitDone, String>,
        cx: &mut Context<Self>,
    ) {
        self.busy = false;
        match result {
            Ok(_) => {
                self.branch.form = None;
                self.branch.selected = Some(name.to_string());
                self.sync.note = Some(Ok(format!("Made the branch {name}. Switch to it here.")));
                self.load_branches(cx);
            }
            Err(e) => self.sync.note = Some(Err(e)),
        }
        cx.notify();
    }

    fn copy_name(&mut self, name: &str, cx: &mut Context<Self>) {
        cx.write_to_clipboard(ClipboardItem::new_string(name.to_string()));
        self.sync.note = Some(Ok(format!("Copied {name}.")));
        cx.notify();
    }

    fn branch_rows(
        &self,
        list: &[GitBranch],
        id: &'static str,
        cx: &mut Context<Self>,
    ) -> Vec<Div> {
        list.iter()
            .enumerate()
            .map(|(i, b)| {
                let name = b.name.clone();
                let title = if b.current {
                    format!("{} (current)", b.name)
                } else {
                    b.name.clone()
                };
                div().pad_x(space(1)).child(
                    ListRow::new((id, i), title)
                        .meta(meta(b))
                        .selected(self.branch.selected.as_deref() == Some(b.name.as_str()))
                        .on_click(cx.listener(move |this, _, _, cx| this.select_branch(&name, cx))),
                )
            })
            .collect()
    }

    fn branch_actions(&self, b: &GitBranch, cx: &mut Context<Self>) -> Div {
        let (switch, from, copy) = (b.name.clone(), b.name.clone(), b.name.clone());
        div()
            .flex()
            .items_center()
            .gap(space(2))
            .pad_x(space(3))
            .py(space(2))
            .border_b_1()
            .border_color(line::border())
            .child(mono(b.name.clone()))
            .child(div().flex_1())
            .child(
                Button::new("git-branch-switch")
                    .primary()
                    .small()
                    .label("Switch")
                    .disabled(b.current || self.busy)
                    .on_click(
                        cx.listener(move |this, _, _, cx| this.switch_to(switch.clone(), cx)),
                    ),
            )
            .child(
                Button::new("git-branch-new")
                    .secondary()
                    .small()
                    .label("New branch from here")
                    .on_click(cx.listener(move |this, _, window, cx| {
                        this.open_new_branch(Some(from.clone()), window, cx)
                    })),
            )
            .child(
                Button::new("git-branch-copy")
                    .ghost()
                    .small()
                    .label("Copy name")
                    .on_click(cx.listener(move |this, _, _, cx| this.copy_name(&copy, cx))),
            )
    }

    fn new_branch_form(&self, cx: &mut Context<Self>) -> Option<Div> {
        let form = self.branch.form.as_ref()?;
        let empty = form.input.read(cx).value().trim().is_empty();
        let from = form.from.clone().unwrap_or_else(|| "the head".into());
        Some(
            div()
                .debug_selector(|| "git-new-branch".into())
                .flex()
                .flex_col()
                .gap(space(2))
                .m(space(3))
                .p(space(3))
                .rounded(radius::lg())
                .border_1()
                .border_color(line::border())
                .bg(surface::raised())
                .child(section(&format!("New branch from {from}"), None))
                .child(
                    div()
                        .flex()
                        .items_center()
                        .gap(space(2))
                        .child(div().flex_1().min_w_0().child(Input::new(&form.input)))
                        .child(
                            Button::new("git-branch-create")
                                .primary()
                                .small()
                                .label("Create")
                                .disabled(empty || self.busy)
                                .on_click(cx.listener(|this, _, _, cx| this.create_branch(cx))),
                        )
                        .child(
                            Button::new("git-branch-cancel")
                                .secondary()
                                .small()
                                .label("Cancel")
                                .on_click(cx.listener(|this, _, _, cx| this.close_new_branch(cx))),
                        ),
                ),
        )
    }

    fn dirty_card(&self) -> Option<Div> {
        let (name, files) = self.branch.dirty.as_ref()?;
        Some(
            div()
                .debug_selector(|| "git-dirty".into())
                .flex()
                .flex_col()
                .gap(space(1))
                .m(space(3))
                .p(space(3))
                .rounded(radius::lg())
                .border_1()
                .border_color(line::border())
                .bg(surface::raised())
                .child(
                    div()
                        .text_size(size::sm())
                        .text_color(Status::Danger.color())
                        .child(format!(
                            "dirty: the tree has changes, so the switch to {name} did not run. \
                             Commit or stash them first."
                        )),
                )
                .children(files.iter().map(|f| {
                    let letter = f.staged.clone().or_else(|| f.unstaged.clone());
                    mono(format!("{} {}", letter.unwrap_or_default(), f.path))
                })),
        )
    }

    pub(super) fn branches_view(&mut self, _: &mut Window, cx: &mut Context<Self>) -> AnyElement {
        let b = match &self.branches {
            Load::Loading => return notice("Reading the branches…", None).into_any_element(),
            Load::Failed(e) => {
                return notice(format!("The branches did not load: {e}"), None).into_any_element();
            }
            Load::Ready(b) => b.clone(),
        };
        let selected = b
            .local
            .iter()
            .chain(&b.remote)
            .find(|x| self.branch.selected.as_deref() == Some(x.name.as_str()))
            .cloned();
        let local = self.branch_rows(&b.local, "git-local-branch", cx);
        let remote = self.branch_rows(&b.remote, "git-remote-branch", cx);
        div()
            .size_full()
            .flex()
            .flex_col()
            .children(self.new_branch_form(cx))
            .children(self.dirty_card())
            .when_some(selected, |el, s| el.child(self.branch_actions(&s, cx)))
            .child(
                div()
                    .id("git-branches")
                    .debug_selector(|| "git-branches".into())
                    .flex_1()
                    .min_h_0()
                    .overflow_y_scroll()
                    .p(space(1))
                    .child(
                        div()
                            .pad_x(space(2))
                            .child(section("Local", Some(b.local.len()))),
                    )
                    .children(local)
                    .child(
                        div()
                            .pad_x(space(2))
                            .child(section("Remote", Some(b.remote.len()))),
                    )
                    .children(remote)
                    .child(
                        div()
                            .pad_x(space(2))
                            .child(section("Worktrees", Some(b.worktrees.len()))),
                    )
                    .children(b.worktrees.iter().map(|w| {
                        let branch = w
                            .branch
                            .clone()
                            .unwrap_or_else(|| format!("detached at {}", w.head));
                        div()
                            .debug_selector(|| "git-worktree".into())
                            .flex()
                            .gap(space(2))
                            .pad_x(space(3))
                            .py(space(1))
                            .child(
                                div()
                                    .text_size(size::sm())
                                    .text_color(text::primary())
                                    .child(branch),
                            )
                            .child(mono(w.path.clone()))
                    })),
            )
            .into_any_element()
    }
}
