//! What the shell does for a context menu: it opens the popup, and it runs the entry that the
//! owner picks.
//!
//! A menu entry and its palette entry run the same code. The menu knows its target. The palette
//! entry finds the target first: the active product, the focused pane, the story screen in view,
//! or the group of the active product (`palette_target`). A command that stops a program, spends
//! money or hides a product asks one Confirm before it runs (`menus::confirms`).

use agentsi_proto::marks_api::{ArchiveArchive, ArchiveKind, ArchiveParams};
use agentsi_proto::reflect_api::{ReflectRun, ReflectRunParams};
use agentsi_proto::story_api::{StoryGet, StoryIdParams};
use agentsi_proto::terminal_api::{Kill, TerminalId};
use agentsi_proto::ticket_api::{PromoteTo, TicketAdd, TicketAddParams, TicketKind, TicketState};
use agentsi_ui::tokens::label::Hue;
use agentsi_ui::tokens::{control, space};
use gpui_kit::component::Placement;
use gpui_kit::component::dock::PanelId;
use gpui_kit::component::menu::PopupMenu;
use gpui_kit::*;

use super::{Ask, PaneRef, Shell, shell_argv_for};
use crate::actions;
use crate::look::LookGlobal;
use crate::menus::{self, Cmd, Facts, Item, Kind, MenuRequest, Target};
use crate::model::ProductRef;
use crate::pane::TerminalPanel;
use crate::prompt::{Choice, Prompt};
use crate::screens::meeting;
use crate::screens::ticket::{self, Op};
use crate::screens::{ScreenKind, ScreenPanel, ScreenSpec};
use crate::tabs::GroupId;

/// The answer of the Confirm that runs the command.
const YES: &str = "yes";
const NO: &str = "no";

/// A menu that is open.
pub(super) struct OpenMenu {
    popup: Entity<PopupMenu>,
    position: Point<Pixels>,
    _dismiss: Subscription,
}

/// `~/x` as an absolute path. A product keeps the path as the owner wrote it.
fn expand(path: &str) -> String {
    match (path.strip_prefix("~/"), std::env::var("HOME")) {
        (Some(rest), Ok(home)) => format!("{home}/{rest}"),
        _ => path.to_string(),
    }
}

impl Shell {
    /// The facts that decide which entries of the menu of `target` are on.
    pub(super) fn menu_facts(&self, target: &Target, cx: &App) -> Facts {
        let core = |host: &Option<String>| self.cores.of(host.as_deref()).map(|c| c.read(cx));
        let meeting = |p: &ProductRef| {
            core(&p.host)
                .and_then(|c| c.meetings.as_ref())
                .is_some_and(|f| f.read(cx).0.live(&p.id).is_some_and(|m| m.running()))
        };
        match target {
            Target::Product(p) => {
                let Some(c) = core(&p.host) else {
                    return Facts::default();
                };
                Facts {
                    folder: c.product(&p.id).is_some_and(|x| x.path.is_some()),
                    local: p.host.is_none(),
                    meeting: meeting(p),
                    reflecting: c.reflecting.contains(&p.id),
                    archived: c.archived_products.iter().any(|x| x.id == p.id),
                    ..Facts::default()
                }
            }
            Target::Pane(p) => {
                let Some(t) = core(&p.host).and_then(|c| c.terminals.get(&p.id)) else {
                    return Facts::default();
                };
                let tab = self.pane_panel(p, cx).map(|t| PanelId::from(t.entity_id()));
                Facts {
                    running: t.running,
                    voice: t.voice,
                    folder: !t.cwd.is_empty(),
                    local: p.host.is_none(),
                    tabbed: tab.is_some(),
                    grouped: tab.is_some_and(|t| self.tabs.read(cx).group_of(t).is_some()),
                    ..Facts::default()
                }
            }
            Target::Meeting(p) => Facts {
                meeting: meeting(p),
                ..Facts::default()
            },
            Target::Story(p, id) => {
                let session = core(&p.host)
                    .and_then(|c| c.sessions.get(&p.id))
                    .and_then(|v| v.iter().find(|s| s.id == *id));
                Facts {
                    session: session.is_some(),
                    ready: session.is_some_and(|s| s.state == "ready"),
                    ..Facts::default()
                }
            }
            Target::Ticket(_, _, state) => Facts {
                ticket: Some(*state),
                ..Facts::default()
            },
            Target::Reflection(_) | Target::Group(..) | Target::TabGroup(_) => Facts::default(),
        }
    }

    /// Opens the menu of `request.target` at `request.position`.
    pub(super) fn open_menu(
        &mut self,
        request: MenuRequest,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let items = menus::items(&request.target, &self.menu_facts(&request.target, cx));
        let shell = cx.weak_entity();
        let target = request.target;
        // The focus goes back where it was, unless the entry moves it.
        let back = window.focused(cx).unwrap_or_else(|| self.focus.clone());
        let popup = menus::popup(
            &items,
            move |cmd, window, cx| {
                let _ = shell.update(cx, |this, cx| {
                    this.run_menu(target.clone(), cmd, window, cx)
                });
            },
            back,
            window,
            cx,
        );
        window.focus(&popup.focus_handle(cx), cx);
        let dismiss = cx.subscribe(&popup, |this, _, _: &DismissEvent, cx| {
            this.menu = None;
            cx.notify();
        });
        self.menu = Some(OpenMenu {
            popup,
            position: request.position,
            _dismiss: dismiss,
        });
        cx.notify();
    }

    /// The open menu, as a layer over the window.
    pub(super) fn menu_layer(&self) -> Option<AnyElement> {
        let m = self.menu.as_ref()?;
        Some(
            deferred(
                anchored()
                    .position(m.position)
                    .snap_to_window_with_margin(space(2))
                    .anchor(Anchor::TopLeft)
                    .child(m.popup.clone()),
            )
            .with_priority(gpui_kit::base::POPUP_PRIORITY)
            .into_any_element(),
        )
    }

    /// The menu key: the menu of the row under the cursor of the sidebar, else of the pane in view.
    pub(super) fn open_context_menu(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.sidebar.read(cx).has_focus(window, cx)
            && self.sidebar.update(cx, |s, cx| s.open_menu_at_cursor(cx))
        {
            return;
        }
        let on_tickets = self.open_screens(cx).iter().any(|s| {
            let s = s.read(cx);
            matches!(s.kind, ScreenKind::Ticket | ScreenKind::Tickets)
                && s.focus_handle(cx).contains_focused(window, cx)
        });
        if on_tickets && let Some(target) = self.palette_target(Kind::Ticket, window, cx) {
            let at = self.dock.read(cx).bounds().origin;
            let position = Point::new(at.x + space(8), at.y + control::TOOLBAR_HEIGHT);
            self.open_menu(MenuRequest { target, position }, window, cx);
            return;
        }
        match self.focused_terminal(window, cx) {
            Some(p) => {
                let target = Target::Pane(PaneRef {
                    host: p.read(cx).host(cx),
                    id: p.read(cx).id.clone(),
                });
                // Just under the top left corner of the tab bar.
                let at = self.dock.read(cx).bounds().origin;
                let position = Point::new(at.x + space(8), at.y + control::TOOLBAR_HEIGHT);
                self.open_menu(MenuRequest { target, position }, window, cx);
            }
            None => self.notice("No row or pane has the focus. Click one first.", cx),
        }
    }

    /// Runs `cmd` on `target`. A command that asks a Confirm asks it first.
    pub fn run_menu(
        &mut self,
        target: Target,
        cmd: Cmd,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let facts = self.menu_facts(&target, cx);
        let off = menus::items(&target, &facts)
            .into_iter()
            .any(|i| matches!(i, Item::Entry(e) if e.cmd == cmd && !e.enabled));
        if off {
            self.notice("That action is not available now.", cx);
        } else if menus::confirms(cmd, &facts) {
            self.ask(Ask::Confirm(target, cmd), window, cx);
        } else {
            self.perform(target, cmd, window, cx);
        }
    }

    /// The entry of the palette: the target is the active product, the focused pane, the story
    /// in view or the group of the active product.
    pub(super) fn run_active(
        &mut self,
        kind: Kind,
        cmd: Cmd,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        match self.palette_target(kind, window, cx) {
            Some(target) => self.run_menu(target, cmd, window, cx),
            None => self.notice(
                match kind {
                    Kind::Pane => "No pane is in view. Open a terminal first.",
                    Kind::Story => "No story session is open. Open one in the sidebar first.",
                    Kind::Ticket => "No ticket is in view. Open the tickets of a product first.",
                    Kind::Group => "The active product is in no group.",
                    Kind::TabGroup => "The active tab is in no tab group.",
                    _ => "No active product. Pick one in the sidebar first.",
                },
                cx,
            ),
        }
    }

    fn palette_target(&self, kind: Kind, window: &Window, cx: &App) -> Option<Target> {
        let active = || self.core.read(cx).active.clone();
        match kind {
            Kind::Product => active().map(Target::Product),
            Kind::Meeting => active().map(Target::Meeting),
            Kind::Reflection => active().map(Target::Reflection),
            Kind::Pane => self.focused_terminal(window, cx).map(|p| {
                Target::Pane(PaneRef {
                    host: p.read(cx).host(cx),
                    id: p.read(cx).id.clone(),
                })
            }),
            Kind::Group => {
                let p = active()?;
                let look = LookGlobal::with(cx, crate::look::LookData::clone)?;
                let name = look.group_of(p.host.as_deref(), &p.id)?.to_string();
                Some(Target::Group(p.host, name))
            }
            Kind::TabGroup => {
                let tab = self.active_tab(window, cx)?;
                self.tabs.read(cx).group_of(tab).map(Target::TabGroup)
            }
            Kind::Ticket => {
                // The ticket screens: the one with the focus, else the one in view.
                let screens = self.open_screens(cx);
                let pick = |focused: bool| {
                    screens.iter().find_map(|s| {
                        let s = s.read(cx);
                        let tickets = matches!(s.kind, ScreenKind::Ticket | ScreenKind::Tickets);
                        let on = if focused {
                            s.focus_handle(cx).contains_focused(window, cx)
                        } else {
                            s.shown
                        };
                        let (id, state) = s.ticket_in_view(cx).filter(|_| tickets && on)?;
                        let product = ProductRef {
                            host: s.host.clone(),
                            id: s.product.clone(),
                        };
                        Some(Target::Ticket(product, id, state))
                    })
                };
                pick(true).or_else(|| pick(false))
            }
            Kind::Story => {
                // The story screen with the focus, else the first session of the active product.
                let shown = self.open_screens(cx).into_iter().find_map(|s| {
                    let s = s.read(cx);
                    let focused = s.focus_handle(cx).contains_focused(window, cx);
                    (s.kind == ScreenKind::Story && focused).then(|| {
                        let product = ProductRef {
                            host: s.host.clone(),
                            id: s.product.clone(),
                        };
                        s.arg.clone().map(|id| Target::Story(product, id))
                    })?
                });
                shown.or_else(|| {
                    let p = active()?;
                    let id = self
                        .cores
                        .of(p.host.as_deref())?
                        .read(cx)
                        .sessions
                        .get(&p.id)?
                        .first()?
                        .id
                        .clone();
                    Some(Target::Story(p, id))
                })
            }
        }
    }

    fn perform(&mut self, target: Target, cmd: Cmd, window: &mut Window, cx: &mut Context<Self>) {
        match target {
            Target::Product(p) => self.product_cmd(p, cmd, window, cx),
            Target::Pane(p) => self.pane_cmd(p, cmd, window, cx),
            Target::Meeting(p) => match cmd {
                Cmd::Open => self.show_screen(ScreenSpec::meeting(&p.id).on(p.host), window, cx),
                Cmd::CancelMeeting => {
                    if let Some(core) = self.cores.of(p.host.as_deref()).cloned() {
                        meeting::cancel(&p.id, &core, cx);
                    }
                }
                _ => {}
            },
            Target::Reflection(p) => {
                if cmd == Cmd::Open {
                    self.show_screen(ScreenSpec::memory(&p.id).on(p.host), window, cx);
                }
            }
            Target::Story(p, id) => self.story_cmd(p, id, cmd, window, cx),
            Target::Ticket(p, id, _) => self.ticket_cmd(p, id, cmd, window, cx),
            Target::TabGroup(g) => self.tab_group_cmd(g, cmd, window, cx),
            Target::Group(host, name) => match cmd {
                Cmd::RenameGroup => self.ask(Ask::GroupName(host, name), window, cx),
                Cmd::ColorGroup => self.ask(Ask::GroupColor(host, name), window, cx),
                Cmd::RemoveGroup => {
                    if let Some(look) = self.look.clone() {
                        look.update(cx, |l, cx| l.remove_group(host.as_deref(), &name, cx));
                    }
                }
                _ => {}
            },
        }
    }

    /// The folder of a product, as an absolute path.
    fn folder_of(&self, p: &ProductRef, cx: &App) -> Option<String> {
        let core = self.cores.of(p.host.as_deref())?.read(cx);
        core.product(&p.id)?.path.as_deref().map(expand)
    }

    fn product_cmd(
        &mut self,
        p: ProductRef,
        cmd: Cmd,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        match cmd {
            Cmd::NewTerminal => {
                let argv = shell_argv_for(p.host.is_some());
                self.open_terminal_in(Some(p), argv, window, cx)
            }
            Cmd::OpenDesk => self.select_product(&p, window, cx),
            Cmd::StartMeeting => {
                self.show_screen(ScreenSpec::meeting(&p.id).on(p.host.clone()), window, cx);
                self.start_meeting(&p, cx);
            }
            Cmd::NewStory | Cmd::Suggest => {
                self.show_screen(ScreenSpec::stories(&p.id).on(p.host.clone()), window, cx);
                let spec = ScreenSpec::stories(&p.id).on(p.host);
                if let Some(panel) = self.story_panel(&spec, cx) {
                    panel.update(cx, |panel, cx| match cmd {
                        Cmd::NewStory => panel.stories_new(cx),
                        _ => panel.stories_suggest(cx),
                    });
                }
            }
            Cmd::NewTicket => self.ask(Ask::NewTicket(p), window, cx),
            Cmd::OpenTickets => self.show_screen(ScreenSpec::tickets(&p.id).on(p.host), window, cx),
            Cmd::Reflect => self.reflect(p, cx),
            Cmd::CancelReflect => self.cancel_reflect(p, cx),
            Cmd::OpenGit => self.show_screen(ScreenSpec::git(&p.id).on(p.host), window, cx),
            Cmd::Label => self.ask(Ask::ProductLabel(p), window, cx),
            Cmd::Color => self.ask(Ask::ProductColor(p), window, cx),
            Cmd::MoveToGroup => self.ask(Ask::ProductGroup(p), window, cx),
            Cmd::CopyPath => match self.folder_of(&p, cx) {
                Some(path) => cx.write_to_clipboard(ClipboardItem::new_string(path)),
                None => self.notice("This product has no folder.", cx),
            },
            Cmd::OpenFolder => match self.folder_of(&p, cx) {
                Some(path) if p.host.is_none() => {
                    cx.open_url(&crate::format::file_url(std::path::Path::new(&path)))
                }
                _ => self.notice("The folder is not on this device.", cx),
            },
            Cmd::DeleteData => {
                self.show_screen(ScreenSpec::settings(), window, cx);
                for panel in self.open_screens(cx) {
                    if panel.read(cx).is(&ScreenSpec::settings()) {
                        let id = Some(p.id.clone());
                        panel.update(cx, |panel, cx| panel.reveal_data(id, cx));
                    }
                }
            }
            Cmd::Archive => self.archive(p, cx),
            Cmd::RestoreProduct => self.restore_product(p, cx),
            _ => {}
        }
    }

    /// `reflect.run` for the product. The tree shows the reflection row while it runs.
    fn reflect(&mut self, p: ProductRef, cx: &mut Context<Self>) {
        let Some(core) = self.cores.of(p.host.as_deref()).cloned() else {
            return;
        };
        core.update(cx, |s, cx| {
            if s.set_reflecting(&p.id, true) {
                cx.notify();
            }
        });
        let call = core.read(cx).link.call::<ReflectRun>(&ReflectRunParams {
            product: p.id.clone(),
            memory_only: false,
        });
        cx.spawn(async move |this, cx| {
            let result = call.await;
            core.update(cx, |s, cx| {
                if s.set_reflecting(&p.id, false) {
                    cx.notify();
                }
            });
            let text = match result {
                Ok(r) if r.entry.is_some() => format!("Reflected on {}.", p.id),
                Ok(_) => format!("Nothing to reflect on in {}.", p.id),
                Err(e) => format!("No reflection of {}: {e}", p.id),
            };
            let _ = this.update(cx, |this, cx| this.notice(&text, cx));
        })
        .detach();
    }

    /// `archive.restore` for an archived product. The core tells the app when the list changes.
    fn restore_product(&mut self, p: ProductRef, cx: &mut Context<Self>) {
        let Some(core) = self.cores.of(p.host.as_deref()).cloned() else {
            return;
        };
        let call = core
            .read(cx)
            .link
            .call::<agentsi_proto::marks_api::ArchiveRestore>(&ArchiveParams {
                product: p.id.clone(),
                kind: ArchiveKind::Product,
                id: None,
            });
        cx.spawn(async move |this, cx| {
            let text = match call.await {
                Ok(_) => format!("{} is restored.", p.id),
                Err(e) => format!("{} was not restored: {e}", p.id),
            };
            let _ = this.update(cx, |this, cx| this.notice(&text, cx));
        })
        .detach();
    }

    /// `reflect.cancel` for the product.
    fn cancel_reflect(&mut self, p: ProductRef, cx: &mut Context<Self>) {
        let Some(core) = self.cores.of(p.host.as_deref()).cloned() else {
            return;
        };
        let call = core
            .read(cx)
            .link
            .call::<agentsi_proto::reflect_api::ReflectCancel>(
                &agentsi_proto::reflect_api::ReflectCancelParams {
                    product: p.id.clone(),
                },
            );
        cx.spawn(async move |this, cx| {
            let text = match call.await {
                Ok(r) if r.canceled => format!("The reflection of {} stops.", p.id),
                Ok(_) => format!("No reflection of {} runs.", p.id),
                Err(e) => format!("Could not cancel: {e}"),
            };
            let _ = this.update(cx, |this, cx| this.notice(&text, cx));
        })
        .detach();
    }

    /// `archive.archive` for the whole product. The core tells the app when the list changes.
    fn archive(&mut self, p: ProductRef, cx: &mut Context<Self>) {
        let Some(core) = self.cores.of(p.host.as_deref()).cloned() else {
            return;
        };
        let call = core.read(cx).link.call::<ArchiveArchive>(&ArchiveParams {
            product: p.id.clone(),
            kind: ArchiveKind::Product,
            id: None,
        });
        cx.spawn(async move |this, cx| {
            if let Err(e) = call.await {
                let _ = this.update(cx, |this, cx| {
                    this.notice(&format!("{} was not archived: {e}", p.id), cx)
                });
            }
        })
        .detach();
    }

    /// The panel of a pane, when the dock has its tab.
    fn pane_panel(&self, pane: &PaneRef, cx: &App) -> Option<Entity<TerminalPanel>> {
        self.open
            .borrow()
            .terminals
            .iter()
            .filter_map(WeakEntity::upgrade)
            .find(|p| {
                p.read(cx).is(pane.host.as_deref(), &pane.id, cx) && self.in_dock(p.entity_id(), cx)
            })
    }

    fn pane_cmd(&mut self, pane: PaneRef, cmd: Cmd, window: &mut Window, cx: &mut Context<Self>) {
        match cmd {
            Cmd::Rename => self.ask(Ask::PaneLabel(pane), window, cx),
            Cmd::ColorPane => self.ask(Ask::PaneColor(pane), window, cx),
            Cmd::TalkToPane | Cmd::VoiceMode => {
                // A pane with no tab opens one first, so the switch of the header has a place.
                if self.pane_panel(&pane, cx).is_none() {
                    self.focus_terminal(pane.host.clone(), &pane.id, window, cx);
                }
                if let Some(panel) = self.pane_panel(&pane, cx) {
                    panel.update(cx, |p, cx| match cmd {
                        Cmd::TalkToPane => p.talk(cx),
                        _ => p.toggle_voice(cx),
                    });
                }
            }
            Cmd::SplitRight => self.split(pane, Placement::Right, window, cx),
            Cmd::SplitDown => self.split(pane, Placement::Bottom, window, cx),
            Cmd::CopyFolder => {
                let cwd = self
                    .cores
                    .of(pane.host.as_deref())
                    .and_then(|c| c.read(cx).terminals.get(&pane.id).map(|t| t.cwd.clone()));
                match cwd.filter(|c| !c.is_empty()) {
                    Some(cwd) => cx.write_to_clipboard(ClipboardItem::new_string(cwd)),
                    None => self.notice("The folder of this pane is not known.", cx),
                }
            }
            Cmd::ClosePane => self.close_pane(pane, window, cx),
            Cmd::NewTabGroup
            | Cmd::AddToTabGroup
            | Cmd::LeaveTabGroup
            | Cmd::CloseOthers
            | Cmd::CloseToRight => {
                let Some(tab) = self
                    .pane_panel(&pane, cx)
                    .map(|t| PanelId::from(t.entity_id()))
                else {
                    self.notice("This pane has no tab. Click its row first.", cx);
                    return;
                };
                match cmd {
                    Cmd::NewTabGroup => self.ask(Ask::NewTabGroup(tab), window, cx),
                    Cmd::AddToTabGroup => self.ask(Ask::AddToTabGroup(tab), window, cx),
                    Cmd::LeaveTabGroup => self.leave_group(tab, window, cx),
                    Cmd::CloseOthers => self.close_others(tab, window, cx),
                    _ => self.close_to_the_right(tab, window, cx),
                }
            }
            _ => {}
        }
    }

    fn tab_group_cmd(
        &mut self,
        group: GroupId,
        cmd: Cmd,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        match cmd {
            Cmd::TabGroupRename => self.ask(Ask::RenameTabGroup(group), window, cx),
            Cmd::TabGroupColor => self.ask(Ask::ColorTabGroup(group), window, cx),
            Cmd::Ungroup => self.ungroup(group, window, cx),
            Cmd::CloseGroup => self.close_group(group, window, cx),
            _ => {}
        }
    }

    /// A new terminal in the folder of `pane`, in a new tab group at its side.
    fn split(
        &mut self,
        pane: PaneRef,
        placement: Placement,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(core) = self.cores.of(pane.host.as_deref()).cloned() else {
            return;
        };
        let Some(t) = core.read(cx).terminals.get(&pane.id).cloned() else {
            self.notice("The terminal of this pane is gone.", cx);
            return;
        };
        let beside = self
            .pane_panel(&pane, cx)
            .map(|p| (PanelId::from(p.entity_id()), placement));
        let argv = shell_argv_for(pane.host.is_some());
        self.create_terminal(core, t.cwd, argv, t.product, beside, window, cx);
    }

    /// Closes the tab of the pane, which kills the terminal, behind the Confirm of the tabs. A
    /// pane with no tab is killed directly.
    fn close_pane(&mut self, pane: PaneRef, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(panel) = self.pane_panel(&pane, cx) {
            self.close_tabs(vec![PanelId::from(panel.entity_id())], window, cx);
            return;
        }
        let Some(core) = self.cores.of(pane.host.as_deref()).cloned() else {
            return;
        };
        let call = core.read(cx).link.call::<Kill>(&TerminalId {
            id: pane.id.clone(),
        });
        core.update(cx, |s, cx| {
            s.close(&pane.id);
            cx.notify();
        });
        cx.background_spawn(async move {
            let _ = call.await;
        })
        .detach();
    }

    fn story_panel(&self, spec: &ScreenSpec, cx: &App) -> Option<Entity<ScreenPanel>> {
        self.open_screens(cx)
            .into_iter()
            .find(|p| p.read(cx).is(spec))
    }

    fn story_cmd(
        &mut self,
        p: ProductRef,
        id: String,
        cmd: Cmd,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let spec = ScreenSpec::story(&p.id, &id).on(p.host.clone());
        match cmd {
            Cmd::Open => self.show_screen(spec, window, cx),
            // The story screen shows the diff. Review reads it again.
            Cmd::Review | Cmd::Accept | Cmd::Reject => {
                self.show_screen(spec.clone(), window, cx);
                if let Some(panel) = self.story_panel(&spec, cx) {
                    panel.update(cx, |panel, cx| match cmd {
                        Cmd::Review => panel.reload(cx),
                        Cmd::Accept => panel.story_act("accept", cx),
                        _ => panel.story_act("reject", cx),
                    });
                }
            }
            Cmd::StoryTerminal => self.story_terminal(p, id, window, cx),
            Cmd::ArchiveStory | Cmd::RestoreStory | Cmd::RecordMerge => {
                self.show_screen(spec.clone(), window, cx);
                if let Some(panel) = self.story_panel(&spec, cx) {
                    panel.update(cx, |panel, cx| {
                        panel.story_act(
                            match cmd {
                                Cmd::ArchiveStory => "archive",
                                Cmd::RestoreStory => "restore",
                                _ => "merge",
                            },
                            cx,
                        )
                    });
                }
            }
            _ => {}
        }
    }

    fn ticket_cmd(
        &mut self,
        p: ProductRef,
        id: String,
        cmd: Cmd,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let op = match cmd {
            Cmd::Open => {
                self.show_screen(ScreenSpec::ticket(&p.id, &id).on(p.host), window, cx);
                return;
            }
            Cmd::CopyId => {
                cx.write_to_clipboard(ClipboardItem::new_string(id));
                return;
            }
            Cmd::TicketWait => {
                self.ask(Ask::TicketWait(p, id), window, cx);
                return;
            }
            Cmd::TicketDone => Op::Move(TicketState::Done, None),
            Cmd::TicketDrop => Op::Move(TicketState::Dropped, None),
            Cmd::TicketReopen => Op::Move(TicketState::Open, None),
            Cmd::PromoteStory => Op::Promote(PromoteTo::Story),
            Cmd::PromoteDecision => Op::Promote(PromoteTo::Decision),
            _ => return,
        };
        self.ticket_op(p, id, op, cx);
    }

    /// Sends a move or a promote of the ticket. The core tells the screens when it is done.
    fn ticket_op(&mut self, p: ProductRef, id: String, op: Op, cx: &mut Context<Self>) {
        let Some(core) = self.cores.of(p.host.as_deref()).cloned() else {
            return;
        };
        let done = ticket::run(&core, cx, &p.id, &id, op);
        cx.spawn(async move |this, cx| {
            let text = match done.await {
                Ok(line) => line,
                Err(e) => format!("Ticket {id} was not changed: {e}"),
            };
            let _ = this.update(cx, |this, cx| this.notice(&text, cx));
        })
        .detach();
    }

    /// `ticket.add` with the title of the owner. The menu adds a note.
    fn new_ticket(&mut self, p: ProductRef, title: String, cx: &mut Context<Self>) {
        let Some(core) = self.cores.of(p.host.as_deref()).cloned() else {
            return;
        };
        let call = core.read(cx).link.call::<TicketAdd>(&TicketAddParams {
            product: p.id.clone(),
            kind: TicketKind::Note,
            title,
            body: None,
            assignee: None,
            links: None,
            scope: None,
            objective: None,
            author: ticket::owner(),
        });
        cx.spawn(async move |this, cx| {
            let text = match call.await {
                Ok(r) => format!("Added {} to {}.", r.ticket.id, p.id),
                Err(e) => format!("No ticket for {}: {e}", p.id),
            };
            let _ = this.update(cx, |this, cx| this.notice(&text, cx));
        })
        .detach();
    }

    /// A terminal in the worktree of the story session.
    fn story_terminal(
        &mut self,
        p: ProductRef,
        id: String,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(core) = self.cores.of(p.host.as_deref()).cloned() else {
            return;
        };
        let call = core.read(cx).link.call::<StoryGet>(&StoryIdParams {
            product: p.id.clone(),
            id: id.clone(),
        });
        cx.spawn_in(window, async move |this, cx| {
            let worktree = match call.await {
                Ok(r) => r.story.session.map(|s| s.worktree),
                Err(e) => {
                    let _ = this.update(cx, |this, cx| {
                        this.notice(&format!("No worktree for {id}: {e}"), cx)
                    });
                    return;
                }
            };
            let _ = this.update_in(cx, |this, window, cx| match worktree {
                Some(dir) => {
                    let argv = shell_argv_for(p.host.is_some());
                    this.create_terminal(core, dir, argv, Some(p.id), None, window, cx)
                }
                None => this.notice(&format!("{id} has no session yet."), cx),
            });
        })
        .detach();
    }

    fn product_name(&self, p: &ProductRef, cx: &App) -> String {
        self.cores
            .of(p.host.as_deref())
            .and_then(|c| c.read(cx).product(&p.id).map(|x| x.name.clone()))
            .unwrap_or_else(|| p.id.clone())
    }

    /// The prompt for an `Ask` of the menus, or `None` for another kind of question.
    pub(super) fn menu_prompt(
        &mut self,
        what: &Ask,
        colors: &[Choice],
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Option<Entity<Prompt>> {
        let data = LookGlobal::with(cx, crate::look::LookData::clone).unwrap_or_default();
        match what {
            Ask::NewTicket(p) => {
                let title = format!("New ticket for {}", self.product_name(p, cx));
                Some(cx.new(|cx| {
                    Prompt::text(
                        title,
                        "Write the title, 120 characters or less. It becomes a note.",
                        "",
                        Vec::new(),
                        window,
                        cx,
                    )
                }))
            }
            Ask::TicketWait(_, id) => {
                let title = format!("{id} waits on");
                Some(cx.new(|cx| {
                    Prompt::text(
                        title,
                        "Write who or what the ticket waits on.",
                        "",
                        Vec::new(),
                        window,
                        cx,
                    )
                }))
            }
            Ask::GroupName(_, name) => {
                let now = name.clone();
                Some(cx.new(|cx| {
                    Prompt::text(
                        "Name for this group",
                        "A name of 40 characters or less.",
                        &now,
                        Vec::new(),
                        window,
                        cx,
                    )
                }))
            }
            Ask::GroupColor(host, name) => {
                let at = data
                    .groups
                    .iter()
                    .find(|g| g.host == *host && g.name == *name)
                    .and_then(|g| g.hue)
                    .and_then(|h: Hue| colors.iter().position(|c| c.value == h.key()))
                    .unwrap_or(0);
                let colors = colors.to_vec();
                Some(cx.new(|cx| Prompt::pick("Color for this group", colors, at, window, cx)))
            }
            Ask::Confirm(target, cmd) => {
                let name = match target {
                    Target::Pane(p) => self
                        .cores
                        .of(p.host.as_deref())
                        .map(|c| crate::pane::shown(c.read(cx), &p.id, cx).title)
                        .unwrap_or_else(|| p.id.clone()),
                    Target::Product(p) | Target::Meeting(p) | Target::Reflection(p) => {
                        self.product_name(p, cx)
                    }
                    Target::Story(p, _) => self.product_name(p, cx),
                    Target::Ticket(_, id, _) => id.clone(),
                    Target::Group(_, name) => name.clone(),
                    Target::TabGroup(_) => "this tab group".to_string(),
                };
                let (title, verb) = menus::confirm_text(*cmd, &name);
                let choices = vec![Choice::new(verb, YES), Choice::new("Keep it as it is", NO)];
                // Enter keeps: the safe answer is the one under the cursor.
                Some(cx.new(|cx| Prompt::pick(title, choices, 1, window, cx)))
            }
            _ => None,
        }
    }

    /// Runs the answer of a question of the menus. Returns false for another kind of question.
    pub(super) fn menu_answer(
        &mut self,
        what: &Ask,
        answer: &str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> bool {
        match what {
            Ask::NewTicket(p) => {
                let title = answer.trim();
                if !title.is_empty() {
                    self.new_ticket(p.clone(), title.to_string(), cx);
                }
            }
            Ask::TicketWait(p, id) => {
                let target = answer.trim();
                if !target.is_empty() {
                    let op = Op::Move(TicketState::Waiting, Some(target.to_string()));
                    self.ticket_op(p.clone(), id.clone(), op, cx);
                }
            }
            Ask::GroupName(host, name) => {
                if let Some(look) = self.look.clone() {
                    let done = look.update(cx, |l, cx| {
                        l.rename_group(host.as_deref(), name, answer, cx)
                    });
                    if !done {
                        self.notice("The group was not renamed: the name is empty or taken.", cx);
                    }
                }
            }
            Ask::GroupColor(host, name) => {
                if let Some(look) = self.look.clone() {
                    let hue = Hue::from_key(answer);
                    look.update(cx, |l, cx| l.set_group_hue(host.as_deref(), name, hue, cx));
                }
            }
            Ask::Confirm(target, cmd) => {
                if answer == YES {
                    self.perform(target.clone(), *cmd, window, cx);
                }
            }
            _ => return false,
        }
        true
    }
}

/// Adds the palette actions that run a menu command to the root element of the shell.
pub(super) fn handlers(root: Stateful<Div>, cx: &mut Context<Shell>) -> Stateful<Div> {
    macro_rules! palette {
        ($root:ident, $( $action:ident => ($kind:ident, $cmd:ident) ),* $(,)?) => {{
            let r = $root;
            $(
                let r = r.on_action(cx.listener(|this, _: &actions::$action, window, cx| {
                    this.run_active(Kind::$kind, Cmd::$cmd, window, cx)
                }));
            )*
            r
        }};
    }
    palette!(
        root,
        StartMeeting => (Product, StartMeeting),
        CancelMeeting => (Meeting, CancelMeeting),
        NewStory => (Product, NewStory),
        SuggestStories => (Product, Suggest),
        ArchiveStorySession => (Story, ArchiveStory),
        RestoreStorySession => (Story, RestoreStory),
        RecordStoryMerge => (Story, RecordMerge),
        NewTicket => (Product, NewTicket),
        OpenTicket => (Ticket, Open),
        TicketDone => (Ticket, TicketDone),
        TicketWait => (Ticket, TicketWait),
        TicketDrop => (Ticket, TicketDrop),
        TicketReopen => (Ticket, TicketReopen),
        PromoteTicketStory => (Ticket, PromoteStory),
        PromoteTicketDecision => (Ticket, PromoteDecision),
        CopyTicketId => (Ticket, CopyId),
        ReflectProduct => (Product, Reflect),
        CancelReflection => (Product, CancelReflect),
        CopyProductPath => (Product, CopyPath),
        OpenProductFolder => (Product, OpenFolder),
        DeleteProductData => (Product, DeleteData),
        ArchiveProduct => (Product, Archive),
        SplitPaneRight => (Pane, SplitRight),
        SplitPaneDown => (Pane, SplitDown),
        CopyPaneFolder => (Pane, CopyFolder),
        ClosePane => (Pane, ClosePane),
        OpenStorySession => (Story, Open),
        ReviewStory => (Story, Review),
        AcceptStory => (Story, Accept),
        RejectStory => (Story, Reject),
        OpenStoryTerminal => (Story, StoryTerminal),
        RenameGroup => (Group, RenameGroup),
        ColorGroup => (Group, ColorGroup),
        RemoveGroup => (Group, RemoveGroup),
    )
}
