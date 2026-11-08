//! `unifiedDiff` of `reflections.ts`.

use std::process::Command;

/// Unified diff of two texts through `git diff --no-index`. Empty when they are equal.
pub fn unified_diff(before: &str, after: &str, label: &str) -> std::io::Result<String> {
    if before == after {
        return Ok(String::new());
    }
    let dir = tempfile::tempdir()?;
    let with_line = |text: &str| {
        if text.is_empty() {
            String::new()
        } else {
            format!("{text}\n")
        }
    };
    std::fs::write(dir.path().join("a"), with_line(before))?;
    std::fs::write(dir.path().join("b"), with_line(after))?;
    let out = Command::new("git")
        .args([
            "diff",
            "--no-index",
            "--no-color",
            "--no-ext-diff",
            "-U3",
            "a",
            "b",
        ])
        .current_dir(dir.path())
        .output()?;
    let text = String::from_utf8_lossy(&out.stdout);
    Ok(match text.find("@@") {
        Some(start) => format!("--- a/{label}\n+++ b/{label}\n{}", &text[start..]),
        None => String::new(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn equal_texts_have_no_diff_and_a_change_shows_its_lines() {
        assert_eq!(unified_diff("a", "a", "x").unwrap(), "");
        let diff = unified_diff("a\nb", "a\nc", "p/memory/dev").unwrap();
        assert!(diff.starts_with("--- a/p/memory/dev\n+++ b/p/memory/dev\n@@"));
        assert!(diff.contains("-b\n+c\n"));
    }

    #[test]
    fn an_empty_side_is_an_empty_file() {
        let diff = unified_diff("", "a", "x").unwrap();
        assert!(diff.contains("+a\n"), "{diff}");
    }
}
