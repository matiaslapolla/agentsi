//! The spaces: one dock for each product, and Home for the tabs of no product.
//!
//! The window draws the dock of one space, the shown one. The other docks stay alive with their
//! panels, so a terminal of a hidden space keeps running and keeps its stream. `Shell::dock` and
//! `Shell::tabs` are the dock and the tab groups of the shown space.
//!
//! The dock of a space tells its panels nothing when the space hides, so `show_space` tells the
//! tab on screen of each bar: a hidden screen must not count as seen by the alerts or the
//! palette.

use std::collections::{BTreeMap, BTreeSet};

use gpui_kit::component::dock::{DockArea, DockEvent, DockPlacement, PanelId, PanelState};
use gpui_kit::*;

use super::{DOCK_VERSION, Shell};
use crate::layout::{self, SpaceSave, TabGroupSave};
use crate::model::ProductRef;
use crate::screens::screen_of;
use crate::tab_bar;
use crate::tabs::{self, SpaceId, TabGroups, TabsChanged};
use crate::tree::{ItemKey, TabRow};

pub(super) struct Space {
    pub dock: Entity<DockArea>,
    pub tabs: Entity<TabGroups>,
    _subscriptions: Vec<Subscription>,
}

impl Space {
    /// A new space with an empty dock, wired to the shell.
    pub(super) fn new(id: &SpaceId, window: &mut Window, cx: &mut Context<Shell>) -> Space {
        let tabs = cx.new(|_| TabGroups::default());
        let dock_id = format!("agentsi-{}", id.key());
        let (dock, skin) =
            tab_bar::dock_area(dock_id, Some(DOCK_VERSION), tabs.clone(), window, cx);
        skin.set_close_button_visible(true, cx);
        let groups = tabs.clone();
        let area = dock.downgrade();
        let subscriptions = vec![
            cx.subscribe_in(
                &dock,
                window,
                move |this, dock, e: &DockEvent, window, cx| {
                    if matches!(e, DockEvent::LayoutChanged) {
                        tabs::reconcile(dock, &groups, window, cx);
                        this.spaces_changed(cx);
                    }
                },
            ),
            cx.subscribe(&tabs, move |this, _, _: &TabsChanged, cx| {
                // The bars read the groups when they draw: tell the dock to draw again.
                if let Some(dock) = area.upgrade() {
                    dock.update(cx, |_, cx| cx.notify());
                }
                this.spaces_changed(cx);
            }),
        ];
        Space {
            dock,
            tabs,
            _subscriptions: subscriptions,
        }
    }
}

impl Shell {
    /// The dock and the tab groups of space `id`, made on first use.
    pub(super) fn space(
        &mut self,
        id: &SpaceId,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> (Entity<DockArea>, Entity<TabGroups>) {
        if !self.spaces.contains_key(id) {
            let space = Space::new(id, window, cx);
            self.spaces.insert(id.clone(), space);
        }
        let s = &self.spaces[id];
        (s.dock.clone(), s.tabs.clone())
    }

    #[cfg(test)]
    pub fn shown_space(&self) -> &SpaceId {
        &self.shown
    }

    /// Draws the dock of `id` in place of the shown one. The panels of the other space stay as
    /// they are. A product space makes its product the active one.
    pub fn show_space(&mut self, id: SpaceId, window: &mut Window, cx: &mut Context<Self>) {
        if let SpaceId::Product(p) = &id
            && self.core.read(cx).active.as_ref() != Some(p)
        {
            self.core.update(cx, |s, cx| {
                s.active = Some(p.clone());
                cx.notify();
            });
        }
        if self.shown == id {
            return;
        }
        let (dock, tabs) = self.space(&id, window, cx);
        let old = std::mem::replace(&mut self.dock, dock);
        self.tabs = tabs;
        self.shown = id;
        Self::tell_shown(&old, false, window, cx);
        Self::tell_shown(&self.dock.clone(), true, window, cx);
        self.spaces_changed(cx);
        cx.notify();
    }

    /// Tells the tab on screen of each bar of `dock` that it shows or hides.
    fn tell_shown(dock: &Entity<DockArea>, shown: bool, window: &mut Window, cx: &mut App) {
        let views: Vec<_> = {
            let d = dock.read(cx);
            tabs::bars(d)
                .into_iter()
                .filter_map(|b| d.panel(*b.panels.get(b.active)?).cloned())
                .collect()
        };
        for v in views {
            v.set_active(shown, window, cx);
        }
    }

    /// The focus goes to the tab on screen of the first bar of the shown space.
    pub(super) fn focus_shown(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let view = {
            let d = self.dock.read(cx);
            tabs::bars(d)
                .first()
                .and_then(|b| d.panel(*b.panels.get(b.active)?).cloned())
        };
        match view {
            Some(v) => window.focus(&v.focus_handle(cx), cx),
            None => window.focus(&self.focus, cx),
        }
    }

    /// The space whose dock holds `panel`.
    pub(super) fn space_of_panel(&self, panel: PanelId, cx: &App) -> Option<SpaceId> {
        self.spaces
            .iter()
            .find(|(_, s)| s.dock.read(cx).panel(panel).is_some())
            .map(|(id, _)| id.clone())
    }

    pub(super) fn dock_of(&self, panel: PanelId, cx: &App) -> Option<Entity<DockArea>> {
        let id = self.space_of_panel(panel, cx)?;
        Some(self.spaces[&id].dock.clone())
    }

    pub(super) fn tabs_of(&self, panel: PanelId, cx: &App) -> Option<Entity<TabGroups>> {
        let id = self.space_of_panel(panel, cx)?;
        Some(self.spaces[&id].tabs.clone())
    }

    /// A dock of some space holds the panel `id`.
    pub(super) fn in_any(&self, id: EntityId, cx: &App) -> bool {
        self.space_of_panel(PanelId::from(id), cx).is_some()
    }

    /// Shows the space of `panel`, selects its tab and gives it the focus.
    pub fn select_tab(&mut self, panel: PanelId, window: &mut Window, cx: &mut Context<Self>) {
        let Some(space) = self.space_of_panel(panel, cx) else {
            return;
        };
        self.show_space(space, window, cx);
        self.dock
            .update(cx, |d, cx| d.select_panel(panel, window, cx));
        if let Some(v) = self.dock.read(cx).panel(panel).cloned() {
            window.focus(&v.focus_handle(cx), cx);
        }
    }

    /// The open tabs of each space, in the order of its bars.
    pub(super) fn tab_rows(&self, cx: &App) -> BTreeMap<SpaceId, Vec<TabRow>> {
        let open = self.open.borrow();
        let terminal = |p: PanelId| {
            open.terminals
                .iter()
                .filter_map(WeakEntity::upgrade)
                .find(|t| PanelId::from(t.entity_id()) == p)
                .map(|t| ItemKey::Pane(t.read(cx).id.clone()))
        };
        let screen = |p: PanelId| {
            open.screens
                .iter()
                .filter_map(WeakEntity::upgrade)
                .find(|s| PanelId::from(s.entity_id()) == p)
                .map(|s| {
                    let s = s.read(cx);
                    ItemKey::Screen(s.kind, s.arg.clone())
                })
        };
        self.spaces
            .iter()
            .map(|(id, s)| {
                let pinned = s.tabs.read(cx).pinned();
                let rows = tabs::bars(s.dock.read(cx))
                    .into_iter()
                    .flat_map(|b| b.panels)
                    .filter_map(|p| {
                        Some(TabRow {
                            key: terminal(p).or_else(|| screen(p))?,
                            panel: p,
                            pinned: pinned.contains(&p),
                        })
                    })
                    .collect();
                (id.clone(), rows)
            })
            .collect()
    }

    /// A tab or a space changed: the sidebar lists the tabs again, and the layout saves.
    pub(super) fn spaces_changed(&mut self, cx: &mut Context<Self>) {
        let rows = self.tab_rows(cx);
        self.sidebar.update(cx, |s, cx| s.set_tabs(rows, cx));
        self.schedule_save(cx);
    }

    /// Each space with a tab, as `layout.json` keeps it.
    pub(super) fn save_spaces(&self, cx: &App) -> BTreeMap<String, SpaceSave> {
        self.spaces
            .iter()
            .filter(|(_, s)| !s.dock.read(cx).is_empty(DockPlacement::Center, cx))
            .map(|(id, s)| {
                let dock = s.dock.read(cx);
                let pinned: BTreeSet<String> = s
                    .tabs
                    .read(cx)
                    .pinned()
                    .iter()
                    .filter_map(|p| layout::panel_key(&dock.panel(*p)?.dump(cx)))
                    .collect();
                let mut state = dock.dump(cx);
                layout::mark_pinned(&mut state, &pinned);
                let save = SpaceSave {
                    dock: state,
                    tab_groups: s.tabs.read(cx).save(dock, cx),
                };
                (id.key(), save)
            })
            .collect()
    }

    /// The space of a panel of a file of version 1 or 2: a screen goes to its product, a terminal
    /// to the product that the core of its host knows for it, and the rest to Home.
    pub(super) fn space_of_saved(&self, state: &PanelState, cx: &App) -> SpaceId {
        if let Some(spec) = screen_of(state) {
            return SpaceId::of_screen(&spec);
        }
        let host = layout::host_of(state);
        let product = layout::terminal_of(state).and_then(|id| {
            let core = self.cores.of(host)?.read(cx);
            let product = core.terminals.get(id)?.product.clone()?;
            Some(ProductRef {
                host: host.map(String::from),
                id: product,
            })
        });
        SpaceId::of(product)
    }

    /// Loads the saved spaces. Returns the terminals and the hosts of the panels that `known` did
    /// not list.
    pub(super) fn restore_spaces(
        &mut self,
        saved: BTreeMap<String, SpaceSave>,
        known: &layout::Known,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Vec<String> {
        let mut dropped = Vec::new();
        for (key, mut s) in saved {
            dropped.extend(layout::prune(&mut s.dock, known));
            let pinned = layout::take_pinned(&mut s.dock);
            let (dock, tabs) = self.space(&SpaceId::from_key(&key), window, cx);
            dock.update(cx, |d, cx| {
                if let Err(e) = d.load(s.dock, window, cx) {
                    eprintln!("agentsi-app: the layout of space {key} does not load: {e}");
                }
            });
            self.restore_tabs(&dock, &tabs, &s.tab_groups, &pinned, window, cx);
        }
        dropped
    }

    /// Reads the groups and the pins of the layout file onto the panels that a dock loaded.
    fn restore_tabs(
        &mut self,
        dock: &Entity<DockArea>,
        tabs: &Entity<TabGroups>,
        groups: &[TabGroupSave],
        pinned: &BTreeSet<String>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let keys: Vec<(String, PanelId)> = {
            let d = dock.read(cx);
            tabs::bars(d)
                .into_iter()
                .flat_map(|b| b.panels)
                .filter_map(|p| Some((layout::panel_key(&d.panel(p)?.dump(cx))?, p)))
                .collect()
        };
        let pins: BTreeSet<PanelId> = keys
            .iter()
            .filter(|(k, _)| pinned.contains(k))
            .map(|(_, p)| *p)
            .collect();
        tabs.update(cx, |t, cx| {
            t.restore(
                groups,
                |k| keys.iter().find(|(key, _)| key == k).map(|(_, p)| *p),
                cx,
            );
            t.restore_pinned(pins, cx);
        });
        tabs::reconcile(dock, tabs, window, cx);
    }
}
