//! The tab bar with tab groups.
//!
//! `DockSkin` of `gpui-component` draws the tab bar, and its tab bar knows no chip. The public
//! `DockAreaRenderer` and `TabGroupRenderer` traits let the app wrap the skin and replace
//! only the tab bar. Everything else (the frame, the content, the drop outline, the split
//! handles, the dock) stays the skin's, so no part of `dock/` is copied.
//!
//! A bar with no tab group and with one tab keeps the skin's own title bar. Any other bar
//! is drawn here:
//!
//! - a chip before the tabs of each group, and its tabs after it with the color of the group as a
//!   3 px bar at the left edge. A collapsed group shows the chip only.
//! - a drop on a tab puts the dragged tab right before it, in the group of that tab. A drop on a
//!   chip puts it at the end of the group. A drop past the last tab takes it out of its group.
//!
//! The dock moves the tab. This module only chooses the slot and tells `TabGroups` the new
//! group, so a drag reorders inside one bar, moves to another bar and joins a group with one
//! gesture.

use std::cell::Cell;
use std::rc::Rc;
use std::sync::Arc;

use agentsi_ui::components::TabChip;
use agentsi_ui::style::StyleExt as _;
use agentsi_ui::tokens::{accent, control, line, radius, space, surface, text};
use gpui_kit::base::ResizeHandleContext;
use gpui_kit::component::button::{Button, ButtonCustomVariant, ButtonVariants as _};
use gpui_kit::component::dock::{
    BasePanelView, ClosePanel, DockArea, DockAreaRenderer, DockContext, DockSkin, DragPanel,
    DropIndicator, NodeId, PanelControl, PanelHandle, PanelId, PanelState, TabGroupContext,
    TabGroupRenderer, ToggleZoom,
};
use gpui_kit::component::menu::DropdownMenu as _;
use gpui_kit::component::tab::{Tab, TabBar};
use gpui_kit::component::{IconName, Selectable as _, Sizable as _};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;

use crate::layout;
use crate::menus::{self, Target};
use crate::shell::PaneRef;
use crate::tabs::{self, Group, GroupId, TabGroups, slot_after, slot_before};

/// The size that a drag reports for its preview, so a drop outline knows where to start.
fn preview_size() -> Size<Pixels> {
    size(space(24), control::height_md())
}

/// A dock area that wears `DockSkin` and draws the tab groups of `tabs`.
pub fn dock_area(
    id: impl Into<SharedString>,
    version: Option<usize>,
    tabs: Entity<TabGroups>,
    window: &mut Window,
    cx: &mut App,
) -> (Entity<DockArea>, Rc<DockSkin>) {
    let mut skin = None;
    let area = cx.new(|cx| {
        let inner = DockSkin::new(cx);
        skin = Some(inner.clone());
        let grouped = GroupedSkin {
            inner,
            area: cx.weak_entity(),
            tabs,
        };
        DockArea::new(id, version, window, cx).with_renderer(Rc::new(grouped))
    });
    (area, skin.expect("the constructor ran"))
}

struct GroupedSkin {
    inner: Rc<DockSkin>,
    area: WeakEntity<DockArea>,
    tabs: Entity<TabGroups>,
}

impl DockAreaRenderer for GroupedSkin {
    fn frame(&self, window: &mut Window, cx: &mut App) -> Stateful<Div> {
        self.inner.frame(window, cx)
    }

    fn split_frame(
        &self,
        node: NodeId,
        axis: Axis,
        window: &mut Window,
        cx: &mut App,
    ) -> Stateful<Div> {
        self.inner.split_frame(node, axis, window, cx)
    }

    fn center_frame(&self, window: &mut Window, cx: &mut App) -> Stateful<Div> {
        self.inner.center_frame(window, cx)
    }

    fn render_split_handle(
        &self,
        handle: &ResizeHandleContext,
        window: &mut Window,
        cx: &mut App,
    ) -> Option<AnyElement> {
        self.inner.render_split_handle(handle, window, cx)
    }

    fn render_dock(
        &self,
        dock: &DockContext,
        content: AnyElement,
        window: &mut Window,
        cx: &mut App,
    ) -> AnyElement {
        self.inner.render_dock(dock, content, window, cx)
    }

    fn build_placeholder(
        &self,
        state: &PanelState,
        window: &mut Window,
        cx: &mut App,
    ) -> Option<Arc<dyn BasePanelView>> {
        self.inner.build_placeholder(state, window, cx)
    }

    fn tab_group_renderer(&self) -> Rc<dyn TabGroupRenderer> {
        Rc::new(GroupedTabs {
            inner: self.inner.tab_group_renderer(),
            area: self.area.clone(),
            tabs: self.tabs.clone(),
            scroll: ScrollHandle::default(),
            last_active: Cell::new(None),
        })
    }
}

/// What the bar draws, in order: a chip, or the tab at an index of the group.
enum Item {
    Chip(GroupId),
    Tab(usize),
}

struct GroupedTabs {
    inner: Rc<dyn TabGroupRenderer>,
    area: WeakEntity<DockArea>,
    tabs: Entity<TabGroups>,
    scroll: ScrollHandle,
    /// The tab that the last frame displayed, to bring a new one into view.
    last_active: Cell<Option<usize>>,
}

impl TabGroupRenderer for GroupedTabs {
    fn frame(&self, group: &TabGroupContext, window: &mut Window, cx: &mut App) -> Stateful<Div> {
        self.inner.frame(group, window, cx)
    }

    fn content_frame(
        &self,
        group: &TabGroupContext,
        window: &mut Window,
        cx: &mut App,
    ) -> Stateful<Div> {
        self.inner.content_frame(group, window, cx)
    }

    fn render_active_panel(
        &self,
        panel: AnyView,
        group: &TabGroupContext,
        window: &mut Window,
        cx: &mut App,
    ) -> AnyElement {
        self.inner.render_active_panel(panel, group, window, cx)
    }

    fn render_drop_indicator(
        &self,
        indicator: DropIndicator,
        window: &mut Window,
        cx: &mut App,
    ) -> Option<AnyElement> {
        self.inner.render_drop_indicator(indicator, window, cx)
    }

    fn render_empty(
        &self,
        group: &TabGroupContext,
        window: &mut Window,
        cx: &mut App,
    ) -> Option<AnyElement> {
        self.inner.render_empty(group, window, cx)
    }

    fn render_tab_bar(
        &self,
        group: &TabGroupContext,
        window: &mut Window,
        cx: &mut App,
    ) -> AnyElement {
        let groups = self.tabs.read(cx).groups().to_vec();
        let visible: Vec<usize> = group
            .panels()
            .iter()
            .enumerate()
            .filter(|(_, p)| p.visible(cx))
            .map(|(ix, _)| ix)
            .collect();
        let grouped = visible.iter().any(|ix| {
            let id = group.panels()[*ix].panel_id(cx);
            groups.iter().any(|g| g.members.contains(&id))
        });
        if visible.len() < 2 && !grouped {
            return self.inner.render_tab_bar(group, window, cx);
        }
        self.bar(group, &groups, &visible, window, cx)
    }
}

/// Where a dropped tab goes.
#[derive(Clone, Copy)]
enum Place {
    /// Right before this tab, and in its group.
    Before(PanelId),
    /// At the end of this group.
    Into(GroupId),
    /// Past the last tab, with no group.
    End,
}

/// Drops `drag` on `place`: tells the groups, then lets the dock move the tab.
fn drop_on(
    group: &TabGroupContext,
    tabs: &Entity<TabGroups>,
    drag: &DragPanel,
    place: Place,
    window: &mut Window,
    cx: &mut App,
) {
    let panels: Vec<PanelId> = group.panels().iter().map(|p| p.panel_id(cx)).collect();
    let moving = drag.panel();
    let (slot, into, activate) = match place {
        Place::Before(target) if target == moving => return,
        Place::Before(target) => (
            Some(slot_before(&panels, target, moving)),
            tabs.read(cx).group_of(target),
            true,
        ),
        Place::Into(id) => {
            let members = tabs
                .read(cx)
                .get(id)
                .map(|g| g.members.clone())
                .unwrap_or_default();
            (Some(slot_after(&panels, &members, moving)), Some(id), true)
        }
        // A tab of this bar needs a slot to move. One from another bar is appended.
        Place::End => (
            (drag.source() == group.node()).then(|| panels.len().saturating_sub(1)),
            None,
            false,
        ),
    };
    tabs.update(cx, |t, cx| match into {
        Some(id) => t.join(moving, id, cx),
        None => t.leave(moving, cx),
    });
    group.drop_panel(drag.clone(), slot, activate, window, cx);
}

/// The preview that follows the pointer while a tab is dragged.
struct TabPreview {
    panel: Arc<dyn gpui_kit::component::dock::PanelView>,
}

impl Render for TabPreview {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        div()
            .id("drag-tab")
            .pad_x(space(3))
            .py(space(1))
            .max_w(space(48))
            .overflow_hidden()
            .whitespace_nowrap()
            .border_1()
            .border_color(line::strong())
            .rounded(radius::md())
            .bg(surface::overlay())
            .text_color(text::primary())
            .opacity(0.75)
            .child(self.panel.title(window, cx))
    }
}

/// The terminal behind a tab, for its context menu. A screen tab has no menu.
fn pane_of(panel: &Arc<dyn BasePanelView>, cx: &App) -> Option<PaneRef> {
    let state = panel.dump(cx);
    let id = layout::terminal_of(&state)?.to_string();
    Some(PaneRef {
        host: layout::host_of(&state).map(String::from),
        id,
    })
}

fn title_of(panel: &Arc<dyn BasePanelView>, window: &mut Window, cx: &mut App) -> AnyElement {
    match PanelHandle::of(panel) {
        Some(h) => match h.tab_name(cx) {
            Some(name) => name.into_any_element(),
            None => h.title(window, cx),
        },
        None => SharedString::from(panel.panel_name(cx)).into_any_element(),
    }
}

impl GroupedTabs {
    fn bar(
        &self,
        group: &TabGroupContext,
        groups: &[Group],
        visible: &[usize],
        window: &mut Window,
        cx: &mut App,
    ) -> AnyElement {
        let collapsed = group.is_collapsed();
        let droppable = group.is_droppable();
        let active_ix = group.active_ix();
        let displayed = group.active_panel().map(|p| p.panel_id(cx));
        let id_at = |ix: usize, cx: &App| group.panels()[ix].panel_id(cx);

        let mut items: Vec<Item> = Vec::new();
        for &ix in visible {
            let id = id_at(ix, cx);
            match groups.iter().find(|g| g.members.contains(&id)) {
                Some(g) => {
                    if !items
                        .iter()
                        .any(|i| matches!(i, Item::Chip(c) if *c == g.id))
                    {
                        items.push(Item::Chip(g.id));
                    }
                    if !g.collapsed {
                        items.push(Item::Tab(ix));
                    }
                }
                None => items.push(Item::Tab(ix)),
            }
        }
        // Bring a newly displayed tab into view.
        if self.last_active.replace(Some(active_ix)) != Some(active_ix)
            && let Some(at) = items
                .iter()
                .position(|i| matches!(i, Item::Tab(ix) if *ix == active_ix))
        {
            self.scroll.scroll_to_item(at);
        }

        let children: Vec<Tab> = items
            .iter()
            .map(|item| match item {
                Item::Chip(id) => {
                    let g = groups
                        .iter()
                        .find(|g| g.id == *id)
                        .expect("a group of the bar");
                    self.chip(group, g, groups, visible, displayed, droppable, cx)
                }
                Item::Tab(ix) => {
                    let g = groups
                        .iter()
                        .find(|g| g.members.contains(&id_at(*ix, cx)))
                        .cloned();
                    self.tab(group, *ix, g, displayed, droppable, window, cx)
                }
            })
            .collect();

        let last = group.panels().len();
        TabBar::new("tab-bar")
            .track_scroll(&self.scroll)
            .children(children)
            .last_empty_space(
                // Room to drop a tab past the last one.
                div()
                    .id("tab-bar-empty-space")
                    .debug_selector(|| "tab-bar-end".to_string())
                    .h_full()
                    .flex_grow_1()
                    .min_w(space(16))
                    .when(droppable, |this| {
                        this.drag_over::<DragPanel>(|this, _, _, _| {
                            this.bg(accent::base().alpha(0.12))
                        })
                        .on_drop({
                            let (group, tabs) = (group.clone(), self.tabs.clone());
                            move |drag: &DragPanel, window, cx| {
                                drop_on(&group, &tabs, drag, Place::End, window, cx)
                            }
                        })
                    }),
            )
            .when(!collapsed && last > 0, |this| {
                this.suffix(self.toolbar(group, window, cx))
            })
            .into_any_element()
    }

    #[allow(clippy::too_many_arguments)]
    fn chip(
        &self,
        group: &TabGroupContext,
        g: &Group,
        groups: &[Group],
        visible: &[usize],
        displayed: Option<PanelId>,
        droppable: bool,
        cx: &App,
    ) -> Tab {
        let _ = groups;
        let count = visible
            .iter()
            .filter(|ix| g.members.contains(&group.panels()[**ix].panel_id(cx)))
            .count();
        let holds_displayed = displayed.is_some_and(|d| g.members.contains(&d));
        let (id, collapsed) = (g.id, g.collapsed);
        let (area, tabs) = (self.area.clone(), self.tabs.clone());
        let chip = TabChip::new(
            ElementId::Name(format!("group-{id}").into()),
            g.name.clone(),
        )
        .hue(g.hue)
        .collapsed(collapsed)
        .count(count)
        .active(collapsed && holds_displayed)
        .on_click(move |_, window, cx| {
            if let Some(dock) = area.upgrade() {
                tabs::set_collapsed(&dock, &tabs, id, !collapsed, window, cx);
            }
        })
        .decorate({
            let (group, tabs) = (group.clone(), self.tabs.clone());
            move |el| {
                let el = el.on_mouse_down(MouseButton::Right, move |e, _, cx| {
                    menus::request(cx, Target::TabGroup(id), e.position)
                });
                if !droppable {
                    return el;
                }
                el.drag_over::<DragPanel>(|this, _, _, _| this.bg(accent::base().alpha(0.2)))
                    .on_drop(move |drag: &DragPanel, window, cx| {
                        drop_on(&group, &tabs, drag, Place::Into(id), window, cx)
                    })
            }
        });
        Tab::new().child(chip)
    }

    #[allow(clippy::too_many_arguments)]
    fn tab(
        &self,
        group: &TabGroupContext,
        ix: usize,
        member_of: Option<Group>,
        displayed: Option<PanelId>,
        droppable: bool,
        window: &mut Window,
        cx: &mut App,
    ) -> Tab {
        let panel = group.panels()[ix].clone();
        let id = panel.panel_id(cx);
        let bar_collapsed = group.is_collapsed();
        let drag = group
            .is_draggable()
            .then(|| group.drag_panel(ix, cx))
            .flatten();
        let preview = PanelHandle::of(&panel).map(PanelHandle::panel);
        let title = title_of(&panel, window, cx);
        let closable = !bar_collapsed && group.is_panel_closable(id, cx);
        let last_active = &self.last_active;
        let _ = last_active;

        Tab::new()
            .debug_selector(move || format!("tab:{}", id.as_u64()))
            .when_some(member_of.as_ref().and_then(|g| g.hue), |tab, hue| {
                tab.child(
                    div()
                        .debug_selector(move || format!("tab-group-bar:{}", id.as_u64()))
                        .absolute()
                        .left_0()
                        .top_0()
                        .bottom_0()
                        .w(control::COLOR_BAR)
                        .bg(hue.color()),
                )
            })
            .child(title)
            .when(closable, |tab| {
                tab.suffix(
                    Button::new(("close-tab", ix))
                        .icon(IconName::Close)
                        .xsmall()
                        .custom(
                            ButtonCustomVariant::new(cx)
                                .foreground(Hsla::from(text::secondary()))
                                .hover(Hsla::from(surface::overlay()))
                                .active(Hsla::from(surface::overlay())),
                        )
                        .ml(-space(2))
                        .mr(space(2))
                        .tab_stop(false)
                        .debug_selector(move || format!("tab-close:{}", id.as_u64()))
                        .on_click({
                            let group = group.clone();
                            move |_, window, cx| {
                                cx.stop_propagation();
                                group.close(id, window, cx);
                            }
                        }),
                )
            })
            .selected(!bar_collapsed && displayed == Some(id))
            .on_mouse_down(MouseButton::Right, {
                let panel = panel.clone();
                move |e, _, cx| {
                    if let Some(pane) = pane_of(&panel, cx) {
                        menus::request(cx, Target::Pane(pane), e.position)
                    }
                }
            })
            .on_click({
                let group = group.clone();
                move |_, window, cx| group.select_tab(ix, window, cx)
            })
            .when(!bar_collapsed, |tab| {
                tab.when_some(drag, |tab, drag| {
                    tab.on_drag(drag, move |drag, offset, _, cx| {
                        cx.stop_propagation();
                        drag.set_drag_offset(offset);
                        drag.set_preview_size(preview_size());
                        let panel = preview.clone().expect("a panel with a handle");
                        cx.new(|_| TabPreview { panel })
                    })
                })
                .when(droppable, |tab| {
                    tab.drag_over::<DragPanel>(|this, _, _, _| {
                        this.rounded_l_none()
                            .border_l_2()
                            .border_r_0()
                            .border_color(accent::base())
                    })
                    .on_drop({
                        let (group, tabs) = (group.clone(), self.tabs.clone());
                        move |drag: &DragPanel, window, cx| {
                            drop_on(&group, &tabs, drag, Place::Before(id), window, cx)
                        }
                    })
                })
            })
    }

    /// The buttons at the right end of the bar: the ones of the displayed panel, a zoom button
    /// when the panel asks for it, and the menu.
    fn toolbar(&self, group: &TabGroupContext, window: &mut Window, cx: &mut App) -> Div {
        let zoomed = group.is_zoomed();
        let handle = group.active_panel().and_then(PanelHandle::of);
        let control = group.active_panel().and_then(|p| {
            p.zoomable(cx)
                .then(|| PanelHandle::of(p).and_then(|h| h.zoom_control(cx)))
                .flatten()
        });
        let toolbar_zoom = control.is_some_and(|c: PanelControl| c.toolbar_visible());
        let menu_zoom = control.is_some_and(|c: PanelControl| c.menu_visible());
        let closable = group.is_closable();
        let buttons = handle.and_then(|h| h.toolbar_buttons(window, cx));
        let suffix = handle.and_then(|h| h.title_suffix(window, cx));
        let panel = handle.map(|h| h.panel());
        div()
            .flex()
            .items_center()
            .h_full()
            .flex_none()
            .border_l_1()
            .border_b_1()
            .border_color(line::border())
            .bg(surface::base())
            .pad_x(space(2))
            .gap(space(1))
            .occlude()
            .children(suffix)
            .when_some(buttons, |this, buttons| {
                this.children(
                    buttons
                        .into_iter()
                        .map(|b| b.xsmall().ghost().tab_stop(false)),
                )
            })
            .when(toolbar_zoom || zoomed, |this| {
                this.child(
                    Button::new("zoom")
                        .icon(if zoomed {
                            IconName::Minimize
                        } else {
                            IconName::Maximize
                        })
                        .xsmall()
                        .ghost()
                        .tab_stop(false)
                        .selected(zoomed)
                        .on_click({
                            let group = group.clone();
                            move |_, window, cx| group.toggle_zoom(window, cx)
                        }),
                )
            })
            .child(
                Button::new("menu")
                    .icon(IconName::Ellipsis)
                    .xsmall()
                    .ghost()
                    .tab_stop(false)
                    .dropdown_menu(move |menu, window, cx| {
                        menu.when_some(panel.clone(), |menu, panel| {
                            panel.dropdown_menu(menu, window, cx)
                        })
                        .separator()
                        .menu_with_disabled(
                            if zoomed { "Zoom out" } else { "Zoom in" },
                            Box::new(ToggleZoom),
                            !menu_zoom,
                        )
                        .when(closable, |menu| {
                            menu.separator().menu("Close", Box::new(ClosePanel))
                        })
                    })
                    .anchor(Anchor::TopRight),
            )
    }
}
