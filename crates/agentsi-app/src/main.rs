//! `agentsi-app`: the window of AgentSI. It is a client of the core of this device.
//!
//! The app connects to the core, or starts one. The core owns the terminals, so they
//! keep running when the window closes. The next start opens the same panes again.

#![cfg_attr(test, recursion_limit = "256")]

mod actions;
mod appearance;
mod atomic;
mod bench;
mod dictation_bind;
mod format;
mod guide;
mod hosts;
mod keys;
mod layout;
mod link;
mod look;
mod menus;
mod model;
mod nav;
mod pane;
mod prompt;
mod screens;
mod shell;
mod sidebar;
mod tab_bar;
mod tabs;
mod tree;

use std::cell::RefCell;
use std::process::ExitCode;
use std::rc::Rc;
use std::time::Instant;

use agentsi_proto::client::{connect_or_start, find_core_bin};
use agentsi_proto::socket::socket_path;
use agentsi_ui::bench::{Bench, BenchKind};
use agentsi_ui::tokens::control;
use gpui_kit::*;

use crate::bench::BenchPlan;
use crate::link::Link;
use crate::model::CoreState;
use crate::shell::{Open, Persist, Shell, register_panels};

fn fail(message: impl std::fmt::Display) -> ExitCode {
    eprintln!("agentsi-app: {message}");
    ExitCode::FAILURE
}

fn main() -> ExitCode {
    let main_start = Instant::now();
    let args: Vec<String> = std::env::args().skip(1).collect();
    if args.first().map(String::as_str) == Some("--version") {
        println!("agentsi-app {}", env!("CARGO_PKG_VERSION"));
        return ExitCode::SUCCESS;
    }
    let bench = Bench::from_env(main_start);
    let socket = match socket_path() {
        Ok(s) => s,
        Err(e) => return fail(e),
    };
    let Some(core_bin) = find_core_bin() else {
        return fail(
            "no agentsi-core program: put it next to agentsi-app, on the PATH, or in AGENTSI_CORE_BIN",
        );
    };
    let stream = match connect_or_start(&socket, &core_bin) {
        Ok(s) => s,
        Err(e) => return fail(format!("no core: {e}")),
    };
    let (link, events) = match Link::start(stream, "agentsi-app", None) {
        Ok(l) => l,
        Err(e) => return fail(format!("the core does not answer: {e}")),
    };
    let persist = match (layout::path(), bench.as_ref().map(Bench::kind)) {
        (Some(p), None) => Persist::File(p),
        (Some(p), Some(BenchKind::Start)) => Persist::ReadOnly(p),
        _ => Persist::None,
    };
    let plan = bench.as_ref().and_then(|b| BenchPlan::new(b, &args));
    // The benches run without the look file of the owner.
    let persist_look = matches!(persist, Persist::File(_));
    let mut events = Some(events);

    gpui_kit::application()
        .with_assets(gpui_kit::assets::Assets)
        .run(move |cx| {
            agentsi_ui::init(cx);
            appearance::install(cx, appearance::path());
            actions::bind_from(cx, keys::path());
            if persist_look {
                look::install(cx, look::path());
            }
            if let Some(env) = screens::settings::DictationGlobal::detect() {
                cx.set_global(env);
            }
            let core = cx.new(|_| CoreState::new(link.clone()));
            let (remote, cores) = hosts::start(core, cx);
            cx.set_global(hosts::HostsGlobal(remote));
            let open = Rc::new(RefCell::new(Open::default()));
            register_panels(&cores, &open, cx);
            let opts = WindowOptions {
                window_bounds: Some(WindowBounds::Windowed(Bounds::centered(
                    None,
                    size(control::WINDOW_WIDTH, control::WINDOW_HEIGHT),
                    cx,
                ))),
                titlebar: Some(TitlebarOptions {
                    title: Some("AgentSI".into()),
                    ..Default::default()
                }),
                app_id: Some("agentsi-app".into()),
                ..Default::default()
            };
            let (b, p, e) = (bench.clone(), plan.clone(), events.take());
            let opened = gpui_kit::open_window(opts, cx, move |window, cx| {
                let events = e.expect("the window opens once");
                cx.new(|cx| Shell::new(cores, events, open, persist, b, p, window, cx))
            });
            let (handle, shell) = match opened {
                Ok(w) => w,
                Err(e) => {
                    eprintln!("agentsi-app: cannot open the window: {e}");
                    cx.quit();
                    return;
                }
            };
            let s = shell.clone();
            let _ = handle.update(cx, |_, window, cx| {
                window.on_window_should_close(cx, move |_, cx| {
                    s.read(cx).before_quit(cx);
                    true
                });
            });
            cx.on_window_closed(|cx, _| cx.quit()).detach();
            let (s, l, p) = (shell.downgrade(), link.clone(), plan.clone());
            cx.on_app_quit(move |cx| {
                hosts::conn::kill_all();
                if let Some(shell) = s.upgrade() {
                    shell.read(cx).before_quit(cx);
                }
                if let Some(p) = &p {
                    p.finish(&l);
                }
                async {}
            })
            .detach();
            cx.activate(true);
        });
    ExitCode::SUCCESS
}
