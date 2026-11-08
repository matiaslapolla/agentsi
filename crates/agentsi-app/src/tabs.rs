//! Tab groups: a named, colored set of tabs in one tab bar. A group draws as a chip before its
//! tabs (`tab_bar.rs`) and collapses to the chip.
//!
//! The dock of `gpui-component` keeps the tabs and their order. This module keeps the groups:
//! which tab belongs to which group, and the name, the color and the collapsed state. The tabs
//! of a group always sit together in the dock, so the bar needs no sort. [`reconcile`] repairs
//! that after a change that the groups did not make.
//!
//! The groups are layout, not look: `layout.json` keeps them (`layout::TabGroupSave`).

use std::collections::BTreeSet;

use agentsi_ui::tokens::label::Hue;
use gpui_kit::component::dock::{DockArea, DockPlacement, InsertTarget, NodeId, PaneRef, PanelId};
use gpui_kit::{App, Context, Entity, EventEmitter, Window};

use crate::layout::{self, TabGroupSave};
use crate::look::clean_label;

pub type GroupId = u32;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Group {
    pub id: GroupId,
    pub name: String,
    pub hue: Option<Hue>,
    pub collapsed: bool,
    pub members: Vec<PanelId>,
}

/// A group or one of its tabs changed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TabsChanged;

#[derive(Default)]
pub struct TabGroups {
    groups: Vec<Group>,
    next: GroupId,
}

impl EventEmitter<TabsChanged> for TabGroups {}

impl TabGroups {
    pub fn groups(&self) -> &[Group] {
        &self.groups
    }

    pub fn get(&self, id: GroupId) -> Option<&Group> {
        self.groups.iter().find(|g| g.id == id)
    }

    pub fn group_of(&self, panel: PanelId) -> Option<GroupId> {
        self.groups
            .iter()
            .find(|g| g.members.contains(&panel))
            .map(|g| g.id)
    }

    fn changed(&self, cx: &mut Context<Self>) {
        cx.emit(TabsChanged);
        cx.notify();
    }

    fn take(&mut self, panel: PanelId) {
        for g in &mut self.groups {
            g.members.retain(|m| *m != panel);
        }
        self.groups.retain(|g| !g.members.is_empty());
    }

    /// The first color that no group uses, else the first color.
    fn fresh_hue(&self) -> Hue {
        Hue::ALL
            .into_iter()
            .find(|h| self.groups.iter().all(|g| g.hue != Some(*h)))
            .unwrap_or(Hue::ALL[0])
    }

    /// A group with `panel` in it. The name is cleaned as a label is. An empty name makes no group.
    pub fn create(
        &mut self,
        name: &str,
        panel: PanelId,
        cx: &mut Context<Self>,
    ) -> Option<GroupId> {
        let name = clean_label(name)?;
        self.take(panel);
        let id = self.next;
        self.next += 1;
        let hue = Some(self.fresh_hue());
        self.groups.push(Group {
            id,
            name,
            hue,
            collapsed: false,
            members: vec![panel],
        });
        self.changed(cx);
        Some(id)
    }

    /// Puts `panel` in the group `id`, and out of the group it was in.
    pub fn join(&mut self, panel: PanelId, id: GroupId, cx: &mut Context<Self>) {
        if self.get(id).is_none() || self.group_of(panel) == Some(id) {
            return;
        }
        self.take(panel);
        if let Some(g) = self.groups.iter_mut().find(|g| g.id == id) {
            g.members.push(panel);
        }
        self.changed(cx);
    }

    pub fn leave(&mut self, panel: PanelId, cx: &mut Context<Self>) {
        if self.group_of(panel).is_some() {
            self.take(panel);
            self.changed(cx);
        }
    }

    /// An empty name keeps the old one.
    pub fn rename(&mut self, id: GroupId, name: &str, cx: &mut Context<Self>) {
        let Some(name) = clean_label(name) else {
            return;
        };
        self.edit(id, cx, |g| g.name = name);
    }

    pub fn set_hue(&mut self, id: GroupId, hue: Option<Hue>, cx: &mut Context<Self>) {
        self.edit(id, cx, |g| g.hue = hue);
    }

    pub fn set_collapsed(&mut self, id: GroupId, collapsed: bool, cx: &mut Context<Self>) {
        self.edit(id, cx, |g| g.collapsed = collapsed);
    }

    fn edit(&mut self, id: GroupId, cx: &mut Context<Self>, f: impl FnOnce(&mut Group)) {
        let Some(g) = self.groups.iter_mut().find(|g| g.id == id) else {
            return;
        };
        let before = g.clone();
        f(g);
        if *g != before {
            self.changed(cx);
        }
    }

    /// Removes the group and keeps its tabs. Returns the tabs.
    pub fn ungroup(&mut self, id: GroupId, cx: &mut Context<Self>) -> Vec<PanelId> {
        let Some(at) = self.groups.iter().position(|g| g.id == id) else {
            return Vec::new();
        };
        let group = self.groups.remove(at);
        self.changed(cx);
        group.members
    }

    /// Drops the tabs that the dock no longer holds, and the groups that stay empty.
    fn keep_only(&mut self, alive: &BTreeSet<PanelId>, cx: &mut Context<Self>) {
        let before = self.groups.clone();
        for g in &mut self.groups {
            g.members.retain(|m| alive.contains(m));
        }
        self.groups.retain(|g| !g.members.is_empty());
        if self.groups != before {
            self.changed(cx);
        }
    }

    /// Moves the tabs of a group that sit in other bars out of it. The group stays in the bar
    /// with most of its tabs: on a tie, the bar of the tab that joined first.
    fn keep_one_bar(&mut self, bars: &[Bar], cx: &mut Context<Self>) {
        let before = self.groups.clone();
        for g in &mut self.groups {
            let node_of = |p: &PanelId| bars.iter().find(|b| b.panels.contains(p)).map(|b| b.node);
            let mut best: Option<(NodeId, usize)> = None;
            for m in &g.members {
                let Some(n) = node_of(m) else { continue };
                let count = g.members.iter().filter(|o| node_of(o) == Some(n)).count();
                if best.is_none_or(|(_, c)| count > c) {
                    best = Some((n, count));
                }
            }
            if let Some((home, _)) = best {
                g.members.retain(|m| node_of(m) == Some(home));
            }
        }
        self.groups.retain(|g| !g.members.is_empty());
        if self.groups != before {
            self.changed(cx);
        }
    }

    /// The groups as the layout file keeps them: the tabs in the order of the dock.
    pub fn save(&self, dock: &DockArea, cx: &App) -> Vec<TabGroupSave> {
        let order: Vec<PanelId> = bars(dock).into_iter().flat_map(|b| b.panels).collect();
        let key = |p: PanelId| layout::panel_key(&dock.panel(p)?.dump(cx));
        self.groups
            .iter()
            .filter_map(|g| {
                let mut members: Vec<PanelId> = g.members.clone();
                members.sort_by_key(|m| order.iter().position(|o| o == m));
                let tabs: Vec<String> = members.into_iter().filter_map(key).collect();
                (!tabs.is_empty()).then(|| TabGroupSave {
                    name: g.name.clone(),
                    color: g.hue.map(|h| h.key().to_string()),
                    collapsed: g.collapsed,
                    tabs,
                })
            })
            .collect()
    }

    /// Replaces the groups with the saved ones. A tab that the dock does not hold is dropped, and
    /// so is a group that stays empty.
    pub fn restore(
        &mut self,
        saved: &[TabGroupSave],
        find: impl Fn(&str) -> Option<PanelId>,
        cx: &mut Context<Self>,
    ) {
        self.groups.clear();
        self.next = 0;
        for s in saved {
            let Some(name) = clean_label(&s.name) else {
                continue;
            };
            let mut members: Vec<PanelId> = Vec::new();
            for key in &s.tabs {
                if let Some(p) = find(key)
                    && !members.contains(&p)
                    && self.group_of(p).is_none()
                {
                    members.push(p);
                }
            }
            if members.is_empty() {
                continue;
            }
            self.groups.push(Group {
                id: self.next,
                name,
                hue: s.color.as_deref().and_then(Hue::from_key),
                collapsed: s.collapsed,
                members,
            });
            self.next += 1;
        }
        self.changed(cx);
    }
}

/// One tab bar of the dock: its node and its tabs in order.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Bar {
    pub node: NodeId,
    pub panels: Vec<PanelId>,
    /// The index of the tab on screen.
    pub active: usize,
}

/// The tab bars of the center of the dock.
pub fn bars(dock: &DockArea) -> Vec<Bar> {
    let mut out = Vec::new();
    if let Some(tree) = dock.layout(DockPlacement::Center) {
        tree.root().walk(&mut |n| {
            if let PaneRef::Tabs { panels, active_ix } = n.kind() {
                out.push(Bar {
                    node: n.id(),
                    panels: panels.to_vec(),
                    active: active_ix,
                });
            }
        });
    }
    out
}

pub fn bar_of(dock: &DockArea, panel: PanelId) -> Option<Bar> {
    bars(dock).into_iter().find(|b| b.panels.contains(&panel))
}

/// The slot that puts `moving` right before `target`, as the dock counts it: after `moving`
/// left its place.
pub fn slot_before(panels: &[PanelId], target: PanelId, moving: PanelId) -> usize {
    panels
        .iter()
        .filter(|p| **p != moving)
        .position(|p| *p == target)
        .unwrap_or(panels.len())
}

/// The slot right after the last of `members`, as the dock counts it: after `moving` left its
/// place. With none of them in the bar, the end.
pub fn slot_after(panels: &[PanelId], members: &[PanelId], moving: PanelId) -> usize {
    let rest: Vec<&PanelId> = panels.iter().filter(|p| **p != moving).collect();
    rest.iter()
        .rposition(|p| members.contains(p) && **p != moving)
        .map_or(rest.len(), |at| at + 1)
}

/// The order of a bar once each group sits together: a group stands where its first tab was.
pub fn grouped_order(panels: &[PanelId], groups: &[Group]) -> Vec<PanelId> {
    let mut out: Vec<PanelId> = Vec::new();
    for p in panels {
        if out.contains(p) {
            continue;
        }
        match groups.iter().find(|g| g.members.contains(p)) {
            Some(g) => out.extend(panels.iter().filter(|o| g.members.contains(o))),
            None => out.push(*p),
        }
    }
    out
}

fn move_to(
    dock: &Entity<DockArea>,
    panel: PanelId,
    node: NodeId,
    ix: usize,
    activate: bool,
    window: &mut Window,
    cx: &mut App,
) {
    dock.update(cx, |d, cx| {
        d.move_panel(
            panel,
            InsertTarget::Tabs {
                node,
                ix: Some(ix),
                activate,
            },
            window,
            cx,
        )
    });
}

/// Puts `panel` in the group `id`: right after the last tab of the group, in the bar of the
/// group.
pub fn add_to_group(
    dock: &Entity<DockArea>,
    tabs: &Entity<TabGroups>,
    panel: PanelId,
    id: GroupId,
    window: &mut Window,
    cx: &mut App,
) {
    let Some(members) = tabs.read(cx).get(id).map(|g| g.members.clone()) else {
        return;
    };
    let home = members
        .iter()
        .find_map(|m| bar_of(dock.read(cx), *m))
        .map(|b| (b.node, b.panels));
    if let Some((node, panels)) = home {
        let at = slot_after(&panels, &members, panel);
        if panels.iter().position(|p| *p == panel) != Some(at) {
            move_to(dock, panel, node, at, false, window, cx);
        }
    }
    tabs.update(cx, |t, cx| t.join(panel, id, cx));
}

/// Takes `panel` out of its group and puts it right after the group.
pub fn remove_from_group(
    dock: &Entity<DockArea>,
    tabs: &Entity<TabGroups>,
    panel: PanelId,
    window: &mut Window,
    cx: &mut App,
) {
    let Some(id) = tabs.read(cx).group_of(panel) else {
        return;
    };
    let rest: Vec<PanelId> = tabs
        .read(cx)
        .get(id)
        .map(|g| g.members.iter().copied().filter(|m| *m != panel).collect())
        .unwrap_or_default();
    if let Some(bar) = bar_of(dock.read(cx), panel) {
        let at = slot_after(&bar.panels, &rest, panel);
        if bar.panels.iter().position(|p| *p == panel) != Some(at) {
            move_to(dock, panel, bar.node, at, false, window, cx);
        }
    }
    tabs.update(cx, |t, cx| t.leave(panel, cx));
}

/// Collapses or opens a group. A collapsed group hides its tabs, so the tab on screen moves to
/// the nearest tab outside the group, when there is one.
pub fn set_collapsed(
    dock: &Entity<DockArea>,
    tabs: &Entity<TabGroups>,
    id: GroupId,
    collapsed: bool,
    window: &mut Window,
    cx: &mut App,
) {
    if collapsed && let Some(g) = tabs.read(cx).get(id) {
        let members = g.members.clone();
        let shown = dock
            .read(cx)
            .layout(DockPlacement::Center)
            .and_then(|tree| {
                let node = members.first().and_then(|m| tree.find_panel_node(*m))?;
                match tree.find_node(node)?.kind() {
                    PaneRef::Tabs { panels, active_ix } => {
                        Some((panels.to_vec(), panels.get(active_ix).copied()))
                    }
                    PaneRef::Split { .. } => None,
                }
            });
        if let Some((panels, Some(active))) = shown
            && members.contains(&active)
        {
            let after = panels
                .iter()
                .skip_while(|p| !members.contains(p))
                .find(|p| !members.contains(p));
            let before = panels.iter().rev().find(|p| !members.contains(p));
            if let Some(next) = after.or(before).copied() {
                dock.update(cx, |d, cx| d.select_panel(next, window, cx));
            }
        }
    }
    tabs.update(cx, |t, cx| t.set_collapsed(id, collapsed, cx));
}

/// Repairs the groups after a change of the dock: drops the tabs that left, takes the tabs of a
/// group out of the bars that are not its own, and moves the tabs of a group together.
pub fn reconcile(
    dock: &Entity<DockArea>,
    tabs: &Entity<TabGroups>,
    window: &mut Window,
    cx: &mut App,
) {
    let all = bars(dock.read(cx));
    let alive: BTreeSet<PanelId> = all.iter().flat_map(|b| b.panels.iter().copied()).collect();
    tabs.update(cx, |t, cx| {
        t.keep_only(&alive, cx);
        t.keep_one_bar(&all, cx);
    });
    let groups = tabs.read(cx).groups().to_vec();
    for bar in all {
        let want = grouped_order(&bar.panels, &groups);
        let mut now = bar.panels.clone();
        for (i, p) in want.iter().enumerate() {
            if now[i] != *p {
                let from = now.iter().position(|o| o == p).unwrap_or(i);
                now.remove(from);
                now.insert(i, *p);
                move_to(dock, *p, bar.node, i, false, window, cx);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use gpui_kit::AppContext as _;

    fn p(n: u64) -> PanelId {
        PanelId::from_u64(n)
    }

    fn g(id: GroupId, members: &[u64]) -> Group {
        Group {
            id,
            name: format!("g{id}"),
            hue: None,
            collapsed: false,
            members: members.iter().map(|n| p(*n)).collect(),
        }
    }

    #[test]
    fn a_slot_counts_after_the_moving_tab_left() {
        let bar = [p(1), p(2), p(3), p(4)];
        // Before tab 3, for tab 1: [2, 1, 3, 4] puts 1 at slot 1.
        assert_eq!(slot_before(&bar, p(3), p(1)), 1);
        // Before tab 1, for tab 4: slot 0.
        assert_eq!(slot_before(&bar, p(1), p(4)), 0);
        // After the group {2, 3}, for tab 1: [2, 3, 1, 4] puts 1 at slot 2.
        assert_eq!(slot_after(&bar, &[p(2), p(3)], p(1)), 2);
        // After the group {1, 2}, for tab 4: slot 2.
        assert_eq!(slot_after(&bar, &[p(1), p(2)], p(4)), 2);
        // With no tab of the group in the bar, the end.
        assert_eq!(slot_after(&bar, &[p(9)], p(1)), 3);
        // The moving tab does not count as a tab of the group.
        assert_eq!(slot_after(&bar, &[p(1), p(2)], p(2)), 1);
    }

    #[test]
    fn the_tabs_of_a_group_come_together_where_its_first_tab_was() {
        let bar = [p(1), p(2), p(3), p(4), p(5)];
        let groups = [g(0, &[2, 4]), g(1, &[5])];
        assert_eq!(grouped_order(&bar, &groups), [p(1), p(2), p(4), p(3), p(5)]);
        // A bar that is in order stays.
        assert_eq!(
            grouped_order(&[p(1), p(2), p(4)], &[g(0, &[2, 4])]),
            [p(1), p(2), p(4)]
        );
        assert_eq!(grouped_order(&bar, &[]), bar);
    }

    #[gpui::test]
    fn the_model_creates_joins_leaves_and_drops_empty_groups(cx: &mut gpui::TestAppContext) {
        let tabs = cx.new(|_| TabGroups::default());
        let events = std::rc::Rc::new(std::cell::Cell::new(0));
        let seen = events.clone();
        let _s = cx.update(|cx| {
            cx.subscribe(&tabs, move |_, _: &TabsChanged, _| seen.set(seen.get() + 1))
        });
        let a = tabs
            .update(cx, |t, cx| t.create("  Build  ", p(1), cx))
            .unwrap();
        assert_eq!(events.get(), 1);
        assert_eq!(
            tabs.read_with(cx, |t, _| t.get(a).unwrap().name.clone()),
            "Build"
        );
        // An empty name makes no group.
        assert_eq!(tabs.update(cx, |t, cx| t.create("   ", p(2), cx)), None);
        let b = tabs.update(cx, |t, cx| t.create("Docs", p(2), cx)).unwrap();
        // The two groups get two colors.
        let hues = tabs.read_with(cx, |t, _| {
            t.groups().iter().map(|g| g.hue).collect::<Vec<_>>()
        });
        assert_ne!(hues[0], hues[1]);
        // A tab is in one group: joining moves it.
        tabs.update(cx, |t, cx| t.join(p(2), a, cx));
        assert_eq!(tabs.read_with(cx, |t, _| t.group_of(p(2))), Some(a));
        assert!(
            tabs.read_with(cx, |t, _| t.get(b).is_none()),
            "the group that lost its last tab is gone"
        );
        tabs.update(cx, |t, cx| t.leave(p(2), cx));
        assert_eq!(tabs.read_with(cx, |t, _| t.group_of(p(2))), None);
        // Changing nothing emits nothing.
        let n = events.get();
        tabs.update(cx, |t, cx| t.set_collapsed(a, false, cx));
        tabs.update(cx, |t, cx| t.rename(a, "Build", cx));
        assert_eq!(events.get(), n);
        tabs.update(cx, |t, cx| t.rename(a, "", cx));
        assert_eq!(
            tabs.read_with(cx, |t, _| t.get(a).unwrap().name.clone()),
            "Build"
        );
        let gone = tabs.update(cx, |t, cx| t.ungroup(a, cx));
        assert_eq!(gone, [p(1)]);
        assert!(tabs.read_with(cx, |t, _| t.groups().is_empty()));
    }

    #[gpui::test]
    fn restore_keeps_the_known_tabs_and_drops_a_group_that_stays_empty(
        cx: &mut gpui::TestAppContext,
    ) {
        let tabs = cx.new(|_| TabGroups::default());
        let saved = [
            TabGroupSave {
                name: "Build".into(),
                color: Some("teal".into()),
                collapsed: true,
                tabs: vec!["a".into(), "gone".into(), "b".into()],
            },
            TabGroupSave {
                name: "Old".into(),
                color: None,
                collapsed: false,
                tabs: vec!["gone".into()],
            },
            TabGroupSave {
                name: "Twice".into(),
                color: Some("not-a-color".into()),
                collapsed: false,
                // `a` is in Build already: a tab belongs to one group.
                tabs: vec!["a".into(), "c".into()],
            },
        ];
        let find = |k: &str| match k {
            "a" => Some(p(1)),
            "b" => Some(p(2)),
            "c" => Some(p(3)),
            _ => None,
        };
        tabs.update(cx, |t, cx| t.restore(&saved, find, cx));
        let groups = tabs.read_with(cx, |t, _| t.groups().to_vec());
        assert_eq!(groups.len(), 2);
        assert_eq!(groups[0].members, [p(1), p(2)]);
        assert_eq!(groups[0].hue, Some(Hue::Teal));
        assert!(groups[0].collapsed);
        assert_eq!(groups[1].members, [p(3)]);
        assert_eq!(groups[1].hue, None, "an unknown color is no color");
    }
}
