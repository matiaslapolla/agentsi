//! Options of one command, as `util.parseArgs` of Node reads them in strict
//! mode: `--name value`, `--name=value` and flags. A positional argument and
//! an unknown option are errors.

use std::collections::BTreeMap;

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    Text,
    Flag,
    /// A text option that may repeat (`multiple: true` of `util.parseArgs`).
    Many,
}

#[derive(Default)]
pub struct Args {
    values: BTreeMap<&'static str, Vec<String>>,
    flags: Vec<&'static str>,
}

impl Args {
    /// The last value, as `util.parseArgs` keeps it when an option repeats.
    pub fn text(&self, name: &str) -> Option<&str> {
        self.values.get(name)?.last().map(String::as_str)
    }

    /// Each value of a `Kind::Many` option, in order.
    pub fn many(&self, name: &str) -> &[String] {
        self.values.get(name).map_or(&[], Vec::as_slice)
    }

    pub fn flag(&self, name: &str) -> bool {
        self.flags.contains(&name)
    }

    fn push(&mut self, key: &'static str, kind: Kind, value: String) {
        let list = self.values.entry(key).or_default();
        if kind == Kind::Text {
            list.clear();
        }
        list.push(value);
    }
}

/// Reads `argv` against the options of a command. The error is the message
/// that the command prints after `agentsi <name>: `.
pub fn parse(argv: &[String], options: &[(&'static str, Kind)]) -> Result<Args, String> {
    parse_inner(argv, options, None)
}

/// Like `parse`, for a command that takes positional arguments (`allowPositionals`).
pub fn parse_positional(
    argv: &[String],
    options: &[(&'static str, Kind)],
) -> Result<(Args, Vec<String>), String> {
    let mut positionals = Vec::new();
    let args = parse_inner(argv, options, Some(&mut positionals))?;
    Ok((args, positionals))
}

fn parse_inner(
    argv: &[String],
    options: &[(&'static str, Kind)],
    mut positionals: Option<&mut Vec<String>>,
) -> Result<Args, String> {
    let mut out = Args::default();
    let mut rest = argv.iter();
    while let Some(arg) = rest.next() {
        // `--` ends the options: each later word is a positional argument.
        if arg == "--" {
            match positionals.as_deref_mut() {
                Some(list) => list.extend(rest.by_ref().cloned()),
                None => {
                    if let Some(word) = rest.next() {
                        return Err(format!(
                            "Unexpected argument '{word}'. This command does not take positional arguments"
                        ));
                    }
                }
            }
            break;
        }
        let Some(long) = arg.strip_prefix("--").filter(|l| !l.is_empty()) else {
            if arg.starts_with('-') && arg.len() > 1 {
                return Err(format!("Unknown option '{arg}'"));
            }
            if let Some(list) = positionals.as_deref_mut() {
                list.push(arg.clone());
                continue;
            }
            return Err(format!(
                "Unexpected argument '{arg}'. This command does not take positional arguments"
            ));
        };
        let (name, inline) = match long.split_once('=') {
            Some((n, v)) => (n, Some(v.to_string())),
            None => (long, None),
        };
        let Some(&(key, kind)) = options.iter().find(|(n, _)| *n == name) else {
            return Err(format!("Unknown option '--{name}'"));
        };
        match (kind, inline) {
            (Kind::Flag, None) => out.flags.push(key),
            (Kind::Flag, Some(_)) => {
                return Err(format!("Option '--{name}' does not take an argument"));
            }
            (_, Some(v)) => out.push(key, kind, v),
            (_, None) => match rest.next().filter(|v| !v.starts_with('-')) {
                Some(v) => out.push(key, kind, v.clone()),
                None => return Err(format!("Option '--{name} <value>' argument missing")),
            },
        }
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn argv(a: &[&str]) -> Vec<String> {
        a.iter().map(|s| s.to_string()).collect()
    }

    const OPTS: &[(&str, Kind)] = &[("product", Kind::Text), ("json", Kind::Flag)];

    #[test]
    fn it_reads_values_flags_and_inline_values() {
        let a = parse(&argv(&["--product", "x", "--json"]), OPTS).unwrap();
        assert_eq!((a.text("product"), a.flag("json")), (Some("x"), true));
        let a = parse(&argv(&["--product=y"]), OPTS).unwrap();
        assert_eq!((a.text("product"), a.flag("json")), (Some("y"), false));
    }

    #[test]
    fn a_command_with_positionals_collects_them_in_order() {
        let (a, rest) =
            parse_positional(&argv(&["one", "--product", "x", "two", "--json"]), OPTS).unwrap();
        assert_eq!((a.text("product"), a.flag("json")), (Some("x"), true));
        assert_eq!(rest, ["one", "two"]);
        assert!(parse_positional(&argv(&["--nope"]), OPTS).is_err());
    }

    #[test]
    fn a_repeated_option_keeps_the_last_value_or_all_of_them() {
        let opts: &[(&str, Kind)] = &[("product", Kind::Text), ("set", Kind::Many)];
        let a = parse(
            &argv(&["--product", "a", "--set", "x", "--product=b", "--set=y"]),
            opts,
        )
        .unwrap();
        assert_eq!(a.text("product"), Some("b"));
        assert_eq!(a.many("set"), ["x", "y"]);
    }

    #[test]
    fn a_double_dash_ends_the_options() {
        let (a, rest) =
            parse_positional(&argv(&["--json", "--", "--not-an-option", "x"]), OPTS).unwrap();
        assert!(a.flag("json"));
        assert_eq!(rest, ["--not-an-option", "x"]);
        assert!(parse(&argv(&["--", "x"]), OPTS).is_err());
    }

    #[test]
    fn it_refuses_what_strict_parse_args_refuses() {
        for bad in [
            &["--nope"][..],
            &["-x"],
            &["word"],
            &["--product"],
            &["--product", "--json"],
            &["--json=1"],
        ] {
            assert!(parse(&argv(bad), OPTS).is_err(), "{bad:?}");
        }
    }
}
