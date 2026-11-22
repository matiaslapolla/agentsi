//! The recent targets of the palette, saved in `layout.json`, and the tabs that closed in this
//! run, for "Reopen closed tab".

use gpui_kit::{App, Global};

use crate::favorites::{self, Favorite, Saved};
use crate::layout::RecentSave;
use crate::look;
use crate::model::ProductRef;
use crate::screens::{ScreenKind, ScreenSpec};

/// The most recent targets that the palette keeps.
pub const RECENT_MAX: usize = 20;
/// The most closed tabs that "Reopen closed tab" keeps.
pub const CLOSED_MAX: usize = 20;

/// A target that the owner opened.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Opened {
    /// A screen, a folder or a command, as a favorite keeps it.
    Saved(Favorite),
    /// A file of a product, by its path from the folder of the product.
    File(ProductRef, String),
}

impl Opened {
    /// The recent target of a screen. The guide and a setup are no target to go back to.
    pub fn screen(spec: &ScreenSpec) -> Option<Opened> {
        if matches!(spec.kind, ScreenKind::Guide | ScreenKind::Setup) {
            return None;
        }
        let product = (!spec.kind.of_no_product()).then(|| ProductRef {
            host: spec.host.clone(),
            id: spec.product.clone(),
        });
        let saved = Saved::Screen(spec.kind, spec.arg.clone());
        Some(Opened::Saved(Favorite {
            label: favorites::default_label(&saved),
            product,
            saved,
        }))
    }

    pub fn product(&self) -> Option<&ProductRef> {
        match self {
            Opened::Saved(f) => f.product.as_ref(),
            Opened::File(p, _) => Some(p),
        }
    }

    /// The text of the palette row, before the product name.
    pub fn label(&self) -> String {
        match self {
            Opened::Saved(f) => f.label.clone(),
            Opened::File(_, path) => path.clone(),
        }
    }

    fn save(&self) -> RecentSave {
        let product = self.product().map(|p| look::key(p.host.as_deref(), &p.id));
        match self {
            Opened::Saved(f) => {
                let (kind, target, folder) = f.fields();
                RecentSave {
                    label: f.label.clone(),
                    product,
                    kind: kind.into(),
                    target,
                    folder,
                }
            }
            Opened::File(_, path) => RecentSave {
                label: path.rsplit('/').next().unwrap_or(path).to_string(),
                product,
                kind: "file".into(),
                target: path.clone(),
                folder: None,
            },
        }
    }

    fn load(s: &RecentSave) -> Option<Opened> {
        if s.kind == "file" {
            let product = look::product_of_key(s.product.as_deref()?);
            return Some(Opened::File(product, s.target.clone()));
        }
        favorites::from_fields(|k| match k {
            "label" => Some(s.label.as_str()),
            "product" => s.product.as_deref(),
            "kind" => Some(s.kind.as_str()),
            "target" => Some(s.target.as_str()),
            "folder" => s.folder.as_deref(),
            _ => None,
        })
        .ok()
        .map(Opened::Saved)
    }

    /// The same target: a screen or a file opened again replaces its older row.
    fn same(&self, other: &Opened) -> bool {
        match (self, other) {
            (Opened::Saved(a), Opened::Saved(b)) => a.product == b.product && a.saved == b.saved,
            (a, b) => a == b,
        }
    }
}

/// The recent targets, newest first.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct Recent(pub Vec<Opened>);

impl Recent {
    /// Puts `target` first. An older row of the same target goes, and the list keeps 20.
    pub fn push(&mut self, target: Opened) -> bool {
        if self.0.first().is_some_and(|t| t.same(&target)) {
            return false;
        }
        self.0.retain(|t| !t.same(&target));
        self.0.insert(0, target);
        self.0.truncate(RECENT_MAX);
        true
    }

    pub fn save(&self) -> Vec<RecentSave> {
        self.0.iter().map(Opened::save).collect()
    }

    /// The list of `layout.json`. A row that does not read is skipped.
    pub fn load(saved: &[RecentSave]) -> Recent {
        let mut r = Recent::default();
        for t in saved.iter().rev().filter_map(Opened::load) {
            r.push(t);
        }
        r
    }
}

/// A tab that closed: what opens it again in its space.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ClosedTab {
    Screen(ScreenSpec),
    /// A terminal tab kills its terminal, so it opens again as a new terminal with the same
    /// program in the same folder.
    Terminal {
        host: Option<String>,
        product: Option<String>,
        argv: Vec<String>,
        cwd: String,
    },
}

/// The tabs that the owner closed in this run, newest last.
#[derive(Default)]
pub struct Closed(pub Vec<ClosedTab>);

impl Global for Closed {}

impl Closed {
    pub fn push(cx: &mut App, tab: ClosedTab) {
        let list = &mut cx.default_global::<Closed>().0;
        list.push(tab);
        if list.len() > CLOSED_MAX {
            list.remove(0);
        }
    }

    pub fn pop(cx: &mut App) -> Option<ClosedTab> {
        cx.default_global::<Closed>().0.pop()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn product(id: &str) -> ProductRef {
        ProductRef {
            host: None,
            id: id.into(),
        }
    }

    fn file(path: &str) -> Opened {
        Opened::File(product("a"), path.into())
    }

    #[test]
    fn a_target_opened_again_moves_first_and_the_list_keeps_the_newest_20() {
        let mut r = Recent::default();
        for n in 0..25 {
            r.push(file(&format!("f{n}")));
        }
        assert_eq!(r.0.len(), RECENT_MAX);
        assert_eq!(r.0[0], file("f24"));
        assert_eq!(r.0[19], file("f5"));
        assert!(r.push(file("f10")));
        assert_eq!(r.0[0], file("f10"));
        assert_eq!(r.0.iter().filter(|t| **t == file("f10")).count(), 1);
        assert_eq!(r.0.len(), RECENT_MAX);
        assert!(!r.push(file("f10")), "the first row again is no change");
    }

    #[test]
    fn a_screen_of_a_new_label_is_the_same_target_and_the_guide_is_none() {
        let mut r = Recent::default();
        let desk = Opened::screen(&ScreenSpec::desk(&product("a"))).unwrap();
        r.push(desk.clone());
        r.push(file("x"));
        let Opened::Saved(mut renamed) = desk.clone() else {
            unreachable!()
        };
        renamed.label = "Desk".into();
        r.push(Opened::Saved(renamed));
        assert_eq!(r.0.len(), 2);
        assert_eq!(Opened::screen(&ScreenSpec::guide()), None);
    }

    #[test]
    fn the_list_round_trips_through_the_layout_rows_and_a_bad_row_is_skipped() {
        let mut r = Recent::default();
        r.push(Opened::screen(&ScreenSpec::git("a").on(Some("studio".into()))).unwrap());
        r.push(Opened::Saved(Favorite {
            label: "Build".into(),
            product: Some(product("a")),
            saved: Saved::Command {
                argv: vec!["cargo".into(), "build".into()],
                folder: "/w/a".into(),
            },
        }));
        r.push(file("src/main.rs"));
        let mut saved = r.save();
        assert_eq!(saved[0].kind, "file");
        assert_eq!(saved[0].label, "main.rs");
        assert_eq!(saved[2].product.as_deref(), Some("@studio/a"));
        saved.push(RecentSave {
            label: "x".into(),
            product: None,
            kind: "nope".into(),
            target: "x".into(),
            folder: None,
        });
        assert_eq!(Recent::load(&saved), r);
    }
}
