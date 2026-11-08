//! P4 prototype: the AgentSI front desk on gpui-component (through gpui-kit).
//! Throwaway. It shows the sidebar, the pending decisions, the minutes list (1,000 rows,
//! virtual scroll), a story card, an empty terminal pane and a command palette (Ctrl+K).
//!
//! Env switches, for the evidence runs:
//! - `AGENTSI_OPEN=palette`: open the palette 1 s after start.
//! - `AGENTSI_BENCH=1`: scroll the minutes list at a constant speed and print frame times.
//! - `AGENTSI_LIFETIME=<seconds>`: quit after that many seconds (default 25).
use std::{
    cell::Cell,
    rc::Rc,
    time::{Duration, Instant},
};

use gpui_kit::assets::Assets;
use gpui_kit::component::{
    ActiveTheme as _, Disableable as _, Icon, IconName, Sizable as _, StyledExt as _, Theme,
    ThemeMode, VirtualListScrollHandle, WindowExt as _,
    button::{Button, ButtonVariants as _},
    command::{Command, CommandEntry, CommandGroup, CommandItem, CommandState},
    h_flex,
    kbd::Kbd,
    sidebar::{Sidebar, SidebarFooter, SidebarGroup, SidebarHeader, SidebarMenu, SidebarMenuItem},
    tag::Tag,
    v_flex, v_virtual_list,
};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;

actions!(
    agentsi,
    [
        OpenPalette,
        ConveneMeeting,
        ConsultDev,
        ConsultQa,
        NewStory,
        OpenSettings
    ]
);

const ROWS: usize = 1000;
const ROW_H: f32 = 44.0;

struct Product {
    name: &'static str,
    host: &'static str,
    pending: usize,
    online: bool,
}

const PRODUCTS: [Product; 4] = [
    Product {
        name: "agentsi",
        host: "omarchy",
        pending: 3,
        online: true,
    },
    Product {
        name: "web app",
        host: "omarchy",
        pending: 1,
        online: true,
    },
    Product {
        name: "mobile app",
        host: "macbook",
        pending: 0,
        online: false,
    },
    Product {
        name: "landing page",
        host: "omarchy",
        pending: 0,
        online: true,
    },
];

struct Decision {
    title: &'static str,
    why: &'static str,
    action: &'static str,
    source: &'static str,
    role: &'static str,
}

const DECISIONS: [Decision; 3] = [
    Decision {
        title: "Add the pending-first wizard to `agentsi mark`",
        why: "The owner leaves 4 of 9 priorities unmarked each week. A one-command wizard shows only the pending ones.",
        action: "Dev writes `agentsi mark --pending` and its `--set` batch mode.",
        source: "docs/specs/agentsi-mark-wizard.md",
        role: "PM",
    },
    Decision {
        title: "Import the Postgres data before the switch-over",
        why: "The Rust core reads SQLite only. The import must keep the row count of each table.",
        action: "Dev writes the importer and checks the counts on the 23 stored snapshots.",
        source: "docs/ideas/agentsi-desktop.md",
        role: "Dev",
    },
    Decision {
        title: "Retire the Python voice package after the pill reaches parity",
        why: "The pill already draws as a layer-shell overlay. Dictation and speech out are still open tests.",
        action: "QA writes the parity test for open tests 2 and 3.",
        source: "prototypes/gpui-pill/README.md",
        role: "QA",
    },
];

const CRITERIA: [&str; 3] = [
    "Given 2 unmarked priorities, when the owner runs `agentsi mark --pending --json`, then it prints both, oldest first.",
    "Given a product with nothing pending, when the owner runs `agentsi mark --pending`, then it prints `Nothing pending.`.",
    "Given one unknown minutes id in a batch, when the owner runs `agentsi mark --set`, then nothing is written and it exits 1.",
];

/// The date `i` days before 2026-11-02.
fn days_back(i: usize) -> (i32, u32, u32) {
    // Days from civil, then back to civil (Howard Hinnant's algorithm).
    let z = 20_759 - i as i64 + 719_468; // 2026-11-02 is day 20,759 after 1970-01-01.
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1_460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let m = if mp < 10 { mp + 3 } else { mp - 9 } as u32;
    let y = (yoe + era * 400 + i64::from(m <= 2)) as i32;
    (y, m, d)
}

fn minute_title(i: usize) -> (String, String, &'static str) {
    const SUBJECTS: [&str; 6] = [
        "Meeting: ranked priorities and disagreements",
        "Meeting: decisions to confirm",
        "Meeting: voice mode and the call script",
        "Meeting: stories and their criteria",
        "Meeting: audit of the repaired minutes",
        "Meeting: open tests of the desktop app",
    ];
    const STATES: [&str; 3] = ["accepted", "pending", "rejected"];
    let (y, m, d) = days_back(i);
    let date = format!("{y}-{m:02}-{d:02}");
    let meta = format!("{} priorities, {} decisions", 3 + i % 4, i % 3);
    (
        format!("{date}  {}", SUBJECTS[i % SUBJECTS.len()]),
        meta,
        STATES[i % 3],
    )
}

struct Bench {
    started: Instant,
    last: Instant,
    frames: Vec<f32>,
    /// Time to build the element tree in `render`, in ms. Layout and paint come after it.
    render_ms: Vec<f32>,
    cpu0: Option<f32>,
}

struct FrontDesk {
    minutes_scroll: VirtualListScrollHandle,
    row_sizes: Rc<Vec<Size<Pixels>>>,
    selected: usize,
    palette: Entity<CommandState>,
    last_command: Option<SharedString>,
    bench: Option<Bench>,
    idle_bench: bool,
}

impl FrontDesk {
    fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        let palette = cx.new(|cx| CommandState::new(window, cx));
        let lifetime = std::env::var("AGENTSI_LIFETIME")
            .ok()
            .and_then(|s| s.parse().ok())
            .map(Duration::from_secs)
            .unwrap_or(Duration::from_secs(25));
        let bench = std::env::var("AGENTSI_BENCH").ok().map(|_| Bench {
            started: Instant::now(),
            last: Instant::now(),
            frames: Vec::new(),
            render_ms: Vec::new(),
            cpu0: None,
        });
        // The window must close by itself, also when nothing redraws it.
        cx.spawn_in(window, async move |this, cx| {
            cx.background_executor().timer(lifetime).await;
            _ = this.update_in(cx, |this, _, cx| {
                if let Some(b) = &this.bench {
                    report(b);
                }
                cx.quit();
            });
        })
        .detach();
        if std::env::var("AGENTSI_OPEN").as_deref() == Ok("palette") {
            cx.spawn_in(window, async move |this, cx| {
                cx.background_executor()
                    .timer(Duration::from_millis(1200))
                    .await;
                _ = this.update_in(cx, |this, window, cx| this.open_palette(window, cx));
            })
            .detach();
        }
        FrontDesk {
            minutes_scroll: VirtualListScrollHandle::new(),
            row_sizes: Rc::new(vec![size(px(100.), px(ROW_H)); ROWS]),
            selected: 0,
            palette,
            last_command: None,
            bench,
            idle_bench: std::env::var("AGENTSI_BENCH").as_deref() == Ok("idle"),
        }
    }

    fn open_palette(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let state = self.palette.clone();
        let owner = cx.weak_entity();
        let focus_on_mount = Rc::new(Cell::new(true));
        window.open_dialog(cx, move |dialog, _, _| {
            let state = state.clone();
            let owner = owner.clone();
            let focus_on_mount = focus_on_mount.clone();
            dialog
                .close_button(false)
                .p_0()
                .content(move |content, window, cx| {
                    if focus_on_mount.replace(false) {
                        let state = state.clone();
                        window.defer(cx, move |window, cx| {
                            state.read(cx).focus_handle(cx).focus(window, cx);
                        });
                    }
                    let owner = owner.clone();
                    let command = palette_entries().into_iter().fold(
                        Command::new(&state)
                            .bordered(false)
                            .max_h(px(440.))
                            .rounded(cx.theme().radius_lg)
                            .placeholder("Type a command or a product…")
                            .on_confirm(move |_index, window, cx| {
                                _ = owner.update(cx, |this, cx| {
                                    this.last_command = Some("command run".into());
                                    cx.notify();
                                });
                                window.close_dialog(cx);
                            })
                            .footer(|_, _, cx| {
                                h_flex()
                                    .gap_4()
                                    .px_3()
                                    .py_2()
                                    .border_t_1()
                                    .border_color(cx.theme().border)
                                    .text_xs()
                                    .text_color(cx.theme().muted_foreground)
                                    .child("↑↓  Navigate")
                                    .child("↵  Run")
                                    .child("Esc  Close")
                            }),
                        |c, e| match e {
                            CommandEntry::Item(i) => c.item(i),
                            CommandEntry::Group(g) => c.group(g),
                            CommandEntry::Separator => c.separator(),
                        },
                    );
                    content.child(command)
                })
        });
    }

    fn decision_card(&self, i: usize, d: &'static Decision, cx: &mut Context<Self>) -> AnyElement {
        let theme = cx.theme().clone();
        let theme = &theme;
        let card = v_flex()
            .id(("decision", i))
            .gap_3()
            .p_4()
            .rounded(theme.radius_lg)
            .border_1()
            .border_color(theme.border)
            .bg(theme.secondary)
            .hover(|s| s.border_color(theme.primary.opacity(0.6)))
            .child(
                h_flex()
                    .justify_between()
                    .items_center()
                    .child(
                        Tag::secondary()
                            .outline()
                            .child(format!("Decision · {}", d.role)),
                    )
                    .child(
                        div()
                            .text_xs()
                            .text_color(theme.muted_foreground)
                            .child(d.source),
                    ),
            )
            .child(
                div()
                    .text_base()
                    .font_semibold()
                    .text_color(theme.foreground)
                    .child(d.title),
            )
            .child(
                div()
                    .text_sm()
                    .text_color(theme.muted_foreground)
                    .child(d.why),
            )
            .child(
                h_flex()
                    .gap_2()
                    .items_start()
                    .text_sm()
                    .child(
                        Icon::new(IconName::ArrowRight)
                            .small()
                            .text_color(theme.primary),
                    )
                    .child(div().flex_1().text_color(theme.foreground).child(d.action)),
            )
            .child(
                h_flex()
                    .gap_2()
                    .pt_1()
                    .child(
                        Button::new(("approve", i))
                            .primary()
                            .small()
                            .icon(IconName::Check)
                            .label("Approve")
                            .on_click(cx.listener(move |this, _, window, cx| {
                                this.last_command =
                                    Some(format!("approved decision {}", i + 1).into());
                                window.push_notification(
                                    "Decision approved. Dev starts the action.",
                                    cx,
                                );
                                cx.notify();
                            })),
                    )
                    .child(
                        Button::new(("reject", i))
                            .outline()
                            .small()
                            .icon(IconName::Close)
                            .label("Reject")
                            .on_click(cx.listener(move |this, _, _, cx| {
                                this.last_command =
                                    Some(format!("rejected decision {}", i + 1).into());
                                cx.notify();
                            })),
                    ),
            );
        card.with_animation(
            ("decision-in", i),
            Animation::new(Duration::from_millis(320 + 90 * i as u64))
                .with_easing(gpui::ease_out_quint()),
            |el, t| el.opacity(t).mt(px((1.0 - t) * 12.0)),
        )
        .into_any_element()
    }

    fn story_card(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.theme();
        v_flex()
            .gap_3()
            .p_4()
            .rounded(theme.radius_lg)
            .border_1()
            .border_color(theme.border)
            .bg(theme.secondary)
            .child(
                h_flex()
                    .justify_between()
                    .child(Tag::secondary().outline().child("Story · not implemented"))
                    .child(div().text_xs().text_color(theme.muted_foreground).child("owner role: dev")),
            )
            .child(
                div().text_base().font_semibold().child(
                    "As the owner, I want to mark pending priorities in one command, so that I decide in a minute.",
                ),
            )
            .children(CRITERIA.iter().enumerate().map(|(i, c)| {
                h_flex()
                    .gap_2()
                    .items_start()
                    .text_sm()
                    .text_color(theme.muted_foreground)
                    .child(div().w(px(16.)).flex_shrink_0().text_color(theme.primary).child(format!("{}", i + 1)))
                    .child(div().flex_1().child(*c))
            }))
            .child(
                h_flex().gap_2().pt_1().child(
                    Button::new("run-story")
                        .primary()
                        .small()
                        .icon(IconName::Play)
                        .label("Run story"),
                ),
            )
    }

    fn minutes(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.theme();
        v_flex()
            .flex_1()
            .min_h_0()
            .rounded(theme.radius_lg)
            .border_1()
            .border_color(theme.border)
            .overflow_hidden()
            .child(
                h_flex()
                    .px_4()
                    .py_2()
                    .justify_between()
                    .border_b_1()
                    .border_color(theme.border)
                    .child(div().text_sm().font_semibold().child("Minutes"))
                    .child(
                        div()
                            .text_xs()
                            .text_color(theme.muted_foreground)
                            .child(format!("{ROWS} meetings")),
                    ),
            )
            .child(
                div().flex_1().min_h_0().child(
                    v_virtual_list(
                        cx.entity(),
                        "minutes",
                        self.row_sizes.clone(),
                        |this, range, _, cx| {
                            range.map(|i| this.minute_row(i, cx)).collect::<Vec<_>>()
                        },
                    )
                    .track_scroll(&self.minutes_scroll),
                ),
            )
    }

    fn minute_row(&self, i: usize, cx: &mut Context<Self>) -> AnyElement {
        let theme = cx.theme().clone();
        let (title, meta, state) = minute_title(i);
        let selected = self.selected == i;
        h_flex()
            .id(("minute", i))
            .w_full()
            .h(px(ROW_H))
            .px_4()
            .gap_3()
            .items_center()
            .justify_between()
            .cursor_pointer()
            .when(selected, |el| el.bg(theme.list_active))
            .hover(|s| s.bg(theme.list_hover))
            .on_click(cx.listener(move |this, _, _, cx| {
                this.selected = i;
                cx.notify();
            }))
            .child(
                v_flex()
                    .flex_1()
                    .min_w_0()
                    .overflow_hidden()
                    .child(
                        div()
                            .text_sm()
                            .truncate()
                            .text_color(theme.foreground)
                            .child(title),
                    )
                    .child(
                        div()
                            .text_xs()
                            .text_color(theme.muted_foreground)
                            .child(meta),
                    ),
            )
            .child(div().flex_shrink_0().child(match state {
                "accepted" => Tag::success().outline().child(state),
                "rejected" => Tag::danger().outline().child(state),
                _ => Tag::secondary().outline().child(state),
            }))
            .into_any_element()
    }

    fn terminal(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.theme();
        v_flex()
            .h(px(150.))
            .flex_shrink_0()
            .rounded(theme.radius_lg)
            .border_1()
            .border_color(theme.border)
            .bg(rgb(0x0a0b0e))
            .child(
                h_flex()
                    .px_3()
                    .py_1p5()
                    .gap_2()
                    .items_center()
                    .border_b_1()
                    .border_color(theme.border)
                    .child(
                        Icon::new(IconName::SquareTerminal)
                            .small()
                            .text_color(theme.muted_foreground),
                    )
                    .child(
                        div()
                            .text_xs()
                            .text_color(theme.muted_foreground)
                            .child("Terminal · no session"),
                    ),
            )
            .child(
                v_flex()
                    .p_3()
                    .gap_1()
                    .font_family(theme.mono_font_family.clone())
                    .text_sm()
                    .child(
                        div()
                            .text_color(theme.muted_foreground)
                            .child("Claude Code runs here, in a pane of the app."),
                    )
                    .child(
                        h_flex()
                            .gap_2()
                            .child(div().text_color(theme.primary).child("agentsi ›"))
                            .child(div().w(px(8.)).h(px(16.)).bg(theme.primary).with_animation(
                                "caret",
                                Animation::new(Duration::from_millis(1100)).repeat(),
                                |el, t| el.opacity(if t < 0.5 { 1.0 } else { 0.0 }),
                            )),
                    ),
            )
    }
}

fn palette_entries() -> Vec<CommandEntry> {
    vec![
        CommandGroup::new()
            .label("Agency")
            .item(
                CommandItem::new()
                    .label("Convene a meeting")
                    .icon(IconName::Bot)
                    .keywords(["minutes"]),
            )
            .item(
                CommandItem::new()
                    .label("Consult the dev role")
                    .icon(IconName::Cpu),
            )
            .item(
                CommandItem::new()
                    .label("Consult the QA role")
                    .icon(IconName::TriangleAlert),
            )
            .item(CommandItem::new().label("New story").icon(IconName::Plus))
            .into(),
        CommandEntry::Separator,
        CommandGroup::new()
            .label("Products")
            .item(
                CommandItem::new()
                    .label("Open agentsi")
                    .icon(IconName::Folder),
            )
            .item(
                CommandItem::new()
                    .label("Open web app")
                    .icon(IconName::Folder),
            )
            .item(
                CommandItem::new()
                    .label("Open mobile app (device off)")
                    .icon(IconName::Folder)
                    .disabled(true),
            )
            .into(),
        CommandEntry::Separator,
        CommandGroup::new()
            .label("Settings")
            .item(
                CommandItem::new()
                    .label("Voice mode: on")
                    .icon(IconName::Bell),
            )
            .item(
                CommandItem::new()
                    .label("Open settings")
                    .icon(IconName::Settings),
            )
            .into(),
    ]
}

impl Render for FrontDesk {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let render_start = Instant::now();
        if let Some(b) = &mut self.bench {
            let now = Instant::now();
            if b.started.elapsed() > Duration::from_secs(2) {
                if b.cpu0.is_none() {
                    b.cpu0 = process_cpu_seconds();
                }
                b.frames
                    .push(now.duration_since(b.last).as_secs_f32() * 1000.0);
            }
            b.last = now;
            // 4,000 px per second, up and down over the list.
            let t = b.started.elapsed().as_secs_f32();
            let span = (ROWS as f32 * ROW_H) - 600.0;
            let pos = (t * 4000.0) % (2.0 * span);
            let pos = if pos > span { 2.0 * span - pos } else { pos };
            if !self.idle_bench {
                self.minutes_scroll.set_offset(point(px(0.), px(-pos)));
            }
            window.request_animation_frame();
        }
        let theme = cx.theme().clone();
        let main = v_flex()
            .flex_1()
            .min_w_0()
            .h_full()
            .child(
                h_flex()
                    .h(px(48.))
                    .px_6()
                    .flex_shrink_0()
                    .items_center()
                    .justify_between()
                    .border_b_1()
                    .border_color(theme.border)
                    .child(
                        h_flex()
                            .gap_2()
                            .items_center()
                            .child(div().text_sm().text_color(theme.muted_foreground).child("agentsi"))
                            .child(div().text_sm().text_color(theme.muted_foreground).child("/"))
                            .child(div().text_sm().font_semibold().child("Front desk")),
                    )
                    .child(
                        h_flex()
                            .gap_2()
                            .items_center()
                            .child(
                                Button::new("palette")
                                    .outline()
                                    .small()
                                    .icon(IconName::Search)
                                    .label("Search or run")
                                    .on_click(cx.listener(|this, _, window, cx| this.open_palette(window, cx))),
                            )
                            .child(Kbd::new(Keystroke::parse("ctrl-k").unwrap()))
                            .child(Button::new("meet").primary().small().icon(IconName::Bot).label("Convene meeting")),
                    ),
            )
            .child(
                h_flex()
                    .flex_1()
                    .min_h_0()
                    .gap_6()
                    .p_6()
                    .child(
                        v_flex()
                            .id("left")
                            .flex_1()
                            .min_w_0()
                            .h_full()
                            .gap_4()
                            .overflow_y_scroll()
                            .child(
                                v_flex().gap_1().child(div().text_xl().font_semibold().child("Good morning, Ana")).child(
                                    div()
                                        .text_sm()
                                        .text_color(theme.muted_foreground)
                                        .child("3 decisions wait for your approval. Nothing acts without it."),
                                ),
                            )
                            .children((0..DECISIONS.len()).map(|i| self.decision_card(i, &DECISIONS[i], cx)).collect::<Vec<_>>())
                            .child(self.story_card(cx)),
                    )
                    .child(
                        v_flex()
                            .w(px(460.))
                            .flex_shrink_0()
                            .h_full()
                            .gap_4()
                            .child(self.minutes(cx))
                            .child(self.terminal(cx)),
                    ),
            );

        let sidebar = Sidebar::new("sidebar")
            .w(px(248.))
            .header(
                SidebarHeader::new().child(
                    h_flex()
                        .gap_3()
                        .items_center()
                        .child(
                            div()
                                .size_7()
                                .rounded(px(8.))
                                .bg(theme.primary)
                                .flex()
                                .items_center()
                                .justify_center()
                                .text_color(theme.primary_foreground)
                                .font_bold()
                                .child("A"),
                        )
                        .child(
                            v_flex()
                                .child(div().font_semibold().child("AgentSI"))
                                .child(
                                    div()
                                        .text_xs()
                                        .text_color(theme.muted_foreground)
                                        .child("Ana · 4 products"),
                                ),
                        ),
                ),
            )
            .child(
                SidebarGroup::new("Products").child(SidebarMenu::new().children(
                    PRODUCTS.iter().enumerate().map(|(i, p)| {
                        let item = SidebarMenuItem::new(p.name)
                            .icon(if p.online {
                                IconName::Folder
                            } else {
                                IconName::FolderClosed
                            })
                            .active(i == 0 && true)
                            .disable(!p.online);
                        let (pending, online, host) = (p.pending, p.online, p.host);
                        item.suffix(move |_, cx| {
                            let muted = cx.theme().muted_foreground;
                            if online {
                                h_flex()
                                    .gap_2()
                                    .items_center()
                                    .when(pending > 0, |el| {
                                        el.child(Tag::primary().child(format!("{pending}")))
                                    })
                                    .child(div().text_xs().text_color(muted).child(host))
                                    .into_any_element()
                            } else {
                                Tag::warning()
                                    .outline()
                                    .child("device off")
                                    .into_any_element()
                            }
                        })
                    }),
                )),
            )
            .child(
                SidebarGroup::new("Agency").child(
                    SidebarMenu::new().children([
                        SidebarMenuItem::new("Front desk")
                            .icon(IconName::LayoutDashboard)
                            .active(true),
                        SidebarMenuItem::new("Minutes").icon(IconName::Inbox),
                        SidebarMenuItem::new("Stories").icon(IconName::BookOpen),
                        SidebarMenuItem::new("Roles").icon(IconName::Bot),
                        SidebarMenuItem::new("Settings").icon(IconName::Settings),
                    ]),
                ),
            )
            .footer(
                SidebarFooter::new().child(
                    h_flex()
                        .gap_2()
                        .items_center()
                        .justify_between()
                        .w_full()
                        .child(
                            div()
                                .text_xs()
                                .text_color(theme.muted_foreground)
                                .child(self.last_command.clone().unwrap_or_else(|| "ready".into())),
                        )
                        .child(Kbd::new(Keystroke::parse("ctrl-k").unwrap())),
                ),
            );

        let root = div()
            .id("root")
            .size_full()
            .bg(theme.background)
            .text_color(theme.foreground)
            .key_context("FrontDesk")
            .on_action(
                cx.listener(|this, _: &OpenPalette, window, cx| this.open_palette(window, cx)),
            )
            .child(h_flex().size_full().child(sidebar).child(main));
        if let Some(b) = &mut self.bench
            && b.started.elapsed() > Duration::from_secs(2)
        {
            b.render_ms
                .push(render_start.elapsed().as_secs_f32() * 1000.0);
        }
        root
    }
}

fn report(b: &Bench) {
    let mut f = b.frames.clone();
    if f.is_empty() {
        println!("bench: no frames");
        return;
    }
    f.sort_by(|a, c| a.partial_cmp(c).unwrap());
    let n = f.len();
    let mean = f.iter().sum::<f32>() / n as f32;
    let p = |q: f32| f[((n as f32 - 1.0) * q) as usize];
    let slow = f.iter().filter(|x| **x > 16.7 * 1.5).count();
    let mut r = b.render_ms.clone();
    r.sort_by(|a, c| a.partial_cmp(c).unwrap());
    if !r.is_empty() {
        let m = r.len();
        println!(
            "render() tree build: mean {:.2} ms, p50 {:.2}, p95 {:.2}, max {:.2} ms",
            r.iter().sum::<f32>() / m as f32,
            r[m / 2],
            r[((m - 1) as f32 * 0.95) as usize],
            r[m - 1]
        );
    }
    if let (Some(c0), Some(c1)) = (b.cpu0, process_cpu_seconds()) {
        println!(
            "process CPU during the measured frames: {:.2} s, {:.2} ms per frame (all threads, /proc/self/stat)",
            c1 - c0,
            (c1 - c0) * 1000.0 / n as f32
        );
    }
    println!(
        "bench: {n} frames, mean {mean:.2} ms ({:.0} fps), p50 {:.2}, p95 {:.2}, p99 {:.2}, max {:.2} ms, frames over 25 ms: {slow}",
        1000.0 / mean,
        p(0.5),
        p(0.95),
        p(0.99),
        f[n - 1]
    );
}

/// utime + stime of this process in seconds (Linux, 100 ticks per second).
fn process_cpu_seconds() -> Option<f32> {
    let stat = std::fs::read_to_string("/proc/self/stat").ok()?;
    let rest = stat.rsplit_once(')')?.1;
    let f: Vec<&str> = rest.split_whitespace().collect();
    let ticks = f.get(11)?.parse::<f32>().ok()? + f.get(12)?.parse::<f32>().ok()?;
    Some(ticks / 100.0)
}

fn main() {
    gpui_kit::application().with_assets(Assets).run(|cx| {
        gpui_kit::init(cx);
        Theme::change(ThemeMode::Dark, None, cx);
        Theme::update(cx, |t| {
            // One accent (violet), a near-black surface, a 8 px radius.
            t.background = rgb(0x0e0f13).into();
            t.foreground = rgb(0xe6e7ea).into();
            t.muted_foreground = rgb(0x8b8f98).into();
            t.border = rgb(0x23252c).into();
            t.secondary = rgb(0x15161b).into();
            t.sidebar = rgb(0x0b0c0f).into();
            t.sidebar_border = rgb(0x1b1d23).into();
            t.primary = rgb(0x8b7cf6).into();
            t.primary_hover = rgb(0x9d90f8).into();
            t.primary_active = rgb(0x7a6af0).into();
            t.primary_foreground = rgb(0x0e0f13).into();
            t.button_primary = t.primary;
            t.button_primary_hover = t.primary_hover;
            t.button_primary_active = t.primary_active;
            t.button_primary_foreground = t.primary_foreground;
            t.list_hover = rgb(0x1a1c22).into();
            t.list_active = rgb(0x20222a).into();
            t.overlay = hsla(0., 0., 0., 0.55);
            t.radius = px(8.);
            t.radius_lg = px(12.);
        });
        cx.bind_keys([KeyBinding::new("ctrl-k", OpenPalette, Some("FrontDesk"))]);
        let opts = WindowOptions {
            window_bounds: Some(WindowBounds::Windowed(Bounds::centered(
                None,
                size(px(1360.), px(860.)),
                cx,
            ))),
            app_id: Some("agentsi-ui-kit".into()),
            ..Default::default()
        };
        gpui_kit::open_window(opts, cx, |window, cx| {
            cx.new(|cx| FrontDesk::new(window, cx))
        })
        .expect("open window");
        cx.activate(true);
    });
}
