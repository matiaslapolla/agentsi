//! `agentsi clear <item> [--product <id>] [--before <date>] [--yes]`: the delete of the
//! "Data" section of the app. With no `--yes` it prints the clear plan and deletes
//! nothing. With `--yes` it asks the core for the plan, then deletes that plan. The core
//! runs `data.backup` first. The command is not in `agentsi help`, which keeps the
//! text of the TypeScript CLI until the switch-over writes it again.

use super::{Command, Fail};
use crate::args::{self, Kind};
use crate::core::Core;
use agentsi_proto::clear_api::{
    ClearItem, ClearItemPlan, ClearKind, DataClear, DataClearParams, DataClearPlan,
    DataClearPlanParams, DataClearPlanResult, DataClearResult,
};

pub const USAGE: &str = "agentsi clear <item> [--product <id>] [--before <date>] [--yes]\n  item: product, meetings, stories, role-learning, call-audio, voice-transcripts, logs, backups, everything\n  --product: required for product, meetings, stories and role-learning, optional for call-audio\n  --before:  YYYY-MM-DD, for meetings and voice-transcripts\n  --yes:     delete (a backup runs first). Without it, the command prints the plan and deletes nothing.";

pub const COMMAND: Command = Command {
    name: "clear",
    summary: "Delete a product, old meetings, call audio, transcripts, logs or backups; --yes deletes",
    usage: USAGE,
    run: Some(run),
};

fn kind(name: &str) -> Option<ClearKind> {
    Some(match name.replace('_', "-").as_str() {
        "product" => ClearKind::Product,
        "meetings" => ClearKind::Meetings,
        "stories" => ClearKind::Stories,
        "role-learning" => ClearKind::RoleLearning,
        "call-audio" => ClearKind::CallAudio,
        "voice-transcripts" => ClearKind::VoiceTranscripts,
        "logs" => ClearKind::Logs,
        "backups" => ClearKind::Backups,
        "everything" => ClearKind::Everything,
        _ => return None,
    })
}

fn size(bytes: u64) -> String {
    const UNITS: [&str; 4] = ["B", "KB", "MB", "GB"];
    let mut v = bytes as f64;
    let mut unit = 0;
    while v >= 1024.0 && unit < UNITS.len() - 1 {
        v /= 1024.0;
        unit += 1;
    }
    if unit == 0 {
        format!("{bytes} B")
    } else {
        format!("{v:.1} {}", UNITS[unit])
    }
}

fn name(item: &ClearItem) -> String {
    let mut text = serde_json::to_value(item.item)
        .ok()
        .and_then(|v| v.as_str().map(|s| s.replace('_', "-")))
        .unwrap_or_default();
    if let Some(p) = &item.product {
        text.push_str(&format!(" {p}"));
    }
    if let Some(d) = &item.before {
        text.push_str(&format!(" before {d}"));
    }
    text
}

fn render_item(plan: &ClearItemPlan) -> String {
    let mut lines = vec![name(&plan.item)];
    for t in &plan.tables {
        lines.push(format!("  rows   {:<16} {}", t.table, t.rows));
    }
    for f in &plan.files {
        let kind = if f.dir { "folder" } else { "file" };
        lines.push(format!("  {kind:<6} {}  ({})", f.path, size(f.bytes)));
    }
    if plan.registry {
        lines.push("  registry  agentsi.config.json".into());
    }
    if plan.rows == 0 && plan.files.is_empty() && !plan.registry {
        lines.push("  nothing to delete".into());
    }
    lines.join("\n")
}

pub fn render_plan(plan: &DataClearPlanResult) -> String {
    let mut lines: Vec<String> = plan.items.iter().map(render_item).collect();
    lines.push(format!(
        "Total: {} rows, {} files, {}.",
        plan.rows,
        plan.files,
        size(plan.bytes)
    ));
    lines.join("\n")
}

fn render_done(done: &DataClearResult) -> String {
    let mut lines = vec![
        format!(
            "Deleted {} rows and {} files ({}).",
            done.rows,
            done.files,
            size(done.bytes)
        ),
        format!("Backup: {}", done.backup),
    ];
    if let Some(r) = &done.registry_backup {
        lines.push(format!("Registry backup: {r}"));
    }
    if !done.registry_removed.is_empty() {
        lines.push(format!(
            "Removed from the registry: {}",
            done.registry_removed.join(", ")
        ));
    }
    for f in &done.failed {
        lines.push(format!("Could not delete {}: {}", f.path, f.error));
    }
    lines.join("\n")
}

pub fn run(argv: &[String]) -> Result<i32, Fail> {
    let (a, positionals) = args::parse_positional(
        argv,
        &[
            ("product", Kind::Text),
            ("before", Kind::Text),
            ("yes", Kind::Flag),
        ],
    )
    .map_err(Fail::Error)?;
    let [word] = positionals.as_slice() else {
        return Err(Fail::Usage(USAGE.into()));
    };
    let Some(item) = kind(word) else {
        return Err(Fail::Usage(format!("Unknown item \"{word}\".")));
    };
    let items = vec![ClearItem {
        item,
        product: a.text("product").map(String::from),
        before: a.text("before").map(String::from),
    }];
    let mut core = Core::connect()?;
    let plan = core.call::<DataClearPlan>(&DataClearPlanParams {
        items: items.clone(),
    })?;
    println!("{}", render_plan(&plan));
    if !a.flag("yes") {
        println!("Nothing was deleted. Repeat with --yes to delete this plan.");
        return Ok(0);
    }
    let done = core.call::<DataClear>(&DataClearParams {
        items,
        plan_id: plan.plan_id,
    })?;
    println!("{}", render_done(&done));
    Ok(if done.failed.is_empty() { 0 } else { 1 })
}
