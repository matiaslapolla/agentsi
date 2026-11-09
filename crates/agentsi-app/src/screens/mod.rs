//! The screens of a product, as panels of the dock.
//!
//! A `ScreenPanel` holds one screen view of one product. To add a screen:
//!
//! 1. Write its view in its own module here. It takes the product id (and an argument,
//!    for example a minutes id) and the `CoreState` entity of the product's host, and reads
//!    the core through `CoreState::link` at each call. It keeps no `Link`: an SSH host gets
//!    a new link at each connection. A view that opens another screen calls `Nav::open` with
//!    its core, so the new screen is on the same host. A view that reads a file or a
//!    folder of this device says so when `CoreState::remote` is set.
//! 2. Add a `ScreenKind` variant, its `name`, its `title`, its arm in `ScreenView`, and
//!    an action and a palette entry in `actions.rs` if the owner opens it with a key.
//!
//! The panel does the rest for each screen. It shows "device off" in place of the view while
//! the host is off, and the shell reloads the view when the host answers or its data changes.
//!
//! The layout file saves each screen panel as `{"kind": <name>, "product": <id>}`, with
//! `"arg"` when the screen has one, and `"host"` for a screen of an SSH host.

pub mod call;
pub mod desk;
pub mod git;
pub mod git_config;
pub mod guide;
pub mod meeting;
pub mod memory;
pub mod minute;
pub mod minutes;
pub mod parts;
pub mod roles;
pub mod roles_report;
pub mod settings;
pub mod stories;
pub mod story;

use gpui_kit::component::dock::{BasePanel, Panel, PanelEvent, PanelInfo, PanelState};
use gpui_kit::*;
use serde_json::{Value, json};

use agentsi_ui::style;
use agentsi_ui::tokens::space;
use gpui_kit::prelude::FluentBuilder as _;

use crate::alerts::{AlertTarget, Alerts};
use crate::layout::{SCREEN_PANEL, host_of};
use crate::look::LookGlobal;
use crate::model::CoreState;
use crate::pane::tab_bar;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ScreenKind {
    Desk,
    /// The list of the minutes of a product.
    Minutes,
    /// One minutes. Its id is the argument of the panel.
    Minute,
    /// The roster, the overlays and the specialists of a product, with validate, apply and reset.
    Roles,
    /// The voice models, the voices and the hotkey. It belongs to no product: its product is "".
    Settings,
    /// The stories of a product.
    Stories,
    /// One story. Its id is the argument of the panel.
    Story,
    /// One live pane for each role of the last meeting of a product.
    Meeting,
    /// The memory items of each agent and the reflections of a product.
    Memory,
    /// The call of one minutes. Its id is the argument of the panel.
    Call,
    /// The branch, the changed files and the commit of a product, and its git config.
    Git,
    /// The guide of the program. It belongs to no product: its product is "".
    Guide,
}

impl ScreenKind {
    pub fn name(self) -> &'static str {
        match self {
            ScreenKind::Desk => "desk",
            ScreenKind::Minutes => "minutes",
            ScreenKind::Minute => "minute",
            ScreenKind::Roles => "roles",
            ScreenKind::Settings => "settings",
            ScreenKind::Stories => "stories",
            ScreenKind::Story => "story",
            ScreenKind::Meeting => "meeting",
            ScreenKind::Memory => "memory",
            ScreenKind::Call => "call",
            ScreenKind::Git => "git",
            ScreenKind::Guide => "guide",
        }
    }

    pub fn parse(name: &str) -> Option<ScreenKind> {
        match name {
            "desk" => Some(ScreenKind::Desk),
            "minutes" => Some(ScreenKind::Minutes),
            "minute" => Some(ScreenKind::Minute),
            "roles" => Some(ScreenKind::Roles),
            "settings" => Some(ScreenKind::Settings),
            "stories" => Some(ScreenKind::Stories),
            "story" => Some(ScreenKind::Story),
            "meeting" => Some(ScreenKind::Meeting),
            "memory" => Some(ScreenKind::Memory),
            "call" => Some(ScreenKind::Call),
            "git" => Some(ScreenKind::Git),
            "guide" => Some(ScreenKind::Guide),
            _ => None,
        }
    }

    /// The settings and the guide belong to no product.
    fn of_no_product(self) -> bool {
        matches!(self, ScreenKind::Settings | ScreenKind::Guide)
    }

    fn title(self, arg: Option<&str>) -> String {
        match self {
            ScreenKind::Desk => "Front desk".into(),
            ScreenKind::Minutes => "Minutes".into(),
            ScreenKind::Minute => short(arg.unwrap_or("Minutes")),
            ScreenKind::Roles => "Roles".into(),
            ScreenKind::Settings => "Settings".into(),
            ScreenKind::Stories => "Stories".into(),
            ScreenKind::Story => arg.unwrap_or("Story").into(),
            ScreenKind::Meeting => "Meeting".into(),
            ScreenKind::Memory => "Memory".into(),
            ScreenKind::Call => "Call".into(),
            ScreenKind::Git => "Git".into(),
            ScreenKind::Guide => "Guide".into(),
        }
    }
}

/// The first characters of a minutes id: the date and the start of its slug.
fn short(id: &str) -> String {
    const SHOWN: usize = 22;
    if id.chars().count() <= SHOWN {
        return id.into();
    }
    id.chars().take(SHOWN).collect::<String>() + "…"
}

/// What the layout file keeps of a screen panel.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ScreenSpec {
    pub kind: ScreenKind,
    /// The SSH host of the product, or `None` for this device.
    pub host: Option<String>,
    pub product: String,
    pub arg: Option<String>,
}

/// The kind, the product and the argument of a screen panel in the layout file.
/// A `minute` panel with no argument is not a screen.
pub fn screen_of(state: &PanelState) -> Option<ScreenSpec> {
    if state.panel_name != SCREEN_PANEL {
        return None;
    }
    let PanelInfo::Panel(v) = &state.info else {
        return None;
    };
    let kind = ScreenKind::parse(v.get("kind")?.as_str()?)?;
    let arg = v.get("arg").and_then(Value::as_str).map(String::from);
    if matches!(
        kind,
        ScreenKind::Minute | ScreenKind::Story | ScreenKind::Call
    ) && arg.is_none()
    {
        return None;
    }
    Some(ScreenSpec {
        kind,
        host: host_of(state).map(String::from),
        product: v.get("product")?.as_str()?.to_string(),
        arg,
    })
}

/// The view of a screen, typed so that the shell can reload it.
enum ScreenView {
    Desk(Entity<desk::DeskScreen>),
    Minutes(Entity<minutes::MinutesScreen>),
    Minute(Entity<minute::MinuteScreen>),
    Roles(Entity<roles::RolesScreen>),
    Settings(Entity<settings::SettingsScreen>),
    Stories(Entity<stories::StoriesScreen>),
    Story(Entity<story::StoryScreen>),
    Meeting(Entity<meeting::MeetingScreen>),
    Memory(Entity<memory::MemoryScreen>),
    Call(Entity<call::CallScreen>),
    Git(Entity<git::GitScreen>),
    Guide(Entity<guide::GuideScreen>),
}

impl ScreenView {
    fn new(
        spec: &ScreenSpec,
        core: &Entity<CoreState>,
        cx: &mut Context<ScreenPanel>,
    ) -> ScreenView {
        let (product, core) = (spec.product.clone(), core.clone());
        match spec.kind {
            ScreenKind::Desk => {
                ScreenView::Desk(cx.new(|cx| desk::DeskScreen::new(product, core, cx)))
            }
            ScreenKind::Minutes => {
                ScreenView::Minutes(cx.new(|cx| minutes::MinutesScreen::new(product, core, cx)))
            }
            ScreenKind::Minute => {
                let id = spec.arg.clone().unwrap_or_default();
                ScreenView::Minute(cx.new(|cx| minute::MinuteScreen::new(id, core, cx)))
            }
            ScreenKind::Roles => {
                ScreenView::Roles(cx.new(|cx| roles::RolesScreen::new(product, core, cx)))
            }
            ScreenKind::Settings => {
                ScreenView::Settings(cx.new(|cx| settings::SettingsScreen::new(core, cx)))
            }
            ScreenKind::Stories => {
                ScreenView::Stories(cx.new(|cx| stories::StoriesScreen::new(product, core, cx)))
            }
            ScreenKind::Story => {
                let id = spec.arg.clone().unwrap_or_default();
                ScreenView::Story(cx.new(|cx| story::StoryScreen::new(product, id, core, cx)))
            }
            ScreenKind::Meeting => {
                ScreenView::Meeting(cx.new(|cx| meeting::MeetingScreen::new(product, core, cx)))
            }
            ScreenKind::Memory => {
                ScreenView::Memory(cx.new(|cx| memory::MemoryScreen::new(product, core, cx)))
            }
            ScreenKind::Call => {
                let id = spec.arg.clone().unwrap_or_default();
                ScreenView::Call(cx.new(|cx| call::CallScreen::new(product, id, core, cx)))
            }
            ScreenKind::Git => ScreenView::Git(cx.new(|cx| git::GitScreen::new(product, core, cx))),
            ScreenKind::Guide => ScreenView::Guide(cx.new(guide::GuideScreen::new)),
        }
    }

    fn any(&self) -> AnyView {
        match self {
            ScreenView::Desk(v) => v.clone().into(),
            ScreenView::Minutes(v) => v.clone().into(),
            ScreenView::Minute(v) => v.clone().into(),
            ScreenView::Roles(v) => v.clone().into(),
            ScreenView::Settings(v) => v.clone().into(),
            ScreenView::Stories(v) => v.clone().into(),
            ScreenView::Story(v) => v.clone().into(),
            ScreenView::Meeting(v) => v.clone().into(),
            ScreenView::Memory(v) => v.clone().into(),
            ScreenView::Call(v) => v.clone().into(),
            ScreenView::Git(v) => v.clone().into(),
            ScreenView::Guide(v) => v.clone().into(),
        }
    }

    /// The dock shows or hides this screen. A palette write acts only on a screen that the
    /// owner sees, so it cannot reach the core of a hidden tab.
    fn show(&self, shown: bool, cx: &mut App) {
        match self {
            ScreenView::Desk(v) => v.update(cx, |s, cx| s.show(shown, cx)),
            ScreenView::Minute(v) => v.update(cx, |s, cx| s.show(shown, cx)),
            ScreenView::Memory(v) => v.update(cx, |s, _| s.show(shown)),
            ScreenView::Call(v) => v.update(cx, |s, cx| s.show(shown, cx)),
            ScreenView::Git(v) => v.update(cx, |s, cx| s.show(shown, cx)),
            ScreenView::Settings(v) => v.update(cx, |s, cx| s.show(shown, cx)),
            _ => {}
        }
    }

    fn reload(&self, cx: &mut App) {
        match self {
            ScreenView::Desk(v) => v.update(cx, |s, cx| s.reload(cx)),
            ScreenView::Minutes(v) => v.update(cx, |s, cx| s.reload(cx)),
            ScreenView::Minute(v) => v.update(cx, |s, cx| s.reload(cx)),
            ScreenView::Roles(v) => v.update(cx, |s, cx| s.reload(cx)),
            ScreenView::Settings(v) => v.update(cx, |s, cx| s.reload(cx)),
            ScreenView::Stories(v) => v.update(cx, |s, cx| s.reload(cx)),
            ScreenView::Story(v) => v.update(cx, |s, cx| s.reload(cx)),
            ScreenView::Memory(v) => v.update(cx, |s, cx| s.reload(cx)),
            ScreenView::Call(v) => v.update(cx, |s, cx| s.reload(cx)),
            ScreenView::Git(v) => v.update(cx, |s, cx| s.reload(cx)),
            // The events keep this screen current.
            ScreenView::Meeting(_) => {}
            // The page is in the program.
            ScreenView::Guide(_) => {}
        }
    }
}

pub struct ScreenPanel {
    pub kind: ScreenKind,
    /// The host of `core`. A panel never shows a product of another host.
    pub host: Option<String>,
    pub product: String,
    pub arg: Option<String>,
    core: Entity<CoreState>,
    view: ScreenView,
    focus: FocusHandle,
    /// The dock displays this tab.
    pub shown: bool,
    _observe: Subscription,
    /// A fade of the tint after the owner opened an alert redraws this screen.
    _alerts: Option<Subscription>,
}

impl ScreenPanel {
    pub fn new(spec: ScreenSpec, core: Entity<CoreState>, cx: &mut Context<Self>) -> Self {
        let view = ScreenView::new(&spec, &core, cx);
        // The dock reports the tab that it displays.
        view.show(false, cx);
        ScreenPanel {
            kind: spec.kind,
            host: core.read(cx).remote.clone(),
            // The tab title shows the product name, which `CoreState` knows.
            _observe: cx.observe(&core, |_, _, cx| cx.notify()),
            _alerts: Alerts::try_get(cx).map(|c| cx.observe(&c, |_, _, cx| cx.notify())),
            product: spec.product,
            arg: spec.arg,
            core,
            view,
            focus: cx.focus_handle(),
            shown: false,
        }
    }

    /// The target of the alerts about this screen: the meeting, the story or the decisions.
    fn alert_target(&self) -> Option<AlertTarget> {
        let (host, product) = (self.host.clone(), self.product.clone());
        match self.kind {
            ScreenKind::Meeting => Some(AlertTarget::Meeting { host, product }),
            ScreenKind::Story => Some(AlertTarget::Story {
                host,
                product,
                id: self.arg.clone()?,
            }),
            ScreenKind::Desk => Some(AlertTarget::Decisions { host, product }),
            _ => None,
        }
    }

    pub fn is(&self, spec: &ScreenSpec) -> bool {
        self.kind == spec.kind
            && self.host == spec.host
            && self.product == spec.product
            && self.arg == spec.arg
    }

    /// A change of the data of `host` reloads this screen. The settings screen follows its
    /// own events, and the guide has no data of a host.
    pub fn follows(&self, host: Option<&str>) -> bool {
        !self.kind.of_no_product() && self.host.as_deref() == host
    }

    /// The front desk of this panel, for the tests.
    #[cfg(test)]
    pub fn desk(&self) -> Option<Entity<desk::DeskScreen>> {
        match &self.view {
            ScreenView::Desk(v) => Some(v.clone()),
            _ => None,
        }
    }

    /// The settings screen of this panel, for the tests.
    #[cfg(test)]
    pub fn settings(&self) -> Option<Entity<settings::SettingsScreen>> {
        match &self.view {
            ScreenView::Settings(v) => Some(v.clone()),
            _ => None,
        }
    }

    /// Scrolls the Data section of a settings screen into view. With a product, it selects
    /// the "Product" row for it.
    pub fn reveal_data(&self, product: Option<String>, cx: &mut App) {
        if let ScreenView::Settings(v) = &self.view {
            v.update(cx, |s, cx| s.reveal_data(product, cx));
        }
    }

    /// Scrolls a guide screen to the section with this anchor.
    pub fn reveal_section(&self, anchor: &str, cx: &mut App) {
        if let ScreenView::Guide(v) = &self.view {
            v.update(cx, |s, cx| s.reveal(anchor, cx));
        }
    }

    /// The guide screen of this panel, for the tests.
    #[cfg(test)]
    pub fn guide(&self) -> Option<Entity<guide::GuideScreen>> {
        match &self.view {
            ScreenView::Guide(v) => Some(v.clone()),
            _ => None,
        }
    }

    /// Starts the meeting in the question field of a meeting screen.
    pub fn start_meeting(&self, cx: &mut App) {
        if let ScreenView::Meeting(v) = &self.view {
            v.update(cx, |s, cx| s.start(cx));
        }
    }

    /// Starts a review action on a story screen: `accept` and `reject` show their Confirm.
    pub fn story_act(&self, action: &str, cx: &mut App) {
        if let ScreenView::Story(v) = &self.view {
            v.update(cx, |s, cx| s.act(action, cx));
        }
    }

    /// A story screen that waits for the Confirm of the owner.
    #[cfg(test)]
    pub fn story_pending(&self, cx: &App) -> bool {
        matches!(&self.view, ScreenView::Story(v) if v.read(cx).has_pending())
    }

    /// The memory screen of this panel, for the palette actions.
    pub fn memory(&self) -> Option<Entity<memory::MemoryScreen>> {
        match &self.view {
            ScreenView::Memory(v) => Some(v.clone()),
            _ => None,
        }
    }

    /// The order of use of the minutes screen of this panel, when the dock shows it.
    pub fn minute_stamp(&self, cx: &App) -> Option<u64> {
        match &self.view {
            ScreenView::Minute(v) => v.read(cx).shown().then(|| v.read(cx).stamp()),
            _ => None,
        }
    }

    /// Reads the screen from the core again.
    pub fn reload(&self, cx: &mut App) {
        self.view.reload(cx);
    }
}

impl EventEmitter<PanelEvent> for ScreenPanel {}

impl Focusable for ScreenPanel {
    /// A list takes the focus as one element, so the panel hands out its handle.
    fn focus_handle(&self, cx: &App) -> FocusHandle {
        match &self.view {
            ScreenView::Minutes(v) => v.read(cx).focus_handle(),
            ScreenView::Desk(v) => v.read(cx).focus_handle(),
            ScreenView::Minute(v) => v.read(cx).focus_handle(),
            ScreenView::Stories(v) => v.read(cx).focus_handle(),
            ScreenView::Memory(v) => v.read(cx).focus_handle(),
            ScreenView::Call(v) => v.read(cx).focus_handle(),
            ScreenView::Git(v) => v.read(cx).focus_handle(),
            ScreenView::Settings(v) => v.read(cx).focus_handle(),
            ScreenView::Guide(v) => v.read(cx).focus_handle(),
            ScreenView::Meeting(v) => v.read(cx).focus_handle(),
            _ => self.focus.clone(),
        }
    }
}

impl BasePanel for ScreenPanel {
    fn panel_name(&self) -> &'static str {
        SCREEN_PANEL
    }

    fn set_active(&mut self, active: bool, _: &mut Window, cx: &mut Context<Self>) {
        self.shown = active;
        if active && let (Some(center), Some(target)) = (Alerts::try_get(cx), self.alert_target()) {
            center.update(cx, |c, cx| c.opened(&target, cx));
        }
        self.view.show(active, cx);
    }

    fn dump(&self, _: &App) -> PanelState {
        let mut info = json!({"kind": self.kind.name(), "product": self.product});
        if let Some(host) = &self.host {
            info["host"] = json!(host);
        }
        if let Some(arg) = &self.arg {
            info["arg"] = json!(arg);
        }
        PanelState {
            info: PanelInfo::panel(info),
            ..PanelState::new(SCREEN_PANEL)
        }
    }
}

impl Panel for ScreenPanel {
    fn title(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        if self.kind.of_no_product() {
            return div().child(self.kind.title(None));
        }
        let name = self
            .core
            .read(cx)
            .product(&self.product)
            .map(|p| p.name.clone())
            .unwrap_or_else(|| self.product.clone());
        let title = self.kind.title(self.arg.as_deref());
        let title = match &self.host {
            Some(host) => format!("{title} · {name} · {host}"),
            None => format!("{title} · {name}"),
        };
        let hue = LookGlobal::with(cx, |look| {
            look.product(self.host.as_deref(), &self.product).hue
        })
        .flatten();
        div()
            .flex()
            .items_center()
            .gap(space(2))
            .min_w_0()
            .when_some(hue, |el, h| {
                el.child(tab_bar(
                    h,
                    format!("tab-color:{}:{}", self.kind.name(), self.product),
                ))
            })
            .child(div().min_w_0().truncate().child(title))
    }

    fn inner_padding(&self, _: &App) -> bool {
        false
    }
}

impl Render for ScreenPanel {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let root = div().track_focus(&self.focus).size_full();
        if let Some((line, why)) = self.core.read(cx).away() {
            return root.child(parts::notice(line, why));
        }
        let fade = self
            .alert_target()
            .zip(Alerts::try_get(cx))
            .and_then(|(t, c)| c.read(cx).fade(&t));
        // A cached view draws again only when it notifies itself: a terminal event
        // elsewhere must not rebuild a screen.
        root.relative()
            .child(
                self.view
                    .any()
                    .cached(StyleRefinement::default().size_full()),
            )
            .when_some(fade, |el, (state, alert)| {
                el.child(style::tint_fade(
                    format!("screen-tint:{}:{alert}", self.kind.name()),
                    state,
                ))
            })
    }
}

#[cfg(test)]
mod tests {
    use super::{SCREEN_PANEL, ScreenKind, ScreenSpec, screen_of, short};
    use gpui_kit::component::dock::{PanelInfo, PanelState};
    use serde_json::{Value, json};

    fn state(info: Value) -> PanelState {
        PanelState {
            info: PanelInfo::panel(info),
            ..PanelState::new(SCREEN_PANEL)
        }
    }

    #[test]
    fn a_screen_panel_reads_back_its_kind_product_and_argument() {
        let desk = screen_of(&state(json!({"kind": "desk", "product": "demo"}))).unwrap();
        assert_eq!(
            desk,
            ScreenSpec {
                kind: ScreenKind::Desk,
                host: None,
                product: "demo".into(),
                arg: None
            }
        );
        let one = screen_of(&state(
            json!({"kind": "minute", "product": "demo", "arg": "m1"}),
        ))
        .unwrap();
        assert_eq!(
            (one.kind, one.arg.as_deref()),
            (ScreenKind::Minute, Some("m1"))
        );
        let list = screen_of(&state(json!({"kind": "minutes", "product": "demo"}))).unwrap();
        assert_eq!(list.kind, ScreenKind::Minutes);
        let guide = screen_of(&state(json!({"kind": "guide", "product": ""}))).unwrap();
        assert_eq!(guide, ScreenSpec::guide());
        let settings = screen_of(&state(json!({"kind": "settings", "product": ""}))).unwrap();
        assert_eq!(
            (settings.kind, settings.product.as_str()),
            (ScreenKind::Settings, "")
        );
    }

    #[test]
    fn a_screen_panel_of_a_host_reads_back_its_host() {
        let desk = screen_of(&state(
            json!({"kind": "desk", "product": "demo", "host": "studio"}),
        ))
        .unwrap();
        assert_eq!(desk.host.as_deref(), Some("studio"));
    }

    #[test]
    fn a_bad_screen_panel_is_not_a_screen() {
        // Not a screen panel, an unknown kind, no product, and a minutes with no id.
        assert!(screen_of(&PanelState::new("other")).is_none());
        assert!(screen_of(&state(json!({"kind": "nope", "product": "demo"}))).is_none());
        assert!(screen_of(&state(json!({"kind": "desk"}))).is_none());
        assert!(screen_of(&state(json!({"kind": "minute", "product": "demo"}))).is_none());
        assert!(screen_of(&state(json!({"kind": "story", "product": "demo"}))).is_none());
        assert!(screen_of(&state(json!({"kind": "call", "product": "demo"}))).is_none());
    }

    #[test]
    fn a_call_panel_reads_back_its_minutes_id_and_its_host() {
        let call = screen_of(&state(
            json!({"kind": "call", "product": "demo", "arg": "m1", "host": "studio"}),
        ))
        .unwrap();
        assert_eq!(
            call,
            ScreenSpec {
                kind: ScreenKind::Call,
                host: Some("studio".into()),
                product: "demo".into(),
                arg: Some("m1".into())
            }
        );
        assert_eq!(ScreenSpec::call("demo", "m1").arg.as_deref(), Some("m1"));
    }

    #[test]
    fn a_long_minutes_id_is_short_in_a_tab_title() {
        assert_eq!(
            short("2026-11-02-1724-review-the-spec"),
            "2026-11-02-1724-review…"
        );
        assert_eq!(short("2026-11-02-1724"), "2026-11-02-1724");
    }

    #[test]
    fn each_kind_has_a_name_that_parses_back() {
        for k in [
            ScreenKind::Desk,
            ScreenKind::Minutes,
            ScreenKind::Minute,
            ScreenKind::Roles,
            ScreenKind::Settings,
            ScreenKind::Stories,
            ScreenKind::Story,
            ScreenKind::Meeting,
            ScreenKind::Memory,
            ScreenKind::Call,
            ScreenKind::Git,
            ScreenKind::Guide,
        ] {
            assert_eq!(ScreenKind::parse(k.name()), Some(k));
        }
    }
}
