//! What the shell does with tabs and tab groups: close a set of tabs behind one Confirm, and
//! keep the groups.
//!
//! Each action is a plain method that takes the tab or the group it acts on. A palette entry
//! resolves the active tab and calls the method. The context menus of the tab bar call the same
//! methods with the tab under the pointer.

use gpui_kit::component::dock::PanelId;
use gpui_kit::*;

use super::{Ask, Shell};
use crate::layout::{self, TabGroupSave};
use crate::prompt::{Choice, Prompt};
use crate::tabs::{self, GroupId};

/// The answer of the Confirm that closes the tabs.
const CLOSE: &str = "close";
const KEEP: &str = "keep";

impl Shell {
    /// The tab with the keyboard focus. Else the pane that the owner used last. Else the tab on
    /// screen in the first bar.
    pub fn active_tab(&self, window: &Window, cx: &App) -> Option<PanelId> {
        let dock = self.dock.read(cx);
        let bars = tabs::bars(dock);
        let focused = bars
            .iter()
            .flat_map(|b| b.panels.iter())
            .find(|p| {
                dock.panel(**p)
                    .is_some_and(|v| v.focus_handle(cx).contains_focused(window, cx))
            })
            .copied();
        focused
            .or_else(|| {
                self.focused_terminal(window, cx)
                    .map(|t| PanelId::from(t.entity_id()))
            })
            .or_else(|| bars.first().and_then(|b| b.panels.get(b.active)).copied())
    }

    /// Runs `f` on the active tab, or says that there is none.
    pub(super) fn on_active_tab(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
        f: impl FnOnce(&mut Self, PanelId, &mut Window, &mut Context<Self>),
    ) {
        match self.active_tab(window, cx) {
            Some(p) => f(self, p, window, cx),
            None => self.notice("No tab is open. Open a terminal first.", cx),
        }
    }

    /// Runs `f` on the tab group of the active tab, or says that the tab has none.
    pub(super) fn on_active_group(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
        f: impl FnOnce(&mut Self, GroupId, &mut Window, &mut Context<Self>),
    ) {
        match self.active_tab(window, cx) {
            Some(p) => match self.tabs.read(cx).group_of(p) {
                Some(g) => f(self, g, window, cx),
                None => self.notice(
                    "The active tab is in no tab group. Use \"New tab group\" first.",
                    cx,
                ),
            },
            None => self.notice("No tab is open. Open a terminal first.", cx),
        }
    }

    fn entity_of(&self, id: PanelId, cx: &App) -> Option<TabEntity> {
        let open = self.open.borrow();
        let same = |e: EntityId| PanelId::from(e) == id;
        open.terminals
            .iter()
            .filter_map(WeakEntity::upgrade)
            .find(|t| same(t.entity_id()))
            .map(TabEntity::Terminal)
            .or_else(|| {
                open.screens
                    .iter()
                    .filter_map(WeakEntity::upgrade)
                    .find(|s| same(s.entity_id()))
                    .map(TabEntity::Screen)
            })
            .filter(|_| self.dock.read(cx).panel(id).is_some())
    }

    /// A program runs in the tab: closing it stops the program.
    fn tab_runs(&self, id: PanelId, cx: &App) -> bool {
        matches!(self.entity_of(id, cx), Some(TabEntity::Terminal(t)) if t.read(cx).running(cx))
    }

    /// Closes the tabs. When a program runs in some of them, one Confirm comes first. A tab that
    /// cannot close stays.
    pub fn close_tabs(&mut self, tabs: Vec<PanelId>, window: &mut Window, cx: &mut Context<Self>) {
        let tabs: Vec<PanelId> = tabs
            .into_iter()
            .filter(|p| self.dock.read(cx).panel(*p).is_some_and(|v| v.closable(cx)))
            .collect();
        if tabs.is_empty() {
            return;
        }
        let running = tabs.iter().filter(|p| self.tab_runs(**p, cx)).count();
        if running == 0 {
            self.remove_tabs(&tabs, window, cx);
        } else {
            self.ask(Ask::CloseTabs(tabs, running), window, cx);
        }
    }

    /// Closes the tabs now. A terminal tab kills its terminal.
    fn remove_tabs(&mut self, tabs: &[PanelId], window: &mut Window, cx: &mut Context<Self>) {
        for id in tabs {
            let Some(tab) = self.entity_of(*id, cx) else {
                continue;
            };
            self.dock.update(cx, |d, cx| match tab {
                TabEntity::Terminal(t) => d.remove_panel(t, window, cx),
                TabEntity::Screen(s) => d.remove_panel(s, window, cx),
            });
        }
    }

    fn bar_of(&self, panel: PanelId, cx: &App) -> Option<tabs::Bar> {
        tabs::bar_of(self.dock.read(cx), panel)
    }

    /// Closes each tab of the bar of `panel`, except `panel`.
    pub fn close_others(&mut self, panel: PanelId, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(bar) = self.bar_of(panel, cx) {
            let others = bar.panels.into_iter().filter(|p| *p != panel).collect();
            self.close_tabs(others, window, cx);
        }
    }

    /// Closes each tab of the bar of `panel` that sits after `panel`.
    pub fn close_to_the_right(
        &mut self,
        panel: PanelId,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if let Some(bar) = self.bar_of(panel, cx) {
            let after = bar
                .panels
                .into_iter()
                .skip_while(|p| *p != panel)
                .skip(1)
                .collect();
            self.close_tabs(after, window, cx);
        }
    }

    /// Closes each tab of the group.
    pub fn close_group(&mut self, id: GroupId, window: &mut Window, cx: &mut Context<Self>) {
        let members = self
            .tabs
            .read(cx)
            .get(id)
            .map(|g| g.members.clone())
            .unwrap_or_default();
        self.close_tabs(members, window, cx);
    }

    /// Dissolves the group and keeps its tabs.
    pub fn ungroup(&mut self, id: GroupId, _: &mut Window, cx: &mut Context<Self>) {
        self.tabs.update(cx, |t, cx| t.ungroup(id, cx));
    }

    /// Collapses the group to its chip, or opens it.
    pub fn toggle_group(&mut self, id: GroupId, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(g) = self.tabs.read(cx).get(id) {
            let collapsed = !g.collapsed;
            tabs::set_collapsed(&self.dock, &self.tabs, id, collapsed, window, cx);
        }
    }

    pub fn add_to_group(
        &mut self,
        panel: PanelId,
        id: GroupId,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        tabs::add_to_group(&self.dock, &self.tabs, panel, id, window, cx);
    }

    /// Takes the tab out of its group.
    pub fn leave_group(&mut self, panel: PanelId, window: &mut Window, cx: &mut Context<Self>) {
        tabs::remove_from_group(&self.dock, &self.tabs, panel, window, cx);
    }

    /// Reads the groups of the layout file onto the panels that the dock loaded.
    pub(super) fn restore_tab_groups(
        &mut self,
        saved: &[TabGroupSave],
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let dock = self.dock.read(cx);
        let keys: Vec<(String, PanelId)> = tabs::bars(dock)
            .into_iter()
            .flat_map(|b| b.panels)
            .filter_map(|p| Some((layout::panel_key(&dock.panel(p)?.dump(cx))?, p)))
            .collect();
        self.tabs.update(cx, |t, cx| {
            t.restore(
                saved,
                |k| keys.iter().find(|(key, _)| key == k).map(|(_, p)| *p),
                cx,
            )
        });
        tabs::reconcile(&self.dock, &self.tabs, window, cx);
    }

    /// The prompt for an `Ask` about tabs, or `None` when it needs none.
    pub(super) fn tab_prompt(
        &mut self,
        what: &Ask,
        colors: &[Choice],
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Option<Entity<Prompt>> {
        match what {
            Ask::NewTabGroup(_) => Some(cx.new(|cx| {
                Prompt::text(
                    "Name for the new tab group",
                    "A name of 40 characters or less.",
                    "",
                    Vec::new(),
                    window,
                    cx,
                )
            })),
            Ask::AddToTabGroup(panel) => {
                let bar = self.bar_of(*panel, cx)?;
                let choices: Vec<Choice> = self
                    .tabs
                    .read(cx)
                    .groups()
                    .iter()
                    .filter(|g| g.members.iter().any(|m| bar.panels.contains(m)))
                    .map(|g| {
                        let c = Choice::new(g.name.clone(), g.id.to_string());
                        match g.hue {
                            Some(h) => c.hue(h),
                            None => c,
                        }
                    })
                    .collect();
                if choices.is_empty() {
                    self.notice(
                        "This bar has no tab group. Use \"New tab group\" first.",
                        cx,
                    );
                    return None;
                }
                Some(
                    cx.new(|cx| {
                        Prompt::pick("Add this tab to a tab group", choices, 0, window, cx)
                    }),
                )
            }
            Ask::RenameTabGroup(id) => {
                let now = self
                    .tabs
                    .read(cx)
                    .get(*id)
                    .map(|g| g.name.clone())
                    .unwrap_or_default();
                Some(cx.new(|cx| {
                    Prompt::text(
                        "Name for this tab group",
                        "A name of 40 characters or less.",
                        &now,
                        Vec::new(),
                        window,
                        cx,
                    )
                }))
            }
            Ask::ColorTabGroup(id) => {
                let at = self
                    .tabs
                    .read(cx)
                    .get(*id)
                    .and_then(|g| g.hue)
                    .and_then(|h| colors.iter().position(|c| c.value == h.key()))
                    .unwrap_or(0);
                let colors = colors.to_vec();
                Some(cx.new(|cx| Prompt::pick("Color for this tab group", colors, at, window, cx)))
            }
            Ask::CloseTabs(tabs, running) => {
                let title = format!(
                    "Close {} tab{}? {} run{} a program, which stops.",
                    tabs.len(),
                    if tabs.len() == 1 { "" } else { "s" },
                    running,
                    if *running == 1 { "s" } else { "" },
                );
                let choices = vec![
                    Choice::new("Close the tabs", CLOSE),
                    Choice::new("Keep them", KEEP),
                ];
                // Enter keeps the tabs: the safe answer is the one under the cursor.
                Some(cx.new(|cx| Prompt::pick(title, choices, 1, window, cx)))
            }
            _ => None,
        }
    }

    /// Runs the answer of a question about tabs. Returns false for a question about the look.
    pub(super) fn tab_answer(
        &mut self,
        what: &Ask,
        answer: &str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> bool {
        match what {
            Ask::NewTabGroup(panel) => {
                self.tabs.update(cx, |t, cx| t.create(answer, *panel, cx));
            }
            Ask::AddToTabGroup(panel) => {
                if let Ok(id) = answer.parse::<GroupId>() {
                    self.add_to_group(*panel, id, window, cx);
                }
            }
            Ask::RenameTabGroup(id) => self.tabs.update(cx, |t, cx| t.rename(*id, answer, cx)),
            Ask::ColorTabGroup(id) => {
                let hue = agentsi_ui::tokens::label::Hue::from_key(answer);
                self.tabs.update(cx, |t, cx| t.set_hue(*id, hue, cx));
            }
            Ask::CloseTabs(tabs, _) => {
                if answer == CLOSE {
                    self.remove_tabs(tabs, window, cx);
                }
            }
            _ => return false,
        }
        true
    }
}

/// The panel entity behind a tab.
enum TabEntity {
    Terminal(Entity<crate::pane::TerminalPanel>),
    Screen(Entity<crate::screens::ScreenPanel>),
}
