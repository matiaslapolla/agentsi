//! The sidebar: the products of each host as a tree. This device comes first, then each SSH
//! host of `hosts.toml`. A product of a host that does not answer shows "device off".
//!
//! - A product row has an arrow. An open product shows its item rows: its panes, its running
//!   meeting, its story sessions and its running reflection (`tree.rs`).
//! - A product group is a header with a color square. A group collapses.
//! - The list takes focus as one element. `Up` and `Down` move a cursor, `Right` opens a
//!   product, `Left` closes it, and `Enter` selects the row. The rows have no focus handles.
//! - The open and closed state is `layout.json` data: the sidebar keeps it and the shell saves it.

use std::cell::RefCell;
use std::collections::BTreeMap;
use std::rc::Rc;

use agentsi_proto::desk_api::ProductInfo;
use agentsi_ui::components::{
    AttentionMarker, GroupHeader, ItemRow, Kbd, ProductRow, StatusDot, neutral_tag, running_chip,
};
use agentsi_ui::style::StyleExt as _;
use agentsi_ui::tokens::{Status, accent, control, line, size, space, surface, text, weight};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;

use crate::actions::product_id;
use crate::hosts::{Hosts, HostsGlobal, RemoteHost};
use crate::keys::{self, Keys};
use crate::look::{self, Look, LookData, LookGlobal};
use crate::menus::{self, Target};
use crate::model::{CoreState, Cores, HostState, ProductRef};
use crate::pane::{AttentionProbe, Marker};
use crate::screens::minutes::cursor_after;
use crate::shell::PaneRef;
use crate::tree::{self, Item, ItemKey};

/// The owner picked a product in the sidebar.
pub struct SelectProduct(pub ProductRef);

/// The owner picked an item row under a product.
pub struct OpenItem {
    pub product: ProductRef,
    pub item: ItemKey,
}

/// A product opened or closed: the shell saves the layout again.
pub struct TreeChanged;

/// A row that the cursor can stand on.
#[derive(Clone, Debug, PartialEq, Eq)]
enum RowKey {
    /// A product group: its host and its name.
    Group(Option<String>, String),
    Product(ProductRef),
    Item(ProductRef, ItemKey),
}

/// Where each row of the last draw is.
type Placed = Vec<(RowKey, Bounds<Pixels>)>;

/// A product that the owner drags onto a product group.
#[derive(Clone)]
struct DraggedProduct {
    product: ProductRef,
    name: SharedString,
}

impl Render for DraggedProduct {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        neutral_tag(self.name.clone())
    }
}

pub struct Sidebar {
    core: Entity<CoreState>,
    cores: Cores,
    hosts: Option<Entity<Hosts>>,
    look: Option<Entity<Look>>,
    /// The attention bench times the product marker too.
    probe: Option<AttentionProbe>,
    focus: FocusHandle,
    /// The row of the keyboard cursor.
    cursor: Option<RowKey>,
    /// The rows of the last draw, top to bottom: what `Up` and `Down` move over.
    stops: Vec<RowKey>,
    /// Where each row was drawn, for a menu that the keyboard opens.
    placed: Rc<RefCell<Placed>>,
    /// The products that the owner opened or closed, by `look::key`. Absent means open.
    open: BTreeMap<String, bool>,
    _observe: Vec<Subscription>,
}

impl EventEmitter<SelectProduct> for Sidebar {}
impl EventEmitter<OpenItem> for Sidebar {}
impl EventEmitter<TreeChanged> for Sidebar {}

impl Sidebar {
    pub fn new(cores: Cores, probe: Option<AttentionProbe>, cx: &mut Context<Self>) -> Self {
        let hosts = cx.try_global::<HostsGlobal>().map(|h| h.0.clone());
        let look = cx.try_global::<LookGlobal>().map(|l| l.0.clone());
        let mut observe: Vec<Subscription> = cores
            .all()
            .map(|c| cx.observe(c, |_, _, cx| cx.notify()))
            .collect();
        // A meeting has no event on the core entity: its feed tells the sidebar.
        for c in cores.all() {
            if let Some(feed) = c.read(cx).meetings.clone() {
                observe.push(cx.observe(&feed, |_, _, cx| cx.notify()));
            }
        }
        if let Some(h) = &hosts {
            observe.push(cx.observe(h, |_, _, cx| cx.notify()));
        }
        if let Some(l) = &look {
            observe.push(cx.observe(l, |_, _, cx| cx.notify()));
        }
        // The sidebar is cached: a new key shows only after it draws again.
        observe.push(cx.observe_global::<Keys>(|_, cx| cx.notify()));
        Sidebar {
            probe,
            hosts,
            look,
            focus: cx.focus_handle().tab_stop(true),
            cursor: None,
            stops: Vec::new(),
            placed: Rc::default(),
            open: BTreeMap::new(),
            _observe: observe,
            core: cores.local.clone(),
            cores,
        }
    }

    /// The products that the owner opened or closed, for `layout.json`.
    pub fn open_state(&self) -> BTreeMap<String, bool> {
        self.open.clone()
    }

    /// Takes the state of `layout.json` at start.
    pub fn set_open_state(&mut self, open: BTreeMap<String, bool>, cx: &mut Context<Self>) {
        self.open = open;
        cx.notify();
    }

    pub fn is_open(&self, product: &ProductRef) -> bool {
        self.open
            .get(&look::key(product.host.as_deref(), &product.id))
            .copied()
            .unwrap_or(true)
    }

    /// Opens or closes a product, and tells the shell to save.
    pub fn set_open(&mut self, product: &ProductRef, open: bool, cx: &mut Context<Self>) {
        if self.is_open(product) == open {
            return;
        }
        self.open
            .insert(look::key(product.host.as_deref(), &product.id), open);
        cx.emit(TreeChanged);
        cx.notify();
    }

    fn data(&self, cx: &App) -> LookData {
        self.look
            .as_ref()
            .map(|l| l.read(cx).data.clone())
            .unwrap_or_default()
    }

    /// The object that the menu of `key` belongs to.
    fn target_of(key: &RowKey) -> Target {
        match key {
            RowKey::Group(host, name) => Target::Group(host.clone(), name.clone()),
            RowKey::Product(p) => Target::Product(p.clone()),
            RowKey::Item(p, item) => match item {
                ItemKey::Pane(id) => Target::Pane(PaneRef {
                    host: p.host.clone(),
                    id: id.clone(),
                }),
                ItemKey::Meeting => Target::Meeting(p.clone()),
                ItemKey::Story(id) => Target::Story(p.clone(), id.clone()),
                ItemKey::Reflection => Target::Reflection(p.clone()),
            },
        }
    }

    /// Gives the sidebar the keyboard focus, as a click on its arrow does.
    #[cfg(test)]
    pub fn take_focus(&self, window: &mut Window, cx: &mut App) {
        window.focus(&self.focus, cx);
    }

    /// The sidebar has the keyboard focus.
    pub fn has_focus(&self, window: &Window, cx: &App) -> bool {
        self.focus.contains_focused(window, cx)
    }

    /// Opens the menu of the row of the cursor, just under the row. Returns false when no row
    /// has the cursor.
    pub fn open_menu_at_cursor(&mut self, cx: &mut Context<Self>) -> bool {
        let Some(cursor) = self.cursor.clone() else {
            return false;
        };
        let Some(bounds) = self
            .placed
            .borrow()
            .iter()
            .find(|(k, _)| *k == cursor)
            .map(|(_, b)| *b)
        else {
            return false;
        };
        let at = Point::new(bounds.origin.x + space(4), bounds.bottom());
        menus::request(cx, Self::target_of(&cursor), at);
        true
    }

    /// The handler of a right click on the row `key`: the cursor moves there and the shell opens
    /// the menu.
    fn on_menu(
        &self,
        key: RowKey,
        cx: &mut Context<Self>,
    ) -> impl Fn(&MouseDownEvent, &mut Window, &mut App) + 'static {
        cx.listener(move |this, e: &MouseDownEvent, window, cx| {
            this.cursor = Some(key.clone());
            window.focus(&this.focus, cx);
            menus::request(cx, Self::target_of(&key), e.position);
            cx.notify();
        })
    }

    /// `row` with a note of where it is drawn, for the menu key.
    fn placed(&self, key: RowKey, row: impl IntoElement) -> AnyElement {
        let placed = self.placed.clone();
        div()
            .relative()
            .child(row)
            .child(
                canvas(
                    move |bounds, _, _| placed.borrow_mut().push((key, bounds)),
                    |_, _, _, _| {},
                )
                .absolute()
                .size_full(),
            )
            .into_any_element()
    }

    fn toggle_group(&mut self, host: Option<&str>, name: &str, cx: &mut Context<Self>) {
        if let Some(l) = &self.look {
            l.update(cx, |l, cx| l.toggle_group(host, name, cx));
        }
    }

    /// A product dropped on a group, or on a row of it, goes into that group. An empty `group`
    /// takes it out of its group. A product never moves to a group of another host.
    fn drop_product(
        &mut self,
        dragged: &DraggedProduct,
        host: Option<&str>,
        group: &str,
        cx: &mut Context<Self>,
    ) {
        if dragged.product.host.as_deref() != host {
            return;
        }
        if let Some(l) = &self.look {
            let id = dragged.product.id.clone();
            l.update(cx, |l, cx| l.move_product(host, &id, group, cx));
        }
    }

    /// The handlers of a row that takes a dropped product into `group` of `host`.
    fn on_drop(
        &self,
        host: Option<String>,
        group: String,
        cx: &mut Context<Self>,
    ) -> impl FnOnce(Stateful<Div>) -> Stateful<Div> + 'static {
        let drop = cx.listener(move |this, d: &DraggedProduct, _, cx| {
            this.drop_product(d, host.as_deref(), &group, cx)
        });
        move |el| {
            el.on_drop(drop)
                .drag_over::<DraggedProduct>(|style, _, _, _| style.bg(accent::SUBTLE))
        }
    }

    /// A host header that takes a dropped product out of its group.
    fn host_drop(
        &self,
        name: String,
        host: Option<String>,
        header: Div,
        cx: &mut Context<Self>,
    ) -> Stateful<Div> {
        let drop = self.on_drop(host, String::new(), cx);
        drop(
            div()
                .id(ElementId::Name(name.clone().into()))
                .debug_selector(move || name)
                .child(header),
        )
    }

    /// The row of the cursor does what a click on it does.
    fn activate(&mut self, key: RowKey, cx: &mut Context<Self>) {
        match key {
            RowKey::Product(p) => cx.emit(SelectProduct(p)),
            RowKey::Item(product, item) => cx.emit(OpenItem { product, item }),
            RowKey::Group(host, name) => self.toggle_group(host.as_deref(), &name, cx),
        }
    }

    fn key_down(&mut self, e: &KeyDownEvent, cx: &mut Context<Self>) {
        let k = &e.keystroke;
        if k.modifiers.modified() {
            return;
        }
        let at = self
            .cursor
            .as_ref()
            .and_then(|c| self.stops.iter().position(|s| s == c));
        let handled = match k.key.as_str() {
            "enter" | "space" => {
                match self.cursor.clone() {
                    Some(c) => self.activate(c, cx),
                    None => return,
                }
                true
            }
            "left" | "right" => self.step_sideways(k.key == "right", at, cx),
            key => {
                let len = self.stops.len();
                let to = match at {
                    None if matches!(key, "down" | "home" | "pagedown") => (len > 0).then_some(0),
                    None if matches!(key, "up" | "end" | "pageup") => len.checked_sub(1),
                    None => None,
                    Some(i) => cursor_after(key, i, len),
                };
                match to.and_then(|i| self.stops.get(i).cloned()) {
                    Some(c) => {
                        self.cursor = Some(c);
                        cx.notify();
                        true
                    }
                    None => false,
                }
            }
        };
        if handled {
            cx.stop_propagation();
        }
    }

    /// `Right` opens a product or a group, or goes into the product. `Left` closes it, or goes
    /// back to the product from an item row.
    fn step_sideways(&mut self, right: bool, at: Option<usize>, cx: &mut Context<Self>) -> bool {
        let Some(cursor) = self.cursor.clone() else {
            return false;
        };
        let data = self.data(cx);
        match (cursor, right) {
            (RowKey::Product(p), true) if !self.is_open(&p) => self.set_open(&p, true, cx),
            (RowKey::Product(_), true) => {
                if let Some(next @ RowKey::Item(..)) = at.and_then(|i| self.stops.get(i + 1)) {
                    self.cursor = Some(next.clone());
                    cx.notify();
                }
            }
            (RowKey::Product(p), false) if self.is_open(&p) => self.set_open(&p, false, cx),
            (RowKey::Item(p, _), false) => {
                self.cursor = Some(RowKey::Product(p));
                cx.notify();
            }
            (RowKey::Group(h, n), right) => {
                let collapsed = data
                    .groups
                    .iter()
                    .any(|g| g.host == h && g.name == n && g.collapsed);
                if collapsed == right {
                    self.toggle_group(h.as_deref(), &n, cx);
                }
            }
            _ => return false,
        }
        true
    }

    /// The rows of one host: its products with no group, then each group with its products.
    /// Each product row is followed by its item rows when it is open.
    #[allow(clippy::too_many_arguments)]
    fn product_rows(
        &mut self,
        host: Option<String>,
        slot: Option<usize>,
        products: &[ProductInfo],
        off: bool,
        first: usize,
        focused: bool,
        cx: &mut Context<Self>,
    ) -> Vec<AnyElement> {
        let Some(core) = self.cores.of(host.as_deref()).cloned() else {
            return Vec::new();
        };
        let data = self.data(cx);
        let (attention, voice_focus, active) = {
            let c = core.read(cx);
            (
                c.products_with_attention(),
                c.focused.clone(),
                self.core.read(cx).active.clone(),
            )
        };
        let feed = core.read(cx).meetings.clone();
        let lists: Vec<Vec<Item>> = products
            .iter()
            .map(|p| {
                let meetings = feed.as_ref().map(|f| &f.read(cx).0);
                tree::items(core.read(cx), meetings, &data, &p.id)
            })
            .collect();
        let arranged = data.arrange(host.as_deref(), products);
        let numbers: Vec<usize> = arranged.flat();
        let scope = slot.map_or_else(String::new, |i| format!("host-{i}-"));
        let mut out = Vec::new();
        let one = |this: &mut Self,
                   i: usize,
                   group: &str,
                   cx: &mut Context<Self>|
         -> Vec<AnyElement> {
            let p = &products[i];
            let list = &lists[i];
            let product = ProductRef {
                host: host.clone(),
                id: p.id.clone(),
            };
            let style = data.product(host.as_deref(), &p.id);
            let open = this.is_open(&product);
            let n = first + numbers.iter().position(|x| *x == i).unwrap_or(i);
            let running = tree::running(list);
            let trailing: AnyElement = if off {
                caption("device off").into_any_element()
            } else {
                let mark: Option<AnyElement> = if attention.contains(&p.id) {
                    Some(
                        div()
                            .child(AttentionMarker::new("").compact())
                            .children(
                                (host.is_none())
                                    .then(|| {
                                        this.probe.as_ref().map(|pr| pr.element(Marker::Product))
                                    })
                                    .flatten(),
                            )
                            .into_any_element(),
                    )
                } else {
                    None
                };
                let chip = (!open && running > 0).then(|| {
                    div()
                        .debug_selector({
                            let id = format!("running-chip:{scope}{}", p.id);
                            move || id
                        })
                        .child(running_chip(running))
                });
                let hint = (mark.is_none() && chip.is_none())
                    .then(|| product_hint(n, cx))
                    .flatten();
                div()
                    .flex()
                    .items_center()
                    .gap(space(2))
                    .children(chip)
                    .children(mark)
                    .children(hint)
                    .into_any_element()
            };
            let id = format!("{scope}product-{}", p.id);
            let key = RowKey::Product(product.clone());
            let (click, toggle) = (product.clone(), product.clone());
            let (click_key, toggle_key) = (key.clone(), key.clone());
            let dragged = DraggedProduct {
                product: product.clone(),
                name: p.name.clone().into(),
            };
            let drop = this.on_drop(host.clone(), group.to_string(), cx);
            let menu = this.on_menu(key.clone(), cx);
            let mut row = ProductRow::new(ElementId::Name(id.clone().into()), p.name.clone())
                .decorate(move |el| {
                    drop(el.on_drag(dragged, |d, _, _, cx| cx.new(|_| d.clone())))
                        .on_mouse_down(MouseButton::Right, menu)
                })
                .label(style.label.map(SharedString::from))
                .hue(style.hue)
                .open(open)
                .selected(active.as_ref() == Some(&product))
                .focused(focused && this.cursor.as_ref() == Some(&key))
                .trailing(trailing)
                .on_click(cx.listener(move |this, _, _, cx| {
                    this.cursor = Some(click_key.clone());
                    cx.emit(SelectProduct(click.clone()));
                }))
                .on_toggle(cx.listener(move |this, _, window, cx| {
                    this.cursor = Some(toggle_key.clone());
                    window.focus(&this.focus, cx);
                    let now = this.is_open(&toggle);
                    this.set_open(&toggle, !now, cx);
                }));
            if let Some(path) = &p.path {
                row = row.meta(path.clone());
            }
            this.stops.push(key.clone());
            let mut rows = vec![this.placed(key, row)];
            if open {
                for item in list {
                    let key = RowKey::Item(product.clone(), item.key.clone());
                    let click_key = key.clone();
                    let (owner, which) = (product.clone(), item.key.clone());
                    let is_focused =
                        matches!(&item.key, ItemKey::Pane(t) if voice_focus.as_deref() == Some(t));
                    let menu = this.on_menu(key.clone(), cx);
                    let row = ItemRow::new(
                        ElementId::Name(format!("{id}-{}", item.key.slug()).into()),
                        item.title.clone(),
                    )
                    .decorate(move |el| el.on_mouse_down(MouseButton::Right, menu))
                    .leading(StatusDot(item.status))
                    .meta(item.meta.clone())
                    .hue(item.hue)
                    .tooltip(item.tooltip.clone().map(SharedString::from))
                    .selected(is_focused)
                    .focused(focused && this.cursor.as_ref() == Some(&key))
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.cursor = Some(click_key.clone());
                        cx.emit(OpenItem {
                            product: owner.clone(),
                            item: which.clone(),
                        });
                    }));
                    this.stops.push(key.clone());
                    rows.push(this.placed(key, row));
                }
            }
            rows
        };
        for &i in &arranged.top {
            out.extend(one(self, i, "", cx));
        }
        for g in &arranged.groups {
            let key = RowKey::Group(host.clone(), g.name.clone());
            let (name, group_host) = (g.name.clone(), host.clone());
            let gid = format!("{scope}group-{}", g.name);
            let drop = self.on_drop(host.clone(), g.name.clone(), cx);
            let menu = self.on_menu(key.clone(), cx);
            let header = GroupHeader::new(ElementId::Name(gid.into()), g.name.clone())
                .decorate(move |el| drop(el).on_mouse_down(MouseButton::Right, menu))
                .hue(g.hue)
                .collapsed(g.collapsed)
                .count(g.members.len())
                .focused(focused && self.cursor.as_ref() == Some(&key))
                .on_click({
                    let key = key.clone();
                    cx.listener(move |this, _, _, cx| {
                        this.cursor = Some(key.clone());
                        this.toggle_group(group_host.as_deref(), &name, cx);
                    })
                });
            self.stops.push(key.clone());
            out.push(self.placed(key, header));
            if !g.collapsed {
                let rows: Vec<AnyElement> = g
                    .members
                    .iter()
                    .flat_map(|&i| one(self, i, &g.name, cx))
                    .collect();
                if !rows.is_empty() {
                    out.push(
                        div()
                            .flex()
                            .flex_col()
                            .gap(space(1))
                            .pl(space(3))
                            .children(rows)
                            .into_any_element(),
                    );
                }
            }
        }
        out
    }

    /// The group of one SSH host. `first` is the number of its first product among all the
    /// products, for the key hints.
    fn remote_group(
        &mut self,
        i: usize,
        h: &RemoteHost,
        first: usize,
        focused: bool,
        cx: &mut Context<Self>,
    ) -> Div {
        let off = matches!(h.state, HostState::Off(_));
        let rows = self.product_rows(
            Some(h.name.clone()),
            Some(i),
            &h.products,
            off,
            first,
            focused,
            cx,
        );
        let empty = h.products.is_empty();
        let header = host_header(&h.state, h.name.clone());
        let header = self.host_drop(format!("host-header-{i}"), Some(h.name.clone()), header, cx);
        div()
            .flex()
            .flex_col()
            .gap(space(1))
            .child(header)
            .children(rows)
            .when(empty, |el| {
                el.child(div().pad_x(space(2)).child(caption(match &h.state {
                    HostState::Up => "No products on this host.",
                    _ => "No products known yet.",
                })))
            })
    }
}

/// The key hint of the product action `n`, with the key in effect.
fn product_hint(n: usize, cx: &App) -> Option<Kbd> {
    product_id(n).map(|id| keys::hint(cx, id))
}

fn caption(s: impl Into<SharedString>) -> Div {
    div()
        .text_size(size::xs())
        .font_weight(weight::MEDIUM)
        .text_color(text::TERTIARY)
        .child(s.into())
}

/// The status dot and the word of a host state.
pub fn host_status(state: &HostState) -> (Status, &'static str) {
    let status = match state {
        HostState::Connecting => Status::Info,
        HostState::Up => Status::Ok,
        HostState::Off(_) => Status::Off,
    };
    (status, state.word())
}

/// The header line of one host group.
fn host_header(state: &HostState, name: impl Into<SharedString>) -> Div {
    let (status, word) = host_status(state);
    div()
        .flex()
        .items_center()
        .gap(space(2))
        .pad_x(space(2))
        .py(space(2))
        .child(StatusDot(status))
        .child(
            div()
                .text_size(size::sm())
                .font_weight(weight::MEDIUM)
                .text_color(text::SECONDARY)
                .child(name.into()),
        )
        .child(caption(word))
}

impl Render for Sidebar {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let focused = self.focus.is_focused(window);
        self.stops.clear();
        self.placed.borrow_mut().clear();
        let hosts: Vec<RemoteHost> = self
            .hosts
            .as_ref()
            .map(|h| h.read(cx).hosts.clone())
            .unwrap_or_default();
        let look_note = self.look.as_ref().and_then(|l| l.read(cx).note.clone());
        let (products, host_state, error, notices) = {
            let core = self.core.read(cx);
            (
                core.products.clone(),
                core.host.clone(),
                core.products_error.clone(),
                core.notices
                    .iter()
                    .cloned()
                    .chain(look_note)
                    .collect::<Vec<_>>(),
            )
        };
        let rows = self.product_rows(None, None, &products, false, 0, focused, cx);
        let local_header = host_header(&host_state, "This device");
        let local_header = self.host_drop("host-header-local".into(), None, local_header, cx);
        let mut first = products.len();
        let mut remote = Vec::new();
        for (i, h) in hosts.iter().enumerate() {
            remote.push(self.remote_group(i, h, first, focused, cx));
            first += h.products.len();
        }
        let empty = rows.is_empty();
        let host_off = match &host_state {
            HostState::Off(why) => Some(why.clone()),
            _ => None,
        };
        div()
            .debug_selector(|| "sidebar".into())
            .flex()
            .flex_col()
            .flex_none()
            .w(control::SIDEBAR_WIDTH)
            .h_full()
            .bg(surface::BASE)
            .border_r_1()
            .border_color(line::BORDER)
            .child(
                div()
                    .debug_selector(|| "sidebar-header".into())
                    .flex()
                    .items_center()
                    .justify_between()
                    .h(control::TOOLBAR_HEIGHT)
                    .pad_x(space(4))
                    .border_b_1()
                    .border_color(line::BORDER)
                    .child(
                        div()
                            .text_size(size::md())
                            .font_weight(weight::SEMIBOLD)
                            .text_color(text::PRIMARY)
                            .child("AgentSI"),
                    )
                    .child(keys::hint(cx, keys::PALETTE)),
            )
            .child(
                div()
                    .id("products")
                    .track_focus(&self.focus)
                    .key_context("Sidebar")
                    .on_key_down(cx.listener(|this, e: &KeyDownEvent, _, cx| this.key_down(e, cx)))
                    .flex()
                    .flex_col()
                    .flex_1()
                    .min_h_0()
                    .overflow_y_scroll()
                    .gap(space(1))
                    .p(space(2))
                    .child(local_header)
                    .children(rows)
                    .when(empty, |el| {
                        el.child(div().pad_x(space(2)).child(caption(
                            error.unwrap_or_else(|| "No products in the store yet.".into()),
                        )))
                    })
                    .children(remote),
            )
            .when(host_off.is_some() || !notices.is_empty(), |el| {
                el.child(
                    div()
                        .flex()
                        .flex_col()
                        .gap(space(2))
                        .p(space(3))
                        .border_t_1()
                        .border_color(line::BORDER)
                        .when_some(host_off, |el, why| el.child(caption(why)))
                        .children(notices.into_iter().map(caption)),
                )
            })
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use agentsi_proto::desk_api::ProductInfo;
    use agentsi_ui::appearance::{Appearance, INK_EM};
    use agentsi_ui::tokens::{control, size};
    use gpui::{Bounds, Pixels, TestAppContext, VisualTestContext};
    use gpui_kit::AppContext as _;

    use super::Sidebar;
    use crate::link::Link;
    use crate::model::{CoreState, Cores};

    fn bounds(cx: &mut VisualTestContext, selector: &str) -> Bounds<Pixels> {
        // The tests are short-lived, so a leaked name costs nothing.
        let name: &'static str = Box::leak(selector.to_string().into_boxed_str());
        cx.debug_bounds(name)
            .unwrap_or_else(|| panic!("{selector} is drawn"))
    }

    fn product(id: &str, name: &str, path: Option<&str>) -> ProductInfo {
        ProductInfo {
            id: id.into(),
            name: name.into(),
            path: path.map(Into::into),
            archived: false,
        }
    }

    /// A sidebar with three products in a window of the app size. The core end must stay open.
    fn fixture(
        cx: &mut TestAppContext,
    ) -> (std::os::unix::net::UnixStream, &mut VisualTestContext) {
        cx.update(agentsi_ui::init);
        let (end, core_end) = std::os::unix::net::UnixStream::pair().unwrap();
        let core = cx.new(|_| CoreState::new(Arc::new(Link::for_tests(end))));
        core.update(cx, |s, cx| {
            s.set_products(
                vec![
                    product("a", "agentsi", Some("~/dev/personal/productos/agentsi")),
                    product("b", "A product with a rather long name", None),
                    product("c", "third", Some("~/dev/third")),
                ],
                cx,
            )
        });
        let (_, cx) = cx.add_window_view(|_, cx| Sidebar::new(Cores::new(core), None, cx));
        cx.simulate_resize(gpui::size(control::WINDOW_WIDTH, control::WINDOW_HEIGHT));
        (core_end, cx)
    }

    #[gpui::test]
    fn the_default_look_lays_out_the_sidebar_as_before_the_appearance_setting(
        cx: &mut TestAppContext,
    ) {
        let (_core_end, cx) = fixture(cx);
        cx.update(|window, cx| window.draw(cx).clear(cx));
        // x, y, width and height, measured with the arrow of the tree (N5).
        let before = [
            ("sidebar", [0., 0., 248., 820.]),
            ("sidebar-header", [0., 0., 247., 48.]),
            ("tree-row:product-a", [8., 97., 231., 44.]),
            ("tree-row:product-b", [8., 145., 231., 44.]),
            ("tree-row:product-c", [8., 193., 231., 44.]),
            ("tree-row:product-a:text", [46., 98.5, 115., 40.5]),
            ("tree-row:product-b:text", [46., 156.5, 115., 21.]),
            ("tree-row:product-c:text", [46., 194.5, 115., 40.5]),
        ];
        for (name, want) in before {
            let b = bounds(cx, name);
            let got = [
                f32::from(b.origin.x),
                f32::from(b.origin.y),
                f32::from(b.size.width),
                f32::from(b.size.height),
            ];
            assert_eq!(got, want, "{name}");
        }
    }

    #[gpui::test]
    fn the_sidebar_lays_out_in_each_of_the_27_combinations(cx: &mut TestAppContext) {
        let (_core_end, cx) = fixture(cx);
        let mut heights = std::collections::BTreeSet::new();
        for a in Appearance::all() {
            cx.update(|window, cx| {
                a.apply(cx);
                window.draw(cx).clear(cx);
            });
            let ctx = format!("{a:?}");
            let side = bounds(cx, "sidebar");
            assert_eq!(side.size.width, control::SIDEBAR_WIDTH, "{ctx}");
            let header = bounds(cx, "sidebar-header");
            assert_eq!(header.size.height, control::TOOLBAR_HEIGHT, "{ctx}");
            let title_ink = f32::from(size::md()) * INK_EM;
            assert!(title_ink <= f32::from(header.size.height), "{ctx}");
            let rows: Vec<_> = ["a", "b", "c"]
                .iter()
                .map(|id| bounds(cx, &format!("tree-row:product-{id}")))
                .collect();
            heights.insert(f32::from(rows[0].size.height).to_bits());
            for (i, r) in rows.iter().enumerate() {
                assert_eq!(r.size.height, control::row(), "{ctx}: row {i}");
                assert!(
                    r.left() >= side.left() && r.right() <= side.right(),
                    "{ctx}: row {i} is cut at the side"
                );
                assert!(
                    r.top() >= header.bottom(),
                    "{ctx}: row {i} is under the header"
                );
                assert!(
                    r.bottom() <= side.bottom(),
                    "{ctx}: row {i} is cut at the bottom"
                );
            }
            for pair in rows.windows(2) {
                assert!(pair[0].bottom() <= pair[1].top(), "{ctx}: two rows overlap");
            }
            // The text of a row with a path keeps two lines inside the row.
            let text = bounds(cx, "tree-row:product-a:text");
            let ink = (f32::from(size::sm()) + f32::from(size::xs())) * INK_EM;
            let mid = f32::from(text.top() + text.size.height / 2.);
            assert!(mid - ink / 2. >= f32::from(rows[0].top()), "{ctx}");
            assert!(mid + ink / 2. <= f32::from(rows[0].bottom()), "{ctx}");
        }
        assert_eq!(heights.len(), 3, "the rows followed the density");
    }

    mod tree_tests {
        use std::cell::RefCell;
        use std::collections::BTreeMap;
        use std::os::unix::net::UnixStream;
        use std::rc::Rc;
        use std::sync::Arc;

        use agentsi_proto::Event as _;
        use agentsi_proto::desk_api::ProductInfo;
        use agentsi_proto::meeting_api::{MeetingProgress, MeetingProgressParams};
        use agentsi_proto::rpc::{Notification, V2};
        use agentsi_proto::terminal_api::TerminalInfo;
        use agentsi_ui::tokens::control;
        use agentsi_ui::tokens::label::Hue;
        use gpui::{Entity, Modifiers, Pixels, Point, TestAppContext, VisualTestContext};
        use gpui_kit::AppContext as _;
        use serde_json::json;

        use crate::link::Link;
        use crate::look::{self, Look};
        use crate::model::{CoreState, Cores, ProductRef, StorySessionItem};
        use crate::screens::meeting::{MeetingFeed, Meetings};
        use crate::sidebar::{OpenItem, SelectProduct, Sidebar, TreeChanged};
        use crate::tree::ItemKey;

        fn product(id: &str) -> ProductInfo {
            ProductInfo {
                id: id.into(),
                name: id.into(),
                path: None,
                archived: false,
            }
        }

        fn terminal(id: &str, product: &str, title: &str) -> TerminalInfo {
            serde_json::from_value(json!({
                "id": id, "argv": ["sh"], "cwd": "/", "product": product, "pid": 1,
                "cols": 80, "rows": 24, "title": title, "state": "working",
                "running": true, "code": null,
            }))
            .unwrap()
        }

        fn progress(stage: &str, note: Option<&str>) -> Notification {
            let p = MeetingProgressParams {
                meeting: "m1".into(),
                product: "a".into(),
                stage: stage.into(),
                phase: "begin".into(),
                note: note.map(Into::into),
                ..MeetingProgressParams::default()
            };
            Notification {
                jsonrpc: V2,
                method: MeetingProgress::NAME.into(),
                params: serde_json::to_value(p).unwrap(),
            }
        }

        /// What the sidebar told the shell.
        #[derive(Debug, Clone, PartialEq)]
        enum Told {
            Select(ProductRef),
            Item(ProductRef, ItemKey),
            Tree,
        }

        struct Tree {
            core: Entity<CoreState>,
            look: Entity<Look>,
            sidebar: Entity<Sidebar>,
            told: Rc<RefCell<Vec<Told>>>,
            file: std::path::PathBuf,
            _end: UnixStream,
        }

        fn tempfile(name: &str) -> std::path::PathBuf {
            let dir =
                std::env::temp_dir().join(format!("agentsi-sidebar-{name}-{}", std::process::id()));
            std::fs::create_dir_all(&dir).unwrap();
            dir.join("look.toml")
        }

        /// A sidebar over a core with: product `a` with two panes, a running meeting, a story
        /// session and a reflection, and product `b` with one pane of a closed tab.
        fn tree<'a>(cx: &'a mut TestAppContext, name: &str) -> (Tree, &'a mut VisualTestContext) {
            cx.update(agentsi_ui::init);
            let file = tempfile(name);
            let _ = std::fs::remove_file(&file);
            let look = cx.update(|cx| look::install(cx, Some(file.clone())));
            let (end, core_end) = UnixStream::pair().unwrap();
            let feed = cx.new(|_| MeetingFeed(Meetings::default()));
            let core = cx.new(|_| CoreState {
                meetings: Some(feed.clone()),
                ..CoreState::new(Arc::new(Link::for_tests(end)))
            });
            feed.update(cx, |f, _| {
                f.0.apply(&progress("start", Some("What next?")));
                f.0.apply(&progress("role", None));
            });
            core.update(cx, |s, cx| {
                s.set_products(vec![product("a"), product("b")], cx);
                s.put(terminal("t1-a", "a", "claude"));
                s.put(terminal("t2-a", "a", "zsh"));
                s.put(terminal("t3-a", "b", "zsh"));
                s.set_sessions(
                    "a",
                    vec![StorySessionItem {
                        id: "S1".into(),
                        title: "Login".into(),
                        state: "work".into(),
                    }],
                );
                s.set_reflecting("a", true);
            });
            let c = core.clone();
            let (sidebar, cx) =
                cx.add_window_view(move |_, cx| Sidebar::new(Cores::new(c), None, cx));
            cx.simulate_resize(gpui::size(control::WINDOW_WIDTH, control::WINDOW_HEIGHT));
            let told: Rc<RefCell<Vec<Told>>> = Rc::default();
            let t = told.clone();
            cx.update(|_, cx| {
                cx.subscribe(&sidebar, {
                    let t = t.clone();
                    move |_, e: &SelectProduct, _| t.borrow_mut().push(Told::Select(e.0.clone()))
                })
                .detach();
                cx.subscribe(&sidebar, {
                    let t = t.clone();
                    move |_, e: &OpenItem, _| {
                        t.borrow_mut()
                            .push(Told::Item(e.product.clone(), e.item.clone()))
                    }
                })
                .detach();
                cx.subscribe(&sidebar, move |_, _: &TreeChanged, _| {
                    t.borrow_mut().push(Told::Tree)
                })
                .detach();
            });
            cx.update(|window, cx| window.draw(cx).clear(cx));
            (
                Tree {
                    core,
                    look,
                    sidebar,
                    told,
                    file,
                    _end: core_end,
                },
                cx,
            )
        }

        fn redraw(cx: &mut VisualTestContext) {
            cx.run_until_parked();
            cx.update(|window, cx| window.draw(cx).clear(cx));
        }

        fn at(cx: &mut VisualTestContext, name: &str) -> Option<gpui::Bounds<Pixels>> {
            let name: &'static str = Box::leak(name.to_string().into_boxed_str());
            cx.debug_bounds(name)
        }

        fn click(cx: &mut VisualTestContext, name: &str) {
            let b = at(cx, name).unwrap_or_else(|| panic!("{name} is drawn"));
            let center = Point::new(
                b.origin.x + b.size.width / 2.,
                b.origin.y + b.size.height / 2.,
            );
            cx.simulate_click(center, Modifiers::none());
            redraw(cx);
        }

        fn mine(id: &str) -> ProductRef {
            ProductRef {
                host: None,
                id: id.into(),
            }
        }

        #[gpui::test]
        fn a_pane_a_meeting_a_story_session_and_a_reflection_nest_under_their_product(
            cx: &mut TestAppContext,
        ) {
            let (t, cx) = tree(cx, "nest");
            let a = at(cx, "tree-row:product-a").unwrap();
            let b = at(cx, "tree-row:product-b").unwrap();
            let items = [
                ("product-a-pane-t1-a", ItemKey::Pane("t1-a".into())),
                ("product-a-pane-t2-a", ItemKey::Pane("t2-a".into())),
                ("product-a-meeting", ItemKey::Meeting),
                ("product-a-story-S1", ItemKey::Story("S1".into())),
                ("product-a-reflection", ItemKey::Reflection),
            ];
            let a_title = at(cx, "tree-row:product-a:title").unwrap();
            let mut last = a.bottom();
            for (id, _) in &items {
                let row = at(cx, &format!("tree-row:{id}")).unwrap_or_else(|| panic!("{id}"));
                assert!(row.top() >= last, "{id} comes after the row above it");
                let title = at(cx, &format!("tree-row:{id}:title")).unwrap();
                assert!(
                    title.left() > a_title.left(),
                    "{id} is indented under its product"
                );
                assert!(row.bottom() <= b.top(), "{id} is above the next product");
                last = row.bottom();
            }
            // The pane of product b sits under b.
            let under_b = at(cx, "tree-row:product-b-pane-t3-a").unwrap();
            assert!(under_b.top() >= b.bottom());
            // A click on each item row tells the shell which item to focus.
            for (id, key) in items {
                t.told.borrow_mut().clear();
                click(cx, &format!("tree-row:{id}"));
                assert_eq!(*t.told.borrow(), [Told::Item(mine("a"), key)], "{id}");
            }
            t.told.borrow_mut().clear();
            click(cx, "tree-row:product-b");
            assert_eq!(*t.told.borrow(), [Told::Select(mine("b"))]);
        }

        #[gpui::test]
        fn the_arrow_closes_a_product_and_shows_the_running_count(cx: &mut TestAppContext) {
            let (t, cx) = tree(cx, "arrow");
            assert!(
                at(cx, "running-chip:a").is_none(),
                "an open product has no chip"
            );
            click(cx, "tree-row:product-a:arrow");
            // The click on the arrow does not select the product.
            assert_eq!(*t.told.borrow(), [Told::Tree]);
            assert!(at(cx, "tree-row:product-a-pane-t1-a").is_none());
            assert!(
                at(cx, "running-chip:a").is_some(),
                "a closed product shows its count"
            );
            assert_eq!(
                t.sidebar.read_with(cx, |s, _| s.open_state()),
                BTreeMap::from([("a".to_string(), false)])
            );
            click(cx, "tree-row:product-a:arrow");
            assert!(at(cx, "tree-row:product-a-pane-t1-a").is_some());
            assert!(at(cx, "running-chip:a").is_none());
        }

        #[gpui::test]
        fn the_open_state_comes_back_from_the_saved_layout(cx: &mut TestAppContext) {
            let (t, cx) = tree(cx, "restore");
            t.sidebar.update(cx, |s, cx| {
                s.set_open_state(BTreeMap::from([("a".to_string(), false)]), cx)
            });
            redraw(cx);
            assert!(at(cx, "tree-row:product-a-pane-t1-a").is_none());
            assert!(at(cx, "tree-row:product-b-pane-t3-a").is_some());
            assert!(!t.sidebar.read_with(cx, |s, _| s.is_open(&mine("a"))));
            assert!(t.sidebar.read_with(cx, |s, _| s.is_open(&mine("b"))));
        }

        #[gpui::test]
        fn the_keys_move_a_cursor_open_and_close_a_product_and_select_a_row(
            cx: &mut TestAppContext,
        ) {
            let (t, cx) = tree(cx, "keys");
            let focus = t.sidebar.read_with(cx, |s, _| s.focus.clone());
            cx.update(|window, cx| window.focus(&focus, cx));
            // Product a, then its first pane.
            cx.simulate_keystrokes("down down");
            cx.simulate_keystrokes("enter");
            assert_eq!(
                *t.told.borrow(),
                [Told::Item(mine("a"), ItemKey::Pane("t1-a".into()))]
            );
            // Left goes back to the product, and Left again closes it.
            t.told.borrow_mut().clear();
            cx.simulate_keystrokes("left left");
            assert_eq!(*t.told.borrow(), [Told::Tree]);
            assert!(!t.sidebar.read_with(cx, |s, _| s.is_open(&mine("a"))));
            // Right opens it, and Right again goes into it.
            cx.simulate_keystrokes("right right");
            assert!(t.sidebar.read_with(cx, |s, _| s.is_open(&mine("a"))));
            t.told.borrow_mut().clear();
            cx.simulate_keystrokes("enter");
            assert_eq!(
                *t.told.borrow(),
                [Told::Item(mine("a"), ItemKey::Pane("t1-a".into()))]
            );
            // Home and Enter select the first product.
            t.told.borrow_mut().clear();
            cx.simulate_keystrokes("home enter");
            assert_eq!(*t.told.borrow(), [Told::Select(mine("a"))]);
        }

        #[gpui::test]
        fn a_product_label_and_color_show_on_its_row_and_a_pane_label_on_its_item_row(
            cx: &mut TestAppContext,
        ) {
            let (t, cx) = tree(cx, "label");
            assert!(at(cx, "tree-row:product-a:label").is_none());
            assert!(at(cx, "tree-row:product-a:bar").is_none());
            t.look.update(cx, |l, cx| {
                l.set_product_label("a", "api", cx);
                l.set_product_hue("a", Some(Hue::Teal), cx);
                l.set_pane_label("t1-a", "build", cx);
                l.set_pane_hue("t1-a", Some(Hue::Pink), cx);
            });
            redraw(cx);
            assert!(at(cx, "tree-row:product-a:label").is_some());
            let bar = at(cx, "tree-row:product-a:bar").unwrap();
            assert_eq!(bar.size.width, control::COLOR_BAR);
            let row = at(cx, "tree-row:product-a").unwrap();
            assert!(bar.left() >= row.left() && bar.right() < row.left() + Pixels::from(8.));
            assert!(at(cx, "tree-row:product-a-pane-t1-a:bar").is_some());
            assert!(at(cx, "tree-row:product-a-pane-t2-a:bar").is_none());
            // The file has them.
            let (data, note) = look::read(&t.file);
            assert_eq!(note, None);
            assert_eq!(data.product(None, "a").label.as_deref(), Some("api"));
            assert_eq!(data.product(None, "a").hue, Some(Hue::Teal));
            assert_eq!(data.pane(None, "t1-a").label.as_deref(), Some("build"));
        }

        fn center(cx: &mut VisualTestContext, name: &str) -> Point<Pixels> {
            let b = at(cx, name).unwrap_or_else(|| panic!("{name} is drawn"));
            Point::new(
                b.origin.x + b.size.width / 2.,
                b.origin.y + b.size.height / 2.,
            )
        }

        /// Presses on `from`, moves in steps to `to` and releases there.
        fn drag(cx: &mut VisualTestContext, from: &str, to: &str) {
            let (a, b) = (center(cx, from), center(cx, to));
            cx.simulate_mouse_move(a, None, Modifiers::none());
            cx.simulate_mouse_down(a, gpui::MouseButton::Left, Modifiers::none());
            for step in 1..=5 {
                let t = step as f32 / 5.;
                let p = Point::new(a.x + (b.x - a.x) * t, a.y + (b.y - a.y) * t);
                cx.simulate_mouse_move(p, gpui::MouseButton::Left, Modifiers::none());
            }
            cx.simulate_mouse_up(b, gpui::MouseButton::Left, Modifiers::none());
            redraw(cx);
        }

        #[gpui::test]
        fn a_product_dragged_onto_a_group_joins_it_and_onto_the_host_header_leaves_it(
            cx: &mut TestAppContext,
        ) {
            let (t, cx) = tree(cx, "drag");
            t.look
                .update(cx, |l, cx| l.move_product(None, "a", "Work", cx));
            redraw(cx);
            // Product b has no group. Dropping it on the header of "Work" puts it in.
            drag(cx, "tree-row:product-b", "tree-row:group-Work");
            assert_eq!(look::read(&t.file).0.group_of(None, "b"), Some("Work"));
            assert!(t.told.borrow().is_empty(), "a drag selects nothing");
            // Dropping it on the header of the host takes it out.
            drag(cx, "tree-row:product-b", "host-header-local");
            assert_eq!(look::read(&t.file).0.group_of(None, "b"), None);
            // Dropping it on a product of the group puts it in that group.
            drag(cx, "tree-row:product-b", "tree-row:product-a");
            assert_eq!(look::read(&t.file).0.group_of(None, "b"), Some("Work"));
        }

        #[gpui::test]
        fn a_product_group_holds_its_products_and_collapses(cx: &mut TestAppContext) {
            let (t, cx) = tree(cx, "group");
            t.look
                .update(cx, |l, cx| l.move_product(None, "b", "Work", cx));
            redraw(cx);
            let header = at(cx, "tree-row:group-Work").unwrap();
            let a = at(cx, "tree-row:product-a").unwrap();
            let b = at(cx, "tree-row:product-b").unwrap();
            assert!(
                a.bottom() <= header.top(),
                "a product with no group stays at the top"
            );
            assert!(b.top() >= header.bottom() && b.left() > a.left());
            assert!(at(cx, "tree-row:group-Work:square").is_some());
            click(cx, "tree-row:group-Work");
            assert!(at(cx, "tree-row:product-b").is_none());
            let (data, _) = look::read(&t.file);
            assert!(data.groups[0].collapsed);
            click(cx, "tree-row:group-Work");
            assert!(at(cx, "tree-row:product-b").is_some());
            let _ = &t.core;
        }
    }
}
