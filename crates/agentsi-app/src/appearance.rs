//! `<config>/agentsi/appearance.toml`: the density, the corners, the text size, the color theme
//! and the motion.
//!
//! ```toml
//! density = "compact"   # compact, normal or comfortable
//! corners = "round"     # square, subtle or round
//! text = "large"        # small, medium or large
//! theme = "light"       # system, dark or light
//! motion = "reduced"    # system, reduced or full
//! ```
//!
//! - A missing file means the defaults. An unknown key, an unknown value or a file that does
//!   not parse gives the default for that part and one line in [`note`]. The app starts anyway.
//! - [`change`] applies a choice at once and saves it. A file that does not parse is not
//!   overwritten, as `keys.toml` is not.

use std::path::{Path, PathBuf};

use agentsi_ui::appearance::{Appearance, ColorTheme, Corners, Density, Motion, TextSize};
use gpui_kit::{App, Global};
use toml_edit::{DocumentMut, Item, value};

/// Everything that `appearance.toml` holds.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Look {
    pub appearance: Appearance,
    pub theme: ColorTheme,
    pub motion: Motion,
}

pub const FILE_NAME: &str = "appearance.toml";

/// `<config>/agentsi/appearance.toml`.
pub fn path() -> Option<PathBuf> {
    crate::layout::path().map(|p| p.with_file_name(FILE_NAME))
}

/// The file in `path`, and one line for the owner when something in it cannot be used.
pub fn read(path: &Path) -> (Look, Option<String>) {
    match std::fs::read_to_string(path) {
        Ok(text) => parse(&text, path),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => (Look::default(), None),
        Err(e) => (
            Look::default(),
            Some(format!(
                "{} cannot be read: {e}. The default look stays.",
                path.display()
            )),
        ),
    }
}

fn parse(text: &str, path: &Path) -> (Look, Option<String>) {
    let doc: DocumentMut = match text.parse() {
        Ok(d) => d,
        Err(e) => {
            let e: toml_edit::TomlError = e;
            return (
                Look::default(),
                Some(format!(
                    "{} does not parse: {}. The default look stays.",
                    path.display(),
                    e.message()
                )),
            );
        }
    };
    let mut problems = Vec::new();
    let mut look = Look::default();
    for (key, item) in doc.iter() {
        match key {
            "density" => pick(
                key,
                item,
                &mut problems,
                Density::from_key,
                &mut look.appearance.density,
                &Density::ALL.map(Density::key),
            ),
            "corners" => pick(
                key,
                item,
                &mut problems,
                Corners::from_key,
                &mut look.appearance.corners,
                &Corners::ALL.map(Corners::key),
            ),
            "text" => pick(
                key,
                item,
                &mut problems,
                TextSize::from_key,
                &mut look.appearance.text,
                &TextSize::ALL.map(TextSize::key),
            ),
            "theme" => pick(
                key,
                item,
                &mut problems,
                ColorTheme::from_key,
                &mut look.theme,
                &ColorTheme::ALL.map(ColorTheme::key),
            ),
            "motion" => pick(
                key,
                item,
                &mut problems,
                Motion::from_key,
                &mut look.motion,
                &Motion::ALL.map(Motion::key),
            ),
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
    (look, note)
}

/// Sets `*slot` from `item` when it names one of `options`. Otherwise adds a line to `problems`.
fn pick<T>(
    key: &str,
    item: &Item,
    problems: &mut Vec<String>,
    parse: fn(&str) -> Option<T>,
    slot: &mut T,
    options: &[&str],
) {
    match item.as_str().map(|s| (s, parse(s))) {
        Some((_, Some(v))) => *slot = v,
        Some((s, None)) => problems.push(format!(
            "{key} \"{s}\" is not a choice ({}).",
            options.join(", ")
        )),
        None => problems.push(format!("{key} must be text ({}).", options.join(", "))),
    }
}

/// The file and the line about it, kept as a global so the settings screen shows them.
#[derive(Clone, Debug, Default)]
pub struct Setting {
    path: Option<PathBuf>,
    note: Option<String>,
}

impl Global for Setting {}

/// Reads the file at `path` and makes it the look of the app. The app calls this once, at start.
pub fn install(cx: &mut App, path: Option<PathBuf>) {
    let (look, note) = match &path {
        Some(p) => read(p),
        None => (Look::default(), None),
    };
    cx.set_global(Setting { path, note });
    apply(cx, look);
}

/// The line about the file, or none when it is whole.
pub fn note(cx: &App) -> Option<String> {
    cx.try_global::<Setting>().and_then(|s| s.note.clone())
}

/// Applies `f` to the appearance in effect and saves the result. The look changes at once, even
/// when the save fails. The error is one line for the owner, and [`note`] shows it too.
pub fn change(cx: &mut App, f: impl FnOnce(&mut Look)) -> Result<(), String> {
    let mut next = current(cx);
    f(&mut next);
    apply(cx, next);
    let saved = match cx.try_global::<Setting>().and_then(|s| s.path.clone()) {
        Some(path) => write(&path, next),
        None => Err("There is no config folder, so the look cannot be saved.".into()),
    };
    let mut setting = cx.try_global::<Setting>().cloned().unwrap_or_default();
    setting.note = saved.clone().err();
    cx.set_global(setting);
    saved
}

/// The look in effect.
pub fn current(cx: &App) -> Look {
    Look {
        appearance: Appearance::get(cx),
        theme: ColorTheme::get(cx),
        motion: Motion::get(cx),
    }
}

fn apply(cx: &mut App, look: Look) {
    look.appearance.apply(cx);
    look.theme.apply(cx);
    look.motion.apply(cx);
}

fn write(path: &Path, look: Look) -> Result<(), String> {
    crate::keys::edit(path, |doc| {
        doc["density"] = value(look.appearance.density.key());
        doc["corners"] = value(look.appearance.corners.key());
        doc["text"] = value(look.appearance.text.key());
        doc["theme"] = value(look.theme.key());
        doc["motion"] = value(look.motion.key());
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
            "agentsi-appearance-{name}-{}-{n}",
            std::process::id()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn of(text: &str) -> (Look, Option<String>) {
        parse(text, Path::new("appearance.toml"))
    }

    #[test]
    fn no_file_gives_the_default_look_and_no_note() {
        let (a, note) = read(&tempdir("missing").join(FILE_NAME));
        assert_eq!((a, note), (Look::default(), None));
    }

    #[test]
    fn each_word_of_the_file_picks_its_choice() {
        let (a, note) = of("density = \"compact\"\ncorners = \"round\"\ntext = \"large\"\n");
        assert_eq!(
            a.appearance,
            Appearance {
                density: Density::Compact,
                corners: Corners::Round,
                text: TextSize::Large
            }
        );
        assert_eq!(note, None);
        let (a, _) = of("text = \"small\"");
        assert_eq!(a.appearance.text, TextSize::Small);
        assert_eq!(a.appearance.density, Density::Normal);
        assert_eq!((a.theme, a.motion), (ColorTheme::Dark, Motion::System));
        let (a, note) = of("theme = \"light\"\nmotion = \"reduced\"\n");
        assert_eq!(
            (a.theme, a.motion, note),
            (ColorTheme::Light, Motion::Reduced, None)
        );
        let (a, note) = of("theme = \"blue\"\nmotion = 1\n");
        assert_eq!((a.theme, a.motion), (ColorTheme::Dark, Motion::System));
        let note = note.unwrap();
        assert!(note.contains("theme \"blue\" is not a choice (system, dark, light)"));
        assert!(note.contains("motion must be text"));
    }

    #[test]
    fn a_bad_value_keeps_its_default_and_the_good_ones_stay() {
        let (a, note) = of("density = \"huge\"\ncorners = \"round\"\ntext = 3\nfont = \"x\"\n");
        assert_eq!(a.appearance.density, Density::Normal);
        assert_eq!(a.appearance.corners, Corners::Round);
        assert_eq!(a.appearance.text, TextSize::Medium);
        let note = note.expect("one note");
        assert!(note.contains("density \"huge\" is not a choice (compact, normal, comfortable)"));
        assert!(note.contains("text must be text"));
        assert!(note.contains("font: no setting has this name"));
        assert_eq!(note.lines().count(), 1, "one note, one line: {note}");
    }

    #[test]
    fn a_file_that_does_not_parse_gives_the_defaults_and_one_note() {
        let (a, note) = of("density = = compact");
        assert_eq!(a, Look::default());
        assert!(note.unwrap().contains("does not parse"));
    }

    #[test]
    fn a_saved_choice_reads_back_and_comments_stay() {
        let path = tempdir("save").join("agentsi").join(FILE_NAME);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(&path, "# my look\ndensity = \"comfortable\"\n").unwrap();
        let a = Look {
            appearance: Appearance {
                density: Density::Compact,
                corners: Corners::Square,
                text: TextSize::Large,
            },
            theme: ColorTheme::Light,
            motion: Motion::Full,
        };
        write(&path, a).unwrap();
        assert_eq!(read(&path), (a, None));
        assert!(
            std::fs::read_to_string(&path)
                .unwrap()
                .contains("# my look")
        );
    }

    #[test]
    fn a_file_that_does_not_parse_is_not_overwritten() {
        let path = tempdir("broken").join(FILE_NAME);
        std::fs::write(&path, "density = = x").unwrap();
        assert!(write(&path, Look::default()).is_err());
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "density = = x");
    }
}
