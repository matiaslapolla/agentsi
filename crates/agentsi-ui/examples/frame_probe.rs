//! Gate probe for U2: counts the frames that GPUI draws, each second.
//!
//! `AGENTSI_PROBE=plain|caret|raf` picks the content: static text, a repeating
//! caret animation, or a `request_animation_frame` call in each render.
//! `AGENTSI_LIFETIME=<s>` quits after that many seconds (default 20).
use std::{cell::Cell, rc::Rc, time::Duration};

use gpui_kit::*;

struct Probe {
    mode: String,
    frames: Rc<Cell<u64>>,
    renders: u64,
}

impl Render for Probe {
    fn render(&mut self, window: &mut Window, _cx: &mut Context<Self>) -> impl IntoElement {
        self.renders += 1;
        if self.mode == "raf" {
            window.request_animation_frame();
        }
        let frames = self.frames.clone();
        let counter = canvas(|_, _, _| (), move |_, _, _, _| frames.set(frames.get() + 1)).size_0();
        let mut body = div()
            .size_full()
            .bg(rgb(0x0e0f13))
            .text_color(rgb(0xe6e7ea))
            .p_6()
            .gap_2()
            .flex()
            .flex_col()
            .child(counter)
            .child(format!("frame probe: {}", self.mode));
        if self.mode == "caret" {
            body = body.child(div().w(px(8.)).h(px(16.)).bg(rgb(0x8b7cf6)).with_animation(
                "caret",
                Animation::new(Duration::from_millis(1100)).repeat(),
                |el, t| el.opacity(if t < 0.5 { 1.0 } else { 0.0 }),
            ));
        }
        body
    }
}

fn main() {
    let mode = std::env::var("AGENTSI_PROBE").unwrap_or_else(|_| "plain".into());
    let lifetime: u64 = std::env::var("AGENTSI_LIFETIME")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(20);
    gpui_kit::application().run(move |cx| {
        let opts = WindowOptions {
            window_bounds: Some(WindowBounds::Windowed(Bounds::centered(None, size(px(480.), px(300.)), cx))),
            app_id: Some("agentsi-frame-probe".into()),
            ..Default::default()
        };
        let frames = Rc::new(Cell::new(0));
        let f2 = frames.clone();
        gpui_kit::open_window(opts, cx, move |window, cx| {
            let view = cx.new(|_| Probe { mode, frames: f2, renders: 0 });
            let weak = view.downgrade();
            window
                .spawn(cx, async move |cx| {
                    let mut last = 0;
                    for t in 1..=lifetime {
                        cx.background_executor().timer(Duration::from_secs(1)).await;
                        let now = frames.get();
                        let (active, renders) = cx
                            .update(|window, cx| (window.is_window_active(), weak.upgrade().map(|v| v.read(cx).renders).unwrap_or(0)))
                            .unwrap_or((false, 0));
                        println!("t={t:>2}s frames={:>4} total={now:>5} renders={renders:>5} active={active}", now - last);
                        last = now;
                    }
                    _ = cx.update(|_, cx| cx.quit());
                })
                .detach();
            view
        })
        .expect("open window");
        cx.activate(true);
    });
}
