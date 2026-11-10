//! The window: the sidebar, the dock of panels, and the command palette.
//!
//! At start the shell reads the products and the terminals of the core, then
//! restores the saved layout. It drops each terminal panel whose terminal the core
//! does not know, and says so in the sidebar. Each change of the dock saves the
//! layout again.
//!
//! The dock holds panels of each host. A panel gets the core of its product's host from
//! `Cores`, at the moment that the shell opens it.

use std::cell::RefCell;
use std::collections::BTreeSet;
use std::path::PathBuf;
use std::rc::Rc;
use std::sync::atomic::Ordering;
use std::time::Duration;

use agentsi_proto::Event as _;
use agentsi_proto::core_api::{Empty, Subscribe, SubscribeParams, Topic};
use agentsi_proto::desk_api::ProductList;
use agentsi_proto::events::Lagged;
use agentsi_proto::marks_api::Verdict;
use agentsi_proto::terminal_api::{Create, CreateParams, List};
use agentsi_ui::bench::{Bench, BenchKind};
use agentsi_ui::tokens::{control, font, radius, size, space, surface, text};
use futures::StreamExt;
use futures::channel::mpsc;
use gpui_kit::component::Placement;
use gpui_kit::component::command::{Command, CommandItem, CommandState};
use gpui_kit::component::dock::{
    DockArea, DockEvent, DockPlacement, PanelId, panel_handle, register_panel,
};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;

use crate::actions::{self, ENTRIES};
use crate::alerts::{AlertTarget, Alerts, Raised};
use crate::bench::{BenchHandle, BenchPlan};
use crate::hosts::HostsGlobal;
use crate::keys;
use crate::layout::{self, Layout, SCREEN_PANEL, TERMINAL_PANEL};
use crate::link::LinkEvent;
use crate::look::{self, Look, LookData, LookGlobal};
use crate::menus::{self, Cmd, MenuRequest, Target};
use crate::model::{CoreEvent, CoreState, Cores, HostState, ProductRef, take_event, take_sessions};
use crate::nav::{Nav, ScreenRequest};
use crate::pane::{QUITTING, TerminalPanel};
use crate::prompt::{Choice, Prompt, PromptEvent};
use crate::screens::meeting;
use crate::screens::memory::MemoryScreen;
use crate::screens::parts::{FIRST_SCREEN_READY, first_screen_ready};
use crate::screens::settings::{ModelFeed, ModelFeedGlobal};
use crate::screens::{ScreenKind, ScreenPanel, ScreenSpec, screen_of};
use crate::sidebar::{NewProduct, OpenItem, SelectProduct, ShowAlerts, Sidebar, TreeChanged};
use crate::tab_bar;
use crate::tabs::{self, GroupId, TabGroups, TabsChanged};
use crate::tree::ItemKey;
use agentsi_ui::tokens::label::Hue;

mod alert_actions;
#[cfg(test)]
mod alert_tests;
mod menu_actions;
#[cfg(test)]
mod menu_tests;
mod tab_actions;
#[cfg(test)]
mod ticket_tests;

/// The time between a change of the dock and the save of the layout.
const SAVE_DELAY: Duration = Duration::from_millis(300);
/// The id and version of the dock in the layout file.
const DOCK_ID: &str = "agentsi";
const DOCK_VERSION: usize = 1;

/// The panels that the shell opened or restored, to find them again.
#[derive(Default)]
pub struct Open {
    pub screens: Vec<WeakEntity<ScreenPanel>>,
    pub terminals: Vec<WeakEntity<TerminalPanel>>,
}

/// Where the shell keeps its layout.
#[derive(Clone)]
pub enum Persist {
    /// Read the layout at start and write it on each change.
    File(PathBuf),
    /// Read it, never write it: the start bench.
    ReadOnly(PathBuf),
    /// No layout file: the other benches.
    None,
}

/// A terminal of one host.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PaneRef {
    pub host: Option<String>,
    pub id: String,
}

/// What a prompt of the shell asks about.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Ask {
    ProductLabel(ProductRef),
    ProductColor(ProductRef),
    ProductGroup(ProductRef),
    PaneLabel(PaneRef),
    PaneColor(PaneRef),
    /// A name for a new tab group that starts with this tab.
    NewTabGroup(PanelId),
    /// The tab group, among the groups of the bar of this tab, that takes this tab.
    AddToTabGroup(PanelId),
    RenameTabGroup(GroupId),
    ColorTabGroup(GroupId),
    /// The one Confirm before tabs close while a program runs in some of them: the tabs, and the
    /// number that run a program.
    CloseTabs(Vec<PanelId>, usize),
    /// The title of a new ticket for this product.
    NewTicket(ProductRef),
    /// What the ticket waits on: the product and the ticket id.
    TicketWait(ProductRef, String),
    /// A new name for the product group of this host.
    GroupName(Option<String>, String),
    GroupColor(Option<String>, String),
    /// The one Confirm before a menu command that stops a program, spends money or hides a
    /// product.
    Confirm(Target, Cmd),
}

struct Asking {
    what: Ask,
    prompt: Entity<Prompt>,
    _answer: Subscription,
}

pub struct Shell {
    /// The core of this device.
    core: Entity<CoreState>,
    cores: Cores,
    dock: Entity<DockArea>,
    /// The tab groups of the dock.
    tabs: Entity<TabGroups>,
    sidebar: Entity<Sidebar>,
    open: Rc<RefCell<Open>>,
    palette: Option<Entity<CommandState>>,
    /// The labels, the colors and the product groups, when the window has a look file.
    look: Option<Entity<Look>>,
    asking: Option<Asking>,
    /// The alert log is open.
    alerts_open: bool,
    /// The context menu that is open.
    menu: Option<menu_actions::OpenMenu>,
    persist: Persist,
    /// false until the restore ended: a save before it would write an empty layout.
    restored: bool,
    save: Option<Task<()>>,
    bench: Option<Bench>,
    plan: Option<BenchPlan>,
    focus: FocusHandle,
    _subscriptions: Vec<Subscription>,
}

/// Registers the panel builders of the layout file. Call it once, before the restore.
pub fn register_panels(cores: &Cores, open: &Rc<RefCell<Open>>, cx: &mut App) {
    let (c, o) = (cores.clone(), open.clone());
    register_panel(cx, TERMINAL_PANEL, move |ctx, window, cx| {
        let id = layout::terminal_of(ctx.state())
            .unwrap_or_default()
            .to_string();
        let core = c.for_panel(layout::host_of(ctx.state()), cx);
        let panel = cx.new(|cx| TerminalPanel::new(id, core, None, None, window, cx));
        o.borrow_mut().terminals.push(panel.downgrade());
        panel_handle(panel)
    });
    let (c, o) = (cores.clone(), open.clone());
    register_panel(cx, SCREEN_PANEL, move |ctx, _, cx| {
        let spec = screen_of(ctx.state()).unwrap_or(ScreenSpec {
            kind: ScreenKind::Desk,
            host: layout::host_of(ctx.state()).map(String::from),
            product: String::new(),
            arg: None,
        });
        let core = c.for_panel(spec.host.as_deref(), cx);
        let panel = cx.new(|cx| ScreenPanel::new(spec, core, cx));
        o.borrow_mut().screens.push(panel.downgrade());
        panel_handle(panel)
    });
}

impl Shell {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        cores: Cores,
        events: mpsc::UnboundedReceiver<LinkEvent>,
        open: Rc<RefCell<Open>>,
        persist: Persist,
        bench: Option<Bench>,
        plan: Option<BenchPlan>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let tabs = cx.new(|_| TabGroups::default());
        let (dock, skin) =
            tab_bar::dock_area(DOCK_ID, Some(DOCK_VERSION), tabs.clone(), window, cx);
        skin.set_close_button_visible(true, cx);
        let probe = plan.as_ref().and_then(|p| p.probe.clone());
        let core = cores.local.clone();
        let sidebar = cx.new(|cx| Sidebar::new(cores.clone(), probe, cx));
        let mut subscriptions = vec![
            cx.subscribe_in(&dock, window, |this, _, e: &DockEvent, window, cx| {
                if matches!(e, DockEvent::LayoutChanged) {
                    tabs::reconcile(&this.dock, &this.tabs, window, cx);
                    this.schedule_save(cx);
                }
            }),
            cx.subscribe(&tabs, |this, _, _: &TabsChanged, cx| {
                // The bars read the groups when they draw: tell the dock to draw again.
                this.dock.update(cx, |_, cx| cx.notify());
                this.schedule_save(cx);
            }),
            cx.subscribe_in(
                &sidebar,
                window,
                |this, _, e: &SelectProduct, window, cx| this.select_product(&e.0, window, cx),
            ),
            cx.subscribe_in(&sidebar, window, |this, _, e: &OpenItem, window, cx| {
                this.open_item(&e.product, &e.item, window, cx)
            }),
            cx.subscribe_in(&sidebar, window, |this, _, e: &NewProduct, window, cx| {
                this.show_screen(ScreenSpec::setup("").on(e.0.clone()), window, cx)
            }),
            cx.subscribe(&sidebar, |this, _, _: &ShowAlerts, cx| {
                this.toggle_alert_log(cx)
            }),
            cx.subscribe(&sidebar, |this, _, _: &TreeChanged, cx| {
                this.schedule_save(cx)
            }),
            cx.observe(&core, |_, _, cx| cx.notify()),
        ];
        let look = cx.try_global::<LookGlobal>().map(|g| g.0.clone());
        if let Some(l) = &look {
            // A new label or color redraws each tab: the dock draws the title of a tab again
            // only when its panel or the shell tells it.
            subscriptions.push(cx.observe(l, |this, _, cx| this.look_changed(cx)));
        }
        // A write of a core, also one by the CLI, makes the open screens of its host read
        // again. So does the return of the host.
        for c in cores.all() {
            let host = c.read(cx).remote.clone();
            subscriptions.push(cx.subscribe(c, move |this, core, e: &CoreEvent, cx| {
                if *e == CoreEvent::Products {
                    this.repair_active(cx);
                    Self::read_sessions(core.clone(), cx);
                } else {
                    this.reload_host(host.as_deref(), cx)
                }
            }));
        }
        if let Some(center) = Alerts::try_get(cx) {
            subscriptions.push(cx.subscribe_in(
                &center,
                window,
                |this, _, raised: &Raised, window, cx| this.alert_raised(&raised.0, window, cx),
            ));
        }
        let hub = menus::install(cx);
        subscriptions.push(cx.subscribe_in(
            &hub,
            window,
            |this, _, r: &MenuRequest, window, cx| this.open_menu(r.clone(), window, cx),
        ));
        let (nav, requests) = Nav::channel();
        cx.set_global(nav);
        let (runs, claude_runs) = crate::nav::Runs::channel();
        cx.set_global(runs);
        let feed = cx.new(|_| ModelFeed);
        cx.set_global(ModelFeedGlobal(feed.clone()));
        if let Some(b) = &bench {
            cx.set_global(BenchHandle(b.clone()));
            b.start(window, cx);
        }
        let shell = Shell {
            core,
            cores,
            dock,
            tabs,
            sidebar,
            open,
            palette: None,
            look,
            asking: None,
            alerts_open: false,
            menu: None,
            persist,
            restored: false,
            save: None,
            bench,
            plan,
            focus: cx.focus_handle(),
            _subscriptions: subscriptions,
        };
        shell.pump_events(events, feed, cx);
        shell.pump_requests(requests, window, cx);
        shell.pump_claude_runs(claude_runs, window, cx);
        shell.start(window, cx);
        window.focus(&shell.focus, cx);
        shell
    }

    fn link(&self, cx: &App) -> std::sync::Arc<crate::link::Link> {
        self.core.read(cx).link.clone()
    }

    /// Takes the events of the core of this device into the state.
    /// A `models.*` event goes to the settings screens only, so no other view draws.
    fn pump_events(
        &self,
        mut events: mpsc::UnboundedReceiver<LinkEvent>,
        feed: Entity<ModelFeed>,
        cx: &mut Context<Self>,
    ) {
        let core = self.core.clone();
        cx.spawn(async move |_, cx| {
            while let Some(event) = events.next().await {
                match event {
                    LinkEvent::Notification(n) => {
                        // A lost event of the models topic makes the settings screen read its
                        // list again.
                        let models = n.method.starts_with("models.");
                        if models || n.method == Lagged::NAME {
                            feed.update(cx, |_, cx| cx.emit(n.clone()));
                        }
                        if models {
                            crate::model::raise_alerts(&core, &n, cx);
                        } else {
                            take_event(&core, &n, cx);
                        }
                    }
                    LinkEvent::Lost(why) => core.update(cx, |s, cx| {
                        s.host = HostState::Off(format!("{why}. Start the app again."));
                        s.meetings_lost(cx);
                        cx.notify();
                    }),
                }
            }
        })
        .detach();
    }

    /// Opens the screens that a screen asks for.
    fn pump_requests(
        &self,
        mut requests: mpsc::UnboundedReceiver<ScreenRequest>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        cx.spawn_in(window, async move |this, cx| {
            while let Some(r) = requests.next().await {
                let _ = this.update_in(cx, |this, window, cx| this.show_screen(r, window, cx));
            }
        })
        .detach();
    }

    /// Opens a Claude pane for each "Run in Claude" click of a screen.
    fn pump_claude_runs(
        &self,
        mut runs: mpsc::UnboundedReceiver<crate::nav::ClaudeRun>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        cx.spawn_in(window, async move |this, cx| {
            while let Some(r) = runs.next().await {
                let _ = this.update_in(cx, |this, window, cx| {
                    let product = ProductRef {
                        host: r.host,
                        id: r.product,
                    };
                    this.open_terminal_in(Some(product), vec!["claude".into(), crate::nav::claude_prompt(&r.text)], window, cx)
                });
            }
        })
        .detach();
    }

    /// Subscribes, reads the products and the terminals, then restores the layout.
    fn start(&self, window: &mut Window, cx: &mut Context<Self>) {
        let link = self.link(cx);
        let subscribed = link.call::<Subscribe>(&SubscribeParams {
            topics: vec![
                Topic::Terminal,
                Topic::Data,
                Topic::Models,
                Topic::Meeting,
                Topic::Voice,
                Topic::Desk,
            ],
        });
        let products = link.call::<ProductList>(&Empty {});
        let terminals = link.call::<List>(&Empty {});
        let core = self.core.clone();
        cx.spawn_in(window, async move |this, cx| {
            let up = subscribed.await.is_ok();
            let products = products.await;
            let terminals = terminals.await;
            let listed = core.update(cx, |s, cx| {
                if up && !matches!(s.host, HostState::Off(_)) {
                    s.host = HostState::Up;
                }
                match products {
                    Ok(p) => s.products = crate::model::listed(p.products),
                    Err(e) => s.products_error = Some(format!("No products: {e}")),
                }
                let listed = match terminals {
                    Ok(t) => {
                        for info in t.terminals {
                            s.put(info);
                        }
                        true
                    }
                    Err(e) => {
                        s.notices.push(format!(
                            "The terminals of the core were not read ({e}). The saved layout stays as it is, and this window does not save it."
                        ));
                        false
                    }
                };
                cx.notify();
                listed
            });
            let _ = this.update_in(cx, |this, window, cx| {
                Self::read_sessions(this.core.clone(), cx);
                if listed {
                    this.restore(window, cx)
                } else {
                    // No restore, so no screen will come: the start bench must not wait.
                    first_screen_ready();
                }
            });
        })
        .detach();
    }

    fn restore(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(plan) = self.plan.clone() {
            self.restored = true;
            plan.run(self, window, cx);
            return;
        }
        let path = match &self.persist {
            Persist::File(p) | Persist::ReadOnly(p) => p.clone(),
            Persist::None => {
                self.restored = true;
                return;
            }
        };
        match layout::read(&path) {
            Ok(Some(mut saved)) => {
                let known: BTreeSet<String> =
                    self.core.read(cx).terminals.keys().cloned().collect();
                let known = layout::Known {
                    terminals: known,
                    hosts: self.known_hosts(cx),
                };
                self.prune_look(&known, cx);
                let tree = std::mem::take(&mut saved.tree);
                self.sidebar.update(cx, |s, cx| s.set_open_state(tree, cx));
                let dropped = layout::prune(&mut saved.dock, &known);
                if !dropped.is_empty() {
                    let note = format!(
                        "Dropped {} pane{} of terminals or hosts that this device does not know: {}. A core restart ends its terminals.",
                        dropped.len(),
                        if dropped.len() == 1 { "" } else { "s" },
                        dropped.join(", ")
                    );
                    eprintln!("agentsi-app: {note}");
                    self.core.update(cx, |s, _| s.notices.push(note));
                }
                let active = saved.active_product.map(|id| ProductRef {
                    host: saved.active_host,
                    id,
                });
                // A product that left the lists since the save gives its place to the first one,
                // so the keys of the active product work also with panes on the screen.
                let active = self.cores.keep_active(active.as_ref(), cx);
                self.core.update(cx, |s, _| s.active = active);
                let (dock, groups) = (saved.dock, saved.tab_groups);
                self.dock.update(cx, |d, cx| {
                    if let Err(e) = d.load(dock, window, cx) {
                        eprintln!("agentsi-app: the layout does not load: {e}");
                    }
                });
                self.restore_tab_groups(&groups, window, cx);
            }
            Ok(None) => {}
            Err(e) => {
                // The next save writes a new file: keep the old one for the owner.
                let kept = path.with_extension("json.bad");
                let _ = std::fs::rename(&path, &kept);
                let note = format!(
                    "The saved layout was not read: {e}. It is now {}.",
                    kept.display()
                );
                eprintln!("agentsi-app: {note}");
                self.core.update(cx, |s, _| s.notices.push(note));
            }
        }
        self.restored = true;
        // An empty dock shows the front desk of the active product, else of the first product.
        let first = {
            let core = self.core.read(cx);
            core.active.clone().or_else(|| {
                core.products.first().map(|p| ProductRef {
                    host: None,
                    id: p.id.clone(),
                })
            })
        };
        if self.dock.read(cx).is_empty(DockPlacement::Center, cx)
            && let Some(p) = first
        {
            self.select_product(&p, window, cx);
        }
        // The start bench waits for the first screen. A layout of terminals, and no
        // product at all, have none.
        if self.open.borrow().screens.is_empty() {
            first_screen_ready();
        }
        cx.notify();
    }

    fn schedule_save(&mut self, cx: &mut Context<Self>) {
        let Persist::File(path) = self.persist.clone() else {
            return;
        };
        if !self.restored {
            return;
        }
        self.save = Some(cx.spawn(async move |this, cx| {
            cx.background_executor().timer(SAVE_DELAY).await;
            let _ = this.update(cx, |this, cx| this.save_now(&path, cx));
        }));
    }

    pub fn save_now(&self, path: &std::path::Path, cx: &App) {
        let active = self.core.read(cx).active.clone();
        let (active_host, active_product) = match active {
            Some(p) => (p.host, Some(p.id)),
            None => (None, None),
        };
        let layout = Layout {
            version: layout::VERSION,
            active_product,
            active_host,
            tree: self.sidebar.read(cx).open_state(),
            tab_groups: self.tabs.read(cx).save(self.dock.read(cx), cx),
            dock: self.dock.read(cx).dump(cx),
        };
        if let Err(e) = layout::write(path, &layout) {
            eprintln!(
                "agentsi-app: cannot save the layout to {}: {e}",
                path.display()
            );
        }
    }

    /// Saves the layout at once and marks the quit, so no panel kills its terminal.
    pub fn before_quit(&self, cx: &App) {
        QUITTING.store(true, Ordering::Relaxed);
        if let Persist::File(path) = &self.persist
            && self.restored
        {
            self.save_now(path, cx);
        }
    }

    pub fn core(&self) -> &Entity<CoreState> {
        &self.core
    }

    pub fn dock(&self) -> &Entity<DockArea> {
        &self.dock
    }

    pub fn bench(&self) -> Option<&Bench> {
        self.bench.as_ref()
    }

    pub fn open(&self) -> &Rc<RefCell<Open>> {
        &self.open
    }

    fn in_dock(&self, id: EntityId, cx: &App) -> bool {
        self.dock.read(cx).panel(PanelId::from(id)).is_some()
    }

    /// The hosts of `hosts.toml`, or `None` when the file did not parse.
    fn known_hosts(&self, cx: &App) -> Option<BTreeSet<String>> {
        let unread = cx
            .try_global::<HostsGlobal>()
            .is_some_and(|h| h.0.read(cx).error.is_some());
        (!unread).then(|| self.cores.names())
    }

    /// After a change of a product list: an active product that its core no longer lists
    /// gives its place to the first product of the sidebar. Its screens stay as they are.
    fn repair_active(&mut self, cx: &mut Context<Self>) {
        let active = self.core.read(cx).active.clone();
        let kept = self.cores.keep_active(active.as_ref(), cx);
        if kept != active {
            self.core.update(cx, |s, cx| {
                s.active = kept;
                cx.notify();
            });
            self.schedule_save(cx);
        }
    }

    fn has_product(&self, product: &ProductRef, cx: &App) -> bool {
        self.cores
            .of(product.host.as_deref())
            .is_some_and(|c| c.read(cx).product(&product.id).is_some())
    }

    /// Makes `product` active and shows its front desk.
    pub fn select_product(
        &mut self,
        product: &ProductRef,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if !self.has_product(product, cx) {
            return;
        }
        self.core.update(cx, |s, cx| {
            s.active = Some(product.clone());
            cx.notify();
        });
        self.show_screen(ScreenSpec::desk(product), window, cx);
        self.schedule_save(cx);
    }

    /// Each product in the order of the sidebar, with its name in the palette.
    fn products(&self, cx: &App) -> Vec<(ProductRef, String)> {
        let look = LookGlobal::with(cx, LookData::clone).unwrap_or_default();
        self.cores
            .all()
            .flat_map(|c| {
                let core = c.read(cx);
                // The order of the sidebar: the products with no group, then each group.
                let order = look.arrange(core.remote.as_deref(), &core.products).flat();
                order.into_iter().map(|i| {
                    let p = &core.products[i];
                    let name = match &core.remote {
                        Some(host) => format!("{} on {host}", p.name),
                        None => p.name.clone(),
                    };
                    let id = p.id.clone();
                    let host = core.remote.clone();
                    (ProductRef { host, id }, name)
                })
            })
            .collect()
    }

    /// Opens a screen of the active product.
    fn show_active(
        &mut self,
        spec: fn(&str) -> ScreenSpec,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if let Some(p) = self.core.read(cx).active.clone() {
            self.show_screen(spec(&p.id).on(p.host), window, cx);
        }
    }

    /// Selects the panel of `spec`, or opens one.
    pub fn show_screen(&mut self, spec: ScreenSpec, window: &mut Window, cx: &mut Context<Self>) {
        let found = self.open.borrow().screens.iter().find_map(|w| {
            let e = w.upgrade()?;
            (e.read(cx).is(&spec) && self.in_dock(e.entity_id(), cx)).then_some(e)
        });
        let panel = match found {
            Some(p) => {
                let id = PanelId::from(p.entity_id());
                self.dock.update(cx, |d, cx| d.select_panel(id, window, cx));
                p
            }
            None => {
                let Some(core) = self.cores.of(spec.host.as_deref()).cloned() else {
                    eprintln!("agentsi-app: no host {:?}", spec.host);
                    return;
                };
                let p = cx.new(|cx| ScreenPanel::new(spec, core, cx));
                self.open.borrow_mut().screens.push(p.downgrade());
                self.dock.update(cx, |d, cx| {
                    d.add_panel_view(
                        panel_handle(p.clone()),
                        DockPlacement::Center,
                        None,
                        window,
                        cx,
                    )
                });
                p
            }
        };
        window.focus(&panel.focus_handle(cx), cx);
    }

    /// Opens the call of the minutes that the owner used last, or of the last meeting.
    fn open_call(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let from_minutes = self
            .open_screens(cx)
            .into_iter()
            .filter_map(|p| {
                let p = p.read(cx);
                let stamp = p.minute_stamp(cx)?;
                let spec = ScreenSpec::call(&p.product, p.arg.as_deref()?).on(p.host.clone());
                Some((stamp, spec))
            })
            .max_by_key(|(stamp, _)| *stamp)
            .map(|(_, spec)| spec);
        let spec = from_minutes.or_else(|| {
            let p = self.core.read(cx).active.clone()?;
            let core = self.cores.of(p.host.as_deref())?;
            let id = meeting::minutes_of(&p.id, core, cx)?;
            Some(ScreenSpec::call(&p.id, &id).on(p.host))
        });
        if let Some(spec) = spec {
            self.show_screen(spec, window, cx);
        }
    }

    /// Starts the meeting in the question field of the open meeting screen of `product`.
    fn start_meeting(&self, product: &ProductRef, cx: &mut Context<Self>) {
        let spec = ScreenSpec::meeting(&product.id).on(product.host.clone());
        let panel = self
            .open
            .borrow()
            .screens
            .iter()
            .filter_map(WeakEntity::upgrade)
            .find(|p| p.read(cx).is(&spec));
        if let Some(p) = panel {
            p.update(cx, |p, cx| p.start_meeting(cx));
        }
    }

    /// Runs a palette action on the memory screen of the active product.
    fn on_memory(
        &self,
        cx: &mut Context<Self>,
        action: impl FnOnce(&mut MemoryScreen, &mut Context<MemoryScreen>),
    ) {
        let Some(product) = self.core.read(cx).active.clone() else {
            return;
        };
        let spec = ScreenSpec::memory(&product.id).on(product.host);
        let screen = self
            .open_screens(cx)
            .into_iter()
            .find(|p| p.read(cx).is(&spec))
            .and_then(|p| p.read(cx).memory());
        if let Some(s) = screen {
            s.update(cx, action);
        }
    }

    /// Runs `f` on the meeting screen of the active product.
    fn on_active_meeting(
        &self,
        window: &mut Window,
        cx: &mut Context<Self>,
        f: impl Fn(&mut ScreenPanel, &mut Window, &mut Context<ScreenPanel>),
    ) {
        let Some(p) = self.core.read(cx).active.clone() else {
            return;
        };
        let spec = ScreenSpec::meeting(&p.id).on(p.host);
        for panel in self.open_screens(cx) {
            if panel.read(cx).is(&spec) {
                panel.update(cx, |p, cx| f(p, window, cx));
            }
        }
    }

    fn open_screens(&self, cx: &App) -> Vec<Entity<ScreenPanel>> {
        self.open
            .borrow()
            .screens
            .iter()
            .filter_map(WeakEntity::upgrade)
            .filter(|p| self.in_dock(p.entity_id(), cx))
            .collect()
    }

    /// Reads each open screen from its core again, the settings screen too.
    fn reload_screens(&self, cx: &mut Context<Self>) {
        for p in self.open_screens(cx) {
            p.update(cx, |p, cx| p.reload(cx));
        }
    }

    /// Reads the open screens of `host` again. A screen of another host keeps its data.
    fn reload_host(&self, host: Option<&str>, cx: &mut Context<Self>) {
        for p in self.open_screens(cx) {
            if p.read(cx).follows(host) {
                p.update(cx, |p, cx| p.reload(cx));
            }
        }
    }

    /// The core of `product`, the folder and the product of a new terminal. The folder of a
    /// product of a host is on that host, so only its core can check it. With no product the
    /// terminal opens in the home folder of this device.
    fn terminal_place(
        &self,
        active: Option<ProductRef>,
        cx: &App,
    ) -> Result<(Entity<CoreState>, String, Option<String>), String> {
        let home = std::env::var("HOME").unwrap_or_else(|_| "/".into());
        let Some(active) = active else {
            return Ok((self.core.clone(), home, None));
        };
        let Some(core) = self.cores.of(active.host.as_deref()).cloned() else {
            return Err(format!("no host {:?}", active.host));
        };
        let path = core
            .read(cx)
            .product(&active.id)
            .and_then(|p| p.path.clone());
        let dir = match (&active.host, path) {
            (None, path) => path
                .filter(|d| std::path::Path::new(d).is_dir())
                .unwrap_or(home),
            (Some(_), Some(path)) => path,
            (Some(host), None) => {
                return Err(format!("{} has no folder on {host}", active.id));
            }
        };
        Ok((core, dir, Some(active.id)))
    }

    /// `terminal.create` in the folder of the active product, on its host, then a panel
    /// attached to it.
    pub fn open_terminal(
        &mut self,
        argv: Vec<String>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let active = self.core.read(cx).active.clone();
        self.open_terminal_in(active, argv, window, cx);
    }

    /// `terminal.create` in the folder of `product`, on its host, then a panel attached to it.
    pub fn open_terminal_in(
        &mut self,
        product: Option<ProductRef>,
        argv: Vec<String>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        match self.terminal_place(product, cx) {
            Ok((core, cwd, product)) => {
                self.create_terminal(core, cwd, argv, product, None, window, cx)
            }
            Err(e) => self.notice(&format!("No new terminal: {e}"), cx),
        }
    }

    /// `terminal.create` on `core`, then a panel attached to it. With `beside`, the panel sits
    /// in a new tab group at that side of the panel `beside.0`.
    #[allow(clippy::too_many_arguments)]
    pub fn create_terminal(
        &mut self,
        core: Entity<CoreState>,
        cwd: String,
        argv: Vec<String>,
        product: Option<String>,
        beside: Option<(PanelId, Placement)>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let created = core.read(cx).link.call::<Create>(&CreateParams {
            cwd,
            argv,
            env: Default::default(),
            product,
            cols: None,
            rows: None,
        });
        cx.spawn_in(window, async move |this, cx| match created.await {
            Ok(info) => {
                let _ = this.update_in(cx, |this, window, cx| {
                    let id = info.id.clone();
                    core.update(cx, |s, cx| {
                        s.put(info);
                        cx.notify();
                    });
                    let panel = this.add_terminal_panel(id, core, window, cx);
                    if let Some((other, placement)) = beside {
                        let new = PanelId::from(panel.entity_id());
                        this.place_beside(new, other, placement, window, cx);
                    }
                });
            }
            Err(e) => {
                let _ = this.update(cx, |this, cx| {
                    this.notice(&format!("No new terminal: {e}"), cx)
                });
            }
        })
        .detach();
    }

    /// Moves panel `new` into a new tab group at the `placement` side of the group of `other`.
    pub fn place_beside(
        &mut self,
        new: PanelId,
        other: PanelId,
        placement: Placement,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let node = self
            .dock
            .read(cx)
            .layout(DockPlacement::Center)
            .and_then(|tree| tree.find_panel_node(other));
        if let Some(node) = node {
            self.dock
                .update(cx, |d, cx| d.split_at(node, new, placement, window, cx));
        }
    }

    /// Adds a panel for terminal `id` of `core` to the dock and focuses it.
    pub fn add_terminal_panel(
        &mut self,
        id: String,
        core: Entity<CoreState>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Entity<TerminalPanel> {
        let bench = self
            .bench
            .clone()
            .filter(|b| b.kind() == BenchKind::Terminal);
        let probe = self.plan.as_ref().and_then(|p| p.probe.clone());
        let panel = cx.new(|cx| TerminalPanel::new(id, core, bench, probe, window, cx));
        self.open.borrow_mut().terminals.push(panel.downgrade());
        self.dock.update(cx, |d, cx| {
            d.add_panel_view(
                panel_handle(panel.clone()),
                DockPlacement::Center,
                None,
                window,
                cx,
            )
        });
        window.focus(&panel.focus_handle(cx), cx);
        panel
    }

    /// The shell of a new terminal. The `SHELL` of this device may not exist on a host, so a
    /// terminal of a host asks for the shell of that host.
    fn shell_argv(&self, cx: &App) -> Vec<String> {
        let remote = self
            .core
            .read(cx)
            .active
            .as_ref()
            .is_some_and(|p| p.host.is_some());
        shell_argv_for(remote)
    }

    /// The panel with the keyboard focus, else the panel in view that had it last: the palette
    /// takes the focus while it is open. The focus on a hidden pane would reach no key.
    fn focused_terminal(&self, window: &Window, cx: &App) -> Option<Entity<TerminalPanel>> {
        let panels: Vec<Entity<TerminalPanel>> = self
            .open
            .borrow()
            .terminals
            .iter()
            .filter_map(WeakEntity::upgrade)
            .filter(|p| self.in_dock(p.entity_id(), cx))
            .collect();
        panels
            .iter()
            .find(|p| p.focus_handle(cx).contains_focused(window, cx))
            .or_else(|| {
                panels
                    .iter()
                    .filter(|p| p.read(cx).shown && p.read(cx).last_use > 0)
                    .max_by_key(|p| p.read(cx).last_use)
            })
            .cloned()
    }

    /// Reads the story sessions of each product of `core`, for the tree.
    fn read_sessions(core: Entity<CoreState>, cx: &mut Context<Self>) {
        let reads = core.update(cx, |s, _| {
            let all: Vec<String> = s.products.iter().map(|p| p.id.clone()).collect();
            s.read_sessions(all)
        });
        cx.spawn(async move |_, cx| take_sessions(&core, reads, cx))
            .detach();
    }

    /// Drops the pane entries of the look file for terminals that the core no longer knows.
    fn prune_look(&self, known: &layout::Known, cx: &mut Context<Self>) {
        if let Some(look) = &self.look {
            look.update(cx, |l, cx| {
                l.prune_panes(&known.terminals, known.hosts.as_ref(), cx);
            });
        }
    }

    /// A label, a color or a group changed: each tab and the sidebar draw again.
    fn look_changed(&mut self, cx: &mut Context<Self>) {
        let (terminals, screens): (Vec<_>, Vec<_>) = {
            let open = self.open.borrow();
            (
                open.terminals
                    .iter()
                    .filter_map(WeakEntity::upgrade)
                    .collect(),
                open.screens
                    .iter()
                    .filter_map(WeakEntity::upgrade)
                    .collect(),
            )
        };
        for p in terminals {
            p.update(cx, |_, cx| cx.notify());
        }
        for p in screens {
            p.update(cx, |_, cx| cx.notify());
        }
        cx.notify();
    }

    /// A click on an item row of the tree: its tab gets the focus, and opens first when it is
    /// closed.
    pub fn open_item(
        &mut self,
        product: &ProductRef,
        item: &ItemKey,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let host = product.host.clone();
        if let (Some(center), Some(target)) = (
            Alerts::try_get(cx),
            AlertTarget::of_item(host.as_deref(), &product.id, item),
        ) {
            center.update(cx, |c, cx| c.opened(&target, cx));
        }
        match item {
            ItemKey::Pane(id) => self.focus_terminal(host, id, window, cx),
            ItemKey::Meeting => {
                self.show_screen(ScreenSpec::meeting(&product.id).on(host), window, cx)
            }
            ItemKey::Story(id) => {
                self.show_screen(ScreenSpec::story(&product.id, id).on(host), window, cx)
            }
            ItemKey::Reflection => {
                self.show_screen(ScreenSpec::memory(&product.id).on(host), window, cx)
            }
        }
    }

    /// Selects the tab of terminal `id`. A terminal with no tab gets a tab that attaches to it.
    pub fn focus_terminal(
        &mut self,
        host: Option<String>,
        id: &str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let found = self
            .open
            .borrow()
            .terminals
            .iter()
            .filter_map(WeakEntity::upgrade)
            .find(|p| p.read(cx).is(host.as_deref(), id, cx) && self.in_dock(p.entity_id(), cx));
        if let Some(panel) = found {
            let panel_id = PanelId::from(panel.entity_id());
            self.dock
                .update(cx, |d, cx| d.select_panel(panel_id, window, cx));
            window.focus(&panel.focus_handle(cx), cx);
            return;
        }
        let Some(core) = self.cores.of(host.as_deref()).cloned() else {
            return;
        };
        if core.read(cx).terminals.contains_key(id) {
            self.add_terminal_panel(id.to_string(), core, window, cx);
        }
    }

    fn toggle_palette(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.palette.is_some() {
            self.close_palette(window, cx);
            return;
        }
        let state = cx.new(|cx| CommandState::new(window, cx));
        state.update(cx, |s, cx| s.focus(window, cx));
        self.palette = Some(state);
        cx.notify();
    }

    /// Closes the palette. The focus goes back to the last pane, else to the shell.
    fn close_palette(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.palette.take().is_none() {
            return;
        }
        // An entry that asks a question has the focus now.
        if self.asking.is_some() {
            cx.notify();
            return;
        }
        match self.focused_terminal(window, cx) {
            Some(p) => window.focus(&p.focus_handle(cx), cx),
            None => window.focus(&self.focus, cx),
        }
        cx.notify();
    }

    fn palette(&self, state: Entity<CommandState>, cx: &mut Context<Self>) -> AnyElement {
        let products = self.products(cx);
        let mut items: Vec<CommandItem> = ENTRIES
            .iter()
            .map(|e| {
                CommandItem::new()
                    .label(e.label)
                    .icon(e.icon.clone())
                    .keywords(e.keywords.iter().copied())
                    .action((e.action)())
            })
            .collect();
        for (n, (_, name)) in products.iter().enumerate() {
            let item = CommandItem::new()
                .label(format!("Go to {name}"))
                .keywords(["product", "select"]);
            items.push(match actions::select_product(n) {
                Some(a) => item.action(a),
                None => item,
            });
        }
        let first_product = ENTRIES.len();
        let this = cx.weak_entity();
        let cancel = this.clone();
        div()
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
                            .items(items)
                            .placeholder("Type a command or a product")
                            .on_confirm(move |ix, window, cx| {
                                let row = ix.row;
                                let _ = this.update(cx, |this, cx| {
                                    // A product past the ninth has no action: the shell runs it here.
                                    let n = row.checked_sub(first_product);
                                    if let Some(n) =
                                        n.filter(|n| actions::select_product(*n).is_none())
                                        && let Some((product, _)) = products.get(n)
                                    {
                                        this.select_product(product, window, cx);
                                    }
                                    this.close_palette(window, cx);
                                });
                            })
                            .on_cancel(move |window, cx| {
                                let _ =
                                    cancel.update(cx, |this, cx| this.close_palette(window, cx));
                            }),
                    ),
            )
            .into_any_element()
    }

    /// Opens the question for `what`: the palette entries and, later, the menu entries use it.
    pub fn ask(&mut self, what: Ask, window: &mut Window, cx: &mut Context<Self>) {
        let data = LookGlobal::with(cx, LookData::clone).unwrap_or_default();
        let name_of = |p: &ProductRef, cx: &App| {
            self.cores
                .of(p.host.as_deref())
                .and_then(|c| c.read(cx).product(&p.id).map(|p| p.name.clone()))
                .unwrap_or_else(|| p.id.clone())
        };
        let colors = Choice::colors();
        let current = |hue: Option<Hue>| {
            hue.and_then(|h| colors.iter().position(|c| c.value == h.key()))
                .unwrap_or(0)
        };
        let prompt = match &what {
            Ask::ProductLabel(p) => {
                let now = data.product(p.host.as_deref(), &p.id).label;
                let title = format!("Label for {}", name_of(p, cx));
                cx.new(|cx| {
                    Prompt::text(
                        title,
                        "A label of 40 characters or less. Enter with no text clears it.",
                        &now.unwrap_or_default(),
                        Vec::new(),
                        window,
                        cx,
                    )
                })
            }
            Ask::ProductColor(p) => {
                let at = current(data.product(p.host.as_deref(), &p.id).hue);
                let title = format!("Color for {}", name_of(p, cx));
                cx.new(|cx| Prompt::pick(title, colors.clone(), at, window, cx))
            }
            Ask::ProductGroup(p) => {
                let now = data.group_of(p.host.as_deref(), &p.id).unwrap_or_default();
                let mut options = vec![Choice::new("No group", "")];
                options.extend(
                    data.group_names(p.host.as_deref())
                        .into_iter()
                        .map(|n| Choice::new(n.to_string(), n)),
                );
                let title = format!("Group for {}", name_of(p, cx));
                cx.new(|cx| {
                    Prompt::text(
                        title,
                        "Type a name to make a group. Down picks a group that exists.",
                        now,
                        options,
                        window,
                        cx,
                    )
                })
            }
            Ask::PaneLabel(p) => {
                let now = data.pane(p.host.as_deref(), &p.id).label;
                cx.new(|cx| {
                    Prompt::text(
                        "Name for this pane",
                        "A label of 40 characters or less. Enter with no text clears it.",
                        &now.unwrap_or_default(),
                        Vec::new(),
                        window,
                        cx,
                    )
                })
            }
            Ask::PaneColor(p) => {
                let at = current(data.pane(p.host.as_deref(), &p.id).hue);
                cx.new(|cx| Prompt::pick("Color for this pane", colors.clone(), at, window, cx))
            }
            other => match self
                .tab_prompt(other, &colors, window, cx)
                .or_else(|| self.menu_prompt(other, &colors, window, cx))
            {
                Some(p) => p,
                None => return,
            },
        };
        let answer = cx.subscribe_in(&prompt, window, |this, _, e: &PromptEvent, window, cx| {
            this.asked(e.clone(), window, cx)
        });
        self.asking = Some(Asking {
            what,
            prompt,
            _answer: answer,
        });
        cx.notify();
    }

    /// The question of the palette for the active product, or a notice when there is none.
    fn ask_product(
        &mut self,
        make: fn(ProductRef) -> Ask,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        match self.core.read(cx).active.clone() {
            Some(p) => self.ask(make(p), window, cx),
            None => self.notice("No active product. Pick one in the sidebar first.", cx),
        }
    }

    /// The question of the palette for the focused pane, or a notice when there is none.
    fn ask_pane(&mut self, make: fn(PaneRef) -> Ask, window: &mut Window, cx: &mut Context<Self>) {
        match self.focused_terminal(window, cx) {
            Some(p) => {
                let pane = PaneRef {
                    host: p.read(cx).host(cx),
                    id: p.read(cx).id.clone(),
                };
                self.ask(make(pane), window, cx)
            }
            None => self.notice("No pane is in view. Open a terminal first.", cx),
        }
    }

    fn notice(&mut self, text: &str, cx: &mut Context<Self>) {
        self.core.update(cx, |s, cx| {
            s.notices.push(text.to_string());
            cx.notify();
        });
    }

    /// The owner answered the question, or left it.
    fn asked(&mut self, event: PromptEvent, window: &mut Window, cx: &mut Context<Self>) {
        let Some(asking) = self.asking.take() else {
            return;
        };
        if let PromptEvent::Answer(text) = &event
            && (self.tab_answer(&asking.what, text, window, cx)
                || self.menu_answer(&asking.what, text, window, cx))
        {
            self.refocus(window, cx);
            return;
        }
        if let (PromptEvent::Answer(text), Some(look)) = (event, self.look.clone()) {
            look.update(cx, |l, cx| match &asking.what {
                Ask::ProductLabel(p) => {
                    l.set_product_label(&look::key(p.host.as_deref(), &p.id), &text, cx)
                }
                Ask::ProductColor(p) => l.set_product_hue(
                    &look::key(p.host.as_deref(), &p.id),
                    Hue::from_key(&text),
                    cx,
                ),
                Ask::ProductGroup(p) => l.move_product(p.host.as_deref(), &p.id, &text, cx),
                Ask::PaneLabel(p) => {
                    l.set_pane_label(&look::key(p.host.as_deref(), &p.id), &text, cx)
                }
                Ask::PaneColor(p) => l.set_pane_hue(
                    &look::key(p.host.as_deref(), &p.id),
                    Hue::from_key(&text),
                    cx,
                ),
                _ => {}
            });
        }
        self.refocus(window, cx);
    }

    /// The focus goes back to the last pane, else to the shell.
    fn refocus(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        match self.focused_terminal(window, cx) {
            Some(p) => window.focus(&p.focus_handle(cx), cx),
            None => window.focus(&self.focus, cx),
        }
        cx.notify();
    }

    fn select_nth(&mut self, n: usize, window: &mut Window, cx: &mut Context<Self>) {
        if let Some((product, _)) = self.products(cx).into_iter().nth(n) {
            self.select_product(&product, window, cx);
        }
    }
}

/// The program of a new shell. `remote` is true for a terminal on an SSH host.
fn shell_argv_for(remote: bool) -> Vec<String> {
    if remote {
        return ["/bin/sh", "-c", "exec \"${SHELL:-/bin/sh}\""]
            .map(String::from)
            .to_vec();
    }
    vec![std::env::var("SHELL").unwrap_or_else(|_| "/bin/sh".into())]
}

impl Render for Shell {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let empty = self.dock.read(cx).is_empty(DockPlacement::Center, cx);
        let (terminal_key, palette_key) = (
            keys::text(cx, "new-terminal"),
            keys::text(cx, keys::PALETTE),
        );
        let palette = self.palette.clone().map(|s| self.palette(s, cx));
        div()
            .id("shell")
            .track_focus(&self.focus)
            .key_context("Shell")
            .size_full()
            .flex()
            .relative()
            .bg(surface::panel())
            .text_color(text::primary())
            .font_family(font::UI)
            .text_size(size::base())
            .on_action(cx.listener(|this, _: &actions::TogglePalette, window, cx| this.toggle_palette(window, cx)))
            .on_action(cx.listener(|this, _: &actions::NewTerminal, window, cx| {
                this.open_terminal(this.shell_argv(cx), window, cx)
            }))
            .on_action(cx.listener(|this, _: &actions::NewClaude, window, cx| {
                this.open_terminal(vec!["claude".into()], window, cx)
            }))
            .on_action(cx.listener(|this, _: &actions::OpenDesk, window, cx| {
                if let Some(p) = this.core.read(cx).active.clone() {
                    this.select_product(&p, window, cx);
                }
            }))
            .on_action(cx.listener(|this, _: &actions::OpenMinutes, window, cx| {
                this.show_active(ScreenSpec::minutes, window, cx)
            }))
            .on_action(cx.listener(|this, _: &actions::OpenRoles, window, cx| {
                this.show_active(ScreenSpec::roles, window, cx)
            }))
            .on_action(cx.listener(|this, _: &actions::OpenStories, window, cx| {
                this.show_active(ScreenSpec::stories, window, cx)
            }))
            .on_action(cx.listener(|this, _: &actions::OpenTickets, window, cx| {
                this.show_active(ScreenSpec::tickets, window, cx)
            }))
            .on_action(cx.listener(|this, _: &actions::OpenGit, window, cx| {
                this.show_active(ScreenSpec::git, window, cx)
            }))
            .on_action(cx.listener(|this, _: &actions::UpdateBrief, window, cx| {
                this.show_active(ScreenSpec::setup, window, cx)
            }))
            .on_action(cx.listener(|this, _: &actions::NewProduct, window, cx| {
                let host = this.core.read(cx).active.as_ref().and_then(|p| p.host.clone());
                this.show_screen(ScreenSpec::setup("").on(host), window, cx)
            }))
            .on_action(cx.listener(|this, _: &actions::OpenMeeting, window, cx| {
                this.show_active(ScreenSpec::meeting, window, cx)
            }))
            .on_action(cx.listener(|this, _: &actions::OpenMemory, window, cx| {
                this.show_active(ScreenSpec::memory, window, cx)
            }))
            .on_action(cx.listener(|this, _: &actions::RetireMemoryItem, _, cx| {
                this.on_memory(cx, |s, cx| s.retire_selected(cx));
            }))
            .on_action(cx.listener(|this, _: &actions::RevertReflection, _, cx| {
                this.on_memory(cx, |s, cx| s.revert_latest(cx));
            }))
            .on_action(cx.listener(|this, _: &actions::ApproveProposal, _, cx| {
                this.on_memory(cx, |s, cx| s.decide_pending(Verdict::Approved, cx));
            }))
            .on_action(cx.listener(|this, _: &actions::RejectProposal, _, cx| {
                this.on_memory(cx, |s, cx| s.decide_pending(Verdict::Rejected, cx));
            }))
            .on_action(cx.listener(|this, _: &actions::OpenMeetingMinutes, window, cx| {
                if let Some(p) = this.core.read(cx).active.clone()
                    && let Some(core) = this.cores.of(p.host.as_deref())
                    && let Some(id) = meeting::minutes_of(&p.id, core, cx)
                {
                    this.show_screen(ScreenSpec::minute(&p.id, &id).on(p.host), window, cx);
                }
            }))
            .on_action(cx.listener(|this, _: &actions::OpenCall, window, cx| {
                this.open_call(window, cx)
            }))
            .on_action(cx.listener(|this, _: &actions::ReloadScreens, _, cx| this.reload_screens(cx)))
            .on_action(cx.listener(|this, _: &actions::OpenSettings, window, cx| {
                this.show_screen(ScreenSpec::settings(), window, cx)
            }))
            .on_action(cx.listener(|this, _: &actions::OpenDataSettings, window, cx| {
                this.show_screen(ScreenSpec::settings(), window, cx);
                for p in this.open_screens(cx) {
                    if p.read(cx).is(&ScreenSpec::settings()) {
                        p.update(cx, |p, cx| p.reveal_data(None, cx));
                    }
                }
            }))
            .on_action(cx.listener(|this, _: &actions::ShowMeetingContext, window, cx| {
                this.show_active(ScreenSpec::meeting, window, cx);
                this.on_active_meeting(window, cx, |p, _, cx| p.meeting_context(cx));
            }))
            .on_action(cx.listener(|this, _: &actions::AskMeetingRole, window, cx| {
                this.show_active(ScreenSpec::meeting, window, cx);
                this.on_active_meeting(window, cx, |p, window, cx| p.meeting_ask(window, cx));
            }))
            .on_action(cx.listener(|this, _: &actions::ToggleArchivedProducts, _, cx| {
                let host = this.core.read(cx).active.as_ref().and_then(|p| p.host.clone());
                this.sidebar
                    .update(cx, |s, cx| s.toggle_archived(host.as_deref(), cx));
            }))
            .on_action(cx.listener(|this, _: &actions::OpenOwnerSettings, window, cx| {
                this.show_screen(ScreenSpec::settings(), window, cx);
                for p in this.open_screens(cx) {
                    if p.read(cx).is(&ScreenSpec::settings()) {
                        p.update(cx, |p, cx| p.reveal_owner(cx));
                    }
                }
            }))
            .on_action(cx.listener(|this, _: &actions::CheckHealth, window, cx| {
                this.show_screen(ScreenSpec::settings(), window, cx);
                for p in this.open_screens(cx) {
                    if p.read(cx).is(&ScreenSpec::settings()) {
                        p.update(cx, |p, cx| p.check_health(cx));
                    }
                }
            }))
            .on_action(cx.listener(|this, _: &actions::BackUpData, window, cx| {
                this.show_screen(ScreenSpec::settings(), window, cx);
                for p in this.open_screens(cx) {
                    if p.read(cx).is(&ScreenSpec::settings()) {
                        p.update(cx, |p, cx| p.back_up_now(cx));
                    }
                }
            }))
            .on_action(cx.listener(|this, _: &actions::OpenGuide, window, cx| {
                this.show_screen(ScreenSpec::guide(), window, cx)
            }))
            .on_action(cx.listener(|this, _: &actions::OpenGuideStart, window, cx| {
                this.show_screen(ScreenSpec::guide(), window, cx);
                for p in this.open_screens(cx) {
                    if p.read(cx).is(&ScreenSpec::guide()) {
                        p.update(cx, |p, cx| p.reveal_section(crate::guide::INSTALL, cx));
                    }
                }
            }))
            .on_action(cx.listener(|this, _: &actions::LabelProduct, window, cx| {
                this.ask_product(Ask::ProductLabel, window, cx)
            }))
            .on_action(cx.listener(|this, _: &actions::ColorProduct, window, cx| {
                this.ask_product(Ask::ProductColor, window, cx)
            }))
            .on_action(cx.listener(|this, _: &actions::MoveProductToGroup, window, cx| {
                this.ask_product(Ask::ProductGroup, window, cx)
            }))
            .on_action(cx.listener(|this, _: &actions::RenamePane, window, cx| {
                this.ask_pane(Ask::PaneLabel, window, cx)
            }))
            .on_action(cx.listener(|this, _: &actions::ColorPane, window, cx| {
                this.ask_pane(Ask::PaneColor, window, cx)
            }))
            .on_action(cx.listener(|this, _: &actions::CloseOtherTabs, window, cx| {
                this.on_active_tab(window, cx, Self::close_others)
            }))
            .on_action(cx.listener(|this, _: &actions::CloseTabsToTheRight, window, cx| {
                this.on_active_tab(window, cx, Self::close_to_the_right)
            }))
            .on_action(cx.listener(|this, _: &actions::NewTabGroup, window, cx| {
                this.on_active_tab(window, cx, |this, p, window, cx| {
                    this.ask(Ask::NewTabGroup(p), window, cx)
                })
            }))
            .on_action(cx.listener(|this, _: &actions::AddTabToGroup, window, cx| {
                this.on_active_tab(window, cx, |this, p, window, cx| {
                    this.ask(Ask::AddToTabGroup(p), window, cx)
                })
            }))
            .on_action(cx.listener(|this, _: &actions::RemoveTabFromGroup, window, cx| {
                this.on_active_tab(window, cx, Self::leave_group)
            }))
            .on_action(cx.listener(|this, _: &actions::RenameTabGroup, window, cx| {
                this.on_active_group(window, cx, |this, g, window, cx| {
                    this.ask(Ask::RenameTabGroup(g), window, cx)
                })
            }))
            .on_action(cx.listener(|this, _: &actions::ColorTabGroup, window, cx| {
                this.on_active_group(window, cx, |this, g, window, cx| {
                    this.ask(Ask::ColorTabGroup(g), window, cx)
                })
            }))
            .on_action(cx.listener(|this, _: &actions::ToggleTabGroup, window, cx| {
                this.on_active_group(window, cx, Self::toggle_group)
            }))
            .on_action(cx.listener(|this, _: &actions::UngroupTabs, window, cx| {
                this.on_active_group(window, cx, Self::ungroup)
            }))
            .on_action(cx.listener(|this, _: &actions::CloseTabGroup, window, cx| {
                this.on_active_group(window, cx, Self::close_group)
            }))
            .on_action(cx.listener(|this, _: &actions::ShowAlerts, _, cx| {
                this.toggle_alert_log(cx)
            }))
            .on_action(cx.listener(|this, _: &actions::ToggleVoice, window, cx| {
                if let Some(p) = this.focused_terminal(window, cx) {
                    p.update(cx, |p, cx| p.toggle_voice(cx));
                }
            }))
            .on_action(cx.listener(|this, _: &actions::TalkToPane, window, cx| {
                if let Some(p) = this.focused_terminal(window, cx) {
                    p.update(cx, |p, cx| p.talk(cx));
                }
            }))
            .map(|el| menu_actions::handlers(el, cx))
            .on_action(cx.listener(|this, _: &actions::OpenContextMenu, window, cx| {
                this.open_context_menu(window, cx)
            }))
            .on_action(cx.listener(|this, _: &actions::Quit, window, cx| {
                this.before_quit(cx);
                window.remove_window();
            }))
            .on_action(cx.listener(|this, _: &actions::SelectProduct1, w, cx| this.select_nth(0, w, cx)))
            .on_action(cx.listener(|this, _: &actions::SelectProduct2, w, cx| this.select_nth(1, w, cx)))
            .on_action(cx.listener(|this, _: &actions::SelectProduct3, w, cx| this.select_nth(2, w, cx)))
            .on_action(cx.listener(|this, _: &actions::SelectProduct4, w, cx| this.select_nth(3, w, cx)))
            .on_action(cx.listener(|this, _: &actions::SelectProduct5, w, cx| this.select_nth(4, w, cx)))
            .on_action(cx.listener(|this, _: &actions::SelectProduct6, w, cx| this.select_nth(5, w, cx)))
            .on_action(cx.listener(|this, _: &actions::SelectProduct7, w, cx| this.select_nth(6, w, cx)))
            .on_action(cx.listener(|this, _: &actions::SelectProduct8, w, cx| this.select_nth(7, w, cx)))
            .on_action(cx.listener(|this, _: &actions::SelectProduct9, w, cx| this.select_nth(8, w, cx)))
            .when_some(
                self.bench
                    .as_ref()
                    // The start bench times the first paint of the front desk: from the
                    // moment that its first screen has its data.
                    .filter(|b| {
                        b.kind() != BenchKind::Start
                            || FIRST_SCREEN_READY.load(Ordering::Relaxed)
                    }),
                |el, b| el.child(b.marker()),
            )
            .child(self.sidebar.clone().cached(
                StyleRefinement::default().h_full().w(control::SIDEBAR_WIDTH).flex_none(),
            ))
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .h_full()
                    .relative()
                    .child(match self.plan.as_ref().and_then(|p| p.size) {
                        Some((w, h)) => div()
                            .w(Pixels::from(w))
                            .h(Pixels::from(h))
                            .child(self.dock.clone())
                            .into_any_element(),
                        None => self.dock.clone().into_any_element(),
                    })
                    .when(empty, |el| {
                        el.child(
                            div()
                                .absolute()
                                .top_0()
                                .left_0()
                                .size_full()
                                .flex()
                                .flex_col()
                                .items_center()
                                .justify_center()
                                .gap(space(2))
                                .text_color(text::secondary())
                                .child(format!(
                                    "Pick a product in the sidebar, or press {terminal_key} for a terminal."
                                ))
                                .child(
                                    div()
                                        .text_size(size::sm())
                                        .text_color(text::tertiary())
                                        .child(format!("{palette_key} lists each action.")),
                                ),
                        )
                    }),
            )
            .children(palette)
            .children(self.alert_log(cx))
            .children(self.asking.as_ref().map(|a| a.prompt.clone()))
            .children(self.menu_layer())
    }
}

#[cfg(test)]
mod tests {
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
    use agentsi_ui::tokens::label::Hue;
    use futures::channel::mpsc;
    use gpui::{Entity, Modifiers, Point, TestAppContext, VisualTestContext};
    use gpui_kit::AppContext as _;
    use gpui_kit::component::dock::{PanelInfo, PanelState};
    use serde_json::json;

    // No glob import: `super` brings `gpui::test` in as `test`, and `#[gpui::test]` then loops.
    use super::{Open, Persist, Shell, register_panels};
    use crate::actions;
    use crate::layout;
    use crate::link::{Link, LinkEvent};
    use crate::look::{self, Look};
    use crate::model::{CoreState, Cores, ProductRef};
    use crate::model::{StorySessionItem, two_cores::methods};
    use crate::screens::meeting::{MeetingFeed, Meetings};
    use crate::screens::{ScreenSpec, screen_of};
    use crate::tabs;
    use agentsi_ui::tokens::control;
    use gpui::{Pixels, WeakEntity};

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

    pub(super) fn folder(name: &str) -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!("agentsi-shell-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    pub(super) struct Fixture {
        pub(super) shell: Entity<Shell>,
        pub(super) core: Entity<CoreState>,
        pub(super) look: Entity<Look>,
        pub(super) open: Rc<RefCell<Open>>,
        /// What the core received.
        pub(super) core_end: UnixStream,
        pub(super) _events: mpsc::UnboundedSender<LinkEvent>,
    }

    /// A window with the real shell and sidebar over a fake core. Product `a` has the panes
    /// `t1-a` and `t2-a`, a running meeting, a story session and a reflection. Product `b`
    /// has the pane `t3-b`, which has no tab. `t1-a` and `t2-a` have tabs.
    pub(super) fn fixture<'a>(
        cx: &'a mut TestAppContext,
        persist: Persist,
        look_file: &std::path::Path,
    ) -> (Fixture, &'a mut VisualTestContext) {
        cx.update(agentsi_ui::init);
        cx.update(|cx| {
            crate::alerts::Alerts::install(cx);
        });
        cx.update(crate::actions::bind);
        let look = cx.update(|cx| look::install(cx, Some(look_file.to_path_buf())));
        let (end, core_end) = UnixStream::pair().unwrap();
        core_end.set_nonblocking(true).unwrap();
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
            s.put(terminal("t3-b", "b", "zsh"));
            s.set_sessions(
                "a",
                vec![StorySessionItem {
                    id: "S1".into(),
                    title: "Login".into(),
                    state: "work".into(),
                    owner_role: "dev".into(),
                }],
            );
            s.set_reflecting("a", true);
            s.active = Some(ProductRef {
                host: None,
                id: "a".into(),
            });
        });
        let cores = Cores::new(core.clone());
        let open = Rc::new(RefCell::new(Open::default()));
        cx.update(|cx| register_panels(&cores, &open, cx));
        let (tx, rx) = mpsc::unbounded();
        let o = open.clone();
        // The window has a Root, as the real window has: the toasts need it.
        let mut built = None;
        let window = cx.add_window(|window, cx| {
            let shell = cx.new(|cx| Shell::new(cores, rx, o, persist, None, None, window, cx));
            built = Some(shell.clone());
            gpui_kit::base::Root::new(shell, window, cx)
        });
        let shell = built.expect("the window builds the shell");
        let cx = VisualTestContext::from_window(*std::ops::Deref::deref(&window), cx).into_mut();
        cx.run_until_parked();
        cx.simulate_resize(gpui::size(control::WINDOW_WIDTH, control::WINDOW_HEIGHT));
        (
            Fixture {
                shell,
                core,
                look,
                open,
                core_end,
                _events: tx,
            },
            cx,
        )
    }

    pub(super) fn with_tabs(f: &Fixture, cx: &mut VisualTestContext) {
        for id in ["t1-a", "t2-a"] {
            let core = f.core.clone();
            f.shell.update_in(cx, |s, window, cx| {
                s.add_terminal_panel(id.into(), core, window, cx);
            });
        }
        redraw(cx);
    }

    /// The owner last used pane `id`. A test window is not active, so a pane gets no focus
    /// event that would record it.
    pub(super) fn use_pane(f: &Fixture, id: &str, cx: &mut VisualTestContext) {
        let panels: Vec<_> = f
            .open
            .borrow()
            .terminals
            .iter()
            .filter_map(WeakEntity::upgrade)
            .collect();
        for p in panels {
            p.update(cx, |p, _| {
                if p.id == id {
                    p.last_use = 1;
                }
            });
        }
    }

    pub(super) fn redraw(cx: &mut VisualTestContext) {
        cx.run_until_parked();
        cx.update(|window, cx| window.draw(cx).clear(cx));
    }

    pub(super) fn at(cx: &mut VisualTestContext, name: &str) -> Option<gpui::Bounds<Pixels>> {
        let name: &'static str = Box::leak(name.to_string().into_boxed_str());
        cx.debug_bounds(name)
    }

    pub(super) fn click(cx: &mut VisualTestContext, name: &str) {
        let b = at(cx, name).unwrap_or_else(|| panic!("{name} is drawn"));
        let center = Point::new(
            b.origin.x + b.size.width / 2.,
            b.origin.y + b.size.height / 2.,
        );
        cx.simulate_click(center, Modifiers::none());
        redraw(cx);
    }

    /// The tab that the dock displays, as `terminal:<id>` or `screen:<kind>[:<arg>]`.
    pub(super) fn active_tab(f: &Fixture, cx: &mut VisualTestContext) -> String {
        fn find(node: &PanelState) -> Option<String> {
            if let PanelInfo::Tabs { active_index } = &node.info {
                let tab = node.children.get(*active_index)?;
                if let Some(id) = layout::terminal_of(tab) {
                    return Some(format!("terminal:{id}"));
                }
                let spec = screen_of(tab)?;
                return Some(match spec.arg {
                    Some(a) => format!("screen:{}:{a}", spec.kind.name()),
                    None => format!("screen:{}", spec.kind.name()),
                });
            }
            node.children.iter().find_map(find)
        }
        let dump = f.shell.read_with(cx, |s, cx| s.dock.read(cx).dump(cx));
        find(&dump.center).unwrap_or_default()
    }

    /// The tabs of the first bar in order: a terminal id, or `screen:<kind>`.
    pub(super) fn order(f: &Fixture, cx: &mut VisualTestContext) -> Vec<String> {
        fn find(node: &PanelState) -> Option<Vec<String>> {
            if let PanelInfo::Tabs { .. } = &node.info {
                return Some(
                    node.children
                        .iter()
                        .map(|t| {
                            layout::terminal_of(t)
                                .map(String::from)
                                .or_else(|| {
                                    screen_of(t).map(|s| format!("screen:{}", s.kind.name()))
                                })
                                .unwrap_or_default()
                        })
                        .collect(),
                );
            }
            node.children.iter().find_map(find)
        }
        let dump = f.shell.read_with(cx, |s, cx| s.dock.read(cx).dump(cx));
        find(&dump.center).unwrap_or_default()
    }

    pub(super) fn terminal_panel(
        f: &Fixture,
        id: &str,
        cx: &mut VisualTestContext,
    ) -> gpui::EntityId {
        let found = f
            .open
            .borrow()
            .terminals
            .iter()
            .filter_map(WeakEntity::upgrade)
            .find(|p| p.read_with(cx, |p, _| p.id == id))
            .unwrap_or_else(|| panic!("{id} has a panel"));
        found.entity_id()
    }

    /// The selector of the tab of terminal `id`.
    pub(super) fn tab_of(f: &Fixture, id: &str, cx: &mut VisualTestContext) -> String {
        let panel = gpui_kit::component::dock::PanelId::from(terminal_panel(f, id, cx));
        format!("tab:{}", panel.as_u64())
    }

    pub(super) fn panel_of(
        f: &Fixture,
        id: &str,
        cx: &mut VisualTestContext,
    ) -> gpui_kit::component::dock::PanelId {
        gpui_kit::component::dock::PanelId::from(terminal_panel(f, id, cx))
    }

    /// The selector of a tab by name: a terminal id, `screen:<kind>` or `chip`.
    fn sel(f: &Fixture, who: &str, cx: &mut VisualTestContext) -> String {
        match who {
            "chip" => "tab-chip:group-0".to_string(),
            w if w.starts_with("screen:") => screen_tab(f, &w["screen:".len()..], cx),
            w => tab_of(f, w, cx),
        }
    }

    /// Where the tab `who` is drawn, if it is.
    fn tab_at(f: &Fixture, who: &str, cx: &mut VisualTestContext) -> Option<gpui::Bounds<Pixels>> {
        let name = sel(f, who, cx);
        at(cx, &name)
    }

    /// Drags the tab `from` to the tab `to`, by name as `sel` reads it.
    fn drag_tab(f: &Fixture, cx: &mut VisualTestContext, from: &str, to: &str) {
        let (a, b) = (sel(f, from, cx), sel(f, to, cx));
        drag(cx, &a, &b);
    }

    /// Presses on `from`, moves to `to` and releases.
    fn drag(cx: &mut VisualTestContext, from: &str, to: &str) {
        let (a, b) = (
            at(cx, from)
                .unwrap_or_else(|| panic!("{from} is drawn"))
                .center(),
            at(cx, to)
                .unwrap_or_else(|| panic!("{to} is drawn"))
                .center(),
        );
        let held = gpui::MouseButton::Left;
        cx.simulate_mouse_down(a, held, Modifiers::none());
        cx.simulate_mouse_move(
            Point::new(a.x + Pixels::from(8.), a.y),
            held,
            Modifiers::none(),
        );
        redraw(cx);
        cx.simulate_mouse_move(b, held, Modifiers::none());
        redraw(cx);
        cx.simulate_mouse_up(b, held, Modifiers::none());
        redraw(cx);
    }

    /// Four tabs in one bar: `t1-a`, `t2-a`, `t3-b` and the minutes of product a.
    pub(super) fn four_tabs(f: &Fixture, cx: &mut VisualTestContext) {
        with_tabs(f, cx);
        let core = f.core.clone();
        f.shell.update_in(cx, |s, window, cx| {
            s.add_terminal_panel("t3-b".into(), core, window, cx);
            s.show_screen(ScreenSpec::minutes("a"), window, cx);
        });
        redraw(cx);
    }

    pub(super) fn make_group(f: &Fixture, id: &str, name: &str, cx: &mut VisualTestContext) {
        let tab = tab_of(f, id, cx);
        click(cx, &tab);
        cx.dispatch_action(actions::NewTabGroup);
        redraw(cx);
        cx.simulate_input(name);
        cx.simulate_keystrokes("enter");
        redraw(cx);
    }

    #[gpui::test]
    fn a_drag_reorders_a_bar_and_joins_a_tab_group_and_layout_json_keeps_both(
        cx: &mut TestAppContext,
    ) {
        let dir = folder("drag");
        let (layout_file, look_file) = (dir.join("layout.json"), dir.join("look.toml"));
        let (f, cx) = fixture(cx, Persist::File(layout_file.clone()), &look_file);
        four_tabs(&f, cx);
        assert_eq!(order(&f, cx), ["t1-a", "t2-a", "t3-b", "screen:minutes"]);
        // Forward in one bar: the tab lands before the tab it was dropped on.
        drag_tab(&f, cx, "t1-a", "t3-b");
        assert_eq!(order(&f, cx), ["t2-a", "t1-a", "t3-b", "screen:minutes"]);
        // Backward in one bar.
        drag_tab(&f, cx, "t3-b", "t2-a");
        assert_eq!(order(&f, cx), ["t3-b", "t2-a", "t1-a", "screen:minutes"]);
        // A tab group starts with t2-a. A tab dropped on its chip joins it, at its end.
        make_group(&f, "t2-a", "Build", cx);
        assert!(at(cx, "tab-chip:group-0").is_some(), "the chip is drawn");
        let bar = format!("tab-group-bar:{}", panel_of(&f, "t2-a", cx).as_u64());
        assert!(at(cx, &bar).is_some());
        drag_tab(&f, cx, "t3-b", "chip");
        assert_eq!(order(&f, cx), ["t2-a", "t3-b", "t1-a", "screen:minutes"]);
        // A tab dropped on a tab of the group joins the group before that tab.
        drag_tab(&f, cx, "t1-a", "t3-b");
        assert_eq!(order(&f, cx), ["t2-a", "t1-a", "t3-b", "screen:minutes"]);
        let members = |f: &Fixture, cx: &mut VisualTestContext| {
            f.shell.read_with(cx, |s, cx| {
                s.tabs
                    .read(cx)
                    .groups()
                    .iter()
                    .map(|g| g.members.len())
                    .collect::<Vec<_>>()
            })
        };
        assert_eq!(members(&f, cx), [3]);
        // A tab dropped on a tab outside the group leaves it.
        drag_tab(&f, cx, "t1-a", "screen:minutes");
        assert_eq!(order(&f, cx), ["t2-a", "t3-b", "t1-a", "screen:minutes"]);
        assert_eq!(members(&f, cx), [2]);
        // A drop past the last tab takes the tab out of its group and puts it last.
        let t2 = sel(&f, "t2-a", cx);
        drag(cx, &t2, "tab-bar-end");
        assert_eq!(order(&f, cx), ["t3-b", "t1-a", "screen:minutes", "t2-a"]);
        assert_eq!(members(&f, cx), [1]);
        drag_tab(&f, cx, "t2-a", "chip");
        assert_eq!(order(&f, cx), ["t3-b", "t2-a", "t1-a", "screen:minutes"]);
        assert_eq!(members(&f, cx), [2]);
        // The file keeps the order of the tabs and the group with its tabs in that order.
        f.shell.read_with(cx, |s, cx| s.save_now(&layout_file, cx));
        let saved = layout::read(&layout_file).unwrap().unwrap();
        assert_eq!(layout::terminals(&saved.dock), ["t3-b", "t2-a", "t1-a"]);
        assert_eq!(saved.tab_groups.len(), 1);
        let g = &saved.tab_groups[0];
        assert_eq!((g.name.as_str(), g.collapsed), ("Build", false));
        assert!(g.color.is_some());
        let key = |id: &str| {
            layout::panel_key(&PanelState {
                panel_name: layout::TERMINAL_PANEL.into(),
                children: vec![],
                info: layout::terminal_info(id, None),
            })
        };
        assert_eq!(g.tabs, [key("t3-b").unwrap(), key("t2-a").unwrap()]);
    }

    /// How many times the core received `method`: waits up to a second for `want` calls, because
    /// the link writes on its own thread.
    pub(super) fn calls(core_end: &mut UnixStream, method: &str, want: usize) -> usize {
        let mut seen = 0;
        for _ in 0..50 {
            seen += methods(core_end).iter().filter(|m| *m == method).count();
            if seen >= want {
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(20));
        }
        seen
    }

    pub(super) fn groups(f: &Fixture, cx: &mut VisualTestContext) -> Vec<crate::tabs::Group> {
        f.shell
            .read_with(cx, |s, cx| s.tabs.read(cx).groups().to_vec())
    }

    #[gpui::test]
    fn a_tab_group_its_color_and_its_collapse_survive_a_restart(cx: &mut TestAppContext) {
        let dir = folder("groups-restart");
        let (layout_file, look_file) = (dir.join("layout.json"), dir.join("look.toml"));
        {
            let (f, cx) = fixture(cx, Persist::File(layout_file.clone()), &look_file);
            four_tabs(&f, cx);
            f.shell.update(cx, |s, _| s.restored = true);
            make_group(&f, "t2-a", "Build", cx);
            drag_tab(&f, cx, "t3-b", "chip");
            // The palette colors the group and a click on the chip collapses it.
            cx.dispatch_action(actions::ColorTabGroup);
            redraw(cx);
            click(cx, "prompt-choice:sky");
            let tab = tab_of(&f, "t3-b", cx);
            click(cx, &tab);
            assert_eq!(active_tab(&f, cx), "terminal:t3-b");
            click(cx, "tab-chip:group-0");
            let shown = active_tab(&f, cx);
            assert!(
                shown != "terminal:t3-b" && shown != "terminal:t2-a",
                "the tab on screen moves out of the collapsed group: {shown}"
            );
            assert!(
                at(cx, "tab-chip:group-0:count").is_some(),
                "a collapsed chip shows a count"
            );
            assert!(
                tab_at(&f, "t2-a", cx).is_none(),
                "the tabs of the group are hidden"
            );
            assert!(tab_at(&f, "t1-a", cx).is_some(), "the other tabs stay");
            f.shell.read_with(cx, |s, cx| s.save_now(&layout_file, cx));
        }
        let saved = layout::read(&layout_file).unwrap().unwrap();
        assert_eq!(saved.version, layout::VERSION);
        assert_eq!(saved.tab_groups.len(), 1);
        assert!(saved.tab_groups[0].collapsed);
        assert_eq!(saved.tab_groups[0].color.as_deref(), Some("sky"));
        // A new window over the same files.
        let (f, cx) = fixture(cx, Persist::File(layout_file.clone()), &look_file);
        f.shell.update_in(cx, |s, window, cx| s.restore(window, cx));
        redraw(cx);
        let groups = groups(&f, cx);
        assert_eq!(groups.len(), 1);
        assert_eq!(
            (
                groups[0].name.as_str(),
                groups[0].hue,
                groups[0].collapsed,
                groups[0].members.len()
            ),
            ("Build", Some(Hue::Sky), true, 2)
        );
        assert!(at(cx, "tab-chip:group-0:count").is_some());
        // The click opens it, and the tabs are in the order of the file.
        click(cx, "tab-chip:group-0");
        assert!(tab_at(&f, "t2-a", cx).is_some());
        assert_eq!(
            order(&f, cx).len(),
            4,
            "the four tabs of the file came back"
        );
        let chip = at(cx, "tab-chip:group-0").unwrap();
        assert!(tab_at(&f, "t2-a", cx).unwrap().left() >= chip.right());
    }

    #[gpui::test]
    fn the_layout_of_version_1_starts_the_app_with_no_tab_group(cx: &mut TestAppContext) {
        let dir = folder("v1");
        let (layout_file, look_file) = (dir.join("layout.json"), dir.join("look.toml"));
        {
            let (f, cx) = fixture(cx, Persist::File(layout_file.clone()), &look_file);
            with_tabs(&f, cx);
            f.shell.read_with(cx, |s, cx| s.save_now(&layout_file, cx));
        }
        // Make the file the one that the app wrote before tab groups.
        let text = std::fs::read_to_string(&layout_file).unwrap();
        std::fs::write(
            &layout_file,
            text.replace("\"version\": 2", "\"version\": 1"),
        )
        .unwrap();
        let (f, cx) = fixture(cx, Persist::File(layout_file), &look_file);
        f.shell.update_in(cx, |s, window, cx| s.restore(window, cx));
        redraw(cx);
        assert_eq!(order(&f, cx), ["t1-a", "t2-a"]);
        assert!(groups(&f, cx).is_empty());
    }

    #[gpui::test]
    fn the_tab_actions_of_the_palette_name_rename_color_ungroup_and_close_a_group(
        cx: &mut TestAppContext,
    ) {
        let file = folder("actions").join("look.toml");
        let (mut f, cx) = fixture(cx, Persist::None, &file);
        four_tabs(&f, cx);
        // The programs of the terminals ended, so no Confirm comes.
        f.core.update(cx, |s, _| {
            for t in s.terminals.values_mut() {
                t.running = false;
            }
        });
        make_group(&f, "t2-a", "Build", cx);
        // The second tab joins through the picker of the bar.
        let tab = tab_of(&f, "t3-b", cx);
        click(cx, &tab);
        cx.dispatch_action(actions::AddTabToGroup);
        redraw(cx);
        click(cx, "prompt-choice:0");
        assert_eq!(order(&f, cx), ["t1-a", "t2-a", "t3-b", "screen:minutes"]);
        assert_eq!(groups(&f, cx)[0].members.len(), 2);
        cx.dispatch_action(actions::RenameTabGroup);
        redraw(cx);
        cx.simulate_input(" Docs");
        cx.simulate_keystrokes("enter");
        redraw(cx);
        assert_eq!(groups(&f, cx)[0].name, "Build Docs");
        // The tab leaves its group and stays after it.
        cx.dispatch_action(actions::RemoveTabFromGroup);
        redraw(cx);
        assert_eq!(groups(&f, cx)[0].members.len(), 1);
        assert_eq!(order(&f, cx), ["t1-a", "t2-a", "t3-b", "screen:minutes"]);
        // Close to the right of t2-a: t3-b and the minutes.
        let tab = tab_of(&f, "t2-a", cx);
        click(cx, &tab);
        let _ = methods(&mut f.core_end);
        cx.dispatch_action(actions::CloseTabsToTheRight);
        redraw(cx);
        assert_eq!(order(&f, cx), ["t1-a", "t2-a"]);
        assert_eq!(calls(&mut f.core_end, "terminal.kill", 1), 1);
        // Ungroup keeps the tab. A group that is closed takes its tabs along.
        cx.dispatch_action(actions::UngroupTabs);
        redraw(cx);
        assert!(groups(&f, cx).is_empty());
        assert_eq!(order(&f, cx), ["t1-a", "t2-a"]);
        make_group(&f, "t2-a", "Again", cx);
        cx.dispatch_action(actions::CloseTabGroup);
        redraw(cx);
        assert_eq!(order(&f, cx), ["t1-a"]);
        assert!(
            groups(&f, cx).is_empty(),
            "the group went with its last tab"
        );
    }

    #[gpui::test]
    fn closing_tabs_that_run_a_program_asks_one_confirm(cx: &mut TestAppContext) {
        let file = folder("confirm").join("look.toml");
        let (mut f, cx) = fixture(cx, Persist::None, &file);
        four_tabs(&f, cx);
        let tab = tab_of(&f, "t2-a", cx);
        click(cx, &tab);
        let _ = methods(&mut f.core_end);
        // t1-a, t3-b and the minutes close: two terminals run a program, so one Confirm comes.
        cx.dispatch_action(actions::CloseOtherTabs);
        redraw(cx);
        assert!(at(cx, "prompt").is_some(), "the Confirm is open");
        assert_eq!(order(&f, cx).len(), 4, "nothing closed before the answer");
        // Keep them.
        click(cx, "prompt-choice:keep");
        assert!(at(cx, "prompt").is_none());
        assert_eq!(order(&f, cx).len(), 4);
        assert_eq!(calls(&mut f.core_end, "terminal.kill", 0), 0);
        // Close them: one Confirm, and each program stops.
        cx.dispatch_action(actions::CloseOtherTabs);
        redraw(cx);
        click(cx, "prompt-choice:close");
        assert!(at(cx, "prompt").is_none());
        assert_eq!(order(&f, cx), ["t2-a"]);
        assert_eq!(calls(&mut f.core_end, "terminal.kill", 2), 2);
    }

    #[gpui::test]
    fn a_tab_that_closes_or_moves_to_another_bar_leaves_its_group(cx: &mut TestAppContext) {
        let file = folder("eject").join("look.toml");
        let (f, cx) = fixture(cx, Persist::None, &file);
        four_tabs(&f, cx);
        make_group(&f, "t2-a", "Build", cx);
        drag_tab(&f, cx, "t3-b", "chip");
        drag_tab(&f, cx, "t1-a", "chip");
        assert_eq!(groups(&f, cx)[0].members.len(), 3);
        // The close button of a tab takes it out of the group.
        let close = format!("tab-close:{}", panel_of(&f, "t1-a", cx).as_u64());
        let tab = tab_of(&f, "t1-a", cx);
        click(cx, &tab);
        click(cx, &close);
        assert_eq!(groups(&f, cx)[0].members.len(), 2);
        // A tab that the dock moves to another bar leaves the group at the next change.
        let moved = panel_of(&f, "t3-b", cx);
        f.shell.update_in(cx, |s, window, cx| {
            let node = tabs::bars(s.dock.read(cx))[0].node;
            s.dock.update(cx, |d, cx| {
                d.move_panel(
                    moved,
                    gpui_kit::component::dock::InsertTarget::Split {
                        node,
                        placement: gpui_kit::base::Placement::Right,
                        size: None,
                    },
                    window,
                    cx,
                )
            });
        });
        redraw(cx);
        let bars = f
            .shell
            .read_with(cx, |s, cx| tabs::bars(s.dock.read(cx)).len());
        assert_eq!(bars, 2, "the tab has a bar of its own");
        let groups = groups(&f, cx);
        assert_eq!(groups.len(), 1);
        assert_eq!(groups[0].members.len(), 1, "t2-a stays alone in the group");
        assert!(groups[0].members.contains(&panel_of(&f, "t2-a", cx)));
    }

    /// The selector of the tab of the screen `kind` of product a.
    fn screen_tab(f: &Fixture, kind: &str, cx: &mut VisualTestContext) -> String {
        let found = f
            .open
            .borrow()
            .screens
            .iter()
            .filter_map(WeakEntity::upgrade)
            .find(|p| p.read_with(cx, |p, _| p.kind.name() == kind))
            .unwrap_or_else(|| panic!("{kind} has a panel"));
        format!(
            "tab:{}",
            gpui_kit::component::dock::PanelId::from(found.entity_id()).as_u64()
        )
    }

    #[gpui::test]
    fn a_click_on_each_item_row_focuses_its_tab_and_a_closed_tab_attaches(cx: &mut TestAppContext) {
        let look = folder("click").join("look.toml");
        let (mut f, cx) = fixture(cx, Persist::None, &look);
        with_tabs(&f, cx);
        // The last tab that was added shows.
        assert_eq!(active_tab(&f, cx), "terminal:t2-a");
        click(cx, "tree-row:product-a-pane-t1-a");
        assert_eq!(active_tab(&f, cx), "terminal:t1-a");
        click(cx, "tree-row:product-a-pane-t2-a");
        assert_eq!(active_tab(&f, cx), "terminal:t2-a");
        click(cx, "tree-row:product-a-meeting");
        assert_eq!(active_tab(&f, cx), "screen:meeting");
        click(cx, "tree-row:product-a-story-S1");
        assert_eq!(active_tab(&f, cx), "screen:story:S1");
        click(cx, "tree-row:product-a-reflection");
        assert_eq!(active_tab(&f, cx), "screen:memory");
        // The pane of product b has no tab: the click opens one that attaches to the same terminal.
        let before = f.open.borrow().terminals.len();
        let _ = methods(&mut f.core_end);
        click(cx, "tree-row:product-b-pane-t3-b");
        assert_eq!(active_tab(&f, cx), "terminal:t3-b");
        assert_eq!(f.open.borrow().terminals.len(), before + 1);
        assert!(methods(&mut f.core_end).contains(&"terminal.attach".to_string()));
        // A click on the pane row of a tab that exists opens no second tab.
        click(cx, "tree-row:product-b-pane-t3-b");
        assert_eq!(f.open.borrow().terminals.len(), before + 1);
    }

    #[gpui::test]
    fn a_label_and_a_color_from_the_palette_show_on_the_tab_the_item_row_and_the_file(
        cx: &mut TestAppContext,
    ) {
        let file = folder("label").join("look.toml");
        let (f, cx) = fixture(cx, Persist::None, &file);
        with_tabs(&f, cx);
        // Label the active product.
        cx.dispatch_action(actions::LabelProduct);
        redraw(cx);
        assert!(at(cx, "prompt").is_some(), "the question is open");
        cx.simulate_input("api");
        cx.simulate_keystrokes("enter");
        redraw(cx);
        assert!(at(cx, "prompt").is_none(), "the answer closes the question");
        assert!(at(cx, "tree-row:product-a:label").is_some());
        // Color the active product.
        cx.dispatch_action(actions::ColorProduct);
        redraw(cx);
        click(cx, "prompt-choice:teal");
        assert!(at(cx, "prompt").is_none());
        assert!(at(cx, "tree-row:product-a:bar").is_some());
        // The tabs of the product show its color.
        assert!(at(cx, "tab-color:t1-a").is_some());
        // Rename and color the pane in view (t2-a).
        use_pane(&f, "t2-a", cx);
        cx.dispatch_action(actions::RenamePane);
        redraw(cx);
        cx.simulate_input("build");
        cx.simulate_keystrokes("enter");
        redraw(cx);
        cx.dispatch_action(actions::ColorPane);
        redraw(cx);
        click(cx, "prompt-choice:pink");
        let shown = f
            .core
            .read_with(cx, |c, cx| crate::pane::shown(c, "t2-a", cx));
        assert_eq!(shown.title, "build");
        assert_eq!(
            shown.tooltip.as_deref(),
            Some("zsh"),
            "the program title stays in the tooltip"
        );
        assert_eq!(shown.hue, Some(Hue::Pink));
        let other = f
            .core
            .read_with(cx, |c, cx| crate::pane::shown(c, "t1-a", cx));
        assert_eq!(
            (other.title.as_str(), other.hue),
            ("claude", Some(Hue::Teal))
        );
        assert!(at(cx, "tree-row:product-a-pane-t2-a:bar").is_some());
        assert!(at(cx, "tab-color:t2-a").is_some());
        let (data, note) = look::read(&file);
        assert_eq!(note, None);
        assert_eq!(data.product(None, "a").label.as_deref(), Some("api"));
        assert_eq!(data.product(None, "a").hue, Some(Hue::Teal));
        assert_eq!(data.pane(None, "t2-a").label.as_deref(), Some("build"));
        assert_eq!(data.pane(None, "t2-a").hue, Some(Hue::Pink));
        assert_eq!(f.look.read_with(cx, |l, _| l.data.clone()), data);
    }

    #[gpui::test]
    fn the_group_prompt_makes_a_group_and_escape_leaves_every_prompt_unchanged(
        cx: &mut TestAppContext,
    ) {
        let file = folder("group").join("look.toml");
        let (f, cx) = fixture(cx, Persist::None, &file);
        cx.dispatch_action(actions::MoveProductToGroup);
        redraw(cx);
        cx.simulate_input("Work");
        cx.simulate_keystrokes("enter");
        redraw(cx);
        assert!(at(cx, "tree-row:group-Work").is_some());
        assert_eq!(look::read(&file).0.group_of(None, "a"), Some("Work"));
        // A second question lists the group. Escape changes nothing.
        cx.dispatch_action(actions::MoveProductToGroup);
        redraw(cx);
        assert!(at(cx, "prompt-choice:Work").is_some());
        cx.simulate_keystrokes("escape");
        redraw(cx);
        assert!(at(cx, "prompt").is_none());
        assert_eq!(look::read(&file).0.group_of(None, "a"), Some("Work"));
        // "No group" takes the product out.
        cx.dispatch_action(actions::MoveProductToGroup);
        redraw(cx);
        click(cx, "prompt-choice:none");
        assert_eq!(look::read(&file).0.group_of(None, "a"), None);
        // With no active product the entry says so and asks nothing.
        f.core.update(cx, |s, _| s.active = None);
        cx.dispatch_action(actions::LabelProduct);
        redraw(cx);
        assert!(at(cx, "prompt").is_none());
        assert!(f.core.read_with(cx, |s, _| {
            s.notices.iter().any(|n| n.contains("No active product"))
        }));
    }

    #[gpui::test]
    fn the_product_numbers_follow_the_order_of_the_sidebar(cx: &mut TestAppContext) {
        let file = folder("order").join("look.toml");
        let (f, cx) = fixture(cx, Persist::None, &file);
        let ids = |f: &Fixture, cx: &mut VisualTestContext| {
            f.shell.read_with(cx, |s, cx| {
                s.products(cx)
                    .into_iter()
                    .map(|(p, _)| p.id)
                    .collect::<Vec<_>>()
            })
        };
        assert_eq!(ids(&f, cx), ["a", "b"]);
        f.look
            .update(cx, |l, cx| l.move_product(None, "a", "Work", cx));
        // The product with no group comes first, then the group, as in the sidebar.
        assert_eq!(ids(&f, cx), ["b", "a"]);
    }

    #[gpui::test]
    fn the_open_state_the_groups_and_the_look_survive_a_restart(cx: &mut TestAppContext) {
        let dir = folder("restart");
        let (layout_file, look_file) = (dir.join("layout.json"), dir.join("look.toml"));
        std::fs::write(
            &look_file,
            "mystery = true\n\n[panes.t9-gone]\nlabel = \"old\"\n\n[panes.t1-a]\nlabel = \"keep\"\n",
        )
        .unwrap();
        // First run: close product a, make a group, save.
        {
            let (f, cx) = fixture(cx, Persist::File(layout_file.clone()), &look_file);
            with_tabs(&f, cx);
            f.shell.update(cx, |s, _| s.restored = true);
            click(cx, "tree-row:product-a:arrow");
            f.look.update(cx, |l, cx| {
                l.move_product(None, "b", "Work", cx);
                l.set_group_hue(None, "Work", Some(Hue::Sky), cx);
            });
            // `before_quit` would set the process-wide QUITTING flag, and a terminal tab of a
            // test that runs at the same time would not kill its terminal.
            f.shell.read_with(cx, |s, cx| s.save_now(&layout_file, cx));
            assert!(layout_file.exists());
        }
        let saved = layout::read(&layout_file).unwrap().unwrap();
        assert_eq!(saved.tree, BTreeMap::from([("a".to_string(), false)]));
        // Second run: a new window over the same files.
        let (f, cx) = fixture(cx, Persist::File(layout_file.clone()), &look_file);
        f.shell.update_in(cx, |s, window, cx| s.restore(window, cx));
        redraw(cx);
        assert!(
            !f.shell
                .read_with(cx, |s, cx| s.sidebar.read(cx).is_open(&ProductRef {
                    host: None,
                    id: "a".into()
                }))
        );
        assert!(
            at(cx, "tree-row:product-a-pane-t1-a").is_none(),
            "product a stays closed"
        );
        assert!(at(cx, "running-chip:a").is_some());
        let header = at(cx, "tree-row:group-Work").expect("the group came back");
        assert!(at(cx, "tree-row:product-b").unwrap().top() >= header.bottom());
        // The pane entry of the gone terminal went. The entry of a known one and the unknown entry stay.
        let text = std::fs::read_to_string(&look_file).unwrap();
        assert!(text.contains("mystery = true"), "{text}");
        assert!(!text.contains("t9-gone"), "{text}");
        assert!(text.contains("label = \"keep\""), "{text}");
        let _ = &f.look;
    }

    /// The top of the page of the guide screen that the dock shows, if one is open.
    fn guide_top(f: &Fixture, cx: &mut VisualTestContext) -> Option<usize> {
        let screens = f.shell.read_with(cx, |s, cx| s.open_screens(cx));
        screens.iter().find_map(|p| {
            let guide = p.read_with(cx, |p, _| p.guide())?;
            Some(guide.read_with(cx, |g, cx| g.top(cx)))
        })
    }

    #[gpui::test]
    fn the_palette_entry_of_the_guide_opens_the_guide_screen_at_the_top(cx: &mut TestAppContext) {
        let file = folder("guide-top").join("look.toml");
        let (f, cx) = fixture(cx, Persist::None, &file);
        with_tabs(&f, cx);
        let tabs = order(&f, cx).len();
        cx.dispatch_action(actions::OpenGuide);
        redraw(cx);
        redraw(cx);
        assert_eq!(active_tab(&f, cx), "screen:guide");
        assert!(at(cx, "guide-root").is_some(), "the page is drawn");
        assert_eq!(order(&f, cx).len(), tabs + 1);
        assert_eq!(guide_top(&f, cx), Some(0));
        assert_eq!(cx.opened_url(), None, "the browser stays closed");
        // The page keys reach the page through the shell.
        cx.simulate_keystrokes("pagedown");
        redraw(cx);
        assert!(
            guide_top(&f, cx).is_some_and(|ix| ix > 0),
            "Page Down scrolled"
        );
        cx.simulate_keystrokes("home");
        redraw(cx);
        assert_eq!(guide_top(&f, cx), Some(0), "Home went to the top");
        // A second entry selects the same tab.
        cx.dispatch_action(actions::OpenGuide);
        redraw(cx);
        assert_eq!(order(&f, cx).len(), tabs + 1);
    }

    #[gpui::test]
    fn the_first_steps_entry_opens_the_guide_scrolled_to_install_and_start(
        cx: &mut TestAppContext,
    ) {
        let file = folder("guide-start").join("look.toml");
        let (f, cx) = fixture(cx, Persist::None, &file);
        with_tabs(&f, cx);
        cx.dispatch_action(actions::OpenGuideStart);
        for _ in 0..3 {
            redraw(cx);
        }
        assert_eq!(active_tab(&f, cx), "screen:guide");
        assert!(
            guide_top(&f, cx).is_some_and(|ix| ix > 0),
            "the page is past its top"
        );
        assert_eq!(cx.opened_url(), None);
    }

    #[gpui::test]
    fn a_request_of_the_settings_line_opens_the_guide_screen(cx: &mut TestAppContext) {
        let file = folder("guide-settings").join("look.toml");
        let (f, cx) = fixture(cx, Persist::None, &file);
        with_tabs(&f, cx);
        cx.update(|_, cx| crate::nav::Nav::open(cx, &f.core, ScreenSpec::guide()));
        redraw(cx);
        redraw(cx);
        assert_eq!(active_tab(&f, cx), "screen:guide");
    }
}
