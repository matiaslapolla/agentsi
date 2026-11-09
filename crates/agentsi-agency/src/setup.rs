//! The setup screen: `setup.scan` reads a product folder with code only. It writes
//! nothing and calls no model.

use crate::registry::Registry;
use agentsi_proto::setup_api::{Manifest, ScanTools, SetupScanResult};
use serde_json::Value;
use std::path::Path;

/// The manifests in the order that the scan tries them. The first one found wins.
const MANIFESTS: [&str; 4] = [
    "package.json",
    "Cargo.toml",
    "pyproject.toml",
    "composer.json",
];

/// The id that the folder name proposes: lowercase, with `-` for each character that an
/// id does not permit.
pub fn proposed_id(name: &str) -> String {
    name.chars()
        .flat_map(char::to_lowercase)
        .map(|c| {
            if c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-' {
                c
            } else {
                '-'
            }
        })
        .collect()
}

/// The package manager of the folder: the command that runs the test script with more
/// arguments, and the command that runs a script.
fn js_runner(dir: &Path) -> (&'static str, &'static str) {
    let has = |f: &str| dir.join(f).exists();
    if has("pnpm-lock.yaml") {
        ("pnpm test", "pnpm")
    } else if has("yarn.lock") {
        ("yarn test", "yarn")
    } else if has("bun.lockb") || has("bun.lock") {
        ("bun run test", "bun run")
    } else {
        ("npm test --", "npm run")
    }
}

/// The scripts that a gate runs, in this order, when the manifest has them.
const GATE_SCRIPTS: [&str; 5] = ["typecheck", "type-check", "check-types", "lint", "check"];

fn package_json(dir: &Path, text: &str) -> Manifest {
    let doc: Value = serde_json::from_str(text).unwrap_or_default();
    let scripts = doc.get("scripts").and_then(Value::as_object);
    let has = |name: &str| scripts.is_some_and(|s| s.contains_key(name));
    let (test_cmd, run) = js_runner(dir);
    let test = has("test").then(|| format!("{test_cmd} {{files}}"));
    Manifest {
        file: "package.json".into(),
        test,
        gate: GATE_SCRIPTS
            .iter()
            .filter(|s| has(s))
            .map(|s| format!("{run} {s}"))
            .collect(),
    }
}

fn cargo_toml() -> Manifest {
    // cargo test takes one filter, not a list of files, so the scan proposes no test command.
    Manifest {
        file: "Cargo.toml".into(),
        test: None,
        gate: vec![
            "cargo test".into(),
            "cargo clippy --all-targets -- -D warnings".into(),
            "cargo fmt --check".into(),
        ],
    }
}

fn pyproject(text: &str) -> Manifest {
    let mut gate = Vec::new();
    if text.contains("ruff") {
        gate.push("ruff check .".to_string());
    }
    if text.contains("mypy") {
        gate.push("mypy .".to_string());
    }
    Manifest {
        file: "pyproject.toml".into(),
        test: text
            .contains("pytest")
            .then(|| "pytest {files}".to_string()),
        gate,
    }
}

fn composer(text: &str) -> Manifest {
    let doc: Value = serde_json::from_str(text).unwrap_or_default();
    let scripts = doc.get("scripts").and_then(Value::as_object);
    let has = |name: &str| scripts.is_some_and(|s| s.contains_key(name));
    let phpunit = text.contains("phpunit/phpunit");
    Manifest {
        file: "composer.json".into(),
        test: phpunit.then(|| "vendor/bin/phpunit {files}".to_string()),
        gate: ["phpstan", "lint", "cs"]
            .iter()
            .filter(|s| has(s))
            .map(|s| format!("composer {s}"))
            .collect(),
    }
}

/// The test command and the gate commands of the first manifest of the folder.
pub fn manifest(dir: &Path) -> Option<Manifest> {
    MANIFESTS.iter().find_map(|name| {
        let text = std::fs::read_to_string(dir.join(name)).ok()?;
        Some(match *name {
            "package.json" => package_json(dir, &text),
            "Cargo.toml" => cargo_toml(),
            "pyproject.toml" => pyproject(&text),
            _ => composer(&text),
        })
    })
}

/// The north-star candidates that exist: `docs/product.md`, `docs/vision*` and
/// `ROADMAP*`, as paths relative to the folder, sorted.
pub fn north_stars(dir: &Path) -> Vec<String> {
    let mut found = Vec::new();
    if dir.join("docs/product.md").is_file() {
        found.push("docs/product.md".to_string());
    }
    let starting = |folder: &Path, prefix: &str, rel: &str| -> Vec<String> {
        let Ok(entries) = std::fs::read_dir(folder) else {
            return Vec::new();
        };
        entries
            .filter_map(|e| e.ok()?.file_name().into_string().ok())
            .filter(|n| n.starts_with(prefix))
            .map(|n| format!("{rel}{n}"))
            .collect()
    };
    found.extend(starting(&dir.join("docs"), "vision", "docs/"));
    found.extend(starting(dir, "ROADMAP", ""));
    found.sort();
    found
}

/// True when `name` is an executable file in a folder of `PATH`.
pub fn on_path(name: &str) -> bool {
    use std::os::unix::fs::PermissionsExt;
    std::env::var_os("PATH").is_some_and(|paths| {
        std::env::split_paths(&paths).any(|d| {
            std::fs::metadata(d.join(name))
                .is_ok_and(|m| m.is_file() && m.permissions().mode() & 0o111 != 0)
        })
    })
}

/// Reads the folder. `registry` is `None` when the data root has no readable registry.
/// The caller checks that `dir` is an existing absolute folder.
pub fn scan(dir: &Path, registry: Option<&Registry>) -> SetupScanResult {
    let name = dir
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default();
    SetupScanResult {
        path: dir.display().to_string(),
        id: proposed_id(&name),
        name,
        registered: registry.and_then(|r| r.detect(dir)).map(|p| p.id.clone()),
        manifest: manifest(dir),
        north_stars: north_stars(dir),
        tools: ScanTools {
            bwrap: on_path("bwrap"),
            codebase_memory_mcp: on_path("codebase-memory-mcp"),
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_folder_name_gives_a_lowercase_id() {
        assert_eq!(proposed_id("My App_2"), "my-app-2");
        assert_eq!(proposed_id("agentsi"), "agentsi");
    }

    #[test]
    fn each_package_manager_gets_its_own_commands() {
        let dir = tempfile::tempdir().unwrap();
        let pkg = r#"{"scripts":{"test":"vitest","lint":"eslint .","typecheck":"tsc"}}"#;
        std::fs::write(dir.path().join("package.json"), pkg).unwrap();
        let npm = manifest(dir.path()).unwrap();
        assert_eq!(npm.test.as_deref(), Some("npm test -- {files}"));
        assert_eq!(npm.gate, ["npm run typecheck", "npm run lint"]);
        std::fs::write(dir.path().join("pnpm-lock.yaml"), "").unwrap();
        let pnpm = manifest(dir.path()).unwrap();
        assert_eq!(pnpm.test.as_deref(), Some("pnpm test {files}"));
        assert_eq!(pnpm.gate, ["pnpm typecheck", "pnpm lint"]);
    }

    #[test]
    fn cargo_python_and_composer_manifests_give_their_commands() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("Cargo.toml"), "[package]\nname = \"x\"\n").unwrap();
        let cargo = manifest(dir.path()).unwrap();
        assert_eq!((cargo.file.as_str(), cargo.test), ("Cargo.toml", None));
        std::fs::remove_file(dir.path().join("Cargo.toml")).unwrap();
        std::fs::write(
            dir.path().join("pyproject.toml"),
            "[tool.pytest.ini_options]\n[tool.ruff]\n",
        )
        .unwrap();
        let py = manifest(dir.path()).unwrap();
        assert_eq!(py.test.as_deref(), Some("pytest {files}"));
        assert_eq!(py.gate, ["ruff check ."]);
        std::fs::remove_file(dir.path().join("pyproject.toml")).unwrap();
        std::fs::write(
            dir.path().join("composer.json"),
            r#"{"require-dev":{"phpunit/phpunit":"^11"},"scripts":{"phpstan":"phpstan analyse"}}"#,
        )
        .unwrap();
        let php = manifest(dir.path()).unwrap();
        assert_eq!(php.test.as_deref(), Some("vendor/bin/phpunit {files}"));
        assert_eq!(php.gate, ["composer phpstan"]);
        std::fs::remove_file(dir.path().join("composer.json")).unwrap();
        assert!(manifest(dir.path()).is_none());
    }
}
