//! The saved layout: `<config>/agentsi/layout.json`.
//!
//! The file holds one dock of `gpui-component` for each space, and the active product. A
//! space is the dock of one product, or "Home" (`HOME`) for the tabs of no product. A terminal
//! panel names its terminal id. A pinned tab has `"pinned": true` in its panel entry. At start, the app drops each panel whose terminal the
//! core does not know: a terminal id names one terminal of one core run, so a panel
//! of an earlier core never attaches to another program.
//!
//! A panel of an SSH host also names the host. The host may be off at start, so such a
//! panel stays, unless `hosts.toml` no longer lists the host.

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

use gpui_kit::component::dock::{DockAreaState, PanelInfo, PanelState};
use serde::{Deserialize, Serialize};

/// The panel name of a terminal pane in the layout file.
pub const TERMINAL_PANEL: &str = "terminal";
/// The panel name of a screen of a product, for example the front desk.
pub const SCREEN_PANEL: &str = "screen";
/// The version of the file. Version 2 added `tab_groups`, version 3 `spaces`. A file of version
/// 1 or 2 reads: [`split_spaces`] moves its one dock into the spaces. A file of another version
/// is not read.
pub const VERSION: u32 = 3;
/// The oldest version that still reads.
const OLDEST: u32 = 1;
/// The key of the space "Home" in `Layout::spaces`. A product key (`look::key`) never starts
/// with `@` unless a `/` follows the host, so it cannot be this.
pub const HOME: &str = "@home";

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Layout {
    pub version: u32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub active_product: Option<String>,
    /// The host of the active product. With none, the product is on this device.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub active_host: Option<String>,
    /// The products that the owner opened or closed in the sidebar tree, by `look::key`. A
    /// product that is not here is open. A file from before the tree has none.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub tree: BTreeMap<String, bool>,
    /// The space on screen, by its key in `spaces`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub shown: Option<String>,
    /// The dock and the tab groups of each space, by `look::key` of its product or [`HOME`].
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub spaces: BTreeMap<String, SpaceSave>,
    /// The id of the settings section that the owner opened last. A file from before the
    /// sections has none, so no version bump.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub settings_section: Option<String>,
    /// The one dock of a file of version 1 or 2. Version 3 writes none.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub dock: Option<DockAreaState>,
    /// The tab groups of `dock`, in a file of version 2.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub tab_groups: Vec<TabGroupSave>,
}

/// One space in the file.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SpaceSave {
    pub dock: DockAreaState,
    /// The tab groups. The dock keeps the order of the tabs, and a group names its tabs by
    /// `panel_key`.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub tab_groups: Vec<TabGroupSave>,
}

/// One tab group in the file.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TabGroupSave {
    pub name: String,
    /// `Hue::key` of the color, if the group has one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub color: Option<String>,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub collapsed: bool,
    /// The tabs of the group, in the order of the tab bar, by `panel_key`.
    pub tabs: Vec<String>,
}

/// `$XDG_CONFIG_HOME`, else `~/.config`.
pub fn config_home() -> Option<PathBuf> {
    std::env::var_os("XDG_CONFIG_HOME")
        .filter(|v| !v.is_empty())
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".config")))
}

/// The config folder of the app, `<config>/agentsi`.
pub fn dir() -> Option<PathBuf> {
    config_home().map(|c| c.join("agentsi"))
}

/// `<config>/agentsi/layout.json`.
pub fn path() -> Option<PathBuf> {
    dir().map(|d| d.join("layout.json"))
}

/// The layout in `path`. A missing file gives `Ok(None)`. A file that does not parse,
/// or has another version, is an error with the reason.
pub fn read(path: &Path) -> Result<Option<Layout>, String> {
    let text = match std::fs::read_to_string(path) {
        Ok(t) => t,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(e) => return Err(format!("cannot read {}: {e}", path.display())),
    };
    let layout: Layout = serde_json::from_str(&text)
        .map_err(|e| format!("{} does not parse: {e}", path.display()))?;
    if !(OLDEST..=VERSION).contains(&layout.version) {
        return Err(format!(
            "{} has version {}, not {VERSION}",
            path.display(),
            layout.version
        ));
    }
    Ok(Some(layout))
}

/// Writes the layout through a temporary file, so a crash leaves the old file whole.
pub fn write(path: &Path, layout: &Layout) -> std::io::Result<()> {
    let text = serde_json::to_string_pretty(layout).map_err(std::io::Error::other)?;
    crate::atomic::write(path, &(text + "\n"))
}

/// The info of a terminal panel. `host` is its SSH host.
pub fn terminal_info(id: &str, host: Option<&str>) -> PanelInfo {
    let mut info = serde_json::json!({ "terminal": id });
    if let Some(h) = host {
        info["host"] = serde_json::json!(h);
    }
    PanelInfo::panel(info)
}

/// The SSH host of a panel, or `None` for a panel of this device.
pub fn host_of(state: &PanelState) -> Option<&str> {
    match &state.info {
        PanelInfo::Panel(v) => v.get("host")?.as_str(),
        _ => None,
    }
}

/// The key that names a panel in `Layout::tab_groups`: the panel name and its info. A terminal
/// id or a screen names one panel, so the key does too. A container has no key.
pub fn panel_key(state: &PanelState) -> Option<String> {
    match &state.info {
        PanelInfo::Panel(v) => Some(format!("{}:{v}", state.panel_name)),
        _ => None,
    }
}

/// The terminal id of a terminal panel.
pub fn terminal_of(state: &PanelState) -> Option<&str> {
    if state.panel_name != TERMINAL_PANEL {
        return None;
    }
    match &state.info {
        PanelInfo::Panel(v) => v.get("terminal")?.as_str(),
        _ => None,
    }
}

/// What the app knows at start, to check the panels of the file.
pub struct Known {
    /// The terminals of the core of this device.
    pub terminals: BTreeSet<String>,
    /// The SSH hosts of `hosts.toml`, or `None` when the file did not parse: then the app
    /// cannot tell which host left, and each panel of a host stays.
    pub hosts: Option<BTreeSet<String>>,
}

/// Removes each panel of a host that `known` does not list, each local terminal panel
/// whose id `known` does not list, and each container that becomes empty. Returns the
/// removed terminal ids and host names, in the order of the file.
pub fn prune(dock: &mut DockAreaState, known: &Known) -> Vec<String> {
    let mut dropped = Vec::new();
    prune_node(&mut dock.center, known, &mut dropped);
    dropped
}

/// Returns false when `node` itself must go.
fn prune_node(node: &mut PanelState, known: &Known, dropped: &mut Vec<String>) -> bool {
    retain(node, &mut |leaf| {
        if let Some(host) = host_of(leaf) {
            if known.hosts.as_ref().is_none_or(|h| h.contains(host)) {
                return true;
            }
            dropped.push(host.to_string());
            return false;
        }
        if leaf.panel_name != TERMINAL_PANEL {
            return true;
        }
        match terminal_of(leaf) {
            Some(id) if known.terminals.contains(id) => true,
            Some(id) => {
                dropped.push(id.to_string());
                false
            }
            None => false,
        }
    })
}

/// Keeps the panels for which `keep` is true, and the containers that still hold one. The tab on
/// screen of a bar falls back to the tab before it. Returns false when `node` itself must go.
fn retain(node: &mut PanelState, keep: &mut dyn FnMut(&PanelState) -> bool) -> bool {
    if matches!(node.info, PanelInfo::Panel(_)) {
        return keep(node);
    }
    match &mut node.info {
        PanelInfo::Stack { sizes, .. } => {
            let mut kept = Vec::new();
            let mut kept_sizes = Vec::new();
            for (i, mut child) in std::mem::take(&mut node.children).into_iter().enumerate() {
                if retain(&mut child, keep) {
                    kept.push(child);
                    if let Some(s) = sizes.get(i) {
                        kept_sizes.push(*s);
                    }
                }
            }
            *sizes = kept_sizes;
            node.children = kept;
        }
        PanelInfo::Tabs { active_index } => {
            let active = *active_index;
            let mut kept = Vec::new();
            let mut new_active = 0;
            for (i, mut child) in std::mem::take(&mut node.children).into_iter().enumerate() {
                if retain(&mut child, keep) {
                    if i <= active {
                        new_active = kept.len();
                    }
                    kept.push(child);
                }
            }
            *active_index = new_active;
            node.children = kept;
        }
        PanelInfo::Panel(_) => {}
    }
    !node.children.is_empty()
}

/// Calls `f` on each panel of the dock, in the order of the file.
fn each_panel(node: &mut PanelState, f: &mut dyn FnMut(&mut PanelState)) {
    if matches!(node.info, PanelInfo::Panel(_)) {
        f(node);
        return;
    }
    for c in &mut node.children {
        each_panel(c, f);
    }
}

/// Splits the one dock of a file of version 1 or 2 into spaces: each panel goes to the space
/// that `space_of` names, and the splits and bars around it come with it. A tab group goes to
/// each space that holds one of its tabs, with those tabs.
pub fn split_spaces(
    dock: DockAreaState,
    groups: &[TabGroupSave],
    space_of: impl Fn(&PanelState) -> String,
) -> BTreeMap<String, SpaceSave> {
    let mut of_key: BTreeMap<String, String> = BTreeMap::new();
    let mut names: BTreeSet<String> = BTreeSet::new();
    let mut center = dock.center.clone();
    each_panel(&mut center, &mut |leaf| {
        let space = space_of(leaf);
        if let Some(k) = panel_key(leaf) {
            of_key.insert(k, space.clone());
        }
        names.insert(space);
    });
    names
        .into_iter()
        .filter_map(|name| {
            let mut part = dock.clone();
            if !retain(&mut part.center, &mut |leaf| space_of(leaf) == name) {
                return None;
            }
            let tab_groups = groups
                .iter()
                .filter_map(|g| {
                    let tabs: Vec<String> = g
                        .tabs
                        .iter()
                        .filter(|t| of_key.get(*t) == Some(&name))
                        .cloned()
                        .collect();
                    (!tabs.is_empty()).then(|| TabGroupSave { tabs, ..g.clone() })
                })
                .collect();
            Some((
                name,
                SpaceSave {
                    dock: part,
                    tab_groups,
                },
            ))
        })
        .collect()
}

/// Takes the `pinned` flag out of each panel entry, so a panel builder never sees it. Returns
/// the `panel_key` of each pinned panel.
pub fn take_pinned(dock: &mut DockAreaState) -> BTreeSet<String> {
    let mut out = BTreeSet::new();
    each_panel(&mut dock.center, &mut |leaf| {
        let PanelInfo::Panel(v) = &mut leaf.info else {
            return;
        };
        let pinned = v
            .as_object_mut()
            .and_then(|o| o.remove("pinned"))
            .is_some_and(|p| p.as_bool() == Some(true));
        if pinned && let Some(k) = panel_key(leaf) {
            out.insert(k);
        }
    });
    out
}

/// Writes `"pinned": true` into the entry of each panel whose `panel_key` is in `pinned`.
pub fn mark_pinned(dock: &mut DockAreaState, pinned: &BTreeSet<String>) {
    each_panel(&mut dock.center, &mut |leaf| {
        if panel_key(leaf).is_some_and(|k| pinned.contains(&k))
            && let PanelInfo::Panel(v) = &mut leaf.info
            && let Some(o) = v.as_object_mut()
        {
            o.insert("pinned".into(), serde_json::Value::Bool(true));
        }
    });
}

/// Each terminal id in the layout, in the order of the file.
#[cfg(test)]
pub fn terminals(dock: &DockAreaState) -> Vec<String> {
    fn walk(node: &PanelState, out: &mut Vec<String>) {
        if let Some(id) = terminal_of(node) {
            out.push(id.to_string());
        }
        for c in &node.children {
            walk(c, out);
        }
    }
    let mut out = Vec::new();
    walk(&dock.center, &mut out);
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use gpui_kit::{Axis, Pixels};

    fn term(id: &str) -> PanelState {
        PanelState {
            panel_name: TERMINAL_PANEL.into(),
            children: vec![],
            info: terminal_info(id, None),
        }
    }

    fn screen() -> PanelState {
        PanelState {
            panel_name: SCREEN_PANEL.into(),
            children: vec![],
            info: PanelInfo::panel(serde_json::json!({"kind": "desk", "product": "demo"})),
        }
    }

    fn tabs(active: usize, children: Vec<PanelState>) -> PanelState {
        PanelState {
            panel_name: "TabPanel".into(),
            children,
            info: PanelInfo::tabs(active),
        }
    }

    fn known(ids: &[&str]) -> Known {
        Known {
            terminals: ids.iter().map(|s| s.to_string()).collect(),
            hosts: Some(BTreeSet::from(["studio".to_string()])),
        }
    }

    #[test]
    fn a_panel_of_a_known_host_stays_and_a_panel_of_a_lost_host_goes() {
        let panel = |info: serde_json::Value, name: &str| PanelState {
            panel_name: name.into(),
            children: vec![],
            info: PanelInfo::panel(info),
        };
        let mut node = tabs(
            0,
            vec![
                PanelState {
                    info: terminal_info("t1-a", Some("studio")),
                    ..term("t1-a")
                },
                PanelState {
                    info: terminal_info("t2-a", Some("laptop")),
                    ..term("t2-a")
                },
                panel(
                    serde_json::json!({"kind": "desk", "product": "demo", "host": "studio"}),
                    SCREEN_PANEL,
                ),
                panel(
                    serde_json::json!({"kind": "desk", "product": "demo", "host": "laptop"}),
                    SCREEN_PANEL,
                ),
            ],
        );
        let mut dropped = vec![];
        assert!(prune_node(&mut node, &known(&[]), &mut dropped));
        assert_eq!(dropped, ["laptop", "laptop"]);
        let hosts: Vec<_> = node.children.iter().map(host_of).collect();
        assert_eq!(hosts, [Some("studio"), Some("studio")]);
    }

    #[test]
    fn a_hosts_file_that_does_not_parse_drops_no_panel_of_a_host() {
        let mut node = tabs(
            0,
            vec![PanelState {
                info: terminal_info("t1-a", Some("laptop")),
                ..term("t1-a")
            }],
        );
        let unread = Known {
            hosts: None,
            ..known(&[])
        };
        let mut dropped = vec![];
        assert!(prune_node(&mut node, &unread, &mut dropped));
        assert!(dropped.is_empty());
    }

    #[test]
    fn panels_of_unknown_terminals_go_and_empty_containers_with_them() {
        let mut dock = DockAreaState {
            version: None,
            center: PanelState {
                panel_name: "StackPanel".into(),
                children: vec![
                    tabs(2, vec![screen(), term("t1-old"), term("t2-run")]),
                    tabs(0, vec![term("t3-old")]),
                    tabs(0, vec![term("t4-run")]),
                ],
                info: PanelInfo::stack(
                    vec![Pixels::from(100.), Pixels::from(200.), Pixels::from(300.)],
                    Axis::Horizontal,
                ),
            },
            ..Default::default()
        };
        let dropped = prune(&mut dock, &known(&["t2-run", "t4-run"]));
        assert_eq!(dropped, ["t1-old", "t3-old"]);
        assert_eq!(terminals(&dock), ["t2-run", "t4-run"]);
        let c = &dock.center;
        assert_eq!(c.children.len(), 2);
        assert_eq!(
            c.info.sizes().unwrap(),
            &vec![Pixels::from(100.), Pixels::from(300.)]
        );
        // The active tab was `t2-run`, now at index 1.
        assert_eq!(c.children[0].info.active_index(), Some(1));
    }

    #[test]
    fn the_active_tab_falls_back_to_the_tab_before_it() {
        let mut node = tabs(1, vec![term("t1-a"), term("t2-old"), term("t3-a")]);
        let mut dropped = vec![];
        assert!(prune_node(
            &mut node,
            &known(&["t1-a", "t3-a"]),
            &mut dropped
        ));
        assert_eq!(node.info.active_index(), Some(0));
    }

    fn space(center: PanelState, groups: Vec<TabGroupSave>) -> SpaceSave {
        SpaceSave {
            dock: DockAreaState {
                version: None,
                center,
                ..Default::default()
            },
            tab_groups: groups,
        }
    }

    fn desk(product: &str) -> PanelState {
        PanelState {
            panel_name: SCREEN_PANEL.into(),
            children: vec![],
            info: PanelInfo::panel(serde_json::json!({"kind": "desk", "product": product})),
        }
    }

    #[test]
    fn the_file_round_trips_and_a_bad_file_is_an_error() {
        let dir = std::env::temp_dir().join(format!("agentsi-layout-{}", std::process::id()));
        let file = dir.join("agentsi").join("layout.json");
        assert_eq!(read(&file), Ok(None));
        let layout = Layout {
            version: VERSION,
            active_product: Some("demo".into()),
            active_host: Some("studio".into()),
            tree: BTreeMap::from([("demo".to_string(), false), ("@studio/x".to_string(), true)]),
            shown: Some("demo".into()),
            spaces: BTreeMap::from([
                (
                    "demo".to_string(),
                    space(
                        tabs(0, vec![term("t1-a")]),
                        vec![TabGroupSave {
                            name: "Build".into(),
                            color: Some("teal".into()),
                            collapsed: true,
                            tabs: vec![panel_key(&term("t1-a")).unwrap()],
                        }],
                    ),
                ),
                (HOME.to_string(), space(tabs(0, vec![term("t9-a")]), vec![])),
            ]),
            settings_section: Some("terminal".into()),
            dock: None,
            tab_groups: vec![],
        };
        write(&file, &layout).unwrap();
        assert_eq!(read(&file), Ok(Some(layout.clone())));
        let text = std::fs::read_to_string(&file).unwrap();
        assert!(text.contains("\"spaces\"") && text.contains("\"tab_groups\""));
        // Version 3 writes no dock of its own.
        let value: serde_json::Value = serde_json::from_str(&text).unwrap();
        assert!(value.get("dock").is_none(), "{text}");
        // A file of version 1, from before the tree and the tab groups, reads with every product
        // open and no tab group.
        std::fs::write(&file, r#"{"version": 1, "dock": {"center": {"panel_name": "", "children": [], "info": {"panel": null}}}}"#).unwrap();
        let old = read(&file).unwrap().unwrap();
        assert!(old.tree.is_empty());
        assert!(old.tab_groups.is_empty());
        assert!(old.spaces.is_empty());
        assert!(old.dock.is_some());
        assert_eq!(old.settings_section, None);
        assert_eq!(old.version, 1);
        // A space with no group writes no `tab_groups` key.
        let mut bare = layout.clone();
        for s in bare.spaces.values_mut() {
            s.tab_groups.clear();
        }
        write(&file, &bare).unwrap();
        assert!(
            !std::fs::read_to_string(&file)
                .unwrap()
                .contains("tab_groups")
        );
        std::fs::write(&file, "{").unwrap();
        assert!(read(&file).unwrap_err().contains("does not parse"));
        std::fs::write(&file, r#"{"version": 9, "dock": {"center": {"panel_name": "", "children": [], "info": {"panel": null}}}}"#).unwrap();
        assert!(read(&file).unwrap_err().contains("version 9"));
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn a_version_2_file_splits_into_one_space_for_each_product_with_its_groups() {
        let dir = std::env::temp_dir().join(format!("agentsi-layout-v2-{}", std::process::id()));
        let file = dir.join("layout.json");
        let center = PanelState {
            panel_name: "StackPanel".into(),
            children: vec![
                tabs(2, vec![desk("a"), term("t1-a"), term("t2-b"), desk("b")]),
                tabs(0, vec![term("t3-a"), term("t4-none")]),
            ],
            info: PanelInfo::stack(
                vec![Pixels::from(100.), Pixels::from(200.)],
                Axis::Horizontal,
            ),
        };
        let v2 = serde_json::json!({
            "version": 2,
            "active_product": "a",
            "tab_groups": [{
                "name": "Mixed",
                "tabs": [panel_key(&term("t1-a")).unwrap(), panel_key(&term("t2-b")).unwrap()],
            }],
            "dock": DockAreaState { version: None, center, ..Default::default() },
        });
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(&file, v2.to_string()).unwrap();
        let mut old = read(&file).unwrap().unwrap();
        assert_eq!(old.version, 2);
        let product = |leaf: &PanelState| match terminal_of(leaf) {
            Some(id) => match id.rsplit('-').next() {
                Some("none") | None => HOME.to_string(),
                Some(p) => p.to_string(),
            },
            None => crate::screens::screen_of(leaf).unwrap().product,
        };
        let spaces = split_spaces(old.dock.take().unwrap(), &old.tab_groups, product);
        let names: Vec<&str> = spaces.keys().map(String::as_str).collect();
        assert_eq!(names, [HOME, "a", "b"]);
        assert_eq!(terminals(&spaces["a"].dock), ["t1-a", "t3-a"]);
        assert_eq!(terminals(&spaces["b"].dock), ["t2-b"]);
        assert_eq!(terminals(&spaces[HOME].dock), ["t4-none"]);
        // Each space keeps its split: `a` has two bars, `b` one.
        assert_eq!(spaces["a"].dock.center.children.len(), 2);
        assert_eq!(spaces["b"].dock.center.children.len(), 1);
        // The old bar showed t2-b: the bar of `b` still does.
        let bar = &spaces["b"].dock.center.children[0];
        assert_eq!(bar.children.len(), 2);
        assert_eq!(bar.info.active_index(), Some(0));
        // The group with tabs of a and b becomes one group in each space.
        assert_eq!(spaces["a"].tab_groups.len(), 1);
        assert_eq!(spaces["a"].tab_groups[0].name, "Mixed");
        assert_eq!(
            spaces["a"].tab_groups[0].tabs,
            [panel_key(&term("t1-a")).unwrap()]
        );
        assert_eq!(
            spaces["b"].tab_groups[0].tabs,
            [panel_key(&term("t2-b")).unwrap()]
        );
        assert!(spaces[HOME].tab_groups.is_empty());
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn the_pinned_flag_leaves_the_entry_before_the_load_and_comes_back_at_the_save() {
        let mut dock = DockAreaState {
            version: None,
            center: tabs(0, vec![term("t1-a"), term("t2-a")]),
            ..Default::default()
        };
        let key = panel_key(&term("t1-a")).unwrap();
        mark_pinned(&mut dock, &BTreeSet::from([key.clone()]));
        let text = serde_json::to_string(&dock).unwrap();
        assert_eq!(text.matches("\"pinned\":true").count(), 1, "{text}");
        assert_eq!(take_pinned(&mut dock), BTreeSet::from([key]));
        assert!(!serde_json::to_string(&dock).unwrap().contains("pinned"));
        // The key of a tab group still finds the tab.
        assert_eq!(
            panel_key(&dock.center.children[0]),
            panel_key(&term("t1-a"))
        );
    }
}
