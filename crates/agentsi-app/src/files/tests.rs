use std::os::unix::net::UnixStream;
use std::sync::Arc;

use agentsi_proto::desk_api::ProductInfo;
use agentsi_proto::rpc::{Notification, V2};
use gpui::{TestAppContext, VisualTestContext};
use serde_json::{Value, json};

// No glob import: `super` brings `gpui::test` in as `test`, and `#[test]` then loops.
use super::{
    FileEntry, FileGit, FileKind, FileTree, FilesListResult, FilesNotice, NameFor, NewKind,
    ProductRef, git_color,
};
use crate::link::Link;
use crate::link::tests::FakeCore;
use crate::model::CoreState;
use agentsi_ui::tokens::{status, text};
use gpui_kit::{AppContext as _, Entity, Focusable as _, SharedString};
use std::path::PathBuf;

struct Ui<'a> {
    files: Entity<FileTree>,
    core: Entity<CoreState>,
    fake: FakeCore,
    cx: &'a mut VisualTestContext,
}

fn mine() -> ProductRef {
    ProductRef {
        host: None,
        id: "a".into(),
    }
}

fn entry(name: &str, kind: FileKind, git: Option<FileGit>) -> FileEntry {
    FileEntry {
        name: name.into(),
        kind,
        size: 0,
        modified: None,
        git,
    }
}

fn listing(path: &str, entries: Vec<FileEntry>) -> FilesListResult {
    FilesListResult {
        path: path.into(),
        entries,
        truncated: false,
    }
}

/// A file tree of product `a` at `/w/a`, shown, with its first read taken from the fake core.
fn ui(cx: &mut TestAppContext) -> Ui<'_> {
    cx.update(agentsi_ui::init);
    let (end, other) = UnixStream::pair().unwrap();
    let core = cx.new(|_| CoreState::new(Arc::new(Link::for_tests(end))));
    core.update(cx, |s, cx| {
        s.set_products(
            vec![ProductInfo {
                id: "a".into(),
                name: "A".into(),
                path: Some("/w/a".into()),
                archived: false,
            }],
            cx,
        );
        s.active = Some(mine());
        cx.notify();
    });
    let (files, cx) = {
        let core = core.clone();
        cx.add_window_view(|window, cx| FileTree::new(core, window, cx))
    };
    let mut ui = Ui {
        files,
        core,
        fake: FakeCore(other),
        cx,
    };
    ui.files.update(ui.cx, |f, cx| f.set_shown(true, cx));
    assert_eq!(ui.list_request(), "");
    ui
}

impl Ui<'_> {
    fn draw(&mut self) {
        self.cx.run_until_parked();
        self.cx.update(|window, cx| window.draw(cx).clear(cx));
    }

    /// The path of the next request, which must be a `files.list`.
    fn list_request(&mut self) -> String {
        let r = self.fake.request();
        assert_eq!(r["method"], "files.list", "{r}");
        assert_eq!(r["params"]["product"], "a");
        r["params"]["path"].as_str().unwrap().to_string()
    }

    fn request(&mut self) -> Value {
        self.fake.request()
    }

    fn no_request(&mut self) -> bool {
        self.fake.0.set_nonblocking(true).unwrap();
        let none = agentsi_proto::frame::read_frame(&mut self.fake.0).is_err();
        self.fake.0.set_nonblocking(false).unwrap();
        none
    }

    fn load(&mut self, path: &str, entries: Vec<FileEntry>) {
        let list = listing(path, entries);
        let path = path.to_string();
        self.files
            .update(self.cx, |f, cx| f.loaded(&mine(), path, Ok(list), cx));
        self.draw();
    }

    fn names(&mut self) -> Vec<String> {
        self.files.read_with(self.cx, |f, cx| f.shown_names(cx))
    }

    /// The root has `docs` (folder, changed), `target` (ignored) and `README.md`.
    fn root(&mut self) {
        self.load(
            "",
            vec![
                entry("docs", FileKind::Folder, Some(FileGit::Changed)),
                entry("target", FileKind::Folder, Some(FileGit::Ignored)),
                entry("README.md", FileKind::File, None),
            ],
        );
    }

    fn open(&mut self, path: &str) {
        let id = SharedString::from(path.to_string());
        self.files
            .update(self.cx, |f, cx| f.set_folder_open(id.as_ref(), true, cx));
        self.draw();
    }

    fn type_name(&mut self, text: &str) {
        let input = self
            .files
            .read_with(self.cx, |f, _| f.naming_input())
            .expect("the name field shows");
        let text = text.to_string();
        self.cx
            .update(|window, cx| input.update(cx, |i, cx| i.set_value(text, window, cx)));
    }
}

#[gpui::test]
fn the_tree_reads_a_folder_when_it_opens_and_shows_reading_until_then(cx: &mut TestAppContext) {
    let mut ui = ui(cx);
    assert_eq!(ui.names(), ["Reading…"]);
    ui.root();
    assert_eq!(ui.names(), ["docs", "target", "README.md"]);
    ui.open("docs");
    assert_eq!(ui.list_request(), "docs");
    assert_eq!(ui.names(), ["docs", "  Reading…", "target", "README.md"]);
    ui.load(
        "docs",
        vec![entry("a.md", FileKind::File, Some(FileGit::New))],
    );
    assert_eq!(ui.names(), ["docs", "  a.md", "target", "README.md"]);
    // A folder that is read once is not read again when it opens again.
    ui.files
        .update(ui.cx, |f, cx| f.set_folder_open("docs", false, cx));
    ui.open("docs");
    assert!(ui.no_request());
}

#[gpui::test]
fn files_changed_of_a_folder_that_the_tree_read_reads_it_again(cx: &mut TestAppContext) {
    let mut ui = ui(cx);
    ui.root();
    let changed = |path: &str, product: &str| Notification {
        jsonrpc: V2,
        method: "files.changed".into(),
        params: json!({"product": product, "path": path}),
    };
    let core = ui.core.clone();
    core.update(ui.cx, |_, cx| cx.emit(FilesNotice(changed("", "a"))));
    assert_eq!(ui.list_request(), "");
    // A folder that the tree did not read, and another product, read nothing.
    core.update(ui.cx, |_, cx| cx.emit(FilesNotice(changed("src", "a"))));
    core.update(ui.cx, |_, cx| cx.emit(FilesNotice(changed("", "b"))));
    assert!(ui.no_request());
}

#[gpui::test]
fn new_folder_shows_an_inline_name_field_and_enter_creates_and_shows_the_folders(
    cx: &mut TestAppContext,
) {
    let mut ui = ui(cx);
    ui.root();
    ui.files.update_in(ui.cx, |f, window, cx| {
        f.start_naming(String::new(), NameFor::New(NewKind::Folder), window, cx)
    });
    ui.draw();
    assert_eq!(ui.names(), ["[name]", "docs", "target", "README.md"]);
    assert!(ui.cx.debug_bounds("file-naming").is_some());
    ui.type_name("docs/tmp");
    ui.cx.simulate_keystrokes("enter");
    ui.draw();
    let r = ui.request();
    assert_eq!(r["method"], "files.create");
    assert_eq!(
        r["params"],
        json!({"product": "a", "path": "docs/tmp", "kind": "folder"})
    );
    ui.files.update(ui.cx, |f, cx| {
        f.named(&mine(), None, "docs/tmp".into(), Ok(()), cx)
    });
    ui.draw();
    // The folder and each folder on the way read again, and they open.
    assert_eq!(ui.list_request(), "");
    assert_eq!(ui.list_request(), "docs");
    ui.root();
    ui.load(
        "docs",
        vec![entry("tmp", FileKind::Folder, Some(FileGit::New))],
    );
    assert_eq!(ui.names(), ["docs", "  tmp", "target", "README.md"]);
    assert!(!ui.files.read_with(ui.cx, |f, _| f.is_naming()));
}

#[gpui::test]
fn escape_cancels_the_name_field_and_a_failed_create_keeps_it_with_the_reason(
    cx: &mut TestAppContext,
) {
    let mut ui = ui(cx);
    ui.root();
    ui.files.update_in(ui.cx, |f, window, cx| {
        f.start_naming("docs".into(), NameFor::New(NewKind::File), window, cx)
    });
    ui.draw();
    assert_eq!(ui.list_request(), "docs");
    ui.load("docs", vec![]);
    assert_eq!(ui.names(), ["docs", "  [name]", "target", "README.md"]);
    ui.cx.simulate_keystrokes("escape");
    ui.draw();
    assert!(!ui.files.read_with(ui.cx, |f, _| f.is_naming()));
    assert_eq!(ui.names(), ["docs", "  Empty", "target", "README.md"]);

    ui.files.update_in(ui.cx, |f, window, cx| {
        f.start_naming("docs".into(), NameFor::New(NewKind::File), window, cx)
    });
    ui.draw();
    ui.files.update(ui.cx, |f, cx| {
        f.named(
            &mine(),
            None,
            "docs/x".into(),
            Err("docs/x exists.".into()),
            cx,
        )
    });
    ui.draw();
    assert!(ui.files.read_with(ui.cx, |f, _| f.is_naming()));
    assert!(ui.files.read_with(ui.cx, |f, _| f.note.is_some()));
}

#[gpui::test]
fn rename_shows_the_name_field_in_place_with_the_old_name(cx: &mut TestAppContext) {
    let mut ui = ui(cx);
    ui.root();
    ui.files.update_in(ui.cx, |f, window, cx| {
        f.start_naming(
            String::new(),
            NameFor::Rename("README.md".into()),
            window,
            cx,
        )
    });
    ui.draw();
    assert_eq!(ui.names(), ["docs", "target", "[name]"]);
    let input = ui.files.read_with(ui.cx, |f, _| f.naming_input()).unwrap();
    assert_eq!(
        input.read_with(ui.cx, |i, _| i.value().to_string()),
        "README.md"
    );
    ui.type_name("READ.md");
    ui.cx.simulate_keystrokes("enter");
    ui.draw();
    let r = ui.request();
    assert_eq!(r["method"], "files.rename");
    assert_eq!(
        r["params"],
        json!({"product": "a", "from": "README.md", "to": "READ.md"})
    );
}

#[gpui::test]
fn the_filter_keeps_the_matches_of_the_read_names_and_their_folders(cx: &mut TestAppContext) {
    let mut ui = ui(cx);
    ui.root();
    ui.open("docs");
    assert_eq!(ui.list_request(), "docs");
    ui.load(
        "docs",
        vec![
            entry("guide.md", FileKind::File, None),
            entry("notes.txt", FileKind::File, None),
        ],
    );
    ui.files
        .update(ui.cx, |f, cx| f.set_folder_open("docs", false, cx));
    let filter = ui.files.read_with(ui.cx, |f, _| f.filter_input());
    ui.cx.update(|window, cx| {
        let focus = filter.focus_handle(cx);
        window.focus(&focus, cx)
    });
    ui.draw();
    ui.cx.simulate_input("GUIDE");
    ui.draw();
    assert_eq!(ui.names(), ["docs", "  guide.md"]);
    ui.cx.simulate_keystrokes("ctrl-a backspace");
    ui.cx.simulate_input("read");
    ui.draw();
    assert_eq!(ui.names(), ["README.md"]);
    ui.cx.simulate_keystrokes("ctrl-a backspace");
    ui.draw();
    assert_eq!(ui.names(), ["docs", "target", "README.md"]);
}

#[gpui::test]
fn the_setting_hides_the_ignored_entries_and_a_row_has_the_color_of_its_state(
    cx: &mut TestAppContext,
) {
    let mut ui = ui(cx);
    ui.root();
    let rows = ui.files.read_with(ui.cx, |f, _| f.rows.clone());
    let git = |name: &str| rows.get(&SharedString::from(name.to_string())).unwrap().git;
    assert_eq!(git_color(git("target")), text::tertiary());
    assert_eq!(git_color(git("docs")), status::warn());
    assert_eq!(git_color(Some(FileGit::New)), status::ok());
    ui.files
        .update(ui.cx, |f, cx| f.set_show_ignored(false, cx));
    ui.draw();
    assert_eq!(ui.names(), ["docs", "README.md"]);
    ui.files.update(ui.cx, |f, cx| f.set_show_ignored(true, cx));
    ui.draw();
    assert_eq!(ui.names(), ["docs", "target", "README.md"]);
}

#[gpui::test]
fn the_selection_is_the_target_of_the_palette_and_the_folder_of_the_product_without_one(
    cx: &mut TestAppContext,
) {
    let mut ui = ui(cx);
    ui.root();
    let selected = |ui: &mut Ui| ui.files.read_with(ui.cx, |f, cx| f.selected(cx).unwrap());
    let none = selected(&mut ui);
    assert_eq!((none.path.as_str(), none.folder), ("", true));
    let tree = ui.files.read_with(ui.cx, |f, _| f.tree_state());
    tree.update(ui.cx, |t, cx| t.set_selected_index(Some(2), cx));
    let file = selected(&mut ui);
    assert_eq!((file.path.as_str(), file.folder), ("README.md", false));
    assert_eq!(file.folder_path(), "");
    assert_eq!(
        ui.files.read_with(ui.cx, |f, _| f.abs("docs/a.md")),
        Some(PathBuf::from("/w/a/docs/a.md"))
    );
}
