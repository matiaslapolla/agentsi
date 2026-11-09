//! The setup screen: it registers a product, or updates its brief, from the app.
//!
//! The steps are Folder, Forms, Review and Done. No write call leaves the screen before the
//! owner's Confirm in the Review step. `roles.apply` without `yes` makes the plan. The inline
//! stage keeps the files off the product repo and off the clipboard. See
//! `docs/specs/agentsi-desktop-setup.md`.

use std::collections::{BTreeMap, BTreeSet, VecDeque};

use agentsi_proto::marks_api::{DecisionAdd, DecisionAddParams};
use agentsi_proto::method::Method;
use agentsi_proto::registry_api::{
    ProductChange, RegistryGet, RegistryResult, RegistryUpdate, RegistryUpdateParams,
};
use agentsi_proto::roles_api::{
    ApplyPlan, BriefView, Registration, RoleDef, RoleSource, RolesApply, RolesApplyParams,
    RolesApplyResult, RolesBrief, RolesBriefParams, RolesList, RolesListParams, RolesListResult,
};
use agentsi_proto::setup_api::{
    SetupDraft, SetupDraftParams, SetupDraftResult, SetupOutcome, SetupOutcomeParams, SetupScan,
    SetupScanParams, SetupScanResult,
};
use agentsi_proto::story_api::{StoryConfig, StoryConfigGet, StoryConfigParams};
use agentsi_ui::components::{Button, neutral_tag, status_tag};
use agentsi_ui::tokens::{Status, line, radius, size, space, status, surface, text, weight};
use gpui_kit::component::checkbox::Checkbox;
use gpui_kit::component::input::{Input, InputState, Textarea, TextareaState};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;

use super::ScreenSpec;
use super::parts::{column, mono, section};
use super::roles::apply_lines;
use super::setup_stage::{
    Extra, Fields, Form, GLOBAL_ROLES, Model, Objective, Problem, STAGES, Source, Stories,
};
use crate::model::CoreState;
use crate::nav::Nav;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Step {
    Folder,
    Forms,
    Review,
    Done,
}

/// A write of the apply step, in the order that the screen sends them.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Write {
    Apply,
    Registry,
    Decision,
    Outcome,
}

/// The cap of one `setup.draft` call, as the core sets it.
const DRAFT_CAP: &str = "0.50";

/// What the screen knows about the draft of Claude.
#[derive(Default)]
struct DraftState {
    /// The owner clicked "Draft with Claude" once. The second click runs the call.
    armed: bool,
    repair_armed: bool,
    running: bool,
    error: Option<String>,
    /// The id, the cost and the time of the last draft, and the fields that it gave.
    last: Option<Drafted>,
    /// A repair runs one time.
    repaired: bool,
    /// What `roles.apply` refused after the draft.
    failures: Vec<String>,
}

struct Drafted {
    id: String,
    cost_usd: f64,
    seconds: f64,
    no_data: Vec<String>,
    fields: Fields,
}

/// The input keys that a field path of a draft fills.
fn keys_of(path: &str) -> &'static [&'static str] {
    match path {
        "product.id" => &["id"],
        "product.name" => &["name"],
        "product.what" => &["what"],
        "product.audience" => &["audience"],
        "product.out_of_scope" => &["out"],
        "objectives.objectives" => &["o0.text"],
        "objectives.north_stars" => &["north"],
        "objectives.sources_of_truth" => &["sources"],
        "stories.test" => &["test"],
        "stories.gate" => &["gate"],
        _ => &[],
    }
}

/// What the screen reads from the core to fill the forms of a registered product.
pub struct Existing {
    pub brief: Option<BriefView>,
    pub list: RolesListResult,
    pub stories: Option<StoryConfig>,
    pub registry: RegistryResult,
    pub defs: Vec<RoleDef>,
}

/// The decision of `setup.md` section 6, for a product with no north star.
const NORTH_STAR_TEXT: &str =
    "Write docs/product.md with the vision, the audience and what stays out.";
const NORTH_STAR_WHY: &str = "The repo has no north star, so the priorities cannot cite one.";

const DEFAULT_WHY: [(&str, &str); 5] = [
    ("pm", "Ranks the work and writes the synthesis."),
    ("dev", "Judges the technical state, the debt and the cost."),
    ("qa", "Judges the quality risk and what is untested."),
    ("social", "Judges what to show, tell or launch."),
    (
        "scout",
        "Judges the tools, the libraries and the competitors.",
    ),
];

pub struct SetupScreen {
    /// The product to update, or "" for a new one.
    product: String,
    core: Entity<CoreState>,
    step: Step,
    inputs: BTreeMap<String, Entity<InputState>>,
    areas: BTreeMap<String, Entity<TextareaState>>,
    /// The text of each input when the forms were filled, to mark what the owner changed.
    base_vals: BTreeMap<String, String>,
    /// Inputs to set at the next draw, which has the window.
    puts: Vec<(String, String)>,
    scan: Option<SetupScanResult>,
    /// The fields that the stored brief gave. `None` for a new product.
    baseline: Option<Fields>,
    stage: String,
    globals: Vec<String>,
    stories_on: bool,
    /// The keys of the rows of extra agents.
    extras: Vec<u32>,
    next_extra: u32,
    problems: Vec<Problem>,
    plan: Option<ApplyPlan>,
    north_star_decision: bool,
    busy: bool,
    message: Option<Result<String, String>>,
    writes: VecDeque<Write>,
    /// The notes of the apply step: what each write did.
    notes: Vec<String>,
    applied_id: Option<String>,
    fields: Fields,
    scroll: ScrollHandle,
    draft: DraftState,
    /// The keys of the fields that the draft could not answer.
    no_data: BTreeSet<String>,
    /// A fill from a draft keeps the marks against the stored brief.
    keep_marks: bool,
}

impl SetupScreen {
    pub fn new(product: String, core: Entity<CoreState>, cx: &mut Context<Self>) -> Self {
        let mut screen = SetupScreen {
            product,
            core,
            step: Step::Folder,
            inputs: BTreeMap::new(),
            areas: BTreeMap::new(),
            base_vals: BTreeMap::new(),
            puts: Vec::new(),
            scan: None,
            baseline: None,
            stage: "building".into(),
            globals: vec!["dev".into(), "qa".into()],
            stories_on: false,
            extras: Vec::new(),
            next_extra: 0,
            problems: Vec::new(),
            plan: None,
            north_star_decision: false,
            busy: false,
            message: None,
            writes: VecDeque::new(),
            notes: Vec::new(),
            applied_id: None,
            fields: Fields::default(),
            scroll: ScrollHandle::new(),
            draft: DraftState::default(),
            no_data: BTreeSet::new(),
            keep_marks: false,
        };
        screen.reload(cx);
        screen
    }

    /// A product to update starts from the folder in the registry.
    pub fn reload(&mut self, cx: &mut Context<Self>) {
        if self.product.is_empty() || self.scan.is_some() || self.step != Step::Folder {
            return;
        }
        let product = self.product.clone();
        self.ask::<RegistryGet>(Default::default(), cx, move |this, r, cx| match r {
            Ok(r) => match r
                .products
                .iter()
                .find(|p| p.id == product)
                .and_then(|p| p.paths.first())
            {
                Some(path) => {
                    this.puts.push(("path".into(), path.clone()));
                    this.scan_path(path.clone(), cx);
                }
                None => this.fail(format!("The registry has no folder for {product}."), cx),
            },
            Err(e) => this.fail(e, cx),
        });
    }

    #[cfg(test)]
    pub fn step(&self) -> Step {
        self.step
    }

    fn fail(&mut self, e: String, cx: &mut Context<Self>) {
        self.busy = false;
        self.message = Some(Err(e));
        cx.notify();
    }

    /// Sends one call and hands the reply to `done`.
    fn ask<M: Method + 'static>(
        &mut self,
        params: M::Params,
        cx: &mut Context<Self>,
        done: impl FnOnce(&mut Self, Result<M::Result, String>, &mut Context<Self>) + 'static,
    ) {
        let call = self.core.read(cx).link.call::<M>(&params);
        cx.spawn(async move |this, cx| {
            let result = call.await.map_err(|e| e.to_string());
            let _ = this.update(cx, |this, cx| done(this, result, cx));
        })
        .detach();
    }

    // ---- inputs ----

    fn input(
        &mut self,
        key: &str,
        placeholder: &'static str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Entity<InputState> {
        if let Some(i) = self.inputs.get(key) {
            return i.clone();
        }
        let input = cx.new(|cx| InputState::new(window, cx).placeholder(placeholder));
        self.inputs.insert(key.into(), input.clone());
        input
    }

    fn area(
        &mut self,
        key: &str,
        placeholder: &'static str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Entity<TextareaState> {
        if let Some(a) = self.areas.get(key) {
            return a.clone();
        }
        let area = cx.new(|cx| TextareaState::new(window, cx).placeholder(placeholder));
        self.areas.insert(key.into(), area.clone());
        area
    }

    fn val(&self, key: &str, cx: &App) -> String {
        if let Some(i) = self.inputs.get(key) {
            return i.read(cx).value().trim().to_string();
        }
        self.areas
            .get(key)
            .map(|a| a.read(cx).value().trim().to_string())
            .unwrap_or_default()
    }

    fn lines(&self, key: &str, cx: &App) -> Vec<String> {
        let text = match self.areas.get(key) {
            Some(a) => a.read(cx).value().to_string(),
            None => String::new(),
        };
        text.lines()
            .map(|l| l.trim().to_string())
            .filter(|l| !l.is_empty())
            .collect()
    }

    /// Sets an input or a text area, and makes it if the draw has not yet.
    fn put(&mut self, key: &str, value: &str, window: &mut Window, cx: &mut Context<Self>) {
        let multi =
            matches!(key, "paths" | "out" | "north" | "gate" | "sources") || key.ends_with(".body");
        if multi {
            let a = self.area(key, "", window, cx);
            a.update(cx, |s, cx| s.set_value(value.to_string(), window, cx));
        } else {
            let i = self.input(key, "", window, cx);
            i.update(cx, |s, cx| s.set_value(value.to_string(), window, cx));
        }
        if !self.keep_marks {
            self.base_vals.insert(key.into(), value.trim().to_string());
        }
    }

    /// True when the draft could not answer the field and it is still empty.
    fn no_data(&self, key: &str, cx: &App) -> bool {
        self.no_data.contains(key) && self.val(key, cx).is_empty()
    }

    fn changed(&self, key: &str, cx: &App) -> bool {
        self.baseline.is_some()
            && self
                .base_vals
                .get(key)
                .is_some_and(|b| *b != self.val(key, cx))
    }

    /// Fills the inputs from `f`. The next draw runs it, because it needs the window.
    fn set_fields(&mut self, f: Fields, cx: &mut Context<Self>) {
        self.stage = f.stage.clone();
        self.globals = f.globals.clone();
        self.stories_on = f.stories.on;
        self.extras = (0..f.extras.len() as u32).collect();
        self.next_extra = f.extras.len() as u32;
        self.fields = f;
        if !self.keep_marks {
            self.base_vals.clear();
        }
        cx.notify();
    }

    fn apply_fill(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        for (k, v) in std::mem::take(&mut self.puts) {
            self.put(&k, &v, window, cx);
            if k == "path" {
                self.base_vals.remove("path");
            }
        }
        if self.fields == Fields::default() {
            return;
        }
        let f = std::mem::take(&mut self.fields);
        self.put("id", &f.id, window, cx);
        self.put("name", &f.name, window, cx);
        self.put("paths", &f.paths.join("\n"), window, cx);
        self.put("what", &f.what, window, cx);
        self.put("audience", &f.audience, window, cx);
        self.put("out", &f.out_of_scope.join("\n"), window, cx);
        for i in 0..3 {
            let o = f.objectives.get(i).cloned().unwrap_or_default();
            self.put(&format!("o{i}.text"), &o.text, window, cx);
            self.put(&format!("o{i}.signal"), &o.signal, window, cx);
            self.put(&format!("o{i}.due"), &o.due, window, cx);
        }
        self.put("north", &f.north_stars.join("\n"), window, cx);
        let sources: Vec<String> = f
            .sources
            .iter()
            .map(|s| format!("{} | {} | {}", s.kind, s.reference, s.note))
            .collect();
        self.put("sources", &sources.join("\n"), window, cx);
        for (n, e) in f.extras.iter().enumerate() {
            self.put(&format!("x{n}.id"), &e.id, window, cx);
            self.put(&format!("x{n}.body"), &e.body, window, cx);
        }
        for (id, w) in &f.why {
            self.put(&format!("why.{id}"), w, window, cx);
        }
        for (id, m) in &f.models {
            self.put(&format!("model.{id}"), &m.model, window, cx);
            self.put(&format!("effort.{id}"), &m.effort, window, cx);
        }
        self.put("test", &f.stories.test, window, cx);
        self.put("gate", &f.stories.gate.join("\n"), window, cx);
        self.put("repo", &f.stories.repo, window, cx);
        self.put("budget", &f.stories.budget, window, cx);
        self.keep_marks = false;
    }

    /// Reads the inputs into `Fields`.
    pub fn read_fields(&self, cx: &App) -> Fields {
        let mut f = Fields {
            id: self.val("id", cx),
            name: self.val("name", cx),
            paths: self.lines("paths", cx),
            what: self.val("what", cx),
            audience: self.val("audience", cx),
            stage: self.stage.clone(),
            out_of_scope: self.lines("out", cx),
            north_stars: self.lines("north", cx),
            globals: self.globals.clone(),
            ..Fields::default()
        };
        f.objectives = (0..3)
            .map(|i| Objective {
                text: self.val(&format!("o{i}.text"), cx),
                signal: self.val(&format!("o{i}.signal"), cx),
                due: self.val(&format!("o{i}.due"), cx),
            })
            .collect();
        f.sources = self
            .lines("sources", cx)
            .iter()
            .map(|l| {
                let mut p = l.splitn(3, '|').map(|s| s.trim().to_string());
                Source {
                    kind: p.next().unwrap_or_default(),
                    reference: p.next().unwrap_or_default(),
                    note: p.next().unwrap_or_default(),
                }
            })
            .collect();
        f.extras = self
            .extras
            .iter()
            .map(|n| Extra {
                id: self.val(&format!("x{n}.id"), cx),
                body: self.val(&format!("x{n}.body"), cx),
            })
            .filter(|e| !e.id.is_empty())
            .collect();
        for id in f.roster() {
            f.why.insert(id.clone(), self.val(&format!("why.{id}"), cx));
            let m = Model {
                model: self.val(&format!("model.{id}"), cx),
                effort: self.val(&format!("effort.{id}"), cx),
            };
            if m != Model::default() {
                f.models.insert(id, m);
            }
        }
        f.stories = Stories {
            on: self.stories_on,
            test: self.val("test", cx),
            gate: self.lines("gate", cx),
            repo: self.val("repo", cx),
            budget: self.val("budget", cx),
        };
        f
    }

    // ---- step 1: the folder ----

    pub fn scan_now(&mut self, cx: &mut Context<Self>) {
        let path = self.val("path", cx);
        self.scan_path(path, cx);
    }

    fn scan_path(&mut self, path: String, cx: &mut Context<Self>) {
        if self.busy {
            return;
        }
        self.busy = true;
        self.message = None;
        cx.notify();
        self.ask::<SetupScan>(SetupScanParams { path }, cx, |this, r, cx| {
            this.scanned(r, cx)
        });
    }

    pub fn scanned(&mut self, r: Result<SetupScanResult, String>, cx: &mut Context<Self>) {
        self.busy = false;
        let scan = match r {
            Ok(s) => s,
            Err(e) => return self.fail(e, cx),
        };
        self.scan = Some(scan.clone());
        match scan.registered.clone() {
            Some(id) => {
                self.product = id.clone();
                self.load_existing(id, scan, cx)
            }
            None => {
                self.product.clear();
                self.baseline = None;
                self.set_fields(Self::defaults(&scan), cx);
                self.step = Step::Forms;
                cx.notify();
            }
        }
    }

    /// The forms of a new product, from what the scan found.
    fn defaults(scan: &SetupScanResult) -> Fields {
        let globals = vec!["dev".to_string(), "qa".to_string()];
        let mut why: BTreeMap<String, String> = DEFAULT_WHY
            .iter()
            .map(|(k, v)| (k.to_string(), v.to_string()))
            .collect();
        why.retain(|k, _| k == "pm" || globals.contains(k));
        let (test, gate) = scan
            .manifest
            .as_ref()
            .map(|m| (m.test.clone().unwrap_or_default(), m.gate.clone()))
            .unwrap_or_default();
        Fields {
            id: scan.id.clone(),
            name: scan.name.clone(),
            paths: vec![scan.path.clone()],
            stage: "building".into(),
            objectives: vec![Objective::default(); 3],
            north_stars: scan.north_stars.clone(),
            globals,
            why,
            stories: Stories {
                on: false,
                test,
                gate,
                ..Stories::default()
            },
            ..Fields::default()
        }
    }

    fn load_existing(&mut self, id: String, scan: SetupScanResult, cx: &mut Context<Self>) {
        self.busy = true;
        cx.notify();
        let link = self.core.read(cx).link.clone();
        let brief = link.call::<RolesBrief>(&RolesBriefParams {
            product: id.clone(),
        });
        let list = link.call::<RolesList>(&RolesListParams {
            product: id.clone(),
            agent: None,
        });
        let stories = link.call::<StoryConfigGet>(&StoryConfigParams {
            product: id.clone(),
            run: false,
        });
        let registry = link.call::<RegistryGet>(&Default::default());
        cx.spawn(async move |this, cx| {
            let loaded = async {
                let brief = brief.await.map_err(|e| e.to_string())?.brief;
                let list = list.await.map_err(|e| e.to_string())?;
                // A product with Stories mode off answers with an error.
                let stories = stories.await.ok();
                let registry = registry.await.map_err(|e| e.to_string())?;
                let mut defs = Vec::new();
                for e in list.roster.iter().flatten() {
                    if matches!(e.source, Some(RoleSource::Overlay | RoleSource::Custom)) {
                        let def = link
                            .call::<RolesList>(&RolesListParams {
                                product: id.clone(),
                                agent: Some(e.id.clone()),
                            })
                            .await
                            .map_err(|e| e.to_string())?;
                        defs.extend(def.agent);
                    }
                }
                Ok(Existing {
                    brief,
                    list,
                    stories,
                    registry,
                    defs,
                })
            }
            .await;
            let _ = this.update(cx, |this, cx| this.existing_loaded(scan, loaded, cx));
        })
        .detach();
    }

    pub fn existing_loaded(
        &mut self,
        scan: SetupScanResult,
        r: Result<Existing, String>,
        cx: &mut Context<Self>,
    ) {
        self.busy = false;
        let existing = match r {
            Ok(e) => e,
            Err(e) => return self.fail(e, cx),
        };
        let fields = Self::stored(&scan, &existing);
        self.baseline = Some(fields.clone());
        self.set_fields(fields, cx);
        self.step = Step::Forms;
        cx.notify();
    }

    /// The forms of a registered product, from its stored brief, roster and Stories block.
    fn stored(scan: &SetupScanResult, e: &Existing) -> Fields {
        let product = e
            .registry
            .products
            .iter()
            .find(|p| Some(&p.id) == scan.registered.as_ref());
        let mut f = Self::defaults(scan);
        f.id = scan.registered.clone().unwrap_or_else(|| scan.id.clone());
        if let Some(p) = product {
            f.name = p.name.clone();
            f.paths = p.paths.clone();
        }
        if let Some(b) = &e.brief {
            f.what = b.what.clone();
            f.audience = b.audience.clone();
            f.stage = b.stage.clone();
            f.out_of_scope = b.out_of_scope.clone();
            f.north_stars = b.north_stars.clone();
            f.sources = b
                .sources_of_truth
                .iter()
                .map(|s| Source {
                    kind: s.kind.clone(),
                    reference: s.reference.clone(),
                    note: s.note.clone(),
                })
                .collect();
            f.objectives = b
                .objectives
                .iter()
                .map(|o| Objective {
                    text: o.text.clone(),
                    signal: o.signal.clone(),
                    due: o.due.clone(),
                })
                .collect();
            f.objectives.resize(3, Objective::default());
            f.why = b.roles.clone().into_iter().collect();
        } else {
            f.north_stars = Vec::new();
        }
        f.globals.clear();
        f.extras.clear();
        f.models.clear();
        for entry in e.list.roster.iter().flatten() {
            let def = e.defs.iter().find(|d| d.id == entry.id);
            let global = GLOBAL_ROLES.contains(&entry.id.as_str());
            if global && def.is_none() {
                f.globals.push(entry.id.clone());
            } else if let Some(d) = def {
                if global {
                    f.globals.push(entry.id.clone());
                }
                let body = match d.source {
                    RoleSource::Custom => d.body.clone(),
                    _ => d
                        .body
                        .rsplit_once("\n\n## This product\n\n")
                        .map(|(_, o)| o.to_string())
                        .unwrap_or_default(),
                };
                f.extras.push(Extra {
                    id: entry.id.clone(),
                    body,
                });
                f.models.insert(
                    entry.id.clone(),
                    Model {
                        model: d.model.clone(),
                        effort: d.effort.clone().unwrap_or_default(),
                    },
                );
            }
        }
        f.stories = match &e.stories {
            Some(s) => Stories {
                on: true,
                test: s.test.clone(),
                gate: s.gate.clone(),
                repo: s.repo.clone(),
                budget: s.budget_usd.to_string(),
            },
            None => Stories {
                on: false,
                ..f.stories
            },
        };
        f
    }

    // ---- step 2: the draft ----

    /// The first click arms the button and shows the cap. The second click runs the call.
    pub fn draft_click(&mut self, cx: &mut Context<Self>) {
        if self.draft.running || self.scan.is_none() {
            return;
        }
        if !self.draft.armed {
            self.draft.armed = true;
            cx.notify();
            return;
        }
        self.draft.armed = false;
        self.run_draft(None, cx);
    }

    /// A repair sends the failures of `roles.apply` and the forms as they are. It costs money
    /// too, so it has its own Confirm, and it runs one time.
    pub fn repair_click(&mut self, cx: &mut Context<Self>) {
        if self.draft.running
            || self.draft.repaired
            || self.draft.failures.is_empty()
            || self.scan.is_none()
        {
            return;
        }
        if !self.draft.repair_armed {
            self.draft.repair_armed = true;
            cx.notify();
            return;
        }
        self.draft.repair_armed = false;
        let previous = self.read_fields(cx).to_draft();
        let failures = self.draft.failures.clone();
        self.draft.repaired = true;
        self.run_draft(Some((previous, failures)), cx);
    }

    fn cancel_draft_confirm(&mut self, cx: &mut Context<Self>) {
        self.draft.armed = false;
        self.draft.repair_armed = false;
        cx.notify();
    }

    fn run_draft(
        &mut self,
        repair: Option<(serde_json::Value, Vec<String>)>,
        cx: &mut Context<Self>,
    ) {
        let Some(path) = self.scan.as_ref().map(|s| s.path.clone()) else {
            return;
        };
        self.draft.running = true;
        self.draft.error = None;
        cx.notify();
        let (previous, failures) = match repair {
            Some((p, f)) => (Some(p), f),
            None => (None, Vec::new()),
        };
        let params = SetupDraftParams {
            path,
            previous,
            failures,
        };
        self.ask::<SetupDraft>(params, cx, |this, r, cx| this.drafted(r, cx));
    }

    /// Fills the forms from the draft. The id, the name, the paths, the repo and the budget
    /// stay as they are: the folder and the owner decide them.
    pub fn drafted(&mut self, r: Result<SetupDraftResult, String>, cx: &mut Context<Self>) {
        self.draft.running = false;
        let d = match r {
            Ok(d) => d,
            Err(e) => {
                self.draft.error = Some(e);
                cx.notify();
                return;
            }
        };
        let current = self.read_fields(cx);
        let mut f = Fields::from_draft(&d.fields);
        f.id = current.id;
        f.name = current.name;
        f.paths = current.paths;
        f.stories.repo = current.stories.repo;
        f.stories.budget = current.stories.budget;
        for id in f.roster() {
            if f.why.get(&id).is_none_or(|w| w.trim().is_empty())
                && let Some((_, w)) = DEFAULT_WHY.iter().find(|(k, _)| *k == id)
            {
                f.why.insert(id, w.to_string());
            }
        }
        self.no_data = d
            .no_data
            .iter()
            .flat_map(|p| keys_of(p))
            .map(|k| k.to_string())
            .collect();
        self.draft.last = Some(Drafted {
            id: d.draft,
            cost_usd: d.cost_usd,
            seconds: d.seconds,
            no_data: d.no_data,
            fields: f.clone(),
        });
        self.draft.failures.clear();
        self.problems.clear();
        self.message = None;
        self.keep_marks = true;
        self.set_fields(f, cx);
    }

    // ---- step 3: review ----

    /// Checks the forms and asks `roles.apply` for the plan. It writes nothing.
    pub fn review_now(&mut self, cx: &mut Context<Self>) {
        if self.busy {
            return;
        }
        let fields = self.read_fields(cx);
        self.problems = fields.problems();
        self.message = None;
        if !self.problems.is_empty() {
            cx.notify();
            return;
        }
        self.busy = true;
        cx.notify();
        let params = self.apply_params(&fields, false);
        self.ask::<RolesApply>(params, cx, |this, r, cx| this.planned(r, cx));
    }

    fn today() -> String {
        crate::format::date_from_secs(crate::format::now_secs())
    }

    fn apply_params(&self, f: &Fields, yes: bool) -> RolesApplyParams {
        RolesApplyParams {
            product: f.id.clone(),
            stage_dir: None,
            stage: Some(f.stage(self.baseline.as_ref(), &Self::today())),
            yes,
            register: self.baseline.is_none().then(|| Registration {
                name: f.name.clone(),
                path: f.paths.first().cloned().unwrap_or_default(),
            }),
        }
    }

    pub fn planned(&mut self, r: Result<RolesApplyResult, String>, cx: &mut Context<Self>) {
        self.busy = false;
        match r {
            Ok(RolesApplyResult {
                plan: Some(plan), ..
            }) => {
                self.plan = Some(plan);
                self.step = Step::Review;
                cx.notify();
            }
            Ok(r) => {
                self.draft.failures = r.failures.clone();
                self.problems = r
                    .failures
                    .iter()
                    .map(|f| Problem {
                        form: Form::of_failure(f),
                        text: f.clone(),
                    })
                    .collect();
                self.message = Some(Err("The core refused the stage.".into()));
                cx.notify();
            }
            Err(e) => self.fail(e, cx),
        }
    }

    // ---- step 5: apply ----

    /// The Confirm of the Review step. This is the first call that writes.
    pub fn confirm(&mut self, cx: &mut Context<Self>) {
        if self.busy || self.step != Step::Review || self.plan.is_none() {
            return;
        }
        let f = self.read_fields(cx);
        self.writes = [
            Some(Write::Apply),
            self.registry_change(&f).map(|_| Write::Registry),
        ]
        .into_iter()
        .flatten()
        .chain(self.north_star_decision.then_some(Write::Decision))
        .chain(self.draft.last.is_some().then_some(Write::Outcome))
        .collect();
        self.notes.clear();
        self.message = None;
        self.applied_id = None;
        self.busy = true;
        cx.notify();
        self.next_write(f, cx);
    }

    /// The name and the paths that `registry.update` must write after the apply.
    fn registry_change(&self, f: &Fields) -> Option<ProductChange> {
        let paths: Vec<String> = f.paths.clone();
        let (name, paths) = match &self.baseline {
            // `register` of `roles.apply` holds the first path. More paths need an update.
            None => (None, (paths.len() > 1).then_some(paths)),
            Some(b) => (
                (b.name != f.name).then(|| f.name.clone()),
                (b.paths != paths).then_some(paths),
            ),
        };
        (name.is_some() || paths.is_some()).then(|| ProductChange {
            id: f.id.clone(),
            name,
            paths,
        })
    }

    fn next_write(&mut self, f: Fields, cx: &mut Context<Self>) {
        let Some(write) = self.writes.pop_front() else {
            return self.finish(f.id, cx);
        };
        match write {
            Write::Apply => {
                let params = self.apply_params(&f, true);
                self.ask::<RolesApply>(params, cx, move |this, r, cx| this.apply_done(r, f, cx));
            }
            Write::Registry => {
                let params = RegistryUpdateParams {
                    owner: None,
                    product: self.registry_change(&f),
                };
                self.ask::<RegistryUpdate>(params, cx, move |this, r, cx| {
                    this.registry_done(r.map(|_| ()), f, cx)
                });
            }
            Write::Outcome => {
                let Some(d) = &self.draft.last else {
                    return self.next_write(f, cx);
                };
                let params = SetupOutcomeParams {
                    draft: d.id.clone(),
                    product: f.id.clone(),
                    fields: d.fields.outcomes(&f),
                };
                self.ask::<SetupOutcome>(params, cx, move |this, r, cx| {
                    this.outcome_done(r.map(|_| ()), f, cx)
                });
            }
            Write::Decision => {
                let params = DecisionAddParams {
                    product: f.id.clone(),
                    text: NORTH_STAR_TEXT.into(),
                    why: NORTH_STAR_WHY.into(),
                    action: None,
                };
                self.ask::<DecisionAdd>(params, cx, move |this, r, cx| {
                    this.decision_done(r.map(|_| ()), f, cx)
                });
            }
        }
    }

    pub fn outcome_done(&mut self, r: Result<(), String>, f: Fields, cx: &mut Context<Self>) {
        if let Err(e) = r {
            self.notes
                .push(format!("The result of the draft was not logged: {e}"));
        }
        self.next_write(f, cx)
    }

    pub fn registry_done(&mut self, r: Result<(), String>, f: Fields, cx: &mut Context<Self>) {
        if let Err(e) = r {
            self.notes
                .push(format!("The name and the paths were not saved: {e}"));
        }
        self.next_write(f, cx)
    }

    pub fn decision_done(&mut self, r: Result<(), String>, f: Fields, cx: &mut Context<Self>) {
        if let Err(e) = r {
            self.notes.push(format!("The decision was not saved: {e}"));
        }
        self.next_write(f, cx)
    }

    pub fn apply_done(
        &mut self,
        r: Result<RolesApplyResult, String>,
        f: Fields,
        cx: &mut Context<Self>,
    ) {
        match r {
            Ok(r) if r.applied => {
                if let Some(c) = r.commit.and_then(|c| c.error) {
                    self.notes.push(format!(
                        "The commit was skipped: {}",
                        c.lines().next().unwrap_or_default()
                    ));
                }
                self.next_write(f, cx)
            }
            Ok(_) => {
                self.writes.clear();
                self.fail(
                    "The data changed since the plan. Nothing was written.".into(),
                    cx,
                )
            }
            Err(e) => {
                self.writes.clear();
                self.fail(e, cx)
            }
        }
    }

    fn finish(&mut self, id: String, cx: &mut Context<Self>) {
        self.busy = false;
        self.step = Step::Done;
        self.applied_id = Some(id.clone());
        self.message = Some(Ok(format!("{id} is set up.")));
        cx.notify();
        Nav::open(cx, &self.core, ScreenSpec::front_desk(&id));
    }

    fn back(&mut self, cx: &mut Context<Self>) {
        self.step = match self.step {
            Step::Review => Step::Forms,
            Step::Forms => Step::Folder,
            s => s,
        };
        self.plan = None;
        self.message = None;
        cx.notify();
    }

    fn browse(&mut self, cx: &mut Context<Self>) {
        let rx = cx.prompt_for_paths(PathPromptOptions {
            files: false,
            directories: true,
            multiple: false,
            prompt: Some("Choose the product folder".into()),
        });
        cx.spawn(async move |this, cx| {
            if let Ok(Ok(Some(paths))) = rx.await
                && let Some(p) = paths.first()
            {
                let p = p.to_string_lossy().to_string();
                let _ = this.update(cx, |this, cx| {
                    this.puts.push(("path".into(), p));
                    cx.notify();
                });
            }
        })
        .detach();
    }
}

// ---- drawing ----

fn label(text: &str, changed: bool, no_data: bool) -> Div {
    div()
        .flex()
        .items_center()
        .gap(space(2))
        .child(
            div()
                .text_size(size::xs())
                .font_weight(weight::MEDIUM)
                .text_color(text::tertiary())
                .child(text.to_string()),
        )
        .when(changed, |el| el.child(status_tag("changed", Status::Info)))
        .when(no_data, |el| el.child(status_tag("no data", Status::Warn)))
}

fn card() -> Div {
    div()
        .flex()
        .flex_col()
        .gap(space(3))
        .p(space(4))
        .rounded(radius::lg())
        .border_1()
        .border_color(line::border())
        .bg(surface::raised())
}

fn hint(t: impl Into<SharedString>) -> Div {
    div()
        .text_size(size::sm())
        .text_color(text::tertiary())
        .child(t.into())
}

impl SetupScreen {
    fn text_field(
        &mut self,
        key: &str,
        title: &str,
        placeholder: &'static str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Div {
        let input = self.input(key, placeholder, window, cx);
        div()
            .flex()
            .flex_col()
            .gap(space(1))
            .child(label(title, self.changed(key, cx), self.no_data(key, cx)))
            .child(Input::new(&input))
    }

    fn area_field(
        &mut self,
        key: &str,
        title: &str,
        placeholder: &'static str,
        height: u16,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Div {
        let area = self.area(key, placeholder, window, cx);
        div()
            .flex()
            .flex_col()
            .gap(space(1))
            .child(label(title, self.changed(key, cx), self.no_data(key, cx)))
            .child(Textarea::new(&area).h(space(height)))
    }

    fn problems_of(&self, form: Form) -> Div {
        div().flex().flex_col().gap(space(1)).children(
            self.problems.iter().filter(|p| p.form == form).map(|p| {
                div()
                    .text_size(size::sm())
                    .text_color(status::danger())
                    .child(p.text.clone())
            }),
        )
    }

    fn folder_step(&mut self, window: &mut Window, cx: &mut Context<Self>) -> Div {
        let local = self.core.read(cx).remote.is_none();
        let path = self.input("path", "/home/you/dev/my-product", window, cx);
        let busy = self.busy;
        card()
            .child(section("The folder of the product", None))
            .child(hint(if local {
                "Type the absolute path, or browse. The app reads the folder and writes nothing."
            } else {
                "Type the absolute path on the host. The app reads the folder and writes nothing."
            }))
            .child(Input::new(&path))
            .child(
                div()
                    .flex()
                    .gap(space(2))
                    .child(
                        Button::new("setup-scan")
                            .primary()
                            .small()
                            .label(if busy {
                                "Reading…"
                            } else {
                                "Read the folder"
                            })
                            .disabled(busy)
                            .on_click(cx.listener(|this, _, _, cx| this.scan_now(cx))),
                    )
                    .when(local, |el| {
                        el.child(
                            Button::new("setup-browse")
                                .secondary()
                                .small()
                                .label("Browse…")
                                .on_click(cx.listener(|this, _, _, cx| this.browse(cx))),
                        )
                    }),
            )
    }

    fn scan_card(&self) -> Option<Div> {
        let s = self.scan.as_ref()?;
        let mut lines = vec![format!("{} · {}", s.name, s.path)];
        lines.push(match &s.manifest {
            Some(m) => format!("Manifest: {}", m.file),
            None => "No manifest found.".into(),
        });
        lines.push(if s.north_stars.is_empty() {
            "No north-star doc found.".into()
        } else {
            format!("North-star docs: {}", s.north_stars.join(", "))
        });
        lines.push(format!(
            "bwrap: {} · codebase-memory-mcp: {}",
            if s.tools.bwrap { "found" } else { "missing" },
            if s.tools.codebase_memory_mcp {
                "found"
            } else {
                "missing"
            }
        ));
        Some(
            card()
                .child(section(
                    if self.baseline.is_some() {
                        "Update mode: the folder is registered"
                    } else {
                        "A new product"
                    },
                    None,
                ))
                .children(lines.into_iter().map(mono)),
        )
    }

    fn product_form(&mut self, window: &mut Window, cx: &mut Context<Self>) -> Div {
        let stage = self.stage.clone();
        let stage_changed = self.baseline.as_ref().is_some_and(|b| b.stage != stage);
        let fixed_id = self.baseline.is_some();
        let id = self.input("id", "my-product", window, cx);
        card()
            .child(section("Product", None))
            .child(
                div()
                    .flex()
                    .flex_col()
                    .gap(space(1))
                    .child(label("Id", false, false))
                    .child(Input::new(&id).disabled(fixed_id)),
            )
            .child(self.text_field("name", "Name", "My product", window, cx))
            .child(self.area_field(
                "paths",
                "Paths, one for each line",
                "/home/you/dev/my-product",
                14,
                window,
                cx,
            ))
            .child(self.text_field(
                "what",
                "What it is, in one sentence",
                "A mail filter for small teams",
                window,
                cx,
            ))
            .child(self.text_field("audience", "Audience", "Who uses it", window, cx))
            .child(
                div()
                    .flex()
                    .flex_col()
                    .gap(space(1))
                    .child(label("Stage", stage_changed, false))
                    .child(
                        div()
                            .flex()
                            .gap(space(2))
                            .children(STAGES.iter().enumerate().map(|(i, s)| {
                                let b = Button::new(("setup-stage", i)).small().label(*s).on_click(
                                    cx.listener(move |this, _, _, cx| {
                                        this.stage = s.to_string();
                                        cx.notify();
                                    }),
                                );
                                if *s == stage {
                                    b.primary()
                                } else {
                                    b.secondary()
                                }
                            })),
                    ),
            )
            .child(self.area_field(
                "out",
                "Out of scope, one for each line",
                "What the agency must never propose",
                20,
                window,
                cx,
            ))
            .child(self.problems_of(Form::Product))
    }

    fn objectives_form(&mut self, window: &mut Window, cx: &mut Context<Self>) -> Div {
        let mut rows = Vec::new();
        for i in 0..3 {
            rows.push(
                div()
                    .flex()
                    .gap(space(2))
                    .child(
                        self.text_field(
                            &format!("o{i}.text"),
                            &format!("Objective {}", i + 1),
                            "What to reach",
                            window,
                            cx,
                        )
                        .flex_1(),
                    )
                    .child(
                        self.text_field(
                            &format!("o{i}.signal"),
                            "Signal",
                            "How you will know",
                            window,
                            cx,
                        )
                        .flex_1(),
                    )
                    .child(
                        self.text_field(&format!("o{i}.due"), "Due", "YYYY-MM-DD", window, cx)
                            .w(space(32)),
                    ),
            );
        }
        card()
            .child(section("Objectives", None))
            .child(hint("One to three. Leave a row empty to skip it."))
            .children(rows)
            .child(self.area_field(
                "north",
                "North-star docs, one path for each line",
                "docs/product.md",
                16,
                window,
                cx,
            ))
            .child(self.area_field(
                "sources",
                "Sources of truth, one for each line: kind | ref | note",
                "issues | github.com/me/app | Open bugs",
                16,
                window,
                cx,
            ))
            .child(self.problems_of(Form::Objectives))
    }

    fn roster_form(&mut self, window: &mut Window, cx: &mut Context<Self>) -> Div {
        let globals = self.globals.clone();
        let roster = self.read_fields(cx).roster();
        let mut extras = Vec::new();
        for (row, n) in self.extras.clone().into_iter().enumerate() {
            let remove = Button::new(("setup-x-remove", row))
                .ghost()
                .small()
                .label("Remove")
                .on_click(cx.listener(move |this, _, _, cx| {
                    this.extras.retain(|x| *x != n);
                    cx.notify();
                }));
            extras.push(
                div()
                    .flex()
                    .flex_col()
                    .gap(space(2))
                    .child(self.text_field(
                        &format!("x{n}.id"),
                        "Agent id",
                        "dev.backend, or seo for a custom role",
                        window,
                        cx,
                    ))
                    .child(self.area_field(
                        &format!("x{n}.body"),
                        "Overlay text of a specialist, or the full definition of a custom role",
                        "You cover the API and the migrations.",
                        24,
                        window,
                        cx,
                    ))
                    .child(div().flex().justify_end().child(remove)),
            );
        }
        let mut whys = Vec::new();
        for id in &roster {
            whys.push(self.text_field(
                &format!("why.{id}"),
                &format!("Why {id} is in the roster"),
                "One line",
                window,
                cx,
            ));
        }
        card()
            .child(section("Roster", Some(roster.len())))
            .child(
                div()
                    .flex()
                    .gap(space(4))
                    .children(GLOBAL_ROLES.iter().map(|r| {
                        let on = *r == "pm" || globals.iter().any(|g| g == r);
                        Checkbox::new((
                            "setup-global",
                            GLOBAL_ROLES.iter().position(|x| x == r).unwrap_or(0),
                        ))
                        .label(*r)
                        .checked(on)
                        .on_click(cx.listener(
                            move |this, on: &bool, _, cx| {
                                this.globals.retain(|g| g != r);
                                if *on && *r != "pm" {
                                    this.globals.push(r.to_string());
                                }
                                cx.notify();
                            },
                        ))
                    })),
            )
            .children(extras)
            .child(
                div().child(
                    Button::new("setup-x-add")
                        .secondary()
                        .small()
                        .label("Add a specialist or a custom role")
                        .on_click(cx.listener(|this, _, _, cx| {
                            this.extras.push(this.next_extra);
                            this.next_extra += 1;
                            cx.notify();
                        })),
                ),
            )
            .children(whys)
            .child(self.problems_of(Form::Roster))
    }

    fn models_form(&mut self, window: &mut Window, cx: &mut Context<Self>) -> Div {
        let roster = self.read_fields(cx).roster();
        let mut rows = Vec::new();
        for id in &roster {
            rows.push(
                div()
                    .flex()
                    .items_end()
                    .gap(space(2))
                    .child(div().w(space(32)).child(mono(id.clone())))
                    .child(
                        self.text_field(
                            &format!("model.{id}"),
                            "Model",
                            "the global default",
                            window,
                            cx,
                        )
                        .flex_1(),
                    )
                    .child(
                        self.text_field(
                            &format!("effort.{id}"),
                            "Effort",
                            "the global default",
                            window,
                            cx,
                        )
                        .flex_1(),
                    ),
            );
        }
        card()
            .child(section("Models", None))
            .child(hint(
                "Leave a field empty to keep the model and the effort of the global role.",
            ))
            .children(rows)
            .child(self.problems_of(Form::Models))
    }

    fn stories_form(&mut self, window: &mut Window, cx: &mut Context<Self>) -> Div {
        let bwrap = self.scan.as_ref().is_none_or(|s| s.tools.bwrap);
        let on = self.stories_on;
        card()
            .child(section("Stories mode", None))
            .child(
                Checkbox::new("setup-stories-on")
                    .label("Stories mode on")
                    .checked(on)
                    .on_click(cx.listener(|this, on: &bool, _, cx| {
                        this.stories_on = *on;
                        cx.notify();
                    })),
            )
            .when(!bwrap, |el| {
                el.child(hint(
                    "bwrap is not on the PATH of this host. Stories mode runs the tests in a sandbox, so we recommend off.",
                ))
            })
            .child(self.text_field("test", "Test command, with {files}", "pnpm test {files}", window, cx))
            .child(self.area_field("gate", "Gate commands, one for each line", "pnpm typecheck", 16, window, cx))
            .child(self.text_field("repo", "Repo, only when the code is not at the first path", "", window, cx))
            .child(self.text_field("budget", "Budget for each story session, in USD", "5", window, cx))
            .child(self.problems_of(Form::Stories))
    }

    fn draft_card(&mut self, cx: &mut Context<Self>) -> Div {
        let (armed, running, repair_armed) = (
            self.draft.armed,
            self.draft.running,
            self.draft.repair_armed,
        );
        let can_repair =
            !self.draft.repaired && self.draft.last.is_some() && !self.draft.failures.is_empty();
        let result = self.draft.last.as_ref().map(|d| {
            let mut t = format!("Drafted for ${:.2} in {:.0} s.", d.cost_usd, d.seconds);
            if !d.no_data.is_empty() {
                t.push_str(&format!(
                    " The files do not answer: {}.",
                    d.no_data.join(", ")
                ));
            }
            t
        });
        card()
            .child(section("Draft with Claude (optional)", None))
            .child(hint(format!(
                "One call to Claude reads up to 15 files of the folder and fills the forms. It costs up to ${DRAFT_CAP} and the folder stays unchanged. You edit each field, and nothing is written before the Review."
            )))
            .child(
                div()
                    .flex()
                    .gap(space(2))
                    .child(
                        Button::new("setup-draft")
                            .secondary()
                            .small()
                            .label(if running {
                                "Drafting…".to_string()
                            } else if armed {
                                format!("Confirm: spend up to ${DRAFT_CAP}")
                            } else {
                                "Draft with Claude".to_string()
                            })
                            .disabled(running)
                            .on_click(cx.listener(|this, _, _, cx| this.draft_click(cx))),
                    )
                    .when(armed || repair_armed, |el| {
                        el.child(
                            Button::new("setup-draft-cancel")
                                .ghost()
                                .small()
                                .label("Cancel")
                                .on_click(cx.listener(|this, _, _, cx| this.cancel_draft_confirm(cx))),
                        )
                    }),
            )
            .children(result.map(mono))
            .children(self.draft.error.clone().map(|e| {
                div()
                    .text_size(size::sm())
                    .text_color(status::danger())
                    .child(format!("The draft failed: {e}"))
            }))
            .when(can_repair, |el| {
                el.child(hint(
                    "The core refused the stage. Claude can repair the draft one time, from the reasons.",
                ))
                .child(
                    div().child(
                        Button::new("setup-repair")
                            .secondary()
                            .small()
                            .label(if repair_armed {
                                format!("Confirm: spend up to ${DRAFT_CAP}")
                            } else {
                                "Repair with Claude".to_string()
                            })
                            .disabled(running)
                            .on_click(cx.listener(|this, _, _, cx| this.repair_click(cx))),
                    ),
                )
            })
    }

    fn forms_step(&mut self, window: &mut Window, cx: &mut Context<Self>) -> Div {
        let other = self.problems_of(Form::Other);
        let busy = self.busy;
        div()
            .flex()
            .flex_col()
            .gap(space(6))
            .children(self.scan_card())
            .child(self.draft_card(cx))
            .child(self.product_form(window, cx))
            .child(self.objectives_form(window, cx))
            .child(self.roster_form(window, cx))
            .child(self.models_form(window, cx))
            .child(self.stories_form(window, cx))
            .child(other)
            .child(
                div()
                    .flex()
                    .gap(space(2))
                    .child(
                        Button::new("setup-review")
                            .primary()
                            .small()
                            .label(if busy { "Checking…" } else { "Review" })
                            .disabled(busy)
                            .on_click(cx.listener(|this, _, _, cx| this.review_now(cx))),
                    )
                    .child(
                        Button::new("setup-back")
                            .secondary()
                            .small()
                            .label("Back")
                            .on_click(cx.listener(|this, _, _, cx| this.back(cx))),
                    ),
            )
    }

    fn review_step(&mut self, cx: &mut Context<Self>) -> Div {
        let lines = self.plan.as_ref().map(apply_lines).unwrap_or_default();
        let busy = self.busy;
        let no_north_star = self.read_fields(cx).north_stars.is_empty();
        let decision = self.north_star_decision;
        card()
            .child(section("Review", None))
            .child(hint("Nothing is written yet. Confirm applies this plan."))
            .child(
                div()
                    .flex()
                    .flex_col()
                    .gap(space(1))
                    .children(lines.into_iter().map(mono)),
            )
            .when(no_north_star, |el| {
                el.child(
                    Checkbox::new("setup-north-star")
                        .label(format!("Also record the decision: {NORTH_STAR_TEXT}"))
                        .checked(decision)
                        .on_click(cx.listener(|this, on: &bool, _, cx| {
                            this.north_star_decision = *on;
                            cx.notify();
                        })),
                )
            })
            .child(
                div()
                    .flex()
                    .gap(space(2))
                    .child(
                        Button::new("setup-confirm")
                            .primary()
                            .small()
                            .label(if busy { "Applying…" } else { "Confirm" })
                            .disabled(busy)
                            .on_click(cx.listener(|this, _, _, cx| this.confirm(cx))),
                    )
                    .child(
                        Button::new("setup-review-back")
                            .secondary()
                            .small()
                            .label("Back to the forms")
                            .on_click(cx.listener(|this, _, _, cx| this.back(cx))),
                    ),
            )
    }

    fn done_step(&mut self, cx: &mut Context<Self>) -> Div {
        let id = self.applied_id.clone().unwrap_or_default();
        card()
            .child(section("Done", None))
            .children(self.notes.iter().cloned().map(mono))
            .child(hint("The front desk of the product is open. Run `agentsi roles validate` to check the roster."))
            .child(
                div().child(
                    Button::new("setup-open-desk")
                        .primary()
                        .small()
                        .label("Open the front desk")
                        .on_click(cx.listener(move |this, _, _, cx| {
                            Nav::open(cx, &this.core, ScreenSpec::front_desk(&id))
                        })),
                ),
            )
    }

    fn steps_line(&self) -> Div {
        let names = [
            (Step::Folder, "1 Folder"),
            (Step::Forms, "2 Forms"),
            (Step::Review, "3 Review"),
            (Step::Done, "4 Done"),
        ];
        div()
            .flex()
            .gap(space(2))
            .children(names.into_iter().map(|(s, n)| {
                if s == self.step {
                    status_tag(n, Status::Info)
                } else {
                    neutral_tag(n)
                }
            }))
    }
}

impl Render for SetupScreen {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        self.apply_fill(window, cx);
        let title = if self.baseline.is_some() || !self.product.is_empty() {
            format!("Update the brief of {}", self.product)
        } else {
            "Set up a product".to_string()
        };
        let body = match self.step {
            Step::Folder => self.folder_step(window, cx),
            Step::Forms => self.forms_step(window, cx),
            Step::Review => self.review_step(cx),
            Step::Done => self.done_step(cx),
        };
        let message = self.message.as_ref().map(|m| match m {
            Ok(t) => super::parts::flash_line(&super::parts::Flash::Done(t.clone())),
            Err(e) => super::parts::flash_line(&super::parts::Flash::Failed(e.clone())),
        });
        div()
            .id("setup-screen")
            .size_full()
            .bg(surface::panel())
            .track_scroll(&self.scroll)
            .overflow_y_scroll()
            .child(
                column()
                    .child(
                        div()
                            .flex()
                            .flex_col()
                            .gap(space(2))
                            .child(
                                div()
                                    .text_size(size::xl())
                                    .font_weight(weight::SEMIBOLD)
                                    .text_color(text::primary())
                                    .child(title),
                            )
                            .child(self.steps_line()),
                    )
                    .children(message)
                    .child(body),
            )
    }
}

#[cfg(test)]
mod tests {
    // No `use super::*`: it brings in the `test` module of `gpui_kit`, which hides `#[test]`.
    use super::{Existing, SetupScreen, Step};
    use crate::link::Link;
    use crate::model::CoreState;
    use gpui::TestAppContext;
    use gpui_kit::AppContext as _;
    use serde_json::{Value, json};
    use std::os::unix::net::UnixStream;
    use std::sync::Arc;

    fn scan(registered: Option<&str>) -> agentsi_proto::setup_api::SetupScanResult {
        serde_json::from_value(json!({
            "path": "/work/demo", "id": "demo", "name": "demo", "registered": registered,
            "manifest": {"file": "package.json", "test": "node --test {files}", "gate": ["pnpm lint"]},
            "north_stars": [],
            "tools": {"bwrap": true, "codebase_memory_mcp": false}
        }))
        .unwrap()
    }

    fn plan() -> agentsi_proto::roles_api::RolesApplyResult {
        serde_json::from_value(json!({
            "plan": {
                "product": "demo", "register": true, "roster_before": [],
                "roster_after": ["pm", "dev", "qa"],
                "changes": [{"path": "brief.json", "status": "new", "diff": ""}],
                "stories_before": null, "stories_after": null, "stories_changed": false
            },
            "failures": [], "applied": false, "commit": null, "warnings": []
        }))
        .unwrap()
    }

    fn applied() -> agentsi_proto::roles_api::RolesApplyResult {
        let mut r = plan();
        r.applied = true;
        r.plan = None;
        r
    }

    fn requests(other: &mut UnixStream) -> Vec<Value> {
        other.set_nonblocking(true).unwrap();
        let mut out = Vec::new();
        while let Ok(Some(f)) = agentsi_proto::frame::read_frame(other) {
            out.push(serde_json::from_slice(&f.payload).unwrap());
        }
        out
    }

    fn methods(other: &mut UnixStream) -> Vec<(String, Value)> {
        requests(other)
            .into_iter()
            .map(|r| {
                (
                    r["method"].as_str().unwrap().to_string(),
                    r["params"].clone(),
                )
            })
            .collect()
    }

    #[gpui::test]
    fn a_new_product_goes_through_the_steps_and_nothing_is_written_before_the_confirm(
        cx: &mut TestAppContext,
    ) {
        cx.update(agentsi_ui::init);
        let (end, mut other) = UnixStream::pair().unwrap();
        let core = cx.new(|_| CoreState::new(Arc::new(Link::for_tests(end))));
        let (screen, cx) = cx.add_window_view(|_, cx| SetupScreen::new(String::new(), core, cx));
        let draw = |cx: &mut gpui::VisualTestContext| cx.update(|w, cx| w.draw(cx).clear(cx));
        draw(cx);
        assert_eq!(screen.read_with(cx, |s, _| s.step()), Step::Folder);

        // 1. Folder: the scan reads. It writes nothing.
        screen.update_in(cx, |s, w, cx| {
            s.put("path", "/work/demo", w, cx);
            s.scan_now(cx);
        });
        let sent = methods(&mut other);
        assert_eq!(
            sent.iter().map(|(m, _)| m.as_str()).collect::<Vec<_>>(),
            ["setup.scan"]
        );
        assert_eq!(sent[0].1, json!({"path": "/work/demo"}));
        screen.update(cx, |s, cx| s.scanned(Ok(scan(None)), cx));
        draw(cx);
        assert_eq!(screen.read_with(cx, |s, _| s.step()), Step::Forms);

        // 2. Forms: the scan filled the id, the paths and the Stories commands.
        let f = screen.read_with(cx, |s, cx| s.read_fields(cx));
        assert_eq!(
            (f.id.as_str(), f.paths.clone()),
            ("demo", vec!["/work/demo".to_string()])
        );
        assert_eq!(f.stories.test, "node --test {files}");
        assert!(!f.stories.on);

        // The review with an empty form is refused, and it sends nothing.
        screen.update(cx, |s, cx| s.review_now(cx));
        assert!(methods(&mut other).is_empty());

        screen.update_in(cx, |s, w, cx| {
            s.put("what", "A demo", w, cx);
            s.put("paths", "/work/demo\n/work/demo-docs", w, cx);
            s.put("o0.text", "Ship it", w, cx);
            s.put("o0.due", "2026-12-31", w, cx);
            s.put("north", "", w, cx);
        });
        screen.update(cx, |s, cx| s.review_now(cx));
        let sent = methods(&mut other);
        assert_eq!(sent.len(), 1);
        assert_eq!(sent[0].0, "roles.apply");
        // 3. Review: the call plans. It has the inline stage and no `yes`.
        let p = &sent[0].1;
        assert_eq!(p["yes"], false);
        assert!(p.get("stage_dir").is_none());
        assert_eq!(p["register"], json!({"name": "demo", "path": "/work/demo"}));
        let stage = p["stage"].as_object().unwrap();
        assert_eq!(
            stage.keys().map(String::as_str).collect::<Vec<_>>(),
            ["brief.json", "roster.json"]
        );
        screen.update(cx, |s, cx| s.planned(Ok(plan()), cx));
        draw(cx);
        assert_eq!(screen.read_with(cx, |s, _| s.step()), Step::Review);
        assert!(
            methods(&mut other).is_empty(),
            "no write before the Confirm"
        );

        // 4. Confirm: the apply, the extra path, then the north-star decision.
        screen.update(cx, |s, cx| {
            s.north_star_decision = true;
            s.confirm(cx)
        });
        let sent = methods(&mut other);
        assert_eq!(sent.len(), 1);
        assert_eq!(
            (sent[0].0.as_str(), &sent[0].1["yes"]),
            ("roles.apply", &json!(true))
        );
        let f = screen.read_with(cx, |s, cx| s.read_fields(cx));
        screen.update(cx, |s, cx| s.apply_done(Ok(applied()), f.clone(), cx));
        let sent = methods(&mut other);
        assert_eq!(sent[0].0, "registry.update");
        assert_eq!(
            sent[0].1,
            json!({"product": {"id": "demo", "paths": ["/work/demo", "/work/demo-docs"]}})
        );
        screen.update(cx, |s, cx| s.registry_done(Ok(()), f.clone(), cx));
        let sent = methods(&mut other);
        assert_eq!(sent[0].0, "decision.add");
        assert_eq!(sent[0].1["product"], "demo");
        screen.update(cx, |s, cx| s.decision_done(Ok(()), f, cx));
        assert_eq!(screen.read_with(cx, |s, _| s.step()), Step::Done);
        draw(cx);
    }

    #[gpui::test]
    fn the_core_failures_show_at_their_form_and_the_screen_stays_in_the_forms(
        cx: &mut TestAppContext,
    ) {
        cx.update(agentsi_ui::init);
        let (end, mut other) = UnixStream::pair().unwrap();
        let core = cx.new(|_| CoreState::new(Arc::new(Link::for_tests(end))));
        let (screen, cx) = cx.add_window_view(|_, cx| SetupScreen::new(String::new(), core, cx));
        screen.update(cx, |s, cx| s.scanned(Ok(scan(None)), cx));
        cx.update(|w, cx| w.draw(cx).clear(cx));
        screen.update_in(cx, |s, w, cx| {
            s.put("what", "A demo", w, cx);
            s.put("o0.text", "Ship it", w, cx);
            s.put("o0.due", "2026-12-31", w, cx);
        });
        screen.update(cx, |s, cx| s.review_now(cx));
        assert_eq!(methods(&mut other).len(), 1);
        let mut refused = plan();
        refused.plan = None;
        refused.failures =
            vec!["Brief: north_stars path \"x\" does not exist under a product path.".into()];
        screen.update(cx, |s, cx| s.planned(Ok(refused), cx));
        cx.update(|w, cx| w.draw(cx).clear(cx));
        assert_eq!(screen.read_with(cx, |s, _| s.step()), Step::Forms);
        screen.read_with(cx, |s, _| {
            assert_eq!(s.problems.len(), 1);
            assert_eq!(s.problems[0].form, super::Form::Objectives);
        });
    }

    #[gpui::test]
    fn a_registered_folder_opens_the_update_mode_and_fills_the_forms_from_the_stored_brief(
        cx: &mut TestAppContext,
    ) {
        cx.update(agentsi_ui::init);
        let (end, _other) = UnixStream::pair().unwrap();
        let core = cx.new(|_| CoreState::new(Arc::new(Link::for_tests(end))));
        let (screen, cx) = cx.add_window_view(|_, cx| SetupScreen::new(String::new(), core, cx));
        let existing = Existing {
            brief: Some(
                serde_json::from_value(json!({
                    "what": "A mail filter", "audience": "Teams", "stage": "launched",
                    "objectives": [{"id": "o1", "text": "Ship", "signal": "10 users", "due": "2026-12-31"}],
                    "north_stars": [], "out_of_scope": ["Billing"],
                    "sources_of_truth": [{"kind": "docs", "ref": "docs/", "note": "the docs"}],
                    "roles": {"pm": "Ranks.", "dev": "Judges."}, "updated_at": null
                }))
                .unwrap(),
            ),
            list: serde_json::from_value(json!({
                "product": "demo",
                "roster": [
                    {"id": "pm", "role": "pm", "source": "global", "model": null, "effort": null, "error": null},
                    {"id": "dev", "role": "dev", "source": "global", "model": null, "effort": null, "error": null}],
                "overlays": [], "globals": ["pm", "dev"], "warnings": []
            }))
            .unwrap(),
            stories: None,
            registry: serde_json::from_value(json!({
                "owner": {"name": "Ana", "language": "English"},
                "products": [{"id": "demo", "name": "Demo", "paths": ["/work/demo"], "status": "active"}]
            }))
            .unwrap(),
            defs: vec![],
        };
        screen.update(cx, |s, cx| {
            s.existing_loaded(scan(Some("demo")), Ok(existing), cx)
        });
        cx.update(|w, cx| w.draw(cx).clear(cx));
        let f = screen.read_with(cx, |s, cx| s.read_fields(cx));
        assert_eq!(
            (
                f.id.as_str(),
                f.name.as_str(),
                f.what.as_str(),
                f.stage.as_str()
            ),
            ("demo", "Demo", "A mail filter", "launched")
        );
        assert_eq!(f.out_of_scope, ["Billing"]);
        assert_eq!(f.sources.len(), 1, "the stored sources fill the box");
        let stage = f.stage(
            screen.read_with(cx, |s, _| s.baseline.clone()).as_ref(),
            "2026-11-09",
        );
        let brief: serde_json::Value = serde_json::from_str(&stage["brief.json"]).unwrap();
        assert_eq!(
            brief["sources_of_truth"],
            json!([{"kind": "docs", "ref": "docs/", "note": "the docs"}]),
            "an untouched box keeps the sources"
        );
        assert_eq!(f.roster(), ["pm", "dev"]);
        assert_eq!(f.why["dev"], "Judges.");
        // An unchanged form keeps the Stories block: no `stories.json` in the stage.
        let base = screen.read_with(cx, |s, _| s.baseline.clone());
        assert!(base.is_some());
        assert!(
            !f.stage(base.as_ref(), "2026-11-09")
                .contains_key("stories.json")
        );
    }

    fn drafted_result() -> agentsi_proto::setup_api::SetupDraftResult {
        serde_json::from_value(json!({
            "draft": "d1", "cost_usd": 0.12, "seconds": 14.0,
            "no_data": ["product.audience"],
            "fields": {
                "product": {"id": "other", "name": "Other", "what": "A mail filter", "audience": "",
                            "stage": "launched", "out_of_scope": ["Billing"]},
                "objectives": {
                    "objectives": [{"text": "Ship", "signal": "10 users", "due": "2026-12-31"}],
                    "north_stars": ["docs/product.md"], "sources_of_truth": []},
                "roster": {"roles": ["pm", "dev"], "specialists": [], "why": []},
                "models": [],
                "stories": {"on": false, "test": "", "gate": []}
            }
        }))
        .unwrap()
    }

    #[gpui::test]
    fn the_draft_asks_one_confirm_marks_no_data_and_logs_what_the_owner_kept_or_edited(
        cx: &mut TestAppContext,
    ) {
        cx.update(agentsi_ui::init);
        let (end, mut other) = UnixStream::pair().unwrap();
        let core = cx.new(|_| CoreState::new(Arc::new(Link::for_tests(end))));
        let (screen, cx) = cx.add_window_view(|_, cx| SetupScreen::new(String::new(), core, cx));
        let draw = |cx: &mut gpui::VisualTestContext| cx.update(|w, cx| w.draw(cx).clear(cx));
        screen.update(cx, |s, cx| s.scanned(Ok(scan(None)), cx));
        draw(cx);
        let _ = methods(&mut other);

        // The first click shows the cap and sends nothing. The second click runs the call.
        screen.update(cx, |s, cx| s.draft_click(cx));
        assert!(methods(&mut other).is_empty(), "no call before the Confirm");
        screen.update(cx, |s, cx| s.draft_click(cx));
        let sent = methods(&mut other);
        assert_eq!(sent.len(), 1);
        assert_eq!(
            (sent[0].0.as_str(), &sent[0].1),
            ("setup.draft", &json!({"path": "/work/demo"}))
        );
        // A click while it runs does nothing.
        screen.update(cx, |s, cx| {
            s.draft_click(cx);
            s.draft_click(cx)
        });
        assert!(methods(&mut other).is_empty());

        screen.update(cx, |s, cx| s.drafted(Ok(drafted_result()), cx));
        draw(cx);
        let f = screen.read_with(cx, |s, cx| s.read_fields(cx));
        assert_eq!(
            (f.id.as_str(), f.what.as_str(), f.stage.as_str()),
            ("demo", "A mail filter", "launched")
        );
        assert_eq!(f.paths, ["/work/demo"], "the folder decides the paths");
        assert_eq!(f.why["pm"], "Ranks the work and writes the synthesis.");
        assert!(screen.read_with(cx, |s, cx| s.no_data("audience", cx)));

        // The owner edits `what`, fills the audience and clears the out of scope box.
        screen.update_in(cx, |s, w, cx| {
            s.put("what", "A spam filter", w, cx);
            s.put("audience", "Teams", w, cx);
            s.put("out", "", w, cx);
        });
        assert!(!screen.read_with(cx, |s, cx| s.no_data("audience", cx)));
        screen.update(cx, |s, cx| s.review_now(cx));
        let sent = methods(&mut other);
        assert_eq!(sent[0].0, "roles.apply");
        screen.update(cx, |s, cx| s.planned(Ok(plan()), cx));
        screen.update(cx, |s, cx| s.confirm(cx));
        let _ = methods(&mut other);
        let f = screen.read_with(cx, |s, cx| s.read_fields(cx));
        screen.update(cx, |s, cx| s.apply_done(Ok(applied()), f.clone(), cx));
        let sent = methods(&mut other);
        assert_eq!(sent[0].0, "setup.outcome");
        assert_eq!(sent[0].1["draft"], "d1");
        assert_eq!(sent[0].1["product"], "demo");
        let fields = &sent[0].1["fields"];
        assert_eq!(fields["product.what"], "edited");
        assert_eq!(fields["product.out_of_scope"], "cleared");
        assert_eq!(fields["product.stage"], "kept");
        assert!(fields.get("product.audience").is_none());
        screen.update(cx, |s, cx| s.outcome_done(Ok(()), f, cx));
        assert_eq!(screen.read_with(cx, |s, _| s.step()), Step::Done);
    }

    #[gpui::test]
    fn a_refused_stage_offers_one_repair_that_asks_its_own_confirm(cx: &mut TestAppContext) {
        cx.update(agentsi_ui::init);
        let (end, mut other) = UnixStream::pair().unwrap();
        let core = cx.new(|_| CoreState::new(Arc::new(Link::for_tests(end))));
        let (screen, cx) = cx.add_window_view(|_, cx| SetupScreen::new(String::new(), core, cx));
        screen.update(cx, |s, cx| s.scanned(Ok(scan(None)), cx));
        cx.update(|w, cx| w.draw(cx).clear(cx));
        screen.update(cx, |s, cx| {
            s.draft_click(cx);
            s.draft_click(cx);
            s.drafted(Ok(drafted_result()), cx)
        });
        cx.update(|w, cx| w.draw(cx).clear(cx));
        let _ = methods(&mut other);
        screen.update(cx, |s, cx| s.review_now(cx));
        let _ = methods(&mut other);
        let mut refused = plan();
        refused.plan = None;
        refused.failures = vec![
            "Brief: north_stars path \"docs/product.md\" does not exist under a product path."
                .into(),
        ];
        screen.update(cx, |s, cx| s.planned(Ok(refused), cx));
        cx.update(|w, cx| w.draw(cx).clear(cx));

        screen.update(cx, |s, cx| s.repair_click(cx));
        assert!(
            methods(&mut other).is_empty(),
            "the repair asks its own Confirm"
        );
        screen.update(cx, |s, cx| s.repair_click(cx));
        let sent = methods(&mut other);
        assert_eq!(sent.len(), 1);
        assert_eq!(sent[0].0, "setup.draft");
        assert_eq!(sent[0].1["failures"].as_array().unwrap().len(), 1);
        assert_eq!(sent[0].1["previous"]["product"]["what"], "A mail filter");
        screen.update(cx, |s, cx| s.drafted(Ok(drafted_result()), cx));
        // One repair only: a second round sends nothing.
        screen.update(cx, |s, cx| {
            s.draft.failures = vec!["x".into()];
            s.repair_click(cx);
            s.repair_click(cx)
        });
        assert!(methods(&mut other).is_empty());
    }
}
