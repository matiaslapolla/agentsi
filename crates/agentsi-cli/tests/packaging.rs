//! The files in `packaging/`. The install script runs in a container
//! (`packaging/test-install-container.sh`), not here.

use serde_json::Value;
use std::path::PathBuf;

fn packaging(file: &str) -> String {
    let dir = PathBuf::from(std::env::var("CARGO_MANIFEST_DIR").expect("run with cargo"));
    std::fs::read_to_string(dir.join("../../packaging").join(file)).unwrap()
}

fn commands(hooks: &Value) -> Vec<(String, String)> {
    let mut out = Vec::new();
    for (event, groups) in hooks["hooks"].as_object().unwrap() {
        for group in groups.as_array().unwrap() {
            for hook in group["hooks"].as_array().unwrap() {
                out.push((event.clone(), hook["command"].as_str().unwrap().to_string()));
            }
        }
    }
    out
}

#[test]
fn the_switch_over_hooks_use_no_node_and_no_plugin_path() {
    let hooks: Value = serde_json::from_str(&packaging("hooks.json")).unwrap();
    let all = commands(&hooks);
    for (event, command) in &all {
        assert!(!command.contains("node"), "{event}: {command}");
        assert!(
            !command.contains("CLAUDE_PLUGIN_ROOT"),
            "{event}: {command}"
        );
    }
    let guard: Vec<_> = all.iter().filter(|(e, _)| e == "PreToolUse").collect();
    assert_eq!(guard.len(), 1);
    assert!(guard[0].1.ends_with("agentsi hook pre-tool-use"));
    assert_eq!(hooks["hooks"]["PreToolUse"][0]["matcher"], "Bash");
}

#[test]
fn the_switch_over_hooks_keep_each_bridge_event_of_the_plugin() {
    let dir = PathBuf::from(std::env::var("CARGO_MANIFEST_DIR").unwrap());
    let plugin = std::fs::read_to_string(dir.join("../../plugin/hooks/hooks.json")).unwrap();
    let plugin: Value = serde_json::from_str(&plugin).unwrap();
    let ours: Value = serde_json::from_str(&packaging("hooks.json")).unwrap();
    let bridge = |hooks: &Value| {
        let mut events: Vec<String> = commands(hooks)
            .into_iter()
            .filter(|(_, c)| c.contains("agentsi-core hook "))
            .map(|(e, c)| format!("{e}:{c}"))
            .collect();
        events.sort();
        events
    };
    assert_eq!(bridge(&ours), bridge(&plugin));
    assert!(!bridge(&ours).is_empty());
}

#[test]
fn the_unit_and_the_agent_start_the_core_in_the_foreground() {
    let unit = packaging("systemd/agentsi-core.service");
    assert!(unit.contains("ExecStart=@BINDIR@/agentsi-core serve\n"));
    assert!(!unit.contains("--detach"));
    assert!(unit.contains("WantedBy=default.target"));
    let plist = packaging("launchd/dev.agentsi.core.plist");
    assert!(plist.contains("<string>@BINDIR@/agentsi-core</string>"));
    assert!(plist.contains("<string>serve</string>"));
    assert!(!plist.contains("--detach"));
}

#[test]
fn the_pill_unit_starts_the_pill_with_the_graphical_session_after_the_core() {
    let unit = packaging("systemd/agentsi-pill.service");
    assert!(unit.contains("ExecStart=@BINDIR@/agentsi-pill\n"));
    assert!(unit.contains("WantedBy=graphical-session.target\n"));
    assert!(unit.contains("PartOf=graphical-session.target\n"));
    assert!(unit.contains("Wants=agentsi-core.service\n"));
    assert!(unit.contains("After=agentsi-core.service graphical-session.target\n"));
    assert!(unit.contains("Restart=on-failure\n"));
    let script = packaging("install.sh");
    assert!(script.contains("systemd/agentsi-pill.service"));
    assert!(script.contains("enable --now agentsi-pill.service"));
}

#[test]
fn the_install_script_refuses_a_target_outside_the_home() {
    let script = packaging("install.sh");
    assert!(script.contains("outside $HOME"));
    assert!(script.contains("set -eu"));
}

#[test]
fn the_plist_comment_holds_no_double_hyphen() {
    // XML forbids `--` inside a comment, and launchctl would refuse the file.
    let plist = packaging("launchd/dev.agentsi.core.plist");
    let start = plist.find("<!-- ").unwrap() + 5;
    let comment_end = plist[start..].find("-->").unwrap() + start;
    assert!(!plist[start..comment_end].contains("--"));
}
