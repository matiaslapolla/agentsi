//! `<config>/agentsi/hosts.toml`: the SSH hosts of the app, with the last product
//! list of each host.
//!
//! The owner edits the file. The app writes only the `products` key of a host, and
//! keeps the comments and the order of the rest.
//!
//! ```toml
//! [[host]]
//! name = "studio"                     # the name in the sidebar, unique
//! ssh = "ana@studio.lan"              # the SSH destination, default: name
//! args = ["-p", "2222"]               # more ssh options, optional
//! command = "agentsi-core connect"    # the remote command, optional
//! ```

use std::path::{Path, PathBuf};
use std::sync::Mutex;

use agentsi_proto::desk_api::ProductInfo;
use serde::Deserialize;

/// The remote command when the file names none.
pub const CONNECT: &str = "agentsi-core connect";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HostConfig {
    pub name: String,
    pub ssh: String,
    pub args: Vec<String>,
    pub command: String,
    /// The product list of the last connection.
    pub products: Vec<ProductInfo>,
}

/// Unknown keys stay allowed: a key of a newer app must not turn off every host.
#[derive(Deserialize)]
struct File {
    #[serde(default)]
    host: Vec<Entry>,
}

#[derive(Deserialize)]
struct Entry {
    name: String,
    ssh: Option<String>,
    #[serde(default)]
    args: Vec<String>,
    command: Option<String>,
    #[serde(default)]
    products: Vec<ProductInfo>,
}

pub fn path() -> Option<PathBuf> {
    crate::layout::path().map(|p| p.with_file_name("hosts.toml"))
}

/// The hosts in `path`. A missing file gives no hosts. A file that does not parse is
/// an error with the reason.
pub fn read(path: &Path) -> Result<Vec<HostConfig>, String> {
    let text = match std::fs::read_to_string(path) {
        Ok(t) => t,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(e) => return Err(format!("cannot read {}: {e}", path.display())),
    };
    parse(&text).map_err(|e| format!("{}: {e}", path.display()))
}

pub fn parse(text: &str) -> Result<Vec<HostConfig>, String> {
    let file: File = toml::from_str(text).map_err(|e| e.message().to_string())?;
    let mut hosts: Vec<HostConfig> = Vec::new();
    for e in file.host {
        let name = e.name.trim().to_string();
        agentsi_proto::hosts_api::check_host(&name)?;
        if hosts.iter().any(|h| h.name == name) {
            return Err(format!("two hosts have the name {name:?}"));
        }
        hosts.push(HostConfig {
            ssh: e.ssh.unwrap_or_else(|| name.clone()),
            name,
            args: e.args,
            command: e.command.unwrap_or_else(|| CONNECT.into()),
            products: e.products,
        });
    }
    Ok(hosts)
}

/// The threads of the hosts write the file one at a time.
static WRITE: Mutex<()> = Mutex::new(());

/// Writes the product list of host `name` into the file, and leaves the rest as it is.
pub fn save_products(path: &Path, name: &str, products: &[ProductInfo]) -> Result<(), String> {
    let _one = WRITE.lock().unwrap_or_else(|p| p.into_inner());
    let text = std::fs::read_to_string(path).map_err(|e| e.to_string())?;
    let text = with_products(&text, name, products)?;
    crate::atomic::write(path, &text).map_err(|e| e.to_string())
}

/// `text` with the `products` of host `name` replaced.
pub fn with_products(text: &str, name: &str, products: &[ProductInfo]) -> Result<String, String> {
    use toml_edit::{Array, DocumentMut, InlineTable, value};
    let mut doc: DocumentMut = text
        .parse()
        .map_err(|e: toml_edit::TomlError| e.to_string())?;
    let hosts = doc
        .get_mut("host")
        .and_then(|h| h.as_array_of_tables_mut())
        .ok_or("the file has no [[host]] table")?;
    let host = hosts
        .iter_mut()
        .find(|t| t.get("name").and_then(|n| n.as_str()).map(str::trim) == Some(name))
        .ok_or_else(|| format!("the file has no host {name:?}"))?;
    let mut list = Array::new();
    for p in products {
        let mut t = InlineTable::new();
        t.insert("id", p.id.as_str().into());
        t.insert("name", p.name.as_str().into());
        if let Some(path) = &p.path {
            t.insert("path", path.as_str().into());
        }
        list.push(t);
    }
    host.insert("products", value(list));
    Ok(doc.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn product(id: &str, path: Option<&str>) -> ProductInfo {
        ProductInfo {
            id: id.into(),
            name: id.to_uppercase(),
            path: path.map(String::from),
            archived: false,
        }
    }

    #[test]
    fn a_host_gets_its_defaults_and_a_bad_file_is_an_error() {
        let hosts = parse("[[host]]\nname = \"studio\"\n").unwrap();
        assert_eq!(hosts[0].ssh, "studio");
        assert_eq!(hosts[0].command, CONNECT);
        assert!(hosts[0].products.is_empty());
        assert!(parse("").unwrap().is_empty());
        assert!(parse("[[host]]\nname = \"a\"\n[[host]]\nname = \"a\"\n").is_err());
        assert!(parse("[[host]]\nname = \"a/b\"\n").is_err());
        // A thread name cannot hold a NUL: the app panicked at start.
        assert!(parse("[[host]]\nname = \"a\\u0000b\"\n").is_err());
        assert_eq!(
            parse("[[host]]\nname = \"a\"\nport = 22\n").unwrap().len(),
            1
        );
    }

    #[test]
    fn saved_products_read_back_and_the_comments_stay() {
        let text = "# my hosts\n[[host]]\nname = \"a\" # first\nssh = \"me@a\"\n\n[[host]]\nname = \"b\"\n";
        let products = [product("x", Some("/srv/x")), product("y", None)];
        let out = with_products(text, "b", &products).unwrap();
        assert!(
            out.contains("# my hosts") && out.contains("# first"),
            "{out}"
        );
        let hosts = parse(&out).unwrap();
        assert!(hosts[0].products.is_empty());
        assert_eq!(hosts[1].products, products);
        // A second save replaces the list.
        let out = with_products(&out, "b", &products[..1]).unwrap();
        assert_eq!(parse(&out).unwrap()[1].products, products[..1]);
        assert!(with_products(text, "c", &products).is_err());
    }

    #[test]
    fn save_products_writes_the_file() {
        let dir = std::env::temp_dir().join(format!("agentsi-hosts-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("hosts.toml");
        std::fs::write(&path, "[[host]]\nname = \"a\"\n").unwrap();
        save_products(&path, "a", &[product("x", None)]).unwrap();
        assert_eq!(read(&path).unwrap()[0].products, [product("x", None)]);
        assert!(read(&dir.join("none.toml")).unwrap().is_empty());
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
