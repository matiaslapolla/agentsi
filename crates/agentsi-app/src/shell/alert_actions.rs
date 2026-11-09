//! What the shell does when an alert is raised: the tone, and later the toast.

use std::time::{Duration, Instant};

use gpui_kit::*;

use super::Shell;
use crate::alerts::{Alert, AlertTarget};
use crate::alerts_file;
use crate::chime::{self, ChimeGlobal, Scene};
use crate::screens::ScreenKind;

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
                    let p = p.read(cx);
                    p.shown
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

    /// Takes a new alert: the tone now. The chime applies the rules.
    pub(super) fn alert_raised(
        &mut self,
        alert: &Alert,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let visible = self.target_visible(&alert.target, window, cx);
        self.play_tone(alert, visible, cx);
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
        // The device closes after 10 s with no tone.
        cx.spawn(async move |_, cx| {
            cx.background_executor()
                .timer(chime::IDLE + Duration::from_millis(100))
                .await;
            chime.borrow_mut().close_if_idle(Instant::now());
        })
        .detach();
    }
}
