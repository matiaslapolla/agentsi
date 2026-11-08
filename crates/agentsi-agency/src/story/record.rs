//! The story record of `story.ts`: the validation, the slug, the Markdown view
//! and the actions that are legal for a story now.

use crate::clock;
use agentsi_proto::story_api::{Story, StorySession, StoryStop};
use regex::Regex;
use std::sync::LazyLock;
use unicode_normalization::UnicodeNormalization;

const TYPES: [&str; 2] = ["story", "task"];
const STATUSES: [&str; 3] = ["not_implemented", "implemented", "discarded"];
const APPROVALS: [&str; 3] = ["pending", "approved", "changes_requested"];
const SESSION_STATES: [&str; 6] = ["queued", "tests", "work", "checks", "ready", "failed"];
const RESULT_KINDS: [&str; 3] = ["merged_unchanged", "merged_changed", "discarded"];
const SOURCE_TYPES: [&str; 5] = ["commit", "issue", "path", "url", "story"];
pub const ACTIONS: [&str; 6] = ["approve", "changes", "discard", "run", "accept", "reject"];

static STORY_ID: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"^[ST]\d{3,}$").unwrap());
static SLUG: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"^[a-z0-9]+(-[a-z0-9]+)*$").unwrap());

pub fn is_story_id(id: &str) -> bool {
    STORY_ID.is_match(id)
}

pub fn prefix(kind: &str) -> &'static str {
    if kind == "task" { "T" } else { "S" }
}

/// The number of an id: `S014` gives 14.
pub fn number(id: &str) -> u64 {
    id.get(1..).and_then(|n| n.parse().ok()).unwrap_or(0)
}

fn text(v: &str) -> bool {
    !v.trim().is_empty()
}

fn is_time(v: &str) -> bool {
    clock::parse_iso_ms(v).is_some()
}

/// A repo-relative path: no leading slash, no backslash, no `.` or `..` segment.
pub fn is_repo_relative(path: &str) -> bool {
    let b = path.as_bytes();
    let drive = b.len() >= 2 && b[0].is_ascii_alphabetic() && b[1] == b':';
    if path.trim().is_empty() || path.starts_with('/') || path.contains('\\') || drive {
        return false;
    }
    path.split('/')
        .all(|seg| !seg.is_empty() && seg != "." && seg != "..")
}

fn session_failures(s: &StorySession, out: &mut Vec<String>) {
    if !SESSION_STATES.contains(&s.state.as_str()) {
        out.push(format!(
            "session.state \"{}\" is not one of {}.",
            s.state,
            SESSION_STATES.join(", ")
        ));
    }
    for (key, v) in [
        ("branch", &s.branch),
        ("worktree", &s.worktree),
        ("base", &s.base),
        ("log", &s.log),
    ] {
        if !text(v) {
            out.push(format!("session.{key} is empty."));
        }
    }
    if s.attempts < 0 {
        out.push("session.attempts must be a whole number of 0 or more.".into());
    }
    if let Some(Some(pid)) = s.pid
        && pid <= 0
    {
        out.push("session.pid must be a process id or null.".into());
    }
    if !is_time(&s.started_at) {
        out.push(format!(
            "session.started_at \"{}\" is not a date.",
            s.started_at
        ));
    }
    if let Some(Some(stop)) = &s.stop
        && !(stop.reason == "budget" && stop.cap_usd > 0.0)
    {
        out.push(
            "session.stop must be null or { reason: \"budget\", cap_usd } with a positive cap."
                .into(),
        );
    }
}

/// The failures of a story against the roster; empty when it is valid.
pub fn validate(s: &Story, roster: &[String]) -> Vec<String> {
    let mut f = Vec::new();
    let is_type = TYPES.contains(&s.kind.as_str());
    if !is_type {
        f.push(format!(
            "type \"{}\" is not one of {}.",
            s.kind,
            TYPES.join(", ")
        ));
    }
    if !is_story_id(&s.id) {
        f.push(format!(
            "id \"{}\" is not S or T followed by 3 or more digits.",
            s.id
        ));
    } else if is_type && !s.id.starts_with(prefix(&s.kind)) {
        f.push(format!(
            "id \"{}\" must start with {} for type \"{}\".",
            s.id,
            prefix(&s.kind),
            s.kind
        ));
    }
    if !SLUG.is_match(&s.slug) {
        f.push(format!("slug \"{}\" is not kebab-case.", s.slug));
    }
    if !text(&s.title) {
        f.push("title is empty.".into());
    }
    let as_a = s.as_a.as_deref();
    if s.kind == "story" && !as_a.is_some_and(text) {
        f.push("as_a is empty; a story of type \"story\" needs a user.".into());
    } else if as_a.is_some_and(|a| !text(a)) {
        f.push("as_a must be a user or null.".into());
    }
    if !text(&s.i_want) {
        f.push("i_want is empty.".into());
    }
    if !text(&s.so_that) {
        f.push("so_that is empty.".into());
    }
    let n = s.criteria.len();
    if !(2..=5).contains(&n) {
        f.push(format!("criteria must hold 2 to 5 items, found {n}."));
    }
    for (i, c) in s.criteria.iter().enumerate() {
        let at = format!("criteria[{}]", i + 1);
        for (key, v) in [
            ("given", &c.given),
            ("when", &c.when),
            ("then", &c.then),
            ("checks", &c.checks),
        ] {
            if !text(v) {
                f.push(format!("{at}.{key} is empty."));
            }
        }
        match &c.test {
            None if s.kind == "story" => f.push(format!(
                "{at}.test is null; a story of type \"story\" needs a test path."
            )),
            None => {}
            Some(t) if !is_repo_relative(t) => {
                f.push(format!("{at}.test \"{t}\" is not a repo-relative path."))
            }
            Some(_) => {}
        }
    }
    if !roster.contains(&s.owner_role) {
        f.push(format!(
            "owner_role \"{}\" is not in the roster ({}).",
            s.owner_role,
            roster.join(", ")
        ));
    }
    for src in &s.sources {
        if !SOURCE_TYPES.contains(&src.kind.as_str()) {
            f.push(format!(
                "Source type \"{}\" is not one of {}.",
                src.kind,
                SOURCE_TYPES.join(", ")
            ));
        }
        if !text(&src.reference) {
            f.push(format!(
                "A source of type \"{}\" has an empty ref.",
                src.kind
            ));
        } else if src.kind == "story" && !is_story_id(&src.reference) {
            f.push(format!(
                "A story source \"{}\" is not a story id.",
                src.reference
            ));
        }
    }
    if !STATUSES.contains(&s.status.as_str()) {
        f.push(format!(
            "status \"{}\" is not one of {}.",
            s.status,
            STATUSES.join(", ")
        ));
    }
    if !APPROVALS.contains(&s.approval.as_str()) {
        f.push(format!(
            "approval \"{}\" is not one of {}.",
            s.approval,
            APPROVALS.join(", ")
        ));
    }
    if s.approval == "changes_requested" && !s.changes_note.as_deref().is_some_and(text) {
        f.push("changes_note is empty; approval changes_requested needs a note.".into());
    }
    if s.proposed_by != "owner" && s.proposed_by != "pm" {
        f.push(format!(
            "proposed_by \"{}\" is not owner or pm.",
            s.proposed_by
        ));
    }
    if let Some(session) = &s.session {
        session_failures(session, &mut f);
    }
    for r in &s.results {
        if !RESULT_KINDS.contains(&r.result.as_str()) {
            f.push(format!(
                "result \"{}\" is not one of {}.",
                r.result,
                RESULT_KINDS.join(", ")
            ));
        }
        if !is_time(&r.at) {
            f.push(format!("A result has an invalid date \"{}\".", r.at));
        }
    }
    if !is_time(&s.created_at) {
        f.push(format!("created_at \"{}\" is not a date.", s.created_at));
    }
    if !is_time(&s.updated_at) {
        f.push(format!("updated_at \"{}\" is not a date.", s.updated_at));
    }
    f
}

/// Kebab-case of the title, without accents, at most 48 characters.
pub fn slug(title: &str) -> String {
    let plain: String = title
        .nfd()
        .filter(|c| !('\u{300}'..='\u{36f}').contains(c))
        .flat_map(char::to_lowercase)
        .collect();
    let mut slug = String::new();
    for c in plain.chars() {
        if c.is_ascii_lowercase() || c.is_ascii_digit() {
            slug.push(c);
        } else if !slug.ends_with('-') {
            slug.push('-');
        }
    }
    let slug = slug.trim_matches('-').to_string();
    if slug.len() <= 48 {
        return if slug.is_empty() {
            "story".into()
        } else {
            slug
        };
    }
    let cut = &slug[..48];
    match cut.rfind('-') {
        Some(dash) if dash > 0 => cut[..dash].into(),
        _ => cut.into(),
    }
}

/// `<id>-<slug>`: the name of the story branch and worktree.
pub fn file_base(s: &Story) -> String {
    format!("{}-{}", s.id, s.slug)
}

/// A JavaScript `toFixed` for the small positive amounts of the costs.
pub fn fixed(v: f64, digits: usize) -> String {
    format!("{v:.digits$}")
}

pub fn stop_text(stop: &StoryStop) -> String {
    format!(
        "Stopped: {}, cap USD {} for each session.",
        stop.reason,
        fixed(stop.cap_usd, 2)
    )
}

/// The stop of a session, when Claude Code stopped it at its cap.
pub fn stop_of(s: &StorySession) -> Option<&StoryStop> {
    s.stop.as_ref().and_then(Option::as_ref)
}

/// The Markdown view of a story, rendered from the story JSON on each read.
pub fn markdown(s: &Story) -> String {
    let mut out: Vec<String> = vec![format!("# {} {}", s.id, s.title), String::new()];
    out.push("<!-- Rendered from the story JSON in the database. -->".into());
    out.push(String::new());
    out.push(format!("- Type: {}", s.kind));
    out.push(format!("- Owner: {}", s.owner_role));
    out.push(format!("- Status: {}", s.status));
    out.push(format!("- Approval: {}", s.approval));
    if let (true, Some(note)) = (
        s.approval == "changes_requested",
        s.changes_note.as_deref().filter(|n| !n.is_empty()),
    ) {
        out.push(format!("- Changes requested: {note}"));
    }
    let minutes = match s.minutes.as_deref().filter(|m| !m.is_empty()) {
        Some(m) => format!(" in minutes {m}"),
        None => String::new(),
    };
    out.push(format!("- Proposed by: {}{minutes}", s.proposed_by));
    if let Some(o) = s.objective.as_deref().filter(|o| !o.is_empty()) {
        out.push(format!("- Objective: {o}"));
    }
    out.push(String::new());
    if let Some(a) = s.as_a.as_deref().filter(|a| !a.is_empty()) {
        out.push(format!("**As a** {a}"));
        out.push(String::new());
    }
    out.extend([
        format!("**I want** {}", s.i_want),
        String::new(),
        format!("**So that** {}", s.so_that),
        String::new(),
    ]);
    out.extend(["## Criteria".into(), String::new()]);
    for (i, c) in s.criteria.iter().enumerate() {
        out.extend([format!("### {}", i + 1), String::new()]);
        out.push(format!("- **Given** {}", c.given));
        out.push(format!("- **When** {}", c.when));
        out.push(format!("- **Then** {}", c.then));
        out.push(format!(
            "- Test: {}",
            match c.test.as_deref().filter(|t| !t.is_empty()) {
                Some(t) => format!("`{t}`"),
                None => "none".into(),
            }
        ));
        out.push(format!("- Checks: {}", c.checks));
        out.push(String::new());
    }
    if !s.sources.is_empty() {
        out.extend(["## Sources".into(), String::new()]);
        out.extend(
            s.sources
                .iter()
                .map(|src| format!("- {} `{}`", src.kind, src.reference)),
        );
        out.push(String::new());
    }
    if let Some(x) = &s.session {
        out.extend(["## Session".into(), String::new()]);
        out.push(format!("- State: {}", x.state));
        out.push(format!("- Branch: `{}`", x.branch));
        out.push(format!("- Attempts: {}", x.attempts));
        if let Some(c) = x.tests_commit.as_deref().filter(|c| !c.is_empty()) {
            out.push(format!("- Tests commit: `{c}`"));
        }
        if let Some(c) = x.final_commit.as_deref().filter(|c| !c.is_empty()) {
            out.push(format!("- Final commit: `{c}`"));
        }
        if let Some(stop) = stop_of(x) {
            out.push(format!("- {}", stop_text(stop)));
        }
        if let Some(f) = x.failure.as_deref().filter(|f| !f.is_empty()) {
            out.push(format!("- Failure: {f}"));
        }
        out.push(String::new());
    }
    if !s.results.is_empty() {
        out.extend(["## Results".into(), String::new()]);
        for r in &s.results {
            let commit = match r.commit.as_deref().filter(|c| !c.is_empty()) {
                Some(c) => format!(" (`{c}`)"),
                None => String::new(),
            };
            out.push(format!("- {}: {}{commit}", r.at, r.result));
        }
        out.push(String::new());
    }
    out.join("\n")
}

/// A session that is under way or waits for review. A failed one no longer holds the story.
pub fn has_active_session(s: &Story) -> bool {
    s.session.as_ref().is_some_and(|x| x.state != "failed")
}

/// A session whose process may still run: not ready and not failed.
pub fn is_running_session(s: &Story) -> bool {
    has_active_session(s) && s.session.as_ref().is_some_and(|x| x.state != "ready")
}

/// Why `action` is not legal for the story now, or `None` when it is.
pub fn action_error(s: &Story, action: &str) -> Option<String> {
    if s.status != "not_implemented" {
        return Some(format!("Story {} is {}.", s.id, s.status));
    }
    let state = s.session.as_ref().map(|x| x.state.as_str());
    let id = &s.id;
    match action {
        "approve" if s.approval != "pending" => Some(format!(
            "Story {id} has approval {}; only a pending story can be approved.",
            s.approval
        )),
        "approve" => None,
        "discard" => state.map(|st| {
            format!(
                "Story {id} has a story session in state {st}. Run agentsi story reject {id} first."
            )
        }),
        "changes" | "run" => {
            if has_active_session(s) {
                return Some(format!(
                    "Story {id} has a story session in state {}.",
                    state.unwrap_or_default()
                ));
            }
            if action == "run" && s.approval != "approved" {
                return Some(format!(
                    "Story {id} has approval {}; approve it before the run.",
                    s.approval
                ));
            }
            None
        }
        "accept" if state == Some("ready") => None,
        "accept" => Some(format!("Story {id} has no story session in state ready.")),
        "reject" if state.is_some() => None,
        "reject" => Some(format!("Story {id} has no story session.")),
        other => Some(format!("Unknown story action \"{other}\".")),
    }
}

/// The actions that are legal for the story now.
pub fn actions(s: &Story) -> Vec<String> {
    ACTIONS
        .iter()
        .filter(|a| action_error(s, a).is_none())
        .map(|a| a.to_string())
        .collect()
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use agentsi_proto::story_api::Criterion;

    pub fn story() -> Story {
        let criterion = |test: &str| Criterion {
            given: "g".into(),
            when: "w".into(),
            then: "t".into(),
            test: Some(test.into()),
            checks: "c".into(),
        };
        Story {
            id: "S001".into(),
            kind: "story".into(),
            slug: "ver-el-resultado".into(),
            title: "Ver el resultado".into(),
            as_a: Some("entrenador".into()),
            i_want: "ver".into(),
            so_that: "decida".into(),
            criteria: vec![criterion("test/a.test.ts"), criterion("test/b.test.ts")],
            owner_role: "dev".into(),
            objective: None,
            sources: Vec::new(),
            status: "not_implemented".into(),
            approval: "pending".into(),
            changes_note: None,
            proposed_by: "owner".into(),
            minutes: None,
            session: None,
            results: Vec::new(),
            created_at: "2026-10-01T10:00:00.000Z".into(),
            updated_at: "2026-10-01T10:00:00.000Z".into(),
            archived: None,
        }
    }

    #[test]
    fn a_valid_story_has_no_failure_and_a_bad_one_names_each_field() {
        let roster = vec!["dev".to_string(), "qa".to_string()];
        assert_eq!(validate(&story(), &roster), Vec::<String>::new());
        let mut bad = story();
        bad.id = "T001".into();
        bad.criteria.truncate(1);
        bad.criteria[0].test = Some("../x".into());
        bad.owner_role = "ghost".into();
        bad.approval = "changes_requested".into();
        let failures = validate(&bad, &roster);
        assert_eq!(
            failures,
            vec![
                "id \"T001\" must start with S for type \"story\".",
                "criteria must hold 2 to 5 items, found 1.",
                "criteria[1].test \"../x\" is not a repo-relative path.",
                "owner_role \"ghost\" is not in the roster (dev, qa).",
                "changes_note is empty; approval changes_requested needs a note.",
            ]
        );
    }

    #[test]
    fn the_slug_drops_accents_and_cuts_at_a_dash() {
        assert_eq!(
            slug("Ver las stories en la web"),
            "ver-las-stories-en-la-web"
        );
        assert_eq!(slug("Añadir búsqueda rápida!"), "anadir-busqueda-rapida");
        assert_eq!(slug("¿?"), "story");
        let long = slug(&"palabra ".repeat(10));
        assert!(long.len() <= 48 && !long.ends_with('-'), "{long}");
    }

    #[test]
    fn the_actions_follow_the_status_the_approval_and_the_session() {
        let mut s = story();
        assert_eq!(actions(&s), vec!["approve", "changes", "discard"]);
        s.approval = "approved".into();
        assert_eq!(actions(&s), vec!["changes", "discard", "run"]);
        s.session = Some(StorySession {
            state: "ready".into(),
            branch: "b".into(),
            worktree: "w".into(),
            base: "x".into(),
            tests_commit: None,
            final_commit: None,
            attempts: 1,
            failure: None,
            log: "l".into(),
            started_at: "2026-10-01T10:00:00.000Z".into(),
            pid: None,
            stop: None,
        });
        assert_eq!(actions(&s), vec!["accept", "reject"]);
        s.status = "implemented".into();
        assert!(actions(&s).is_empty());
    }

    #[test]
    fn a_repo_relative_path_has_no_dot_segment_and_no_root() {
        assert!(is_repo_relative("test/a.test.ts"));
        for bad in ["/abs", "a/../b", "./a", "a//b", "C:/x", "a\\b", " "] {
            assert!(!is_repo_relative(bad), "{bad}");
        }
    }
}
