//! The commands of the CLI. Each command parses its options, calls the method
//! of the same name on the core and prints what the TypeScript command prints.

mod archive;
mod ask;
mod audit;
mod backup;
mod check;
pub(crate) mod clear;
mod data;
mod decide;
mod decision;
mod desk;
mod doctor;
pub(crate) mod index;
mod interrupt;
mod mark;
mod meet;
mod migrate;
mod reflect;
mod render;
mod reset;
mod roles;
mod snapshot;
mod status;
mod story;
mod voice;
mod web;
mod whoami;
mod wizard;

use crate::args;
use crate::core::Core;
use agentsi_proto::agency_api::{ProductIdentity, ProductWhoami, WhoamiParams};

/// Why a command did not finish.
#[derive(Debug)]
pub enum Fail {
    /// No product matches the folder. Printed with the usage line, exit code 2.
    NoProduct(String),
    /// A wrong use of the command. Printed with the usage line, exit code 1.
    Usage(String),
    /// Any other error, printed as `agentsi <name>: <text>` with exit code 1.
    Error(String),
}

impl From<String> for Fail {
    fn from(text: String) -> Fail {
        Fail::Error(text)
    }
}

/// The entry point of a command. It gets the arguments after the command name
/// and returns the exit code.
pub type Run = fn(&[String]) -> Result<i32, Fail>;

pub struct Command {
    pub name: &'static str,
    pub summary: &'static str,
    pub usage: &'static str,
    /// `None`: not ported yet.
    pub run: Option<Run>,
}

const fn command(
    name: &'static str,
    summary: &'static str,
    usage: &'static str,
    run: Option<Run>,
) -> Command {
    Command {
        name,
        summary,
        usage,
        run,
    }
}

/// The commands in the order of `agentsi help`.
pub const COMMANDS: &[Command] = &[
    command(
        "up",
        "Start Postgres and apply migrations",
        migrate::UP_USAGE,
        Some(migrate::up),
    ),
    command(
        "doctor",
        "Check the tools that AgentSI needs",
        doctor::USAGE,
        Some(doctor::run),
    ),
    command(
        "migrate",
        "Apply pending SQL migrations",
        migrate::MIGRATE_USAGE,
        Some(migrate::migrate),
    ),
    command(
        "data",
        "Move data of an old data root into the database",
        data::USAGE,
        Some(data::run),
    ),
    command(
        "backup",
        "Dump the database and copy the registry into the backups folder of the data root",
        backup::USAGE,
        Some(backup::run),
    ),
    command(
        "whoami",
        "Detect the product of a directory",
        whoami::USAGE,
        Some(whoami::run),
    ),
    command(
        "desk",
        "Open the front desk: product, index, status and pending decisions as one JSON object",
        desk::USAGE,
        Some(desk::run),
    ),
    command(
        "index",
        "Update the knowledge base index",
        index::USAGE,
        Some(index::run),
    ),
    command(
        "status",
        "Product state, pending decisions and last minutes",
        status::USAGE,
        Some(status::run),
    ),
    command(
        "snapshot",
        "Markdown context for a meeting",
        snapshot::USAGE,
        Some(snapshot::run),
    ),
    command(
        "check",
        "Run the pass/fail check on a minutes file",
        check::USAGE,
        Some(check::run),
    ),
    command(
        "audit",
        "Health of the product's config, meetings, marks and memory (deterministic)",
        audit::USAGE,
        Some(audit::run),
    ),
    command("meet", "Run a meeting", meet::USAGE, Some(meet::run)),
    command(
        "ask",
        "Ask one role one question",
        ask::USAGE,
        Some(ask::run),
    ),
    command(
        "decide",
        "Approve or reject a pending decision",
        decide::USAGE,
        Some(decide::run),
    ),
    command(
        "decision",
        "Record a pending decision outside a meeting",
        decision::USAGE,
        Some(decision::run),
    ),
    command(
        "story",
        "Stories mode: propose, approve, run and review stories",
        story::USAGE,
        Some(story::run),
    ),
    command(
        "mark",
        "Mark priorities of a minutes file, or walk through everything pending",
        mark::USAGE,
        Some(mark::run),
    ),
    command(
        "archive",
        "Hide a minutes file, a pending decision or a story from the lists and counts; nothing is deleted",
        archive::ARCHIVE_USAGE,
        Some(archive::archive),
    ),
    command(
        "restore",
        "Bring an archived minutes file, decision or story back to the lists",
        archive::RESTORE_USAGE,
        Some(archive::restore),
    ),
    command(
        "roles",
        "Roster of the product (validate: check it; apply: install a staged brief and roster)",
        roles::USAGE,
        Some(roles::run),
    ),
    command(
        "reset",
        "Return an agent (or the whole product) to the global roles; --all drops brief, overlays, specialists and custom roles",
        reset::USAGE,
        Some(reset::run),
    ),
    command(
        "reflect",
        "Curate each agent's memory, then propose one prompt change gated by replay (revert: undo one)",
        reflect::USAGE,
        Some(reflect::run),
    ),
    command(
        "render",
        "Render the audio of a meeting",
        render::USAGE,
        Some(render::run),
    ),
    command(
        "voice",
        "Turn voice mode on or off for the current Claude Code session",
        voice::USAGE,
        Some(voice::run),
    ),
    command("web", "Start the web app", web::USAGE, Some(web::run)),
    command(
        "mcp",
        "Run the MCP server on stdio",
        crate::mcp::USAGE,
        Some(crate::mcp::run),
    ),
];

/// `--product`, then the folder of the process: the product that a command works on.
pub(crate) fn resolve_product(
    core: &mut Core,
    flag: Option<&str>,
) -> Result<ProductIdentity, Fail> {
    let cwd = std::env::current_dir().map_err(|e| e.to_string())?;
    let cwd = cwd.to_string_lossy().into_owned();
    let found = core.call::<ProductWhoami>(&WhoamiParams {
        cwd: Some(cwd.clone()),
        product: product_flag(flag),
        ..WhoamiParams::default()
    })?;
    found
        .product
        .ok_or_else(|| Fail::NoProduct(format!("No product matches {cwd}. Pass --product <id>.")))
}

/// The options of the commands that take only `--product`.
pub(crate) fn product_only(argv: &[String]) -> Result<Option<String>, Fail> {
    let a = args::parse(argv, &[("product", args::Kind::Text)]).map_err(Fail::Error)?;
    Ok(a.text("product").map(String::from))
}

/// An empty flag is no flag, as `if (flag)` of TypeScript reads it.
fn product_flag(flag: Option<&str>) -> Option<String> {
    flag.filter(|f| !f.is_empty()).map(String::from)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_empty_product_flag_is_no_flag() {
        assert_eq!(product_flag(Some("")), None);
        assert_eq!(product_flag(None), None);
        assert_eq!(product_flag(Some("beta")).as_deref(), Some("beta"));
    }
}
