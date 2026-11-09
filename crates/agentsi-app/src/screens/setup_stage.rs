//! The values of the setup forms as plain data, and the stage files that they make.
//!
//! The setup screen reads its inputs into `Fields`. `Fields::stage` makes the text of the
//! stage files of `roles.apply`, by `plugin/skills/agentsi/setup.md` sections 4 and 5. This
//! module holds no view, so a test reads each rule without a window.

use std::collections::BTreeMap;

use serde_json::{Value, json};

/// The five global roles. `pm` is always in the roster.
pub const GLOBAL_ROLES: [&str; 5] = ["pm", "dev", "qa", "social", "scout"];

/// The stages of a product.
pub const STAGES: [&str; 5] = ["idea", "building", "launched", "growing", "maintenance"];

/// The forms of the screen, to show a problem at the form that owns the field.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum Form {
    Product,
    Objectives,
    Roster,
    Models,
    Stories,
    /// A failure that names no field.
    Other,
}

impl Form {
    /// The form that a failure of `roles.apply` is about.
    pub fn of_failure(failure: &str) -> Form {
        let f = failure.to_lowercase();
        let has = |words: &[&str]| words.iter().any(|w| f.contains(w));
        if has(&["stories"]) {
            Form::Stories
        } else if has(&["objective", "north_stars", "sources_of_truth"]) {
            Form::Objectives
        } else if has(&["product.", "out_of_scope"]) {
            Form::Product
        } else if has(&["model", "effort"]) {
            Form::Models
        } else if has(&["agent", "roster", "roles", "overlay", "extends"]) {
            Form::Roster
        } else {
            Form::Other
        }
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Objective {
    pub text: String,
    pub signal: String,
    pub due: String,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Source {
    pub kind: String,
    pub reference: String,
    pub note: String,
}

/// A specialist (`dev.backend`, an overlay) or a custom role (`seo`, a full definition).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Extra {
    pub id: String,
    pub body: String,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Model {
    pub model: String,
    pub effort: String,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Stories {
    pub on: bool,
    pub test: String,
    pub gate: Vec<String>,
    pub repo: String,
    pub budget: String,
}

/// What the Stories form does to the `stories` block of the registry.
#[derive(Clone, Debug, PartialEq)]
pub enum StoriesChange {
    Keep,
    Off,
    On(Value),
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Fields {
    pub id: String,
    pub name: String,
    pub paths: Vec<String>,
    pub what: String,
    pub audience: String,
    pub stage: String,
    pub out_of_scope: Vec<String>,
    pub objectives: Vec<Objective>,
    pub north_stars: Vec<String>,
    pub sources: Vec<Source>,
    /// The global roles in the roster. `pm` is always there.
    pub globals: Vec<String>,
    pub extras: Vec<Extra>,
    /// One line for each agent: why it is in the roster.
    pub why: BTreeMap<String, String>,
    pub models: BTreeMap<String, Model>,
    pub stories: Stories,
}

/// A field that the stage cannot hold, and the form that shows it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Problem {
    pub form: Form,
    pub text: String,
}

fn problem(form: Form, text: impl Into<String>) -> Problem {
    Problem {
        form,
        text: text.into(),
    }
}

/// The date `YYYY-MM-DD` with a real month and day.
pub fn is_date(s: &str) -> bool {
    let b = s.as_bytes();
    if b.len() != 10 || b[4] != b'-' || b[7] != b'-' {
        return false;
    }
    let num = |r: std::ops::Range<usize>| s[r].parse::<u32>().ok();
    match (num(0..4), num(5..7), num(8..10)) {
        (Some(_), Some(m), Some(d)) => (1..=12).contains(&m) && (1..=31).contains(&d),
        _ => false,
    }
}

/// An agent id: `dev` or `dev.backend`, lowercase letters and dashes.
pub fn is_agent_id(id: &str) -> bool {
    let part = |p: &str| {
        let mut c = p.chars();
        c.next().is_some_and(|f| f.is_ascii_lowercase())
            && c.all(|c| c.is_ascii_lowercase() || c == '-')
    };
    let mut parts = id.split('.');
    let (first, second) = (parts.next(), parts.next());
    first.is_some_and(part) && second.is_none_or(part) && parts.next().is_none()
}

/// A product id: lowercase letters, digits and dashes.
pub fn is_product_id(id: &str) -> bool {
    !id.is_empty()
        && !id.starts_with('-')
        && id
            .chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-')
}

fn clean(lines: &[String]) -> Vec<String> {
    lines
        .iter()
        .map(|l| l.trim().to_string())
        .filter(|l| !l.is_empty())
        .collect()
}

impl Fields {
    /// The agents of the roster: `pm` first, then the other global roles, then the extras.
    pub fn roster(&self) -> Vec<String> {
        let mut ids: Vec<String> = vec!["pm".into()];
        ids.extend(
            GLOBAL_ROLES
                .iter()
                .filter(|r| **r != "pm" && self.globals.iter().any(|g| g == **r))
                .map(|r| r.to_string()),
        );
        for e in &self.extras {
            let id = e.id.trim();
            if !id.is_empty() && !ids.iter().any(|i| i == id) {
                ids.push(id.to_string());
            }
        }
        ids
    }

    /// What stops the stage. Each item names the form that shows it.
    pub fn problems(&self) -> Vec<Problem> {
        let mut out = Vec::new();
        if !is_product_id(self.id.trim()) {
            out.push(problem(
                Form::Product,
                "The id needs lowercase letters, digits and dashes.",
            ));
        }
        if self.name.trim().is_empty() {
            out.push(problem(Form::Product, "The name is empty."));
        }
        let paths = clean(&self.paths);
        if paths.is_empty() {
            out.push(problem(
                Form::Product,
                "The product needs one path at least.",
            ));
        }
        if paths.iter().any(|p| !p.starts_with('/')) {
            out.push(problem(Form::Product, "Each path must be absolute."));
        }
        if self.what.trim().is_empty() {
            out.push(problem(Form::Product, "Say what the product is."));
        }
        if !STAGES.contains(&self.stage.as_str()) {
            out.push(problem(Form::Product, "Pick a stage."));
        }
        let objectives = self.filled_objectives();
        if objectives.is_empty() {
            out.push(problem(Form::Objectives, "Write one objective at least."));
        }
        for (i, o) in objectives.iter().enumerate() {
            if !is_date(o.due.trim()) {
                out.push(problem(
                    Form::Objectives,
                    format!("Objective {} needs a due date as YYYY-MM-DD.", i + 1),
                ));
            }
        }
        for s in &self.sources {
            if !["issues", "analytics", "docs", "other"].contains(&s.kind.trim()) {
                out.push(problem(
                    Form::Objectives,
                    "A source of truth is issues, analytics, docs or other.",
                ));
                break;
            }
        }
        for id in self.roster() {
            if !is_agent_id(&id) {
                out.push(problem(
                    Form::Roster,
                    format!("\"{id}\" is not an agent id."),
                ));
            }
            if self.why.get(&id).is_none_or(|w| w.trim().is_empty()) {
                out.push(problem(
                    Form::Roster,
                    format!("Say why {id} is in the roster."),
                ));
            }
        }
        for e in &self.extras {
            if !e.id.trim().is_empty() && e.body.trim().is_empty() {
                out.push(problem(
                    Form::Roster,
                    format!("{} needs an overlay or a definition.", e.id.trim()),
                ));
            }
        }
        if self.stories.on {
            if !self.stories.test.contains("{files}") {
                out.push(problem(
                    Form::Stories,
                    "The test command must contain {files}.",
                ));
            }
            if !self.stories.budget.trim().is_empty()
                && self
                    .stories
                    .budget
                    .trim()
                    .parse::<f64>()
                    .is_ok_and(|b| b <= 0.0)
            {
                out.push(problem(Form::Stories, "The budget must be more than 0."));
            }
        }
        out
    }

    pub fn filled_objectives(&self) -> Vec<&Objective> {
        self.objectives
            .iter()
            .filter(|o| !o.text.trim().is_empty())
            .take(3)
            .collect()
    }

    /// What the Stories form does, against the fields that the screen loaded. A new product
    /// has no baseline.
    pub fn stories_change(&self, baseline: Option<&Fields>) -> StoriesChange {
        match baseline {
            Some(b) if b.stories == self.stories => StoriesChange::Keep,
            Some(_) if !self.stories.on => StoriesChange::Off,
            None if !self.stories.on => StoriesChange::Keep,
            _ => {
                let mut block = json!({
                    "test": self.stories.test.trim(),
                    "gate": clean(&self.stories.gate),
                });
                if !self.stories.repo.trim().is_empty() {
                    block["repo"] = json!(self.stories.repo.trim());
                }
                if let Ok(b) = self.stories.budget.trim().parse::<f64>() {
                    block["budget_usd"] = json!(b);
                }
                StoriesChange::On(block)
            }
        }
    }

    /// The text of the stage files, by file name.
    pub fn stage(&self, baseline: Option<&Fields>, today: &str) -> BTreeMap<String, String> {
        let roster = self.roster();
        let mut files = BTreeMap::new();
        let objectives: Vec<Value> = self
            .filled_objectives()
            .iter()
            .enumerate()
            .map(|(i, o)| {
                json!({
                    "id": format!("o{}", i + 1),
                    "text": o.text.trim(),
                    "signal": o.signal.trim(),
                    "due": o.due.trim(),
                })
            })
            .collect();
        let sources: Vec<Value> = self
            .sources
            .iter()
            .filter(|s| !s.kind.trim().is_empty())
            .map(|s| json!({"kind": s.kind.trim(), "ref": s.reference.trim(), "note": s.note.trim()}))
            .collect();
        let roles: serde_json::Map<String, Value> = roster
            .iter()
            .map(|id| {
                let why = self.why.get(id).map(|w| w.trim()).unwrap_or_default();
                (id.clone(), json!(why))
            })
            .collect();
        let brief = json!({
            "version": 1,
            "product": {
                "what": self.what.trim(),
                "audience": self.audience.trim(),
                "stage": self.stage,
            },
            "objectives": objectives,
            "north_stars": clean(&self.north_stars),
            "sources_of_truth": sources,
            "out_of_scope": clean(&self.out_of_scope),
            "roles": roles,
            "updated_at": today,
        });
        files.insert("brief.json".into(), pretty(&brief));
        files.insert("roster.json".into(), pretty(&json!(roster)));
        for id in &roster {
            if let Some(text) = self.agent_file(id) {
                files.insert(format!("agents/{id}.md"), text);
            }
        }
        match self.stories_change(baseline) {
            StoriesChange::Keep => {}
            StoriesChange::Off => {
                files.insert("stories.json".into(), "null\n".into());
            }
            StoriesChange::On(block) => {
                files.insert("stories.json".into(), pretty(&block));
            }
        }
        files
    }

    /// The file of an agent that differs from its global role.
    fn agent_file(&self, id: &str) -> Option<String> {
        let model = self.models.get(id).cloned().unwrap_or_default();
        let (m, e) = (model.model.trim(), model.effort.trim());
        let extra = self.extras.iter().find(|x| x.id.trim() == id);
        let mut head = String::new();
        let keys = |head: &mut String| {
            if !m.is_empty() {
                head.push_str(&format!("model: {m}\n"));
            }
            if !e.is_empty() {
                head.push_str(&format!("effort: {e}\n"));
            }
        };
        match extra {
            Some(x) if x.body.trim_start().starts_with("---") => {
                Some(format!("{}\n", x.body.trim_end()))
            }
            Some(x) => {
                let body = x.body.trim();
                let base = match id.split_once('.') {
                    Some((base, _)) => Some(base),
                    None if GLOBAL_ROLES.contains(&id) => Some(id),
                    None => None,
                };
                match base {
                    Some(base) => {
                        head.push_str(&format!("extends: {base}\n"));
                        keys(&mut head);
                    }
                    None => {
                        let why = self.why.get(id).map(|w| w.trim()).unwrap_or(id);
                        head.push_str(&format!("name: {id}\ndescription: {why}\n"));
                        if m.is_empty() {
                            head.push_str("model: sonnet\n");
                        }
                        if e.is_empty() {
                            head.push_str("effort: medium\n");
                        }
                        keys(&mut head);
                    }
                }
                Some(format!("---\n{head}---\n\n{body}\n"))
            }
            None if !m.is_empty() || !e.is_empty() => {
                head.push_str(&format!("extends: {id}\n"));
                keys(&mut head);
                Some(format!("---\n{head}---\n"))
            }
            None => None,
        }
    }
}

fn pretty(v: &Value) -> String {
    format!("{}\n", serde_json::to_string_pretty(v).unwrap_or_default())
}

#[cfg(test)]
mod tests {
    use super::*;

    pub(crate) fn sample() -> Fields {
        Fields {
            id: "demo".into(),
            name: "Demo".into(),
            paths: vec!["/work/demo".into()],
            what: "A mail filter".into(),
            audience: "Small teams".into(),
            stage: "building".into(),
            out_of_scope: vec!["".into(), "Billing".into()],
            objectives: vec![
                Objective {
                    text: "Ship v1".into(),
                    signal: "10 users".into(),
                    due: "2026-12-31".into(),
                },
                Objective::default(),
            ],
            north_stars: vec!["docs/product.md".into()],
            sources: vec![],
            globals: vec!["dev".into()],
            extras: vec![],
            why: [("pm", "Ranks the work."), ("dev", "Judges the code.")]
                .into_iter()
                .map(|(k, v)| (k.to_string(), v.to_string()))
                .collect(),
            models: BTreeMap::new(),
            stories: Stories::default(),
        }
    }

    #[test]
    fn a_valid_form_makes_the_brief_and_the_roster_with_pm_first() {
        let f = sample();
        assert!(f.problems().is_empty(), "{:?}", f.problems());
        let stage = f.stage(None, "2026-11-09");
        let brief: Value = serde_json::from_str(&stage["brief.json"]).unwrap();
        assert_eq!(brief["version"], 1);
        assert_eq!(brief["objectives"].as_array().unwrap().len(), 1);
        assert_eq!(brief["objectives"][0]["id"], "o1");
        assert_eq!(brief["out_of_scope"], json!(["Billing"]));
        assert_eq!(brief["roles"]["dev"], "Judges the code.");
        assert_eq!(brief["updated_at"], "2026-11-09");
        let roster: Value = serde_json::from_str(&stage["roster.json"]).unwrap();
        assert_eq!(roster, json!(["pm", "dev"]));
        assert_eq!(
            stage.keys().collect::<Vec<_>>(),
            ["brief.json", "roster.json"]
        );
    }

    #[test]
    fn a_specialist_makes_an_overlay_and_a_custom_role_makes_a_full_definition() {
        let mut f = sample();
        f.extras = vec![
            Extra {
                id: "dev.backend".into(),
                body: "You cover the API.".into(),
            },
            Extra {
                id: "seo".into(),
                body: "You judge the search traffic.".into(),
            },
        ];
        f.why.insert("dev.backend".into(), "The API.".into());
        f.why.insert("seo".into(), "Search.".into());
        f.models.insert(
            "dev.backend".into(),
            Model {
                model: "opus".into(),
                effort: "high".into(),
            },
        );
        let stage = f.stage(None, "2026-11-09");
        assert_eq!(
            stage["agents/dev.backend.md"],
            "---\nextends: dev\nmodel: opus\neffort: high\n---\n\nYou cover the API.\n"
        );
        let seo = &stage["agents/seo.md"];
        assert!(
            seo.starts_with("---\nname: seo\ndescription: Search.\n"),
            "{seo}"
        );
        assert!(
            seo.contains("model: sonnet\n") && seo.ends_with("You judge the search traffic.\n")
        );
        assert!(!stage.contains_key("agents/dev.md"));
    }

    #[test]
    fn a_model_on_a_global_role_makes_an_overlay_with_only_the_changed_keys() {
        let mut f = sample();
        f.models.insert(
            "dev".into(),
            Model {
                model: "haiku".into(),
                effort: "".into(),
            },
        );
        assert_eq!(
            f.stage(None, "2026-11-09")["agents/dev.md"],
            "---\nextends: dev\nmodel: haiku\n---\n"
        );
    }

    #[test]
    fn the_stories_form_keeps_turns_off_or_sets_the_block() {
        let mut f = sample();
        assert_eq!(f.stories_change(None), StoriesChange::Keep);
        let base = f.clone();
        assert_eq!(f.stories_change(Some(&base)), StoriesChange::Keep);
        f.stories = Stories {
            on: true,
            test: "pnpm test {files}".into(),
            gate: vec!["pnpm lint".into(), " ".into()],
            repo: "".into(),
            budget: "3".into(),
        };
        let StoriesChange::On(block) = f.stories_change(Some(&base)) else {
            panic!("on")
        };
        assert_eq!(
            block,
            json!({"test": "pnpm test {files}", "gate": ["pnpm lint"], "budget_usd": 3.0})
        );
        assert!(
            f.stage(Some(&base), "2026-11-09")
                .contains_key("stories.json")
        );
        let on = f.clone();
        f.stories.on = false;
        assert_eq!(f.stories_change(Some(&on)), StoriesChange::Off);
        assert_eq!(f.stage(Some(&on), "2026-11-09")["stories.json"], "null\n");
    }

    #[test]
    fn a_bad_field_is_a_problem_at_its_form() {
        let mut f = sample();
        f.id = "Demo App".into();
        f.objectives[0].due = "soon".into();
        f.why.remove("dev");
        f.stories = Stories {
            on: true,
            test: "pnpm test".into(),
            ..Stories::default()
        };
        let forms: Vec<Form> = f.problems().iter().map(|p| p.form).collect();
        assert_eq!(
            forms,
            [Form::Product, Form::Objectives, Form::Roster, Form::Stories]
        );
    }

    #[test]
    fn a_failure_of_the_core_goes_to_the_form_that_it_is_about() {
        for (text, form) in [
            ("product.stage \"x\" is not one of idea.", Form::Product),
            (
                "Brief: objectives must hold 1 to 3 items, found 0.",
                Form::Objectives,
            ),
            (
                "Brief: north_stars path \"a\" does not exist under a product path.",
                Form::Objectives,
            ),
            ("Agent \"dev.x\": extends nothing", Form::Roster),
            ("roles has no entry for roster agent \"qa\".", Form::Roster),
            ("stories.json must be an object or null.", Form::Stories),
            ("Stage has no brief.json.", Form::Other),
        ] {
            assert_eq!(Form::of_failure(text), form, "{text}");
        }
    }

    #[test]
    fn dates_and_ids_follow_the_rules_of_the_core() {
        assert!(is_date("2026-12-31") && !is_date("2026-13-01") && !is_date("26-1-1"));
        assert!(is_agent_id("dev.backend") && is_agent_id("seo") && !is_agent_id("Dev"));
        assert!(!is_agent_id("a.b.c") && !is_agent_id("dev.") && !is_agent_id("1x"));
        assert!(is_product_id("my-app2") && !is_product_id("My App") && !is_product_id(""));
    }
}
