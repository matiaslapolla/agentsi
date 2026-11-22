//! The command palette: its sources, its prefixes, the recent targets and its second step.
//! Spec: `docs/specs/agentsi-desktop-palette-more.md`.
//!
//! The palette filters its items itself, so the `Command` of gpui-component shows each item that
//! a source gives. A prefix limits the palette to one source. With no query, the recent targets
//! come first, then the actions. The files source asks the core with `files.find`, only with the
//! prefix `/`, and only after a pause in the typing.
//!
//! An entry that asks a question opens a [`PaletteStep`]: the question of `Shell::ask`, with the
//! query of the palette under it. Escape in the question opens the palette again at that query.

mod recent;
mod sources;
#[cfg(test)]
pub(crate) mod tests;

pub use recent::{Closed, ClosedTab, Opened, Recent};
pub use sources::source;

use std::rc::Rc;
use std::time::Duration;

use agentsi_proto::files_api::{FilesFind, FilesFindParams, FilesFindResult};
use agentsi_ui::style::StyleExt as _;
use agentsi_ui::tokens::{control, radius, size, space, surface, text};
use gpui_kit::component::IconName;
use gpui_kit::component::command::{Command, CommandItem, CommandState};
use gpui_kit::component::dock::PanelId;
use gpui_kit::*;

use crate::model::ProductRef;
use crate::shell::Shell;

/// The pause in the typing before the files source asks the core.
pub const FIND_DELAY: Duration = Duration::from_millis(150);
/// The most files that the files source shows.
const FIND_LIMIT: usize = 50;

/// One kind of result of the palette.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    Recent,
    Actions,
    Tabs,
    Settings,
    Files,
    Favorites,
}

/// Each prefix, the source it limits the palette to, and its word in the footer.
pub const PREFIXES: [(char, Kind, &str); 4] = [
    ('>', Kind::Actions, "actions"),
    ('@', Kind::Tabs, "tabs"),
    ('#', Kind::Settings, "settings"),
    ('/', Kind::Files, "files"),
];

/// The source that the prefix of `query` names, and the query after the prefix.
pub fn split(query: &str) -> (Option<Kind>, &str) {
    let query = query.trim_start();
    let mut chars = query.chars();
    match chars
        .next()
        .and_then(|c| PREFIXES.iter().find(|p| p.0 == c))
    {
        Some(p) => (Some(p.1), chars.as_str().trim()),
        None => (None, query.trim_end()),
    }
}

/// The sources of a query, in the order of the list.
pub fn kinds(query: &str) -> Vec<Kind> {
    match split(query) {
        (Some(k), _) => vec![k],
        (None, "") => vec![Kind::Recent, Kind::Actions, Kind::Settings],
        (None, _) => vec![
            Kind::Recent,
            Kind::Actions,
            Kind::Tabs,
            Kind::Settings,
            Kind::Favorites,
        ],
    }
}

/// True when `query` is in the label or in a keyword, with no regard to case. An empty query
/// matches each item. The palette matched its entries so before it had sources.
pub fn matches<S: AsRef<str>>(label: &str, keywords: &[S], query: &str) -> bool {
    if query.is_empty() {
        return true;
    }
    let query = query.to_lowercase();
    label.to_lowercase().contains(&query)
        || keywords
            .iter()
            .any(|k| k.as_ref().to_lowercase().contains(&query))
}

/// What an item does.
pub enum Run {
    /// `Command` dispatches the action itself.
    Action(Box<dyn Action>),
    /// A product past the ninth, which has no action.
    Product(ProductRef),
    Tab(PanelId),
    File(ProductRef, String),
    /// A favorite, by its place in the list.
    Favorite(usize),
    Recent(Opened),
}

pub struct PaletteItem {
    pub label: SharedString,
    pub icon: IconName,
    pub keywords: Vec<SharedString>,
    pub source: Kind,
    pub run: Run,
}

impl PaletteItem {
    pub fn new(source: Kind, label: impl Into<SharedString>, icon: IconName, run: Run) -> Self {
        PaletteItem {
            label: label.into(),
            icon,
            keywords: Vec::new(),
            source,
            run,
        }
    }

    pub fn keywords<S: Into<SharedString>>(mut self, k: impl IntoIterator<Item = S>) -> Self {
        self.keywords = k.into_iter().map(Into::into).collect();
        self
    }

    fn matches(&self, query: &str) -> bool {
        matches(&self.label, &self.keywords, query)
    }

    fn command_item(&self) -> CommandItem {
        let item = CommandItem::new()
            .label(self.label.clone())
            .icon(self.icon.clone())
            .keywords(self.keywords.iter().cloned());
        match &self.run {
            Run::Action(a) => item.action(a.boxed_clone()),
            _ => item,
        }
    }
}

/// One kind of result: the items of a query, and what an item does.
pub trait PaletteSource {
    /// The items that match `query`, which has no prefix, best first.
    fn items(&self, query: &str, shell: &Shell, cx: &App) -> Vec<PaletteItem>;
    /// Runs an item of this source. The palette closes after it, unless the item opened a
    /// question or the palette again.
    fn run(&self, run: Run, shell: &mut Shell, window: &mut Window, cx: &mut Context<Shell>);
}

/// Each item of `query`, in the order of the sources.
pub fn items(query: &str, shell: &Shell, cx: &App) -> Vec<PaletteItem> {
    let (_, rest) = split(query);
    kinds(query)
        .into_iter()
        .flat_map(|k| source(k).items(rest, shell, cx))
        .collect()
}

/// The answer of the core for the files source.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct Found {
    pub product: Option<ProductRef>,
    pub query: String,
    pub paths: Vec<String>,
    pub truncated: bool,
    /// The core did not answer yet.
    pub waiting: bool,
    pub error: Option<String>,
}

/// The open palette.
pub struct Palette {
    pub state: Entity<CommandState>,
    query: String,
    pub files: Found,
    find: Option<Task<()>>,
}

impl Palette {
    pub fn new(query: &str, window: &mut Window, cx: &mut Context<Shell>) -> Palette {
        let state = cx.new(|cx| CommandState::new(window, cx));
        state.update(cx, |s, cx| {
            s.set_query(query.to_string(), window, cx);
            s.focus(window, cx);
        });
        Palette {
            state,
            query: query.to_string(),
            files: Found::default(),
            find: None,
        }
    }

    pub fn query(&self) -> &str {
        &self.query
    }
}

/// The second step of the palette: an entry asked a question. Escape in the question opens the
/// palette again at `query`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PaletteStep {
    pub query: String,
}

impl Shell {
    /// Opens the palette at `query`, in place of an open one.
    pub(crate) fn open_palette(
        &mut self,
        query: &str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let palette = Palette::new(query, window, cx);
        self.palette = Some(palette);
        self.palette_query(query, cx);
    }

    /// The owner typed in the palette.
    fn palette_query(&mut self, query: &str, cx: &mut Context<Self>) {
        let Some(p) = self.palette.as_mut() else {
            return;
        };
        p.query = query.to_string();
        if let (Some(Kind::Files), rest) = split(query) {
            let rest = rest.to_string();
            self.find_files(rest, cx);
        }
        cx.notify();
    }

    /// Asks the core for the files of the active product that match `query`, after a pause. A
    /// new key press drops the request that waits.
    fn find_files(&mut self, query: String, cx: &mut Context<Self>) {
        let product = self.core.read(cx).active.clone();
        let core = product
            .as_ref()
            .and_then(|p| self.cores.of(p.host.as_deref()).cloned());
        let Some(palette) = self.palette.as_mut() else {
            return;
        };
        if palette.files.query == query && palette.files.product == product {
            return;
        }
        let (Some(product), Some(core), false) = (product.clone(), core, query.is_empty()) else {
            palette.files = Found {
                product,
                query,
                ..Found::default()
            };
            palette.find = None;
            return;
        };
        palette.files = Found {
            product: Some(product.clone()),
            query: query.clone(),
            waiting: true,
            ..Found::default()
        };
        let link = core.read(cx).link.clone();
        palette.find = Some(cx.spawn(async move |this, cx| {
            cx.background_executor().timer(FIND_DELAY).await;
            let params = FilesFindParams {
                product: product.id.clone(),
                query: query.clone(),
                limit: Some(FIND_LIMIT),
            };
            let result = link.call::<FilesFind>(&params).await;
            let result = result.map_err(|e| e.to_string());
            let _ = this.update(cx, |this, cx| {
                this.files_found(&product, &query, result, cx)
            });
        }));
    }

    /// The answer of `files.find`. An answer to an older query is dropped.
    pub(crate) fn files_found(
        &mut self,
        product: &ProductRef,
        query: &str,
        result: Result<FilesFindResult, String>,
        cx: &mut Context<Self>,
    ) {
        let Some(p) = self.palette.as_mut() else {
            return;
        };
        if p.files.query != query || p.files.product.as_ref() != Some(product) {
            return;
        }
        p.files.waiting = false;
        match result {
            Ok(r) => {
                p.files.paths = r.paths;
                p.files.truncated = r.truncated;
            }
            Err(e) => p.files.error = Some(e),
        }
        cx.notify();
    }

    /// The line that shows when no item matches.
    fn palette_empty(&self, palette: &Palette) -> String {
        let f = &palette.files;
        match split(&palette.query) {
            (Some(Kind::Files), "") => "Type a part of a file name.".into(),
            (Some(Kind::Files), _) if f.product.is_none() => {
                "No active product. Pick one in the sidebar first.".into()
            }
            (Some(Kind::Files), _) if f.waiting => "Reading the files…".into(),
            (Some(Kind::Files), _) if f.error.is_some() => {
                format!(
                    "The files cannot be read: {}",
                    f.error.clone().unwrap_or_default()
                )
            }
            _ => "Nothing matches.".into(),
        }
    }

    pub(crate) fn palette_view(&self, cx: &mut Context<Self>) -> Option<AnyElement> {
        let palette = self.palette.as_ref()?;
        let state = palette.state.clone();
        let items = Rc::new(items(&palette.query, self, cx));
        let empty = SharedString::from(self.palette_empty(palette));
        let commands: Vec<CommandItem> = items.iter().map(PaletteItem::command_item).collect();
        let this = cx.weak_entity();
        let (cancel, query) = (this.clone(), this.clone());
        let confirmed = state.entity_id();
        let element = div()
            .absolute()
            .top_0()
            .left_0()
            .size_full()
            .flex()
            .justify_center()
            .items_start()
            .pt(space(16))
            .bg(surface::base().alpha(control::SCRIM_ALPHA))
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(|this, _, window, cx| this.close_palette(window, cx)),
            )
            .child(
                div()
                    .w(control::PALETTE_WIDTH)
                    .h_auto()
                    .rounded(radius::lg())
                    .bg(surface::overlay())
                    .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                    .child(
                        Command::new(&state)
                            .items(commands)
                            .filterable(false)
                            .placeholder("Type a command, a tab, a setting or / and a file")
                            .empty(move |_, _, _| {
                                div()
                                    .p(space(3))
                                    .text_size(size::sm())
                                    .text_color(text::tertiary())
                                    .child(empty.clone())
                            })
                            .footer(|_, _, _| footer())
                            .on_query(move |q, _, cx| {
                                let _ = query.update(cx, |this, cx| this.palette_query(q, cx));
                            })
                            .on_confirm(move |ix, window, cx| {
                                let items = items.clone();
                                let _ = this.update(cx, |this, cx| {
                                    let focused =
                                        this.palette_confirmed(&items, ix.row, window, cx);
                                    // An entry that opened the palette again keeps it.
                                    let same = this
                                        .palette
                                        .as_ref()
                                        .is_some_and(|p| p.state.entity_id() == confirmed);
                                    if same && focused {
                                        this.palette = None;
                                        cx.notify();
                                    } else if same {
                                        this.close_palette(window, cx);
                                    }
                                });
                            })
                            .on_cancel(move |window, cx| {
                                let _ =
                                    cancel.update(cx, |this, cx| this.close_palette(window, cx));
                            }),
                    ),
            )
            .into_any_element();
        Some(element)
    }

    /// Runs the item at `row`. `Command` dispatched an action already. True when the item put
    /// the focus on a tab, which the palette then keeps.
    fn palette_confirmed(
        &mut self,
        items: &Rc<Vec<PaletteItem>>,
        row: usize,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> bool {
        let Some(item) = items.get(row) else {
            return false;
        };
        let focused = matches!(
            item.run,
            Run::Product(_) | Run::Tab(_) | Run::Favorite(_) | Run::Recent(_)
        );
        let run = match &item.run {
            Run::Action(_) => return false,
            Run::Product(p) => Run::Product(p.clone()),
            Run::Tab(p) => Run::Tab(*p),
            Run::File(p, path) => Run::File(p.clone(), path.clone()),
            Run::Favorite(at) => Run::Favorite(*at),
            Run::Recent(t) => Run::Recent(t.clone()),
        };
        source(item.source).run(run, self, window, cx);
        focused && self.asking.is_none()
    }

    /// Puts `target` first in the recent targets, and saves the layout.
    pub(crate) fn opened(&mut self, target: Opened, cx: &mut Context<Self>) {
        if self.recent.push(target) {
            self.schedule_save(cx);
        }
    }

    /// Opens a recent target again.
    pub(crate) fn open_recent(
        &mut self,
        target: Opened,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        match target {
            Opened::File(product, path) => self.open_file(product, path, window, cx),
            Opened::Saved(f) => match f.screen() {
                Some(spec) => self.show_screen(spec, window, cx),
                None => self.open_saved(f, window, cx),
            },
        }
    }
}

fn footer() -> impl IntoElement {
    div()
        .flex()
        .gap(space(4))
        .pad_x(space(3))
        .py(space(2))
        .text_size(size::xs())
        .text_color(text::tertiary())
        .children(
            PREFIXES
                .iter()
                .map(|(c, _, word)| div().child(format!("{c} {word}"))),
        )
}
