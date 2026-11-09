//! The Guide line: a button that opens the Guide screen.

use agentsi_ui::components::Button;
use agentsi_ui::tokens::{size, space, text};
use gpui_kit::*;

use super::SettingsScreen;
use crate::nav::{Nav, ScreenRequest};
use crate::screens::parts::section;

impl SettingsScreen {
    pub(super) fn guide_section(&self, cx: &mut Context<Self>) -> Div {
        div()
            .flex()
            .flex_col()
            .gap(space(3))
            .child(section("Guide", None))
            .child(
                div()
                    .flex()
                    .items_center()
                    .justify_between()
                    .gap(space(3))
                    .child(
                        div()
                            .text_size(size::sm())
                            .text_color(text::secondary())
                            .child("How to install, start and use AgentSI. It opens in a tab."),
                    )
                    .child(
                        Button::new("settings-guide")
                            .secondary()
                            .small()
                            .label("Open")
                            .on_click(cx.listener(|this, _, _, cx| {
                                Nav::open(cx, &this.core, ScreenRequest::guide())
                            })),
                    ),
            )
    }
}

#[cfg(test)]
mod tests {
    use gpui::{Modifiers, TestAppContext};

    use crate::nav::{Nav, ScreenRequest};
    use crate::screens::settings::tests::support::setup;

    #[gpui::test]
    fn the_open_button_of_the_guide_line_asks_the_shell_for_the_guide_screen(
        cx: &mut TestAppContext,
    ) {
        let (nav, mut requests) = Nav::channel();
        cx.update(|cx| cx.set_global(nav));
        let (_t, cx) = setup(cx, None);
        // A tall window draws the whole screen, so the Guide line is in view.
        cx.simulate_resize(gpui::size(
            agentsi_ui::tokens::control::WINDOW_WIDTH,
            gpui::Pixels::from(8000.),
        ));
        cx.update(|window, cx| window.draw(cx).clear(cx));
        let button = cx
            .debug_bounds("button:settings-guide")
            .expect("the Guide line is drawn");
        cx.simulate_click(button.center(), Modifiers::none());
        assert_eq!(requests.try_recv().unwrap(), ScreenRequest::guide());
        assert_eq!(cx.opened_url(), None, "the browser stays closed");
    }
}
