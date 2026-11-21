//! The git summaries that the sidebar and the tabs show, one cache for each core.
//!
//! The cache reads `git.summary` of each product of its core and of each terminal folder.
//! It reads a source when the source first shows, again on `git.changed` of its product, on
//! `lagged` and when the host answers again. A terminal whose folder changes is a new source.
//! Nothing else reads: an idle app sends no call and draws no frame.
//! Spec: `docs/specs/agentsi-desktop-git-more.md`.

use std::collections::{BTreeMap, BTreeSet};

use agentsi_proto::Event as _;
use agentsi_proto::events::{GitChanged, Lagged};
use agentsi_proto::git_api::{GitSummary, GitSummaryInfo, GitSummaryParams};
use gpui_kit::*;

use crate::model::{CoreEvent, CoreState, DeskNotice, GitNotice};

/// What a summary is of.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub enum Source {
    Product(String),
    /// The folder of a terminal.
    Folder(String),
}

/// The summary of one source changed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MarkChanged(pub Source);

#[derive(Default)]
struct Entry {
    /// `None` before the first answer, and for a folder outside a repo.
    summary: Option<GitSummaryInfo>,
    reading: bool,
    /// A reason to read came while a read ran: read once more after it.
    again: bool,
}

pub struct GitMarks {
    core: WeakEntity<CoreState>,
    entries: BTreeMap<Source, Entry>,
    _subscriptions: Vec<Subscription>,
}

impl EventEmitter<MarkChanged> for GitMarks {}

impl GitMarks {
    /// Gives `core` a cache of its own, and reads the sources that it has now.
    pub fn install(core: &Entity<CoreState>, cx: &mut App) -> Entity<GitMarks> {
        let marks = cx.new(|cx| {
            let subscriptions = vec![
                cx.observe(core, |this: &mut GitMarks, core, cx| {
                    this.reconcile(&core, cx)
                }),
                cx.subscribe(core, |this, _, e: &GitNotice, cx| {
                    if let Some(p) = e.0.parse::<GitChanged>() {
                        this.changed(&p.product, cx);
                    }
                }),
                cx.subscribe(core, |this, _, e: &DeskNotice, cx| {
                    if e.0.method == Lagged::NAME {
                        this.read_all(cx);
                    }
                }),
                cx.subscribe(core, |this, _, e: &CoreEvent, cx| {
                    if *e == CoreEvent::Up {
                        this.read_all(cx);
                    }
                }),
            ];
            GitMarks {
                core: core.downgrade(),
                entries: BTreeMap::new(),
                _subscriptions: subscriptions,
            }
        });
        core.update(cx, |s, _| s.git = Some(marks.clone()));
        marks.update(cx, |m, cx| m.reconcile(core, cx));
        marks
    }

    pub fn get(&self, source: &Source) -> Option<&GitSummaryInfo> {
        self.entries.get(source)?.summary.as_ref()
    }

    /// The sources that the core has now: each product and each terminal folder.
    fn wanted(core: &CoreState) -> BTreeSet<Source> {
        let products = core.products.iter().map(|p| Source::Product(p.id.clone()));
        let folders = core
            .terminals
            .values()
            .filter(|t| !t.cwd.is_empty())
            .map(|t| Source::Folder(t.cwd.clone()));
        products.chain(folders).collect()
    }

    /// Reads each new source and drops each source that went.
    fn reconcile(&mut self, core: &Entity<CoreState>, cx: &mut Context<Self>) {
        let wanted = Self::wanted(core.read(cx));
        self.entries.retain(|s, _| wanted.contains(s));
        for source in wanted {
            if !self.entries.contains_key(&source) {
                self.read(source, cx);
            }
        }
    }

    fn read_all(&mut self, cx: &mut Context<Self>) {
        let all: Vec<Source> = self.entries.keys().cloned().collect();
        for source in all {
            self.read(source, cx);
        }
    }

    /// `git.changed {product}`: the product, the folders of its terminals, and each folder in
    /// the same repo as the product read again.
    fn changed(&mut self, product: &str, cx: &mut Context<Self>) {
        let Some(core) = self.core.upgrade() else {
            return;
        };
        let own = Source::Product(product.to_string());
        let repo = self.get(&own).map(|s| s.repo.clone());
        let mut sources: BTreeSet<Source> = core
            .read(cx)
            .terminals
            .values()
            .filter(|t| t.product.as_deref() == Some(product) && !t.cwd.is_empty())
            .map(|t| Source::Folder(t.cwd.clone()))
            .collect();
        sources.extend(
            self.entries
                .iter()
                .filter(|(_, e)| {
                    repo.is_some() && e.summary.as_ref().map(|s| &s.repo) == repo.as_ref()
                })
                .map(|(s, _)| s.clone()),
        );
        sources.insert(own);
        for source in sources {
            if self.entries.contains_key(&source) {
                self.read(source, cx);
            }
        }
    }

    fn read(&mut self, source: Source, cx: &mut Context<Self>) {
        let Some(core) = self.core.upgrade() else {
            return;
        };
        let entry = self.entries.entry(source.clone()).or_default();
        if entry.reading {
            entry.again = true;
            return;
        }
        entry.reading = true;
        let params = match &source {
            Source::Product(id) => GitSummaryParams {
                product: Some(id.clone()),
                cwd: None,
            },
            Source::Folder(cwd) => GitSummaryParams {
                product: None,
                cwd: Some(cwd.clone()),
            },
        };
        let call = core.read(cx).link.call::<GitSummary>(&params);
        cx.spawn(async move |this, cx| {
            let result = call.await.map(|r| r.summary).map_err(|e| e.to_string());
            let _ = this.update(cx, |this, cx| this.loaded(source, result, cx));
        })
        .detach();
    }

    /// The answer of one read. An error keeps the summary that the cache has.
    pub fn loaded(
        &mut self,
        source: Source,
        result: Result<Option<GitSummaryInfo>, String>,
        cx: &mut Context<Self>,
    ) {
        let Some(entry) = self.entries.get_mut(&source) else {
            return;
        };
        entry.reading = false;
        let again = std::mem::take(&mut entry.again);
        if let Ok(summary) = result
            && entry.summary != summary
        {
            entry.summary = summary;
            cx.emit(MarkChanged(source.clone()));
            cx.notify();
        }
        if again {
            self.read(source, cx);
        }
    }
}

/// The meta text of a product row: `branch ·N ↑a ↓b`. A clean tree shows only the branch, and
/// a count of 0 does not show.
pub fn meta(s: &GitSummaryInfo) -> String {
    let mut out = head(s);
    if s.changed > 0 {
        out.push_str(&format!(" ·{}", s.changed));
    }
    if let Some(a) = s.ahead.filter(|a| *a > 0) {
        out.push_str(&format!(" ↑{a}"));
    }
    if let Some(b) = s.behind.filter(|b| *b > 0) {
        out.push_str(&format!(" ↓{b}"));
    }
    out
}

/// The tooltip line of a terminal tab, for example `main → origin/main · 3 changed · ↑1`.
pub fn tooltip(s: &GitSummaryInfo) -> String {
    let mut out = head(s);
    match &s.upstream {
        Some(up) => out.push_str(&format!(" → {up}")),
        None if s.branch.is_some() => out.push_str(" · no upstream"),
        None => {}
    }
    match s.changed {
        0 => out.push_str(" · clean"),
        n => out.push_str(&format!(" · {n} changed")),
    }
    let ahead = s.ahead.filter(|a| *a > 0).map(|a| format!("↑{a}"));
    let behind = s.behind.filter(|b| *b > 0).map(|b| format!("↓{b}"));
    let sync: Vec<String> = ahead.into_iter().chain(behind).collect();
    if !sync.is_empty() {
        out.push_str(&format!(" · {}", sync.join(" ")));
    }
    out
}

/// The branch, or the short head of a detached head.
fn head(s: &GitSummaryInfo) -> String {
    match (&s.branch, &s.head) {
        (Some(b), _) => b.clone(),
        (None, Some(h)) => format!("detached {h}"),
        (None, None) => "detached".into(),
    }
}

/// The summary of the folder of terminal `id`.
pub fn of_terminal<'a>(core: &CoreState, id: &str, cx: &'a App) -> Option<&'a GitSummaryInfo> {
    let cwd = core.terminals.get(id)?.cwd.clone();
    core.git.as_ref()?.read(cx).get(&Source::Folder(cwd))
}

/// The summary of product `id`.
pub fn of_product<'a>(core: &CoreState, id: &str, cx: &'a App) -> Option<&'a GitSummaryInfo> {
    core.git
        .as_ref()?
        .read(cx)
        .get(&Source::Product(id.to_string()))
}

#[cfg(test)]
mod tests {
    use std::os::unix::net::UnixStream;
    use std::sync::Arc;

    use agentsi_proto::desk_api::ProductInfo;
    use agentsi_proto::events::{GitChangedParams, LaggedParams};
    use agentsi_proto::rpc::Notification;
    use agentsi_proto::terminal_api::TerminalInfo;
    use gpui::TestAppContext;
    use serde_json::{Value, json};

    use agentsi_proto::events::{GitChanged, Lagged};
    use agentsi_proto::git_api::GitSummaryInfo;
    use gpui_kit::{AppContext as _, Entity};

    use super::{GitMarks, MarkChanged, Source, meta, tooltip};
    use crate::link::Link;
    use crate::model::CoreState;
    use crate::model::two_cores::received;

    fn summary(branch: Option<&str>, changed: u32, sync: Option<(u32, u32)>) -> GitSummaryInfo {
        GitSummaryInfo {
            repo: "/w".into(),
            branch: branch.map(String::from),
            head: Some("abc1234".into()),
            upstream: sync.map(|_| "origin/main".into()),
            ahead: sync.map(|s| s.0),
            behind: sync.map(|s| s.1),
            changed,
        }
    }

    #[test]
    fn the_meta_text_shows_the_branch_the_changes_and_ahead_and_behind() {
        let main = Some("main");
        assert_eq!(meta(&summary(main, 0, Some((0, 0)))), "main");
        assert_eq!(meta(&summary(main, 3, Some((0, 0)))), "main ·3");
        assert_eq!(meta(&summary(main, 0, Some((2, 0)))), "main ↑2");
        assert_eq!(meta(&summary(main, 0, Some((0, 4)))), "main ↓4");
        assert_eq!(meta(&summary(main, 1, Some((2, 4)))), "main ·1 ↑2 ↓4");
        assert_eq!(meta(&summary(main, 5, None)), "main ·5");
        assert_eq!(meta(&summary(None, 0, None)), "detached abc1234");
        let empty = GitSummaryInfo {
            head: None,
            ..summary(None, 0, None)
        };
        assert_eq!(meta(&empty), "detached");
    }

    #[test]
    fn the_tooltip_names_the_upstream_and_says_clean() {
        let main = Some("main");
        assert_eq!(
            tooltip(&summary(main, 0, Some((0, 0)))),
            "main → origin/main · clean"
        );
        assert_eq!(
            tooltip(&summary(main, 2, Some((1, 3)))),
            "main → origin/main · 2 changed · ↑1 ↓3"
        );
        assert_eq!(
            tooltip(&summary(main, 0, None)),
            "main · no upstream · clean"
        );
        assert_eq!(
            tooltip(&summary(None, 1, None)),
            "detached abc1234 · 1 changed"
        );
    }

    fn terminal(id: &str, product: &str, cwd: &str) -> TerminalInfo {
        serde_json::from_value(json!({
            "id": id, "argv": ["sh"], "cwd": cwd, "product": product, "pid": 1,
            "cols": 80, "rows": 24, "title": "", "state": null, "running": true, "code": null,
        }))
        .unwrap()
    }

    fn product(id: &str) -> ProductInfo {
        ProductInfo {
            id: id.into(),
            name: id.into(),
            path: Some(format!("/w/{id}")),
            archived: false,
        }
    }

    /// The params of each `git.summary` that the core got since the last read.
    fn reads(socket: &mut UnixStream) -> Vec<Value> {
        received(socket)
            .into_iter()
            .filter(|r| r["method"] == "git.summary")
            .map(|r| r["params"].clone())
            .collect()
    }

    fn setup(cx: &mut TestAppContext) -> (Entity<CoreState>, Entity<GitMarks>, UnixStream) {
        let (end, socket) = UnixStream::pair().unwrap();
        socket.set_nonblocking(true).unwrap();
        let core = cx.new(|_| CoreState::new(Arc::new(Link::for_tests(end))));
        let marks = cx.update(|cx| GitMarks::install(&core, cx));
        (core, marks, socket)
    }

    #[gpui::test]
    fn each_new_source_reads_once_and_an_idle_core_reads_nothing(cx: &mut TestAppContext) {
        let (core, marks, mut socket) = setup(cx);
        core.update(cx, |s, cx| {
            s.set_products(vec![product("a")], cx);
            s.put(terminal("t1", "a", "/w/a/sub"));
            s.put(terminal("t2", "a", "/w/a/sub"));
            cx.notify();
        });
        cx.run_until_parked();
        assert_eq!(
            reads(&mut socket),
            [json!({"product": "a"}), json!({"cwd": "/w/a/sub"})]
        );
        // A change of the core with no new source, and a quiet time, read nothing.
        core.update(cx, |s, cx| {
            s.terminals.get_mut("t1").unwrap().title = "vim".into();
            cx.notify();
        });
        cx.run_until_parked();
        assert_eq!(reads(&mut socket), Vec::<Value>::new());
        // The folder of a terminal changes: the new folder reads, the old one goes.
        core.update(cx, |s, cx| {
            for t in s.terminals.values_mut() {
                t.cwd = "/x".into();
            }
            cx.notify();
        });
        cx.run_until_parked();
        assert_eq!(reads(&mut socket), [json!({"cwd": "/x"})]);
        marks.read_with(cx, |m, _| {
            assert!(!m.entries.contains_key(&Source::Folder("/w/a/sub".into())));
        });
    }

    #[gpui::test]
    fn git_changed_reads_the_product_and_its_terminal_folders_again(cx: &mut TestAppContext) {
        let (core, marks, mut socket) = setup(cx);
        core.update(cx, |s, cx| {
            s.set_products(vec![product("a"), product("b")], cx);
            s.put(terminal("t1", "a", "/w/a/sub"));
            s.put(terminal("t2", "b", "/w/b"));
            cx.notify();
        });
        cx.run_until_parked();
        assert_eq!(reads(&mut socket).len(), 4);
        let seen = Arc::new(std::sync::Mutex::new(Vec::new()));
        let s = seen.clone();
        cx.update(|cx| {
            cx.subscribe(&marks, move |_, e: &MarkChanged, _| {
                s.lock().unwrap().push(e.0.clone())
            })
            .detach()
        });
        let a = Source::Product("a".into());
        marks.update(cx, |m, cx| {
            for source in m.entries.keys().cloned().collect::<Vec<_>>() {
                m.loaded(source, Ok(Some(summary(Some("main"), 0, None))), cx);
            }
        });
        assert_eq!(seen.lock().unwrap().len(), 4);
        // The same answer again changes nothing.
        marks.update(cx, |m, cx| {
            m.read(a.clone(), cx);
            m.loaded(a.clone(), Ok(Some(summary(Some("main"), 0, None))), cx);
        });
        assert_eq!(seen.lock().unwrap().len(), 4);
        reads(&mut socket);

        let changed = |product: &str| {
            crate::model::GitNotice(Notification::of::<GitChanged>(&GitChangedParams {
                product: product.into(),
            }))
        };
        // The entries share the repo `/w`: each one reads again.
        core.update(cx, |_, cx| cx.emit(changed("a")));
        cx.run_until_parked();
        let got = reads(&mut socket);
        assert_eq!(got.len(), 4, "{got:?}");
        marks.update(cx, |m, cx| {
            for source in m.entries.keys().cloned().collect::<Vec<_>>() {
                m.loaded(source, Ok(None), cx);
            }
        });
        // Out of a repo now: only the product and the folder of its terminal read.
        core.update(cx, |_, cx| cx.emit(changed("a")));
        cx.run_until_parked();
        assert_eq!(
            reads(&mut socket),
            [json!({"product": "a"}), json!({"cwd": "/w/a/sub"})]
        );
        // A second reason while a read runs reads once more after the answer.
        core.update(cx, |_, cx| cx.emit(changed("a")));
        cx.run_until_parked();
        assert_eq!(reads(&mut socket), Vec::<Value>::new());
        marks.update(cx, |m, cx| m.loaded(a.clone(), Ok(None), cx));
        assert_eq!(reads(&mut socket), [json!({"product": "a"})]);
        assert_eq!(marks.read_with(cx, |m, _| m.get(&a).cloned()), None);
        let lagged = Notification::of::<Lagged>(&LaggedParams { missed: 3 });
        // The folder still had a second read to make.
        for _ in 0..2 {
            marks.update(cx, |m, cx| {
                for source in m.entries.keys().cloned().collect::<Vec<_>>() {
                    m.loaded(source, Ok(None), cx);
                }
            });
        }
        reads(&mut socket);
        core.update(cx, |_, cx| cx.emit(crate::model::DeskNotice(lagged)));
        cx.run_until_parked();
        assert_eq!(reads(&mut socket).len(), 4);
    }
}
