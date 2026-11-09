//! The gallery of `agentsi-ui`: each token, and each component in each state.
//!
//! `cargo run -p agentsi-ui --example gallery`
//! `cargo run -p agentsi-ui --example gallery --release -- --bench idle --seconds 10 --json`
//!
//! `AGENTSI_LIFETIME=<s>` closes the window after that many seconds.
use std::time::{Duration, Instant};

use agentsi_ui::bench::Bench;
use agentsi_ui::components::{
    AttentionMarker, Button, Card, Force, GroupHeader, ItemRow, ListRow, ProductRow, StatusDot,
    Switch, TabChip, key_hint, neutral_tag, running_chip, status_tag,
};
use agentsi_ui::tokens::label::Hue;
use agentsi_ui::tokens::{
    self, Color, Status, accent, control, font, line, radius, size, space, surface, text, weight,
};
use gpui_kit::component::IconName;
use gpui_kit::component::input::{Input, InputState};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;

const MINUTES: usize = 1000;

/// The root view. It only lays out the 3 child views, so a frame of one child is cheap.
struct Gallery {
    bench: Option<Bench>,
    states: Entity<StatesPanel>,
    live: Entity<LivePanel>,
    minutes: Entity<MinutesList>,
}

/// The tokens and the grid of states. It never changes.
struct StatesPanel;

/// Components that take input.
struct LivePanel {
    voice_on: bool,
    input: Entity<InputState>,
}

/// 1,000 minutes rows. The scroll bench moves this list.
struct MinutesList {
    bench: Option<Bench>,
    selected: usize,
    list: UniformListScrollHandle,
}

/// The states of the grid, as columns.
const STATES: [(&str, Force, bool); 5] = [
    ("Default", Force::None, false),
    ("Hover", Force::Hover, false),
    ("Pressed", Force::Pressed, false),
    ("Focused", Force::Focused, false),
    ("Disabled", Force::None, true),
];

fn label(s: impl Into<SharedString>) -> Div {
    div()
        .text_size(size::xs())
        .font_weight(weight::MEDIUM)
        .text_color(text::tertiary().hsla())
        .child(s.into())
}

fn section(title: &'static str) -> Div {
    div().flex().flex_col().gap(space(3)).child(
        div()
            .text_size(size::md())
            .font_weight(weight::SEMIBOLD)
            .text_color(text::primary().hsla())
            .child(title),
    )
}

fn swatch(name: &'static str, c: Color) -> Div {
    div()
        .flex()
        .flex_col()
        .gap(space(1))
        .w(px(104.))
        .child(
            div()
                .h(space(8))
                .rounded(radius::md())
                .border_1()
                .border_color(line::border().hsla())
                .bg(c.hsla()),
        )
        .child(
            div()
                .text_size(size::xs())
                .text_color(text::secondary().hsla())
                .child(name),
        )
        .child(
            div()
                .text_size(size::xs())
                .font_family(font::MONO)
                .text_color(text::tertiary().hsla())
                .child(format!("#{:06x}", c.0)),
        )
}

fn minute(i: usize) -> (String, String, Status, &'static str) {
    const SUBJECTS: [&str; 5] = [
        "Ranked priorities and disagreements",
        "Decisions to confirm",
        "Voice mode and the call script",
        "Stories and their criteria",
        "Open tests of the desktop app",
    ];
    let day = 2 + (MINUTES - i) % 27;
    let (status, word) = match i % 3 {
        0 => (Status::Ok, "accepted"),
        1 => (Status::Warn, "pending"),
        _ => (Status::Off, "rejected"),
    };
    (
        format!(
            "Meeting {:04}: {}",
            MINUTES - i,
            SUBJECTS[i % SUBJECTS.len()]
        ),
        format!(
            "2026-10-{day:02} · {} priorities · {} decision{}",
            3 + i % 4,
            i % 3,
            if i % 3 == 1 { "" } else { "s" }
        ),
        status,
        word,
    )
}

impl StatesPanel {
    fn states_grid(&self) -> Div {
        let head = div()
            .flex()
            .gap(space(4))
            .child(div().w(px(110.)))
            .children(
                STATES
                    .iter()
                    .map(|(name, _, _)| div().w(px(136.)).child(label(*name))),
            );
        let row = |name: &'static str, make: &dyn Fn(usize, Force, bool) -> AnyElement| {
            div()
                .flex()
                .items_center()
                .gap(space(4))
                .min_h(px(40.))
                .child(
                    div()
                        .w(px(110.))
                        .text_size(size::sm())
                        .text_color(text::secondary().hsla())
                        .child(name),
                )
                .children(
                    STATES
                        .iter()
                        .enumerate()
                        .map(|(i, (_, f, d))| div().w(px(136.)).child(make(i, *f, *d))),
                )
        };
        div()
            .flex()
            .flex_col()
            .gap(space(2))
            .child(head)
            .child(row("Primary", &|i, f, d| {
                Button::new(("g-pri", i))
                    .label("Approve")
                    .icon(IconName::Check)
                    .force(f)
                    .disabled(d)
                    .into_any_element()
            }))
            .child(row("Secondary", &|i, f, d| {
                Button::new(("g-sec", i))
                    .secondary()
                    .label("Open")
                    .force(f)
                    .disabled(d)
                    .into_any_element()
            }))
            .child(row("Ghost", &|i, f, d| {
                Button::new(("g-gho", i))
                    .ghost()
                    .label("Skip")
                    .force(f)
                    .disabled(d)
                    .into_any_element()
            }))
            .child(row("Danger", &|i, f, d| {
                Button::new(("g-dan", i))
                    .danger()
                    .label("Reject")
                    .icon(IconName::Close)
                    .force(f)
                    .disabled(d)
                    .into_any_element()
            }))
            .child(row("Small, icon", &|i, f, d| {
                div()
                    .flex()
                    .gap(space(2))
                    .child(
                        Button::new(("g-sm", i))
                            .secondary()
                            .small()
                            .label("Run")
                            .force(f)
                            .disabled(d),
                    )
                    .child(
                        Button::new(("g-ico", i))
                            .ghost()
                            .icon(IconName::Settings)
                            .force(f)
                            .disabled(d),
                    )
                    .into_any_element()
            }))
            .child(row("Switch", &|i, f, d| {
                div()
                    .flex()
                    .gap(space(3))
                    .child(
                        Switch::new(("g-sw-on", i))
                            .checked(true)
                            .force(f)
                            .disabled(d),
                    )
                    .child(
                        Switch::new(("g-sw-off", i))
                            .checked(false)
                            .force(f)
                            .disabled(d),
                    )
                    .into_any_element()
            }))
            .child(row("Card", &|i, f, d| {
                Card::new(("g-card", i))
                    .on_click(|_, _, _| {})
                    .force(f)
                    .disabled(d)
                    .child(
                        div()
                            .text_size(size::sm())
                            .font_weight(weight::MEDIUM)
                            .child("Story · dev"),
                    )
                    .into_any_element()
            }))
            .child(row("List row", &|i, f, d| {
                ListRow::new(("g-row", i), "Decisions to confirm")
                    .meta("3 priorities")
                    .selected(i == 0)
                    .force(f)
                    .disabled(d)
                    .into_any_element()
            }))
            .child(row("Product row", &|i, f, _| {
                ProductRow::new(("g-prow", i), "agentsi")
                    .label(Some("api".into()))
                    .hue(Some(Hue::Teal))
                    .meta("~/dev/agentsi")
                    .open(i == 0)
                    .selected(i == 0)
                    .trailing(running_chip(3))
                    .force(f)
                    .into_any_element()
            }))
            .child(row("Item row", &|i, f, _| {
                ItemRow::new(("g-irow", i), "claude")
                    .leading(StatusDot(Status::Info))
                    .meta("working")
                    .hue(Some(Hue::Pink))
                    .force(f)
                    .into_any_element()
            }))
            .child(row("Group header", &|i, f, _| {
                GroupHeader::new(("g-group", i), "Work")
                    .hue(Some(Hue::Sky))
                    .collapsed(i == 0)
                    .count(3)
                    .force(f)
                    .into_any_element()
            }))
            .child(row("Tab chip", &|i, f, _| {
                TabChip::new(("g-chip", i), "Build")
                    .hue(Some(Hue::Teal))
                    .collapsed(i == 0)
                    .count(3)
                    .force(f)
                    .into_any_element()
            }))
    }

    fn tokens_panel(&self) -> Div {
        div()
            .flex()
            .flex_col()
            .gap(space(6))
            .child(
                section("Type").child(
                    div()
                        .flex()
                        .flex_col()
                        .gap(space(2))
                        .child(div().text_size(size::xl()).font_weight(weight::SEMIBOLD).child("Front desk 28"))
                        .child(div().text_size(size::lg()).font_weight(weight::SEMIBOLD).child("Pending decisions 20"))
                        .child(div().text_size(size::md()).font_weight(weight::SEMIBOLD).child("Import the Postgres data first 16"))
                        .child(div().text_size(size::base()).text_color(text::secondary().hsla()).child(
                            "Body 14: the core reads SQLite only. The import keeps the row count of each table.",
                        ))
                        .child(div().text_size(size::sm()).text_color(text::secondary().hsla()).child("List 13: 3 priorities, 2 decisions"))
                        .child(div().text_size(size::xs()).text_color(text::tertiary().hsla()).child("Meta 12: docs/specs/agentsi-desktop-ui.md"))
                        .child(
                            div()
                                .font_family(font::MONO)
                                .text_size(size::MONO)
                                .text_color(tokens::terminal::FOREGROUND.hsla())
                                .child("agentsi › claude --model haiku   ▁▂▃▅▇ ┌─┐ ⣿"),
                        ),
                ),
            )
            .child(
                section("Surfaces and text").child(
                    div()
                        .flex()
                        .flex_wrap()
                        .gap(space(3))
                        .child(swatch("BASE", surface::base()))
                        .child(swatch("PANEL", surface::panel()))
                        .child(swatch("RAISED", surface::raised()))
                        .child(swatch("OVERLAY", surface::overlay()))
                        .child(swatch("text PRIMARY", text::primary()))
                        .child(swatch("text SECONDARY", text::secondary()))
                        .child(swatch("text TERTIARY", text::tertiary())),
                ),
            )
            .child(
                section("Accent and status").child(
                    div()
                        .flex()
                        .flex_wrap()
                        .gap(space(3))
                        .child(swatch("accent", accent::base()))
                        .child(swatch("accent HOVER", accent::hover()))
                        .child(swatch("accent SUBTLE", accent::subtle()))
                        .child(swatch("line BORDER", line::border()))
                        .children(Status::ALL.iter().map(|s| swatch(s.label(), s.color()))),
                ),
            )
            .child(
                section("Label colors").child(
                    div()
                        .flex()
                        .flex_wrap()
                        .gap(space(3))
                        .children(Hue::ALL.iter().map(|h| swatch(h.name(), h.color()))),
                ),
            )
    }

    fn status_panel(&self) -> Div {
        section("Status").child(
            div()
                .flex()
                .flex_col()
                .gap(space(3))
                .child(
                    div()
                        .flex()
                        .flex_wrap()
                        .gap(space(2))
                        .child(status_tag("idle", Status::Off))
                        .child(status_tag("working", Status::Info))
                        .child(status_tag("done", Status::Ok))
                        .child(status_tag("blocked", Status::Warn))
                        .child(status_tag("error", Status::Danger))
                        .child(neutral_tag("Decision · PM")),
                )
                .child(
                    div()
                        .flex()
                        .items_center()
                        .gap(space(4))
                        .child(
                            div()
                                .flex()
                                .items_center()
                                .gap(space(2))
                                .child(StatusDot(Status::Ok))
                                .child(div().text_size(size::sm()).child("omarchy")),
                        )
                        .child(
                            div()
                                .flex()
                                .items_center()
                                .gap(space(2))
                                .child(StatusDot(Status::Off))
                                .child(
                                    div()
                                        .text_size(size::sm())
                                        .text_color(text::secondary().hsla())
                                        .child("macbook · device off"),
                                ),
                        )
                        .child(AttentionMarker::new("Claude asks for a permission"))
                        .child(AttentionMarker::new("").compact())
                        .child(key_hint("ctrl-k")),
                ),
        )
    }
}

impl Render for LivePanel {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let voice_on = self.voice_on;
        section("Live").size_full().child(
            div()
                .flex()
                .flex_col()
                .gap(space(3))
                .child(Input::new(&self.input))
                .child(
                    div()
                        .flex()
                        .items_center()
                        .gap(space(3))
                        .child(Button::new("live-pri").label("Convene meeting").icon(IconName::Bot).on_click(|_, _, _| {}))
                        .child(Button::new("live-sec").secondary().label("Search or run").icon(IconName::Search))
                        .child(
                            div()
                                .flex()
                                .items_center()
                                .gap(space(2))
                                .child(Switch::new("live-voice").checked(voice_on).on_click(cx.listener(
                                    |this, on: &bool, _, cx| {
                                        this.voice_on = *on;
                                        cx.notify();
                                    },
                                )))
                                .child(div().text_size(size::sm()).text_color(text::secondary().hsla()).child(if voice_on { "Voice on" } else { "Voice off" })),
                        ),
                )
                .child(
                    Card::new("live-card")
                        .on_click(|_, _, _| {})
                        .child(div().flex().justify_between().child(neutral_tag("Decision · Dev")).child(label("docs/ideas/agentsi-desktop.md")))
                        .child(div().text_size(size::md()).font_weight(weight::SEMIBOLD).child("Import the Postgres data before the switch-over"))
                        .child(div().text_size(size::sm()).text_color(text::secondary().hsla()).child(
                            "The Rust core reads SQLite only. The import must keep the row count of each table.",
                        )),
                ),
        )
    }
}

impl Render for MinutesList {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        if let Some(b) = &self.bench {
            let range = control::row() * (MINUTES as f32) - px(600.);
            if let Some(off) = b.scroll_offset(range) {
                self.list
                    .0
                    .borrow()
                    .base_handle
                    .set_offset(point(px(0.), -off));
                window.request_animation_frame();
            }
        }
        div()
            .size_full()
            .flex()
            .flex_col()
            .rounded(radius::lg())
            .border_1()
            .border_color(line::border().hsla())
            .bg(surface::panel().hsla())
            .overflow_hidden()
            .child(
                div()
                    .flex()
                    .justify_between()
                    .items_center()
                    .px(space(4))
                    .h(px(40.))
                    .border_b_1()
                    .border_color(line::border().hsla())
                    .child(
                        div()
                            .text_size(size::sm())
                            .font_weight(weight::SEMIBOLD)
                            .child("Minutes"),
                    )
                    .child(label(format!("{MINUTES} meetings"))),
            )
            .child(
                uniform_list(
                    "minutes",
                    MINUTES,
                    cx.processor(|this, range: std::ops::Range<usize>, _, cx| {
                        range
                            .map(|i| {
                                let (title, meta, status, word) = minute(i);
                                div().px(space(1)).child(
                                    ListRow::new(("minute", i), title)
                                        .meta(meta)
                                        .selected(this.selected == i)
                                        .trailing(status_tag(word, status))
                                        .on_click(cx.listener(move |this, _, _, cx| {
                                            this.selected = i;
                                            cx.notify();
                                        })),
                                )
                            })
                            .collect::<Vec<_>>()
                    }),
                )
                .track_scroll(&self.list)
                .flex_1()
                .p(space(1)),
            )
    }
}

impl Render for StatesPanel {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        div()
            .id("left")
            .size_full()
            .flex()
            .flex_col()
            .gap(space(8))
            .p(space(8))
            .overflow_y_scroll()
            .child(
                div()
                    .flex()
                    .flex_col()
                    .gap(space(1))
                    .child(div().text_size(size::xl()).font_weight(weight::SEMIBOLD).child("agentsi-ui"))
                    .child(div().text_size(size::base()).text_color(text::secondary().hsla()).child(
                        "Tokens and components of the AgentSI app. Each transition is 120 ms ease-out.",
                    )),
            )
            .child(section("States").child(self.states_grid()))
            .child(self.status_panel())
            .child(self.tokens_panel())
    }
}

impl Render for Gallery {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        let fill = StyleRefinement::default().size_full();
        div()
            .size_full()
            .flex()
            .bg(surface::panel().hsla())
            .text_color(text::primary().hsla())
            .font_family(font::UI)
            .text_size(size::base())
            .when_some(self.bench.as_ref(), |el, b| el.child(b.marker()))
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .h_full()
                    .child(self.states.clone().cached(fill.clone())),
            )
            .child(
                div()
                    .flex()
                    .flex_col()
                    .w(px(460.))
                    .h_full()
                    .flex_none()
                    .gap(space(6))
                    .p(space(6))
                    .bg(surface::base().hsla())
                    .border_l_1()
                    .border_color(line::border().hsla())
                    .child(
                        div()
                            .w_full()
                            .h(px(290.))
                            .flex_none()
                            .child(self.live.clone().cached(fill.clone())),
                    )
                    .child(
                        div()
                            .w_full()
                            .flex_1()
                            .min_h_0()
                            .child(self.minutes.clone().cached(fill)),
                    ),
            )
    }
}

fn main() {
    let main_start = Instant::now();
    let bench = Bench::from_env(main_start);
    let lifetime = std::env::var("AGENTSI_LIFETIME")
        .ok()
        .and_then(|s| s.parse::<u64>().ok());
    gpui_kit::application()
        .with_assets(gpui_kit::assets::Assets)
        .run(move |cx| {
            agentsi_ui::init(cx);
            let opts = WindowOptions {
                window_bounds: Some(WindowBounds::Windowed(Bounds::centered(
                    None,
                    size(px(1440.), px(940.)),
                    cx,
                ))),
                app_id: Some("agentsi-ui-gallery".into()),
                ..Default::default()
            };
            let b = bench.clone();
            gpui_kit::open_window(opts, cx, move |window, cx| {
                if let Some(s) = lifetime {
                    window
                        .spawn(cx, async move |cx| {
                            cx.background_executor().timer(Duration::from_secs(s)).await;
                            _ = cx.update(|_, cx| cx.quit());
                        })
                        .detach();
                }
                let input = cx.new(|cx| {
                    InputState::new(window, cx).placeholder("Type a command or a product…")
                });
                let states = cx.new(|_| StatesPanel);
                let live = cx.new(|_| LivePanel {
                    voice_on: true,
                    input,
                });
                let minutes = cx.new(|_| MinutesList {
                    bench: b.clone(),
                    selected: 0,
                    list: UniformListScrollHandle::new(),
                });
                cx.new(|cx| {
                    if let Some(b) = &b {
                        b.start(window, cx);
                    }
                    Gallery {
                        bench: b,
                        states,
                        live,
                        minutes,
                    }
                })
            })
            .expect("open the gallery window");
            cx.activate(true);
        });
}
