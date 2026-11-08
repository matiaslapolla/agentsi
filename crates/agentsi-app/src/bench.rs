//! The bench modes of the app, on top of `agentsi_ui::bench`.
//!
//! A bench run uses no layout file. It opens its own panes in a grid, on the
//! running core, and kills their terminals when it quits.
//!
//! - `--screen desk|minutes|settings` shows that screen (of the first product) in place of panes, in any
//!   bench: `--bench idle --screen desk` counts the frames of an idle front desk.
//! - `--bench scroll`: the minutes list of the first product. The core needs a product with
//!   about 1,000 minutes, and the bench scrolls the list at `--speed` px per second.
//! - `--bench idle`: one idle `sh` pane. `--panes 8 --program "<sh command>"` with
//!   `--seconds 60` gives the RSS with 8 panes.
//! - `--bench terminal`: one pane with the writer of the terminal bench.
//! - `--bench attention`: one pane of the first product, whose program turns `blocked`
//!   and `idle` on a line of input. The bench sends the line and takes the time to the
//!   paint of both markers: on the pane and on the product.
//! - `--bench start --runs 10`: the normal start, with the layout file read only.
//! - `--size WxH` fixes the area of the panes, so that runs on different monitors compare.

use std::cell::RefCell;
use std::rc::Rc;
use std::time::{Duration, Instant};

use agentsi_proto::terminal_api::{Create, CreateParams, Input, InputParams, Kill, TerminalId};
use agentsi_ui::bench::{Bench, BenchKind};
use gpui_kit::component::dock::{DockLayout, panel_handle};
use gpui_kit::*;

use crate::pane::{AttentionProbe, TerminalPanel};
use crate::screens::{ScreenKind, ScreenSpec};
use crate::shell::Shell;

const WRITER: &str = include_str!("../../agentsi-term-view/bench/terminal.sh");
/// Prints an OSC 7501 `blocked` record on the line `b`, and `idle` on each other line.
const ATTENTION: &str = r#"stty -echo; while read l; do if [ "$l" = b ]; then printf '\033]7501;state=blocked:kind=permission:msg=YmVuY2g=\007'; else printf '\033]7501;state=idle\007'; fi; done"#;
/// One `blocked` and one `idle` in each cycle of the attention bench.
const CYCLE: Duration = Duration::from_millis(400);
/// The time before the first cycle: the pane attaches and the first paint ends.
const SETTLE: Duration = Duration::from_millis(800);

/// The bench of this run, for a screen that the bench drives: the minutes list in the scroll bench.
pub struct BenchHandle(pub Bench);

impl Global for BenchHandle {}

#[derive(Clone)]
pub struct BenchPlan {
    /// A bench that shows a screen of the first product in place of panes.
    screen: Option<ScreenKind>,
    pub panes: usize,
    /// `--size WxH`: the size in pixels of the area of the panes. Without it, the
    /// panes fill the window, whose size the window manager decides.
    pub size: Option<(f32, f32)>,
    pub argv: Vec<String>,
    pub probe: Option<AttentionProbe>,
    /// The terminals that the bench made, to kill at the end.
    pub made: Rc<RefCell<Vec<String>>>,
}

impl BenchPlan {
    /// The plan of a bench, or `None` for the start bench, which uses the normal start.
    pub fn new(bench: &Bench, args: &[String]) -> Option<BenchPlan> {
        let kind = bench.kind();
        if kind == BenchKind::Start {
            return None;
        }
        let flag = |name: &str| {
            args.iter()
                .position(|a| a == name)
                .and_then(|i| args.get(i + 1))
                .cloned()
        };
        let panes = flag("--panes")
            .and_then(|n| n.parse().ok())
            .unwrap_or(1usize)
            .clamp(1, 16);
        let script = match (flag("--program"), kind) {
            (Some(p), _) => p,
            (None, BenchKind::Terminal) => WRITER.to_string(),
            (None, BenchKind::Attention) => ATTENTION.to_string(),
            (None, _) => "exec sh".to_string(),
        };
        let size = flag("--size").and_then(|s| {
            let (w, h) = s.split_once('x')?;
            Some((w.parse().ok()?, h.parse().ok()?))
        });
        Some(BenchPlan {
            screen: flag("--screen")
                // `minute` needs a minutes id, which a bench does not have.
                .and_then(|n| ScreenKind::parse(&n))
                .filter(|k| *k != ScreenKind::Minute)
                .or((kind == BenchKind::Scroll).then_some(ScreenKind::Minutes)),
            panes,
            size,
            argv: vec!["sh".into(), "-c".into(), script],
            probe: (kind == BenchKind::Attention).then(AttentionProbe::default),
            made: Rc::default(),
        })
    }

    /// Makes the terminals, lays out their panes in a grid, and starts the attention driver.
    pub fn run(&self, shell: &mut Shell, window: &mut Window, cx: &mut Context<Shell>) {
        if self.screen == Some(ScreenKind::Settings) {
            shell.show_screen(ScreenSpec::settings(), window, cx);
            return;
        }
        if let Some(kind) = self.screen {
            let product = shell.core().read(cx).products.first().map(|p| p.id.clone());
            match product {
                Some(product) => shell.show_screen(
                    ScreenSpec {
                        kind,
                        host: None,
                        product,
                        arg: None,
                    },
                    window,
                    cx,
                ),
                None => eprintln!("agentsi-app bench: the core has no product to show"),
            }
            return;
        }
        let link = shell.core().read(cx).link.clone();
        let cwd = std::env::temp_dir().display().to_string();
        // A pane of the first product, so the attention bench also times the product marker.
        let product = shell.core().read(cx).products.first().map(|p| p.id.clone());
        let created: Vec<_> = (0..self.panes)
            .map(|_| {
                link.call::<Create>(&CreateParams {
                    cwd: cwd.clone(),
                    argv: self.argv.clone(),
                    env: Default::default(),
                    product: product.clone(),
                    cols: None,
                    rows: None,
                })
            })
            .collect();
        let plan = self.clone();
        cx.spawn_in(window, async move |this, cx| {
            let mut ids = Vec::new();
            for c in created {
                match c.await {
                    Ok(info) => ids.push(info),
                    Err(e) => eprintln!("agentsi-app bench: terminal.create: {e}"),
                }
            }
            plan.made
                .borrow_mut()
                .extend(ids.iter().map(|t| t.id.clone()));
            let first = ids.first().map(|t| t.id.clone());
            let _ = this.update_in(cx, |shell, window, cx| {
                let core = shell.core().clone();
                // Only the terminal bench takes the read-to-paint samples of the pane.
                let bench = shell
                    .bench()
                    .cloned()
                    .filter(|b| b.kind() == BenchKind::Terminal);
                let mut panels = Vec::new();
                for info in ids {
                    let id = info.id.clone();
                    core.update(cx, |s, _| s.put(info));
                    let (c, b, p) = (core.clone(), bench.clone(), plan.probe.clone());
                    let panel = cx.new(|cx| TerminalPanel::new(id, c, b, p, window, cx));
                    shell.open().borrow_mut().terminals.push(panel.downgrade());
                    panels.push(panel);
                }
                let grid = grid(&panels, cx);
                shell
                    .dock()
                    .update(cx, |d, cx| d.set_center(grid, window, cx));
                if let Some(p) = panels.first() {
                    window.focus(&p.focus_handle(cx), cx);
                }
            });
            if let (Some(id), Some(probe), Some(bench)) = (
                first,
                plan.probe.clone(),
                this.update(cx, |s, _| s.bench().cloned()).ok().flatten(),
            ) {
                drive_attention(id, probe, bench, product.is_some(), link, cx).await;
            }
        })
        .detach();
    }

    /// Kills the terminals of the bench. Each call is written before this returns.
    pub fn finish(&self, link: &crate::link::Link) {
        for id in self.made.borrow().iter() {
            drop(link.call::<Kill>(&TerminalId { id: id.clone() }));
        }
    }
}

/// The panes in rows of at most 4.
fn grid(panels: &[Entity<TerminalPanel>], cx: &App) -> DockLayout {
    let mut rows = DockLayout::v_split();
    for row in panels.chunks(4) {
        let mut line = DockLayout::h_split();
        for p in row {
            line = line.child(
                DockLayout::tabs().panel_view(panel_handle(p.clone()), cx),
                None,
            );
        }
        rows = rows.child(line, None);
    }
    rows
}

/// Each cycle sends `b` (the program turns `blocked`), then a blank line (`idle`).
/// The probe records the time from the send of `b` to the paint of the marker.
async fn drive_attention(
    id: String,
    probe: AttentionProbe,
    bench: Bench,
    with_product: bool,
    link: std::sync::Arc<crate::link::Link>,
    cx: &mut AsyncWindowContext,
) {
    cx.background_executor().timer(SETTLE).await;
    loop {
        probe.arm(Instant::now(), bench.clone(), with_product);
        let sent = link.call::<Input>(&InputParams {
            id: id.clone(),
            text: "b\n".into(),
            submit: false,
        });
        if sent.await.is_err() {
            return;
        }
        cx.background_executor().timer(CYCLE / 2).await;
        let sent = link.call::<Input>(&InputParams {
            id: id.clone(),
            text: "\n".into(),
            submit: false,
        });
        if sent.await.is_err() {
            return;
        }
        cx.background_executor().timer(CYCLE / 2).await;
    }
}
