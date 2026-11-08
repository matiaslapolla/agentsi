//! `<config>/agentsi/look.toml`: the labels, the colors and the product groups.
//!
//! ```toml
//! [products.demo]          # a product of this device, by product id
//! label = "api"
//! color = "teal"
//!
//! [products."@studio/demo"] # a product of the SSH host "studio"
//! color = "violet"
//!
//! [panes.t1-a]             # a terminal, by terminal id
//! label = "build"
//!
//! [[groups]]               # a product group, in the order of the sidebar
//! host = "studio"          # absent for this device
//! name = "Work"
//! color = "sky"
//! collapsed = false
//! products = ["demo", "other"]
//! ```
//!
//! - A label is free text of 40 characters or less. A color is one of `tokens::label::Hue`.
//! - The app reads what it knows. An entry or a key that it does not know stays in the file:
//!   each write edits the file in place. A file that does not parse is not overwritten.
//! - At start, the app drops the pane entries of terminals that the core no longer knows
//!   ([`Look::prune_panes`]), as `layout.rs` prunes panels.
//! - The state of the open and closed products is in `layout.json`, not here.

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

use agentsi_proto::desk_api::ProductInfo;
use agentsi_ui::tokens::label::Hue;
use gpui_kit::{App, AppContext as _, Context, Entity, Global};
use toml_edit::{Array, ArrayOfTables, DocumentMut, Item, Table, value};

pub const FILE_NAME: &str = "look.toml";
/// The longest label, in characters.
pub const LABEL_MAX: usize = 40;

/// `<config>/agentsi/look.toml`.
pub fn path() -> Option<PathBuf> {
    crate::layout::path().map(|p| p.with_file_name(FILE_NAME))
}

/// The key of a product or a terminal in the file: the id on this device, `@host/id` on a host.
pub fn key(host: Option<&str>, id: &str) -> String {
    match host {
        None => id.to_string(),
        Some(h) => format!("@{h}/{id}"),
    }
}

/// The host of a key made by [`key`].
fn host_of_key(key: &str) -> Option<&str> {
    key.strip_prefix('@')?.split_once('/').map(|(h, _)| h)
}

/// A label and a color. Both are optional.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Style {
    pub label: Option<String>,
    pub hue: Option<Hue>,
}

impl Style {
    fn is_empty(&self) -> bool {
        self.label.is_none() && self.hue.is_none()
    }
}

/// Trims a label and cuts it to [`LABEL_MAX`] characters. An empty label is none.
pub fn clean_label(text: &str) -> Option<String> {
    let text: String = text.trim().chars().take(LABEL_MAX).collect();
    let text = text.trim_end().to_string();
    (!text.is_empty()).then_some(text)
}

/// A named, ordered set of products of one host.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Group {
    /// The SSH host, or `None` for this device.
    pub host: Option<String>,
    pub name: String,
    pub hue: Option<Hue>,
    pub collapsed: bool,
    /// Product ids, in order.
    pub products: Vec<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct LookData {
    pub products: BTreeMap<String, Style>,
    pub panes: BTreeMap<String, Style>,
    pub groups: Vec<Group>,
}

/// One group with its products, for the sidebar.
#[derive(Debug, Clone, PartialEq)]
pub struct GroupView {
    pub name: String,
    pub hue: Option<Hue>,
    pub collapsed: bool,
    /// Indexes into the product list, in the order of the group.
    pub members: Vec<usize>,
}

/// The products of one host as the sidebar shows them: those with no group first, then each
/// group.
#[derive(Debug, Clone, PartialEq)]
pub struct Arranged {
    pub top: Vec<usize>,
    pub groups: Vec<GroupView>,
}

impl Arranged {
    /// Each product index in the order of the sidebar, also those in a collapsed group.
    pub fn flat(&self) -> Vec<usize> {
        let mut out = self.top.clone();
        for g in &self.groups {
            out.extend(&g.members);
        }
        out
    }
}

impl LookData {
    pub fn product(&self, host: Option<&str>, id: &str) -> Style {
        self.products
            .get(&key(host, id))
            .cloned()
            .unwrap_or_default()
    }

    pub fn pane(&self, host: Option<&str>, id: &str) -> Style {
        self.panes.get(&key(host, id)).cloned().unwrap_or_default()
    }

    /// The name of the group of a product.
    pub fn group_of(&self, host: Option<&str>, id: &str) -> Option<&str> {
        self.groups
            .iter()
            .find(|g| g.host.as_deref() == host && g.products.iter().any(|p| p == id))
            .map(|g| g.name.as_str())
    }

    /// The names of the groups of one host, in order.
    pub fn group_names(&self, host: Option<&str>) -> Vec<&str> {
        self.groups
            .iter()
            .filter(|g| g.host.as_deref() == host)
            .map(|g| g.name.as_str())
            .collect()
    }

    /// The products of `host` in the order of the sidebar. A product that two groups name
    /// stays in the first. A group names products that the host may not list: they are skipped.
    pub fn arrange(&self, host: Option<&str>, products: &[ProductInfo]) -> Arranged {
        let mut seen = BTreeSet::new();
        let mut groups = Vec::new();
        for g in self.groups.iter().filter(|g| g.host.as_deref() == host) {
            let members: Vec<usize> = g
                .products
                .iter()
                .filter_map(|id| products.iter().position(|p| &p.id == id))
                .filter(|i| seen.insert(*i))
                .collect();
            groups.push(GroupView {
                name: g.name.clone(),
                hue: g.hue,
                collapsed: g.collapsed,
                members,
            });
        }
        let top = (0..products.len()).filter(|i| !seen.contains(i)).collect();
        Arranged { top, groups }
    }
}

/// The file and the data, as an entity that the views observe.
pub struct Look {
    path: Option<PathBuf>,
    pub data: LookData,
    /// One line about the file when something in it cannot be used or saved.
    pub note: Option<String>,
}

/// The `Look` of the window, for the views that draw a label or a color.
#[derive(Clone)]
pub struct LookGlobal(pub Entity<Look>);

impl Global for LookGlobal {}

impl LookGlobal {
    /// Reads the data of the window. It gives none while no look is installed: some benches
    /// and tests have none.
    pub fn with<R>(cx: &App, f: impl FnOnce(&LookData) -> R) -> Option<R> {
        cx.try_global::<LookGlobal>().map(|g| f(&g.0.read(cx).data))
    }
}

/// Reads the file at `path` and makes it the look of the window. Call it once, at start.
pub fn install(cx: &mut App, path: Option<PathBuf>) -> Entity<Look> {
    let look = cx.new(|_| Look::open(path));
    cx.set_global(LookGlobal(look.clone()));
    look
}

impl Look {
    pub fn open(path: Option<PathBuf>) -> Look {
        let (data, note) = match &path {
            Some(p) => read(p),
            None => (LookData::default(), None),
        };
        Look { path, data, note }
    }

    /// Writes the data. A failed save keeps the change in memory and says why in `note`.
    fn save(&mut self) {
        let Some(path) = self.path.clone() else {
            return;
        };
        self.note = write(&path, &self.data).err();
    }

    fn changed(&mut self, cx: &mut Context<Self>) {
        self.save();
        cx.notify();
    }

    pub fn set_product_label(&mut self, key: &str, label: &str, cx: &mut Context<Self>) {
        let label = clean_label(label);
        style_of(&mut self.data.products, key, |s| s.label = label);
        self.changed(cx);
    }

    pub fn set_product_hue(&mut self, key: &str, hue: Option<Hue>, cx: &mut Context<Self>) {
        style_of(&mut self.data.products, key, |s| s.hue = hue);
        self.changed(cx);
    }

    pub fn set_pane_label(&mut self, key: &str, label: &str, cx: &mut Context<Self>) {
        let label = clean_label(label);
        style_of(&mut self.data.panes, key, |s| s.label = label);
        self.changed(cx);
    }

    pub fn set_pane_hue(&mut self, key: &str, hue: Option<Hue>, cx: &mut Context<Self>) {
        style_of(&mut self.data.panes, key, |s| s.hue = hue);
        self.changed(cx);
    }

    /// Puts a product into the group `name` of its host, which this call makes when it is new.
    /// An empty name takes the product out of its group.
    pub fn move_product(
        &mut self,
        host: Option<&str>,
        id: &str,
        name: &str,
        cx: &mut Context<Self>,
    ) {
        for g in self
            .data
            .groups
            .iter_mut()
            .filter(|g| g.host.as_deref() == host)
        {
            g.products.retain(|p| p != id);
        }
        if let Some(name) = clean_label(name) {
            let at = self
                .data
                .groups
                .iter()
                .position(|g| g.host.as_deref() == host && g.name == name);
            let at = at.unwrap_or_else(|| {
                self.data.groups.push(Group {
                    host: host.map(String::from),
                    name,
                    hue: None,
                    collapsed: false,
                    products: Vec::new(),
                });
                self.data.groups.len() - 1
            });
            self.data.groups[at].products.push(id.to_string());
        }
        self.changed(cx);
    }

    fn group_mut(&mut self, host: Option<&str>, name: &str) -> Option<&mut Group> {
        self.data
            .groups
            .iter_mut()
            .find(|g| g.host.as_deref() == host && g.name == name)
    }

    pub fn toggle_group(&mut self, host: Option<&str>, name: &str, cx: &mut Context<Self>) {
        if let Some(g) = self.group_mut(host, name) {
            g.collapsed = !g.collapsed;
            self.changed(cx);
        }
    }

    pub fn set_group_hue(
        &mut self,
        host: Option<&str>,
        name: &str,
        hue: Option<Hue>,
        cx: &mut Context<Self>,
    ) {
        if let Some(g) = self.group_mut(host, name) {
            g.hue = hue;
            self.changed(cx);
        }
    }

    /// Renames a group. Returns false when the new name is empty or another group of the host
    /// has it.
    pub fn rename_group(
        &mut self,
        host: Option<&str>,
        from: &str,
        to: &str,
        cx: &mut Context<Self>,
    ) -> bool {
        let Some(to) = clean_label(to) else {
            return false;
        };
        if to != from && self.data.group_names(host).contains(&to.as_str()) {
            return false;
        }
        match self.group_mut(host, from) {
            Some(g) => g.name = to,
            None => return false,
        }
        self.changed(cx);
        true
    }

    /// Removes a group and keeps its products.
    pub fn remove_group(&mut self, host: Option<&str>, name: &str, cx: &mut Context<Self>) {
        self.data
            .groups
            .retain(|g| !(g.host.as_deref() == host && g.name == name));
        self.changed(cx);
    }

    /// Moves a group to place `to` among the groups of its host.
    #[cfg_attr(not(test), allow(dead_code))] // no menu entry moves a group yet
    pub fn move_group(
        &mut self,
        host: Option<&str>,
        name: &str,
        to: usize,
        cx: &mut Context<Self>,
    ) {
        let of_host: Vec<usize> = (0..self.data.groups.len())
            .filter(|i| self.data.groups[*i].host.as_deref() == host)
            .collect();
        let Some(from) = of_host
            .iter()
            .position(|i| self.data.groups[*i].name == name)
        else {
            return;
        };
        let to = to.min(of_host.len() - 1);
        if from == to {
            return;
        }
        // The groups of one host take the places of that host in the list.
        let mut moved: Vec<Group> = of_host
            .iter()
            .map(|i| self.data.groups[*i].clone())
            .collect();
        let g = moved.remove(from);
        moved.insert(to, g);
        for (slot, g) in of_host.into_iter().zip(moved) {
            self.data.groups[slot] = g;
        }
        self.changed(cx);
    }

    /// Drops the pane entries of terminals that the core no longer knows. `local` is the set of
    /// terminals of this device. `hosts` is the set of hosts of `hosts.toml`, or `None` when that
    /// file did not parse: then no host entry goes. An entry of a host that is off stays.
    /// Returns the number of entries dropped.
    pub fn prune_panes(
        &mut self,
        local: &BTreeSet<String>,
        hosts: Option<&BTreeSet<String>>,
        cx: &mut Context<Self>,
    ) -> usize {
        let before = self.data.panes.len();
        self.data.panes.retain(|k, _| match host_of_key(k) {
            None => local.contains(k),
            Some(h) => hosts.is_none_or(|hosts| hosts.contains(h)),
        });
        let dropped = before - self.data.panes.len();
        if dropped > 0 {
            self.changed(cx);
        }
        dropped
    }
}

fn style_of(map: &mut BTreeMap<String, Style>, key: &str, f: impl FnOnce(&mut Style)) {
    let mut style = map.get(key).cloned().unwrap_or_default();
    f(&mut style);
    if style.is_empty() {
        map.remove(key);
    } else {
        map.insert(key.to_string(), style);
    }
}

/// The file in `path`, and one line for the owner when something in it cannot be used.
pub fn read(path: &Path) -> (LookData, Option<String>) {
    match std::fs::read_to_string(path) {
        Ok(text) => parse(&text, path),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => (LookData::default(), None),
        Err(e) => (
            LookData::default(),
            Some(format!("{} cannot be read: {e}.", path.display())),
        ),
    }
}

fn parse(text: &str, path: &Path) -> (LookData, Option<String>) {
    let doc: DocumentMut = match text.parse() {
        Ok(d) => d,
        Err(e) => {
            let e: toml_edit::TomlError = e;
            return (
                LookData::default(),
                Some(format!(
                    "{} does not parse: {}. Labels, colors and groups stay off until you fix it.",
                    path.display(),
                    e.message()
                )),
            );
        }
    };
    let mut problems = Vec::new();
    let mut data = LookData {
        products: styles(doc.get("products"), "products", &mut problems),
        panes: styles(doc.get("panes"), "panes", &mut problems),
        ..LookData::default()
    };
    if let Some(groups) = doc.get("groups").and_then(Item::as_array_of_tables) {
        for t in groups.iter() {
            let Some(name) = t.get("name").and_then(Item::as_str).and_then(clean_label) else {
                problems.push("a group has no name.".into());
                continue;
            };
            data.groups.push(Group {
                host: t.get("host").and_then(Item::as_str).map(String::from),
                hue: hue_of(t, &format!("group {name}"), &mut problems),
                collapsed: t.get("collapsed").and_then(Item::as_bool).unwrap_or(false),
                products: t
                    .get("products")
                    .and_then(Item::as_array)
                    .map(|a| {
                        a.iter()
                            .filter_map(|v| v.as_str().map(String::from))
                            .collect()
                    })
                    .unwrap_or_default(),
                name,
            });
        }
    }
    let note = (!problems.is_empty()).then(|| {
        format!(
            "{}: {} The app skips each of these.",
            path.display(),
            problems.join(" ")
        )
    });
    (data, note)
}

fn hue_of(t: &Table, what: &str, problems: &mut Vec<String>) -> Option<Hue> {
    let word = t.get("color")?.as_str();
    match word.map(|w| (w, Hue::from_key(w))) {
        Some((_, Some(h))) => Some(h),
        Some((w, None)) => {
            problems.push(format!("{what}: color \"{w}\" is not one of the 8 colors."));
            None
        }
        None => {
            problems.push(format!("{what}: color must be text."));
            None
        }
    }
}

fn styles(item: Option<&Item>, table: &str, problems: &mut Vec<String>) -> BTreeMap<String, Style> {
    let mut out = BTreeMap::new();
    let Some(t) = item.and_then(Item::as_table_like) else {
        return out;
    };
    for (k, v) in t.iter() {
        let Some(entry) = v.as_table_like() else {
            continue;
        };
        let hue = entry.get("color").and_then(|c| match c.as_str() {
            Some(w) => match Hue::from_key(w) {
                Some(h) => Some(h),
                None => {
                    problems.push(format!(
                        "{table}.{k}: color \"{w}\" is not one of the 8 colors."
                    ));
                    None
                }
            },
            None => None,
        });
        let style = Style {
            label: entry
                .get("label")
                .and_then(Item::as_str)
                .and_then(clean_label),
            hue,
        };
        if !style.is_empty() {
            out.insert(k.to_string(), style);
        }
    }
    out
}

/// Writes `data` into the file in place: the known keys change, the rest stays.
fn write(path: &Path, data: &LookData) -> Result<(), String> {
    crate::keys::edit(path, |doc| {
        sync_styles(doc, "products", &data.products)?;
        sync_styles(doc, "panes", &data.panes)?;
        sync_groups(doc, &data.groups)
    })
}

fn sync_styles(
    doc: &mut DocumentMut,
    name: &str,
    styles: &BTreeMap<String, Style>,
) -> Result<(), String> {
    if styles.is_empty() && !doc.contains_key(name) {
        return Ok(());
    }
    let root = doc
        .entry(name)
        .or_insert_with(|| Item::Table(implicit()))
        .as_table_like_mut()
        .ok_or_else(|| format!("`{name}` in look.toml is not a table. Fix it by hand first."))?;
    // An entry that the data no longer holds loses its two keys. It goes when nothing else is in it.
    let stale: Vec<String> = root
        .iter()
        .filter(|(k, _)| !styles.contains_key(*k))
        .map(|(k, _)| k.to_string())
        .collect();
    for k in stale {
        let empty = match root.get_mut(&k).and_then(Item::as_table_like_mut) {
            Some(t) => {
                t.remove("label");
                t.remove("color");
                t.is_empty()
            }
            None => false,
        };
        if empty {
            root.remove(&k);
        }
    }
    for (k, s) in styles {
        if !root.contains_key(k) {
            root.insert(k, Item::Table(Table::new()));
        }
        let Some(t) = root.get_mut(k).and_then(Item::as_table_like_mut) else {
            continue;
        };
        match &s.label {
            Some(l) => {
                t.insert("label", value(l.as_str()));
            }
            None => {
                t.remove("label");
            }
        }
        match s.hue {
            Some(h) => {
                t.insert("color", value(h.key()));
            }
            None => {
                t.remove("color");
            }
        }
    }
    if root.is_empty() {
        doc.remove(name);
    }
    Ok(())
}

/// A table with no header of its own, so `[products.demo]` stays the form of the file.
fn implicit() -> Table {
    let mut t = Table::new();
    t.set_implicit(true);
    t
}

fn sync_groups(doc: &mut DocumentMut, groups: &[Group]) -> Result<(), String> {
    let old: Vec<Table> = match doc.get("groups") {
        None => Vec::new(),
        Some(i) => i
            .as_array_of_tables()
            .ok_or("`groups` in look.toml is not a list of tables. Fix it by hand first.")?
            .iter()
            .cloned()
            .collect(),
    };
    if groups.is_empty() && old.is_empty() {
        return Ok(());
    }
    let mut out = ArrayOfTables::new();
    for g in groups {
        // A group that the file has already keeps its unknown keys.
        let mut t = old
            .iter()
            .find(|t| {
                t.get("name").and_then(Item::as_str) == Some(g.name.as_str())
                    && t.get("host").and_then(Item::as_str) == g.host.as_deref()
            })
            .cloned()
            .unwrap_or_default();
        match &g.host {
            Some(h) => {
                t.insert("host", value(h.as_str()));
            }
            None => {
                t.remove("host");
            }
        }
        t.insert("name", value(g.name.as_str()));
        match g.hue {
            Some(h) => {
                t.insert("color", value(h.key()));
            }
            None => {
                t.remove("color");
            }
        }
        t.insert("collapsed", value(g.collapsed));
        let mut ids = Array::new();
        for p in &g.products {
            ids.push(p.as_str());
        }
        t.insert("products", value(ids));
        out.push(t);
    }
    if out.is_empty() {
        doc.remove("groups");
    } else {
        doc.insert("groups", Item::ArrayOfTables(out));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use gpui::TestAppContext;

    fn tempdir(name: &str) -> PathBuf {
        static N: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
        let n = N.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let dir =
            std::env::temp_dir().join(format!("agentsi-look-{name}-{}-{n}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn product(id: &str) -> ProductInfo {
        ProductInfo {
            id: id.into(),
            name: id.into(),
            path: None,
            archived: false,
        }
    }

    fn open(file: &Path, cx: &mut TestAppContext) -> Entity<Look> {
        let file = file.to_path_buf();
        cx.new(move |_| Look::open(Some(file)))
    }

    #[test]
    fn a_label_is_trimmed_cut_to_40_characters_and_empty_means_none() {
        assert_eq!(clean_label("  api  ").as_deref(), Some("api"));
        assert_eq!(clean_label("   "), None);
        let long = "x".repeat(60);
        assert_eq!(clean_label(&long).unwrap().chars().count(), LABEL_MAX);
        assert_eq!(
            clean_label("é".repeat(50).as_str())
                .unwrap()
                .chars()
                .count(),
            40
        );
    }

    #[test]
    fn a_file_is_read_into_labels_colors_and_groups() {
        let text = r#"
            [products.demo]
            label = "api"
            color = "teal"

            [products."@studio/demo"]
            color = "violet"

            [panes.t1-a]
            label = "build"

            [[groups]]
            name = "Work"
            color = "sky"
            collapsed = true
            products = ["demo", "other"]

            [[groups]]
            host = "studio"
            name = "Remote"
            products = ["demo"]
        "#;
        let (data, note) = parse(text, Path::new("look.toml"));
        assert_eq!(note, None);
        assert_eq!(
            data.product(None, "demo"),
            Style {
                label: Some("api".into()),
                hue: Some(Hue::Teal)
            }
        );
        assert_eq!(data.product(Some("studio"), "demo").hue, Some(Hue::Violet));
        assert_eq!(data.pane(None, "t1-a").label.as_deref(), Some("build"));
        assert_eq!(data.groups.len(), 2);
        assert!(data.groups[0].collapsed);
        assert_eq!(data.groups[1].host.as_deref(), Some("studio"));
        assert_eq!(data.group_of(None, "other"), Some("Work"));
    }

    #[test]
    fn a_bad_color_is_skipped_with_one_note_and_the_rest_stays() {
        let (data, note) = parse(
            "[products.a]\nlabel = \"x\"\ncolor = \"mauve\"\n[products.b]\ncolor = \"rose\"\n",
            Path::new("look.toml"),
        );
        assert_eq!(data.product(None, "a").label.as_deref(), Some("x"));
        assert_eq!(data.product(None, "a").hue, None);
        assert_eq!(data.product(None, "b").hue, Some(Hue::Rose));
        let note = note.unwrap();
        assert!(note.contains("mauve"), "{note}");
        assert_eq!(note.lines().count(), 1);
    }

    #[test]
    fn the_sidebar_order_puts_the_products_with_no_group_first() {
        let mut data = LookData::default();
        data.groups.push(Group {
            host: None,
            name: "Work".into(),
            hue: Some(Hue::Sky),
            collapsed: false,
            products: vec!["c".into(), "gone".into(), "a".into()],
        });
        data.groups.push(Group {
            host: None,
            name: "Dup".into(),
            hue: None,
            collapsed: false,
            products: vec!["a".into()],
        });
        let ps = [product("a"), product("b"), product("c"), product("d")];
        let arranged = data.arrange(None, &ps);
        assert_eq!(arranged.top, [1, 3]);
        assert_eq!(arranged.groups[0].members, [2, 0]);
        assert!(
            arranged.groups[1].members.is_empty(),
            "a product stays in its first group"
        );
        assert_eq!(arranged.flat(), [1, 3, 2, 0]);
        // A group of the host "studio" does not touch the products of this device.
        assert_eq!(data.arrange(Some("studio"), &ps).top, [0, 1, 2, 3]);
    }

    #[gpui::test]
    fn each_change_is_saved_and_reads_back_with_the_unknown_entries_kept(cx: &mut TestAppContext) {
        let file = tempdir("save").join("look.toml");
        std::fs::write(
            &file,
            "# my notes\ntheme = \"night\"\n\n[products.demo]\nmood = \"calm\"\n\n[future]\nx = 1\n",
        )
        .unwrap();
        let look = open(&file, cx);
        look.update(cx, |l, cx| {
            l.set_product_label("demo", "api", cx);
            l.set_product_hue("demo", Some(Hue::Teal), cx);
            l.set_pane_label("t1-a", "build", cx);
            l.set_pane_hue("t1-a", Some(Hue::Pink), cx);
            l.move_product(None, "demo", "Work", cx);
            l.toggle_group(None, "Work", cx);
        });
        let text = std::fs::read_to_string(&file).unwrap();
        for kept in [
            "# my notes",
            "theme = \"night\"",
            "mood = \"calm\"",
            "[future]",
            "x = 1",
        ] {
            assert!(text.contains(kept), "{kept} is lost:\n{text}");
        }
        let (data, note) = read(&file);
        assert_eq!(note, None);
        assert_eq!(data, look.read_with(cx, |l, _| l.data.clone()));
        assert_eq!(data.product(None, "demo").label.as_deref(), Some("api"));
        assert_eq!(data.pane(None, "t1-a").hue, Some(Hue::Pink));
        assert!(data.groups[0].collapsed);
        // Clearing a label and a color drops the entry but keeps the unknown key beside them.
        look.update(cx, |l, cx| {
            l.set_product_label("demo", "", cx);
            l.set_product_hue("demo", None, cx);
        });
        let text = std::fs::read_to_string(&file).unwrap();
        assert!(text.contains("mood = \"calm\""), "{text}");
        assert!(!text.contains("label = \"api\""), "{text}");
    }

    #[gpui::test]
    fn a_gone_terminal_loses_its_entry_at_start_and_an_unknown_entry_stays(
        cx: &mut TestAppContext,
    ) {
        let file = tempdir("prune").join("look.toml");
        std::fs::write(
            &file,
            "mystery = [1, 2]\n\n[panes.t1-old]\nlabel = \"gone\"\n\n[panes.t2-run]\nlabel = \"kept\"\n\n[panes.\"@studio/t3-x\"]\ncolor = \"rose\"\n\n[panes.\"@laptop/t4-x\"]\ncolor = \"rose\"\n",
        )
        .unwrap();
        let look = open(&file, cx);
        let local = BTreeSet::from(["t2-run".to_string()]);
        let hosts = BTreeSet::from(["studio".to_string()]);
        let dropped = look.update(cx, |l, cx| l.prune_panes(&local, Some(&hosts), cx));
        assert_eq!(dropped, 2);
        let text = std::fs::read_to_string(&file).unwrap();
        assert!(text.contains("mystery = [1, 2]"), "{text}");
        assert!(
            !text.contains("t1-old") && !text.contains("laptop"),
            "{text}"
        );
        let (data, _) = read(&file);
        assert_eq!(data.pane(None, "t2-run").label.as_deref(), Some("kept"));
        assert_eq!(data.pane(Some("studio"), "t3-x").hue, Some(Hue::Rose));
        // With hosts.toml unread, no host entry goes.
        let again = look.update(cx, |l, cx| {
            l.data.panes.insert(
                key(Some("far"), "t9-x"),
                Style {
                    label: Some("x".into()),
                    hue: None,
                },
            );
            l.prune_panes(&local, None, cx)
        });
        assert_eq!(again, 0);
    }

    #[gpui::test]
    fn a_file_that_does_not_parse_is_not_overwritten(cx: &mut TestAppContext) {
        let file = tempdir("broken").join("look.toml");
        std::fs::write(&file, "[products\nlabel = =").unwrap();
        let look = open(&file, cx);
        assert!(
            look.read_with(cx, |l, _| l.note.clone())
                .unwrap()
                .contains("does not parse")
        );
        look.update(cx, |l, cx| l.set_product_label("demo", "api", cx));
        assert_eq!(
            std::fs::read_to_string(&file).unwrap(),
            "[products\nlabel = ="
        );
        assert!(look.read_with(cx, |l, _| l.note.clone()).is_some());
        // The change still shows in the window.
        assert_eq!(
            look.read_with(cx, |l, _| l.data.product(None, "demo").label),
            Some("api".into())
        );
    }

    #[gpui::test]
    fn groups_rename_reorder_collapse_and_go_while_their_products_stay(cx: &mut TestAppContext) {
        let file = tempdir("groups").join("look.toml");
        let look = open(&file, cx);
        look.update(cx, |l, cx| {
            l.move_product(None, "a", "One", cx);
            l.move_product(None, "b", "Two", cx);
            l.move_product(Some("studio"), "a", "One", cx);
            l.move_product(None, "c", "Two", cx);
        });
        let names = |cx: &mut TestAppContext| {
            look.read_with(cx, |l, _| {
                l.data
                    .group_names(None)
                    .into_iter()
                    .map(String::from)
                    .collect::<Vec<_>>()
            })
        };
        assert_eq!(names(cx), ["One", "Two"]);
        look.update(cx, |l, cx| l.move_group(None, "Two", 0, cx));
        assert_eq!(names(cx), ["Two", "One"]);
        // The group of the other host keeps its own place.
        assert_eq!(
            look.read_with(cx, |l, _| l.data.group_names(Some("studio")).len()),
            1
        );
        assert!(look.update(cx, |l, cx| l.rename_group(None, "Two", "Ops", cx)));
        assert!(!look.update(cx, |l, cx| l.rename_group(None, "Ops", "One", cx)));
        assert!(!look.update(cx, |l, cx| l.rename_group(None, "Ops", "  ", cx)));
        look.update(cx, |l, cx| {
            l.set_group_hue(None, "Ops", Some(Hue::Amber), cx)
        });
        look.update(cx, |l, cx| l.remove_group(None, "Ops", cx));
        assert_eq!(names(cx), ["One"]);
        // A product moves to the group of an empty name: out of every group of its host.
        look.update(cx, |l, cx| l.move_product(None, "a", "", cx));
        let (data, _) = read(&file);
        assert!(data.group_of(None, "a").is_none());
        assert_eq!(data.group_of(Some("studio"), "a"), Some("One"));
        // "b" and "c" are no longer in a group: the products stay in the sidebar.
        let ps = [product("a"), product("b"), product("c")];
        assert_eq!(data.arrange(None, &ps).top, [0, 1, 2]);
    }
}
