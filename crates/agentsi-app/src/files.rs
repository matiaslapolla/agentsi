//! The Files mode of the sidebar: the folder tree of the selected product.
//! Spec: `docs/specs/agentsi-desktop-files.md`.
//!
//! The tree reads one folder at a time with `files.list`: the folder of the product when the
//! mode shows, and a folder when the owner opens it. A `files.changed` of a folder that the
//! tree read makes it read that folder again. The core watches the folders that a list read.
//! The filter matches the names that the tree read, and opens the folders of each match.
//! New file, New folder and Rename show an inline name field in the tree.

use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::path::PathBuf;
use std::rc::Rc;

use agentsi_proto::Event as _;
use agentsi_proto::events::{FilesChanged, FilesChangedParams, Lagged};
use agentsi_proto::files_api::{
    FileEntry, FileGit, FileKind, FilesCreate, FilesCreateParams, FilesList, FilesListResult,
    FilesPathParams, FilesRename, FilesRenameParams, FilesTrash, NewKind,
};
use agentsi_ui::style::StyleExt as _;
use agentsi_ui::tokens::{Color, control, size, space, status, text};
use futures::FutureExt as _;
use gpui_kit::component::input::{Input, InputEvent, InputState};
use gpui_kit::component::list::ListItem;
use gpui_kit::component::tree::{TreeEvent, TreeItem, TreeState, tree};
use gpui_kit::component::{Icon, IconName, Sizable as _};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;

use crate::menus::{self, Cmd, Target};
use crate::model::{CoreState, FilesNotice, ProductRef};

/// The id of the row of the inline name field.
const NAMING: &str = "\0naming";

/// One entry of the tree, for its menu and the palette.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct FileRef {
    pub product: ProductRef,
    /// From the folder of the product. Empty for the folder itself.
    pub path: String,
    pub folder: bool,
}

impl FileRef {
    /// The folder that a new entry goes into: the folder itself, or the folder of a file.
    pub fn folder_path(&self) -> String {
        if self.folder {
            self.path.clone()
        } else {
            parent(&self.path).to_string()
        }
    }
}

/// What the inline name field makes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum NameFor {
    New(NewKind),
    /// The entry at this path gets the new name.
    Rename(String),
}

struct Naming {
    /// The folder of the new or renamed entry.
    folder: String,
    what: NameFor,
    input: Entity<InputState>,
    _sub: Subscription,
}

/// The view asks the shell to run a menu command, as a double click on a file does.
#[derive(Debug, Clone)]
pub struct RunFile(pub Target, pub Cmd);

/// The owner changed what the layout saves: the ignored entries show or hide.
#[derive(Debug, Clone, Copy)]
pub struct FilesSettingChanged;

impl EventEmitter<RunFile> for FileTree {}
impl EventEmitter<FilesSettingChanged> for FileTree {}

#[derive(Clone)]
struct Row {
    name: SharedString,
    kind: FileKind,
    git: Option<FileGit>,
    /// A disabled line: "Reading…", "Empty" or an error.
    note: bool,
}

pub struct FileTree {
    core: Entity<CoreState>,
    product: Option<ProductRef>,
    /// The absolute folder of the product.
    root: Option<PathBuf>,
    /// The folders that the tree read, by path.
    folders: BTreeMap<String, Result<FilesListResult, String>>,
    /// The reads that run, and the folders to read again when their read ends.
    loading: BTreeSet<String>,
    again: BTreeSet<String>,
    /// The open folders.
    open: BTreeSet<String>,
    pub show_ignored: bool,
    shown: bool,
    filter: Entity<InputState>,
    naming: Option<Naming>,
    /// One line about the last write that failed.
    note: Option<String>,
    tree: Entity<TreeState>,
    rows: Rc<HashMap<SharedString, Row>>,
    focus: FocusHandle,
    _subs: Vec<Subscription>,
}

/// The folder of `path`: empty for an entry at the top.
pub fn parent(path: &str) -> &str {
    path.rsplit_once('/').map_or("", |(p, _)| p)
}

fn join(folder: &str, name: &str) -> String {
    if folder.is_empty() {
        name.to_string()
    } else {
        format!("{folder}/{name}")
    }
}

/// The color of a git state: changed, new and ignored.
pub fn git_color(git: Option<FileGit>) -> Color {
    match git {
        Some(FileGit::Changed) => status::warn(),
        Some(FileGit::New) => status::ok(),
        Some(FileGit::Ignored) => text::tertiary(),
        None => text::primary(),
    }
}

impl FileTree {
    pub fn new(core: Entity<CoreState>, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let tree = cx.new(|cx| TreeState::new(cx));
        let filter = cx.new(|cx| InputState::new(window, cx).placeholder("Filter names"));
        let subs = vec![
            cx.subscribe(&tree, |this, _, e: &TreeEvent, cx| match e {
                TreeEvent::Expanded(id) => this.set_folder_open(id.as_ref(), true, cx),
                TreeEvent::Collapsed(id) => this.set_folder_open(id.as_ref(), false, cx),
            }),
            cx.subscribe(&filter, |this, _, e: &InputEvent, cx| {
                if matches!(e, InputEvent::Change) {
                    this.rebuild(cx);
                }
            }),
            cx.subscribe(&core, |this, _, e: &FilesNotice, cx| this.take(&e.0, cx)),
            cx.observe(&core, |this, _, cx| this.follow_active(cx)),
        ];
        let mut t = FileTree {
            core,
            product: None,
            root: None,
            folders: BTreeMap::new(),
            loading: BTreeSet::new(),
            again: BTreeSet::new(),
            open: BTreeSet::new(),
            show_ignored: true,
            shown: false,
            filter,
            naming: None,
            note: None,
            tree,
            rows: Rc::default(),
            focus: cx.focus_handle(),
            _subs: subs,
        };
        t.follow_active(cx);
        t
    }

    pub fn has_focus(&self, window: &Window, cx: &App) -> bool {
        self.focus.contains_focused(window, cx)
    }

    /// Puts the keyboard focus in the tree.
    pub fn take_focus(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.tree.update(cx, |t, cx| t.focus(window, cx));
    }

    /// The mode shows or hides. The folder of the product is read when it first shows.
    pub fn set_shown(&mut self, shown: bool, cx: &mut Context<Self>) {
        self.shown = shown;
        if shown && self.local().is_some() && !self.folders.contains_key("") {
            self.load(String::new(), cx);
        }
    }

    pub fn set_show_ignored(&mut self, show: bool, cx: &mut Context<Self>) {
        if self.show_ignored != show {
            self.show_ignored = show;
            cx.emit(FilesSettingChanged);
            self.rebuild(cx);
        }
    }

    /// Takes the product of the sidebar selection. A product of a host has no tree yet.
    fn follow_active(&mut self, cx: &mut Context<Self>) {
        let core = self.core.read(cx);
        let active = core.active.clone();
        // The product list can come after the selection.
        let root = active.as_ref().filter(|p| p.host.is_none()).and_then(|p| {
            let path = core.product(&p.id)?.path.as_deref()?;
            Some(PathBuf::from(crate::format::expand_home(path)))
        });
        if active == self.product && root == self.root {
            return;
        }
        self.product = active;
        self.root = root;
        self.folders.clear();
        self.loading.clear();
        self.again.clear();
        self.open.clear();
        self.naming = None;
        self.note = None;
        if self.shown && self.local().is_some() {
            self.load(String::new(), cx);
        }
        self.rebuild(cx);
    }

    /// The product, when its tree can be read: it is on this device and has a folder.
    fn local(&self) -> Option<&ProductRef> {
        self.product
            .as_ref()
            .filter(|p| p.host.is_none() && self.root.is_some())
    }

    /// The absolute path of `path`.
    pub fn abs(&self, path: &str) -> Option<PathBuf> {
        let root = self.root.clone()?;
        Some(if path.is_empty() {
            root
        } else {
            root.join(path)
        })
    }

    fn take(&mut self, n: &agentsi_proto::rpc::Notification, cx: &mut Context<Self>) {
        if n.method == Lagged::NAME {
            for path in self.folders.keys().cloned().collect::<Vec<_>>() {
                self.load(path, cx);
            }
            return;
        }
        if n.method != FilesChanged::NAME {
            return;
        }
        let Ok(p) = serde_json::from_value::<FilesChangedParams>(n.params.clone()) else {
            return;
        };
        let mine = self.local().is_some_and(|me| me.id == p.product);
        if mine && self.folders.contains_key(&p.path) {
            self.load(p.path, cx);
        }
    }

    fn set_folder_open(&mut self, id: &str, open: bool, cx: &mut Context<Self>) {
        if id.starts_with('\0') {
            return;
        }
        if open {
            self.open.insert(id.to_string());
            if !self.folders.contains_key(id) {
                self.load(id.to_string(), cx);
            }
        } else {
            self.open.remove(id);
        }
        self.rebuild(cx);
    }

    /// Reads `path` with `files.list`. A read that runs reads again when it ends.
    pub fn load(&mut self, path: String, cx: &mut Context<Self>) {
        let Some(product) = self.local().cloned() else {
            return;
        };
        if !self.loading.insert(path.clone()) {
            self.again.insert(path);
            return;
        }
        let call = self.core.read(cx).link.call::<FilesList>(&FilesPathParams {
            product: product.id.clone(),
            path: path.clone(),
        });
        cx.spawn(async move |this, cx| {
            let result = call.await.map_err(|e| e.to_string());
            let _ = this.update(cx, |t, cx| t.loaded(&product, path, result, cx));
        })
        .detach();
        self.rebuild(cx);
    }

    /// Takes the reply of a read.
    pub fn loaded(
        &mut self,
        product: &ProductRef,
        path: String,
        result: Result<FilesListResult, String>,
        cx: &mut Context<Self>,
    ) {
        if self.product.as_ref() != Some(product) {
            return;
        }
        self.loading.remove(&path);
        match result {
            Err(_) if !path.is_empty() && self.folders.contains_key(&path) => {
                // The folder went away: its parent reads again on its own event.
                self.folders
                    .retain(|k, _| k != &path && !k.starts_with(&format!("{path}/")));
                self.open.remove(&path);
            }
            r => {
                self.folders.insert(path.clone(), r);
            }
        }
        if self.again.remove(&path) {
            self.load(path, cx);
        }
        self.rebuild(cx);
    }

    /// The entry under the selection of the tree, or the folder of the product.
    pub fn selected(&self, cx: &App) -> Option<FileRef> {
        let product = self.local()?.clone();
        let entry = self.tree.read(cx).selected_entry();
        let (path, folder) = match entry {
            Some(e) if !e.item().id.starts_with('\0') => {
                let row = self.rows.get(&e.item().id);
                (
                    e.item().id.to_string(),
                    row.is_some_and(|r| r.kind == FileKind::Folder),
                )
            }
            _ => (String::new(), true),
        };
        Some(FileRef {
            product,
            path,
            folder,
        })
    }

    /// Shows the inline name field in `folder`, or on the entry to rename.
    pub fn start_naming(
        &mut self,
        folder: String,
        what: NameFor,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.local().is_none() {
            return;
        }
        let initial = match &what {
            NameFor::Rename(path) => path.rsplit('/').next().unwrap_or(path).to_string(),
            NameFor::New(_) => String::new(),
        };
        let input = cx.new(|cx| {
            let mut s = InputState::new(window, cx);
            s.set_value(initial, window, cx);
            s
        });
        let sub = cx.subscribe(&input, |this, _, e: &InputEvent, cx| match e {
            InputEvent::PressEnter { .. } => this.commit_naming(cx),
            InputEvent::Blur => this.cancel_naming(cx),
            _ => {}
        });
        let focus = input.focus_handle(cx);
        window.defer(cx, move |window, cx| window.focus(&focus, cx));
        if !folder.is_empty() {
            for (i, _) in folder.match_indices('/').chain([(folder.len(), "")]) {
                let up = folder[..i].to_string();
                self.open.insert(up.clone());
                if !self.folders.contains_key(&up) {
                    self.load(up, cx);
                }
            }
        }
        self.note = None;
        self.naming = Some(Naming {
            folder,
            what,
            input,
            _sub: sub,
        });
        self.rebuild(cx);
    }

    pub fn cancel_naming(&mut self, cx: &mut Context<Self>) {
        if self.naming.take().is_some() {
            self.rebuild(cx);
        }
    }

    #[cfg(test)]
    pub fn is_naming(&self) -> bool {
        self.naming.is_some()
    }

    /// The text of the inline name field, for the tests.
    #[cfg(test)]
    pub fn naming_input(&self) -> Option<Entity<InputState>> {
        self.naming.as_ref().map(|n| n.input.clone())
    }

    /// Enter in the name field: create or rename, then read the folders again.
    fn commit_naming(&mut self, cx: &mut Context<Self>) {
        let Some(product) = self.local().cloned() else {
            return;
        };
        let Some(n) = self.naming.as_ref() else {
            return;
        };
        let name = n
            .input
            .read(cx)
            .value()
            .trim()
            .trim_matches('/')
            .to_string();
        if name.is_empty() {
            self.cancel_naming(cx);
            return;
        }
        let path = join(&n.folder, &name);
        let core = self.core.read(cx);
        let (call, from) = match &n.what {
            NameFor::New(kind) => (
                core.link
                    .call::<FilesCreate>(&FilesCreateParams {
                        product: product.id.clone(),
                        path: path.clone(),
                        kind: *kind,
                    })
                    .boxed_local(),
                None,
            ),
            NameFor::Rename(from) if *from == path => {
                self.cancel_naming(cx);
                return;
            }
            NameFor::Rename(from) => (
                core.link
                    .call::<FilesRename>(&FilesRenameParams {
                        product: product.id.clone(),
                        from: from.clone(),
                        to: path.clone(),
                    })
                    .boxed_local(),
                Some(from.clone()),
            ),
        };
        cx.spawn(async move |this, cx| {
            let result = call.await.map(drop).map_err(|e| e.to_string());
            let _ = this.update(cx, |t, cx| t.named(&product, from, path, result, cx));
        })
        .detach();
    }

    /// Takes the reply of a create or a rename.
    pub fn named(
        &mut self,
        product: &ProductRef,
        from: Option<String>,
        path: String,
        result: Result<(), String>,
        cx: &mut Context<Self>,
    ) {
        if self.product.as_ref() != Some(product) {
            return;
        }
        if let Err(e) = result {
            self.note = Some(e);
            cx.notify();
            return;
        }
        self.naming = None;
        if let Some(from) = &from {
            let moved: Vec<String> = self
                .open
                .iter()
                .filter(|o| *o == from || o.starts_with(&format!("{from}/")))
                .cloned()
                .collect();
            for o in moved {
                self.open.remove(&o);
                self.open.insert(format!("{path}{}", &o[from.len()..]));
            }
            self.load(parent(from).to_string(), cx);
        }
        // Each new folder on the way shows at once, opened.
        let mut folder = String::new();
        for part in path.split('/') {
            self.open.insert(folder.clone());
            self.load(folder.clone(), cx);
            folder = join(&folder, part);
        }
        self.select(&path, cx);
    }

    fn select(&mut self, path: &str, cx: &mut Context<Self>) {
        let id = SharedString::from(path.to_string());
        self.tree.update(cx, |t, cx| {
            let ix = t.index_of(&id);
            t.set_selected_index(ix, cx);
        });
    }

    /// `files.trash` of `path`. The folder reads again after it.
    pub fn trash(&mut self, path: String, cx: &mut Context<Self>) {
        let Some(product) = self.local().cloned() else {
            return;
        };
        let call = self
            .core
            .read(cx)
            .link
            .call::<FilesTrash>(&FilesPathParams {
                product: product.id.clone(),
                path: path.clone(),
            });
        cx.spawn(async move |this, cx| {
            let result = call.await.map(drop).map_err(|e| e.to_string());
            let _ = this.update(cx, |t, cx| {
                match result {
                    Ok(()) => {
                        t.note = None;
                        t.load(parent(&path).to_string(), cx);
                    }
                    Err(e) => t.note = Some(format!("{path} stays: {e}")),
                }
                cx.notify();
            });
        })
        .detach();
    }

    fn filter_text(&self, cx: &App) -> Option<String> {
        let f = self.filter.read(cx).value().trim().to_lowercase();
        (!f.is_empty()).then_some(f)
    }

    /// Builds the items of the tree from the folders that it read.
    fn rebuild(&mut self, cx: &mut Context<Self>) {
        let filter = self.filter_text(cx);
        let mut rows = HashMap::new();
        let items = self.items("", filter.as_deref(), &mut rows);
        self.rows = Rc::new(rows);
        self.tree.update(cx, |t, cx| {
            let keep = t.selected_item().map(|i| i.id.clone());
            t.set_items(items, cx);
            if let Some(id) = keep {
                let ix = t.index_of(&id);
                t.set_selected_index(ix, cx);
            }
        });
        cx.notify();
    }

    fn note_item(&self, id: String, text: &str, rows: &mut HashMap<SharedString, Row>) -> TreeItem {
        rows.insert(
            id.clone().into(),
            Row {
                name: text.to_string().into(),
                kind: FileKind::File,
                git: None,
                note: true,
            },
        );
        TreeItem::new(id, text.to_string()).disabled(true)
    }

    /// The items of the folder `folder`. With a filter, only the matches and their folders.
    fn items(
        &self,
        folder: &str,
        filter: Option<&str>,
        rows: &mut HashMap<SharedString, Row>,
    ) -> Vec<TreeItem> {
        let mut out = Vec::new();
        let naming_here = self
            .naming
            .as_ref()
            .filter(|n| n.folder == folder && matches!(n.what, NameFor::New(_)));
        if let Some(n) = naming_here {
            out.push(self.naming_item(n, rows));
        }
        let list = match self.folders.get(folder) {
            Some(Ok(list)) => list,
            Some(Err(e)) => {
                out.push(self.note_item(format!("\0error:{folder}"), e, rows));
                return out;
            }
            None => {
                if filter.is_none() {
                    out.push(self.note_item(format!("\0reading:{folder}"), "Reading…", rows));
                }
                return out;
            }
        };
        for e in &list.entries {
            if !self.show_ignored && e.git == Some(FileGit::Ignored) {
                continue;
            }
            let path = join(folder, &e.name);
            let renaming = self
                .naming
                .as_ref()
                .filter(|n| n.what == NameFor::Rename(path.clone()));
            if let Some(n) = renaming {
                out.push(self.naming_item(n, rows));
                continue;
            }
            let hit = filter.is_none_or(|f| e.name.to_lowercase().contains(f));
            let item = if e.kind == FileKind::Folder {
                let open = self.open.contains(&path);
                let mut children = if open || filter.is_some() {
                    self.items(&path, filter, rows)
                } else {
                    Vec::new()
                };
                if filter.is_some() && !hit && children.is_empty() {
                    continue;
                }
                let expand = open || (filter.is_some() && !children.is_empty());
                if children.is_empty() {
                    let empty = if self.folders.contains_key(&path) {
                        "Empty"
                    } else {
                        "Reading…"
                    };
                    children.push(self.note_item(format!("\0note:{path}"), empty, rows));
                }
                TreeItem::new(path.clone(), e.name.clone())
                    .children(children)
                    .expanded(expand)
            } else {
                if !hit {
                    continue;
                }
                TreeItem::new(path.clone(), e.name.clone())
            };
            rows.insert(path.into(), Row::of(e));
            out.push(item);
        }
        if list.truncated && filter.is_none() {
            out.push(self.note_item(
                format!("\0more:{folder}"),
                "More entries: the list stops at 5000",
                rows,
            ));
        }
        out
    }

    fn naming_item(&self, n: &Naming, rows: &mut HashMap<SharedString, Row>) -> TreeItem {
        let kind = match &n.what {
            NameFor::New(NewKind::Folder) => FileKind::Folder,
            NameFor::Rename(path) => self
                .rows
                .get(&SharedString::from(path.clone()))
                .map_or(FileKind::File, |r| r.kind),
            NameFor::New(NewKind::File) => FileKind::File,
        };
        rows.insert(
            NAMING.into(),
            Row {
                name: SharedString::default(),
                kind,
                git: None,
                note: false,
            },
        );
        TreeItem::new(NAMING, "")
    }

    fn target(&self, id: &str) -> Option<Target> {
        let product = self.local()?.clone();
        let row = self.rows.get(&SharedString::from(id.to_string()))?;
        Some(Target::File(FileRef {
            product,
            path: id.to_string(),
            folder: row.kind == FileKind::Folder,
        }))
    }

    /// The names on screen, top to bottom, with the open state of a folder. For the tests.
    #[cfg(test)]
    pub fn shown_names(&self, cx: &App) -> Vec<String> {
        let t = self.tree.read(cx);
        (0..)
            .map_while(|ix| t.entry(ix))
            .map(|e| {
                let name = self
                    .rows
                    .get(&e.item().id)
                    .map_or(String::new(), |r| r.name.to_string());
                let id = &e.item().id;
                let name = if id.as_ref() == NAMING {
                    "[name]".to_string()
                } else {
                    name
                };
                format!("{}{}", "  ".repeat(e.depth()), name)
            })
            .collect()
    }

    #[cfg(test)]
    pub fn filter_input(&self) -> Entity<InputState> {
        self.filter.clone()
    }

    #[cfg(test)]
    pub fn tree_state(&self) -> Entity<TreeState> {
        self.tree.clone()
    }
}

impl Row {
    fn of(e: &FileEntry) -> Row {
        Row {
            name: e.name.clone().into(),
            kind: e.kind,
            git: e.git,
            note: false,
        }
    }
}

fn icon_of(kind: FileKind, open: bool) -> IconName {
    match kind {
        FileKind::Folder if open => IconName::FolderOpen,
        FileKind::Folder => IconName::Folder,
        FileKind::File => IconName::File,
        FileKind::Link => IconName::ExternalLink,
    }
}

impl Render for FileTree {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let message = match (&self.product, &self.root) {
            (None, _) => Some("Pick a product to see its files."),
            (Some(p), _) if p.host.is_some() => {
                Some("The files of a product on a host come in a later version.")
            }
            (Some(_), None) => Some("This product has no folder."),
            _ => None,
        };
        let rows = self.rows.clone();
        let naming = self.naming.as_ref().map(|n| n.input.clone());
        let this = cx.weak_entity();
        let root_error = match self.folders.get("") {
            Some(Err(e)) => Some(e.clone()),
            _ => None,
        };
        let body = tree(&self.tree, move |ix, entry, _selected, _, _| {
            let id = entry.item().id.clone();
            let row = rows.get(&id).cloned();
            let indent = space(3) * entry.depth() as f32;
            let mut item = ListItem::new(ix).h(control::height_sm()).py_0();
            let line = div()
                .debug_selector(|| format!("file-row:{id}"))
                .flex()
                .items_center()
                .gap(space(2))
                .w_full()
                .pl(indent)
                .text_size(size::sm());
            let Some(row) = row else {
                return item.child(line);
            };
            if id.as_ref() == NAMING {
                let line = line.child(
                    Icon::new(icon_of(row.kind, false))
                        .size(control::icon_sm())
                        .text_color(text::secondary()),
                );
                return match &naming {
                    Some(input) => item.child(
                        line.debug_selector(|| "file-naming".into())
                            .on_key_down({
                                let this = this.clone();
                                move |e: &KeyDownEvent, _, cx| {
                                    if e.keystroke.key == "escape" {
                                        let _ = this.update(cx, |t, cx| t.cancel_naming(cx));
                                        cx.stop_propagation();
                                    }
                                }
                            })
                            .child(Input::new(input).small().flex_1()),
                    ),
                    None => item.child(line),
                };
            }
            if row.note {
                return item.child(line.text_color(text::tertiary()).child(row.name.clone()));
            }
            let target = {
                let this = this.clone();
                let id = id.clone();
                move |cx: &App| this.upgrade().and_then(|t| t.read(cx).target(&id))
            };
            item = item.on_mouse_down(MouseButton::Right, {
                let target = target.clone();
                move |e, _, cx| {
                    if let Some(t) = target(cx) {
                        menus::request(cx, t, e.position);
                    }
                }
            });
            if row.kind != FileKind::Folder {
                let this = this.clone();
                item = item.on_click(move |e, _, cx| {
                    if e.click_count() >= 2
                        && let Some(t) = target(cx)
                    {
                        let _ = this.update(cx, |_, cx| cx.emit(RunFile(t, Cmd::Open)));
                    }
                });
            }
            item.child(
                line.child(
                    Icon::new(icon_of(row.kind, entry.is_expanded()))
                        .size(control::icon_sm())
                        .text_color(text::secondary()),
                )
                .child(
                    div()
                        .flex_1()
                        .min_w_0()
                        .truncate()
                        .text_color(git_color(row.git))
                        .child(row.name.clone()),
                ),
            )
        });
        let ignored = self.show_ignored;
        div()
            .debug_selector(|| "files".into())
            .track_focus(&self.focus)
            .flex()
            .flex_col()
            .flex_1()
            .min_h_0()
            .gap(space(2))
            .p(space(2))
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap(space(1))
                    .child(div().flex_1().child(Input::new(&self.filter).small()))
                    .child(
                        agentsi_ui::components::Button::new("files-ignored")
                            .icon(if ignored {
                                IconName::Eye
                            } else {
                                IconName::EyeOff
                            })
                            .ghost()
                            .small()
                            .on_click(cx.listener(move |this, _, _, cx| {
                                this.set_show_ignored(!ignored, cx)
                            })),
                    ),
            )
            .when_some(self.note.clone(), |el, note| {
                el.child(
                    div()
                        .text_size(size::xs())
                        .text_color(status::danger())
                        .child(note),
                )
            })
            .map(|el| match message.map(str::to_string).or(root_error) {
                Some(m) => el.child(
                    div()
                        .pad_x(space(2))
                        .text_size(size::sm())
                        .text_color(text::secondary())
                        .child(m),
                ),
                None => el.child(div().flex_1().min_h_0().child(body)),
            })
    }
}

#[cfg(test)]
mod tests;
