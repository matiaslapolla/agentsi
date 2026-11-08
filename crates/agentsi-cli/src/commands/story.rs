//! `agentsi story`: the 13 subcommands of stories mode. The texts, the exit
//! codes and the `--json` output are those of `packages/cli/src/commands/story*.ts`.

use super::{Fail, resolve_product};
use crate::args::{self, Args, Kind};
use crate::core::Core;
use crate::json;
use agentsi_proto::story_api::*;
use serde_json::{Map, Value, json};

pub const USAGE: &str = "agentsi story list [--json] [--show-archived] [--product <id>]
       agentsi story show <id> [--json] [--product <id>]
       agentsi story new \"<text>\" [--type task] [--product <id>]
       agentsi story suggest [--product <id>]
       agentsi story changes <id> \"<note>\" [--product <id>]
       agentsi story approve|discard <id> [--product <id>]
       agentsi story run|review|accept <id> [--product <id>]
       agentsi story reject <id> [--force] [--product <id>]
       agentsi story result <id> merged_changed|merged_unchanged --commit <sha> [--product <id>]";

const SUBCOMMANDS: [&str; 13] = [
    "list", "show", "new", "suggest", "changes", "approve", "discard", "run", "review", "accept",
    "reject", "result", "_exec",
];

const RESULT_USAGE: &str =
    "agentsi story result <id> merged_changed|merged_unchanged --commit <sha> [--product <id>]";

/// What a subcommand works with.
struct Ctx {
    core: Core,
    product: String,
    config: StoryConfig,
}

/// `--product` as `parseArgs` with `strict: false` reads it: a text value only.
fn product_flag(argv: &[String]) -> Option<String> {
    let mut found = None;
    let mut it = argv.iter().peekable();
    while let Some(a) = it.next() {
        if let Some(v) = a.strip_prefix("--product=") {
            found = Some(v.to_string());
        } else if a == "--product" {
            found = it.next_if(|v| !v.starts_with('-')).cloned();
        }
    }
    found
}

pub fn run(argv: &[String]) -> Result<i32, Fail> {
    let Some((sub, rest)) = argv.split_first() else {
        return Err(Fail::Usage("Missing story subcommand.".into()));
    };
    if !SUBCOMMANDS.contains(&sub.as_str()) {
        return Err(Fail::Usage(format!("Unknown story subcommand \"{sub}\".")));
    }
    let mut core = Core::connect()?;
    let product = resolve_product(&mut core, product_flag(rest).as_deref())?.product;
    let config = core.call::<StoryConfigGet>(&StoryConfigParams {
        product: product.clone(),
        run: sub == "run",
    })?;
    let mut ctx = Ctx {
        core,
        product,
        config,
    };
    match sub.as_str() {
        "list" => list(&mut ctx, rest),
        "show" => show(&mut ctx, rest),
        "new" => create(&mut ctx, rest),
        "suggest" => suggest(&mut ctx, rest),
        "changes" => changes(&mut ctx, rest),
        "approve" => verdict(&mut ctx, rest, "approve"),
        "discard" => verdict(&mut ctx, rest, "discard"),
        "run" => run_story(&mut ctx, rest),
        "_exec" => exec(&mut ctx, rest),
        "review" => review(&mut ctx, rest),
        "accept" => accept(&mut ctx, rest),
        "reject" => reject(&mut ctx, rest),
        _ => result(&mut ctx, rest),
    }
}

fn parse(argv: &[String], options: &[(&'static str, Kind)]) -> Result<(Args, Vec<String>), Fail> {
    args::parse_positional(argv, options).map_err(Fail::Error)
}

/// Postgres `jsonb` order: shorter keys first, then by bytes. The TypeScript
/// CLI prints the story as `jsonb` gives it back.
fn jsonb(v: Value) -> Value {
    match v {
        Value::Object(map) => {
            let mut entries: Vec<(String, Value)> = map.into_iter().collect();
            entries.sort_by(|a, b| a.0.len().cmp(&b.0.len()).then_with(|| a.0.cmp(&b.0)));
            Value::Object(
                entries
                    .into_iter()
                    .map(|(k, v)| (k, jsonb(v)))
                    .collect::<Map<_, _>>(),
            )
        }
        Value::Array(items) => Value::Array(items.into_iter().map(jsonb).collect()),
        other => other,
    }
}

/// The story as the TypeScript CLI prints it: the stored keys in `jsonb`
/// order, then `archived` when `listStories` set it.
fn story_json(s: &Story) -> Value {
    let mut v = serde_json::to_value(s).unwrap_or(Value::Null);
    let archived = v.as_object_mut().and_then(|m| m.shift_remove("archived"));
    let mut v = jsonb(v);
    if let (Some(a), Some(m)) = (archived, v.as_object_mut()) {
        m.insert("archived".into(), a);
    }
    v
}

fn list(ctx: &mut Ctx, argv: &[String]) -> Result<i32, Fail> {
    let (a, positionals) = parse(
        argv,
        &[
            ("product", Kind::Text),
            ("json", Kind::Flag),
            ("show-archived", Kind::Flag),
        ],
    )?;
    if !positionals.is_empty() {
        return Err(Fail::Usage(USAGE.into()));
    }
    let r = ctx.core.call::<StoryList>(&StoryListParams {
        product: ctx.product.clone(),
        include_archived: a.flag("show-archived"),
    })?;
    if a.flag("json") {
        let stories: Vec<Value> = r.stories.iter().map(story_json).collect();
        println!(
            "{}",
            json::pretty(&json!({ "product": r.product, "stories": stories }))
        );
        return Ok(0);
    }
    if r.stories.is_empty() {
        println!("{} has no stories.", r.product);
        return Ok(0);
    }
    let rows: Vec<[String; 6]> = r
        .stories
        .iter()
        .map(|s| {
            [
                s.id.clone(),
                s.status.clone(),
                s.approval.clone(),
                s.session.as_ref().map_or("-".into(), |x| x.state.clone()),
                s.owner_role.clone(),
                if s.archived == Some(true) {
                    format!("[archived] {}", s.title)
                } else {
                    s.title.clone()
                },
            ]
        })
        .collect();
    let len = |s: &str| s.encode_utf16().count();
    let widths: Vec<usize> = (0..5)
        .map(|i| rows.iter().map(|r| len(&r[i])).max().unwrap_or(0))
        .collect();
    for r in &rows {
        let mut cells: Vec<String> = (0..5)
            .map(|i| format!("{}{}", r[i], " ".repeat(widths[i] - len(&r[i]))))
            .collect();
        cells.push(r[5].clone());
        println!("{}", cells.join("  "));
    }
    Ok(0)
}

fn one_id(positionals: &[String], usage: &str) -> Result<String, Fail> {
    match positionals {
        [id] if !id.is_empty() => Ok(id.clone()),
        _ => Err(Fail::Usage(usage.into())),
    }
}

fn show(ctx: &mut Ctx, argv: &[String]) -> Result<i32, Fail> {
    let (a, positionals) = parse(argv, &[("product", Kind::Text), ("json", Kind::Flag)])?;
    let id = one_id(&positionals, USAGE)?;
    let r = ctx.core.call::<StoryGet>(&StoryIdParams {
        product: ctx.product.clone(),
        id,
    })?;
    if a.flag("json") {
        println!("{}", json::pretty(&story_json(&r.story)));
    } else {
        println!("{}", r.markdown);
    }
    Ok(0)
}

/// The options of the proposal subcommands, which share one parse.
fn propose_parse(argv: &[String]) -> Result<(Args, Vec<String>), Fail> {
    parse(
        argv,
        &[
            ("product", Kind::Text),
            ("type", Kind::Text),
            ("minutes", Kind::Text),
        ],
    )
}

fn line(s: &Story) -> String {
    format!(
        "{} {} ({}, owner {}, {} criteria)",
        s.id,
        s.title,
        s.kind,
        s.owner_role,
        s.criteria.len()
    )
}

fn report(r: &StoryProposeResult, verb: &str) {
    for s in &r.stories {
        println!("{verb} {}", line(s));
    }
    if r.stories.is_empty() {
        println!("The PM proposed no stories.");
    }
    println!("Cost: ${:.3}", r.total_cost_usd);
}

fn create(ctx: &mut Ctx, argv: &[String]) -> Result<i32, Fail> {
    let (a, positionals) = propose_parse(argv)?;
    let usage = "agentsi story new \"<text>\" [--type task] [--minutes <id>] [--product <id>]";
    let text = match positionals.as_slice() {
        [t] if !t.is_empty() => t.clone(),
        _ => return Err(Fail::Usage(usage.into())),
    };
    let kind = a.text("type").map(String::from);
    if kind.as_deref().is_some_and(|k| k != "task" && k != "story") {
        return Err(Fail::Usage("--type must be \"story\" or \"task\".".into()));
    }
    let r = ctx.core.call::<StoryNew>(&StoryNewParams {
        product: ctx.product.clone(),
        text,
        kind,
        minutes: a.text("minutes").map(String::from),
    })?;
    report(&r, "Proposed");
    println!("Approve it with: agentsi story approve <id>");
    Ok(0)
}

fn suggest(ctx: &mut Ctx, argv: &[String]) -> Result<i32, Fail> {
    if !propose_parse(argv)?.1.is_empty() {
        return Err(Fail::Usage("agentsi story suggest [--product <id>]".into()));
    }
    let r = ctx.core.call::<StorySuggest>(&ProductParams {
        product: ctx.product.clone(),
    })?;
    report(&r, "Suggested");
    Ok(0)
}

fn changes(ctx: &mut Ctx, argv: &[String]) -> Result<i32, Fail> {
    let positionals = propose_parse(argv)?.1;
    let (id, note) = match positionals.as_slice() {
        [id, note] if !id.is_empty() && !note.is_empty() => (id.clone(), note.clone()),
        _ => {
            return Err(Fail::Usage(
                "agentsi story changes <id> \"<note>\" [--product <id>]".into(),
            ));
        }
    };
    let r = ctx.core.call::<StoryChanges>(&StoryChangesParams {
        product: ctx.product.clone(),
        id,
        note,
        rewrite: true,
    })?;
    report(&r, "Rewritten");
    Ok(0)
}

fn verdict(ctx: &mut Ctx, argv: &[String], action: &str) -> Result<i32, Fail> {
    let positionals = propose_parse(argv)?.1;
    let id = one_id(
        &positionals,
        &format!("agentsi story {action} <id> [--product <id>]"),
    )?;
    let params = StoryIdParams {
        product: ctx.product.clone(),
        id,
    };
    let (verb, story) = if action == "approve" {
        ("Approved", ctx.core.call::<StoryApprove>(&params)?.story)
    } else {
        ("Discarded", ctx.core.call::<StoryDiscard>(&params)?.story)
    };
    println!("{verb} {}", line(&story));
    Ok(0)
}

/// The id and `--force` of the session subcommands.
fn story_args(argv: &[String], usage: &str) -> Result<(String, bool), Fail> {
    let (a, positionals) = parse(argv, &[("product", Kind::Text), ("force", Kind::Flag)])?;
    Ok((one_id(&positionals, usage)?, a.flag("force")))
}

fn id_params(ctx: &Ctx, argv: &[String], usage: &str) -> Result<StoryIdParams, Fail> {
    Ok(StoryIdParams {
        product: ctx.product.clone(),
        id: story_args(argv, usage)?.0,
    })
}

fn run_story(ctx: &mut Ctx, argv: &[String]) -> Result<i32, Fail> {
    let p = id_params(ctx, argv, "agentsi story run <id> [--product <id>]")?;
    let story = ctx.core.call::<StoryRun>(&p)?.story;
    let branch = story
        .session
        .as_ref()
        .map(|s| s.branch.clone())
        .unwrap_or_default();
    println!("Story session of {} started on {branch}.", p.id);
    println!("Log: story_logs {}/{}", p.product, p.id);
    println!("Then: agentsi story review {}", p.id);
    Ok(0)
}

fn exec(ctx: &mut Ctx, argv: &[String]) -> Result<i32, Fail> {
    let p = id_params(ctx, argv, "agentsi story _exec <id> [--product <id>]")?;
    let story = ctx.core.call::<StoryExec>(&p)?.story;
    Ok(if story.session.is_some_and(|s| s.state == "ready") {
        0
    } else {
        1
    })
}

fn stop_text(stop: &StoryStop) -> String {
    format!(
        "Stopped: {}, cap USD {:.2} for each session.",
        stop.reason, stop.cap_usd
    )
}

fn review(ctx: &mut Ctx, argv: &[String]) -> Result<i32, Fail> {
    let p = id_params(ctx, argv, "agentsi story review <id> [--product <id>]")?;
    let r = ctx.core.call::<StoryReview>(&p)?;
    let (story, id) = (&r.story, &p.id);
    let Some(s) = &story.session else {
        return Err(Fail::Error(format!("Story {id} has no story session.")));
    };
    println!("{} {}", story.id, story.title);
    println!(
        "Session: {}, attempt {}, branch {}",
        s.state, s.attempts, s.branch
    );
    println!("Worktree: {}", s.worktree);
    if let Some(c) = s.tests_commit.as_deref().filter(|c| !c.is_empty()) {
        println!("Tests commit: {c}");
    }
    if let Some(c) = s.final_commit.as_deref().filter(|c| !c.is_empty()) {
        println!("Final commit: {c}");
    }
    let failure = s.failure.clone().unwrap_or_default();
    let stop = s.stop.as_ref().and_then(Option::as_ref);
    println!(
        "{}",
        if s.state == "ready" {
            "Checks: the criterion tests failed before the work, no test file changed, the tests pass and each gate command passes.".to_string()
        } else if let Some(stop) = stop {
            format!("{}\n{failure}", stop_text(stop))
        } else if s.state == "failed" {
            format!("Checks: failed.\n{failure}")
        } else {
            "Checks: not done yet.".into()
        }
    );
    println!("Log: {}", s.log);
    if !r.stat.is_empty() {
        println!("\n{}", r.stat);
    }
    println!();
    if s.state == "ready" {
        println!(
            "Diff: git -C {} diff {}..{}",
            ctx.config.repo, s.base, s.branch
        );
        println!("Merge: agentsi story accept {id}");
    }
    let force = if s.state == "ready" || s.state == "failed" {
        ""
    } else {
        " (add --force to stop a session that still runs)"
    };
    println!("Discard: agentsi story reject {id}{force}");
    Ok(0)
}

fn accept(ctx: &mut Ctx, argv: &[String]) -> Result<i32, Fail> {
    let p = id_params(ctx, argv, "agentsi story accept <id> [--product <id>]")?;
    let r = ctx.core.call::<StoryAccept>(&p)?;
    println!(
        "Story {} merged into the main branch as {} ({}).",
        p.id, r.commit, r.result
    );
    Ok(0)
}

fn reject(ctx: &mut Ctx, argv: &[String]) -> Result<i32, Fail> {
    let (id, force) = story_args(argv, "agentsi story reject <id> [--force] [--product <id>]")?;
    ctx.core.call::<StoryReject>(&StoryRejectParams {
        product: ctx.product.clone(),
        id: id.clone(),
        force,
    })?;
    println!("Story {id} discarded: worktree and branch removed. It can run again.");
    Ok(0)
}

fn result(ctx: &mut Ctx, argv: &[String]) -> Result<i32, Fail> {
    let (a, positionals) = parse(argv, &[("product", Kind::Text), ("commit", Kind::Text)])?;
    let commit = a.text("commit").filter(|c| !c.is_empty());
    let (id, kind, commit) = match (positionals.as_slice(), commit) {
        ([id, kind], Some(commit))
            if !id.is_empty()
                && ["merged_changed", "merged_unchanged"].contains(&kind.as_str()) =>
        {
            (id.clone(), kind.clone(), commit.to_string())
        }
        _ => return Err(Fail::Usage(RESULT_USAGE.into())),
    };
    let story = ctx
        .core
        .call::<StoryResultSet>(&StoryResultParams {
            product: ctx.product.clone(),
            id: id.clone(),
            kind: kind.clone(),
            commit,
        })?
        .story;
    let at = story
        .results
        .last()
        .and_then(|r| r.commit.clone())
        .unwrap_or_default();
    println!("Story {id} is implemented, with the result {kind} at {at}.");
    Ok(0)
}
