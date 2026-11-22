//! The model select and the effort select of the desk conversation. The core keeps the desk
//! model and the desk effort of each product (`desk.settings`, `desk.configure`). A pick applies to the next owner message.
//!
//! Spec: `docs/specs/agentsi-desktop-desk-model.md`.

use agentsi_proto::desk_session_api::{DESK_EFFORTS, DESK_MODELS, DeskChoice};
use agentsi_ui::tokens::{control, space};
use gpui_kit::component::IndexPath;
use gpui_kit::component::Sizable as _;
use gpui_kit::component::select::{SearchableVec, Select, SelectEvent, SelectItem, SelectState};
use gpui_kit::*;

/// The value of the first entry of each select: the role default.
pub const ROLE_DEFAULT: &str = "";

/// One entry of a select, and one answer of a palette step: a label and a value.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Pick {
    pub label: String,
    pub value: String,
}

impl Pick {
    fn new(label: impl Into<String>, value: impl Into<String>) -> Self {
        Pick {
            label: label.into(),
            value: value.into(),
        }
    }
}

impl SelectItem for Pick {
    type Value = String;

    fn title(&self) -> SharedString {
        self.label.clone().into()
    }

    fn value(&self) -> &String {
        &self.value
    }
}

pub type Options = SearchableVec<Pick>;

/// The label of a model id: its name in the list, else the id itself (a role may name any
/// model).
pub fn model_label(id: &str) -> String {
    DESK_MODELS
        .iter()
        .find(|(m, _)| *m == id)
        .map_or_else(|| id.to_string(), |(_, label)| label.to_string())
}

/// The model entries: "Role default (<model of pm>)", then the models.
pub fn model_options(choice: Option<&DeskChoice>) -> Vec<Pick> {
    let default = match choice {
        Some(c) => format!("Role default ({})", model_label(&c.role_model)),
        None => "Role default".to_string(),
    };
    std::iter::once(Pick::new(default, ROLE_DEFAULT))
        .chain(DESK_MODELS.iter().map(|(id, label)| Pick::new(*label, *id)))
        .collect()
}

/// The effort entries: "Role default (<effort of pm>)" or "Role default (none)", then the levels.
pub fn effort_options(choice: Option<&DeskChoice>) -> Vec<Pick> {
    let default = match choice {
        Some(c) => format!(
            "Role default ({})",
            c.role_effort.as_deref().unwrap_or("none")
        ),
        None => "Role default".to_string(),
    };
    std::iter::once(Pick::new(default, ROLE_DEFAULT))
        .chain(DESK_EFFORTS.iter().map(|e| Pick::new(*e, *e)))
        .collect()
}

pub fn model_value(choice: Option<&DeskChoice>) -> String {
    choice
        .and_then(|c| c.model.clone())
        .unwrap_or_else(|| ROLE_DEFAULT.into())
}

pub fn effort_value(choice: Option<&DeskChoice>) -> String {
    choice
        .and_then(|c| c.effort.clone())
        .unwrap_or_else(|| ROLE_DEFAULT.into())
}

/// The note that shows while a reply runs after a pick.
pub fn next_message_note(choice: &DeskChoice) -> String {
    let model = choice
        .model
        .as_deref()
        .unwrap_or(choice.role_model.as_str());
    format!("The next message uses {}.", model_label(model))
}

/// The two selects. A pick sends its field, and the other field keeps its stored value.
pub struct Selects {
    model: Entity<SelectState<Options>>,
    effort: Entity<SelectState<Options>>,
    _subs: Vec<Subscription>,
}

/// Which select a pick came from.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Field {
    Model,
    Effort,
}

impl Selects {
    pub fn new<T: 'static>(
        choice: Option<&DeskChoice>,
        window: &mut Window,
        cx: &mut Context<T>,
        on_pick: fn(&mut T, Field, String, &mut Context<T>),
    ) -> Selects {
        let make = |options: Vec<Pick>, value: String, window: &mut Window, cx: &mut Context<T>| {
            let at = options
                .iter()
                .position(|o| o.value == value)
                .map(|row| IndexPath::default().row(row));
            cx.new(|cx| SelectState::new(SearchableVec::new(options), at, window, cx))
        };
        let model = make(model_options(choice), model_value(choice), window, cx);
        let effort = make(effort_options(choice), effort_value(choice), window, cx);
        let model_sub = cx.subscribe_in(
            &model,
            window,
            move |this, _, e: &SelectEvent<Options>, _, cx| {
                let SelectEvent::Confirm(value) = e;
                if let Some(v) = value {
                    on_pick(this, Field::Model, v.clone(), cx);
                }
            },
        );
        let effort_sub = cx.subscribe_in(
            &effort,
            window,
            move |this, _, e: &SelectEvent<Options>, _, cx| {
                let SelectEvent::Confirm(value) = e;
                if let Some(v) = value {
                    on_pick(this, Field::Effort, v.clone(), cx);
                }
            },
        );
        Selects {
            model,
            effort,
            _subs: vec![model_sub, effort_sub],
        }
    }

    /// Shows what the core stored: the entries with the role defaults, and the picked values.
    pub fn show(&self, choice: Option<&DeskChoice>, window: &mut Window, cx: &mut App) {
        let apply = |state: &Entity<SelectState<Options>>,
                     options: Vec<Pick>,
                     value: String,
                     window: &mut Window,
                     cx: &mut App| {
            state.update(cx, |s, cx| {
                s.set_items(SearchableVec::new(options), window, cx);
                s.set_selected_value(&value, window, cx);
            });
        };
        apply(
            &self.model,
            model_options(choice),
            model_value(choice),
            window,
            cx,
        );
        apply(
            &self.effort,
            effort_options(choice),
            effort_value(choice),
            window,
            cx,
        );
    }

    /// The row of the two selects.
    pub fn row(&self) -> Div {
        let one = |state: &Entity<SelectState<Options>>, id: &'static str, label: &'static str| {
            div().flex_1().min_w_0().max_w(control::TRAILING_MAX).child(
                Select::new(state)
                    .id(id)
                    .small()
                    .accessibility_label(label)
                    .title_prefix(format!("{label}: ")),
            )
        };
        div()
            .flex()
            .items_center()
            .gap(space(2))
            .child(one(&self.model, "deskc-model", "Model"))
            .child(one(&self.effort, "deskc-effort", "Effort"))
    }
}
