//! The git config of a product. `<config>/agentsi/git.toml` holds a `[shared]` table and one
//! `[product.<id>]` table for each override. The agency gives the result to git through the
//! environment (`GIT_CONFIG_COUNT`, `GIT_CONFIG_KEY_<n>`, `GIT_CONFIG_VALUE_<n>`), so it wins
//! over the repo, global and system files, and AgentSI writes none of them.
//!
//! Each git call of a product, each pane, story session and meeting process of a product
//! takes `env_for_cwd` or `env_for_product`. Spec: `docs/specs/agentsi-desktop-git.md`.

use crate::registry::Registry;
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use toml_edit::{DocumentMut, Item, Table, value};

pub const FILE: &str = "git.toml";

/// The tables of the file, with the keys in file order.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct GitConfig {
    pub shared: Vec<(String, String)>,
    pub products: BTreeMap<String, Vec<(String, String)>>,
}

/// One key of the result for a product.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Resolved {
    pub key: String,
    pub value: String,
    pub from_product: bool,
}

/// `<config>/agentsi/git.toml`, where `<config>` is `$XDG_CONFIG_HOME` or `~/.config`.
pub fn path() -> Option<PathBuf> {
    let base = std::env::var_os("XDG_CONFIG_HOME")
        .filter(|v| !v.is_empty())
        .map(PathBuf::from)
        .or_else(|| {
            std::env::var_os("HOME")
                .filter(|v| !v.is_empty())
                .map(|h| PathBuf::from(h).join(".config"))
        })?;
    Some(base.join("agentsi").join(FILE))
}

fn strings(table: &Table) -> Vec<(String, String)> {
    table
        .iter()
        .filter_map(|(k, v)| v.as_str().map(|s| (k.to_string(), s.to_string())))
        .collect()
}

impl GitConfig {
    pub fn parse(text: &str) -> Result<GitConfig, String> {
        let doc: DocumentMut = text
            .parse()
            .map_err(|e| format!("{FILE} does not parse: {e}"))?;
        let mut config = GitConfig::default();
        if let Some(t) = doc.get("shared").and_then(Item::as_table) {
            config.shared = strings(t);
        }
        if let Some(products) = doc.get("product").and_then(Item::as_table) {
            for (id, item) in products.iter() {
                if let Some(t) = item.as_table() {
                    config.products.insert(id.to_string(), strings(t));
                }
            }
        }
        Ok(config)
    }

    /// The file at `path`. A missing file gives empty tables.
    pub fn read(path: &Path) -> Result<GitConfig, String> {
        match std::fs::read_to_string(path) {
            Ok(text) => GitConfig::parse(&text),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(GitConfig::default()),
            Err(e) => Err(format!("cannot read {}: {e}", path.display())),
        }
    }

    pub fn product(&self, id: &str) -> &[(String, String)] {
        self.products.get(id).map_or(&[], Vec::as_slice)
    }

    /// The keys for a product: its own table over `[shared]`. An empty value in the product
    /// table removes the shared key for that product.
    pub fn resolve(&self, product: &str) -> Vec<Resolved> {
        let mut out: Vec<Resolved> = self
            .shared
            .iter()
            .filter(|(_, v)| !v.is_empty())
            .map(|(k, v)| Resolved {
                key: k.clone(),
                value: v.clone(),
                from_product: false,
            })
            .collect();
        for (key, val) in self.product(product) {
            let at = out.iter().position(|r| same_key(&r.key, key));
            match (at, val.is_empty()) {
                (Some(i), true) => {
                    out.remove(i);
                }
                (None, true) => {}
                (Some(i), false) => {
                    out[i] = Resolved {
                        key: key.clone(),
                        value: val.clone(),
                        from_product: true,
                    }
                }
                (None, false) => out.push(Resolved {
                    key: key.clone(),
                    value: val.clone(),
                    from_product: true,
                }),
            }
        }
        out
    }
}

/// Git compares the section and the variable name without case, and the subsection with case.
fn norm(key: &str) -> String {
    match (key.find('.'), key.rfind('.')) {
        (Some(a), Some(b)) if a < b => format!(
            "{}{}{}",
            key[..a].to_ascii_lowercase(),
            &key[a..=b],
            key[b + 1..].to_ascii_lowercase()
        ),
        _ => key.to_ascii_lowercase(),
    }
}

fn same_key(a: &str, b: &str) -> bool {
    norm(a) == norm(b)
}

/// The variables that give `entries` to git. They come after the entries that the
/// environment already holds, so those stay.
pub fn env_pairs(entries: &[(String, String)], inherited: usize) -> Vec<(String, String)> {
    if entries.is_empty() {
        return Vec::new();
    }
    let mut out = vec![(
        "GIT_CONFIG_COUNT".to_string(),
        (inherited + entries.len()).to_string(),
    )];
    for (i, (k, v)) in entries.iter().enumerate() {
        let n = inherited + i;
        out.push((format!("GIT_CONFIG_KEY_{n}"), k.clone()));
        out.push((format!("GIT_CONFIG_VALUE_{n}"), v.clone()));
    }
    out
}

fn inherited_count() -> usize {
    std::env::var("GIT_CONFIG_COUNT")
        .ok()
        .and_then(|v| v.trim().parse().ok())
        .unwrap_or(0)
}

/// The variables for a product, from the file at `file`.
pub fn env_in(file: &Path, product: &str) -> Vec<(String, String)> {
    let Ok(config) = GitConfig::read(file) else {
        return Vec::new();
    };
    let entries: Vec<(String, String)> = config
        .resolve(product)
        .into_iter()
        .map(|r| (r.key, r.value))
        .collect();
    env_pairs(&entries, inherited_count())
}

/// The variables for a product, from the file of this machine.
pub fn env_for_product(product: &str) -> Vec<(String, String)> {
    match path().filter(|p| p.exists()) {
        Some(file) => env_in(&file, product),
        None => Vec::new(),
    }
}

/// The variables for the product whose folder holds `cwd`. A folder of no product, and a
/// machine with no `git.toml`, give none.
pub fn env_for_cwd(cwd: &Path) -> Vec<(String, String)> {
    let Some(file) = path().filter(|p| p.exists()) else {
        return Vec::new();
    };
    let Ok(root) = agentsi_store::paths::data_root(None) else {
        return Vec::new();
    };
    let Ok(registry) = Registry::load(&root) else {
        return Vec::new();
    };
    match registry.detect(cwd) {
        Some(p) => env_in(&file, &p.id),
        None => Vec::new(),
    }
}

/// `section.name` or `section.subsection.name`.
pub fn check_key(key: &str) -> Result<(), String> {
    let bad = |why: &str| Err(format!("\"{key}\" is not a git config key: {why}."));
    let (Some(a), Some(b)) = (key.find('.'), key.rfind('.')) else {
        return bad("write section.name");
    };
    let (section, name) = (&key[..a], &key[b + 1..]);
    let word = |s: &str| {
        s.chars().next().is_some_and(|c| c.is_ascii_alphabetic())
            && s.chars().all(|c| c.is_ascii_alphanumeric() || c == '-')
    };
    if !word(section) || !word(name) {
        return bad("the section and the name use letters, digits and dashes");
    }
    if a < b && key[a + 1..b].chars().any(|c| c == '\n' || c == '\0') {
        return bad("the subsection holds a control character");
    }
    Ok(())
}

static WRITE: Mutex<()> = Mutex::new(());

/// Replaces one table of the file and keeps the other tables and their comments. The write
/// goes through a temporary file. An empty `entries` removes the table.
pub fn set_table(
    file: &Path,
    product: Option<&str>,
    entries: &[(String, String)],
) -> Result<(), String> {
    let mut seen: Vec<String> = Vec::new();
    for (k, v) in entries {
        check_key(k)?;
        if v.contains('\0') {
            return Err(format!("The value of {k} holds a NUL character."));
        }
        if product.is_none() && v.is_empty() {
            return Err(format!(
                "The shared value of {k} is empty. An empty value works only in the table of a product."
            ));
        }
        let n = norm(k);
        if seen.contains(&n) {
            return Err(format!("{k} is in the table two times."));
        }
        seen.push(n);
    }
    let _guard = WRITE.lock().unwrap_or_else(|p| p.into_inner());
    let text = match std::fs::read_to_string(file) {
        Ok(t) => t,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => String::new(),
        Err(e) => return Err(format!("cannot read {}: {e}", file.display())),
    };
    let mut doc: DocumentMut = text
        .parse()
        .map_err(|e| format!("{FILE} does not parse: {e}"))?;
    let table = match product {
        None => {
            if entries.is_empty() {
                doc.remove("shared");
                None
            } else {
                Some(table_of(&mut doc, &["shared"])?)
            }
        }
        Some(id) => {
            if entries.is_empty() {
                if let Some(t) = doc.get_mut("product").and_then(Item::as_table_mut) {
                    t.remove(id);
                    if t.is_empty() {
                        doc.remove("product");
                    }
                }
                None
            } else {
                Some(table_of(&mut doc, &["product", id])?)
            }
        }
    };
    if let Some(table) = table {
        let old: Vec<String> = table.iter().map(|(k, _)| k.to_string()).collect();
        for k in old {
            if !entries.iter().any(|(n, _)| *n == k) {
                table.remove(&k);
            }
        }
        for (k, v) in entries {
            if table.get(k).and_then(Item::as_str) != Some(v.as_str()) {
                table.insert(k, value(v.as_str()));
            }
        }
    }
    write_atomic(file, &doc.to_string())
}

/// The table at `names` inside the document. A table that the code makes stays implicit
/// above, so only `[shared]` and `[product.<id>]` show in the file.
fn table_of<'a>(doc: &'a mut DocumentMut, names: &[&str]) -> Result<&'a mut Table, String> {
    let mut table = doc.as_table_mut();
    for (depth, name) in names.iter().enumerate() {
        if !table.contains_key(name) {
            let mut t = Table::new();
            t.set_implicit(depth + 1 < names.len());
            table.insert(name, Item::Table(t));
        }
        table = table
            .get_mut(name)
            .and_then(Item::as_table_mut)
            .ok_or_else(|| format!("{name} in {FILE} is not a table."))?;
    }
    Ok(table)
}

fn write_atomic(file: &Path, text: &str) -> Result<(), String> {
    let fail = |e: std::io::Error| format!("cannot write {}: {e}", file.display());
    if let Some(dir) = file.parent() {
        std::fs::create_dir_all(dir).map_err(fail)?;
    }
    let mut tmp = file.as_os_str().to_owned();
    tmp.push(format!(".tmp{}", std::process::id()));
    let tmp = PathBuf::from(tmp);
    std::fs::write(&tmp, text).map_err(fail)?;
    std::fs::rename(&tmp, file).map_err(|e| {
        let _ = std::fs::remove_file(&tmp);
        fail(e)
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pairs(v: &[(&str, &str)]) -> Vec<(String, String)> {
        v.iter()
            .map(|(k, v)| (k.to_string(), v.to_string()))
            .collect()
    }

    const FILE_TEXT: &str = "# the owner's notes\n[shared]\n\"user.name\" = \"Ada\"\n\"commit.gpgsign\" = \"false\"\n\n[product.alpha]\n\"user.email\" = \"a@work.example\"\n\"commit.gpgsign\" = \"\"\n\n# keep me\n[product.beta]\n\"user.name\" = \"Bo\"\n";

    #[test]
    fn a_product_table_goes_over_the_shared_one() {
        let c = GitConfig::parse(FILE_TEXT).unwrap();
        let alpha: Vec<_> = c
            .resolve("alpha")
            .into_iter()
            .map(|r| (r.key, r.value, r.from_product))
            .collect();
        // The empty value removed commit.gpgsign for alpha only.
        assert_eq!(
            alpha,
            vec![
                ("user.name".into(), "Ada".into(), false),
                ("user.email".into(), "a@work.example".into(), true)
            ]
        );
        let beta = c.resolve("beta");
        assert_eq!(beta.len(), 2);
        assert!(beta.iter().any(|r| r.key == "commit.gpgsign"));
        assert_eq!(
            beta.iter().find(|r| r.key == "user.name").unwrap().value,
            "Bo"
        );
        assert_eq!(c.resolve("nobody").len(), 2);
    }

    #[test]
    fn the_keys_compare_the_way_git_does() {
        assert!(same_key("User.Email", "user.email"));
        assert!(same_key(
            "url.https://X.example/.insteadOf",
            "url.https://X.example/.insteadof"
        ));
        assert!(!same_key(
            "url.https://X.example/.insteadOf",
            "url.https://x.example/.insteadOf"
        ));
    }

    #[test]
    fn the_variables_follow_the_count_of_the_environment() {
        assert!(env_pairs(&[], 0).is_empty());
        let got = env_pairs(&pairs(&[("user.name", "A"), ("user.email", "e")]), 1);
        assert_eq!(
            got,
            pairs(&[
                ("GIT_CONFIG_COUNT", "3"),
                ("GIT_CONFIG_KEY_1", "user.name"),
                ("GIT_CONFIG_VALUE_1", "A"),
                ("GIT_CONFIG_KEY_2", "user.email"),
                ("GIT_CONFIG_VALUE_2", "e"),
            ])
        );
    }

    #[test]
    fn a_write_replaces_one_table_and_keeps_the_comments_of_the_rest() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("agentsi").join(FILE);
        std::fs::create_dir_all(file.parent().unwrap()).unwrap();
        std::fs::write(&file, FILE_TEXT).unwrap();
        set_table(
            &file,
            Some("alpha"),
            &pairs(&[("user.email", "new@work.example")]),
        )
        .unwrap();
        let text = std::fs::read_to_string(&file).unwrap();
        assert!(text.contains("# the owner's notes"), "{text}");
        assert!(text.contains("# keep me"), "{text}");
        assert!(text.contains("new@work.example"), "{text}");
        assert!(!text.contains("a@work.example"), "{text}");
        let c = GitConfig::parse(&text).unwrap();
        assert_eq!(
            c.product("alpha"),
            pairs(&[("user.email", "new@work.example")])
        );
        assert_eq!(c.product("beta"), pairs(&[("user.name", "Bo")]));
        assert_eq!(c.shared.len(), 2);
        let leftovers: Vec<_> = std::fs::read_dir(file.parent().unwrap())
            .unwrap()
            .map(|e| e.unwrap().file_name())
            .collect();
        assert_eq!(leftovers.len(), 1, "{leftovers:?}");
    }

    #[test]
    fn a_new_file_gets_the_tables_and_an_empty_list_removes_one() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("deep").join(FILE);
        set_table(&file, None, &pairs(&[("user.name", "Ada")])).unwrap();
        set_table(
            &file,
            Some("alpha"),
            &pairs(&[("user.email", "e@x.example")]),
        )
        .unwrap();
        let c = GitConfig::read(&file).unwrap();
        assert_eq!(c.shared, pairs(&[("user.name", "Ada")]));
        assert_eq!(c.product("alpha"), pairs(&[("user.email", "e@x.example")]));
        let text = std::fs::read_to_string(&file).unwrap();
        assert!(text.contains("[product.alpha]"), "{text}");
        set_table(&file, Some("alpha"), &[]).unwrap();
        let c = GitConfig::read(&file).unwrap();
        assert!(c.products.is_empty());
        assert_eq!(c.shared.len(), 1);
    }

    #[test]
    fn a_bad_key_or_an_empty_shared_value_is_refused() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join(FILE);
        for bad in ["name", "user name.x", ".x", "user.", "a.b c"] {
            assert!(
                set_table(&file, None, &pairs(&[(bad, "v")])).is_err(),
                "{bad}"
            );
        }
        assert!(set_table(&file, None, &pairs(&[("user.name", "")])).is_err());
        assert!(
            set_table(
                &file,
                None,
                &pairs(&[("user.name", "a"), ("User.Name", "b")])
            )
            .is_err()
        );
        assert!(set_table(&file, Some("p"), &pairs(&[("user.name", "")])).is_ok());
        assert!(!dir.path().join("x").exists());
    }

    #[test]
    fn a_broken_file_gives_no_variables() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join(FILE);
        std::fs::write(&file, "[shared\n").unwrap();
        assert!(GitConfig::read(&file).is_err());
        assert!(env_in(&file, "alpha").is_empty());
    }
}
