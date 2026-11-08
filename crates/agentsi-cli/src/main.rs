//! The `agentsi` command. It keeps the names, the options, the exit codes and
//! the output of the TypeScript CLI for the commands that it ports. Each
//! command is a client of the core. A command that is not ported prints one
//! line that says so and stops with exit code 1.

mod args;
mod commands;
mod core;
mod hook;
mod json;
mod mcp;

use commands::{COMMANDS, Fail};
use std::process::ExitCode;

fn help() -> String {
    let width = COMMANDS.iter().map(|c| c.name.len()).max().unwrap_or(0);
    let mut lines = vec![
        "Usage: agentsi <command> [options]".to_string(),
        String::new(),
    ];
    lines.extend(
        COMMANDS
            .iter()
            .map(|c| format!("  {:width$}  {}", c.name, c.summary)),
    );
    lines.join("\n")
}

fn main() -> ExitCode {
    let argv: Vec<String> = std::env::args().skip(1).collect();
    let Some((name, rest)) = argv.split_first() else {
        println!("{}", help());
        return ExitCode::from(1);
    };
    if matches!(name.as_str(), "help" | "--help" | "-h") {
        println!("{}", help());
        return ExitCode::SUCCESS;
    }
    // The hook bridge is for Claude Code, not for people: it is not in the help.
    if name == "hook" {
        return match hook::run(rest) {
            Ok(code) => ExitCode::from(code as u8),
            Err(Fail::Usage(text) | Fail::Error(text) | Fail::NoProduct(text)) => {
                eprintln!("{text}\nUsage: {}", hook::USAGE);
                ExitCode::from(1)
            }
        };
    }
    // `clear` is Rust only, so it is not in the help, which keeps the TypeScript text.
    let found = COMMANDS
        .iter()
        .find(|c| c.name == name)
        .or_else(|| (name == commands::clear::COMMAND.name).then_some(&commands::clear::COMMAND));
    let Some(command) = found else {
        eprintln!("Unknown command \"{name}\"\n\n{}", help());
        return ExitCode::from(1);
    };
    let Some(run) = command.run else {
        eprintln!(
            "agentsi {name}: not ported to the Rust CLI yet. Use the TypeScript CLI for this command."
        );
        return ExitCode::from(1);
    };
    match run(rest) {
        Ok(code) => ExitCode::from(code as u8),
        Err(Fail::NoProduct(text)) => {
            eprintln!("{text}\nUsage: {}", command.usage);
            ExitCode::from(2)
        }
        Err(Fail::Usage(text)) => {
            eprintln!("{text}\nUsage: {}", command.usage);
            ExitCode::from(1)
        }
        Err(Fail::Error(text)) => {
            eprintln!("agentsi {name}: {text}");
            ExitCode::from(1)
        }
    }
}
