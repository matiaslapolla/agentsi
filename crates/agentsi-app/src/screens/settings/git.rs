//! The Git section: the shared table of `git.toml`. A product overrides keys on its git screen.

use agentsi_ui::tokens::{size, space, text};
use gpui_kit::*;

use super::SettingsScreen;
use crate::screens::parts::section;

impl SettingsScreen {
    pub(super) fn git_section(&self) -> Div {
        div()
            .flex()
            .flex_col()
            .gap(space(3))
            .child(section("Git", None))
            .child(
                div()
                    .text_size(size::sm())
                    .text_color(text::secondary())
                    .child("Git config that AgentSI gives to each git command of each product. A product overrides a key on its git screen. The files .git/config and ~/.gitconfig stay as they are."),
            )
            .child(self.git.clone())
    }
}
