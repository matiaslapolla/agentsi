//! The setup screen. `setup.scan` reads a product folder with code only. `setup.draft`
//! reads at most 15 files and 60 KiB of it for one capped Claude call. The two append
//! their numbers to `logs/setup.jsonl`, and nothing else is written.

use crate::registry::Registry;
use crate::story::claude::Call;
use agentsi_proto::setup_api::{Manifest, ScanTools, SetupScanResult};
use serde_json::{Value, json};
use std::path::Path;
use std::time::Duration;

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

/// The cap of one setup draft, in USD.
pub const DRAFT_BUDGET_USD: f64 = 0.5;
/// The model of a setup draft.
pub const DRAFT_MODEL: &str = "sonnet";
/// At most this many files go into the context of a draft.
pub const MAX_FILES: usize = 15;
/// At most this many bytes of file text go into the context of a draft.
pub const MAX_BYTES: usize = 60 * 1024;
const DRAFT_TIMEOUT: Duration = Duration::from_secs(300);

/// The files of a folder that a draft reads, in the order of `setup.md` section 2:
/// the README, `CLAUDE.md`, the docs index, the manifests, the north-star docs, then the
/// idea briefs and the specs.
pub fn context_files(dir: &Path) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    let mut add = |rel: String| {
        if dir.join(&rel).is_file() && !out.contains(&rel) {
            out.push(rel);
        }
    };
    let sorted = |folder: &Path, keep: &dyn Fn(&str) -> bool| -> Vec<String> {
        let mut names: Vec<String> = std::fs::read_dir(folder)
            .map(|d| {
                d.filter_map(|e| e.ok()?.file_name().into_string().ok())
                    .filter(|n| keep(n))
                    .collect()
            })
            .unwrap_or_default();
        names.sort();
        names
    };
    for name in sorted(dir, &|n| n.to_ascii_uppercase().starts_with("README")) {
        add(name);
    }
    add("CLAUDE.md".into());
    for index in ["docs/README.md", "docs/index.md"] {
        add(index.into());
    }
    for name in [
        "package.json",
        "composer.json",
        "Cargo.toml",
        "pyproject.toml",
    ] {
        add(name.into());
    }
    for star in north_stars(dir) {
        add(star);
    }
    for folder in ["docs/ideas", "docs/specs"] {
        for name in sorted(&dir.join(folder), &|n| n.ends_with(".md")) {
            add(format!("{folder}/{name}"));
        }
    }
    out.truncate(MAX_FILES);
    out
}

/// What a draft reads from the folder.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Context {
    /// The files that went in, relative to the folder.
    pub files: Vec<String>,
    /// The bytes of file text that went in.
    pub bytes: usize,
    /// The text of the prompt: each file under its path, then the git log.
    pub text: String,
}

/// The last whole character at or before `at`.
fn floor_char(s: &str, mut at: usize) -> usize {
    while !s.is_char_boundary(at) {
        at -= 1;
    }
    at
}

/// Reads the files of `context_files` up to `MAX_BYTES`, and `git log --oneline -30`.
pub fn context(dir: &Path) -> Context {
    let mut c = Context::default();
    for rel in context_files(dir) {
        let Ok(text) = std::fs::read_to_string(dir.join(&rel)) else {
            continue;
        };
        let room = MAX_BYTES - c.bytes;
        if room == 0 {
            break;
        }
        let part = &text[..floor_char(&text, text.len().min(room))];
        c.bytes += part.len();
        c.text.push_str(&format!("## File {rel}\n\n{part}\n\n"));
        c.files.push(rel);
    }
    let log = std::process::Command::new("git")
        .args(["log", "--oneline", "-30"])
        .current_dir(dir)
        .envs(crate::git_env::env_for_cwd(dir))
        .output()
        .ok()
        .filter(|o| o.status.success())
        .map(|o| String::from_utf8_lossy(&o.stdout).into_owned());
    c.text.push_str(&format!(
        "## git log --oneline -30\n\n{}\n",
        log.as_deref().unwrap_or("no git history")
    ));
    c
}

/// The JSON schema of the answer: the five forms and the list of the fields with no data.
pub fn draft_schema() -> Value {
    let text = json!({"type": "string"});
    let texts = json!({"type": "array", "items": {"type": "string"}});
    let object = |props: Value, required: &[&str]| {
        json!({"type": "object", "properties": props, "required": required,
               "additionalProperties": false})
    };
    let product = object(
        json!({"id": text, "name": text, "what": text, "audience": text, "stage": text,
               "out_of_scope": texts}),
        &["id", "name", "what", "audience", "stage", "out_of_scope"],
    );
    let objective = object(
        json!({"text": text, "signal": text, "due": text}),
        &["text", "signal", "due"],
    );
    let source = object(
        json!({"kind": text, "ref": text, "note": text}),
        &["kind", "ref", "note"],
    );
    let objectives = object(
        json!({"objectives": {"type": "array", "items": objective, "maxItems": 3},
               "north_stars": texts, "sources_of_truth": {"type": "array", "items": source}}),
        &["objectives", "north_stars", "sources_of_truth"],
    );
    let specialist = object(
        json!({"id": text, "role": text, "overlay": text}),
        &["id", "role", "overlay"],
    );
    let why = object(json!({"agent": text, "why": text}), &["agent", "why"]);
    let roster = object(
        json!({"roles": texts, "specialists": {"type": "array", "items": specialist},
               "why": {"type": "array", "items": why}}),
        &["roles", "specialists", "why"],
    );
    let model = object(
        json!({"agent": text, "model": text, "effort": text}),
        &["agent", "model", "effort"],
    );
    let stories = object(
        json!({"on": {"type": "boolean"}, "test": text, "gate": texts}),
        &["on", "test", "gate"],
    );
    let fields = object(
        json!({"product": product, "objectives": objectives, "roster": roster,
               "models": {"type": "array", "items": model}, "stories": stories}),
        &["product", "objectives", "roster", "models", "stories"],
    );
    object(
        json!({"fields": fields, "no_data": texts}),
        &["fields", "no_data"],
    )
}

const SYSTEM: &str = "You draft the setup of a product for AgentSI, an agency of AI roles. \
You read the files of the product folder that the prompt holds. You answer only with the JSON \
of the schema. Write each value from the files alone. When the files do not answer a field, \
leave it empty and put its path in no_data, for example \"product.audience\" or \
\"objectives.objectives\". Never invent a value to fill a gap. The global roles are pm, social, \
dev, qa and scout. pm is always in the roster. Propose a specialist only for an area of the \
code that the files show. Leave models empty: the global roles set them. Turn stories on only \
when the files show a test runner and a gate that need no database and no network. A test \
command must contain {files}.";

/// The prompt of a draft. `previous` and `failures` ask for a repair of an earlier draft.
pub fn draft_prompt(
    dir: &Path,
    context: &Context,
    previous: Option<&Value>,
    failures: &[String],
) -> String {
    let mut prompt = format!(
        "Draft the setup of the product in {}.\n\n{}",
        dir.display(),
        context.text
    );
    if let Some(previous) = previous {
        prompt.push_str(&format!(
            "\n## Repair\n\nYour earlier draft was:\n\n{previous}\n\nThe check of AgentSI \
             refused it for these reasons:\n\n{}\n\nAnswer with a draft that fixes each reason.\n",
            failures
                .iter()
                .map(|f| format!("- {f}"))
                .collect::<Vec<_>>()
                .join("\n")
        ));
    }
    prompt
}

/// The one call of a draft: model `sonnet`, no tools, a cap of `DRAFT_BUDGET_USD`.
pub fn draft_call(dir: &Path, prompt: String) -> Call {
    Call {
        prompt,
        model: DRAFT_MODEL.into(),
        system: SYSTEM.into(),
        cwd: dir.display().to_string(),
        tools: Vec::new(),
        schema: Some(draft_schema()),
        effort: None,
        timeout: DRAFT_TIMEOUT,
        mcp: None,
        budget_usd: Some(DRAFT_BUDGET_USD),
    }
}

/// Appends one JSON line to `<data root>/logs/setup.jsonl`.
pub fn log_line(root: &Path, line: &Value) -> std::io::Result<()> {
    use std::io::Write;
    let logs = root.join("logs");
    std::fs::create_dir_all(&logs)?;
    let mut file = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(logs.join("setup.jsonl"))?;
    writeln!(file, "{line}")
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

    #[test]
    fn a_draft_reads_at_most_15_files_and_60_kib_in_the_order_of_the_skill() {
        let dir = tempfile::tempdir().unwrap();
        let d = dir.path();
        std::fs::create_dir_all(d.join("docs/specs")).unwrap();
        std::fs::write(d.join("README.md"), "readme").unwrap();
        std::fs::write(d.join("package.json"), "{}").unwrap();
        std::fs::write(d.join("docs/product.md"), "x".repeat(40 * 1024)).unwrap();
        for n in 0..20 {
            std::fs::write(d.join(format!("docs/specs/s{n:02}.md")), "y".repeat(1024)).unwrap();
        }
        let files = context_files(d);
        assert_eq!(files.len(), MAX_FILES);
        assert_eq!(
            files[..4],
            [
                "README.md",
                "package.json",
                "docs/product.md",
                "docs/specs/s00.md"
            ]
        );
        let c = context(d);
        assert!(c.bytes <= MAX_BYTES, "{}", c.bytes);
        assert_eq!(c.files, files);
        std::fs::write(d.join("docs/product.md"), "x".repeat(70 * 1024)).unwrap();
        let c = context(d);
        assert_eq!(c.bytes, MAX_BYTES);
        assert_eq!(c.files, ["README.md", "package.json", "docs/product.md"]);
        assert!(c.text.contains("## git log --oneline -30"));
    }

    #[test]
    fn the_draft_call_has_no_tools_model_sonnet_and_a_cap_of_half_a_dollar() {
        let call = draft_call(Path::new("/x"), "p".into());
        let args = crate::story::claude::call_args(&call);
        let after = |flag: &str| args[args.iter().position(|a| a == flag).unwrap() + 1].clone();
        assert_eq!(after("--max-budget-usd"), "0.5");
        assert_eq!(after("--model"), "sonnet");
        assert!(args.contains(&"--tools=".to_string()));
        assert!(!args.contains(&"--allowedTools".to_string()));
        let prompt = draft_prompt(
            Path::new("/x"),
            &Context::default(),
            Some(&json!({"product": {}})),
            &["roster.json must be an array of agent ids.".into()],
        );
        assert!(prompt.contains("## Repair"));
        assert!(prompt.contains("- roster.json must be an array of agent ids."));
    }
}
