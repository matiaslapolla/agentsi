//! Open test 1 of docs/ideas/agentsi-desktop.md: can gpui-pre draw the voice pill as a
//! click-through-free, focus-free overlay at the top of the screen on Wayland?
//! Throwaway: it shows the "preparing" wave for 2 s, then "playing" bars for 4 s, then exits.
use std::time::{Duration, Instant};

#[cfg(target_os = "linux")]
use gpui::layer_shell::*;
use gpui::{
    App, Bounds, Context, MouseButton, Size, Window, WindowBackgroundAppearance, WindowBounds, WindowKind,
    WindowOptions, div, point, prelude::*, px, rgb, rgba,
};
use gpui_platform::application;

const BARS: usize = 9;

struct Pill {
    started: Instant,
}

impl Pill {
    fn new(cx: &mut Context<Self>) -> Self {
        cx.spawn(async move |this, cx| {
            loop {
                if this.update(cx, |_, cx| cx.notify()).is_err() {
                    break;
                }
                cx.background_executor().timer(Duration::from_millis(33)).await;
            }
        })
        .detach();
        Pill { started: Instant::now() }
    }
}

impl Render for Pill {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let t = self.started.elapsed().as_secs_f32();
        if t > 6.0 {
            cx.quit();
        }
        let playing = t > 2.0;
        let bars = (0..BARS).map(|i| {
            let i = i as f32;
            let level = if playing {
                // A fake speech envelope: loud and quiet syllables at about 4 per second.
                let syllable = ((t * 4.0).sin() * 0.5 + 0.5) * (0.6 + 0.4 * (i * 1.7).sin().abs());
                6.0 + 24.0 * syllable
            } else {
                6.0 + 10.0 * (0.5 + 0.5 * (t * 5.5 + i * 0.8).sin())
            };
            div()
                .w(px(6.))
                .h(px(level))
                .rounded(px(3.))
                .bg(rgb(0xcba6f7))
                .opacity(if playing { 1.0 } else { 0.6 })
        });
        div()
            .size_full()
            .flex()
            .items_center()
            .justify_center()
            .gap(px(4.))
            .rounded(px(20.))
            .bg(rgba(0x1e1e2ee6))
            .border_1()
            .border_color(rgb(0x45475a))
            .children(bars)
            .on_mouse_down(MouseButton::Left, |_, _, cx| cx.quit())
    }
}

/// Wayland: a layer-shell surface on the overlay layer, with no keyboard focus.
#[cfg(target_os = "linux")]
fn pill_kind() -> WindowKind {
    WindowKind::LayerShell(LayerShellOptions {
        namespace: "agentsi-pill".to_string(),
        layer: Layer::Overlay,
        anchor: Anchor::TOP,
        margin: Some((px(47.), px(0.), px(0.), px(0.))),
        keyboard_interactivity: KeyboardInteractivity::None,
        ..Default::default()
    })
}

/// macOS: GPUI makes a PopUp a nonactivating NSPanel at NSPopUpWindowLevel, as Hex does for its pill.
#[cfg(not(target_os = "linux"))]
fn pill_kind() -> WindowKind {
    WindowKind::PopUp
}

fn main() {
    application().run(|cx: &mut App| {
        cx.open_window(
            WindowOptions {
                titlebar: None,
                window_bounds: Some(WindowBounds::Windowed(Bounds {
                    origin: point(px(0.), px(0.)),
                    size: Size::new(px(168.), px(40.)),
                })),
                app_id: Some("agentsi-pill".to_string()),
                window_background: WindowBackgroundAppearance::Transparent,
                kind: pill_kind(),
                focus: false,
                ..Default::default()
            },
            |_, cx| cx.new(Pill::new),
        )
        .expect("the compositor must support zwlr_layer_shell_v1");
    });
}
