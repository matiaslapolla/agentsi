//! What the app knows about each core: the products, the terminals with their program
//! state, and voice mode. Each host has one `CoreState` entity, and `Cores` holds them.
//!
//! A screen or a pane holds the entity of its product's host and reads the link from it.
//! It holds no other core, so a write of a screen cannot reach another host.

use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;

use agentsi_proto::Event as _;
use agentsi_proto::core_api::Empty;
use agentsi_proto::desk_api::{ProductDesk, ProductDeskParams, ProductInfo, ProductList};
use agentsi_proto::events::{
    DataChanged, Lagged, ProgramState, TerminalExit, TerminalState, TerminalStateParams,
    TerminalTitle,
};
use agentsi_proto::rpc::Notification;
use agentsi_proto::story_api::{Story, StoryList, StoryListParams};
use agentsi_proto::terminal_api::{Kill, List, TerminalId, TerminalInfo};
use futures::FutureExt as _;
use futures::future::LocalBoxFuture;
use gpui_kit::{App, AppContext as _, AsyncApp, Context, Entity, EventEmitter};

use crate::alerts::{Alerts, Name};
use crate::link::{CallError, Link};
use crate::screens::meeting::MeetingFeed;

/// The `kind` values of `blocked` that wait for the owner.
const ATTENTION_KINDS: [&str; 3] = ["permission", "question", "auth"];

/// What a terminal shows in the app.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Term {
    pub id: String,
    pub product: Option<String>,
    pub title: String,
    pub state: Option<ProgramState>,
    pub kind: Option<String>,
    pub msg: Option<String>,
    pub running: bool,
    pub code: Option<i32>,
    pub voice: bool,
    /// The folder where the program started.
    pub cwd: String,
}

impl Term {
    pub fn from_info(t: TerminalInfo) -> Term {
        Term {
            id: t.id,
            product: t.product,
            title: t.title,
            state: t.state,
            kind: t.kind,
            msg: t.msg,
            running: t.running,
            code: t.code,
            voice: false,
            cwd: t.cwd,
        }
    }

    /// The text of the attention marker, when the program waits for the owner.
    pub fn attention(&self) -> Option<String> {
        if self.state != Some(ProgramState::Blocked) {
            return None;
        }
        attention_text(self.kind.as_deref()?, self.msg.as_deref())
    }
}

/// The text of a `blocked` state of kind `kind`, or none when the kind does not wait for the owner.
pub fn attention_text(kind: &str, msg: Option<&str>) -> Option<String> {
    if !ATTENTION_KINDS.contains(&kind) {
        return None;
    }
    Some(msg.map(str::to_string).unwrap_or_else(|| match kind {
        "question" => "Claude asks a question".to_string(),
        "auth" => "Claude needs a login".to_string(),
        _ => "Claude asks for a permission".to_string(),
    }))
}

/// A story that has a session, for the sidebar tree.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StorySessionItem {
    pub id: String,
    pub title: String,
    /// `queued`, `tests`, `work`, `checks`, `ready` or `failed`.
    pub state: String,
}

impl StorySessionItem {
    /// The sessions of a story list: a story that is not done and has a session.
    pub fn of(stories: Vec<Story>) -> Vec<StorySessionItem> {
        stories
            .into_iter()
            .filter(|s| s.archived != Some(true) && s.status == "not_implemented")
            .filter_map(|s| {
                let state = s.session?.state;
                Some(StorySessionItem {
                    id: s.id,
                    title: s.title,
                    state,
                })
            })
            .collect()
    }

    /// The session still works: it is neither ready for review nor failed.
    pub fn running(&self) -> bool {
        !matches!(self.state.as_str(), "ready" | "failed")
    }
}

/// One `story.list` call for one product, and the product.
pub type SessionRead = (
    String,
    LocalBoxFuture<'static, Result<Vec<StorySessionItem>, CallError>>,
);

/// What one event did to the state.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Applied {
    Changed,
    Same,
    /// The event names a terminal that the app does not know: another client made it.
    /// Read `terminal.list` again.
    Unknown,
}

/// The state of the link to the core of a host: this device or an SSH host.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum HostState {
    Connecting,
    Up,
    /// The link to the core broke, with the reason.
    Off(String),
}

impl HostState {
    /// The word of the state in the sidebar, the settings screen and the log.
    pub fn word(&self) -> &'static str {
        match self {
            HostState::Connecting => "connecting",
            HostState::Up => "connected",
            HostState::Off(_) => DEVICE_OFF,
        }
    }
}

/// The word of a host that does not answer.
pub const DEVICE_OFF: &str = "device off";

/// One product of one host. A local product and a product of a host can have the same id.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProductRef {
    /// The SSH host, or `None` for this device.
    pub host: Option<String>,
    pub id: String,
}

/// What a core tells the views apart from a redraw.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CoreEvent {
    /// The data of this core changed: its open screens read again.
    DataChanged,
    /// The host answers again on a new link: its screens read again and its panes attach.
    Up,
    /// The product list of this core changed: a product was archived or restored, or the
    /// host came back with another list.
    Products,
}

impl EventEmitter<CoreEvent> for CoreState {}

pub struct CoreState {
    /// The link of this core. A core of an SSH host gets a new link at each connection, so
    /// read it at each call.
    pub link: Arc<Link>,
    /// The name of the SSH host of this core, or `None` for the core of this device.
    pub remote: Option<String>,
    pub host: HostState,
    pub products: Vec<ProductInfo>,
    /// Why the product list is missing, for the sidebar.
    pub products_error: Option<String>,
    pub terminals: BTreeMap<String, Term>,
    /// Notices for the owner, newest last. For example: panes that the app dropped at start.
    pub notices: Vec<String>,
    /// The product of the sidebar selection: `Ctrl+T` opens a terminal in its folder.
    /// Only the state of this device holds it, and it can name a product of any host.
    pub active: Option<ProductRef>,
    /// The terminal of the focused pane, as the app last reported it with `focus.report`.
    pub focused: Option<String>,
    /// The meetings of this core. A meeting belongs to one core, so two cores with the same
    /// product id show two meetings.
    pub meetings: Option<Entity<MeetingFeed>>,
    /// Terminals whose tab the owner closed. The core lists them until the program stops,
    /// so a later `terminal.list` must not add them again.
    pub gone: BTreeSet<String>,
    /// The story sessions that are not done, by product. A story has no progress event, so
    /// `read_sessions` runs again after `data.changed` of kind `stories`.
    pub sessions: BTreeMap<String, Vec<StorySessionItem>>,
    /// The products with a reflection that this window started and that runs. The core has no
    /// list of running reflections, so a reflection that the CLI started does not show here.
    pub reflecting: BTreeSet<String>,
}

impl CoreState {
    pub fn new(link: Arc<Link>) -> CoreState {
        CoreState {
            link,
            remote: None,
            host: HostState::Connecting,
            products: Vec::new(),
            products_error: None,
            terminals: BTreeMap::new(),
            notices: Vec::new(),
            active: None,
            focused: None,
            meetings: None,
            gone: BTreeSet::new(),
            sessions: BTreeMap::new(),
            reflecting: BTreeSet::new(),
        }
    }

    /// One `story.list` call for each of `products`, to hand to `take_sessions`.
    pub fn read_sessions(&self, products: impl IntoIterator<Item = String>) -> Vec<SessionRead> {
        products
            .into_iter()
            .map(|product| {
                let call = self.link.call::<StoryList>(&StoryListParams {
                    product: product.clone(),
                    include_archived: false,
                });
                let read = async move { call.await.map(|r| StorySessionItem::of(r.stories)) };
                (product, read.boxed_local())
            })
            .collect()
    }

    /// Takes the sessions of a product. An empty list drops the entry. Returns true on a change.
    pub fn set_sessions(&mut self, product: &str, items: Vec<StorySessionItem>) -> bool {
        if items.is_empty() {
            return self.sessions.remove(product).is_some();
        }
        self.sessions.insert(product.to_string(), items.clone()) != Some(items)
    }

    /// A reflection of `product` starts or ends. Returns true on a change.
    #[cfg_attr(not(test), allow(dead_code))] // the Reflect entry of N8
    pub fn set_reflecting(&mut self, product: &str, on: bool) -> bool {
        if on {
            self.reflecting.insert(product.to_string())
        } else {
            self.reflecting.remove(product)
        }
    }

    /// The state of the core of SSH host `name`, before its first connection.
    pub fn remote(name: &str, products: Vec<ProductInfo>) -> CoreState {
        CoreState {
            remote: Some(name.to_string()),
            products,
            ..CoreState::new(Link::lost("the host is not connected yet"))
        }
    }

    /// Why the screens of this core cannot read it now. The core of this device has no
    /// such state: without it the app does not start.
    pub fn away(&self) -> Option<(String, Option<String>)> {
        let name = self.remote.as_deref()?;
        match &self.host {
            HostState::Up => None,
            HostState::Connecting => Some((format!("Connecting to {name}…"), None)),
            HostState::Off(why) => {
                Some((format!("{name}: {}", self.host.word()), Some(why.clone())))
            }
        }
    }

    /// The link broke, so no meeting of this core can report its end.
    pub fn meetings_lost(&self, cx: &mut App) {
        if let Some(feed) = &self.meetings {
            feed.update(cx, |m, cx| {
                if m.0.link_lost() {
                    cx.notify();
                }
            });
        }
    }

    /// Takes the terminal list of a new connection. A terminal that the list does not
    /// hold ended with the old core. Returns the terminals whose tab the owner closed while
    /// the host was off: they still run, and the caller stops them.
    pub fn sync(&mut self, list: Vec<TerminalInfo>) -> Vec<String> {
        let (closed, list): (Vec<_>, Vec<_>) =
            list.into_iter().partition(|t| self.gone.contains(&t.id));
        let listed: BTreeSet<String> = list.iter().map(|t| t.id.clone()).collect();
        self.terminals.retain(|id, _| listed.contains(id));
        for info in list {
            self.put(info);
        }
        closed.into_iter().map(|t| t.id).collect()
    }

    pub fn product(&self, id: &str) -> Option<&ProductInfo> {
        self.products.iter().find(|p| p.id == id)
    }

    /// The products that have a terminal that waits for the owner.
    pub fn products_with_attention(&self) -> BTreeSet<String> {
        self.terminals
            .values()
            .filter(|t| t.attention().is_some())
            .filter_map(|t| t.product.clone())
            .collect()
    }

    /// Takes one event of the core.
    pub fn apply(&mut self, n: &Notification) -> Applied {
        if let Some(p) = n.parse::<TerminalState>() {
            return self.set_state(p);
        }
        if let Some(p) = n.parse::<TerminalTitle>() {
            return match self.terminals.get_mut(&p.id) {
                Some(t) if t.title != p.title => {
                    t.title = p.title;
                    Applied::Changed
                }
                Some(_) => Applied::Same,
                None => Applied::Unknown,
            };
        }
        if let Some(p) = n.parse::<TerminalExit>() {
            return match self.terminals.get_mut(&p.id) {
                Some(t) => {
                    t.running = false;
                    t.code = p.code;
                    Applied::Changed
                }
                None => Applied::Unknown,
            };
        }
        Applied::Same
    }

    fn set_state(&mut self, p: TerminalStateParams) -> Applied {
        let Some(t) = self.terminals.get_mut(&p.id) else {
            return Applied::Unknown;
        };
        t.state = Some(p.state);
        t.kind = p.kind;
        t.msg = p.msg;
        Applied::Changed
    }

    /// The owner closed the tab of `id`: the app forgets the terminal for good.
    pub fn close(&mut self, id: &str) {
        self.terminals.remove(id);
        self.gone.insert(id.to_string());
    }

    /// Takes a new product list, without the archived products, and tells the views.
    pub fn set_products(&mut self, products: Vec<ProductInfo>, cx: &mut Context<CoreState>) {
        self.products = listed(products);
        self.products_error = None;
        cx.emit(CoreEvent::Products);
        cx.notify();
    }

    /// Adds or replaces a terminal from `terminal.create` or `terminal.list`. Keeps its voice
    /// mode. A terminal whose tab the owner closed stays out.
    pub fn put(&mut self, info: TerminalInfo) {
        if self.gone.contains(&info.id) {
            return;
        }
        let voice = self.terminals.get(&info.id).is_some_and(|t| t.voice);
        let mut t = Term::from_info(info);
        t.voice = voice;
        self.terminals.insert(t.id.clone(), t);
    }
}

/// The core of each host. The order of `remote` is the order of `hosts.toml`.
#[derive(Clone)]
pub struct Cores {
    pub local: Entity<CoreState>,
    pub remote: Vec<(String, Entity<CoreState>)>,
}

impl Cores {
    pub fn new(local: Entity<CoreState>) -> Cores {
        Cores {
            local,
            remote: Vec::new(),
        }
    }

    /// The core of `host`, or `None` for a host that `hosts.toml` does not list. A caller
    /// never takes the local core in place of a missing host.
    pub fn of(&self, host: Option<&str>) -> Option<&Entity<CoreState>> {
        match host {
            None => Some(&self.local),
            Some(name) => self.remote.iter().find(|(n, _)| n == name).map(|(_, c)| c),
        }
    }

    pub fn all(&self) -> impl Iterator<Item = &Entity<CoreState>> {
        std::iter::once(&self.local).chain(self.remote.iter().map(|(_, c)| c))
    }

    /// The active product after a change of the lists: `active` while its core lists it, else
    /// the first product of the sidebar, else `None`. A call with no active product gives none.
    pub fn keep_active(&self, active: Option<&ProductRef>, cx: &App) -> Option<ProductRef> {
        let active = active?;
        let listed = |p: &ProductRef| {
            self.of(p.host.as_deref())
                .is_some_and(|c| c.read(cx).product(&p.id).is_some())
        };
        if listed(active) {
            return Some(active.clone());
        }
        self.all().find_map(|c| {
            let core = c.read(cx);
            core.products.first().map(|p| ProductRef {
                host: core.remote.clone(),
                id: p.id.clone(),
            })
        })
    }

    pub fn names(&self) -> BTreeSet<String> {
        self.remote.iter().map(|(n, _)| n.clone()).collect()
    }

    /// The core of `host` for a panel of the saved layout. A host that left `hosts.toml`
    /// gets a core of its own that stays off.
    pub fn for_panel(&self, host: Option<&str>, cx: &mut App) -> Entity<CoreState> {
        if let Some(c) = self.of(host) {
            return c.clone();
        }
        let name = host.unwrap_or_default();
        cx.new(|_| {
            let mut s = CoreState::remote(name, Vec::new());
            s.host = HostState::Off("This host is not in hosts.toml.".into());
            s
        })
    }
}

/// Takes one event of a core: `data.changed` and `lagged` make the screens of that core read
/// again, and a terminal that the app does not know reads the list again. The list runs on
/// its own task, so a slow host holds up no other event. A `meeting.*` event goes to the
/// meetings of that core only, so no other view draws.
pub fn take_event(core: &Entity<CoreState>, n: &Notification, cx: &mut AsyncApp) {
    raise_alerts(core, n, cx);
    let meeting = n.method.starts_with("meeting.");
    // An archive, a restore or a clear of a product changes the list of the sidebar.
    let products = (n.method == DataChanged::NAME
        && matches!(
            n.params.get("kind").and_then(|k| k.as_str()),
            Some("product" | "clear")
        ))
    .then(|| core.read_with(cx, |s, _| s.link.call::<ProductList>(&Empty {})));
    if let Some(call) = products {
        let core = core.clone();
        cx.spawn(async move |cx| {
            if let Ok(list) = call.await {
                core.update(cx, |s, cx| s.set_products(list.products, cx));
            }
        })
        .detach();
    }
    let sessions = core.read_with(cx, |s, _| {
        if n.method == Lagged::NAME {
            let all = s.products.iter().map(|p| p.id.clone()).collect::<Vec<_>>();
            return s.read_sessions(all);
        }
        let stories = n.method == DataChanged::NAME
            && n.params.get("kind").and_then(|k| k.as_str()) == Some("stories");
        match n.params.get("product").and_then(|p| p.as_str()) {
            Some(product) if stories => s.read_sessions([product.to_string()]),
            _ => Vec::new(),
        }
    });
    take_sessions(core, sessions, cx);
    let decisions = n.method == DataChanged::NAME
        && matches!(
            n.params.get("kind").and_then(|k| k.as_str()),
            Some("decision" | "minutes")
        );
    if decisions && let Some(product) = n.params.get("product").and_then(|p| p.as_str()) {
        read_desk(core, product, cx);
    }
    let refresh = core.update(cx, |s, cx| {
        if (meeting || n.method == Lagged::NAME)
            && let Some(feed) = &s.meetings
        {
            feed.update(cx, |m, cx| {
                if m.0.apply(n) {
                    cx.notify();
                }
            });
        }
        if meeting {
            return None;
        }
        if n.method == DataChanged::NAME || n.method == Lagged::NAME {
            cx.emit(CoreEvent::DataChanged);
        }
        match s.apply(n) {
            Applied::Changed => {
                cx.notify();
                None
            }
            Applied::Same => None,
            Applied::Unknown => Some(s.link.call::<List>(&Empty {})),
        }
    });
    let Some(call) = refresh else {
        return;
    };
    let core = core.clone();
    cx.spawn(async move |cx| {
        if let Ok(list) = call.await {
            core.update(cx, |s, cx| {
                for info in list.terminals {
                    s.put(info);
                }
                cx.notify();
            });
        }
    })
    .detach();
}

/// Gives one event of a core to the alert center.
pub fn raise_alerts(core: &Entity<CoreState>, n: &Notification, cx: &mut AsyncApp) {
    if let Some(chime) = cx.update(|cx| {
        cx.try_global::<crate::chime::ChimeGlobal>()
            .map(|g| g.0.clone())
    }) {
        chime.borrow_mut().hear(std::time::Instant::now(), n);
    }
    let Some(center) = cx.update(|cx| Alerts::try_get(cx)) else {
        return;
    };
    let (host, name) = core.read_with(cx, |s, _| {
        let term = n
            .params
            .get("id")
            .and_then(|i| i.as_str())
            .and_then(|id| s.terminals.get(id));
        let name = Name {
            title: term.map(|t| t.title.clone()).unwrap_or_default(),
            product: term.and_then(|t| t.product.clone()),
        };
        (s.remote.clone(), name)
    });
    center.update(cx, |c, cx| {
        c.take(host.as_deref(), n, &|_| name.clone(), cx)
    });
}

/// Reads the pending decisions of `product` and tells the alert center.
fn read_desk(core: &Entity<CoreState>, product: &str, cx: &mut AsyncApp) {
    let Some(center) = cx.update(|cx| Alerts::try_get(cx)) else {
        return;
    };
    let (host, call) = core.read_with(cx, |s, _| {
        (
            s.remote.clone(),
            s.link.call::<ProductDesk>(&ProductDeskParams {
                product: product.to_string(),
            }),
        )
    });
    let product = product.to_string();
    cx.spawn(async move |cx| {
        if let Ok(desk) = call.await {
            center.update(cx, |c, cx| {
                c.take_desk(host.as_deref(), &product, &desk.pending_decisions, cx)
            });
        }
    })
    .detach();
}

/// Takes the answers of `read_sessions`. A product for which stories mode is off, or that the
/// core does not know, has no sessions. A broken link keeps what the app knows.
pub fn take_sessions(core: &Entity<CoreState>, reads: Vec<SessionRead>, cx: &mut AsyncApp) {
    if reads.is_empty() {
        return;
    }
    let core = core.clone();
    cx.spawn(async move |cx| {
        for (product, read) in reads {
            let items = match read.await {
                Ok(items) => items,
                Err(CallError::Rpc(_)) => Vec::new(),
                Err(CallError::Link(_)) => continue,
            };
            let host = core.update(cx, |s, cx| {
                if s.set_sessions(&product, items.clone()) {
                    cx.notify();
                }
                s.remote.clone()
            });
            if let Some(center) = cx.update(|cx| Alerts::try_get(cx)) {
                center.update(cx, |c, cx| {
                    c.take_stories(host.as_deref(), &product, &items, cx)
                });
            }
        }
    })
    .detach();
}

/// The host answers again: the views read its core, and its terminal list is read again.
pub fn take_up(core: &Entity<CoreState>, cx: &mut AsyncApp) {
    let (call, sessions) = core.update(cx, |s, cx| {
        cx.emit(CoreEvent::Up);
        let all = s.products.iter().map(|p| p.id.clone()).collect::<Vec<_>>();
        (s.link.call::<List>(&Empty {}), s.read_sessions(all))
    });
    take_sessions(core, sessions, cx);
    let core = core.clone();
    cx.spawn(async move |cx| {
        if let Ok(list) = call.await {
            core.update(cx, |s, cx| {
                for id in s.sync(list.terminals) {
                    drop(s.link.call::<Kill>(&TerminalId { id }));
                }
                cx.notify();
            });
        }
    })
    .detach();
}

/// Two cores that both hold product `demo`, for the tests of the views.
/// The products that the sidebar lists. An archived product stays out, as on the web page.
/// A core that sends no `archived` field has no archived product.
pub fn listed(products: Vec<ProductInfo>) -> Vec<ProductInfo> {
    products.into_iter().filter(|p| !p.archived).collect()
}

#[cfg(test)]
pub(crate) mod two_cores {
    use std::os::unix::net::UnixStream;

    use gpui::TestAppContext;

    use super::*;

    pub(crate) struct TwoCores {
        pub cores: Cores,
        /// The sockets of the two cores: what a view sent to each one.
        pub local: UnixStream,
        pub studio: UnixStream,
    }

    fn state(remote: Option<&str>, end: UnixStream) -> CoreState {
        let mut s = CoreState::new(Arc::new(Link::for_tests(end)));
        s.remote = remote.map(String::from);
        s.host = HostState::Up;
        s.products = vec![ProductInfo {
            id: "demo".into(),
            name: "Demo".into(),
            path: None,
            archived: false,
        }];
        s
    }

    pub(crate) fn two_cores(cx: &mut TestAppContext) -> TwoCores {
        let (a, local) = UnixStream::pair().unwrap();
        let (b, studio) = UnixStream::pair().unwrap();
        local.set_nonblocking(true).unwrap();
        studio.set_nonblocking(true).unwrap();
        let mut cores = Cores::new(cx.new(|_| state(None, a)));
        let remote = cx.new(|_| state(Some("studio"), b));
        cores.remote.push(("studio".into(), remote));
        TwoCores {
            cores,
            local,
            studio,
        }
    }

    /// Each method that the core on `socket` received since the last read.
    pub(crate) fn received(socket: &mut UnixStream) -> Vec<serde_json::Value> {
        let mut out = Vec::new();
        while let Ok(Some(frame)) = agentsi_proto::frame::read_frame(socket) {
            out.push(serde_json::from_slice(&frame.payload).unwrap());
        }
        out
    }

    pub(crate) fn methods(socket: &mut UnixStream) -> Vec<String> {
        received(socket)
            .iter()
            .map(|r| r["method"].as_str().unwrap_or_default().to_string())
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::two_cores::{methods, two_cores};
    use super::*;
    use gpui::TestAppContext;
    use serde_json::json;

    fn info(id: &str, product: Option<&str>) -> TerminalInfo {
        serde_json::from_value(json!({
            "id": id, "argv": ["sh"], "cwd": "/", "product": product, "pid": 1,
            "cols": 80, "rows": 24, "title": "", "state": null, "running": true, "code": null,
        }))
        .unwrap()
    }

    fn note(method: &str, params: serde_json::Value) -> Notification {
        serde_json::from_value(json!({"jsonrpc": "2.0", "method": method, "params": params}))
            .unwrap()
    }

    #[test]
    fn the_sidebar_lists_no_archived_product_and_a_missing_field_means_not_archived() {
        let from_core: agentsi_proto::desk_api::ProductListResult = serde_json::from_value(json!({
            "products": [
                {"id": "old", "name": "Old", "path": null, "archived": true},
                {"id": "live", "name": "Live", "path": null},
                {"id": "kept", "name": "Kept", "path": "/k", "archived": false},
            ]
        }))
        .unwrap();
        let ids: Vec<_> = listed(from_core.products)
            .into_iter()
            .map(|p| p.id)
            .collect();
        assert_eq!(ids, ["live", "kept"]);
    }

    fn state() -> CoreState {
        let (a, _b) = std::os::unix::net::UnixStream::pair().unwrap();
        // The link is never used in these tests: a socket pair with no core behind it.
        let link = Arc::new(crate::link::Link::for_tests(a));
        CoreState::new(link)
    }

    #[test]
    fn a_blocked_state_with_a_permission_marks_the_terminal_and_its_product() {
        let mut s = state();
        s.put(info("t1-a", Some("demo")));
        s.put(info("t2-a", None));
        assert!(s.products_with_attention().is_empty());
        assert_eq!(
            s.apply(&note(
                "terminal.state",
                json!({"id": "t1-a", "state": "blocked", "kind": "permission", "msg": "Bash"})
            )),
            Applied::Changed
        );
        assert_eq!(s.terminals["t1-a"].attention().as_deref(), Some("Bash"));
        assert_eq!(
            s.products_with_attention().into_iter().collect::<Vec<_>>(),
            ["demo"]
        );
        // A blocked state of another kind, or without a kind, is no attention.
        s.apply(&note(
            "terminal.state",
            json!({"id": "t1-a", "state": "blocked", "kind": "rate_limit"}),
        ));
        assert_eq!(s.terminals["t1-a"].attention(), None);
        s.apply(&note(
            "terminal.state",
            json!({"id": "t1-a", "state": "blocked", "kind": "question"}),
        ));
        assert_eq!(
            s.terminals["t1-a"].attention().as_deref(),
            Some("Claude asks a question")
        );
        s.apply(&note(
            "terminal.state",
            json!({"id": "t1-a", "state": "working"}),
        ));
        assert!(s.products_with_attention().is_empty());
    }

    #[test]
    fn title_and_exit_events_change_the_terminal_and_an_unknown_id_asks_for_the_list() {
        let mut s = state();
        s.put(info("t1-a", None));
        let title = note("terminal.title", json!({"id": "t1-a", "title": "zsh"}));
        assert_eq!(s.apply(&title), Applied::Changed);
        assert_eq!(s.apply(&title), Applied::Same);
        let exit = note("terminal.exit", json!({"id": "t1-a", "code": 3}));
        assert_eq!(s.apply(&exit), Applied::Changed);
        let t = &s.terminals["t1-a"];
        assert_eq!(
            (t.title.as_str(), t.running, t.code),
            ("zsh", false, Some(3))
        );
        let other = note("terminal.state", json!({"id": "t9-a", "state": "idle"}));
        assert_eq!(s.apply(&other), Applied::Unknown);
        let other = note("terminal.exit", json!({"id": "t9-a", "code": 0}));
        assert_eq!(s.apply(&other), Applied::Unknown);
        let voice = note("voice.stop", json!({"terminal": "t1-a"}));
        assert_eq!(s.apply(&voice), Applied::Same);
    }

    #[test]
    fn a_closed_terminal_does_not_come_back_from_a_later_list() {
        let mut s = state();
        s.put(info("t1-a", None));
        s.close("t1-a");
        s.put(info("t1-a", None));
        assert!(s.terminals.is_empty());
    }

    #[test]
    fn a_new_connection_drops_the_ended_terminals_and_names_the_closed_ones() {
        let mut s = state();
        s.put(info("t1-a", None));
        s.put(info("t2-a", None));
        s.put(info("t3-a", None));
        s.terminals.get_mut("t1-a").unwrap().voice = true;
        s.close("t3-a");
        let closed = s.sync(vec![info("t1-a", None), info("t3-a", None)]);
        assert_eq!(closed, ["t3-a"]);
        assert_eq!(s.terminals.keys().collect::<Vec<_>>(), ["t1-a"]);
        assert!(s.terminals["t1-a"].voice);
    }

    /// The tree reads `story.list` again when a story changes, because stories have no
    /// progress event.
    #[gpui::test]
    fn a_stories_event_reads_the_story_list_of_its_product_and_lagged_reads_each_product(
        cx: &mut TestAppContext,
    ) {
        let mut t = two_cores(cx);
        let note = |method: &str, kind: &str| -> Notification {
            serde_json::from_value(json!({"jsonrpc": "2.0", "method": method,
                "params": {"product": "demo", "kind": kind, "missed": 3}}))
            .unwrap()
        };
        take_event(
            &t.cores.local,
            &note("data.changed", "minutes"),
            &mut cx.to_async(),
        );
        cx.run_until_parked();
        assert_eq!(methods(&mut t.local), Vec::<String>::new());
        take_event(
            &t.cores.local,
            &note("data.changed", "stories"),
            &mut cx.to_async(),
        );
        cx.run_until_parked();
        let sent = super::two_cores::received(&mut t.local);
        assert_eq!(sent.len(), 1);
        assert_eq!(sent[0]["method"], "story.list");
        assert_eq!(sent[0]["params"]["product"], "demo");
        take_event(&t.cores.local, &note("core.lagged", ""), &mut cx.to_async());
        cx.run_until_parked();
        assert!(methods(&mut t.local).contains(&"story.list".to_string()));
    }

    fn story(id: &str, status: &str, session: Option<&str>, archived: bool) -> Story {
        serde_json::from_value(json!({
            "id": id, "type": "story", "slug": "s", "title": format!("Title {id}"),
            "owner_role": "dev", "status": status, "approval": "approved",
            "proposed_by": "owner", "created_at": "t", "updated_at": "t",
            "archived": archived.then_some(true),
            "session": session.map(|state| json!({
                "state": state, "branch": "b", "worktree": "w", "base": "main",
                "log": "l", "started_at": "t",
            })),
        }))
        .unwrap()
    }

    #[test]
    fn only_a_story_that_is_not_done_and_has_a_session_is_an_item() {
        let items = StorySessionItem::of(vec![
            story("S1", "not_implemented", Some("work"), false),
            story("S2", "not_implemented", None, false),
            story("S3", "implemented", Some("ready"), false),
            story("S4", "not_implemented", Some("ready"), true),
            story("S5", "not_implemented", Some("failed"), false),
        ]);
        let got: Vec<_> = items.iter().map(|i| (i.id.as_str(), i.running())).collect();
        assert_eq!(got, [("S1", true), ("S5", false)]);
        assert_eq!(items[0].title, "Title S1");
    }

    #[test]
    fn sessions_and_reflections_report_a_change_once() {
        let mut s = state();
        let one = vec![StorySessionItem {
            id: "S1".into(),
            title: "T".into(),
            state: "work".into(),
        }];
        assert!(s.set_sessions("demo", one.clone()));
        assert!(!s.set_sessions("demo", one));
        assert!(s.set_sessions("demo", vec![]));
        assert!(!s.set_sessions("demo", vec![]));
        assert!(s.set_reflecting("demo", true));
        assert!(!s.set_reflecting("demo", true));
        assert!(s.set_reflecting("demo", false));
    }

    #[gpui::test]
    fn a_missing_host_has_no_core_and_is_never_the_local_core(cx: &mut gpui::TestAppContext) {
        let t = two_cores::two_cores(cx);
        let local = t.cores.local.entity_id();
        assert_eq!(t.cores.of(None).unwrap().entity_id(), local);
        assert_ne!(t.cores.of(Some("studio")).unwrap().entity_id(), local);
        assert!(t.cores.of(Some("laptop")).is_none());
        let gone = cx.update(|cx| t.cores.for_panel(Some("laptop"), cx));
        assert_ne!(gone.entity_id(), local);
        assert!(gone.read_with(cx, |s, _| s.away().is_some()));
    }

    #[test]
    fn only_a_core_of_a_host_can_be_away() {
        let mut s = state();
        s.host = HostState::Off("gone".into());
        assert_eq!(s.away(), None);
        let mut r = CoreState::remote("studio", Vec::new());
        assert_eq!(r.away().unwrap().0, "Connecting to studio…");
        r.host = HostState::Off("gone".into());
        assert_eq!(
            r.away(),
            Some(("studio: device off".into(), Some("gone".into())))
        );
        r.host = HostState::Up;
        assert_eq!(r.away(), None);
    }

    #[test]
    fn put_keeps_the_voice_mode() {
        let mut s = state();
        s.put(info("t1-a", None));
        s.terminals.get_mut("t1-a").unwrap().voice = true;
        s.put(info("t1-a", None));
        assert!(s.terminals["t1-a"].voice);
    }
}

#[cfg(test)]
mod product_tests {
    use super::two_cores::{methods, two_cores};
    use super::*;
    use gpui::TestAppContext;
    use serde_json::json;

    fn info(id: &str) -> ProductInfo {
        ProductInfo {
            id: id.into(),
            name: id.into(),
            path: None,
            archived: false,
        }
    }

    fn at(host: Option<&str>, id: &str) -> ProductRef {
        ProductRef {
            host: host.map(String::from),
            id: id.into(),
        }
    }

    /// B5: the host reconnects with the active product archived.
    #[gpui::test]
    fn the_active_product_moves_when_its_core_stops_listing_it(cx: &mut TestAppContext) {
        let t = two_cores(cx);
        let studio = t.cores.of(Some("studio")).unwrap().clone();
        studio.update(cx, |s, cx| {
            s.set_products(vec![info("demo"), info("old")], cx)
        });
        let on_studio = Some(at(Some("studio"), "old"));
        cx.update(|cx| assert_eq!(t.cores.keep_active(on_studio.as_ref(), cx), on_studio));
        studio.update(cx, |s, cx| s.set_products(vec![info("demo")], cx));
        cx.update(|cx| {
            assert_eq!(
                t.cores.keep_active(on_studio.as_ref(), cx),
                Some(at(None, "demo")),
                "the first product of the sidebar takes its place"
            );
            assert_eq!(t.cores.keep_active(None, cx), None);
            let kept = Some(at(Some("studio"), "demo"));
            assert_eq!(t.cores.keep_active(kept.as_ref(), cx), kept);
        });
        studio.update(cx, |s, cx| s.set_products(vec![], cx));
        t.cores.local.update(cx, |s, cx| s.set_products(vec![], cx));
        cx.update(|cx| assert_eq!(t.cores.keep_active(on_studio.as_ref(), cx), None));
    }

    /// B6: the restore names an archived product as active.
    #[gpui::test]
    fn a_saved_active_product_that_is_gone_gives_the_first_product(cx: &mut TestAppContext) {
        let t = two_cores(cx);
        cx.update(|cx| {
            let saved = Some(at(None, "archived-since"));
            assert_eq!(
                t.cores.keep_active(saved.as_ref(), cx),
                Some(at(None, "demo"))
            );
        });
    }

    /// B7: `agentsi restore --product X` with the app open.
    #[gpui::test]
    fn a_data_event_of_kind_product_reads_the_product_list_again(cx: &mut TestAppContext) {
        let mut t = two_cores(cx);
        let note = |kind: &str| -> Notification {
            serde_json::from_value(json!({"jsonrpc": "2.0", "method": "data.changed",
                "params": {"product": "demo", "kind": kind}}))
            .unwrap()
        };
        take_event(&t.cores.local, &note("minutes"), &mut cx.to_async());
        cx.run_until_parked();
        assert_eq!(methods(&mut t.local), Vec::<String>::new());
        take_event(&t.cores.local, &note("product"), &mut cx.to_async());
        cx.run_until_parked();
        assert_eq!(methods(&mut t.local), ["product.list"]);
        // A clear of data can remove a product.
        take_event(&t.cores.local, &note("clear"), &mut cx.to_async());
        cx.run_until_parked();
        assert_eq!(methods(&mut t.local), ["product.list"]);
    }

    #[gpui::test]
    fn the_product_list_reply_replaces_the_list_and_tells_the_views(cx: &mut TestAppContext) {
        let t = two_cores(cx);
        let seen: std::rc::Rc<std::cell::RefCell<Vec<CoreEvent>>> = Default::default();
        let out = seen.clone();
        cx.update(|cx| {
            cx.subscribe(&t.cores.local, move |_, e: &CoreEvent, _| {
                out.borrow_mut().push(*e)
            })
            .detach()
        });
        t.cores.local.update(cx, |s, cx| {
            s.set_products(
                vec![
                    info("a"),
                    ProductInfo {
                        archived: true,
                        ..info("b")
                    },
                ],
                cx,
            )
        });
        assert_eq!(
            t.cores.local.read_with(cx, |s, _| s
                .products
                .iter()
                .map(|p| p.id.clone())
                .collect::<Vec<_>>()),
            ["a"]
        );
        assert_eq!(*seen.borrow(), [CoreEvent::Products]);
    }
}
