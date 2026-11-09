//! The Alerts section: the mute, the volume, the tone of Done, the system notifications and
//! the quiet hours. A click applies the choice and saves it in `alerts.toml` (see
//! `crate::alerts_file`). A change of the volume plays `attention` once.

use std::time::Instant;

use agentsi_ui::components::{Button, Switch};
use agentsi_ui::tokens::{Status, size, space};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;

use super::{SettingsScreen, pair};
use crate::alerts_file::{self, AlertsConfig, Minute, format_time};
use crate::chime::ChimeGlobal;
use crate::screens::parts::section;

/// The volumes of the buttons.
const VOLUMES: [u8; 5] = [0, 25, 50, 75, 100];
/// The quiet hours of the buttons, from and to.
const QUIET: [(Minute, Minute); 2] = [(22 * 60, 8 * 60), (23 * 60, 7 * 60)];

fn quiet_label(range: (Minute, Minute)) -> String {
    format!("{} to {}", format_time(range.0), format_time(range.1))
}

impl SettingsScreen {
    /// A switch row that sets one flag of the alerts file.
    fn flag(
        &self,
        id: &'static str,
        value: bool,
        set: fn(&mut AlertsConfig, bool),
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let this = cx.entity().downgrade();
        Switch::new(id)
            .checked(value)
            .accessibility_label(id)
            .on_click(move |on, _, cx| {
                let result = alerts_file::change(cx, |c| set(c, *on));
                let _ = this.update(cx, |t, cx| {
                    t.alerts_error = result.err();
                    cx.notify();
                });
            })
    }

    fn volume_row(&self, now: &AlertsConfig, cx: &mut Context<Self>) -> Div {
        div()
            .debug_selector(|| "alerts-volume".into())
            .flex()
            .items_center()
            .gap(space(2))
            .children(VOLUMES.into_iter().enumerate().map(|(i, volume)| {
                let button = Button::new(("alerts-volume", i))
                    .small()
                    .label(if volume == 0 {
                        "Off".into()
                    } else {
                        format!("{volume}")
                    })
                    .on_click(cx.listener(move |this, _, _, cx| {
                        let result = alerts_file::change(cx, |c| c.volume = volume);
                        this.alerts_error = result.err();
                        let config = alerts_file::current(cx);
                        if let Some(chime) = cx.try_global::<ChimeGlobal>() {
                            chime.0.borrow_mut().preview(&config, Instant::now());
                        }
                        cx.notify();
                    }));
                if now.volume == volume {
                    button.primary()
                } else {
                    button.secondary()
                }
            }))
    }

    fn quiet_row(&self, now: &AlertsConfig, cx: &mut Context<Self>) -> Div {
        let off = Button::new("alerts-quiet-off")
            .small()
            .label("Off")
            .on_click(cx.listener(|this, _, _, cx| {
                this.alerts_error = alerts_file::change(cx, |c| c.quiet = None).err();
                cx.notify();
            }));
        let custom = now.quiet.filter(|q| !QUIET.contains(q));
        div()
            .debug_selector(|| "alerts-quiet".into())
            .flex()
            .items_center()
            .gap(space(2))
            .child(if now.quiet.is_none() {
                off.primary()
            } else {
                off.secondary()
            })
            .children(QUIET.into_iter().enumerate().map(|(i, range)| {
                let button = Button::new(("alerts-quiet", i))
                    .small()
                    .label(quiet_label(range))
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.alerts_error =
                            alerts_file::change(cx, |c| c.quiet = Some(range)).err();
                        cx.notify();
                    }));
                if now.quiet == Some(range) {
                    button.primary()
                } else {
                    button.secondary()
                }
            }))
            .when_some(custom, |el, range| {
                el.child(
                    Button::new("alerts-quiet-custom")
                        .small()
                        .primary()
                        .label(quiet_label(range)),
                )
            })
    }

    pub(super) fn alerts_section(&self, cx: &mut Context<Self>) -> Div {
        let now = alerts_file::current(cx);
        let note = self.alerts_error.clone().or_else(|| alerts_file::note(cx));
        div()
            .flex()
            .flex_col()
            .gap(space(3))
            .child(section("Alerts", None))
            .child(pair(
                "Mute",
                self.flag("alerts-mute", now.mute, |c, v| c.mute = v, cx),
            ))
            .child(pair("Volume", self.volume_row(&now, cx)))
            .child(pair(
                "Tone when work is done",
                self.flag("alerts-done", now.done, |c, v| c.done = v, cx),
            ))
            .child(pair(
                "System notifications",
                self.flag("alerts-system", now.system, |c, v| c.system = v, cx),
            ))
            .child(pair("Quiet hours", self.quiet_row(&now, cx)))
            .when_some(note, |el, n| {
                el.child(
                    div()
                        .debug_selector(|| "alerts-note".into())
                        .text_size(size::sm())
                        .text_color(Status::Warn.color())
                        .child(n),
                )
            })
    }
}
