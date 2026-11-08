//! The dictation key for a Hyprland session.
//!
//! A Wayland client cannot read a global key, so the compositor runs `agentsi-core dictate
//! start` on press and `dictate stop` on release. AgentSI writes the bind file in its own
//! config folder. The owner includes that file one time, with the include line. AgentSI never
//! edits the compositor config.
//!
//! The toggle form (`Mode::Toggle`) runs `agentsi-core dictate toggle` on press and has no
//! release bind: one press starts the dictation and the next press stops it.
//!
//! A Lua Hyprland config has no release bind that works: it misses the release when the
//! modifier goes up first. The Lua bind file stops the dictation on the first key release of
//! the raw key stream instead.
//!
//! A function that runs `hyprctl` or writes a file blocks. Call it off the event loop.

use std::fmt;
use std::path::{Path, PathBuf};
use std::process::Command;

use agentsi_proto::client::find_core_bin;
use agentsi_proto::socket::SOCKET_ENV;
use gpui_kit::{Keystroke, OperatingSystem};

/// The description of the bind. It marks the bind as the one of AgentSI in `hyprctl binds`.
pub const DESCRIPTION: &str = "AgentSI: hold to dictate";
/// The description of the bind of the toggle form.
pub const DESCRIPTION_TOGGLE: &str = "AgentSI: press to dictate";
/// The description of the conversation bind.
pub const DESCRIPTION_CONVERSE: &str = "AgentSI: press to talk to a pane";

/// How the dictation key works. `keys.toml` keeps it as `[dictation] mode`.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Mode {
    /// The dictation lasts while the owner holds the key.
    #[default]
    Hold,
    /// One press starts the dictation and the next press stops it.
    Toggle,
}

impl Mode {
    pub fn as_str(self) -> &'static str {
        match self {
            Mode::Hold => "hold",
            Mode::Toggle => "toggle",
        }
    }

    pub fn parse(text: &str) -> Option<Mode> {
        match text {
            "hold" => Some(Mode::Hold),
            "toggle" => Some(Mode::Toggle),
            _ => None,
        }
    }
}

const LUA_FILE: &str = "hypr-dictation.lua";
const CONF_FILE: &str = "hypr-dictation.conf";

/// Modifier bits of `hyprctl binds -j`. Caps lock and num lock never count.
const SHIFT: u32 = 1;
const CTRL: u32 = 4;
const ALT: u32 = 8;
const SUPER: u32 = 64;
const LOCKS: u32 = 2 | 16;

/// The form of the compositor config.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Form {
    /// `hyprland.lua`: the bind file is Lua.
    Lua,
    /// `hyprland.conf`: the bind file has `bindd` and `bindrnid` lines. The release bind ignores
    /// the modifiers, because a plain `bindr` misses the release when the modifier goes up first.
    /// It does not consume the key, so a plain release of the key still reaches its program.
    Conf,
    /// No Hyprland session, or no config in the folder: the owner gets the two commands.
    CommandsOnly,
}

/// `<config>/hypr`: the folder of the Hyprland config.
pub fn hypr_dir() -> Option<PathBuf> {
    crate::layout::config_home().map(|c| c.join("hypr"))
}

/// `<config>/agentsi`. The bind file lives here.
pub fn agentsi_dir() -> Option<PathBuf> {
    crate::layout::dir()
}

/// True when this process runs in a Hyprland session.
pub fn in_session() -> bool {
    std::env::var_os("HYPRLAND_INSTANCE_SIGNATURE").is_some_and(|v| !v.is_empty())
}

/// The form of the config in `hypr_dir`. `hyprland.lua` wins over `hyprland.conf`, as in
/// Hyprland.
pub fn detect(hypr_dir: &Path, in_session: bool) -> Form {
    if !in_session {
        Form::CommandsOnly
    } else if hypr_dir.join("hyprland.lua").is_file() {
        Form::Lua
    } else if hypr_dir.join("hyprland.conf").is_file() {
        Form::Conf
    } else {
        Form::CommandsOnly
    }
}

/// The program that the bind runs, and the socket that it gets.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CoreCall {
    /// Absolute path of `agentsi-core`.
    pub bin: PathBuf,
    /// The value of `AGENTSI_CORE_SOCKET` of the app, when it is set.
    pub socket: Option<String>,
}

impl CoreCall {
    /// `AGENTSI_CORE_BIN`, else the sibling of the running app, else the first `agentsi-core`
    /// on the `PATH`.
    pub fn from_env() -> Option<CoreCall> {
        let bin = find_core_bin()?;
        let bin = std::path::absolute(&bin).unwrap_or(bin);
        let socket = std::env::var(SOCKET_ENV).ok().filter(|s| !s.is_empty());
        Some(CoreCall { bin, socket })
    }

    /// The shell command for `dictate <action>`.
    pub fn command(&self, action: &str) -> String {
        self.command_of("dictate", action)
    }

    /// The shell command for `converse toggle`: one press starts conversation mode and the
    /// next stops it.
    pub fn converse_command(&self) -> String {
        self.command_of("converse", "toggle")
    }

    /// A core call that no text uses. The empty file needs none.
    fn none() -> CoreCall {
        CoreCall {
            bin: PathBuf::new(),
            socket: None,
        }
    }

    fn command_of(&self, verb: &str, action: &str) -> String {
        let env = self
            .socket
            .as_ref()
            .map(|s| format!("{SOCKET_ENV}={} ", quote(s)))
            .unwrap_or_default();
        format!(
            "{env}{} {verb} {action}",
            quote(&self.bin.to_string_lossy())
        )
    }

    /// The two commands for a compositor that AgentSI cannot write for: press, then release.
    pub fn commands(&self) -> [String; 2] {
        [self.command("start"), self.command("stop")]
    }
}

fn quote(s: &str) -> String {
    format!("'{}'", s.replace('\'', "'\\''"))
}

/// Hyprlang reads `#` as the start of a comment, and `##` as one `#`.
fn hyprlang(s: &str) -> String {
    s.replace('#', "##")
}

fn lua_string(s: &str) -> String {
    let mut out = String::from("\"");
    for c in s.chars() {
        match c {
            '\\' => out.push_str("\\\\"),
            '"' => out.push_str("\\\""),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            c => out.push(c),
        }
    }
    out.push('"');
    out
}

/// The text of the bind file for the Hyprland key `hypr_key` and the mode.
#[cfg(test)]
pub fn render_for(mode: Mode, form: Form, hypr_key: &str, core: &CoreCall) -> String {
    render_with(mode, form, Some(hypr_key), None, core)
}

/// The text of the bind file for the dictation key and the conversation key, each in Hyprland
/// form (`SUPER + F10`) or absent. The conversation key is one bind that runs `converse toggle`
/// in the same file, in both modes of the dictation key. With neither key, the file is valid
/// and binds nothing.
pub fn render_with(
    mode: Mode,
    form: Form,
    dictation: Option<&str>,
    converse: Option<&str>,
    core: &CoreCall,
) -> String {
    match form {
        Form::Lua => {
            let mut text = String::from(LUA_HEAD);
            if let Some(key) = dictation {
                text.push_str(&match mode {
                    Mode::Hold => lua_hold(key, core),
                    Mode::Toggle => lua_toggle(key, core),
                });
            }
            if let Some(key) = converse {
                text.push_str(&lua_converse(key, core));
            }
            text.push_str("end\n");
            if dictation.is_none() && converse.is_none() {
                text.push_str("-- No dictation key is set.\n");
            }
            text
        }
        Form::Conf => {
            let mut text = String::from(CONF_HEADER);
            if let Some(key) = dictation {
                text.push_str(&match mode {
                    Mode::Hold => conf_hold(key, core),
                    Mode::Toggle => conf_bind(key, DESCRIPTION_TOGGLE, &core.command("toggle")),
                });
            }
            if let Some(key) = converse {
                text.push_str(&conf_bind(
                    key,
                    DESCRIPTION_CONVERSE,
                    &core.converse_command(),
                ));
            }
            if dictation.is_none() && converse.is_none() {
                text.push_str("# No dictation key is set.\n");
            }
            text
        }
        Form::CommandsOnly => String::new(),
    }
}

/// `SUPER + F10` as the modifiers and the key of a `bindd` line.
fn conf_chord(hypr_key: &str) -> (String, &str) {
    let (mods, key) = hypr_key.rsplit_once(" + ").unwrap_or(("", hypr_key));
    (mods.replace(" + ", " "), key)
}

/// One `bindd` line.
fn conf_bind(hypr_key: &str, description: &str, command: &str) -> String {
    let (mods, key) = conf_chord(hypr_key);
    format!(
        "bindd = {mods}, {key}, {description}, exec, {}\n",
        hyprlang(command)
    )
}

fn conf_hold(hypr_key: &str, core: &CoreCall) -> String {
    let (mods, key) = conf_chord(hypr_key);
    let (start, stop) = (
        hyprlang(&core.command("start")),
        hyprlang(&core.command("stop")),
    );
    format!(
        "bindd = {mods}, {key}, {DESCRIPTION}, exec, {start}\nbindrnid = {mods}, {key}, {DESCRIPTION}, exec, {stop}\n"
    )
}

fn lua_toggle(hypr_key: &str, core: &CoreCall) -> String {
    format!(
        "  state.bind = hl.bind({key}, function()\n    hl.exec_cmd({toggle})\n  end, {{ description = {desc} }})\n",
        key = lua_string(hypr_key),
        toggle = lua_string(&core.command("toggle")),
        desc = lua_string(DESCRIPTION_TOGGLE),
    )
}

fn lua_converse(hypr_key: &str, core: &CoreCall) -> String {
    format!(
        "  state.converse = hl.bind({key}, function()\n    hl.exec_cmd({toggle})\n  end, {{ description = {desc} }})\n",
        key = lua_string(hypr_key),
        toggle = lua_string(&core.converse_command()),
        desc = lua_string(DESCRIPTION_CONVERSE),
    )
}

fn lua_hold(hypr_key: &str, core: &CoreCall) -> String {
    format!(
        "  local holding = false\n  state.bind = hl.bind({key}, function()\n    if holding then return end\n    holding = true\n    hl.exec_cmd({start})\n  end, {{ description = {desc} }})\n  state.listener = hl.on(\"input.keyboard.key\", function(_, _, key_state)\n    if state.token == token and key_state == 0 and holding then\n      holding = false\n      hl.exec_cmd({stop})\n    end\n  end)\n",
        key = lua_string(hypr_key),
        start = lua_string(&core.command("start")),
        stop = lua_string(&core.command("stop")),
        desc = lua_string(DESCRIPTION),
    )
}

/// The text of the bind file for the Hyprland key `hypr_key` (`SUPER + F10`), in the hold form.
#[cfg(test)]
pub fn render(form: Form, hypr_key: &str, core: &CoreCall) -> String {
    render_with(Mode::Hold, form, Some(hypr_key), None, core)
}

/// The start of the Lua bind file. `hyprctl eval` loads the file again in the Lua state that
/// Hyprland keeps across reloads, so the file keeps its state in one global table. It removes
/// the binds of an older load only when the handle is alive. A reload expires the handles, so
/// after a reload nothing is removed and a bind of the owner on the old key stays. Hyprland
/// removes every bind on the chord of a live handle. Each load gets a new token, so the key
/// listener of an older load does nothing. The conversation bind keeps its handle in
/// `state.converse`, so a second load removes it the same way.
const LUA_HEAD: &str =
    "-- Written by AgentSI. Change the key in the AgentSI settings. Edits are lost.
agentsi_dictation = agentsi_dictation or {}
do
  local state = agentsi_dictation
  if state.bind ~= nil and tostring(state.bind) ~= \"HL.Keybind(expired)\" then
    pcall(function() state.bind:remove() end)
  end
  if state.converse ~= nil and tostring(state.converse) ~= \"HL.Keybind(expired)\" then
    pcall(function() state.converse:remove() end)
  end
  if state.listener ~= nil then
    pcall(function() state.listener:remove() end)
  end
  state.bind, state.listener, state.converse = nil, nil, nil
  state.token = (state.token or 0) + 1
  local token = state.token
";
const CONF_HEADER: &str =
    "# Written by AgentSI. Change the key in the AgentSI settings. Edits are lost.\n";

/// A valid bind file with no bind. It is what a removed key leaves, so the include line stays
/// valid. The Lua form still removes the bind of the load before, because `eval` loads it
/// without a reload.
pub fn render_empty(form: Form) -> String {
    render_with(Mode::Hold, form, None, None, &CoreCall::none())
}

/// Modifiers of a key, in the order that both forms print them.
const MODS: [(&str, &str, u32); 4] = [
    ("super", "SUPER", SUPER),
    ("ctrl", "CTRL", CTRL),
    ("alt", "ALT", ALT),
    ("shift", "SHIFT", SHIFT),
];

/// Keys that are not a letter, a digit or a function key: GPUI name, Hyprland name.
const NAMED: [(&str, &str); 28] = [
    ("space", "SPACE"),
    ("enter", "RETURN"),
    ("escape", "ESCAPE"),
    ("tab", "TAB"),
    ("backspace", "BACKSPACE"),
    ("delete", "DELETE"),
    ("insert", "INSERT"),
    ("home", "HOME"),
    ("end", "END"),
    ("pageup", "PRIOR"),
    ("pagedown", "NEXT"),
    ("up", "UP"),
    ("down", "DOWN"),
    ("left", "LEFT"),
    ("right", "RIGHT"),
    ("-", "MINUS"),
    ("=", "EQUAL"),
    (",", "COMMA"),
    (".", "PERIOD"),
    ("/", "SLASH"),
    (";", "SEMICOLON"),
    ("'", "APOSTROPHE"),
    ("[", "BRACKETLEFT"),
    ("]", "BRACKETRIGHT"),
    ("\\", "BACKSLASH"),
    ("`", "GRAVE"),
    ("print", "PRINT"),
    ("menu", "MENU"),
];

/// Names that Hyprland accepts for a key in `NAMED`, besides the one in the table.
const ALIASES: [(&str, &str); 5] = [
    ("PAGE_UP", "pageup"),
    ("PAGE_DOWN", "pagedown"),
    ("ENTER", "enter"),
    ("ESC", "escape"),
    ("INS", "insert"),
];

#[derive(Debug, PartialEq, Eq)]
struct Chord {
    mods: u32,
    /// The GPUI name of the key: `v`, `f10`, `space`.
    key: String,
}

impl Chord {
    fn is_digit(&self) -> bool {
        self.key.len() == 1 && self.key.as_bytes()[0].is_ascii_digit()
    }
}

fn parse_parts(text: &str, parts: &[&str], hypr: bool) -> Result<Chord, String> {
    let mut mods = 0;
    let mut key = None;
    for part in parts.iter().map(|p| p.trim()) {
        if part.is_empty() {
            return Err(format!("{text:?} has an empty part"));
        }
        let modifier = match part.to_ascii_lowercase().as_str() {
            "super" | "cmd" | "win" | "mod4" => Some(SUPER),
            "ctrl" | "control" => Some(CTRL),
            "alt" | "mod1" => Some(ALT),
            "shift" => Some(SHIFT),
            _ => None,
        };
        match (modifier, key) {
            (Some(bit), _) => mods |= bit,
            (None, None) => key = Some(part),
            (None, Some(_)) => return Err(format!("{text:?} has more than one key")),
        }
    }
    let key = key.ok_or_else(|| format!("{text:?} has no key"))?;
    let key = if hypr {
        gpui_key(key)
    } else {
        known_gpui_key(key)
    };
    key.map(|key| Chord { mods, key })
        .ok_or_else(|| format!("the key {text:?} is not supported"))
}

/// A GPUI key such as `super-f10`.
fn parse_gpui(text: &str) -> Result<Chord, String> {
    let ks = Keystroke::parse_with_os(text, OperatingSystem::Linux)
        .ok()
        .filter(|ks| !ks.key.is_empty() && !ks.is_modifier_key() && !ks.modifiers.function)
        .ok_or_else(|| format!("{text:?} is not a key"))?;
    let m = ks.modifiers;
    let mods = [
        (m.platform, SUPER),
        (m.control, CTRL),
        (m.alt, ALT),
        (m.shift, SHIFT),
    ]
    .iter()
    .filter(|(on, _)| *on)
    .fold(0, |bits, (_, bit)| bits | bit);
    known_gpui_key(&ks.key)
        .map(|key| Chord { mods, key })
        .ok_or_else(|| format!("the key {text:?} is not supported"))
}

fn known_gpui_key(key: &str) -> Option<String> {
    let lower = key.to_ascii_lowercase();
    let simple = lower.len() == 1 && lower.chars().all(|c| c.is_ascii_alphanumeric());
    let function = crate::keys::is_function_key(&lower);
    let named = NAMED.iter().any(|(g, _)| *g == key);
    (simple || function || named).then(|| if named { key.to_string() } else { lower })
}

/// The GPUI name of a Hyprland key name, or `None` when the table has no such key.
fn gpui_key(hypr: &str) -> Option<String> {
    let upper = hypr.to_ascii_uppercase();
    if let Some((g, _)) = NAMED.iter().find(|(_, h)| *h == upper) {
        return Some((*g).to_string());
    }
    if let Some((_, g)) = ALIASES.iter().find(|(a, _)| *a == upper) {
        return Some((*g).to_string());
    }
    known_gpui_key(hypr)
}

fn hypr_key(gpui: &str) -> Option<String> {
    if let Some((_, h)) = NAMED.iter().find(|(g, _)| *g == gpui) {
        return Some((*h).to_string());
    }
    known_gpui_key(gpui).map(|k| k.to_ascii_uppercase())
}

/// The Hyprland form of a GPUI key: `super-f10` gives `SUPER + F10`.
pub fn to_hypr(gpui: &str) -> Result<String, String> {
    let chord = parse_gpui(gpui)?;
    let key = hypr_key(&chord.key).ok_or_else(|| format!("the key {gpui:?} is not supported"))?;
    let mut parts: Vec<String> = MODS
        .iter()
        .filter(|(_, _, bit)| chord.mods & bit != 0)
        .map(|(_, h, _)| (*h).to_string())
        .collect();
    parts.push(key);
    Ok(parts.join(" + "))
}

/// The GPUI form of a Hyprland key: `SUPER + F10` gives `super-f10`.
pub fn from_hypr(hypr: &str) -> Result<String, String> {
    let parts: Vec<&str> = hypr.split('+').collect();
    Ok(gpui_form(&parse_parts(hypr, &parts, true)?))
}

fn gpui_form(chord: &Chord) -> String {
    let mut parts: Vec<&str> = [
        ("ctrl", CTRL),
        ("alt", ALT),
        ("shift", SHIFT),
        ("super", SUPER),
    ]
    .iter()
    .filter(|(_, bit)| chord.mods & bit != 0)
    .map(|(g, _)| *g)
    .collect();
    parts.push(&chord.key);
    parts.join("-")
}

/// One line of `hyprctl binds -j`.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Bind {
    pub key: String,
    pub modmask: u32,
    pub description: String,
    pub dispatcher: String,
    pub arg: String,
    pub mouse: bool,
}

impl Bind {
    /// A name for the owner: the description, else what the bind runs.
    pub fn describe(&self) -> String {
        if !self.description.is_empty() {
            return self.description.clone();
        }
        match (self.dispatcher.as_str(), self.arg.as_str()) {
            ("__lua", _) => "a Lua bind with no description".into(),
            (d, "") => d.to_string(),
            (d, a) => format!("{d} {a}"),
        }
    }

    fn on(&self, chord: &Chord) -> bool {
        !self.mouse
            && self.modmask & !LOCKS == chord.mods
            && gpui_key(&self.key).is_some_and(|k| k == chord.key)
    }

    /// `hyprctl binds -j` lists a `code:N` bind with an empty key, so the JSON cannot say which
    /// key it is. A digit key shares its codes with the number row: `SUPER + 1` switches the
    /// workspace. So an unnamed bind with the same modifiers holds a digit key. A letter has
    /// no such rule, because each `SUPER` letter would be refused.
    fn on_unnamed(&self, chord: &Chord) -> bool {
        !self.mouse
            && self.key.is_empty()
            && chord.is_digit()
            && self.modmask & !LOCKS == chord.mods
    }

    fn is_ours(&self) -> bool {
        [DESCRIPTION, DESCRIPTION_TOGGLE, DESCRIPTION_CONVERSE].contains(&self.description.as_str())
    }
}

/// The bind of someone else that holds `chord`.
fn holder<'a>(binds: &'a [Bind], chord: &Chord) -> Option<&'a Bind> {
    binds
        .iter()
        .find(|b| !b.is_ours() && (b.on(chord) || b.on_unnamed(chord)))
}

/// The part of `hyprctl` that this module uses. The tests use a fake, so no test changes a
/// real compositor.
pub trait Hyprctl {
    /// `hyprctl binds -j`. A read.
    fn binds(&self) -> Result<Vec<Bind>, String>;
    /// `hyprctl reload`. Omarchy moves the active workspace on each reload, so only the conf
    /// form uses it.
    fn reload(&self) -> Result<(), String>;
    /// `hyprctl eval <code>`: runs the Lua code in the Lua state of Hyprland.
    fn eval(&self, code: &str) -> Result<(), String>;
}

/// `hyprctl` of the session of this process.
pub struct RealHyprctl;

impl RealHyprctl {
    fn run(args: &[&str]) -> Result<String, String> {
        let out = Command::new("hyprctl")
            .args(args)
            .output()
            .map_err(|e| format!("cannot run hyprctl: {e}"))?;
        let text = String::from_utf8_lossy(&out.stdout).trim().to_string();
        if out.status.success() {
            Ok(text)
        } else {
            let err = String::from_utf8_lossy(&out.stderr);
            Err(format!("hyprctl {} failed: {} {err}", args.join(" "), text)
                .trim()
                .to_string())
        }
    }
}

impl Hyprctl for RealHyprctl {
    fn binds(&self) -> Result<Vec<Bind>, String> {
        parse_binds(&Self::run(&["binds", "-j"])?)
    }

    fn reload(&self) -> Result<(), String> {
        match Self::run(&["reload"])?.as_str() {
            "ok" | "" => Ok(()),
            other => Err(format!("hyprctl reload said: {other}")),
        }
    }

    fn eval(&self, code: &str) -> Result<(), String> {
        match Self::run(&["eval", code])?.as_str() {
            "ok" | "" => Ok(()),
            other => Err(format!("hyprctl eval said: {other}")),
        }
    }
}

/// The binds in the JSON of `hyprctl binds -j`.
pub fn parse_binds(json: &str) -> Result<Vec<Bind>, String> {
    let rows: Vec<serde_json::Value> =
        serde_json::from_str(json).map_err(|e| format!("hyprctl binds -j does not parse: {e}"))?;
    let text = |row: &serde_json::Value, name: &str| {
        row.get(name)
            .and_then(|v| v.as_str())
            .unwrap_or_default()
            .to_string()
    };
    Ok(rows
        .iter()
        .map(|row| Bind {
            key: text(row, "key"),
            modmask: row.get("modmask").and_then(|v| v.as_u64()).unwrap_or(0) as u32,
            description: text(row, "description"),
            dispatcher: text(row, "dispatcher"),
            arg: text(row, "arg"),
            mouse: row.get("mouse").and_then(|v| v.as_bool()).unwrap_or(false),
        })
        .collect())
}

/// What the dictation section shows.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum State {
    /// The compositor holds the bind.
    Active,
    /// The compositor does not hold the bind yet: the include line is not in the config.
    IncludeMissing,
    /// Another bind holds the key. The text names it.
    Taken(String),
    /// The compositor holds the bind, but the bind file differs from what AgentSI writes now.
    OutOfDate,
}

/// The state of the GPUI key `key` in the compositor.
pub fn state(hyprctl: &dyn Hyprctl, key: &str) -> Result<State, String> {
    let chord = parse_gpui(key)?;
    let binds = hyprctl.binds()?;
    if let Some(other) = holder(&binds, &chord) {
        return Ok(State::Taken(other.describe()));
    }
    Ok(if binds.iter().any(|b| b.on(&chord) && b.is_ours()) {
        State::Active
    } else {
        State::IncludeMissing
    })
}

/// Where AgentSI writes the bind file, and what it writes.
#[derive(Clone, Debug)]
pub struct Setup {
    pub form: Form,
    /// The folder `<config>/agentsi`.
    pub dir: PathBuf,
    pub core: CoreCall,
    /// How the key works: the form of the bind file.
    pub mode: Mode,
    /// The conversation key in GPUI form, when the bind file also binds it.
    pub converse: Option<String>,
}

#[derive(Debug, PartialEq, Eq)]
pub enum SaveError {
    /// The session has no config that AgentSI can write for.
    NoCompositor,
    BadKey(String),
    /// Another bind holds the key. The text names it.
    Taken(String),
    Io(String),
    /// `hyprctl` failed. The text says which call.
    Hyprctl(String),
    /// The file is written, and Hyprland did not apply it: `hyprctl eval` or `hyprctl reload`
    /// failed.
    Apply(String),
    /// A path has a line break, which a bind file cannot hold.
    BadPath(String),
}

impl fmt::Display for SaveError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NoCompositor => write!(f, "No Hyprland config was found. Use the two commands."),
            Self::BadKey(e) => write!(f, "{e}"),
            Self::Taken(by) => write!(f, "The key is taken by {by}. Pick another key."),
            Self::Io(e) => write!(f, "Cannot write the bind file: {e}"),
            Self::Hyprctl(e) => write!(f, "Cannot read the binds: {e}"),
            Self::Apply(e) => write!(
                f,
                "The bind file is saved, but Hyprland did not apply it: {e}"
            ),
            Self::BadPath(p) => write!(
                f,
                "The path {p:?} has a line break, which a bind file cannot hold. Use another path."
            ),
        }
    }
}

impl Setup {
    /// The bind file, or `None` when the form has none.
    pub fn bind_path(&self) -> Option<PathBuf> {
        let name = match self.form {
            Form::Lua => LUA_FILE,
            Form::Conf => CONF_FILE,
            Form::CommandsOnly => return None,
        };
        Some(self.dir.join(name))
    }

    /// Refuses a path that a bind file cannot hold: the bind file, the program or the socket.
    pub fn check_paths(&self) -> Result<(), SaveError> {
        let bind = self.bind_path().ok_or(SaveError::NoCompositor)?;
        let socket = self.core.socket.as_deref().map(Path::new);
        for path in [Some(bind.as_path()), Some(self.core.bin.as_path()), socket]
            .into_iter()
            .flatten()
        {
            if path.to_string_lossy().contains(['\n', '\r']) {
                return Err(SaveError::BadPath(path.to_string_lossy().into_owned()));
            }
        }
        Ok(())
    }

    /// The one line that the owner adds to the compositor config. `None` when there is no
    /// bind file or its path has a line break.
    pub fn include_line(&self) -> Option<String> {
        self.check_paths().ok()?;
        let path = self.bind_path()?;
        let path = path.to_string_lossy();
        // `pcall` keeps a missing or broken file from stopping the rest of the config.
        Some(match self.form {
            Form::Lua => format!("pcall(dofile, {})", lua_string(&path)),
            Form::Conf => format!("source = {}", hyprlang(&path)),
            Form::CommandsOnly => return None,
        })
    }

    /// Writes an empty bind file when there is none, so the include line works before the first
    /// key. It does not reload.
    pub fn ensure_file(&self) -> Result<(), SaveError> {
        self.check_paths()?;
        let path = self.bind_path().ok_or(SaveError::NoCompositor)?;
        if path.exists() {
            return Ok(());
        }
        crate::atomic::write(&path, &render_empty(self.form))
            .map_err(|e| SaveError::Io(e.to_string()))
    }

    /// The Hyprland form of the conversation key, when the setup has one that parses.
    fn converse_hypr(&self) -> Option<String> {
        self.converse.as_deref().and_then(|k| to_hypr(k).ok())
    }

    /// The bind file holds a dictation bind: it differs from the file with the conversation
    /// bind only.
    pub fn bound(&self) -> bool {
        let none = render_with(
            self.mode,
            self.form,
            None,
            self.converse_hypr().as_deref(),
            &self.core,
        );
        self.bind_path()
            .and_then(|p| std::fs::read_to_string(p).ok())
            .is_some_and(|text| text != none && text != render_empty(self.form))
    }

    /// The bind file holds a conversation bind that the setup does not list.
    pub fn converse_bound(&self) -> bool {
        self.bind_path()
            .and_then(|p| std::fs::read_to_string(p).ok())
            .is_some_and(|text| text.contains(DESCRIPTION_CONVERSE))
    }

    /// The bind file is what `save` writes now for the GPUI key `key`.
    fn is_current(&self, key: &str) -> bool {
        let (Some(path), Ok(hypr)) = (self.bind_path(), to_hypr(key)) else {
            return false;
        };
        std::fs::read_to_string(path).is_ok_and(|t| {
            t == render_with(
                self.mode,
                self.form,
                Some(&hypr),
                self.converse_hypr().as_deref(),
                &self.core,
            )
        })
    }

    /// The state of the GPUI key `key`. A bind that the compositor holds from a bind file with
    /// another program or socket than the current one is out of date.
    pub fn state(&self, hyprctl: &dyn Hyprctl, key: &str) -> Result<State, String> {
        Ok(match state(hyprctl, key)? {
            State::Active if !self.is_current(key) => State::OutOfDate,
            other => other,
        })
    }

    /// Makes the running Hyprland read the bind file. The Lua form runs `eval` of the file, and
    /// the conf form reloads, because `hyprctl keyword` cannot remove one bind of the owner.
    pub fn apply(&self, hyprctl: &dyn Hyprctl) -> Result<(), SaveError> {
        let applied = match self.form {
            Form::Lua => {
                let path = self.bind_path().ok_or(SaveError::NoCompositor)?;
                hyprctl.eval(&format!("dofile({})", lua_string(&path.to_string_lossy())))
            }
            Form::Conf => hyprctl.reload(),
            Form::CommandsOnly => return Err(SaveError::NoCompositor),
        };
        applied.map_err(SaveError::Apply)
    }

    /// Writes the bind file for the GPUI key `key`, then applies it. A key that another bind
    /// holds, a key that is not valid, or a path with a line break leaves the file as it was.
    pub fn save(&self, key: &str, hyprctl: &dyn Hyprctl) -> Result<(), SaveError> {
        self.check_paths()?;
        let path = self.bind_path().ok_or(SaveError::NoCompositor)?;
        let chord = parse_gpui(key).map_err(SaveError::BadKey)?;
        let ks = Keystroke::parse_with_os(key, OperatingSystem::Linux)
            .map_err(|e| SaveError::BadKey(e.to_string()))?;
        crate::keys::check_rules(&ks).map_err(|r| SaveError::BadKey(r.to_string()))?;
        let binds = hyprctl.binds().map_err(SaveError::Hyprctl)?;
        if let Some(other) = holder(&binds, &chord) {
            return Err(SaveError::Taken(other.describe()));
        }
        if self.converse_chord().is_some_and(|c| c == chord) {
            return Err(SaveError::Taken("the conversation key".into()));
        }
        let text = render_with(
            self.mode,
            self.form,
            Some(&to_hypr(key).map_err(SaveError::BadKey)?),
            self.converse_hypr().as_deref(),
            &self.core,
        );
        crate::atomic::write(&path, &text).map_err(|e| SaveError::Io(e.to_string()))?;
        self.apply(hyprctl)
    }

    fn converse_chord(&self) -> Option<Chord> {
        self.converse.as_deref().and_then(|k| parse_gpui(k).ok())
    }

    /// The Hyprland form of the dictation key `key` (GPUI form), or `None` for no key.
    fn dictation_hypr(key: Option<&str>) -> Result<Option<String>, SaveError> {
        key.map(|k| to_hypr(k).map_err(SaveError::BadKey))
            .transpose()
    }

    /// Writes the bind file for the GPUI conversation key `key`, with the dictation key
    /// `dictation` (GPUI form) that it also holds, then applies it. A key that another bind
    /// or the dictation key holds, or a key that is not valid, leaves the file as it was.
    pub fn save_converse(
        &self,
        key: &str,
        dictation: Option<&str>,
        hyprctl: &dyn Hyprctl,
    ) -> Result<(), SaveError> {
        self.check_paths()?;
        let path = self.bind_path().ok_or(SaveError::NoCompositor)?;
        let chord = parse_gpui(key).map_err(SaveError::BadKey)?;
        let ks = Keystroke::parse_with_os(key, OperatingSystem::Linux)
            .map_err(|e| SaveError::BadKey(e.to_string()))?;
        crate::keys::check_rules(&ks).map_err(|r| SaveError::BadKey(r.to_string()))?;
        let binds = hyprctl.binds().map_err(SaveError::Hyprctl)?;
        if let Some(other) = holder(&binds, &chord) {
            return Err(SaveError::Taken(other.describe()));
        }
        if dictation
            .and_then(|d| parse_gpui(d).ok())
            .is_some_and(|c| c == chord)
        {
            return Err(SaveError::Taken("the dictation key".into()));
        }
        let text = render_with(
            self.mode,
            self.form,
            Self::dictation_hypr(dictation)?.as_deref(),
            Some(&to_hypr(key).map_err(SaveError::BadKey)?),
            &self.core,
        );
        crate::atomic::write(&path, &text).map_err(|e| SaveError::Io(e.to_string()))?;
        self.apply(hyprctl)
    }

    /// Writes the bind file with the dictation key `dictation` (GPUI form) and no conversation
    /// bind, then applies it.
    pub fn remove_converse(
        &self,
        dictation: Option<&str>,
        hyprctl: &dyn Hyprctl,
    ) -> Result<(), SaveError> {
        self.check_paths()?;
        let path = self.bind_path().ok_or(SaveError::NoCompositor)?;
        let text = render_with(
            self.mode,
            self.form,
            Self::dictation_hypr(dictation)?.as_deref(),
            None,
            &self.core,
        );
        crate::atomic::write(&path, &text).map_err(|e| SaveError::Io(e.to_string()))?;
        self.apply(hyprctl)
    }

    /// The state of the GPUI conversation key `key` in the compositor. A bind from a bind file
    /// that differs from what AgentSI writes now, with the dictation key `dictation`, is out of
    /// date.
    pub fn converse_state(
        &self,
        hyprctl: &dyn Hyprctl,
        key: &str,
        dictation: Option<&str>,
    ) -> Result<State, String> {
        let current = (|| {
            let path = self.bind_path()?;
            let hypr = to_hypr(key).ok()?;
            let dictation = dictation.map(|d| to_hypr(d).ok()).unwrap_or(None);
            let text = render_with(
                self.mode,
                self.form,
                dictation.as_deref(),
                Some(&hypr),
                &self.core,
            );
            std::fs::read_to_string(path).ok().map(|t| t == text)
        })()
        .unwrap_or(false);
        Ok(match state(hyprctl, key)? {
            State::Active if !current => State::OutOfDate,
            other => other,
        })
    }

    /// Writes an empty but valid bind file, then applies it.
    pub fn remove(&self, hyprctl: &dyn Hyprctl) -> Result<(), SaveError> {
        self.check_paths()?;
        let path = self.bind_path().ok_or(SaveError::NoCompositor)?;
        let text = render_with(
            self.mode,
            self.form,
            None,
            self.converse_hypr().as_deref(),
            &self.core,
        );
        crate::atomic::write(&path, &text).map_err(|e| SaveError::Io(e.to_string()))?;
        self.apply(hyprctl)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::{Cell, RefCell};

    fn core(socket: Option<&str>) -> CoreCall {
        CoreCall {
            bin: "/opt/agentsi/agentsi-core".into(),
            socket: socket.map(String::from),
        }
    }

    struct Fake {
        binds: Vec<Bind>,
        fail_binds: bool,
        reloads: Cell<u32>,
        log: RefCell<Vec<String>>,
    }

    impl Fake {
        fn new(binds: Vec<Bind>) -> Fake {
            Fake {
                binds,
                fail_binds: false,
                reloads: Cell::new(0),
                log: RefCell::new(vec![]),
            }
        }
    }

    impl Hyprctl for Fake {
        fn binds(&self) -> Result<Vec<Bind>, String> {
            self.log.borrow_mut().push("binds".into());
            if self.fail_binds {
                Err("no session".into())
            } else {
                Ok(self.binds.clone())
            }
        }

        fn reload(&self) -> Result<(), String> {
            self.log.borrow_mut().push("reload".into());
            self.reloads.set(self.reloads.get() + 1);
            Ok(())
        }

        fn eval(&self, code: &str) -> Result<(), String> {
            self.log.borrow_mut().push(format!("eval {code}"));
            Ok(())
        }
    }

    impl Fake {
        /// The calls that change the compositor: `eval` and `reload`, in order.
        fn applies(&self) -> Vec<String> {
            self.log
                .borrow()
                .iter()
                .filter(|l| *l != "binds")
                .cloned()
                .collect()
        }
    }

    fn bind(key: &str, modmask: u32, description: &str) -> Bind {
        Bind {
            key: key.into(),
            modmask,
            description: description.into(),
            dispatcher: "__lua".into(),
            arg: "7".into(),
            mouse: false,
        }
    }

    fn scratch(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("agentsi-bind-{}-{name}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn setup(form: Form, dir: &Path) -> Setup {
        Setup {
            form,
            dir: dir.join("agentsi"),
            core: core(None),
            mode: Mode::Hold,
            converse: None,
        }
    }

    #[test]
    fn a_digit_key_is_taken_by_a_code_bind_with_the_same_modifiers() {
        // `hyprctl binds -j` lists a `code:N` bind with an empty key.
        let code_bind = Bind {
            key: String::new(),
            modmask: SUPER,
            description: String::new(),
            dispatcher: "workspace".into(),
            arg: "1".into(),
            mouse: false,
        };
        let fake = Fake::new(vec![code_bind]);
        assert_eq!(
            state(&fake, "super-1"),
            Ok(State::Taken("workspace 1".into()))
        );
        // Letters are not refused, or each `SUPER` key would be.
        assert_eq!(state(&fake, "super-v"), Ok(State::IncludeMissing));
        assert_eq!(state(&fake, "ctrl-1"), Ok(State::IncludeMissing));
        let dir = scratch("code-bind");
        let setup = setup(Form::Lua, &dir);
        assert_eq!(
            setup.save("super-1", &fake),
            Err(SaveError::Taken("workspace 1".into()))
        );
        assert!(!setup.bind_path().unwrap().exists());
        assert!(fake.applies().is_empty());
    }

    #[test]
    fn a_hash_in_a_path_is_escaped_in_the_conf_form() {
        let core = CoreCall {
            bin: "/opt/a#b/agentsi-core".into(),
            socket: Some("/run/x#y.sock".into()),
        };
        let text = render(Form::Conf, "SUPER + F10", &core);
        for line in text.lines().filter(|l| !l.starts_with('#')) {
            assert!(!line.replace("##", "").contains('#'), "{line}");
            assert!(line.contains("a##b") && line.contains("x##y"), "{line}");
        }
        let setup = Setup {
            form: Form::Conf,
            dir: "/tmp/c#d/agentsi".into(),
            core,
            mode: Mode::Hold,
            converse: None,
        };
        assert_eq!(
            setup.include_line().unwrap(),
            "source = /tmp/c##d/agentsi/hypr-dictation.conf"
        );
        // Lua has no comment character in a string.
        let lua = Setup {
            form: Form::Lua,
            ..setup
        };
        assert!(lua.include_line().unwrap().contains("c#d"));
    }

    #[test]
    fn a_path_with_a_line_break_is_refused_in_both_forms_before_any_write() {
        let bad = CoreCall {
            bin: "/opt/a\nb/agentsi-core".into(),
            socket: None,
        };
        for form in [Form::Lua, Form::Conf] {
            let dir = scratch("line-break");
            let setup = Setup {
                form,
                dir: dir.join("agentsi"),
                core: bad.clone(),
                mode: Mode::Hold,
                converse: None,
            };
            let fake = Fake::new(vec![]);
            let err = setup.save("super-f10", &fake).unwrap_err();
            assert!(err.to_string().contains("line break"), "{form:?}: {err}");
            assert!(setup.ensure_file().is_err());
            assert!(setup.remove(&fake).is_err());
            assert!(!setup.bind_path().unwrap().exists(), "{form:?}");
            assert!(fake.applies().is_empty());
            assert!(setup.include_line().is_none());
        }
        // A line break in the folder of the bind file counts too.
        let setup = Setup {
            form: Form::Lua,
            dir: "/tmp/a\nb".into(),
            core: core(None),
            mode: Mode::Hold,
            converse: None,
        };
        assert!(setup.save("super-f10", &Fake::new(vec![])).is_err());
    }

    #[test]
    fn a_bind_file_that_differs_from_what_render_gives_now_is_out_of_date() {
        let dir = scratch("stale");
        let mut setup = setup(Form::Lua, &dir);
        let fake = Fake::new(vec![bind("F10", SUPER, DESCRIPTION)]);
        setup.save("super-f10", &fake).unwrap();
        assert_eq!(setup.state(&fake, "super-f10"), Ok(State::Active));
        let old = setup.core.clone();
        // The program moved.
        setup.core.bin = "/opt/other/agentsi-core".into();
        assert_eq!(setup.state(&fake, "super-f10"), Ok(State::OutOfDate));
        // The socket changed.
        setup.core = CoreCall {
            socket: Some("/run/old.sock".into()),
            ..old.clone()
        };
        assert_eq!(setup.state(&fake, "super-f10"), Ok(State::OutOfDate));
        // A write with the current program makes it current again.
        setup.save("super-f10", &fake).unwrap();
        assert_eq!(setup.state(&fake, "super-f10"), Ok(State::Active));
        // A key that is not Active keeps its state.
        assert_eq!(
            setup.state(&Fake::new(vec![]), "super-f10"),
            Ok(State::IncludeMissing)
        );
    }

    #[test]
    fn lua_text_without_socket() {
        let text = render(Form::Lua, "SUPER + F10", &core(None));
        let expected = r#"-- Written by AgentSI. Change the key in the AgentSI settings. Edits are lost.
agentsi_dictation = agentsi_dictation or {}
do
  local state = agentsi_dictation
  if state.bind ~= nil and tostring(state.bind) ~= "HL.Keybind(expired)" then
    pcall(function() state.bind:remove() end)
  end
  if state.converse ~= nil and tostring(state.converse) ~= "HL.Keybind(expired)" then
    pcall(function() state.converse:remove() end)
  end
  if state.listener ~= nil then
    pcall(function() state.listener:remove() end)
  end
  state.bind, state.listener, state.converse = nil, nil, nil
  state.token = (state.token or 0) + 1
  local token = state.token
  local holding = false
  state.bind = hl.bind("SUPER + F10", function()
    if holding then return end
    holding = true
    hl.exec_cmd("'/opt/agentsi/agentsi-core' dictate start")
  end, { description = "AgentSI: hold to dictate" })
  state.listener = hl.on("input.keyboard.key", function(_, _, key_state)
    if state.token == token and key_state == 0 and holding then
      holding = false
      hl.exec_cmd("'/opt/agentsi/agentsi-core' dictate stop")
    end
  end)
end
"#;
        assert_eq!(text, expected);
    }

    #[test]
    fn lua_text_with_socket() {
        let text = render(
            Form::Lua,
            "CTRL + ALT + V",
            &core(Some("/run/user/1000/a b.sock")),
        );
        assert!(text.contains(
            r#"hl.exec_cmd("AGENTSI_CORE_SOCKET='/run/user/1000/a b.sock' '/opt/agentsi/agentsi-core' dictate start")"#
        ));
        assert!(text.contains(r#"hl.bind("CTRL + ALT + V", function()"#));
        assert!(text.contains(r#"dictate stop")"#));
    }

    #[test]
    fn conf_text_without_socket() {
        let text = render(Form::Conf, "SUPER + F10", &core(None));
        let expected = "# Written by AgentSI. Change the key in the AgentSI settings. Edits are lost.\n\
bindd = SUPER, F10, AgentSI: hold to dictate, exec, '/opt/agentsi/agentsi-core' dictate start\n\
bindrnid = SUPER, F10, AgentSI: hold to dictate, exec, '/opt/agentsi/agentsi-core' dictate stop\n";
        assert_eq!(text, expected);
    }

    #[test]
    fn conf_text_with_socket_and_two_modifiers() {
        let text = render(Form::Conf, "SUPER + SHIFT + D", &core(Some("/run/x.sock")));
        assert!(text.contains(
            "bindd = SUPER SHIFT, D, AgentSI: hold to dictate, exec, AGENTSI_CORE_SOCKET='/run/x.sock' '/opt/agentsi/agentsi-core' dictate start\n"
        ));
        assert!(text.contains("bindrnid = SUPER SHIFT, D, AgentSI: hold to dictate, exec, AGENTSI_CORE_SOCKET='/run/x.sock' '/opt/agentsi/agentsi-core' dictate stop\n"));
    }

    #[test]
    fn the_toggle_form_has_one_bind_and_no_release_in_lua() {
        let text = render_for(Mode::Toggle, Form::Lua, "SUPER + F10", &core(None));
        assert!(text.starts_with(LUA_HEAD), "{text}");
        assert!(text.ends_with(
            "  state.bind = hl.bind(\"SUPER + F10\", function()\n    hl.exec_cmd(\"'/opt/agentsi/agentsi-core' dictate toggle\")\n  end, { description = \"AgentSI: press to dictate\" })\nend\n"
        ), "{text}");
        // The head removes the bind and the listener of an older load, and the toggle form adds
        // no release listener, no start and no stop.
        assert!(!text.contains("hl.on") && !text.contains("holding"));
        assert!(!text.contains("dictate start") && !text.contains("dictate stop"));
    }

    #[test]
    fn the_toggle_form_has_one_bind_and_no_release_in_conf() {
        let text = render_for(
            Mode::Toggle,
            Form::Conf,
            "SUPER + SHIFT + D",
            &core(Some("/run/x.sock")),
        );
        assert_eq!(
            text,
            "# Written by AgentSI. Change the key in the AgentSI settings. Edits are lost.\n\
bindd = SUPER SHIFT, D, AgentSI: press to dictate, exec, AGENTSI_CORE_SOCKET='/run/x.sock' '/opt/agentsi/agentsi-core' dictate toggle\n"
        );
        assert!(!text.contains("bindr"));
        assert_eq!(
            render_for(Mode::Toggle, Form::CommandsOnly, "F10", &core(None)),
            ""
        );
        // The hold form is the form of before.
        assert_eq!(
            render_for(Mode::Hold, Form::Conf, "SUPER + F10", &core(None)),
            render(Form::Conf, "SUPER + F10", &core(None))
        );
    }

    #[test]
    fn a_bind_of_either_mode_is_ours_and_a_file_of_the_other_mode_is_out_of_date() {
        let ours = [DESCRIPTION, DESCRIPTION_TOGGLE].map(|d| bind("F10", SUPER, d));
        for b in &ours {
            assert_eq!(
                state(&Fake::new(vec![b.clone()]), "super-f10").unwrap(),
                State::Active
            );
        }
        let dir = scratch("mode-stale");
        let mut s = setup(Form::Lua, &dir);
        let fake = Fake::new(vec![]);
        s.mode = Mode::Toggle;
        s.save("super-f10", &fake).unwrap();
        let active = Fake::new(vec![bind("F10", SUPER, DESCRIPTION_TOGGLE)]);
        assert_eq!(s.state(&active, "super-f10").unwrap(), State::Active);
        // The owner changes the mode in keys.toml: the file on disk is the other form.
        s.mode = Mode::Hold;
        assert_eq!(s.state(&active, "super-f10").unwrap(), State::OutOfDate);
        s.save("super-f10", &fake).unwrap();
        assert!(
            !std::fs::read_to_string(s.bind_path().unwrap())
                .unwrap()
                .contains("toggle")
        );
    }

    #[test]
    fn modes_have_the_names_of_the_key_file() {
        assert_eq!(Mode::parse("hold"), Some(Mode::Hold));
        assert_eq!(Mode::parse("toggle"), Some(Mode::Toggle));
        assert_eq!(Mode::parse("Toggle"), None);
        assert_eq!(Mode::default(), Mode::Hold);
        for mode in [Mode::Hold, Mode::Toggle] {
            assert_eq!(Mode::parse(mode.as_str()), Some(mode));
        }
    }

    #[test]
    fn a_quote_in_the_path_is_escaped() {
        let call = CoreCall {
            bin: "/o'k/agentsi-core".into(),
            socket: None,
        };
        assert_eq!(
            call.command("start"),
            r#"'/o'\''k/agentsi-core' dictate start"#
        );
    }

    #[test]
    fn commands_for_a_compositor_without_a_bind_file() {
        let [start, stop] = core(Some("/s")).commands();
        assert_eq!(
            start,
            "AGENTSI_CORE_SOCKET='/s' '/opt/agentsi/agentsi-core' dictate start"
        );
        assert!(stop.ends_with("dictate stop"));
    }

    #[test]
    fn the_empty_conf_file_is_only_comments() {
        assert!(render_empty(Form::Conf).lines().all(|l| l.starts_with('#')));
    }

    #[test]
    fn the_empty_lua_file_removes_the_old_bind_and_binds_nothing() {
        let text = render_empty(Form::Lua);
        assert!(text.contains("state.bind:remove()"));
        assert!(text.contains("state.token = (state.token or 0) + 1"));
        assert!(!text.contains("hl.bind") && !text.contains("hl.on("));
        assert!(text.trim_end().ends_with("-- No dictation key is set."));
    }

    #[test]
    fn the_lua_file_is_safe_to_load_twice() {
        let text = render(Form::Lua, "SUPER + F10", &core(None));
        // One global table holds the state, so a second load finds the first.
        assert!(text.contains("agentsi_dictation = agentsi_dictation or {}"));
        // A bind is removed only while its handle is alive: a reload expires the handles.
        assert!(text.contains(r#"tostring(state.bind) ~= "HL.Keybind(expired)""#));
        // The listener of an older load does nothing.
        assert!(text.contains("state.token == token"));
        // No call removes the chord of the owner by key.
        assert!(!text.contains("hl.unbind"));
    }

    #[test]
    fn include_lines() {
        let dir = Path::new("/home/me/.config");
        assert_eq!(
            setup(Form::Lua, dir).include_line().unwrap(),
            r#"pcall(dofile, "/home/me/.config/agentsi/hypr-dictation.lua")"#
        );
        assert_eq!(
            setup(Form::Conf, dir).include_line().unwrap(),
            "source = /home/me/.config/agentsi/hypr-dictation.conf"
        );
        assert_eq!(setup(Form::CommandsOnly, dir).include_line(), None);
    }

    #[test]
    fn form_follows_the_config_folder_and_the_session() {
        let dir = scratch("detect");
        assert_eq!(detect(&dir, true), Form::CommandsOnly);
        std::fs::write(dir.join("hyprland.conf"), "").unwrap();
        assert_eq!(detect(&dir, true), Form::Conf);
        assert_eq!(detect(&dir, false), Form::CommandsOnly);
        std::fs::write(dir.join("hyprland.lua"), "").unwrap();
        assert_eq!(detect(&dir, true), Form::Lua);
        assert_eq!(detect(&dir, false), Form::CommandsOnly);
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn keys_convert_both_ways() {
        for (gpui, hypr) in [
            ("super-f10", "SUPER + F10"),
            ("ctrl-alt-v", "CTRL + ALT + V"),
            ("shift-super-d", "SUPER + SHIFT + D"),
            ("ctrl-space", "CTRL + SPACE"),
            ("alt-enter", "ALT + RETURN"),
            ("super-pageup", "SUPER + PRIOR"),
            ("ctrl-.", "CTRL + PERIOD"),
            ("ctrl--", "CTRL + MINUS"),
            ("f9", "F9"),
            ("ctrl-1", "CTRL + 1"),
        ] {
            assert_eq!(to_hypr(gpui).as_deref(), Ok(hypr), "{gpui}");
            assert_eq!(from_hypr(hypr).as_deref(), Ok(gpui), "{hypr}");
        }
    }

    #[test]
    fn modifier_order_is_free_on_input() {
        assert_eq!(to_hypr("shift-super-f10").unwrap(), "SUPER + SHIFT + F10");
        assert_eq!(from_hypr("shift+super+f10").unwrap(), "shift-super-f10");
        assert!(from_hypr("SUPER_L + V").is_err());
        assert_eq!(from_hypr("MOD4 + Page_Up").unwrap(), "super-pageup");
    }

    #[test]
    fn bad_keys_are_refused() {
        assert!(to_hypr("").is_err());
        assert!(to_hypr("ctrl-").is_err());
        assert!(to_hypr("ctrl").is_err());
        assert!(to_hypr("ctrl-a-b").is_err());
        assert!(to_hypr("ctrl-f36").is_err());
        assert!(to_hypr("ctrl-é").is_err());
    }

    #[test]
    fn three_states_come_from_a_fake_hyprctl() {
        let ours = bind("F10", SUPER, DESCRIPTION);
        let theirs = bind("F10", SUPER, "Omarchy menu");
        let other_key = bind("F9", SUPER, "Menu");

        let none = Fake::new(vec![other_key.clone()]);
        assert_eq!(state(&none, "super-f10"), Ok(State::IncludeMissing));

        let active = Fake::new(vec![other_key.clone(), ours.clone()]);
        assert_eq!(state(&active, "super-f10"), Ok(State::Active));

        let taken = Fake::new(vec![theirs, ours]);
        assert_eq!(
            state(&taken, "super-f10"),
            Ok(State::Taken("Omarchy menu".into()))
        );
        assert!(
            none.applies().is_empty() && active.applies().is_empty() && taken.applies().is_empty()
        );
    }

    #[test]
    fn the_match_ignores_case_locks_and_mouse_binds() {
        let lower = bind("f10", SUPER | 2 | 16, "x");
        let mouse = Bind {
            mouse: true,
            ..bind("F10", SUPER, "drag")
        };
        let shifted = bind("F10", SUPER | SHIFT, "y");
        let fake = Fake::new(vec![mouse, shifted]);
        assert_eq!(state(&fake, "super-f10"), Ok(State::IncludeMissing));
        let fake = Fake::new(vec![lower]);
        assert_eq!(state(&fake, "super-f10"), Ok(State::Taken("x".into())));
    }

    #[test]
    fn a_bind_with_no_description_is_named_by_its_command() {
        let exec = Bind {
            dispatcher: "exec".into(),
            arg: "firefox".into(),
            ..bind("F10", SUPER, "")
        };
        assert_eq!(
            state(&Fake::new(vec![exec]), "super-f10"),
            Ok(State::Taken("exec firefox".into()))
        );
        let lua = bind("F10", SUPER, "");
        assert_eq!(
            state(&Fake::new(vec![lua]), "super-f10"),
            Ok(State::Taken("a Lua bind with no description".into()))
        );
    }

    #[test]
    fn parses_the_json_of_hyprctl() {
        let json = r#"[
{"locked": false, "mouse": false, "release": false, "modmask": 64, "key": "SPACE",
 "description": "Omarchy menu", "dispatcher": "__lua", "arg": "263"},
{"mouse": true, "modmask": 64, "key": "mouse:272", "description": "", "dispatcher": "x", "arg": ""}
]"#;
        let binds = parse_binds(json).unwrap();
        assert_eq!(binds.len(), 2);
        assert_eq!(binds[0].key, "SPACE");
        assert_eq!(binds[0].modmask, 64);
        assert_eq!(binds[0].describe(), "Omarchy menu");
        assert!(binds[1].mouse);
        assert!(parse_binds("not json").is_err());
    }

    #[test]
    fn a_lua_save_writes_the_file_and_evals_it_with_no_reload() {
        let dir = scratch("save");
        let s = setup(Form::Lua, &dir);
        let fake = Fake::new(vec![bind("F9", SUPER, "Menu")]);
        s.save("super-f10", &fake).unwrap();
        let path = s.bind_path().unwrap();
        let written = std::fs::read_to_string(&path).unwrap();
        assert_eq!(written, render(Form::Lua, "SUPER + F10", &s.core));
        assert_eq!(fake.reloads.get(), 0);
        assert_eq!(
            *fake.log.borrow(),
            [
                "binds".to_string(),
                format!("eval dofile(\"{}\")", path.display())
            ]
        );
        assert!(!s.dir.join("hypr-dictation.lua.tmp").exists());
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn save_over_its_own_bind_is_allowed() {
        let dir = scratch("own");
        let s = setup(Form::Conf, &dir);
        let fake = Fake::new(vec![bind("F10", SUPER, DESCRIPTION)]);
        assert_eq!(s.save("super-f10", &fake), Ok(()));
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn a_taken_key_leaves_the_file_unchanged() {
        let dir = scratch("taken");
        let s = setup(Form::Lua, &dir);
        let before = render(Form::Lua, "SUPER + F9", &s.core);
        std::fs::create_dir_all(&s.dir).unwrap();
        std::fs::write(s.bind_path().unwrap(), &before).unwrap();
        let fake = Fake::new(vec![bind("F10", SUPER, "Omarchy menu")]);
        let err = s.save("super-f10", &fake).unwrap_err();
        assert_eq!(err, SaveError::Taken("Omarchy menu".into()));
        assert_eq!(
            err.to_string(),
            "The key is taken by Omarchy menu. Pick another key."
        );
        assert_eq!(
            std::fs::read_to_string(s.bind_path().unwrap()).unwrap(),
            before
        );
        assert!(fake.applies().is_empty());
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn a_taken_key_creates_no_file() {
        let dir = scratch("taken-new");
        let s = setup(Form::Lua, &dir);
        let fake = Fake::new(vec![bind("F10", SUPER, "Omarchy menu")]);
        assert!(s.save("super-f10", &fake).is_err());
        assert!(!s.dir.exists());
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn a_key_with_no_modifier_is_refused_unless_it_is_a_function_key() {
        let dir = scratch("bare");
        let s = setup(Form::Lua, &dir);
        let fake = Fake::new(vec![]);
        assert!(matches!(s.save("v", &fake), Err(SaveError::BadKey(_))));
        assert!(matches!(
            s.save("shift-v", &fake),
            Err(SaveError::BadKey(_))
        ));
        assert_eq!(s.save("f9", &fake), Ok(()));
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn unreadable_binds_leave_the_file_unchanged() {
        let dir = scratch("nobinds");
        let s = setup(Form::Lua, &dir);
        let mut fake = Fake::new(vec![]);
        fake.fail_binds = true;
        assert!(matches!(
            s.save("super-f10", &fake),
            Err(SaveError::Hyprctl(_))
        ));
        assert!(!s.dir.exists());
        assert!(fake.applies().is_empty());
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn a_conf_save_and_remove_reload_and_never_eval() {
        let dir = scratch("remove");
        let s = setup(Form::Conf, &dir);
        let fake = Fake::new(vec![]);
        s.save("super-f10", &fake).unwrap();
        s.remove(&fake).unwrap();
        let text = std::fs::read_to_string(s.bind_path().unwrap()).unwrap();
        assert_eq!(text, render_empty(Form::Conf));
        assert_eq!(fake.reloads.get(), 2);
        assert_eq!(fake.applies(), ["reload", "reload"]);
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn a_lua_remove_evals_the_empty_file_with_no_reload() {
        let dir = scratch("remove-lua");
        let s = setup(Form::Lua, &dir);
        let fake = Fake::new(vec![]);
        s.save("super-f10", &fake).unwrap();
        s.remove(&fake).unwrap();
        let path = s.bind_path().unwrap();
        assert_eq!(
            std::fs::read_to_string(&path).unwrap(),
            render_empty(Form::Lua)
        );
        let eval = format!("eval dofile(\"{}\")", path.display());
        assert_eq!(fake.applies(), [eval.clone(), eval]);
        assert_eq!(fake.reloads.get(), 0);
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn the_eval_names_a_path_with_a_quote_or_a_backslash_as_one_lua_string() {
        let dir = scratch("quote\"path");
        let s = setup(Form::Lua, &dir);
        let fake = Fake::new(vec![]);
        s.save("super-f10", &fake).unwrap();
        let log = fake.applies();
        assert!(log[0].starts_with("eval dofile(\"") && log[0].contains("quote\\\"path"));
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn ensure_file_keeps_an_existing_file() {
        let dir = scratch("ensure");
        let s = setup(Form::Lua, &dir);
        s.ensure_file().unwrap();
        let path = s.bind_path().unwrap();
        assert_eq!(
            std::fs::read_to_string(&path).unwrap(),
            render_empty(Form::Lua)
        );
        std::fs::write(&path, "kept").unwrap();
        s.ensure_file().unwrap();
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "kept");
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn no_form_means_nothing_to_write() {
        let dir = scratch("noform");
        let s = setup(Form::CommandsOnly, &dir);
        let fake = Fake::new(vec![]);
        assert_eq!(s.save("super-f10", &fake), Err(SaveError::NoCompositor));
        assert_eq!(s.remove(&fake), Err(SaveError::NoCompositor));
        assert!(fake.applies().is_empty());
        std::fs::remove_dir_all(&dir).unwrap();
    }

    /// Writes the bind files and the include lines to the folder in `AGENTSI_BIND_DUMP`, for the check in a real
    /// Hyprland. It never runs `hyprctl`. Run it with `--ignored`.
    #[test]
    #[ignore = "writes files for the manual check in a nested Hyprland"]
    fn dump_bind_files() {
        let Some(dir) = std::env::var_os("AGENTSI_BIND_DUMP").map(PathBuf::from) else {
            return;
        };
        let call = CoreCall::from_env().unwrap_or_else(|| core(None));
        let key = std::env::var("AGENTSI_BIND_KEY").unwrap_or("super-f10".into());
        for (form, name, include) in [
            (Form::Lua, LUA_FILE, "include.lua.txt"),
            (Form::Conf, CONF_FILE, "include.conf.txt"),
        ] {
            let s = Setup {
                form,
                dir: dir.clone(),
                core: call.clone(),
                mode: Mode::Hold,
                converse: None,
            };
            let mode = std::env::var("AGENTSI_BIND_MODE")
                .ok()
                .and_then(|m| Mode::parse(&m))
                .unwrap_or_default();
            // `AGENTSI_BIND_CONVERSE_KEY` adds the conversation bind. `AGENTSI_BIND_KEY=none`
            // leaves out the dictation bind.
            let converse = std::env::var("AGENTSI_BIND_CONVERSE_KEY")
                .ok()
                .map(|k| to_hypr(&k).unwrap());
            let dictation = (key != "none").then(|| to_hypr(&key).unwrap());
            let text = if std::env::var_os("AGENTSI_BIND_EMPTY").is_some() {
                render_empty(form)
            } else {
                render_with(mode, form, dictation.as_deref(), converse.as_deref(), &call)
            };
            std::fs::write(dir.join(name), text).unwrap();
            std::fs::write(dir.join(include), s.include_line().unwrap() + "\n").unwrap();
        }
    }

    /// Reads a saved `hyprctl binds -j` from `AGENTSI_BINDS_JSON` and prints the state of the
    /// key in `AGENTSI_BIND_KEY`. It never runs `hyprctl`.
    #[test]
    #[ignore = "reads a saved binds file for the manual check in a nested Hyprland"]
    fn state_from_saved_json() {
        let (Some(file), Ok(key)) = (
            std::env::var_os("AGENTSI_BINDS_JSON"),
            std::env::var("AGENTSI_BIND_KEY"),
        ) else {
            return;
        };
        struct Saved(Vec<Bind>);
        impl Hyprctl for Saved {
            fn binds(&self) -> Result<Vec<Bind>, String> {
                Ok(self.0.clone())
            }
            fn reload(&self) -> Result<(), String> {
                Err("not in this test".into())
            }
            fn eval(&self, _: &str) -> Result<(), String> {
                Err("not in this test".into())
            }
        }
        let binds = parse_binds(&std::fs::read_to_string(file).unwrap()).unwrap();
        println!("STATE {:?}", state(&Saved(binds), &key));
    }

    #[test]
    fn the_conversation_bind_sits_beside_the_dictation_bind_in_both_forms() {
        let call = core(None);
        for mode in [Mode::Hold, Mode::Toggle] {
            let lua = render_with(
                mode,
                Form::Lua,
                Some("SUPER + F10"),
                Some("SUPER + F9"),
                &call,
            );
            assert!(lua.starts_with(LUA_HEAD));
            assert!(
                lua.contains("state.bind = hl.bind(\"SUPER + F10\""),
                "{lua}"
            );
            assert!(
                lua.contains("state.converse = hl.bind(\"SUPER + F9\""),
                "{lua}"
            );
            assert!(
                lua.contains("hl.exec_cmd(\"'/opt/agentsi/agentsi-core' converse toggle\")"),
                "{lua}"
            );
            assert!(lua.contains("AgentSI: press to talk to a pane"));
            assert!(lua.ends_with("end\n"));
            let conf = render_with(
                mode,
                Form::Conf,
                Some("SUPER + F10"),
                Some("SUPER + F9"),
                &call,
            );
            assert!(
                conf.contains(
                    "bindd = SUPER, F9, AgentSI: press to talk to a pane, exec, '/opt/agentsi/agentsi-core' converse toggle\n"
                ),
                "{conf}"
            );
        }
        // A file with the conversation key alone binds only that key, and it is not "empty".
        let lua = render_with(Mode::Hold, Form::Lua, None, Some("SUPER + F9"), &call);
        assert!(!lua.contains("state.bind = hl.bind"), "{lua}");
        assert!(lua.contains("state.converse = hl.bind"), "{lua}");
        assert!(!lua.contains("No dictation key"), "{lua}");
        assert_ne!(lua, render_empty(Form::Lua));
        // No key leaves the file that it always left.
        assert_eq!(
            render_with(Mode::Hold, Form::Lua, None, None, &call),
            render_empty(Form::Lua)
        );
        assert!(render_empty(Form::Lua).ends_with("end\n-- No dictation key is set.\n"));
    }

    #[test]
    fn the_head_of_the_lua_file_removes_both_handles_of_an_older_load() {
        assert!(LUA_HEAD.contains("pcall(function() state.bind:remove() end)"));
        assert!(LUA_HEAD.contains("pcall(function() state.converse:remove() end)"));
        assert!(LUA_HEAD.contains("state.bind, state.listener, state.converse = nil, nil, nil"));
    }

    #[test]
    fn a_conversation_key_is_saved_removed_and_read_with_the_dictation_key_kept() {
        let dir = scratch("converse");
        let setup = setup(Form::Lua, &dir);
        let fake = Fake::new(vec![]);
        setup.save("super-f10", &fake).unwrap();
        setup
            .save_converse("super-f9", Some("super-f10"), &fake)
            .unwrap();
        let text = std::fs::read_to_string(setup.bind_path().unwrap()).unwrap();
        assert_eq!(
            text,
            render_with(
                Mode::Hold,
                Form::Lua,
                Some("SUPER + F10"),
                Some("SUPER + F9"),
                &setup.core
            )
        );
        assert!(setup.converse_bound());
        // The setup that knows the key sees a current file. Without it, the file is out of date.
        let known = Setup {
            converse: Some("super-f9".into()),
            ..setup.clone()
        };
        let both = vec![
            bind("F10", SUPER, DESCRIPTION),
            bind("F9", SUPER, DESCRIPTION_CONVERSE),
        ];
        let active = Fake::new(both);
        assert_eq!(
            known.converse_state(&active, "super-f9", Some("super-f10")),
            Ok(State::Active)
        );
        assert_eq!(known.state(&active, "super-f10"), Ok(State::Active));
        assert_eq!(
            setup.state(&active, "super-f10"),
            Ok(State::OutOfDate),
            "a dictation save with no conversation key would drop that bind"
        );
        // A save of the dictation key from a setup that knows the conversation key keeps it.
        known.save("super-f11", &fake).unwrap();
        let text = std::fs::read_to_string(known.bind_path().unwrap()).unwrap();
        assert!(
            text.contains("SUPER + F11") && text.contains("SUPER + F9"),
            "{text}"
        );
        assert!(!text.contains("SUPER + F10"));
        // The two keys differ.
        let e = known.save("super-f9", &fake).unwrap_err();
        assert_eq!(e, SaveError::Taken("the conversation key".into()));
        let e = known
            .save_converse("super-f11", Some("super-f11"), &fake)
            .unwrap_err();
        assert_eq!(e, SaveError::Taken("the dictation key".into()));
        // A key of another bind is refused.
        let foreign = Fake::new(vec![bind("F7", SUPER, "Launch the foreign tool")]);
        let e = known
            .save_converse("super-f7", Some("super-f11"), &foreign)
            .unwrap_err();
        assert_eq!(e, SaveError::Taken("Launch the foreign tool".into()));
        // The remove drops the conversation bind and keeps the dictation bind. The dictation
        // remove keeps the conversation bind.
        known.remove_converse(Some("super-f11"), &fake).unwrap();
        let text = std::fs::read_to_string(known.bind_path().unwrap()).unwrap();
        assert!(
            text.contains("SUPER + F11") && !text.contains("converse toggle"),
            "{text}"
        );
        assert!(!known.converse_bound());
        known
            .save_converse("super-f9", Some("super-f11"), &fake)
            .unwrap();
        known.remove(&fake).unwrap();
        let text = std::fs::read_to_string(known.bind_path().unwrap()).unwrap();
        assert!(
            text.contains("SUPER + F9") && !text.contains("F11"),
            "{text}"
        );
        assert!(!known.bound(), "no dictation bind is left");
    }
}
