//! What the shell does for the entries of `agentsi-desktop-palette-more.md`: the second steps
//! of the palette, the open tabs, a found file, "Reopen closed tab", the next space, a restart
//! of the core and a read of the settings files.

use agentsi_proto::core_api::{Empty, Shutdown};
use agentsi_proto::git_api::{GitBranches, GitBranchesResult, GitRepoParams};
use gpui_kit::component::dock::PanelId;
use gpui_kit::*;

use super::{Ask, Shell, shell_argv_for};
use crate::favorites::FavoritesGlobal;
use crate::look::LookGlobal;
use crate::model::ProductRef;
use crate::palette::{Closed, ClosedTab, Opened};
use crate::prompt::{Choice, Prompt};
use crate::screens::desk_model::Field as DeskField;
use crate::screens::git::Tab as GitTab;
use crate::screens::parts::Live;
use crate::tabs::SpaceId;
use crate::tree::ItemKey;

/// The answer of the Confirm of a restart.
const RESTART: &str = "restart";

fn desk_field(what: &Ask) -> Option<DeskField> {
    match what {
        Ask::DeskModel => Some(DeskField::Model),
        Ask::DeskEffort => Some(DeskField::Effort),
        _ => None,
    }
}

/// One open tab, for the palette.
pub struct OpenTab {
    pub space: SpaceId,
    pub panel: PanelId,
    /// The title of the tab, without its product.
    pub title: String,
    pub terminal: bool,
}

/// The choices of a switch: the local branches, the current one first and marked, then the
/// remote ones.
pub fn branch_choices(b: &GitBranchesResult) -> Vec<Choice> {
    b.local
        .iter()
        .map(|x| {
            let label = if x.current {
                format!("{} (current)", x.name)
            } else {
                x.name.clone()
            };
            Choice::new(label, x.name.clone())
        })
        .chain(
            b.remote
                .iter()
                .map(|x| Choice::new(x.name.clone(), x.name.clone())),
        )
        .collect()
}

impl Shell {
    /// The open tabs of each space, in the order of its bars.
    pub(crate) fn open_tabs(&self, cx: &App) -> Vec<OpenTab> {
        let open = self.open.borrow();
        let rows = self.tab_rows(cx);
        rows.into_iter()
            .flat_map(|(space, rows)| rows.into_iter().map(move |r| (space.clone(), r)))
            .filter_map(|(space, r)| {
                let (title, terminal) = match &r.key {
                    ItemKey::Pane(_) => {
                        let t = open
                            .terminals
                            .iter()
                            .filter_map(WeakEntity::upgrade)
                            .find(|t| PanelId::from(t.entity_id()) == r.panel)?;
                        let t = t.read(cx);
                        (crate::pane::shown(t.core().read(cx), &t.id, cx).title, true)
                    }
                    ItemKey::Screen(kind, arg) => (kind.title(arg.as_deref()), false),
                    _ => return None,
                };
                Some(OpenTab {
                    space,
                    panel: r.panel,
                    title,
                    terminal,
                })
            })
            .collect()
    }

    /// Opens a file of a product with the program of the system, as the file tree opens it, and
    /// keeps it in the recent targets. The path comes from the product itself, not from the tree.
    pub(crate) fn open_file(
        &mut self,
        product: ProductRef,
        path: String,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let abs = (product.host.is_none())
            .then(|| self.folder_of(&product, cx))
            .flatten()
            .map(|dir| std::path::Path::new(&dir).join(&path));
        let Some(abs) = abs else {
            self.notice("This product has no folder on this device.", cx);
            return;
        };
        self.opened(Opened::File(product, path), cx);
        cx.open_with_system(&abs);
    }

    /// The prompt of a second step that only the palette asks.
    pub(super) fn step_prompt(
        &mut self,
        what: &Ask,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Option<Entity<Prompt>> {
        match what {
            Ask::SwitchBranch(p) => {
                let core = self.cores.of(p.host.as_deref())?.clone();
                let name = self
                    .products(cx)
                    .into_iter()
                    .find(|(r, _)| r == p)
                    .map_or_else(|| p.id.clone(), |(_, n)| n);
                let prompt = cx.new(|cx| {
                    Prompt::pick(format!("Switch {name} to"), Vec::new(), 0, window, cx)
                        .with_hint("Reading the branches…")
                });
                let call = core.read(cx).link.call::<GitBranches>(&GitRepoParams {
                    product: p.id.clone(),
                    repo: None,
                });
                let weak = prompt.downgrade();
                cx.spawn(async move |_, cx| {
                    let result = call.await;
                    let _ = weak.update(cx, |prompt, cx| match result {
                        Ok(b) => prompt.set_choices(branch_choices(&b), 0, None, cx),
                        Err(e) => prompt.set_choices(
                            Vec::new(),
                            0,
                            Some(format!("The branches cannot be read: {e}").into()),
                            cx,
                        ),
                    });
                })
                .detach();
                Some(prompt)
            }
            Ask::DeskModel | Ask::DeskEffort => {
                let desk = Live::desk(cx)?;
                let field = desk_field(what)?;
                let (options, now) = desk.read(cx).conversation().read(cx).options(field);
                let at = options.iter().position(|o| o.value == now).unwrap_or(0);
                let choices = options
                    .into_iter()
                    .map(|o| Choice::new(o.label, o.value))
                    .collect();
                let title = match field {
                    DeskField::Model => "Model of the desk conversation",
                    DeskField::Effort => "Effort of the desk conversation",
                };
                Some(cx.new(|cx| {
                    Prompt::pick(title, choices, at, window, cx).with_hint(
                        "A reply that runs keeps its model. The next message uses the pick.",
                    )
                }))
            }
            Ask::RestartCore => Some(cx.new(|cx| {
                Prompt::pick(
                    "Restart the core of this device?",
                    vec![
                        Choice::new("Keep the core running", ""),
                        Choice::new("Restart the core and end each terminal", RESTART),
                    ],
                    0,
                    window,
                    cx,
                )
                .with_hint(
                    "The window opens again with a new core. The terminals of this device end.",
                )
            })),
            _ => None,
        }
    }

    pub(super) fn step_answer(
        &mut self,
        what: &Ask,
        answer: &str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> bool {
        match what {
            Ask::SwitchBranch(p) => {
                // The active product may have changed while the question was open.
                self.select_product(p, window, cx);
                let name = answer.to_string();
                self.on_git(window, cx, move |s, _, cx| {
                    s.set_tab(GitTab::Branches, cx);
                    s.switch_to(name.clone(), cx);
                });
            }
            Ask::DeskModel | Ask::DeskEffort => {
                if let (Some(desk), Some(field)) = (Live::desk(cx), desk_field(what)) {
                    let conversation = desk.read(cx).conversation().clone();
                    let value = answer.to_string();
                    conversation.update(cx, |c, cx| c.pick(field, value, cx));
                }
            }
            Ask::RestartCore => {
                if answer == RESTART {
                    self.restart_core(cx);
                }
            }
            _ => return false,
        }
        true
    }

    /// Asks for the model or the effort of the desk conversation on screen. With none open,
    /// a notice says so.
    pub(super) fn ask_desk_pick(&mut self, what: Ask, window: &mut Window, cx: &mut Context<Self>) {
        if Live::desk(cx).is_none() {
            self.notice("Open the front desk first.", cx);
            return;
        }
        self.ask(what, window, cx);
    }

    /// Asks for a branch of the active product.
    pub(super) fn ask_branch(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.ask_product(Ask::SwitchBranch, window, cx);
    }

    /// Opens the tab that closed last in its space: a screen again, or a new terminal with the
    /// program and the folder of the closed one.
    pub(super) fn reopen_closed_tab(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(tab) = Closed::pop(cx) else {
            self.notice("No tab closed since the app started.", cx);
            return;
        };
        match tab {
            ClosedTab::Screen(spec) => self.show_screen(spec, window, cx),
            ClosedTab::Terminal {
                host,
                product,
                argv,
                cwd,
            } => {
                let Some(core) = self.cores.of(host.as_deref()).cloned() else {
                    self.notice("The host of that tab is gone from this window.", cx);
                    return;
                };
                let argv = if argv.is_empty() {
                    shell_argv_for(host.is_some(), cx)
                } else {
                    argv
                };
                self.create_terminal(core, cwd, argv, product, None, window, cx);
            }
        }
    }

    /// Shows the space of the next product of the sidebar, or of the previous one. From Home it
    /// goes to the first product, or to the last one.
    pub(super) fn step_space(
        &mut self,
        forward: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let products = self.products(cx);
        if products.is_empty() {
            return;
        }
        let at = match &self.shown {
            SpaceId::Product(p) => products.iter().position(|(r, _)| r == p),
            SpaceId::Home => None,
        };
        let n = products.len();
        let next = match (at, forward) {
            (Some(i), true) => (i + 1) % n,
            (Some(i), false) => (i + n - 1) % n,
            (None, true) => 0,
            (None, false) => n - 1,
        };
        let product = products[next].0.clone();
        self.select_product(&product, window, cx);
    }

    /// Stops the core of this device and starts the app again, which starts a new core. The
    /// layout saves first, and the panes of the old terminals drop at the next start.
    fn restart_core(&mut self, cx: &mut Context<Self>) {
        let call = self.core.read(cx).link.call::<Shutdown>(&Empty {});
        self.before_quit(cx);
        cx.spawn(async move |_, cx| {
            let stopped = call.await;
            // The core removes its socket when it exits: a new app before that would attach
            // to the old core.
            if let Ok(socket) = agentsi_proto::socket::socket_path() {
                for _ in 0..100 {
                    if !socket.path.exists() {
                        break;
                    }
                    cx.background_executor()
                        .timer(std::time::Duration::from_millis(50))
                        .await;
                }
            }
            let started = std::env::current_exe().and_then(|exe| {
                std::process::Command::new(exe)
                    .args(std::env::args_os().skip(1))
                    .spawn()
            });
            if let Err(e) = stopped
                .as_ref()
                .map_err(ToString::to_string)
                .and(started.as_ref().map(|_| ()).map_err(ToString::to_string))
            {
                eprintln!("agentsi-app: the restart of the core failed: {e}");
            }
            cx.update(|cx| cx.quit());
        })
        .detach();
    }

    /// Reads each settings file of the app again: the keys, the appearance, the terminal, the
    /// alerts, the labels and colors, and the favorites.
    pub(super) fn reload_settings_files(&mut self, cx: &mut Context<Self>) {
        crate::keys::reload(cx);
        crate::appearance::reload(cx);
        crate::terminal_settings::reload(cx);
        crate::alerts_file::reload(cx);
        if let Some(look) = cx.try_global::<LookGlobal>().map(|g| g.0.clone()) {
            look.update(cx, |l, cx| l.reload(cx));
        }
        if let Some(list) = FavoritesGlobal::get(cx) {
            list.update(cx, |l, cx| l.reload(cx));
        }
        self.notice("Read the settings files again.", cx);
        cx.refresh_windows();
    }
}
