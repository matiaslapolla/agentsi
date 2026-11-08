use agentsi_models::{Catalog, Kind};

#[test]
fn the_builtin_catalog_is_valid() {
    let catalog = Catalog::builtin();
    let ids: Vec<_> = catalog.models().iter().map(|m| m.id.as_str()).collect();
    assert_eq!(
        ids,
        [
            "kokoro",
            "parakeet-tdt-v3-int8",
            "parakeet-tdt-v2-int8",
            "silero-vad-v5"
        ]
    );
    for m in catalog.models() {
        assert!(m.total_size() > 0 && !m.license.is_empty(), "{}", m.id);
    }
}

#[test]
fn kokoro_has_the_files_that_packages_voice_opens() {
    let catalog = Catalog::builtin();
    let kokoro = catalog.get("kokoro").unwrap();
    assert_eq!(kokoro.kind, Kind::SpeechOut);
    let names: Vec<_> = kokoro.files.iter().map(|f| f.name.as_str()).collect();
    assert_eq!(names, ["kokoro-v1.0.onnx", "voices-v1.0.bin"]);
}

#[test]
fn the_check_refuses_bad_entries() {
    let entry = |extra: &str| {
        format!(
            r#"[[model]]
id = "m"
name = "n"
kind = "speech-in"
license = "MIT"
description = "d"
repo = "o/r"
revision = "{}"
{extra}
[[model.file]]
name = "{}"
size = 1
sha256 = "{}"
"#,
            "a".repeat(40),
            "../x",
            "0".repeat(64)
        )
    };
    assert!(
        Catalog::parse(&entry(""))
            .unwrap_err()
            .contains("unsafe file name")
    );
    assert!(
        Catalog::parse(
            &entry("")
                .replace("../x", "x")
                .replace(&"a".repeat(40), "main")
        )
        .unwrap_err()
        .contains("commit hash")
    );
    assert!(
        Catalog::parse(
            &entry("")
                .replace("../x", "x")
                .replace(&"0".repeat(64), "ABC")
        )
        .unwrap_err()
        .contains("SHA-256")
    );
}
