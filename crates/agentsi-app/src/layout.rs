//! The saved layout: `<config>/agentsi/layout.json`.
//!
//! The file holds the dock of `gpui-component` and the active product. A terminal
//! panel names its terminal id. At start, the app drops each panel whose terminal the
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
/// The version of the file. Version 2 added `tab_groups`. A file of version 1 reads, with no
/// tab group. A file of another version is not read.
pub const VERSION: u32 = 2;
/// The oldest version that still reads.
const OLDEST: u32 = 1;

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
    /// The tab groups. The dock keeps the order of the tabs, and a group names its tabs by
    /// `panel_key`.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub tab_groups: Vec<TabGroupSave>,
    pub dock: DockAreaState,
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
    if let Some(host) = host_of(node) {
        if known.hosts.as_ref().is_none_or(|h| h.contains(host)) {
            return true;
        }
        dropped.push(host.to_string());
        return false;
    }
    if node.panel_name == TERMINAL_PANEL {
        return match terminal_of(node) {
            Some(id) if known.terminals.contains(id) => true,
            Some(id) => {
                dropped.push(id.to_string());
                false
            }
            None => false,
        };
    }
    match &mut node.info {
        PanelInfo::Stack { sizes, .. } => {
            let mut keep = Vec::new();
            let mut kept_sizes = Vec::new();
            for (i, mut child) in std::mem::take(&mut node.children).into_iter().enumerate() {
                if prune_node(&mut child, known, dropped) {
                    keep.push(child);
                    if let Some(s) = sizes.get(i) {
                        kept_sizes.push(*s);
                    }
                }
            }
            *sizes = kept_sizes;
            node.children = keep;
            !node.children.is_empty()
        }
        PanelInfo::Tabs { active_index } => {
            let active = *active_index;
            let mut keep = Vec::new();
            let mut new_active = 0;
            for (i, mut child) in std::mem::take(&mut node.children).into_iter().enumerate() {
                if prune_node(&mut child, known, dropped) {
                    if i <= active {
                        new_active = keep.len();
                    }
                    keep.push(child);
                }
            }
            *active_index = new_active;
            node.children = keep;
            !node.children.is_empty()
        }
        PanelInfo::Panel(_) => true,
    }
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
            tab_groups: vec![TabGroupSave {
                name: "Build".into(),
                color: Some("teal".into()),
                collapsed: true,
                tabs: vec![panel_key(&term("t1-a")).unwrap()],
            }],
            dock: DockAreaState {
                version: None,
                center: tabs(0, vec![term("t1-a")]),
                ..Default::default()
            },
        };
        write(&file, &layout).unwrap();
        assert_eq!(read(&file), Ok(Some(layout.clone())));
        // A file of version 1, from before the tree and the tab groups, reads with every product
        // open and no tab group.
        std::fs::write(&file, r#"{"version": 1, "dock": {"center": {"panel_name": "", "children": [], "info": {"panel": null}}}}"#).unwrap();
        let old = read(&file).unwrap().unwrap();
        assert!(old.tree.is_empty());
        assert!(old.tab_groups.is_empty());
        assert_eq!(old.version, 1);
        // The groups survive a write and a read, and a file with none writes no key.
        write(&file, &layout).unwrap();
        assert!(
            std::fs::read_to_string(&file)
                .unwrap()
                .contains("\"tab_groups\"")
        );
        write(
            &file,
            &Layout {
                tab_groups: vec![],
                ..layout.clone()
            },
        )
        .unwrap();
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
}
