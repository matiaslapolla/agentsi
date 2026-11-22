//! `<config>/agentsi/terminal.toml`: the terminal settings.
//!
//! ```toml
//! font_size = 13            # 9 to 24 px
//! line_height = 1.25        # 1.0 to 1.6, in steps of 0.05
//! cursor_style = "program"  # program, block, bar or underline
//! cursor_blink = "program"  # program, on or off
//! scrollback_mb = 10        # 1 to 100, for new panes
//! shell = ""                # empty uses $SHELL, for new panes
//! palette = "follow-theme"  # agentsi-dark, agentsi-light or follow-theme
//! copy_on_select = false
//! option_as_meta = true
//! link_modifier = "ctrl"   # ctrl or none: what a click on a link needs to open it
//! ```
//!
//! - A missing file means the defaults. An unknown key, a bad value or a file that does not
//!   parse gives the default for that part and one line in [`note`]. The app starts anyway.
//! - [`change`] applies a value at once and saves it. A file that does not parse is not
//!   overwritten, as `keys.toml` is not.

use std::path::{Path, PathBuf};

use agentsi_ui::appearance::{
    CursorBlink, CursorStyle, LinkModifier, TerminalPalette, TerminalSettings,
};
use gpui_kit::{App, Global, SharedString};
use toml_edit::{DocumentMut, Item, value};

pub const FILE_NAME: &str = "terminal.toml";

/// `<config>/agentsi/terminal.toml`.
pub fn path() -> Option<PathBuf> {
    crate::layout::path().map(|p| p.with_file_name(FILE_NAME))
}

/// The file in `path`, and one line for the owner when something in it cannot be used.
pub fn read(path: &Path) -> (TerminalSettings, Option<String>) {
    match std::fs::read_to_string(path) {
        Ok(text) => parse(&text, path),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => (TerminalSettings::default(), None),
        Err(e) => (
            TerminalSettings::default(),
            Some(format!(
                "{} cannot be read: {e}. The default terminal settings stay.",
                path.display()
            )),
        ),
    }
}

/// The value of a line height, when it is one of the steps of 0.05 from 1.0 to 1.6.
fn line_height_of(item: &Item) -> Option<f32> {
    let v = item
        .as_float()
        .or_else(|| item.as_integer().map(|n| n as f64))?;
    let steps = v * 20.;
    let on_step = (steps - steps.round()).abs() < 1e-6;
    let v = (steps.round() / 20.) as f32;
    (on_step && TerminalSettings::LINE_HEIGHTS.contains(&v)).then_some(v)
}

fn parse(text: &str, path: &Path) -> (TerminalSettings, Option<String>) {
    let doc: DocumentMut = match text.parse() {
        Ok(d) => d,
        Err(e) => {
            let e: toml_edit::TomlError = e;
            return (
                TerminalSettings::default(),
                Some(format!(
                    "{} does not parse: {}. The default terminal settings stay.",
                    path.display(),
                    e.message()
                )),
            );
        }
    };
    let mut problems = Vec::new();
    let mut t = TerminalSettings::default();
    for (key, item) in doc.iter() {
        let mut bad = |why: String| problems.push(format!("{key} {why}"));
        match key {
            "font_size" => match item.as_integer().and_then(|n| u8::try_from(n).ok()) {
                Some(n) if TerminalSettings::FONT_SIZES.contains(&n) => t.font_size = n,
                _ => bad("must be a whole number from 9 to 24.".into()),
            },
            "line_height" => match line_height_of(item) {
                Some(v) => t.line_height = v,
                None => bad("must be a number from 1.0 to 1.6, in steps of 0.05.".into()),
            },
            "cursor_style" => match item.as_str().and_then(CursorStyle::from_key) {
                Some(v) => t.cursor_style = v,
                None => bad(format!(
                    "must be one of {}.",
                    CursorStyle::ALL.map(CursorStyle::key).join(", ")
                )),
            },
            "cursor_blink" => match item.as_str().and_then(CursorBlink::from_key) {
                Some(v) => t.cursor_blink = v,
                None => bad(format!(
                    "must be one of {}.",
                    CursorBlink::ALL.map(CursorBlink::key).join(", ")
                )),
            },
            "scrollback_mb" => match item.as_integer().and_then(|n| u16::try_from(n).ok()) {
                Some(n) if TerminalSettings::SCROLLBACK_MB.contains(&n) => t.scrollback_mb = n,
                _ => bad("must be a whole number from 1 to 100.".into()),
            },
            "shell" => match item.as_str() {
                Some(s) => t.shell = SharedString::from(s.trim().to_string()),
                None => bad("must be text: a path, or empty for $SHELL.".into()),
            },
            "palette" => match item.as_str().and_then(TerminalPalette::from_key) {
                Some(v) => t.palette = v,
                None => bad(format!(
                    "must be one of {}.",
                    TerminalPalette::ALL.map(TerminalPalette::key).join(", ")
                )),
            },
            "copy_on_select" => match item.as_bool() {
                Some(b) => t.copy_on_select = b,
                None => bad("must be true or false.".into()),
            },
            "option_as_meta" => match item.as_bool() {
                Some(b) => t.option_as_meta = b,
                None => bad("must be true or false.".into()),
            },
            "link_modifier" => match item.as_str().and_then(LinkModifier::from_key) {
                Some(v) => t.link_modifier = v,
                None => bad(format!(
                    "must be one of {}.",
                    LinkModifier::ALL.map(LinkModifier::key).join(", ")
                )),
            },
            other => problems.push(format!("{other}: no setting has this name.")),
        }
    }
    let note = (!problems.is_empty()).then(|| {
        format!(
            "{}: {} The default stays for each of these.",
            path.display(),
            problems.join(" ")
        )
    });
    (t, note)
}

/// The file and the line about it, kept as a global so the settings screen shows them.
#[derive(Clone, Debug, Default)]
pub struct Setting {
    path: Option<PathBuf>,
    note: Option<String>,
}

impl Global for Setting {}

/// Reads the file at `path` and makes it the terminal settings of the app. The app calls this
/// once, at start.
/// Reads the file of `install` again and applies it.
pub fn reload(cx: &mut App) {
    let path = cx.try_global::<Setting>().and_then(|s| s.path.clone());
    install(cx, path);
}

pub fn install(cx: &mut App, path: Option<PathBuf>) {
    let (settings, note) = match &path {
        Some(p) => read(p),
        None => (TerminalSettings::default(), None),
    };
    cx.set_global(Setting { path, note });
    settings.apply(cx);
}

/// The line about the file, or none when it is whole.
pub fn note(cx: &App) -> Option<String> {
    cx.try_global::<Setting>().and_then(|s| s.note.clone())
}

/// Applies `f` to the settings in effect and saves the result. The panes change at once, even
/// when the save fails. The error is one line for the owner, and [`note`] shows it too.
pub fn change(cx: &mut App, f: impl FnOnce(&mut TerminalSettings)) -> Result<(), String> {
    let mut next = TerminalSettings::get(cx);
    f(&mut next);
    next.clone().apply(cx);
    let saved = match cx.try_global::<Setting>().and_then(|s| s.path.clone()) {
        Some(path) => write(&path, &next),
        None => Err("There is no config folder, so the terminal settings cannot be saved.".into()),
    };
    let mut setting = cx.try_global::<Setting>().cloned().unwrap_or_default();
    setting.note = saved.clone().err();
    cx.set_global(setting);
    saved
}

/// The program of a new shell on this device: the shell of the settings, else `$SHELL`, else
/// `/bin/sh`.
pub fn local_shell(cx: &App) -> String {
    let shell = TerminalSettings::get(cx).shell;
    if shell.is_empty() {
        std::env::var("SHELL").unwrap_or_else(|_| "/bin/sh".into())
    } else {
        shell.to_string()
    }
}

/// The scrollback of a new pane, in bytes.
pub fn scrollback_bytes(cx: &App) -> usize {
    usize::from(TerminalSettings::get(cx).scrollback_mb) * 1_000_000
}

fn write(path: &Path, t: &TerminalSettings) -> Result<(), String> {
    crate::keys::edit(path, |doc| {
        doc["font_size"] = value(i64::from(t.font_size));
        // The steps of 0.05 keep two decimals, so the file shows 1.25 and not 1.2500000476837158.
        doc["line_height"] = value((f64::from(t.line_height) * 100.).round() / 100.);
        doc["cursor_style"] = value(t.cursor_style.key());
        doc["cursor_blink"] = value(t.cursor_blink.key());
        doc["scrollback_mb"] = value(i64::from(t.scrollback_mb));
        doc["shell"] = value(t.shell.as_ref());
        doc["palette"] = value(t.palette.key());
        doc["copy_on_select"] = value(t.copy_on_select);
        doc["option_as_meta"] = value(t.option_as_meta);
        doc["link_modifier"] = value(t.link_modifier.key());
        Ok(())
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tempdir(name: &str) -> PathBuf {
        static N: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
        let n = N.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let dir = std::env::temp_dir().join(format!(
            "agentsi-terminal-{name}-{}-{n}",
            std::process::id()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn of(text: &str) -> (TerminalSettings, Option<String>) {
        parse(text, Path::new("terminal.toml"))
    }

    #[test]
    fn no_file_gives_the_defaults_of_the_spec_and_no_note() {
        let (t, note) = read(&tempdir("missing").join(FILE_NAME));
        assert_eq!(note, None);
        assert_eq!(t, TerminalSettings::default());
        assert_eq!(
            (t.font_size, t.line_height, t.scrollback_mb),
            (13, 1.25, 10)
        );
        assert_eq!(
            (t.cursor_style, t.cursor_blink, t.palette),
            (
                CursorStyle::Program,
                CursorBlink::Program,
                TerminalPalette::FollowTheme
            )
        );
        assert!(t.shell.is_empty());
        assert!(!t.copy_on_select && t.option_as_meta);
        assert_eq!(t.link_modifier, LinkModifier::Ctrl);
    }

    #[test]
    fn each_key_reads_its_value() {
        let (t, note) = of(
            "font_size = 16\nline_height = 1.4\ncursor_style = \"bar\"\ncursor_blink = \"off\"\n\
             scrollback_mb = 50\nshell = \"/usr/bin/fish\"\npalette = \"agentsi-light\"\n\
             copy_on_select = true\noption_as_meta = false\nlink_modifier = \"none\"\n",
        );
        assert_eq!(note, None);
        assert_eq!(
            t,
            TerminalSettings {
                font_size: 16,
                line_height: 1.4,
                cursor_style: CursorStyle::Bar,
                cursor_blink: CursorBlink::Off,
                scrollback_mb: 50,
                shell: "/usr/bin/fish".into(),
                palette: TerminalPalette::AgentsiLight,
                copy_on_select: true,
                option_as_meta: false,
                link_modifier: LinkModifier::None,
            }
        );
        assert_eq!(of("line_height = 1").0.line_height, 1.0);
    }

    #[test]
    fn a_bad_value_falls_back_to_its_default_with_one_note_line() {
        let (t, note) = of("font_size = 30\ncursor_style = \"beam\"\nfont_family = \"x\"\n");
        assert_eq!(t, TerminalSettings::default());
        let note = note.expect("a note");
        assert_eq!(note.lines().count(), 1, "{note}");
        assert!(
            note.contains("font_size must be a whole number from 9 to 24"),
            "{note}"
        );
        assert!(note.contains("cursor_style must be one of program, block, bar, underline"));
        assert!(note.contains("font_family: no setting has this name"));
        for bad in [
            "line_height = 1.33",
            "line_height = 1.7",
            "line_height = \"tall\"",
        ] {
            let (t, note) = of(bad);
            assert_eq!(t.line_height, 1.25, "{bad}");
            assert!(note.unwrap().contains("steps of 0.05"), "{bad}");
        }
        let (t, note) = of("link_modifier = \"alt\"");
        assert_eq!(t.link_modifier, LinkModifier::Ctrl);
        assert!(
            note.unwrap()
                .contains("link_modifier must be one of ctrl, none")
        );
        let (t, note) = of("scrollback_mb = 0\ncopy_on_select = \"yes\"\nfont_size = 12\n");
        assert_eq!(
            (t.scrollback_mb, t.copy_on_select, t.font_size),
            (10, false, 12)
        );
        assert!(
            note.unwrap()
                .contains("copy_on_select must be true or false")
        );
    }

    #[test]
    fn a_file_that_does_not_parse_gives_the_defaults_and_one_note() {
        let (t, note) = of("font_size = = 3");
        assert_eq!(t, TerminalSettings::default());
        assert!(note.unwrap().contains("does not parse"));
    }

    #[test]
    fn a_saved_value_reads_back_and_comments_stay() {
        let path = tempdir("save").join(FILE_NAME);
        std::fs::write(&path, "# my terminal\nfont_size = 15\n").unwrap();
        let t = TerminalSettings {
            font_size: 18,
            line_height: 1.35,
            shell: "/bin/zsh".into(),
            palette: TerminalPalette::AgentsiDark,
            ..TerminalSettings::default()
        };
        write(&path, &t).unwrap();
        assert_eq!(read(&path), (t, None));
        let text = std::fs::read_to_string(&path).unwrap();
        assert!(text.contains("# my terminal"));
        assert!(text.contains("line_height = 1.35"), "{text}");
    }

    #[test]
    fn a_file_that_does_not_parse_is_not_overwritten() {
        let path = tempdir("broken").join(FILE_NAME);
        std::fs::write(&path, "font_size = = x").unwrap();
        assert!(write(&path, &TerminalSettings::default()).is_err());
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "font_size = = x");
    }
}
