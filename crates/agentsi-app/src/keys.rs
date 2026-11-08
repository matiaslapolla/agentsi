//! `<config>/agentsi/keys.toml`: the keys of the app that differ from the defaults, and the
//! dictation key.
//!
//! ```toml
//! [app]
//! new-terminal = "ctrl-alt-y"     # an id of `actions::KEY_ACTIONS`, a key in GPUI form
//!
//! [dictation]
//! key = "SUPER + F10"             # Hyprland form: `dictation_bind` writes the bind file from it
//! mode = "toggle"                 # "hold" (default) or "toggle": press to start, press to stop
//!
//! [conversation]
//! key = "SUPER + F9"              # Hyprland form: one press starts conversation mode, the next stops it
//! ```
//!
//! - A missing file means the defaults. An unknown id or a refused key is skipped, with one
//!   line in [`Keys::problems`]. The app starts anyway.
//! - The app writes the file in one step and keeps the other entries and the comments.
//! - [`change`] and [`reset`] save one key and bind it at once, with no restart.
//! - [`capture`] waits for the next key press. The settings screen uses it for the app keys, and
//!   the dictation section uses it with its own rules.

use std::collections::{BTreeMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::Mutex;

use agentsi_ui::components::{Kbd, key_hint_for};
use gpui_kit::{App, Context, Global, KeyBinding, Keystroke, Subscription, WeakEntity, Window};
use toml_edit::{DocumentMut, Item, value};

use crate::actions::{BINDINGS, KEY_ACTIONS};
use crate::dictation_bind::Mode;

pub use gpui_kit::OperatingSystem;

/// The id of the key that opens the command palette.
pub const PALETTE: &str = "palette";

pub const FILE_NAME: &str = "keys.toml";

/// `<config>/agentsi/keys.toml`.
pub fn path() -> Option<PathBuf> {
    crate::layout::path().map(|p| p.with_file_name(FILE_NAME))
}

/// What the file holds, before any rule applies.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct KeyFile {
    /// The key of each action in `[app]`, by id.
    pub app: BTreeMap<String, String>,
    /// `key` of `[dictation]`.
    pub dictation: Option<String>,
    /// `mode` of `[dictation]`.
    pub dictation_mode: Option<Mode>,
    /// `key` of `[conversation]`.
    pub conversation: Option<String>,
}

/// The threads that write the file go one at a time.
static WRITE: Mutex<()> = Mutex::new(());

impl KeyFile {
    /// The file in `path`, with a line for each entry that cannot be used. A missing file is
    /// empty. A file that does not parse is an error with the reason.
    pub fn read(path: &Path) -> Result<(KeyFile, Vec<String>), String> {
        let text = match std::fs::read_to_string(path) {
            Ok(t) => t,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                return Ok((KeyFile::default(), Vec::new()));
            }
            Err(e) => return Err(format!("cannot read {}: {e}", path.display())),
        };
        Self::parse(&text).map_err(|e| format!("{} does not parse: {e}", path.display()))
    }

    pub fn parse(text: &str) -> Result<(KeyFile, Vec<String>), String> {
        let doc: DocumentMut = text
            .parse()
            .map_err(|e: toml_edit::TomlError| e.message().to_string())?;
        let (mut file, mut problems) = (KeyFile::default(), Vec::new());
        match doc.get("app") {
            None => {}
            Some(Item::Table(t)) => {
                for (id, item) in t.iter() {
                    if !KEY_ACTIONS.iter().any(|a| a.id == id) {
                        problems.push(format!("[app] {id}: no action has this id. Skipped."));
                    } else if let Some(keys) = item.as_str() {
                        file.app.insert(id.to_string(), keys.to_string());
                    } else {
                        problems.push(format!("[app] {id}: the key must be text. Skipped."));
                    }
                }
            }
            Some(_) => problems.push("[app] must be a table. Skipped.".into()),
        }
        match doc.get("dictation").and_then(Item::as_table) {
            Some(t) => {
                match t.get("key") {
                    None => {}
                    Some(k) => match k.as_str() {
                        Some(s) => file.dictation = Some(s.to_string()),
                        None => {
                            problems.push("[dictation] key: the key must be text. Skipped.".into())
                        }
                    },
                }
                match t.get("mode") {
                    None => {}
                    Some(m) => match m.as_str().and_then(Mode::parse) {
                        Some(mode) => file.dictation_mode = Some(mode),
                        None => problems
                            .push("[dictation] mode: use \"hold\" or \"toggle\". Skipped.".into()),
                    },
                }
            }
            None if doc.contains_key("dictation") => {
                problems.push("[dictation] must be a table. Skipped.".into());
            }
            None => {}
        }
        match doc.get("conversation").and_then(Item::as_table) {
            Some(t) => {
                if let Some(k) = t.get("key") {
                    match k.as_str() {
                        Some(s) => file.conversation = Some(s.to_string()),
                        None => problems
                            .push("[conversation] key: the key must be text. Skipped.".into()),
                    }
                }
            }
            None if doc.contains_key("conversation") => {
                problems.push("[conversation] must be a table. Skipped.".into());
            }
            None => {}
        }
        Ok((file, problems))
    }

    /// Writes the key of action `id` into `[app]`, or removes the entry when `keys` is `None`.
    pub fn write_app(path: &Path, id: &str, keys: Option<&str>) -> Result<(), String> {
        edit(path, |doc| set_entry(doc, "app", id, keys))
    }

    /// Writes `mode` of `[dictation]`. The default, hold, removes the entry.
    pub fn write_dictation_mode(path: &Path, mode: Mode) -> Result<(), String> {
        let text = (mode != Mode::Hold).then(|| mode.as_str());
        edit(path, |doc| set_entry(doc, "dictation", "mode", text))
    }

    /// Writes `key` of `[dictation]`, or removes it when `key` is `None`.
    pub fn write_dictation(path: &Path, key: Option<&str>) -> Result<(), String> {
        edit(path, |doc| set_entry(doc, "dictation", "key", key))
    }

    /// Writes `key` of `[conversation]`, or removes it when `key` is `None`.
    pub fn write_conversation(path: &Path, key: Option<&str>) -> Result<(), String> {
        edit(path, |doc| set_entry(doc, "conversation", "key", key))
    }
}

/// Sets `table.name` to `text`, or removes it. An emptied table goes away too.
fn set_entry(
    doc: &mut DocumentMut,
    table: &str,
    name: &str,
    text: Option<&str>,
) -> Result<(), String> {
    match text {
        Some(text) => {
            if !doc.contains_key(table) {
                doc[table] = toml_edit::table();
            }
            let t = doc[table]
                .as_table_mut()
                .ok_or_else(|| format!("[{table}] is not a table. Fix it by hand."))?;
            t.insert(name, value(text));
        }
        None => {
            let Some(t) = doc.get_mut(table).and_then(Item::as_table_mut) else {
                return Ok(());
            };
            t.remove(name);
            if t.is_empty() {
                doc.remove(table);
            }
        }
    }
    Ok(())
}

pub(crate) fn edit(
    path: &Path,
    f: impl FnOnce(&mut DocumentMut) -> Result<(), String>,
) -> Result<(), String> {
    let _one = WRITE.lock().unwrap_or_else(|p| p.into_inner());
    let text = match std::fs::read_to_string(path) {
        Ok(t) => t,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => String::new(),
        Err(e) => return Err(format!("cannot read {}: {e}", path.display())),
    };
    let mut doc: DocumentMut = text.parse().map_err(|e: toml_edit::TomlError| {
        format!(
            "{} does not parse: {}. Fix it by hand first.",
            path.display(),
            e.message()
        )
    })?;
    f(&mut doc)?;
    crate::atomic::write(path, &doc.to_string())
        .map_err(|e| format!("cannot write {}: {e}", path.display()))
}

/// Why the editor refuses a key. `Display` gives the one line for the owner.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Refusal {
    /// A key with no `Ctrl`, `Alt` or `Super`, and no function key.
    NoModifier { key: String },
    /// A key that the terminal pane keeps: copy, paste and scrolling.
    PaneKey { key: String },
    /// `Ctrl` with one letter and no other modifier: terminal programs read these keys.
    ProgramKey { key: String },
    /// A key that another action has.
    Taken { key: String, label: &'static str },
    /// A text that GPUI cannot read as one key.
    NotAKey { text: String },
}

impl std::fmt::Display for Refusal {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let command = if cfg!(target_os = "macos") {
            "Cmd"
        } else {
            "Super"
        };
        match self {
            Refusal::NoModifier { key } => write!(
                f,
                "{key} needs Ctrl, Alt or {command}. Only a function key can stand alone, because the terminal programs need the other keys."
            ),
            Refusal::PaneKey { key } => write!(
                f,
                "{key} is a key of the terminal pane (copy, paste and scroll)."
            ),
            Refusal::ProgramKey { key } => write!(
                f,
                "{key} is a key of terminal programs (interrupt, end of input, delete word and more). Add Shift or Alt."
            ),
            Refusal::Taken { key, label } => {
                write!(f, "{key} is already the key of \"{label}\".")
            }
            Refusal::NotAKey { text } => write!(f, "\"{text}\" is not one key."),
        }
    }
}

/// How the owner reads a key: `Ctrl+Shift+Y`.
pub fn display(ks: &Keystroke) -> String {
    Kbd::format(ks)
}

/// `f1` to `f35`: a key that stands alone.
pub fn is_function_key(key: &str) -> bool {
    key.strip_prefix('f')
        .and_then(|n| n.parse::<u8>().ok())
        .is_some_and(|n| (1..=35).contains(&n))
}

/// The rules that need no other key: a modifier, and not a key of the pane or of the programs.
/// The default of an action is exempt for that action (`Ctrl+K` and `Ctrl+T` are defaults).
pub fn check_rules(ks: &Keystroke) -> Result<(), Refusal> {
    let key = display(ks);
    if ks.is_modifier_key() {
        return Err(Refusal::NotAKey { text: key });
    }
    if agentsi_term_view::shortcut(ks).is_some() {
        return Err(Refusal::PaneKey { key });
    }
    let m = &ks.modifiers;
    let only_ctrl = m.control && !(m.alt || m.shift || m.platform);
    if only_ctrl && ks.key.len() == 1 && ks.key.as_bytes()[0].is_ascii_alphabetic() {
        return Err(Refusal::ProgramKey { key });
    }
    if !(m.control || m.alt || m.platform || is_function_key(&ks.key)) {
        return Err(Refusal::NoModifier { key });
    }
    Ok(())
}

fn same(a: &Keystroke, b: &Keystroke) -> bool {
    a.modifiers == b.modifiers && a.key == b.key
}

/// `text` as one key. `secondary` is accepted, as in `BINDINGS`.
pub fn parse_key(text: &str, os: OperatingSystem) -> Result<Keystroke, Refusal> {
    let text = text.trim();
    let not_a_key = || Refusal::NotAKey { text: text.into() };
    if text.is_empty() || text.contains(char::is_whitespace) {
        return Err(not_a_key());
    }
    let ks = Keystroke::parse_with_os(text, os).map_err(|_| not_a_key())?;
    if ks.is_modifier_key() || ks.key.is_empty() {
        return Err(not_a_key());
    }
    Ok(ks)
}

/// The text of a key for the file: GPUI form, with the modifiers of this system.
pub fn key_text(ks: &Keystroke, os: OperatingSystem) -> String {
    ks.unparse(os)
}

/// One row of the settings screen.
#[derive(Debug, Clone, PartialEq)]
pub struct Row {
    pub id: &'static str,
    pub label: &'static str,
    pub keystroke: Keystroke,
    /// The key differs from the default.
    pub changed: bool,
}

/// The keys in effect.
pub struct Keys {
    path: Option<PathBuf>,
    os: OperatingSystem,
    /// The keys of the file that passed each rule, by id.
    overrides: BTreeMap<&'static str, Keystroke>,
    dictation: Option<String>,
    dictation_mode: Mode,
    conversation: Option<String>,
    problems: Vec<String>,
}

impl Global for Keys {}

impl Keys {
    /// The keys from the file at `path`. With no path or no file, they are the defaults.
    pub fn load(path: Option<PathBuf>, os: OperatingSystem) -> Keys {
        let mut problems = Vec::new();
        let file = match path.as_deref().map(KeyFile::read) {
            None => KeyFile::default(),
            Some(Ok((file, mut found))) => {
                problems.append(&mut found);
                file
            }
            Some(Err(e)) => {
                problems.push(format!("{e}. The default keys stay."));
                KeyFile::default()
            }
        };
        let mut keys = Keys {
            path,
            os,
            overrides: BTreeMap::new(),
            dictation: file.dictation,
            dictation_mode: file.dictation_mode.unwrap_or_default(),
            conversation: file.conversation,
            problems,
        };
        for (i, a) in KEY_ACTIONS.iter().enumerate() {
            let Some(text) = file.app.get(a.id) else {
                continue;
            };
            let default = keys.default_of(i);
            let key = parse_key(text, os).and_then(|ks| {
                if same(&ks, &default) {
                    return Ok(ks);
                }
                check_rules(&ks).map(|()| ks)
            });
            match key {
                Ok(ks) => {
                    keys.overrides.insert(a.id, ks);
                }
                Err(why) => keys
                    .problems
                    .push(format!("[app] {}: {why} Skipped.", a.id)),
            }
        }
        // A key that two actions share would run the first only. The file loses to the default
        // and to the actions before it. Two swapped keys are no conflict.
        while let Some((i, other)) = keys.conflict() {
            keys.problems.push(format!(
                "[app] {}: {} is already the key of \"{}\". Skipped.",
                KEY_ACTIONS[i].id,
                display(&keys.effective(i)),
                KEY_ACTIONS[other].label
            ));
            keys.overrides.remove(KEY_ACTIONS[i].id);
        }
        keys
    }

    /// The last action of the file that has the key of another action, and that other action.
    fn conflict(&self) -> Option<(usize, usize)> {
        (0..KEY_ACTIONS.len())
            .rev()
            .filter(|i| self.overrides.contains_key(KEY_ACTIONS[*i].id))
            .find_map(|i| {
                let mine = self.effective(i);
                (0..KEY_ACTIONS.len())
                    .find(|j| *j != i && same(&self.effective(*j), &mine))
                    .map(|j| (i, j))
            })
    }

    fn default_of(&self, i: usize) -> Keystroke {
        Keystroke::parse_with_os(BINDINGS[i].0, self.os).expect("a valid default key")
    }

    fn effective(&self, i: usize) -> Keystroke {
        self.overrides
            .get(KEY_ACTIONS[i].id)
            .cloned()
            .unwrap_or_else(|| self.default_of(i))
    }

    fn index(id: &str) -> usize {
        KEY_ACTIONS
            .iter()
            .position(|a| a.id == id)
            .unwrap_or_else(|| panic!("no key action {id:?}"))
    }

    /// The key in effect for action `id`.
    pub fn keystroke(&self, id: &str) -> Keystroke {
        self.effective(Self::index(id))
    }

    /// Each action with its key, in the order of `BINDINGS`.
    pub fn rows(&self) -> Vec<Row> {
        (0..KEY_ACTIONS.len())
            .map(|i| Row {
                id: KEY_ACTIONS[i].id,
                label: KEY_ACTIONS[i].label,
                keystroke: self.effective(i),
                changed: self.overrides.contains_key(KEY_ACTIONS[i].id),
            })
            .collect()
    }

    /// One line for each entry of the file that was skipped.
    pub fn problems(&self) -> &[String] {
        &self.problems
    }

    /// `key` of `[dictation]`, as the file holds it.
    pub fn dictation(&self) -> Option<&str> {
        self.dictation.as_deref()
    }

    /// `mode` of `[dictation]`. Hold when the file does not set it.
    pub fn dictation_mode(&self) -> Mode {
        self.dictation_mode
    }

    /// `key` of `[conversation]`, as the file holds it.
    pub fn conversation(&self) -> Option<&str> {
        self.conversation.as_deref()
    }

    pub fn path(&self) -> Option<&Path> {
        self.path.as_deref()
    }

    /// The key of the dictation bind, when the file has a valid one.
    fn dictation_keystroke(&self) -> Option<Keystroke> {
        let gpui = crate::dictation_bind::from_hypr(self.dictation.as_deref()?).ok()?;
        parse_key(&gpui, self.os).ok()
    }

    /// The key of the conversation bind, when the file has a valid one.
    fn conversation_keystroke(&self) -> Option<Keystroke> {
        let gpui = crate::dictation_bind::from_hypr(self.conversation.as_deref()?).ok()?;
        parse_key(&gpui, self.os).ok()
    }

    /// The label of the action, the dictation key or the conversation key that holds `ks`.
    /// `skip` is an action that does not count.
    fn holder(&self, ks: &Keystroke, skip: Option<usize>) -> Option<&'static str> {
        let action = (0..KEY_ACTIONS.len())
            .find(|j| Some(*j) != skip && same(&self.effective(*j), ks))
            .map(|j| KEY_ACTIONS[j].label);
        action
            .or_else(|| {
                self.dictation_keystroke()
                    .is_some_and(|d| same(&d, ks))
                    .then_some(DICTATION_LABEL)
            })
            .or_else(|| {
                self.conversation_keystroke()
                    .is_some_and(|d| same(&d, ks))
                    .then_some(CONVERSATION_LABEL)
            })
    }

    /// Whether `ks` may be the key of action `id`: the keys of the other actions and of the
    /// dictation, then the rules. The default of the action is exempt from the rules.
    pub fn check(&self, id: &str, ks: &Keystroke) -> Result<(), Refusal> {
        let me = Self::index(id);
        if let Some(label) = self.holder(ks, Some(me)) {
            return Err(Refusal::Taken {
                key: display(ks),
                label,
            });
        }
        if same(ks, &self.default_of(me)) {
            return Ok(());
        }
        check_rules(ks)
    }

    /// Whether `ks` may be the dictation key. The compositor takes the key from every window,
    /// so no action of the app may hold it, and the rules of the app keys apply.
    pub fn check_dictation(&self, ks: &Keystroke) -> Result<(), Refusal> {
        if let Some(j) = (0..KEY_ACTIONS.len()).find(|j| same(&self.effective(*j), ks)) {
            return Err(Refusal::Taken {
                key: display(ks),
                label: KEY_ACTIONS[j].label,
            });
        }
        if self.conversation_keystroke().is_some_and(|c| same(&c, ks)) {
            return Err(Refusal::Taken {
                key: display(ks),
                label: CONVERSATION_LABEL,
            });
        }
        check_rules(ks)
    }

    /// Whether `ks` may be the conversation key: the same rules as the dictation key, and it
    /// differs from the dictation key.
    pub fn check_conversation(&self, ks: &Keystroke) -> Result<(), Refusal> {
        if let Some(j) = (0..KEY_ACTIONS.len()).find(|j| same(&self.effective(*j), ks)) {
            return Err(Refusal::Taken {
                key: display(ks),
                label: KEY_ACTIONS[j].label,
            });
        }
        if self.dictation_keystroke().is_some_and(|d| same(&d, ks)) {
            return Err(Refusal::Taken {
                key: display(ks),
                label: DICTATION_LABEL,
            });
        }
        check_rules(ks)
    }
}

/// The name of the dictation key in a refusal.
const DICTATION_LABEL: &str = "Dictation key";
/// The name of the conversation key in a refusal.
const CONVERSATION_LABEL: &str = "Conversation key";

/// Reads `path` and binds the keys in effect. The app calls this once, at start.
pub fn install(cx: &mut App, path: Option<PathBuf>) {
    let keys = Keys::load(path, cx.platform_info().os);
    cx.set_global(keys);
    apply(cx);
}

/// Runs `f` on the keys in effect. Before `install`, they are the defaults.
pub fn with<R>(cx: &App, f: impl FnOnce(&Keys) -> R) -> R {
    match cx.try_global::<Keys>() {
        Some(keys) => f(keys),
        None => f(&Keys::load(None, cx.platform_info().os)),
    }
}

/// The key in effect for action `id`.
pub fn keystroke(cx: &App, id: &str) -> Keystroke {
    with(cx, |k| k.keystroke(id))
}

/// The key in effect for action `id`, as the owner reads it: `Ctrl+K`.
pub fn text(cx: &App, id: &str) -> String {
    display(&keystroke(cx, id))
}

/// The key hint of action `id`.
pub fn hint(cx: &App, id: &str) -> Kbd {
    hint_for(&keystroke(cx, id))
}

/// The key hint of a key.
pub fn hint_for(ks: &Keystroke) -> Kbd {
    key_hint_for(ks.clone())
}

/// Replaces the bindings of the app with the keys in effect. The bindings of the components
/// stay, because the keymap has no call to drop one binding.
fn apply(cx: &mut App) {
    let mut bindings: Vec<KeyBinding> = with(cx, |keys| {
        BINDINGS
            .iter()
            .zip(KEY_ACTIONS)
            .map(|((_, make), a)| {
                let text = key_text(&keys.keystroke(a.id), keys.os);
                KeyBinding::load(&text, make(), None, false, None, cx).expect("a valid key binding")
            })
            .collect()
    });
    // The Menu key opens the context menu, as Shift+F10 does. A key with no modifier cannot be a
    // key of the file, so this one is fixed.
    bindings.push(KeyBinding::new(
        "menu",
        crate::actions::OpenContextMenu,
        None,
    ));
    let ours: HashSet<&'static str> = BINDINGS.iter().map(|(_, make)| make().name()).collect();
    let keymap = cx.key_bindings();
    let others: Vec<KeyBinding> = keymap
        .borrow()
        .bindings()
        .filter(|b| !ours.contains(b.action().name()))
        .cloned()
        .collect();
    cx.clear_key_bindings();
    cx.bind_keys(others.into_iter().chain(bindings));
}

/// Reads the file again and binds what it holds. Call it after a write of the file.
pub fn reload(cx: &mut App) {
    let (path, os) = with(cx, |k| (k.path.clone(), k.os));
    cx.set_global(Keys::load(path, os));
    apply(cx);
}

fn file_path(cx: &App) -> Result<PathBuf, String> {
    with(cx, |k| k.path.clone())
        .ok_or_else(|| "There is no config folder, so the key cannot be saved.".to_string())
}

/// Saves `ks` as the key of action `id` and binds it. The old key does nothing from now on.
/// The error is one line for the owner.
pub fn change(cx: &mut App, id: &str, ks: &Keystroke) -> Result<(), String> {
    with(cx, |k| k.check(id, ks)).map_err(|r| r.to_string())?;
    let path = file_path(cx)?;
    let os = with(cx, |k| k.os);
    // A key that equals the default is no change: the file holds only the differences.
    let default = with(cx, |k| k.default_of(Keys::index(id)));
    let text = (!same(ks, &default)).then(|| key_text(ks, os));
    KeyFile::write_app(&path, id, text.as_deref())?;
    reload(cx);
    Ok(())
}

/// Removes the entry of action `id` from the file, so the default key is back.
pub fn reset(cx: &mut App, id: &str) -> Result<(), String> {
    let path = file_path(cx)?;
    KeyFile::write_app(&path, id, None)?;
    reload(cx);
    Ok(())
}

/// `key` of `[dictation]`.
pub fn dictation(cx: &App) -> Option<String> {
    with(cx, |k| k.dictation().map(String::from))
}

/// `mode` of `[dictation]`.
pub fn dictation_mode(cx: &App) -> Mode {
    with(cx, |k| k.dictation_mode())
}

/// `key` of `[conversation]`.
pub fn conversation(cx: &App) -> Option<String> {
    with(cx, |k| k.conversation().map(String::from))
}

/// What a [`capture`] saw.
#[derive(Debug, Clone, PartialEq)]
pub enum Captured {
    /// The next key press, with its modifiers. No rule has been applied.
    Key(Keystroke),
    /// Escape, with no modifier.
    Cancelled,
}

/// Waits for the next key press of the window and calls `on` with it. While it waits, no action
/// of the app runs and no pane gets the key. The capture lasts until the caller drops the
/// subscription, so a caller drops it when `on` runs. A modifier alone is no key press.
///
/// The caller checks the key with its own rules: the app keys use [`Keys::check`], and the
/// dictation key accepts `Super`.
pub fn capture<T: 'static>(
    this: WeakEntity<T>,
    cx: &mut App,
    on: impl Fn(&mut T, Captured, &mut Window, &mut Context<T>) + 'static,
) -> Subscription {
    cx.intercept_keystrokes(move |event, window, cx| {
        let ks = &event.keystroke;
        if ks.is_modifier_key() {
            return;
        }
        cx.stop_propagation();
        let captured = if ks.key == "escape" && !ks.modifiers.modified() {
            Captured::Cancelled
        } else {
            Captured::Key(ks.clone())
        };
        let _ = this.update(cx, |t, cx| on(t, captured, window, cx));
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    const OS: OperatingSystem = OperatingSystem::Linux;

    fn ks(text: &str) -> Keystroke {
        parse_key(text, OS).unwrap()
    }

    fn keys_of(text: &str) -> Keys {
        let dir = tempdir("load");
        let path = dir.join("keys.toml");
        std::fs::write(&path, text).unwrap();
        Keys::load(Some(path), OS)
    }

    fn tempdir(name: &str) -> PathBuf {
        static N: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
        let n = N.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let dir =
            std::env::temp_dir().join(format!("agentsi-keys-{name}-{}-{n}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn no_file_gives_the_default_keys_of_the_bindings() {
        let none = Keys::load(None, OS);
        let missing = Keys::load(Some(tempdir("missing").join("keys.toml")), OS);
        for keys in [none, missing] {
            assert!(keys.problems().is_empty());
            for (a, (default, _)) in KEY_ACTIONS.iter().zip(BINDINGS) {
                let want = Keystroke::parse_with_os(default, OS).unwrap();
                assert_eq!(keys.keystroke(a.id), want, "{}", a.id);
            }
            assert!(keys.rows().iter().all(|r| !r.changed));
            assert_eq!(keys.dictation(), None);
        }
    }

    #[test]
    fn the_default_keys_pass_the_rules_and_are_all_different() {
        let keys = Keys::load(None, OS);
        let rows = keys.rows();
        for r in &rows {
            assert_eq!(keys.check(r.id, &r.keystroke), Ok(()), "{}", r.id);
            let others = rows.iter().filter(|o| same(&o.keystroke, &r.keystroke));
            assert_eq!(others.count(), 1, "{} shares its key", r.id);
        }
    }

    #[test]
    fn a_file_changes_a_key_and_keeps_the_others() {
        let keys = keys_of("[app]\nnew-terminal = \"ctrl-alt-y\"\n");
        assert!(keys.problems().is_empty(), "{:?}", keys.problems());
        assert_eq!(keys.keystroke("new-terminal"), ks("ctrl-alt-y"));
        assert_eq!(keys.keystroke("palette"), ks("ctrl-k"));
        let row = keys.rows().into_iter().find(|r| r.id == "new-terminal");
        assert!(row.unwrap().changed);
    }

    #[test]
    fn each_bad_entry_is_skipped_with_one_line_and_the_good_ones_stay() {
        let keys = keys_of(
            "[app]\nno-such-action = \"ctrl-alt-y\"\nroles = \"not a key\"\nstories = \"ctrl-alt-s\"\n\
             desk = 7\nmeeting = \"ctrl-shift-c\"\nmemory = \"y\"\nsettings = \"ctrl-shift-d\"\n",
        );
        assert_eq!(keys.keystroke("stories"), ks("ctrl-alt-s"));
        for id in ["roles", "desk", "meeting", "memory", "settings"] {
            let default = Keys::load(None, OS).keystroke(id);
            assert_eq!(keys.keystroke(id), default, "{id}");
        }
        let lines = keys.problems();
        assert_eq!(lines.len(), 6, "{lines:#?}");
        assert!(lines[0].starts_with("[app] no-such-action"), "{lines:?}");
        for (i, id) in ["roles", "desk", "meeting", "memory", "settings"]
            .iter()
            .enumerate()
        {
            assert!(
                lines.iter().any(|l| l.starts_with(&format!("[app] {id}:"))),
                "{id} {i}: {lines:?}"
            );
        }
        let taken = lines.iter().find(|l| l.contains("settings")).unwrap();
        assert!(taken.contains("\"Front desk\""), "{taken}");
    }

    #[test]
    fn a_broken_file_gives_the_defaults_and_one_line() {
        let keys = keys_of("[app\nnew-terminal = ");
        assert_eq!(keys.problems().len(), 1);
        assert!(keys.problems()[0].contains("does not parse"));
        assert_eq!(keys.keystroke("new-terminal"), ks("ctrl-t"));
    }

    #[test]
    fn two_actions_may_swap_their_keys() {
        let keys = keys_of("[app]\nroles = \"ctrl-shift-s\"\nstories = \"ctrl-shift-r\"\n");
        assert!(keys.problems().is_empty(), "{:?}", keys.problems());
        assert_eq!(keys.keystroke("roles"), ks("ctrl-shift-s"));
        assert_eq!(keys.keystroke("stories"), ks("ctrl-shift-r"));
    }

    #[test]
    fn two_entries_with_one_key_keep_the_first() {
        let keys = keys_of("[app]\nroles = \"ctrl-alt-j\"\nstories = \"ctrl-alt-j\"\n");
        assert_eq!(keys.keystroke("roles"), ks("ctrl-alt-j"));
        assert_eq!(keys.keystroke("stories"), ks("ctrl-shift-s"));
        assert_eq!(keys.problems().len(), 1);
    }

    #[test]
    fn the_editor_refuses_each_kind_of_key_with_its_reason() {
        let keys = Keys::load(None, OS);
        let no_modifier = keys.check("roles", &ks("y")).unwrap_err();
        assert!(matches!(no_modifier, Refusal::NoModifier { .. }));
        assert!(no_modifier.to_string().contains("needs Ctrl, Alt"));
        assert!(keys.check("roles", &ks("shift-y")).is_err());
        let pane = keys.check("roles", &ks("ctrl-shift-c")).unwrap_err();
        assert!(matches!(pane, Refusal::PaneKey { .. }), "{pane}");
        assert!(pane.to_string().contains("terminal pane"));
        assert!(matches!(
            keys.check("roles", &ks("ctrl-shift-v")),
            Err(Refusal::PaneKey { .. })
        ));
        let taken = keys.check("roles", &ks("ctrl-k")).unwrap_err();
        assert_eq!(
            taken.to_string(),
            "Ctrl+K is already the key of \"Command palette\"."
        );
        // The action's own key is no conflict, and a function key stands alone.
        assert_eq!(keys.check("palette", &ks("ctrl-k")), Ok(()));
        assert_eq!(keys.check("roles", &ks("f9")), Ok(()));
        assert_eq!(keys.check("roles", &ks("alt-y")), Ok(()));
        assert!(matches!(
            parse_key("ctrl-k ctrl-j", OS),
            Err(Refusal::NotAKey { .. })
        ));
        assert!(parse_key("", OS).is_err());
        assert!(parse_key("ctrl", OS).is_err());
    }

    #[test]
    fn ctrl_with_one_letter_is_refused_except_as_the_default_of_the_same_action() {
        let keys = Keys::load(None, OS);
        for key in ["ctrl-c", "ctrl-d", "ctrl-w", "ctrl-z", "ctrl-a", "ctrl-r"] {
            let refusal = keys.check("roles", &ks(key)).unwrap_err();
            assert!(
                matches!(refusal, Refusal::ProgramKey { .. }),
                "{key}: {refusal}"
            );
            assert!(
                refusal.to_string().contains("terminal programs"),
                "{refusal}"
            );
        }
        // Another modifier, a digit or a symbol is fine.
        for key in [
            "ctrl-shift-j",
            "ctrl-alt-c",
            "ctrl-0",
            "ctrl-.",
            "alt-c",
            "super-c",
        ] {
            assert_eq!(keys.check("minutes", &ks(key)), Ok(()), "{key}");
        }
        // Two defaults are Ctrl and one letter. They stay, and each stays valid on its action.
        assert_eq!(keys.check("palette", &ks("ctrl-k")), Ok(()));
        assert_eq!(keys.check("new-terminal", &ks("ctrl-t")), Ok(()));
        let from_file = keys_of("[app]\npalette = \"ctrl-k\"\ndesk = \"ctrl-d\"\n");
        assert_eq!(from_file.problems().len(), 1, "{:?}", from_file.problems());
        assert!(from_file.problems()[0].starts_with("[app] desk:"));
        assert!(from_file.problems()[0].contains("terminal programs"));
        assert_eq!(from_file.keystroke("desk"), keys.keystroke("desk"));
    }

    #[test]
    fn a_key_that_parses_to_an_empty_key_is_an_invalid_entry() {
        let keys = keys_of("[app]\ndesk = \"ctrl-\"\nroles = \"alt-\"\n");
        assert_eq!(keys.problems().len(), 2, "{:?}", keys.problems());
        assert!(keys.problems()[0].starts_with("[app] desk:"));
        assert_eq!(
            keys.keystroke("desk"),
            Keys::load(None, OS).keystroke("desk")
        );
        assert!(parse_key("ctrl-", OS).is_err());
    }

    #[test]
    fn the_dictation_key_follows_the_rules_of_the_app_keys_and_no_action_may_hold_it() {
        let keys = Keys::load(None, OS);
        assert!(matches!(
            keys.check_dictation(&ks("ctrl-c")),
            Err(Refusal::ProgramKey { .. })
        ));
        assert!(matches!(
            keys.check_dictation(&ks("ctrl-shift-c")),
            Err(Refusal::PaneKey { .. })
        ));
        let held = keys.check_dictation(&ks("ctrl-shift-d")).unwrap_err();
        assert_eq!(
            held.to_string(),
            "Ctrl+Shift+D is already the key of \"Front desk\"."
        );
        // The default of an action is not exempt here: the compositor would take it.
        assert!(keys.check_dictation(&ks("ctrl-k")).is_err());
        for ok in ["super-f10", "f9", "ctrl-alt-v", "super-v"] {
            assert_eq!(keys.check_dictation(&ks(ok)), Ok(()), "{ok}");
        }
    }

    #[test]
    fn an_app_key_cannot_take_the_key_of_the_dictation() {
        let dir = tempdir("dictation-held");
        let path = dir.join("keys.toml");
        KeyFile::write_dictation(&path, Some("SUPER + F10")).unwrap();
        let keys = Keys::load(Some(path), OS);
        let refusal = keys.check("roles", &ks("super-f10")).unwrap_err();
        assert!(
            refusal.to_string().contains("\"Dictation key\""),
            "{refusal}"
        );
        assert_eq!(keys.check("roles", &ks("super-f9")), Ok(()));
    }

    #[test]
    fn the_conversation_key_is_read_and_written_beside_the_dictation_key() {
        let dir = tempdir("conversation");
        let path = dir.join("keys.toml");
        std::fs::write(&path, "[dictation]\nkey = \"SUPER + F10\" # mine\n").unwrap();
        KeyFile::write_conversation(&path, Some("SUPER + F9")).unwrap();
        let keys = Keys::load(Some(path.clone()), OS);
        assert_eq!(keys.conversation(), Some("SUPER + F9"));
        assert_eq!(keys.dictation(), Some("SUPER + F10"));
        assert!(std::fs::read_to_string(&path).unwrap().contains("# mine"));
        // Neither key may take the other, and an app key may take neither.
        assert!(
            keys.check_conversation(&ks("super-f10"))
                .unwrap_err()
                .to_string()
                .contains("\"Dictation key\"")
        );
        assert!(
            keys.check_dictation(&ks("super-f9"))
                .unwrap_err()
                .to_string()
                .contains("\"Conversation key\"")
        );
        assert!(
            keys.check("roles", &ks("super-f9"))
                .unwrap_err()
                .to_string()
                .contains("\"Conversation key\"")
        );
        assert_eq!(keys.check_conversation(&ks("super-f8")), Ok(()));
        assert!(keys.check_conversation(&ks("ctrl-c")).is_err());
        assert!(keys.check_conversation(&ks("ctrl-shift-d")).is_err());
        KeyFile::write_conversation(&path, None).unwrap();
        let text = std::fs::read_to_string(&path).unwrap();
        assert!(!text.contains("conversation"), "{text}");
        let (file, problems) = KeyFile::parse("[conversation]\nkey = 3\n").unwrap();
        assert_eq!(file.conversation, None);
        assert_eq!(problems.len(), 1);
        let (_, problems) = KeyFile::parse("conversation = 3\n").unwrap();
        assert_eq!(problems.len(), 1);
    }

    #[test]
    fn a_saved_key_reads_back_and_the_comments_and_other_entries_stay() {
        let dir = tempdir("write");
        let path = dir.join("agentsi").join("keys.toml");
        KeyFile::write_app(&path, "new-terminal", Some("ctrl-alt-y")).unwrap();
        let text = std::fs::read_to_string(&path).unwrap();
        assert!(text.contains("new-terminal = \"ctrl-alt-y\""), "{text}");

        std::fs::write(
            &path,
            "# my keys\n[app]\nzzz = \"x\" # bad entry\nroles = \"ctrl-alt-r\"\n\n[dictation]\nkey = \"SUPER + F10\"\n",
        )
        .unwrap();
        KeyFile::write_app(&path, "desk", Some("ctrl-alt-d")).unwrap();
        let text = std::fs::read_to_string(&path).unwrap();
        assert!(
            text.contains("# my keys") && text.contains("# bad entry"),
            "{text}"
        );
        let (file, problems) = KeyFile::parse(&text).unwrap();
        assert_eq!(file.app["roles"], "ctrl-alt-r");
        assert_eq!(file.app["desk"], "ctrl-alt-d");
        assert_eq!(file.dictation.as_deref(), Some("SUPER + F10"));
        assert_eq!(problems.len(), 1);
        assert!(!path.with_extension("toml.tmp").exists());
    }

    #[test]
    fn a_removed_entry_leaves_the_rest_and_an_empty_table_goes() {
        let dir = tempdir("remove");
        let path = dir.join("keys.toml");
        std::fs::write(
            &path,
            "[app]\nroles = \"ctrl-alt-r\"\n\n[dictation]\nkey = \"F9\"\n",
        )
        .unwrap();
        KeyFile::write_app(&path, "roles", None).unwrap();
        let text = std::fs::read_to_string(&path).unwrap();
        assert!(!text.contains("roles") && !text.contains("[app]"), "{text}");
        assert!(text.contains("key = \"F9\""), "{text}");
        KeyFile::write_dictation(&path, None).unwrap();
        assert_eq!(std::fs::read_to_string(&path).unwrap().trim(), "");
        // Removing from a missing file or a missing entry is no error.
        KeyFile::write_app(&path, "roles", None).unwrap();
        KeyFile::write_app(&dir.join("none.toml"), "roles", None).unwrap();
    }

    #[test]
    fn the_dictation_key_is_read_and_written_with_the_app_keys_kept() {
        let dir = tempdir("dictation");
        let path = dir.join("keys.toml");
        std::fs::write(&path, "[app]\nroles = \"ctrl-alt-r\"\n").unwrap();
        KeyFile::write_dictation(&path, Some("SUPER + F10")).unwrap();
        let keys = Keys::load(Some(path.clone()), OS);
        assert_eq!(keys.dictation(), Some("SUPER + F10"));
        assert_eq!(keys.keystroke("roles"), ks("ctrl-alt-r"));
        KeyFile::write_dictation(&path, Some("SUPER + F11")).unwrap();
        let (file, _) = KeyFile::read(&path).unwrap();
        assert_eq!(file.dictation.as_deref(), Some("SUPER + F11"));
        let (file, problems) = KeyFile::parse("[dictation]\nkey = 3\n").unwrap();
        assert_eq!(file.dictation, None);
        assert_eq!(problems.len(), 1);
    }

    #[test]
    fn the_dictation_mode_is_hold_unless_the_file_says_toggle() {
        let dir = tempdir("dictation-mode");
        let path = dir.join("keys.toml");
        assert_eq!(
            Keys::load(Some(path.clone()), OS).dictation_mode(),
            Mode::Hold
        );
        std::fs::write(
            &path,
            "# my keys\n[dictation]\nkey = \"SUPER + F10\" # mine\n",
        )
        .unwrap();
        KeyFile::write_dictation_mode(&path, Mode::Toggle).unwrap();
        let text = std::fs::read_to_string(&path).unwrap();
        assert!(
            text.contains("# my keys") && text.contains("# mine"),
            "{text}"
        );
        let keys = Keys::load(Some(path.clone()), OS);
        assert_eq!(
            (keys.dictation_mode(), keys.dictation()),
            (Mode::Toggle, Some("SUPER + F10"))
        );
        // Removing the key keeps the mode. Hold removes the entry, and the empty table goes.
        KeyFile::write_dictation(&path, None).unwrap();
        assert_eq!(
            Keys::load(Some(path.clone()), OS).dictation_mode(),
            Mode::Toggle
        );
        KeyFile::write_dictation_mode(&path, Mode::Hold).unwrap();
        assert_eq!(std::fs::read_to_string(&path).unwrap().trim(), "");
        // A mode that is not one of the two is skipped with a line.
        let (file, problems) = KeyFile::parse("[dictation]\nmode = \"sticky\"\n").unwrap();
        assert_eq!(file.dictation_mode, None);
        assert_eq!(problems.len(), 1);
        let (file, problems) = KeyFile::parse("[dictation]\nmode = 1\n").unwrap();
        assert_eq!((file.dictation_mode, problems.len()), (None, 1));
    }

    #[test]
    fn a_file_that_does_not_parse_is_not_overwritten() {
        let dir = tempdir("broken");
        let path = dir.join("keys.toml");
        std::fs::write(&path, "[app\n").unwrap();
        let err = KeyFile::write_app(&path, "roles", Some("ctrl-alt-r")).unwrap_err();
        assert!(err.contains("does not parse"), "{err}");
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "[app\n");
    }

    #[test]
    fn a_key_goes_to_the_file_in_gpui_form() {
        let pressed = Keystroke {
            modifiers: gpui_kit::Modifiers {
                control: true,
                shift: true,
                ..Default::default()
            },
            key: "y".into(),
            key_char: Some("Y".into()),
        };
        assert_eq!(key_text(&pressed, OS), "ctrl-shift-y");
        assert_eq!(ks(&key_text(&pressed, OS)), ks("ctrl-shift-y"));
        assert_eq!(display(&pressed), "Ctrl+Shift+Y");
    }
}
