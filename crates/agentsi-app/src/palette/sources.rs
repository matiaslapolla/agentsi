//! The sources of the palette.

use gpui_kit::component::IconName;
use gpui_kit::*;

use super::{Kind, Opened, PaletteItem, PaletteSource, Run};
use crate::actions::{self, ENTRIES, OpenSettingsSection};
use crate::favorites::{FavoritesGlobal, Saved};
use crate::model::ProductRef;
use crate::screens::settings::Section;
use crate::shell::Shell;
use crate::tabs::SpaceId;

/// The source of `kind`.
pub fn source(kind: Kind) -> &'static dyn PaletteSource {
    match kind {
        Kind::Recent => &Recents,
        Kind::Actions => &Actions,
        Kind::Tabs => &Tabs,
        Kind::Settings => &Settings,
        Kind::Files => &Files,
        Kind::Favorites => &Favorites,
    }
}

/// The name of a product for a row, or its id when no core lists it.
fn name_of(products: &[(ProductRef, String)], p: &ProductRef) -> String {
    products
        .iter()
        .find(|(r, _)| r == p)
        .map_or_else(|| p.id.clone(), |(_, n)| n.clone())
}

/// The entries of `ENTRIES` in their order, then a "Go to" entry for each product.
struct Actions;

impl PaletteSource for Actions {
    fn items(&self, query: &str, shell: &Shell, cx: &App) -> Vec<PaletteItem> {
        let entries = ENTRIES.iter().map(|e| {
            PaletteItem::new(
                Kind::Actions,
                e.label,
                e.icon.clone(),
                Run::Action((e.action)()),
            )
            .keywords(e.keywords.iter().copied())
        });
        let products = shell
            .products(cx)
            .into_iter()
            .enumerate()
            .map(|(n, (p, name))| {
                let run = match actions::select_product(n) {
                    Some(a) => Run::Action(a),
                    None => Run::Product(p),
                };
                PaletteItem::new(
                    Kind::Actions,
                    format!("Go to {name}"),
                    IconName::ArrowRight,
                    run,
                )
                .keywords(["product", "select"])
            });
        entries
            .chain(products)
            .filter(|i| i.matches(query))
            .collect()
    }

    fn run(&self, run: Run, shell: &mut Shell, window: &mut Window, cx: &mut Context<Shell>) {
        if let Run::Product(p) = run {
            shell.select_product(&p, window, cx);
        }
    }
}

/// The open tabs of each space, with the name of the product of the space.
struct Tabs;

impl PaletteSource for Tabs {
    fn items(&self, query: &str, shell: &Shell, cx: &App) -> Vec<PaletteItem> {
        let products = shell.products(cx);
        shell
            .open_tabs(cx)
            .into_iter()
            .map(|t| {
                let space = match &t.space {
                    SpaceId::Home => "Home".to_string(),
                    SpaceId::Product(p) => name_of(&products, p),
                };
                let icon = if t.terminal {
                    IconName::SquareTerminal
                } else {
                    IconName::LayoutDashboard
                };
                PaletteItem::new(
                    Kind::Tabs,
                    format!("{} · {space}", t.title),
                    icon,
                    Run::Tab(t.panel),
                )
                .keywords(["tab".to_string(), t.title, space])
            })
            .filter(|i| i.matches(query))
            .collect()
    }

    fn run(&self, run: Run, shell: &mut Shell, window: &mut Window, cx: &mut Context<Shell>) {
        if let Run::Tab(panel) = run {
            shell.select_tab(panel, window, cx);
        }
    }
}

/// Each settings section that the app shows.
struct Settings;

impl PaletteSource for Settings {
    fn items(&self, query: &str, _: &Shell, cx: &App) -> Vec<PaletteItem> {
        Section::shown(cx)
            .into_iter()
            .map(|section| {
                PaletteItem::new(
                    Kind::Settings,
                    actions::settings_entry(section),
                    IconName::Settings,
                    Run::Action(Box::new(OpenSettingsSection { section })),
                )
                .keywords(["settings", "section", "preferences"])
            })
            .filter(|i| i.matches(query))
            .collect()
    }

    fn run(&self, _: Run, _: &mut Shell, _: &mut Window, _: &mut Context<Shell>) {}
}

/// The files of the active product that the core found for the query.
struct Files;

impl PaletteSource for Files {
    fn items(&self, query: &str, shell: &Shell, _: &App) -> Vec<PaletteItem> {
        let Some(found) = shell.palette.as_ref().map(|p| &p.files) else {
            return Vec::new();
        };
        let Some(product) = found.product.as_ref().filter(|_| found.query == query) else {
            return Vec::new();
        };
        found
            .paths
            .iter()
            .map(|path| {
                PaletteItem::new(
                    Kind::Files,
                    path.clone(),
                    IconName::File,
                    Run::File(product.clone(), path.clone()),
                )
            })
            .collect()
    }

    fn run(&self, run: Run, shell: &mut Shell, window: &mut Window, cx: &mut Context<Shell>) {
        if let Run::File(product, path) = run {
            shell.open_file(product, path, window, cx);
        }
    }
}

/// The favorites of the sidebar.
struct Favorites;

impl PaletteSource for Favorites {
    fn items(&self, query: &str, shell: &Shell, cx: &App) -> Vec<PaletteItem> {
        let Some(list) = FavoritesGlobal::get(cx) else {
            return Vec::new();
        };
        let products = shell.products(cx);
        list.read(cx)
            .list
            .iter()
            .enumerate()
            .map(|(at, f)| {
                let place = match &f.product {
                    Some(p) => name_of(&products, p),
                    None => "Home".into(),
                };
                PaletteItem::new(
                    Kind::Favorites,
                    format!("{} · {place}", f.label),
                    IconName::Star,
                    Run::Favorite(at),
                )
                .keywords(["favorite"])
            })
            .filter(|i| i.matches(query))
            .collect()
    }

    fn run(&self, run: Run, shell: &mut Shell, window: &mut Window, cx: &mut Context<Shell>) {
        if let Run::Favorite(at) = run {
            shell.open_favorite(at, window, cx);
        }
    }
}

/// The targets that the owner opened last, newest first.
struct Recents;

impl PaletteSource for Recents {
    fn items(&self, query: &str, shell: &Shell, cx: &App) -> Vec<PaletteItem> {
        // A target of a product that no core lists now stays in the list for a host that comes
        // back, and does not show.
        let products = shell.products(cx);
        shell
            .recent
            .0
            .iter()
            .filter(|t| {
                t.product()
                    .is_none_or(|p| products.iter().any(|(r, _)| r == p))
            })
            .map(|t| {
                let place = match t.product() {
                    Some(p) => name_of(&products, p),
                    None => "Home".into(),
                };
                let icon = match t {
                    Opened::File(..) => IconName::File,
                    Opened::Saved(f) => match f.saved {
                        Saved::Screen(..) => IconName::LayoutDashboard,
                        Saved::Folder(_) => IconName::Folder,
                        Saved::Command { .. } => IconName::SquareTerminal,
                    },
                };
                PaletteItem::new(
                    Kind::Recent,
                    format!("{} · {place}", t.label()),
                    icon,
                    Run::Recent(t.clone()),
                )
                .keywords(["recent"])
            })
            .filter(|i| i.matches(query))
            .collect()
    }

    fn run(&self, run: Run, shell: &mut Shell, window: &mut Window, cx: &mut Context<Shell>) {
        if let Run::Recent(t) = run {
            shell.open_recent(t, window, cx);
        }
    }
}
