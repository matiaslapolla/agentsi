//! `<config>/agentsi/appearance.toml`: the density, the corners and the text size.
//!
//! ```toml
//! density = "compact"   # compact, normal or comfortable
//! corners = "round"     # square, subtle or round
//! text = "large"        # small, medium or large
//! ```
//!
//! - A missing file means the defaults. An unknown key, an unknown value or a file that does
//!   not parse gives the default for that part and one line in [`note`]. The app starts anyway.
//! - [`change`] applies a choice at once and saves it. A file that does not parse is not
//!   overwritten, as `keys.toml` is not.

use std::path::{Path, PathBuf};

use agentsi_ui::appearance::{Appearance, Corners, Density, TextSize};
use gpui_kit::{App, Global};
use toml_edit::{DocumentMut, Item, value};

pub const FILE_NAME: &str = "appearance.toml";

/// `<config>/agentsi/appearance.toml`.
pub fn path() -> Option<PathBuf> {
    crate::layout::path().map(|p| p.with_file_name(FILE_NAME))
}

/// The file in `path`, and one line for the owner when something in it cannot be used.
pub fn read(path: &Path) -> (Appearance, Option<String>) {
    match std::fs::read_to_string(path) {
        Ok(text) => parse(&text, path),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => (Appearance::default(), None),
        Err(e) => (
            Appearance::default(),
            Some(format!(
                "{} cannot be read: {e}. The default look stays.",
                path.display()
            )),
        ),
    }
}

fn parse(text: &str, path: &Path) -> (Appearance, Option<String>) {
    let doc: DocumentMut = match text.parse() {
        Ok(d) => d,
        Err(e) => {
            let e: toml_edit::TomlError = e;
            return (
                Appearance::default(),
                Some(format!(
                    "{} does not parse: {}. The default look stays.",
                    path.display(),
                    e.message()
                )),
            );
        }
    };
    let mut problems = Vec::new();
    let mut appearance = Appearance::default();
    for (key, item) in doc.iter() {
        match key {
            "density" => pick(
                key,
                item,
                &mut problems,
                Density::from_key,
                &mut appearance.density,
                &Density::ALL.map(Density::key),
            ),
            "corners" => pick(
                key,
                item,
                &mut problems,
                Corners::from_key,
                &mut appearance.corners,
                &Corners::ALL.map(Corners::key),
            ),
            "text" => pick(
                key,
                item,
                &mut problems,
                TextSize::from_key,
                &mut appearance.text,
                &TextSize::ALL.map(TextSize::key),
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
    (appearance, note)
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
    let (appearance, note) = match &path {
        Some(p) => read(p),
        None => (Appearance::default(), None),
    };
    cx.set_global(Setting { path, note });
    appearance.apply(cx);
}

/// The line about the file, or none when it is whole.
pub fn note(cx: &App) -> Option<String> {
    cx.try_global::<Setting>().and_then(|s| s.note.clone())
}

/// Applies `f` to the appearance in effect and saves the result. The look changes at once, even
/// when the save fails. The error is one line for the owner, and [`note`] shows it too.
pub fn change(cx: &mut App, f: impl FnOnce(&mut Appearance)) -> Result<(), String> {
    let mut next = Appearance::get(cx);
    f(&mut next);
    next.apply(cx);
    let saved = match cx.try_global::<Setting>().and_then(|s| s.path.clone()) {
        Some(path) => write(&path, next),
        None => Err("There is no config folder, so the look cannot be saved.".into()),
    };
    let mut setting = cx.try_global::<Setting>().cloned().unwrap_or_default();
    setting.note = saved.clone().err();
    cx.set_global(setting);
    saved
}

fn write(path: &Path, a: Appearance) -> Result<(), String> {
    crate::keys::edit(path, |doc| {
        doc["density"] = value(a.density.key());
        doc["corners"] = value(a.corners.key());
        doc["text"] = value(a.text.key());
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

    fn of(text: &str) -> (Appearance, Option<String>) {
        parse(text, Path::new("appearance.toml"))
    }

    #[test]
    fn no_file_gives_the_default_look_and_no_note() {
        let (a, note) = read(&tempdir("missing").join(FILE_NAME));
        assert_eq!((a, note), (Appearance::default(), None));
    }

    #[test]
    fn each_word_of_the_file_picks_its_choice() {
        let (a, note) = of("density = \"compact\"\ncorners = \"round\"\ntext = \"large\"\n");
        assert_eq!(
            a,
            Appearance {
                density: Density::Compact,
                corners: Corners::Round,
                text: TextSize::Large
            }
        );
        assert_eq!(note, None);
        let (a, _) = of("text = \"small\"");
        assert_eq!(a.text, TextSize::Small);
        assert_eq!(a.density, Density::Normal);
    }

    #[test]
    fn a_bad_value_keeps_its_default_and_the_good_ones_stay() {
        let (a, note) = of("density = \"huge\"\ncorners = \"round\"\ntext = 3\nfont = \"x\"\n");
        assert_eq!(a.density, Density::Normal);
        assert_eq!(a.corners, Corners::Round);
        assert_eq!(a.text, TextSize::Medium);
        let note = note.expect("one note");
        assert!(note.contains("density \"huge\" is not a choice (compact, normal, comfortable)"));
        assert!(note.contains("text must be text"));
        assert!(note.contains("font: no setting has this name"));
        assert_eq!(note.lines().count(), 1, "one note, one line: {note}");
    }

    #[test]
    fn a_file_that_does_not_parse_gives_the_defaults_and_one_note() {
        let (a, note) = of("density = = compact");
        assert_eq!(a, Appearance::default());
        assert!(note.unwrap().contains("does not parse"));
    }

    #[test]
    fn a_saved_choice_reads_back_and_comments_stay() {
        let path = tempdir("save").join("agentsi").join(FILE_NAME);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(&path, "# my look\ndensity = \"comfortable\"\n").unwrap();
        let a = Appearance {
            density: Density::Compact,
            corners: Corners::Square,
            text: TextSize::Large,
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
        assert!(write(&path, Appearance::default()).is_err());
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "density = = x");
    }
}
