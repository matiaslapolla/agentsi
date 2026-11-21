//! The History tab: the commit list, and for the selected commit its files at the left and
//! the diff of one file at the right, in the diff view of Changes.

use super::{DiffView, GitScreen, cut_line, day, diff_body, patch_view, state_tag};
use crate::screens::parts::{Load, mono, notice};
use agentsi_proto::git_api::{GitShow, GitShowFile, GitShowParams, GitShowResult};
use agentsi_ui::components::ListRow;
use agentsi_ui::style::StyleExt as _;
use agentsi_ui::tokens::{control, line, space};
use gpui_kit::component::diff::DiffState;
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;

#[derive(Default)]
pub(super) struct History {
    /// The id of the selected commit.
    commit: Option<String>,
    show: Option<Load<GitShowResult>>,
    /// The selected file of the commit.
    pub(super) file: usize,
    pub(super) view: Option<DiffView>,
    diff: Option<Entity<DiffState>>,
}

/// The part of a commit patch that belongs to one file: from its `diff --git` line to the
/// next one. `None` when the patch has no such part, for example past the cut.
pub fn file_patch(patch: &str, file: &GitShowFile) -> Option<String> {
    let head = format!(
        "diff --git a/{} b/{}",
        file.orig_path.as_deref().unwrap_or(&file.path),
        file.path
    );
    let start = patch
        .match_indices("diff --git ")
        .map(|(i, _)| i)
        .filter(|&i| i == 0 || patch.as_bytes()[i - 1] == b'\n')
        .find(|&i| patch[i..].lines().next() == Some(head.as_str()))?;
    let rest = &patch[start + 1..];
    let end = rest
        .find("\ndiff --git ")
        .map_or(patch.len(), |i| start + 1 + i + 1);
    Some(patch[start..end].to_string())
}

impl GitScreen {
    /// Reads the files and the patch of a commit.
    pub fn select_commit(&mut self, id: String, cx: &mut Context<Self>) {
        if self.history.commit.as_deref() == Some(id.as_str()) {
            return;
        }
        self.history.commit = Some(id.clone());
        self.history.show = Some(Load::Loading);
        self.history.view = None;
        self.history.file = 0;
        let call = self.core.read(cx).link.call::<GitShow>(&GitShowParams {
            product: self.product.clone(),
            repo: None,
            commit: id.clone(),
        });
        cx.spawn(async move |this, cx| {
            let result = call.await.map_err(|e| e.to_string());
            let _ = this.update(cx, |this, cx| this.show_loaded(&id, result, cx));
        })
        .detach();
        cx.notify();
    }

    /// The answer of `git.show`. It counts only while the commit is still the selected one.
    pub fn show_loaded(
        &mut self,
        id: &str,
        result: Result<GitShowResult, String>,
        cx: &mut Context<Self>,
    ) {
        if self.history.commit.as_deref() != Some(id) {
            return;
        }
        let mut show = Load::Loading;
        show.settle(result);
        self.history.show = Some(show);
        self.select_commit_file(0, cx);
    }

    pub fn select_commit_file(&mut self, i: usize, cx: &mut Context<Self>) {
        self.history.file = i;
        let Some(Load::Ready(show)) = &self.history.show else {
            self.history.view = None;
            cx.notify();
            return;
        };
        self.history.view = show.files.get(i).map(|f| match file_patch(&show.patch, f) {
            Some(patch) => patch_view(&patch, show.truncated, &mut self.history.diff, cx),
            None if show.truncated => DiffView::Text(vec![
                "The patch of this file is past the 512 KiB cut.".into(),
            ]),
            None => DiffView::Text(vec!["This file has no text change to show.".into()]),
        });
        cx.notify();
    }

    fn commit_list(&self, cx: &mut Context<Self>) -> AnyElement {
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
                .debug_selector(|| "git-history".into())
                .size_full()
                .overflow_y_scroll()
                .p(space(1))
                .children(commits.iter().enumerate().map(|(i, c)| {
                    let id = c.id.clone();
                    div().pad_x(space(1)).child(
                        ListRow::new(("git-commit-row", i), c.subject.clone())
                            .meta(format!(
                                "{} · {} · {}",
                                c.id.chars().take(8).collect::<String>(),
                                c.author,
                                day(c.time)
                            ))
                            .selected(self.history.commit.as_deref() == Some(c.id.as_str()))
                            .on_click(cx.listener(move |this, _, _, cx| {
                                this.select_commit(id.clone(), cx)
                            })),
                    )
                }))
                .into_any_element(),
        }
    }

    fn commit_files(&self, cx: &mut Context<Self>) -> Option<AnyElement> {
        Some(match self.history.show.as_ref()? {
            Load::Loading => notice("Reading the commit…", None).into_any_element(),
            Load::Failed(e) => {
                notice(format!("The commit did not load: {e}"), None).into_any_element()
            }
            Load::Ready(show) if show.files.is_empty() => {
                notice("This commit changes no file.", None).into_any_element()
            }
            Load::Ready(show) => div()
                .id("git-commit-files")
                .debug_selector(|| "git-commit-files".into())
                .size_full()
                .overflow_y_scroll()
                .p(space(1))
                .children(show.files.iter().enumerate().map(|(i, f)| {
                    let row = ListRow::new(("git-commit-file", i), f.path.clone())
                        .selected(self.history.file == i)
                        .trailing(state_tag(&f.letter))
                        .on_click(
                            cx.listener(move |this, _, _, cx| this.select_commit_file(i, cx)),
                        );
                    let row = match &f.orig_path {
                        Some(o) => row.meta(format!("from {o}")),
                        None => row,
                    };
                    div().pad_x(space(1)).child(row)
                }))
                .into_any_element(),
        })
    }

    fn commit_diff(&self) -> Div {
        let pane = div().flex_1().min_w_0().flex().flex_col();
        let (Some(Load::Ready(show)), Some(view)) = (&self.history.show, &self.history.view) else {
            let text = match self.history.commit {
                None => "Select a commit to see its files and its diff.",
                Some(_) => "No file to show.",
            };
            return pane.child(notice(text, None));
        };
        let path = show
            .files
            .get(self.history.file)
            .map(|f| f.path.clone())
            .unwrap_or_default();
        pane.child(
            div()
                .flex()
                .items_center()
                .h(control::row())
                .flex_none()
                .pad_x(space(3))
                .border_b_1()
                .border_color(line::border())
                .child(mono(format!(
                    "{} · {}",
                    show.commit.id.chars().take(8).collect::<String>(),
                    path
                ))),
        )
        .when(show.truncated, |el| el.child(cut_line("git-commit-cut")))
        .child(
            div()
                .debug_selector(|| "git-commit-diff".into())
                .flex_1()
                .min_h_0()
                .child(diff_body(
                    "git-commit-text",
                    view,
                    self.history.diff.as_ref(),
                )),
        )
    }

    pub(super) fn history_view(&self, cx: &mut Context<Self>) -> AnyElement {
        let files = self.commit_files(cx);
        div()
            .size_full()
            .flex()
            .child(
                div()
                    .w(control::SIDEBAR_WIDTH)
                    .flex_none()
                    .flex()
                    .flex_col()
                    .border_r_1()
                    .border_color(line::border())
                    .child(div().flex_1().min_h_0().child(self.commit_list(cx)))
                    .when_some(files, |el, files| {
                        el.child(
                            div()
                                .flex_1()
                                .min_h_0()
                                .border_t_1()
                                .border_color(line::border())
                                .child(files),
                        )
                    }),
            )
            .child(self.commit_diff())
            .into_any_element()
    }
}

#[cfg(test)]
mod tests {
    use super::file_patch;
    use agentsi_proto::git_api::GitShowFile;

    fn file(path: &str, orig: Option<&str>) -> GitShowFile {
        GitShowFile {
            path: path.into(),
            orig_path: orig.map(String::from),
            letter: "M".into(),
        }
    }

    #[test]
    fn the_patch_of_one_file_is_cut_out_of_the_commit_patch() {
        let patch = "diff --git a/a.txt b/a.txt\n--- a/a.txt\n+++ b/a.txt\n@@ -1 +1 @@\n-a\n+b\n\
            diff --git a/old b c b/new\nsimilarity index 90%\nrename from old b c\nrename to new\n\
            diff --git a/z.txt b/z.txt\n--- a/z.txt\n+++ b/z.txt\n@@ -1 +1 @@\n-y\n+z\n";
        assert_eq!(
            file_patch(patch, &file("a.txt", None)).unwrap(),
            "diff --git a/a.txt b/a.txt\n--- a/a.txt\n+++ b/a.txt\n@@ -1 +1 @@\n-a\n+b\n"
        );
        assert!(
            file_patch(patch, &file("new", Some("old b c")))
                .unwrap()
                .ends_with("rename to new\n")
        );
        assert!(
            file_patch(patch, &file("z.txt", None))
                .unwrap()
                .ends_with("+z\n")
        );
        assert_eq!(file_patch(patch, &file("missing", None)), None);
    }
}
