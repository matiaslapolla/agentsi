//! The keep time of the voice models, in the section "Voice models": how long the pill keeps a
//! model in memory after its last use, and the memory of the pill now.
//!
//! The core writes the choice in `pill.toml` and sends it to the pill (`voice.models.keep`).
//! The memory line reads `VmRSS` of the running `agentsi-pill` from `/proc`. It reads when the
//! section opens and after a change: at once, and once more after [`SETTLE`], when the pill has
//! dropped or loaded its models. It never reads on a repeating timer.

use agentsi_proto::voice_api::{KeepTime, ModelsKeep, ModelsKeepParams};
use agentsi_ui::components::Button;
use agentsi_ui::tokens::{Status, size, space, text};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;

use super::{SettingsScreen, pair};
use crate::screens::parts::Load;

/// A drop takes milliseconds and a load about 2 s, so the second read after a change waits this.
const SETTLE: std::time::Duration = std::time::Duration::from_secs(3);

/// What the section knows about the keep time and the pill.
pub(super) struct KeepState {
    keep: Load<KeepTime>,
    /// The resident memory of the pill in KB. `None` before the first read, `Some(None)` when
    /// no pill runs.
    pill_kb: Option<Option<u64>>,
    /// The refusal of the last change.
    error: Option<String>,
}

impl Default for KeepState {
    fn default() -> Self {
        KeepState {
            keep: Load::Loading,
            pill_kb: None,
            error: None,
        }
    }
}

pub fn label(keep: KeepTime) -> &'static str {
    match keep {
        KeepTime::OneMinute => "1 minute",
        KeepTime::FiveMinutes => "5 minutes",
        KeepTime::FifteenMinutes => "15 minutes",
        KeepTime::OneHour => "1 hour",
        KeepTime::Always => "Always",
        KeepTime::Start => "Load at start",
    }
}

/// The resident memory of the first `agentsi-pill` process of this user, in KB.
fn pill_kb() -> Option<u64> {
    use std::os::unix::fs::MetadataExt as _;
    let me = std::fs::metadata("/proc/self").ok()?.uid();
    let mut pids: Vec<u32> = std::fs::read_dir("/proc")
        .ok()?
        .filter_map(|e| e.ok()?.file_name().to_str()?.parse().ok())
        .collect();
    pids.sort_unstable();
    pids.into_iter().find_map(|pid| {
        let dir = std::path::PathBuf::from(format!("/proc/{pid}"));
        if std::fs::metadata(&dir).ok()?.uid() != me {
            return None;
        }
        let comm = std::fs::read_to_string(dir.join("comm")).ok()?;
        if comm.trim_end() != "agentsi-pill" {
            return None;
        }
        agentsi_proto::voice_api::rss_kb(&std::fs::read_to_string(dir.join("status")).ok()?)
    })
}

/// The line under the choice.
pub fn memory_line(kb: Option<u64>) -> String {
    match kb {
        Some(kb) => format!("The pill uses {} MB of memory now.", kb.div_ceil(1024)),
        None => "The pill does not run.".into(),
    }
}

impl SettingsScreen {
    /// Reads the keep time from the core, and the memory of the pill.
    pub(super) fn load_keep(&mut self, cx: &mut Context<Self>) {
        let call = self
            .core
            .read(cx)
            .link
            .call::<ModelsKeep>(&ModelsKeepParams { keep: None });
        cx.spawn(async move |this, cx| {
            let result = call.await.map(|r| r.keep).map_err(|e| e.to_string());
            let _ = this.update(cx, |this, cx| {
                this.keep.keep.settle(result);
                cx.notify();
            });
        })
        .detach();
        self.read_pill_memory(cx);
    }

    /// Reads `/proc` off the main thread.
    pub(super) fn read_pill_memory(&mut self, cx: &mut Context<Self>) {
        let read = cx.background_spawn(async move { pill_kb() });
        cx.spawn(async move |this, cx| {
            let kb = read.await;
            let _ = this.update(cx, |this, cx| {
                if this.keep.pill_kb != Some(kb) {
                    this.keep.pill_kb = Some(kb);
                    cx.notify();
                }
            });
        })
        .detach();
    }

    fn set_keep(&mut self, keep: KeepTime, cx: &mut Context<Self>) {
        let call = self
            .core
            .read(cx)
            .link
            .call::<ModelsKeep>(&ModelsKeepParams { keep: Some(keep) });
        cx.spawn(async move |this, cx| {
            let result = call.await;
            let _ = this.update(cx, |this, cx| {
                match result {
                    Ok(r) => {
                        this.keep.keep = Load::Ready(r.keep);
                        this.keep.error = None;
                    }
                    Err(e) => this.keep.error = Some(e.to_string()),
                }
                this.read_pill_memory(cx);
                cx.notify();
            });
            cx.background_executor().timer(SETTLE).await;
            let _ = this.update(cx, |this, cx| this.read_pill_memory(cx));
        })
        .detach();
    }

    /// The choice "Keep the models in memory", and the memory line under it.
    pub(super) fn keep_rows(&self, cx: &mut Context<Self>) -> Div {
        let choice = match &self.keep.keep {
            Load::Loading => div()
                .text_size(size::sm())
                .text_color(text::secondary())
                .child("Reading…"),
            Load::Failed(e) => div()
                .text_size(size::sm())
                .text_color(Status::Danger.color())
                .child(format!("The keep time was not read: {e}")),
            Load::Ready(current) => {
                let current = *current;
                div()
                    .debug_selector(|| "voice-models-keep".into())
                    .flex()
                    .flex_wrap()
                    .items_center()
                    .gap(space(2))
                    .children(KeepTime::ALL.into_iter().map(|keep| {
                        let button = Button::new(("voice-models-keep", keep as usize))
                            .small()
                            .label(label(keep))
                            .on_click(cx.listener(move |this, _, _, cx| this.set_keep(keep, cx)));
                        if keep == current {
                            button.primary()
                        } else {
                            button.secondary()
                        }
                    }))
            }
        };
        let memory = self.keep.pill_kb.map(memory_line);
        div()
            .flex()
            .flex_col()
            .gap(space(1))
            .child(pair("Keep the models in memory", choice))
            .child(
                div()
                    .text_size(size::sm())
                    .text_color(text::secondary())
                    .child(
                        "A model loads at its first use and drops after this time with no use. \
                         Load at start loads the models when the pill starts and keeps them.",
                    ),
            )
            .when_some(memory, |el, line| {
                el.child(
                    div()
                        .debug_selector(|| "voice-models-memory".into())
                        .text_size(size::sm())
                        .text_color(text::secondary())
                        .child(line),
                )
            })
            .when_some(self.keep.error.clone(), |el, e| {
                el.child(
                    div()
                        .text_size(size::sm())
                        .text_color(Status::Danger.color())
                        .child(format!("The keep time was not saved: {e}")),
                )
            })
    }
}

#[cfg(test)]
mod tests {
    use super::{KeepTime, label, memory_line};
    use crate::screens::parts::Load;
    use crate::screens::settings::Section;
    use crate::screens::settings::tests::support::setup;
    use gpui::{Modifiers, TestAppContext};
    use std::io::Read as _;

    /// What the app wrote to the core within a second.
    fn sent(core_end: &mut std::os::unix::net::UnixStream, want: &str) -> String {
        core_end.set_nonblocking(true).unwrap();
        let mut text = String::new();
        for _ in 0..50 {
            let mut buf = [0u8; 4096];
            while let Ok(n) = core_end.read(&mut buf) {
                if n == 0 {
                    break;
                }
                text.push_str(&String::from_utf8_lossy(&buf[..n]));
            }
            if text.contains(want) {
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(20));
        }
        text
    }

    #[gpui::test]
    fn the_section_shows_the_choice_and_the_memory_and_a_click_asks_the_core(
        cx: &mut TestAppContext,
    ) {
        let (mut t, cx) = setup(cx, None);
        t.open(cx, Section::VoiceModels);
        t.screen.update(cx, |s, cx| {
            s.keep.keep = Load::Ready(KeepTime::FiveMinutes);
            s.keep.pill_kb = Some(Some(143_360));
            cx.notify();
        });
        cx.update(|window, cx| window.draw(cx).clear(cx));
        assert!(cx.debug_bounds("voice-models-keep").is_some());
        assert!(cx.debug_bounds("voice-models-memory").is_some());
        for id in [
            "button:voice-models-keep-0",
            "button:voice-models-keep-1",
            "button:voice-models-keep-2",
            "button:voice-models-keep-3",
            "button:voice-models-keep-4",
            "button:voice-models-keep-5",
        ] {
            assert!(cx.debug_bounds(id).is_some(), "{id}");
        }
        let hour = cx
            .debug_bounds("button:voice-models-keep-3")
            .expect("the button 1 hour");
        cx.simulate_click(hour.center(), Modifiers::none());
        let text = sent(&mut t.core_end, "voice.models.keep");
        assert!(
            text.contains(r#""method":"voice.models.keep","params":{"keep":"1h"}"#),
            "{text}"
        );
    }

    #[test]
    fn the_memory_line_gives_whole_mb() {
        assert_eq!(
            memory_line(Some(143_360)),
            "The pill uses 140 MB of memory now."
        );
        assert_eq!(memory_line(None), "The pill does not run.");
    }

    #[test]
    fn each_keep_time_has_a_label() {
        let labels: Vec<_> = KeepTime::ALL.into_iter().map(label).collect();
        assert_eq!(
            labels,
            [
                "1 minute",
                "5 minutes",
                "15 minutes",
                "1 hour",
                "Always",
                "Load at start"
            ]
        );
    }
}
