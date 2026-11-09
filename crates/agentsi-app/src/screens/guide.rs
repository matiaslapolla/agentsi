//! The Guide screen: the page of `docs/guide/README.md`, drawn as Markdown with its images.
//! It belongs to no product. The page scrolls, and Page Up, Page Down, Home and End scroll it.

use agentsi_ui::tokens::surface;
use gpui_kit::component::text::{TextView, TextViewState};
use gpui_kit::*;

use super::parts::column;
use crate::guide::{self, Link};

pub struct GuideScreen {
    page: Entity<TextViewState>,
    focus: FocusHandle,
    /// The section to scroll to, which waits until the page is parsed.
    pending: Option<String>,
    _page: Subscription,
}

impl GuideScreen {
    pub fn new(cx: &mut Context<Self>) -> Self {
        let page = cx.new(|cx| TextViewState::markdown(guide::PAGE, cx));
        // The page parses in the background. Each commit of the parse may let the section show.
        let observe = cx.observe(&page, |this, _, cx| this.reveal_pending(cx));
        GuideScreen {
            page,
            focus: cx.focus_handle(),
            pending: None,
            _page: observe,
        }
    }

    /// The page takes the focus, so the page keys reach it.
    pub fn focus_handle(&self) -> FocusHandle {
        self.focus.clone()
    }

    /// Scrolls the page to the section with this anchor. A name that is no section does nothing.
    pub fn reveal(&mut self, anchor: &str, cx: &mut Context<Self>) {
        if guide::section(anchor).is_some() {
            self.pending = Some(anchor.to_string());
            self.reveal_pending(cx);
        }
    }

    /// Asks the page to show the waiting section. The page answers a request only after its
    /// parse has landed, so the request stays until then.
    fn reveal_pending(&mut self, cx: &mut Context<Self>) {
        let Some(anchor) = self.pending.as_deref() else {
            return;
        };
        let Some(source) = guide::section(anchor) else {
            self.pending = None;
            return;
        };
        let rendered = self.page.read(cx).rendered_text();
        if rendered.source() != guide::PAGE {
            return;
        }
        if let Some(range) = rendered.range_for_source(source) {
            self.page
                .update(cx, |page, cx| page.reveal_range(range, cx))
                .ok();
        }
        self.pending = None;
    }

    /// The index of the block at the top of the page.
    #[cfg(test)]
    pub fn top(&self, cx: &App) -> usize {
        self.page.read(cx).list_state().logical_scroll_top().item_ix
    }

    /// Page Up, Page Down, Home and End scroll the page, as on the Settings screen.
    fn scroll_key(&mut self, e: &KeyDownEvent, cx: &mut Context<Self>) {
        let k = &e.keystroke;
        if k.modifiers.modified() {
            return;
        }
        let list = self.page.read(cx).list_state().clone();
        let page = list.viewport_bounds().size.height;
        match k.key.as_str() {
            "pagedown" => list.scroll_by(page),
            "pageup" => list.scroll_by(-page),
            "home" => list.scroll_to(ListOffset {
                item_ix: 0,
                offset_in_item: Pixels::ZERO,
            }),
            "end" => list.scroll_to_end(),
            _ => return,
        }
        cx.stop_propagation();
        cx.notify();
    }

    /// A click on a link: http and https open the browser, `#name` scrolls to a section.
    fn open_link(&mut self, url: &str, cx: &mut Context<Self>) {
        match guide::link(url) {
            Link::Browser(url) => cx.open_url(url),
            Link::Section(anchor) => self.reveal(anchor, cx),
            Link::Nothing => {}
        }
    }
}

impl Render for GuideScreen {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let this = cx.entity().downgrade();
        let view = TextView::new(&self.page)
            .scrollable(true)
            .selectable(true)
            .image_source(guide::image_source)
            .on_link_click(move |url, _, _, cx| {
                this.update(cx, |this, cx| this.open_link(url, cx)).ok();
            });
        div()
            .id("guide")
            .debug_selector(|| "guide-root".into())
            .size_full()
            .bg(surface::panel())
            .track_focus(&self.focus)
            .on_key_down(cx.listener(|this, e: &KeyDownEvent, _, cx| this.scroll_key(e, cx)))
            .child(column().h_full().child(view))
    }
}

#[cfg(test)]
mod tests {
    use agentsi_ui::tokens::control::WINDOW_WIDTH;
    use gpui::{Entity, ListOffset, Pixels, TestAppContext, VisualTestContext, size};

    use super::GuideScreen;
    use crate::guide;

    fn open(cx: &mut TestAppContext) -> (Entity<GuideScreen>, &mut VisualTestContext) {
        cx.update(agentsi_ui::init);
        let (screen, cx) = cx.add_window_view(|_, cx| GuideScreen::new(cx));
        cx.simulate_resize(size(WINDOW_WIDTH, Pixels::from(700.)));
        let focus = screen.read_with(cx, |s, _| s.focus_handle());
        cx.update(|window, cx| window.focus(&focus, cx));
        screen.update(cx, |_, cx| cx.notify());
        (screen, cx)
    }

    /// Lets the page parse and lays it out.
    fn settle(cx: &mut VisualTestContext) {
        cx.run_until_parked();
        cx.update(|window, cx| window.draw(cx).clear(cx));
        cx.run_until_parked();
        cx.update(|window, cx| window.draw(cx).clear(cx));
    }

    fn top(screen: &Entity<GuideScreen>, cx: &mut VisualTestContext) -> ListOffset {
        screen.read_with(cx, |s, cx| {
            s.page.read(cx).list_state().logical_scroll_top()
        })
    }

    fn key(cx: &mut VisualTestContext, key: &str) {
        cx.simulate_keystrokes(key);
        settle(cx);
    }

    #[gpui::test]
    fn the_page_parses_and_the_screen_draws_it(cx: &mut TestAppContext) {
        let (screen, cx) = open(cx);
        assert!(cx.debug_bounds("guide-root").is_some());
        settle(cx);
        let rendered = screen.read_with(cx, |s, cx| s.page.read(cx).rendered_text());
        assert_eq!(rendered.source(), guide::PAGE);
        assert!(rendered.as_str().contains("What AgentSI is"));
        let items = screen.read_with(cx, |s, cx| s.page.read(cx).list_state().item_count());
        assert!(items > 50, "{items} blocks");
    }

    #[gpui::test]
    fn the_page_keys_scroll_the_page_and_stop_at_both_ends(cx: &mut TestAppContext) {
        let (screen, cx) = open(cx);
        settle(cx);
        assert_eq!(
            (top(&screen, cx).item_ix, top(&screen, cx).offset_in_item),
            (0, Pixels::ZERO)
        );

        key(cx, "pagedown");
        let one = top(&screen, cx);
        assert!(
            one.item_ix > 0 || one.offset_in_item > Pixels::ZERO,
            "Page Down moved"
        );
        key(cx, "pagedown");
        let two = top(&screen, cx);
        assert!(
            (two.item_ix, two.offset_in_item) > (one.item_ix, one.offset_in_item),
            "a second Page Down moved on"
        );
        key(cx, "pageup");
        let back = top(&screen, cx);
        assert!((back.item_ix, back.offset_in_item) < (two.item_ix, two.offset_in_item));

        key(cx, "end");
        let end = top(&screen, cx);
        assert!(end.item_ix > two.item_ix, "End goes to the last block");
        key(cx, "pagedown");
        assert_eq!(
            top(&screen, cx).item_ix,
            end.item_ix,
            "the end stops Page Down"
        );

        key(cx, "home");
        let home = top(&screen, cx);
        assert_eq!((home.item_ix, home.offset_in_item), (0, Pixels::ZERO));
        key(cx, "pageup");
        assert_eq!(top(&screen, cx).item_ix, 0, "the top stops Page Up");
    }

    #[gpui::test]
    fn a_key_with_a_modifier_or_another_key_does_not_scroll(cx: &mut TestAppContext) {
        let (screen, cx) = open(cx);
        settle(cx);
        key(cx, "ctrl-end");
        key(cx, "a");
        let at = top(&screen, cx);
        assert_eq!((at.item_ix, at.offset_in_item), (0, Pixels::ZERO));
    }

    #[gpui::test]
    fn a_section_requested_before_the_page_is_parsed_shows_after_the_parse(
        cx: &mut TestAppContext,
    ) {
        let (screen, cx) = open(cx);
        // The page holds other text, as it does while the parse of the guide runs.
        screen.update(cx, |s, cx| {
            s.page.update(cx, |p, cx| p.set_text("# Other\n", cx))
        });
        settle(cx);
        screen.update(cx, |s, cx| s.reveal(guide::INSTALL, cx));
        assert!(
            screen.read_with(cx, |s, _| s.pending.is_some()),
            "the request waits"
        );
        assert_eq!(top(&screen, cx).item_ix, 0);
        screen.update(cx, |s, cx| {
            s.page.update(cx, |p, cx| p.set_text(guide::PAGE, cx))
        });
        settle(cx);
        assert!(screen.read_with(cx, |s, _| s.pending.is_none()));
        assert!(
            top(&screen, cx).item_ix > 0,
            "the page scrolled past its top"
        );
    }

    #[gpui::test]
    fn a_section_requested_after_the_parse_shows_at_once(cx: &mut TestAppContext) {
        let (screen, cx) = open(cx);
        settle(cx);
        screen.update(cx, |s, cx| s.reveal("data", cx));
        settle(cx);
        let data = top(&screen, cx).item_ix;
        screen.update(cx, |s, cx| s.reveal(guide::INSTALL, cx));
        settle(cx);
        let install = top(&screen, cx).item_ix;
        assert!(
            install > 0 && install < data,
            "install {install}, data {data}"
        );
        screen.update(cx, |s, cx| s.reveal("no-such-section", cx));
        assert!(screen.read_with(cx, |s, _| s.pending.is_none()));
    }

    #[gpui::test]
    fn a_web_link_opens_the_browser_and_a_section_link_scrolls(cx: &mut TestAppContext) {
        let (screen, cx) = open(cx);
        settle(cx);
        screen.update(cx, |s, cx| s.open_link("https://example.com/docs", cx));
        assert_eq!(cx.opened_url().as_deref(), Some("https://example.com/docs"));
        screen.update(cx, |s, cx| s.open_link("#keys", cx));
        settle(cx);
        assert!(top(&screen, cx).item_ix > 0);
        let opened = cx.opened_url();
        // A link to a file of the repo opens nothing.
        screen.update(cx, |s, cx| s.open_link("../../CONTRIBUTING.md", cx));
        assert_eq!(cx.opened_url(), opened);
    }
}
