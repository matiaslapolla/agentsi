//! `<config>/agentsi/favorites.toml`: the targets that the owner saved to open again.
//!
//! ```toml
//! [[favorite]]
//! label = "Front desk"
//! product = "demo"          # `look::key` of the product: "@studio/demo" on a host. Absent: Home
//! kind = "screen"           # screen, folder or command
//! target = "desk"           # a screen kind, with ":<arg>" for a screen of one item ("story:S1")
//!
//! [[favorite]]
//! label = "Build"
//! product = "demo"
//! kind = "command"
//! target = "cargo build --release"   # the command line, split as a shell splits words
//! folder = "/home/me/demo"           # the folder of the command
//! ```
//!
//! - The order of the file is the order of the sidebar.
//! - The app reads what it knows. An entry that it cannot use is skipped with a note and stays
//!   at the end of the file of the next write, and so does a key that it does not know. A file that does not parse is
//!   not overwritten.

use std::path::{Path, PathBuf};

use gpui_kit::{App, AppContext as _, Context, Entity, Global};
use toml_edit::{ArrayOfTables, DocumentMut, Item, Table, value};

use crate::look::{self, clean_label};
use crate::model::ProductRef;
use crate::screens::{ScreenKind, ScreenSpec};

pub const FILE_NAME: &str = "favorites.toml";
const ENTRY: &str = "favorite";
/// The keys of an entry that the app writes.
const KNOWN: [&str; 5] = ["label", "product", "kind", "target", "folder"];

/// `<config>/agentsi/favorites.toml`.
pub fn path() -> Option<PathBuf> {
    crate::layout::path().map(|p| p.with_file_name(FILE_NAME))
}

/// What a favorite opens.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Saved {
    /// A screen of the product: its kind and its argument.
    Screen(ScreenKind, Option<String>),
    /// A folder. It opens a shell in the folder.
    Folder(String),
    /// A terminal command and the folder it runs in.
    Command { argv: Vec<String>, folder: String },
}

impl Saved {
    fn kind(&self) -> &'static str {
        match self {
            Saved::Screen(..) => "screen",
            Saved::Folder(_) => "folder",
            Saved::Command { .. } => "command",
        }
    }

    fn target(&self) -> String {
        match self {
            Saved::Screen(kind, None) => kind.name().to_string(),
            Saved::Screen(kind, Some(arg)) => format!("{}:{arg}", kind.name()),
            Saved::Folder(path) => path.clone(),
            Saved::Command { argv, .. } => join_words(argv),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Favorite {
    pub label: String,
    /// The product, or `None` for Home.
    pub product: Option<ProductRef>,
    pub saved: Saved,
}

impl Favorite {
    /// The screen of a screen favorite.
    pub fn screen(&self) -> Option<ScreenSpec> {
        let Saved::Screen(kind, arg) = &self.saved else {
            return None;
        };
        let (host, product) = match &self.product {
            Some(p) => (p.host.clone(), p.id.clone()),
            None => (None, String::new()),
        };
        Some(ScreenSpec {
            kind: *kind,
            host,
            product,
            arg: arg.clone(),
        })
    }
}

/// The file and the list, as an entity that the sidebar observes.
pub struct Favorites {
    path: Option<PathBuf>,
    pub list: Vec<Favorite>,
    /// One line about the file when something in it cannot be used or saved.
    pub note: Option<String>,
}

#[derive(Clone)]
pub struct FavoritesGlobal(pub Entity<Favorites>);

impl Global for FavoritesGlobal {}

impl FavoritesGlobal {
    pub fn get(cx: &App) -> Option<Entity<Favorites>> {
        cx.try_global::<FavoritesGlobal>().map(|g| g.0.clone())
    }
}

/// Reads the file at `path` and makes it the favorites of the window. Call it once, at start.
pub fn install(cx: &mut App, path: Option<PathBuf>) -> Entity<Favorites> {
    let (list, note) = match &path {
        Some(p) => read(p),
        None => (Vec::new(), None),
    };
    let favorites = cx.new(|_| Favorites { path, list, note });
    cx.set_global(FavoritesGlobal(favorites.clone()));
    favorites
}

impl Favorites {
    fn changed(&mut self, cx: &mut Context<Self>) {
        if let Some(path) = &self.path {
            self.note = write(path, &self.list).err();
        }
        cx.notify();
    }

    /// Reads the file again, for an edit by hand.
    pub fn reload(&mut self, cx: &mut Context<Self>) {
        (self.list, self.note) = match &self.path {
            Some(p) => read(p),
            None => (Vec::new(), None),
        };
        cx.notify();
    }

    /// Adds `favorite` at the end. An empty label takes the target as its label.
    pub fn add(&mut self, mut favorite: Favorite, cx: &mut Context<Self>) {
        favorite.label =
            clean_label(&favorite.label).unwrap_or_else(|| default_label(&favorite.saved));
        self.list.push(favorite);
        self.changed(cx);
    }

    /// An empty label keeps the old one.
    pub fn rename(&mut self, at: usize, label: &str, cx: &mut Context<Self>) {
        let Some(label) = clean_label(label) else {
            return;
        };
        if let Some(f) = self.list.get_mut(at)
            && f.label != label
        {
            f.label = label;
            self.changed(cx);
        }
    }

    pub fn remove(&mut self, at: usize, cx: &mut Context<Self>) {
        if at < self.list.len() {
            self.list.remove(at);
            self.changed(cx);
        }
    }

    /// Moves the favorite at `from` to the place of the favorite at `to`.
    pub fn move_to(&mut self, from: usize, to: usize, cx: &mut Context<Self>) {
        if from == to || from >= self.list.len() || to >= self.list.len() {
            return;
        }
        let f = self.list.remove(from);
        self.list.insert(to, f);
        self.changed(cx);
    }
}

/// The label of a favorite that the owner did not name.
pub fn default_label(saved: &Saved) -> String {
    let text = match saved {
        Saved::Screen(kind, arg) => match arg {
            Some(a) => format!("{} {a}", kind.title(None)),
            None => kind.title(None),
        },
        Saved::Folder(path) => path
            .rsplit('/')
            .find(|s| !s.is_empty())
            .unwrap_or(path)
            .to_string(),
        Saved::Command { argv, .. } => join_words(argv),
    };
    clean_label(&text).unwrap_or_else(|| "Favorite".into())
}

/// The file in `path`, and one line for the owner when something in it cannot be used.
pub fn read(path: &Path) -> (Vec<Favorite>, Option<String>) {
    match std::fs::read_to_string(path) {
        Ok(text) => parse(&text, path),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => (Vec::new(), None),
        Err(e) => (
            Vec::new(),
            Some(format!("{} cannot be read: {e}.", path.display())),
        ),
    }
}

fn parse(text: &str, path: &Path) -> (Vec<Favorite>, Option<String>) {
    let doc: DocumentMut = match text.parse() {
        Ok(d) => d,
        Err(e) => {
            let e: toml_edit::TomlError = e;
            return (
                Vec::new(),
                Some(format!(
                    "{} does not parse: {}. The favorites stay off until you fix it.",
                    path.display(),
                    e.message()
                )),
            );
        }
    };
    let mut problems = Vec::new();
    let mut out = Vec::new();
    if let Some(entries) = doc.get(ENTRY).and_then(Item::as_array_of_tables) {
        for (n, t) in entries.iter().enumerate() {
            match entry(t) {
                Ok(f) => out.push(f),
                Err(why) => problems.push(format!("favorite {}: {why}.", n + 1)),
            }
        }
    }
    let note = (!problems.is_empty()).then(|| {
        format!(
            "{}: {} The app skips each of these.",
            path.display(),
            problems.join(" ")
        )
    });
    (out, note)
}

fn entry(t: &Table) -> Result<Favorite, String> {
    from_fields(|k| t.get(k).and_then(Item::as_str))
}

/// The favorite of the keys of one entry: `label`, `product`, `kind`, `target` and `folder`.
/// The recent targets of `layout.json` use the same keys.
pub fn from_fields<'a>(text: impl Fn(&str) -> Option<&'a str>) -> Result<Favorite, String> {
    let target = text("target")
        .filter(|s| !s.trim().is_empty())
        .ok_or("it has no target")?;
    let saved = match text("kind") {
        Some("screen") => {
            let (name, arg) = match target.split_once(':') {
                Some((n, a)) => (n, Some(a.to_string())),
                None => (target, None),
            };
            let kind =
                ScreenKind::parse(name).ok_or_else(|| format!("\"{name}\" is not a screen"))?;
            Saved::Screen(kind, arg)
        }
        Some("folder") => Saved::Folder(target.to_string()),
        Some("command") => {
            let argv = split_words(target).ok_or("its command has an open quote")?;
            Saved::Command {
                argv,
                folder: text("folder")
                    .ok_or("its command has no folder")?
                    .to_string(),
            }
        }
        Some(other) => return Err(format!("kind \"{other}\" is not screen, folder or command")),
        None => return Err("it has no kind".into()),
    };
    let product = text("product")
        .filter(|p| !p.is_empty())
        .map(look::product_of_key);
    if matches!(saved, Saved::Screen(kind, _) if product.is_none() && !kind.of_no_product()) {
        return Err("its screen needs a product".into());
    }
    let label = text("label")
        .and_then(clean_label)
        .unwrap_or_else(|| default_label(&saved));
    Ok(Favorite {
        label,
        product,
        saved,
    })
}

impl Favorite {
    /// The `kind`, the `target` and the `folder` of the entry of this favorite.
    pub fn fields(&self) -> (&'static str, String, Option<String>) {
        let folder = match &self.saved {
            Saved::Command { folder, .. } => Some(folder.clone()),
            _ => None,
        };
        (self.saved.kind(), self.saved.target(), folder)
    }
}

/// Writes the list into the file in place: the entries change, the other keys of the file stay,
/// and so do the keys of an entry that the app does not know.
fn write(path: &Path, list: &[Favorite]) -> Result<(), String> {
    crate::keys::edit(path, |doc| {
        let old: Vec<Table> = doc
            .get(ENTRY)
            .and_then(Item::as_array_of_tables)
            .map(|a| a.iter().cloned().collect())
            .unwrap_or_default();
        let mut array = ArrayOfTables::new();
        for f in list {
            // An entry keeps the keys that the app does not know, from the old entry with the
            // same target. A new table, so the order of the list is the order of the file.
            let mut t = Table::new();
            let base = old.iter().find(|t| {
                t.get("kind").and_then(Item::as_str) == Some(f.saved.kind())
                    && t.get("target").and_then(Item::as_str) == Some(&f.saved.target())
            });
            for (k, v) in base.into_iter().flat_map(|b| b.iter()) {
                if !KNOWN.contains(&k) {
                    t.insert(k, v.clone());
                }
            }
            t.insert("label", value(f.label.as_str()));
            match &f.product {
                Some(p) => t.insert("product", value(look::key(p.host.as_deref(), &p.id))),
                None => t.remove("product"),
            };
            t.insert("kind", value(f.saved.kind()));
            t.insert("target", value(f.saved.target()));
            match &f.saved {
                Saved::Command { folder, .. } => t.insert("folder", value(folder.as_str())),
                _ => t.remove("folder"),
            };
            array.push(t);
        }
        // An entry that the app skipped at the read stays for the owner to fix.
        for t in old.iter().filter(|t| entry(t).is_err()) {
            let mut kept = Table::new();
            for (k, v) in t.iter() {
                kept.insert(k, v.clone());
            }
            array.push(kept);
        }
        if array.is_empty() {
            doc.remove(ENTRY);
        } else {
            doc.insert(ENTRY, Item::ArrayOfTables(array));
        }
        Ok(())
    })
}

/// The words as a shell reads them back: a word with a space or a quote goes in single quotes.
pub fn join_words(argv: &[String]) -> String {
    argv.iter()
        .map(|w| {
            let plain = !w.is_empty()
                && w.chars()
                    .all(|c| c.is_ascii_alphanumeric() || "-_./=:,+@%".contains(c));
            if plain {
                w.clone()
            } else {
                format!("'{}'", w.replace('\'', r"'\''"))
            }
        })
        .collect::<Vec<_>>()
        .join(" ")
}

/// Splits a command line into words: spaces part words, single quotes keep a text as it is,
/// double quotes keep spaces, and a backslash outside single quotes keeps the next character.
/// An open quote gives `None`.
pub fn split_words(line: &str) -> Option<Vec<String>> {
    let mut out = Vec::new();
    let mut word = String::new();
    let mut in_word = false;
    let mut chars = line.chars();
    while let Some(c) = chars.next() {
        match c {
            '\'' => {
                in_word = true;
                loop {
                    match chars.next()? {
                        '\'' => break,
                        c => word.push(c),
                    }
                }
            }
            '"' => {
                in_word = true;
                loop {
                    match chars.next()? {
                        '"' => break,
                        '\\' => word.push(chars.next()?),
                        c => word.push(c),
                    }
                }
            }
            '\\' => {
                in_word = true;
                word.push(chars.next()?);
            }
            c if c.is_whitespace() => {
                if in_word {
                    out.push(std::mem::take(&mut word));
                    in_word = false;
                }
            }
            c => {
                in_word = true;
                word.push(c);
            }
        }
    }
    if in_word {
        out.push(word);
    }
    Some(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn file(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("agentsi-fav-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        dir.join(FILE_NAME)
    }

    fn mine(id: &str) -> Option<ProductRef> {
        Some(ProductRef {
            host: None,
            id: id.into(),
        })
    }

    #[test]
    fn the_three_kinds_parse_and_a_bad_entry_is_skipped_with_one_note() {
        let text = r#"
[[favorite]]
label = "Desk"
product = "demo"
kind = "screen"
target = "desk"

[[favorite]]
product = "@studio/web"
kind = "screen"
target = "story:S1"

[[favorite]]
label = "Src"
kind = "folder"
target = "/home/me/src"

[[favorite]]
label = "Build"
product = "demo"
kind = "command"
target = "cargo build --features 'a b'"
folder = "/home/me/demo"

[[favorite]]
kind = "screen"
target = "nowhere"

[[favorite]]
kind = "command"
target = "make"

[[favorite]]
kind = "screen"
target = "desk"
"#;
        let (list, note) = parse(text, Path::new("favorites.toml"));
        assert_eq!(list.len(), 4);
        assert_eq!(list[0].label, "Desk");
        assert_eq!(list[0].saved, Saved::Screen(ScreenKind::Desk, None));
        assert_eq!(list[1].label, "Story S1", "no label takes the target");
        assert_eq!(
            list[1].product,
            Some(ProductRef {
                host: Some("studio".into()),
                id: "web".into()
            })
        );
        assert_eq!(
            list[1].screen().unwrap(),
            ScreenSpec::story("web", "S1").on(Some("studio".into()))
        );
        assert_eq!(list[2].product, None);
        assert_eq!(list[2].saved, Saved::Folder("/home/me/src".into()));
        assert_eq!(
            list[3].saved,
            Saved::Command {
                argv: vec![
                    "cargo".into(),
                    "build".into(),
                    "--features".into(),
                    "a b".into()
                ],
                folder: "/home/me/demo".into(),
            }
        );
        let note = note.unwrap();
        assert!(
            note.contains("favorite 5: \"nowhere\" is not a screen"),
            "{note}"
        );
        assert!(
            note.contains("favorite 6: its command has no folder"),
            "{note}"
        );
        assert!(
            note.contains("favorite 7: its screen needs a product"),
            "{note}"
        );
        assert_eq!(note.lines().count(), 1);
        let (none, note) = parse("[[favorite]\n", Path::new("favorites.toml"));
        assert!(none.is_empty());
        assert!(note.unwrap().contains("does not parse"));
    }

    #[test]
    fn a_write_keeps_the_order_the_unknown_keys_and_reads_back_the_same() {
        let path = file("write");
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(
            &path,
            "# mine\ntheme = 1\n\n[[favorite]]\nlabel = \"Src\"\nkind = \"folder\"\ntarget = \"/src\"\nicon = \"star\"\n\n[[favorite]]\nkind = \"portal\"\ntarget = \"x\"\n",
        )
        .unwrap();
        let (mut list, note) = read(&path);
        assert!(note.unwrap().contains("portal"));
        list.insert(
            0,
            Favorite {
                label: "Claude".into(),
                product: mine("demo"),
                saved: Saved::Command {
                    argv: vec!["claude".into(), "it's done".into()],
                    folder: "/demo".into(),
                },
            },
        );
        write(&path, &list).unwrap();
        let text = std::fs::read_to_string(&path).unwrap();
        assert!(
            text.contains("# mine") && text.contains("theme = 1"),
            "{text}"
        );
        assert!(text.contains("icon = \"star\""), "{text}");
        assert!(text.find("Claude") < text.find("Src"), "{text}");
        assert!(
            text.contains("kind = \"portal\""),
            "a skipped entry stays: {text}"
        );
        let (back, _) = read(&path);
        assert_eq!(back, list);
        std::fs::write(&path, "").unwrap();
        write(&path, &[]).unwrap();
        assert!(!std::fs::read_to_string(&path).unwrap().contains("favorite"));
        let _ = std::fs::remove_dir_all(path.parent().unwrap());
    }

    #[test]
    fn a_file_that_does_not_parse_is_not_overwritten() {
        let path = file("bad");
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(&path, "[[favorite]\n").unwrap();
        assert!(write(&path, &[]).is_err());
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "[[favorite]\n");
        let _ = std::fs::remove_dir_all(path.parent().unwrap());
    }

    #[test]
    fn words_split_as_a_shell_splits_them_and_join_back() {
        assert_eq!(
            split_words(r#"a  'b c' "d \"e\"" f\ g ''"#).unwrap(),
            ["a", "b c", "d \"e\"", "f g", ""]
        );
        assert_eq!(split_words("open 'quote"), None);
        for argv in [
            vec!["claude".to_string()],
            vec!["sh".into(), "-c".into(), "echo 'hi' && ls".into()],
            vec!["".into(), "x".into()],
        ] {
            assert_eq!(split_words(&join_words(&argv)).unwrap(), argv);
        }
    }

    #[gpui::test]
    fn the_entity_adds_renames_moves_and_removes_and_saves_each_change(
        cx: &mut gpui::TestAppContext,
    ) {
        let path = file("entity");
        let favorites = cx.update(|cx| install(cx, Some(path.clone())));
        let add = |label: &str, target: &str, cx: &mut gpui::TestAppContext| {
            let f = Favorite {
                label: label.into(),
                product: None,
                saved: Saved::Folder(target.into()),
            };
            favorites.update(cx, |l, cx| l.add(f, cx));
        };
        add("", "/home/me/src/", cx);
        add("Two", "/two", cx);
        add("Three", "/three", cx);
        let labels = |cx: &mut gpui::TestAppContext| {
            favorites.read_with(cx, |l, _| {
                l.list.iter().map(|f| f.label.clone()).collect::<Vec<_>>()
            })
        };
        assert_eq!(labels(cx), ["src", "Two", "Three"]);
        favorites.update(cx, |l, cx| l.move_to(2, 0, cx));
        favorites.update(cx, |l, cx| l.rename(1, "  First  ", cx));
        favorites.update(cx, |l, cx| l.rename(1, "", cx));
        assert_eq!(labels(cx), ["Three", "First", "Two"]);
        favorites.update(cx, |l, cx| l.remove(2, cx));
        let (back, _) = read(&path);
        let saved: Vec<_> = back.iter().map(|f| f.label.as_str()).collect();
        assert_eq!(saved, ["Three", "First"]);
        let _ = std::fs::remove_dir_all(path.parent().unwrap());
    }
}
