//! The keep time of the voice models in `pill.toml`, as `[models] keep`.
//!
//! The pill and the core both read it with [`keep_in`], so the core, the app and the pill
//! always agree on it.

use agentsi_proto::voice_api::KeepTime;

/// The keep time of a `pill.toml` text, and the one line that says why it is the default: a
/// text that does not parse, or a bad value. A missing value is the default, with no line.
/// Other keys do not count: the pill reads them with its own rules.
pub fn keep_in(text: &str) -> (KeepTime, Option<String>) {
    let default = KeepTime::default();
    let uses = format!("The pill uses \"{}\".", default.key());
    let table = match text.parse::<toml::Table>() {
        Ok(table) => table,
        Err(e) => {
            let why = e.message().replace('\n', " ");
            return (
                default,
                Some(format!("pill.toml does not parse: {why}. {uses}")),
            );
        }
    };
    let Some(value) = table.get("models").and_then(|m| m.get("keep")) else {
        return (default, None);
    };
    if let Some(keep) = value.as_str().and_then(KeepTime::from_key) {
        return (keep, None);
    }
    let valid: Vec<&str> = KeepTime::ALL.iter().map(|k| k.key()).collect();
    (
        default,
        Some(format!(
            "pill.toml: [models] keep = {value} is not one of {}. {uses}",
            valid.join(", ")
        )),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_models_table_sets_the_keep_time() {
        assert_eq!(
            keep_in("input = \"USB\"\n\n[models]\nkeep = \"1h\"\n"),
            (KeepTime::OneHour, None)
        );
        assert_eq!(keep_in("[models]\n"), (KeepTime::FiveMinutes, None));
        assert_eq!(keep_in(""), (KeepTime::FiveMinutes, None));
        // Other keys, known or not, do not change the keep time.
        assert_eq!(
            keep_in("inputs = 1\n[models]\nkeep = \"start\"\nlater = true\n"),
            (KeepTime::Start, None)
        );
    }

    #[test]
    fn a_bad_value_or_a_broken_file_gives_one_line_and_the_default() {
        for text in [
            "[models]\nkeep = \"2m\"\n",
            "[models]\nkeep = 5\n",
            "input = \n",
        ] {
            let (keep, line) = keep_in(text);
            assert_eq!(keep, KeepTime::FiveMinutes, "{text}");
            let line = line.unwrap_or_else(|| panic!("a line for {text}"));
            assert!(!line.contains('\n'), "{line}");
            assert!(line.ends_with("The pill uses \"5m\"."), "{line}");
        }
        let (_, line) = keep_in("[models]\nkeep = \"2m\"\n");
        assert!(line.unwrap().contains("keep = \"2m\" is not one of 1m, 5m"));
    }
}
