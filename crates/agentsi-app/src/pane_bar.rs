//! The pane bar: the row at the top of the window. It shows the app icon and name, one toggle
//! for each pane, and the palette hint. A bright toggle is an open pane, a muted one a hidden
//! pane. The Alerts toggle shows the number of unseen alerts, open or hidden.

use std::rc::Rc;
use std::sync::{Arc, LazyLock};

use agentsi_ui::components::PaneToggle;
use agentsi_ui::style::StyleExt as _;
use agentsi_ui::tokens::{control, line, size, space, surface, text, weight};
use gpui_kit::component::IconName;
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;

use crate::keys;
use crate::panes::{PaneKind, Panes};

/// The icon of the desktop entry. `img` renders an SVG in its own colors.
static APP_ICON: LazyLock<Arc<Image>> = LazyLock::new(|| {
    Arc::new(Image::from_bytes(
        ImageFormat::Svg,
        include_bytes!("../../../packaging/icons/agentsi.svg").to_vec(),
    ))
});

fn icon(kind: PaneKind) -> IconName {
    match kind {
        PaneKind::Products => IconName::LayoutDashboard,
        PaneKind::Files => IconName::Folder,
        PaneKind::Alerts => IconName::Bell,
    }
}

/// What a click on a toggle does.
pub type OnToggle = Rc<dyn Fn(PaneKind, &mut Window, &mut App)>;

/// The bar for `panes`, with `unseen` alerts.
pub fn pane_bar(panes: &Panes, unseen: usize, on_toggle: OnToggle, cx: &App) -> Div {
    let toggles = PaneKind::ALL.into_iter().map(|kind| {
        let open = panes.is_open(kind);
        let tooltip = format!(
            "{} {}: {}",
            if open { "Hide" } else { "Show" },
            kind.name(),
            keys::text(cx, kind.key_id())
        );
        let f = on_toggle.clone();
        PaneToggle::new(kind.word(), icon(kind), open)
            .label(kind.name())
            .when(kind == PaneKind::Alerts, |t| t.count(unseen))
            .tooltip(tooltip)
            .on_click(move |_, window, cx| f(kind, window, cx))
    });
    div()
        .debug_selector(|| "pane-bar".into())
        .flex()
        .flex_none()
        .items_center()
        .justify_between()
        .w_full()
        .h(control::TOOLBAR_HEIGHT)
        .pad_x(space(4))
        .bg(surface::base())
        .border_b_1()
        .border_color(line::border())
        .child(
            div()
                .flex()
                .items_center()
                .gap(space(2))
                .child(img(APP_ICON.clone()).size(control::APP_ICON).flex_none())
                .child(
                    div()
                        .text_size(size::md())
                        .font_weight(weight::SEMIBOLD)
                        .text_color(text::primary())
                        .mr(space(2))
                        .child("AgentSI"),
                )
                .children(toggles),
        )
        .child(keys::hint(cx, keys::PALETTE))
}
