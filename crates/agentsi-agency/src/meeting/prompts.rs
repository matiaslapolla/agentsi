//! The prompts and the schemas of a meeting. The rules and the texts copy
//! `packages/core/src/meeting.ts`. A text
//! change here is a change of what the roles read, so a golden case compares
//! each prompt with the prompt of the TypeScript code.

use crate::meeting::log::NO_DATA;
use crate::prompt_inputs::{self, brief_xml, objective_ids};
use crate::roster::js_text;
use crate::snapshot::OPEN_TICKETS;
use crate::story::claude::Call;
use crate::story::code_graph::mcp;
use agentsi_proto::agency_api::Owner;
use agentsi_proto::roles_api::RoleDef;
use serde_json::{Map, Value, json};
use std::path::Path;
use std::time::Duration;

const ROLE_TIMEOUT: Duration = Duration::from_secs(6 * 60);
const BASE_TOOLS: [&str; 3] = ["Read", "Grep", "Glob"];

/// true when the snapshot lists open tickets, so a priority may cite a ticket.
pub fn cites_tickets(snapshot: &str) -> bool {
    snapshot.lines().any(|l| l == OPEN_TICKETS)
}

fn source_schema(tickets: bool) -> Value {
    let mut types = vec!["commit", "issue", "path", "url", "story"];
    if tickets {
        types.push("ticket");
    }
    json!({
        "type": "object",
        "properties": {
            "type": {"type": "string", "enum": types},
            "ref": {"type": "string"},
        },
        "required": ["type", "ref"],
    })
}

pub fn role_answer_schema(tickets: bool) -> Value {
    let strings = json!({"type": "array", "items": {"type": "string"}});
    json!({
        "type": "object",
        "properties": {
            "answer": {"type": "string"},
            "priorities": {
                "type": "array",
                "items": {
                    "type": "object",
                    "properties": {
                        "title": {"type": "string"},
                        "why": {"type": "string"},
                        "sources": {"type": "array", "items": source_schema(tickets)},
                    },
                    "required": ["title", "why", "sources"],
                },
            },
            "concerns": strings,
            "memory_notes": strings,
        },
        "required": ["answer", "priorities", "concerns", "memory_notes"],
    })
}

/// With a brief, each priority must name one of its objectives.
pub fn synthesis_schema(brief: Option<&Value>, tickets: bool) -> Value {
    let mut priority = Map::new();
    priority.insert("title".into(), json!({"type": "string"}));
    priority.insert("why".into(), json!({"type": "string"}));
    priority.insert(
        "sources".into(),
        json!({"type": "array", "items": source_schema(tickets)}),
    );
    priority.insert("owner_role".into(), json!({"type": "string"}));
    let mut required = vec!["title", "why", "sources", "owner_role"];
    if let Some(brief) = brief {
        priority.insert(
            "objective".into(),
            json!({"type": "string", "enum": objective_ids(brief)}),
        );
        required.push("objective");
    }
    json!({
        "type": "object",
        "properties": {
            "priorities": {
                "type": "array",
                "items": {"type": "object", "properties": priority, "required": required},
            },
            "disagreements": {
                "type": "array",
                "items": {
                    "type": "object",
                    "properties": {
                        "topic": {"type": "string"},
                        "positions": {
                            "type": "array",
                            "items": {
                                "type": "object",
                                "properties": {"role": {"type": "string"}, "view": {"type": "string"}},
                                "required": ["role", "view"],
                            },
                        },
                    },
                    "required": ["topic", "positions"],
                },
            },
            "decisions": {
                "type": "array",
                "items": {
                    "type": "object",
                    "properties": {
                        "text": {"type": "string"},
                        "why": {"type": "string"},
                        "action": {"type": "string"},
                        "priorities": {"type": "array", "items": {"type": "integer"}},
                    },
                    "required": ["text", "why"],
                },
            },
            "script": {
                "type": "array",
                "items": {
                    "type": "object",
                    "properties": {"role": {"type": "string"}, "text": {"type": "string"}},
                    "required": ["role", "text"],
                },
            },
        },
        "required": ["priorities", "disagreements", "decisions", "script"],
    })
}

fn path_rule(scope: Option<&str>) -> String {
    match scope {
        Some(scope) => format!(
            "Cite file paths relative to the product root, so prefix them with \"{scope}/\" (for example \"{scope}/docs/x.md\")."
        ),
        None => "Cite file paths relative to the working directory.".into(),
    }
}

/// What every agent of one meeting reads.
pub struct RoleContext<'a> {
    pub question: &'a str,
    pub snapshot: &'a str,
    pub memory: &'a str,
    pub scope: Option<&'a str>,
    pub cwd: &'a Path,
    pub brief: Option<&'a Value>,
    pub owner: &'a Owner,
    /// The code graph projects that were reindexed for this meeting.
    pub code_graph: &'a [String],
}

fn role_prompt(
    ctx: &RoleContext,
    role: &str,
    web: bool,
    meeting: bool,
    code_graph: &[String],
) -> String {
    let web_tool = if web { ", WebSearch" } else { "" };
    let mut rules = vec![
        format!(
            "- You are the {role} of the agency. Working directory: {} (read-only: Read, Grep, Glob{web_tool}).",
            ctx.cwd.display()
        ),
        format!(
            "- Answer in {}, short and direct. Keep identifiers, paths and code in English.",
            ctx.owner.language
        ),
        format!(
            "- Base every claim on the snapshot, your memory or files you read. Write \"{NO_DATA}\" and name what is missing; never invent numbers, users, metrics or dates."
        ),
        format!(
            "- You only give opinions. Each change you propose is a decision {} must approve. One priority you can defend beats five vague ones.",
            ctx.owner.name
        ),
    ];
    if !code_graph.is_empty() {
        rules.push(format!(
            "- The code graph tools (mcp__codegraph__*, projects: {}) find symbols, callers and call paths fast. Read the file to confirm before you cite it.",
            code_graph.join(", ")
        ));
    }
    if meeting {
        let for_web = if web { ", for web pages" } else { "" };
        let types = if cites_tickets(ctx.snapshot) {
            format!(
                "url (full http(s) URL{for_web}), story (an id from the Stories section of the snapshot) or ticket (an id from the Open tickets section of the snapshot)"
            )
        } else {
            format!(
                "url (full http(s) URL{for_web}) or story (an id from the Stories section of the snapshot)"
            )
        };
        rules.push(format!(
            "- Give at most 3 priorities. Each cites at least one source of type commit (hash from the snapshot), issue (number), path (a file that exists), {types}. {}",
            path_rule(ctx.scope)
        ));
        rules.push(if ctx.brief.is_some() {
            "- Cite the north stars declared in the <brief> (<north_stars>) where they support a priority. Keep everything under <out_of_scope> out of your priorities.".into()
        } else {
            "- Cite the product's north-star or guideline docs (CLAUDE.md, CONTEXT.md, PRODUCT.md, DESIGN.md, README.md, docs/...) where they support a priority.".to_string()
        });
        rules.push("- \"answer\" is your view in at most 150 words. \"concerns\": at most 3 risks you see. \"memory_notes\": only facts about this product still true next week (a goal, a constraint, a decision, a recurring risk), not opinions about today; usually empty.".into());
    } else {
        rules.push(
            "- Cite sources inline: commit hashes, issue numbers or repo-relative paths. Plain text, no JSON."
                .into(),
        );
    }
    // Stable context first, so repeated calls on the same product share a cacheable prefix.
    let mut parts = Vec::new();
    if let Some(brief) = ctx.brief {
        parts.push(format!("# Brief of the product\n{}", brief_xml(brief)));
    }
    parts.push(format!("# Snapshot of the product\n{}", ctx.snapshot));
    let memory = if ctx.memory.is_empty() {
        "_Empty: you have not met on this product yet._"
    } else {
        ctx.memory
    };
    parts.push(format!("# Your memory about this product\n{memory}"));
    parts.push(format!("# Question\n{}", ctx.question));
    parts.push(format!("# Instructions\n{}", rules.join("\n")));
    parts.join("\n\n")
}

/// The call that asks one agent. `meeting` asks for the structured answer of a meeting.
pub fn role_call(def: &RoleDef, ctx: &RoleContext, meeting: bool) -> Call {
    let tools: Vec<String> = def
        .tools
        .clone()
        .unwrap_or_else(|| BASE_TOOLS.map(String::from).to_vec());
    let code_graph: &[String] = if tools.is_empty() {
        &[]
    } else {
        ctx.code_graph
    };
    let web = tools.iter().any(|t| t == "WebSearch");
    Call {
        budget_usd: None,
        prompt: role_prompt(ctx, &def.id, web, meeting, code_graph),
        model: def.model.clone(),
        system: def.body.clone(),
        cwd: ctx.cwd.to_string_lossy().into_owned(),
        tools,
        schema: meeting.then(|| role_answer_schema(cites_tickets(ctx.snapshot))),
        effort: def.effort.clone(),
        timeout: ROLE_TIMEOUT,
        mcp: (!code_graph.is_empty()).then(mcp),
    }
}

const COMPACT_MAX: usize = 14_000;

/// Keeps the sections of the snapshot that the synthesis needs, within a budget of characters.
pub fn compact_snapshot(full: &str) -> String {
    prompt_inputs::compact_snapshot(full, COMPACT_MAX)
}

/// The output of the failed synthesis and its failures, for the repair call.
pub struct Repair<'a> {
    pub previous: &'a Value,
    pub failures: &'a [String],
}

fn synthesis_prompt(
    question: &str,
    answers: &[(String, Value)],
    compact: &str,
    brief: Option<&Value>,
    owner: &Owner,
    repair: Option<&Repair>,
) -> String {
    let role_text: Vec<String> = answers
        .iter()
        .map(|(role, a)| {
            let view = json!({
                "answer": a.get("answer"),
                "priorities": a.get("priorities"),
                "concerns": a.get("concerns"),
            });
            format!("## {role}\n\n{view}")
        })
        .collect();
    let role_text = if role_text.is_empty() {
        "_No agent answered._".to_string()
    } else {
        role_text.join("\n\n")
    };
    let mut parts = vec![
        format!("# Question\n{question}"),
        format!("# Agent answers\n{role_text}"),
    ];
    if let Some(brief) = brief {
        parts.push(format!("# Brief of the product\n{}", brief_xml(brief)));
    }
    parts.push(format!("# Snapshot (compact)\n{compact}"));
    let name = &owner.name;
    let language = &owner.language;
    let mut rules = vec![
        "# Instructions".to_string(),
        "- Rank at most 5 priorities. Each cites at least one source, copied verbatim (type and ref) from the agent answers or the snapshot. Never invent a source.".to_string(),
    ];
    match brief {
        Some(brief) => {
            rules.push(format!(
                "- \"objective\" is the id of the brief objective the priority serves: one of {}.",
                objective_ids(brief).join(", ")
            ));
            let stars: Vec<String> = brief
                .get("north_stars")
                .and_then(Value::as_array)
                .map_or(&[][..], Vec::as_slice)
                .iter()
                .map(|p| js_text(Some(p)))
                .collect();
            if !stars.is_empty() {
                rules.push(format!(
                    "- At least one priority cites a declared north star as a path source ({}). Cite only those as north stars.",
                    stars.join(", ")
                ));
            }
            rules.push("- Keep everything under <out_of_scope> out of the priorities.".into());
        }
        None => rules.push(
            "- At least one priority cites a north-star or guideline doc as a path source (CLAUDE.md, CONTEXT.md, PRODUCT.md, DESIGN.md, README.md or a file under docs/). Use the paths exactly as the agent answers or the snapshot give them.".into(),
        ),
    }
    rules.push("- \"owner_role\" is the id of the agent that proposed the priority most strongly, exactly as it heads its answer.".into());
    rules.push("- \"disagreements\": only real conflicts between agents; an empty array when there are none. Each position's \"role\" is an agent id.".into());
    rules.push(format!(
        "- \"decisions\": things that need the approval of {name} before anything changes. Each with its reason and an \"action\": what an agent would do after the approval, in one sentence. Each also lists in \"priorities\" the indexes (starting at 0, in the order of your \"priorities\") of the priorities it covers; an empty array when it covers none. Approving the decision marks those priorities accepted."
    ));
    rules.push(format!(
        "- \"script\": a team call in {language}, written to be spoken, in ASD-STE100 Simplified Technical English style applied to {language}: one topic in each sentence, at most 20 words in a sentence, the active voice with a named agent, simple verb forms, the condition first, one name for each thing. No slogans, metaphors, filler or fragments for effect. The pm opens by stating the question. Each agent that answered speaks its own view in 2 to 4 sentences, in first person. Refer to commits and files by what they are, never read hashes or paths aloud. Name a story by its title, never by its id. Add a short back-and-forth where agents disagree. The pm closes with the ranked priorities and the decisions {name} must take. Total about 2 to 4 minutes of speech. No markdown, no emojis. The \"role\" of each segment is the id of one of the agents that answered, or pm."
    ));
    rules.push(format!(
        "- Everything in {language} except identifiers, paths and code."
    ));
    parts.push(rules.join("\n"));
    if let Some(repair) = repair {
        let failures: Vec<String> = repair.failures.iter().map(|f| format!("- {f}")).collect();
        parts.push(format!(
            "# Repair\nYour previous output failed these checks. Fix them and return the full corrected output.\n\nFailures:\n{}\n\nPrevious output:\n{}",
            failures.join("\n"),
            repair.previous
        ));
    }
    parts.join("\n\n")
}

/// What the synthesis of one meeting reads.
pub struct SynthesisContext<'a> {
    pub question: &'a str,
    pub answers: &'a [(String, Value)],
    pub compact: &'a str,
    pub cwd: &'a Path,
    pub brief: Option<&'a Value>,
    pub owner: &'a Owner,
}

/// The synthesis call, made with the pm agent of the product as its system prompt.
pub fn synthesis_call(pm: &RoleDef, ctx: &SynthesisContext, repair: Option<&Repair>) -> Call {
    Call {
        budget_usd: None,
        prompt: synthesis_prompt(
            ctx.question,
            ctx.answers,
            ctx.compact,
            ctx.brief,
            ctx.owner,
            repair,
        ),
        model: pm.model.clone(),
        system: format!(
            "{}\n\nNow you write the minutes of the meeting from the agent answers.",
            pm.body
        ),
        cwd: ctx.cwd.to_string_lossy().into_owned(),
        tools: Vec::new(),
        schema: Some(synthesis_schema(ctx.brief, cites_tickets(ctx.compact))),
        effort: pm.effort.clone(),
        timeout: ROLE_TIMEOUT,
        mcp: None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_synthesis_schema_requires_an_objective_only_with_a_brief() {
        let brief = json!({"objectives": [{"id": "o1"}]});
        let with = synthesis_schema(Some(&brief), false);
        let props = &with["properties"]["priorities"]["items"];
        assert_eq!(props["properties"]["objective"]["enum"], json!(["o1"]));
        assert_eq!(props["required"][4], json!("objective"));
        let without = synthesis_schema(None, false);
        let props = &without["properties"]["priorities"]["items"];
        assert!(props["properties"].get("objective").is_none());
    }

    #[test]
    fn the_role_rule_lists_the_type_ticket_only_with_open_tickets() {
        let owner = Owner {
            name: "Ada".into(),
            language: "English".into(),
        };
        let prompt = |snapshot: &str| {
            let ctx = RoleContext {
                question: "q",
                snapshot,
                memory: "",
                scope: None,
                cwd: Path::new("/work"),
                brief: None,
                owner: &owner,
                code_graph: &[],
            };
            role_prompt(&ctx, "dev", false, true, &[])
        };
        let with = prompt("# Snapshot\n\n## Open tickets\n\n- C001 [note] x");
        assert!(
            with.contains("story (an id from the Stories section of the snapshot) or ticket (an id from the Open tickets section of the snapshot). "),
            "{with}"
        );
        let without = prompt("# Snapshot\n\n## Open issues\n\n_None indexed._");
        assert!(
            without.contains(
                "url (full http(s) URL) or story (an id from the Stories section of the snapshot). "
            ),
            "{without}"
        );
        assert!(!without.contains("ticket"), "{without}");
    }

    #[test]
    fn the_source_type_ticket_comes_only_with_open_tickets() {
        let types = |schema: &Value| {
            schema["properties"]["priorities"]["items"]["properties"]
            ["sources"]["items"]["properties"]["type"]["enum"]
            .clone()
        };
        assert_eq!(
            types(&synthesis_schema(None, false)),
            json!(["commit", "issue", "path", "url", "story"])
        );
        assert_eq!(
            types(&synthesis_schema(None, true)),
            json!(["commit", "issue", "path", "url", "story", "ticket"])
        );
        assert_eq!(
            types(&role_answer_schema(true)),
            json!(["commit", "issue", "path", "url", "story", "ticket"])
        );
        assert!(cites_tickets(
            "# Snapshot\n\n## Open tickets\n\n- C001 [note] x"
        ));
        assert!(!cites_tickets(
            "# Snapshot\n\n## Open issues\n\n## Open tickets later"
        ));
    }
}
