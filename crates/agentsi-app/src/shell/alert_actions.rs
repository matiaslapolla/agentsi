//! What the shell does when an alert is raised: the tone, the toast and the system
//! notification. It also opens the target of an alert. The alert log is the Alerts pane.

use std::time::Instant;

use agentsi_ui::components::{Button, StateMark, state_icon};
use agentsi_ui::style::StyleExt as _;
use agentsi_ui::tokens::{line, size, space, text};
use gpui_kit::component::notification::{Notification, NotificationDelivery};
use gpui_kit::component::{Icon, WindowExt as _};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;

use super::{Shell, product_names};
use crate::alerts::{Alert, AlertTarget, Alerts};
use crate::alerts_file::{self, format_time, local_minute};
use crate::chime::{self, ChimeGlobal, Scene};
use crate::look::LookGlobal;
use crate::model::Cores;
use crate::model::ProductRef;
use crate::panes::PaneKind;
use crate::screens::{ScreenKind, ScreenSpec};

/// The id type of the toasts of the alerts. The key is the target.
struct AlertToast;
/// The id type of the system notifications of the alerts.
struct AlertSystem;

impl Shell {
    /// True when the owner sees `target` now: the window is active and the tab of the target
    /// is the one that its dock shows.
    pub(super) fn target_visible(&self, target: &AlertTarget, window: &Window, cx: &App) -> bool {
        if !window.is_window_active() {
            return false;
        }
        let screen = |kind: ScreenKind, host: &Option<String>, product: &str, arg: Option<&str>| {
            self.open
                .borrow()
                .screens
                .iter()
                .filter_map(WeakEntity::upgrade)
                .any(|p| {
                    let in_view = self.in_dock(p.entity_id(), cx);
                    let p = p.read(cx);
                    p.shown
                        && in_view
                        && p.kind == kind
                        && p.host == *host
                        && (kind == ScreenKind::Settings
                            || (p.product == product
                                && arg.is_none_or(|a| p.arg.as_deref() == Some(a))))
                })
        };
        match target {
            AlertTarget::Pane { host, id } => self
                .open
                .borrow()
                .terminals
                .iter()
                .filter_map(WeakEntity::upgrade)
                .any(|p| {
                    p.read(cx).shown
                        && p.read(cx).is(host.as_deref(), id, cx)
                        && self.in_dock(p.entity_id(), cx)
                }),
            AlertTarget::Meeting { host, product } => {
                screen(ScreenKind::Meeting, host, product, None)
            }
            AlertTarget::Story { host, product, id } => {
                screen(ScreenKind::Story, host, product, Some(id))
            }
            AlertTarget::Decisions { host, product } => {
                screen(ScreenKind::Desk, host, product, None)
            }
            AlertTarget::Model { .. } => screen(ScreenKind::Settings, &None, "", None),
        }
    }

    /// Takes a new alert: the tone, then the toast when the owner cannot see the target, then
    /// the system notification when the window is not active.
    pub(super) fn alert_raised(
        &mut self,
        alert: &Alert,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        // The open log shows the alert at once.
        if self.panes.is_open(PaneKind::Alerts)
            && let Some(center) = Alerts::try_get(cx)
        {
            center.update(cx, |c, cx| c.mark_seen(cx));
        }
        let visible = self.target_visible(&alert.target, window, cx);
        self.play_tone(alert, visible, cx);
        if !Alert::is_alertable(alert.state) {
            return;
        }
        let config = alerts_file::current(cx);
        let key: SharedString = format!("{:?}", alert.target).into();
        let shell = cx.weak_entity();
        let target = alert.target.clone();
        let open = move |_: &ClickEvent, window: &mut Window, cx: &mut App| {
            let _ = shell.update(cx, |this, cx| this.open_alert_target(&target, window, cx));
        };
        let icon = state_icon(alert.state)
            .map(|i| Icon::new(i).text_color(Hsla::from(alert.state.color())));
        if !visible {
            let toast = Notification::new()
                .id1::<AlertToast>(key.clone())
                .message(alert.line())
                .on_click(open.clone())
                .when_some(icon, |n, i| n.icon(i));
            window.push_notification(toast, cx);
        }
        let quiet = config.is_quiet(local_minute(std::time::SystemTime::now()));
        if !window.is_window_active() && config.system && !quiet {
            let system = Notification::new()
                .id1::<AlertSystem>(key)
                .title(alert.state.word())
                .message(alert.text.clone())
                .delivery(NotificationDelivery::System)
                .on_click(open);
            window.push_notification(system, cx);
        }
    }

    /// Shows the target of an alert, and ends its glow.
    pub(super) fn open_alert_target(
        &mut self,
        target: &AlertTarget,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if let Some(center) = Alerts::try_get(cx) {
            center.update(cx, |c, cx| c.opened(target, cx));
        }
        match target {
            AlertTarget::Pane { host, id } => self.focus_terminal(host.clone(), id, window, cx),
            AlertTarget::Meeting { host, product } => {
                self.show_screen(ScreenSpec::meeting(product).on(host.clone()), window, cx)
            }
            AlertTarget::Story { host, product, id } => {
                self.show_screen(ScreenSpec::story(product, id).on(host.clone()), window, cx)
            }
            AlertTarget::Decisions { host, product } => self.show_screen(
                ScreenSpec::desk(&ProductRef {
                    host: host.clone(),
                    id: product.clone(),
                }),
                window,
                cx,
            ),
            AlertTarget::Model { .. } => self.show_screen(ScreenSpec::settings(), window, cx),
        }
    }

    fn play_tone(&mut self, alert: &Alert, visible: bool, cx: &mut Context<Self>) {
        let Some(chime) = cx.try_global::<ChimeGlobal>().map(|g| g.0.clone()) else {
            return;
        };
        let now = Instant::now();
        let scene = Scene {
            now,
            minute: alerts_file::local_minute(std::time::SystemTime::now()),
            visible,
            call_playing: chime::call_playing(),
        };
        let config = alerts_file::current(cx);
        if chime.borrow_mut().consider(alert, &config, scene).is_err() {
            return;
        }
        chime::close_when_idle(cx);
    }
}

/// The owner pressed Open on a row of the log.
pub struct OpenAlert(pub AlertTarget);

/// The Alerts pane: the alerts of this run, newest first.
pub struct AlertLog {
    cores: Cores,
    _observe: Vec<Subscription>,
}

impl EventEmitter<OpenAlert> for AlertLog {}

impl AlertLog {
    pub fn new(cores: Cores, cx: &mut Context<Self>) -> Self {
        let mut observe: Vec<Subscription> = cores
            .all()
            .map(|c| cx.observe(c, |_, _, cx| cx.notify()))
            .collect();
        if let Some(center) = Alerts::try_get(cx) {
            observe.push(cx.observe(&center, |_, _, cx| cx.notify()));
        }
        if let Some(l) = cx.try_global::<LookGlobal>().map(|g| g.0.clone()) {
            observe.push(cx.observe(&l, |_, _, cx| cx.notify()));
        }
        AlertLog {
            cores,
            _observe: observe,
        }
    }
}

impl Render for AlertLog {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let rows: Vec<Alert> = Alerts::try_get(cx)
            .map(|c| c.read(cx).log().cloned().collect())
            .unwrap_or_default();
        let products = product_names(&self.cores, cx);
        let name = move |alert: &Alert| -> Option<String> {
            let id = alert.product.as_deref()?;
            let host = alert.target.host();
            products
                .iter()
                .find(|(p, _)| p.id == id && p.host.as_deref() == host)
                .map(|(_, n)| n.clone())
        };
        div()
            .id("alert-log")
            .debug_selector(|| "alert-log".into())
            .flex()
            .flex_col()
            .size_full()
            .overflow_y_scroll()
            .pad_x(space(3))
            .when(rows.is_empty(), |el| {
                el.child(
                    div()
                        .py(space(2))
                        .text_size(size::sm())
                        .text_color(text::tertiary())
                        .child("No alerts in this run."),
                )
            })
            .children(rows.iter().enumerate().map(|(i, alert)| {
                let target = alert.target.clone();
                let who = [
                    name(alert),
                    Some(alert.label.clone()).filter(|l| !l.is_empty()),
                ]
                .into_iter()
                .flatten()
                .collect::<Vec<_>>()
                .join(" \u{b7} ");
                div()
                    .debug_selector(|| format!("alert-row-{i}"))
                    .flex()
                    .flex_col()
                    .gap(space(1))
                    .py(space(2))
                    .border_b_1()
                    .border_color(line::border())
                    .child(
                        div()
                            .flex()
                            .gap(space(2))
                            .text_size(size::sm())
                            .text_color(text::tertiary())
                            .child(format_time(local_minute(alert.at)))
                            .child(who),
                    )
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .justify_between()
                            .gap(space(2))
                            .child(StateMark::new(alert.state).reason(alert.text.clone()))
                            .child(
                                Button::new(("alert-open", i))
                                    .small()
                                    .secondary()
                                    .label("Open")
                                    .on_click(cx.listener(move |_, _, _, cx| {
                                        cx.emit(OpenAlert(target.clone()))
                                    })),
                            ),
                    )
            }))
    }
}
