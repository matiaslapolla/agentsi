//! Pieces that the screens share: the load state, a section title, a stat tile and the
//! tags of statuses and marks. Each color and size comes from `agentsi_ui::tokens`.

use std::sync::atomic::{AtomicBool, Ordering};

use agentsi_ui::components::{neutral_tag, status_tag};
use agentsi_ui::style::StyleExt as _;
use agentsi_ui::tokens::{
    Status, control, font, line, radius, size, space, status, surface, text, weight,
};
use gpui_kit::component::tag::Tag;
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;

use crate::model::CoreState;

use super::desk::DeskScreen;
use super::minute::MinuteScreen;
use super::minutes::MinutesScreen;

/// Set when the first screen of the app has its data (or its error). The start bench
/// counts the first paint of the front desk from this moment.
pub static FIRST_SCREEN_READY: AtomicBool = AtomicBool::new(false);

pub fn first_screen_ready() {
    FIRST_SCREEN_READY.store(true, Ordering::Relaxed);
}

/// What a screen shows while it reads the core.
pub enum Load<T> {
    Loading,
    Ready(T),
    Failed(String),
}

impl<T> Load<T> {
    /// Keeps `Ready` while a reload runs, so a reload does not blank the screen.
    pub fn settle(&mut self, result: Result<T, String>) {
        *self = match result {
            Ok(v) => Load::Ready(v),
            Err(e) => Load::Failed(e),
        };
    }
}

/// What a screen knows about the call of one minutes.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct CallInfo {
    /// An audio file exists, so the owner can listen.
    pub audio: bool,
    /// A render runs: segments done and segments of the script.
    pub render: Option<(u32, u32)>,
}

impl From<&agentsi_proto::call_api::CallGetResult> for CallInfo {
    fn from(c: &agentsi_proto::call_api::CallGetResult) -> Self {
        CallInfo {
            audio: c.audio.is_some(),
            render: c.render.as_ref().map(|r| (r.done, r.of)),
        }
    }
}

static NEXT_RENDER: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(1);

/// A name for the events of a render that this app starts. The core tells it from a render of the CLI.
pub fn render_name() -> String {
    format!(
        "app-{}-{}",
        std::process::id(),
        NEXT_RENDER.fetch_add(1, Ordering::Relaxed)
    )
}

/// A centered line of text: the loading state, an empty list or an error.
pub fn notice(message: impl Into<SharedString>, hint: Option<String>) -> Div {
    div()
        .size_full()
        .flex()
        .flex_col()
        .items_center()
        .justify_center()
        .gap(space(2))
        .p(space(8))
        .text_color(text::secondary())
        .child(div().text_size(size::base()).child(message.into()))
        .when_some(hint, |el, h| {
            el.child(
                div()
                    .text_size(size::sm())
                    .text_color(text::tertiary())
                    .child(h),
            )
        })
}

/// The line under a screen that did not load: how to read it again.
pub fn reload_hint(cx: &App) -> String {
    format!(
        "Reload the screens from the command palette ({}).",
        crate::keys::text(cx, crate::keys::PALETTE)
    )
}

/// A section title with a count: `Pending decisions  3`.
pub fn section(title: &str, count: Option<usize>) -> Div {
    div()
        .flex()
        .items_baseline()
        .gap(space(2))
        .child(
            div()
                .text_size(size::md())
                .font_weight(weight::SEMIBOLD)
                .text_color(text::primary())
                .child(title.to_string()),
        )
        .when_some(count, |el, n| {
            el.child(
                div()
                    .text_size(size::sm())
                    .text_color(text::tertiary())
                    .child(n.to_string()),
            )
        })
}

/// A small label above a value, for a row of figures.
pub fn stat(label: &str, value: impl Into<SharedString>) -> Div {
    div()
        .flex()
        .flex_col()
        .gap(space(1))
        .child(
            div()
                .text_size(size::xs())
                .font_weight(weight::MEDIUM)
                .text_color(text::tertiary())
                .child(label.to_string()),
        )
        .child(
            div()
                .text_size(size::lg())
                .font_weight(weight::SEMIBOLD)
                .text_color(text::primary())
                .child(value.into()),
        )
}

/// A row of figures in one bordered block.
pub fn stats(items: Vec<Div>) -> Div {
    div()
        .flex()
        .flex_wrap()
        .gap_x(space(8))
        .gap_y(space(3))
        .p(space(4))
        .rounded(radius::lg())
        .border_1()
        .border_color(line::border())
        .bg(surface::raised())
        .children(items)
}

/// Text in the mono font, for ids and commands.
pub fn mono(s: impl Into<SharedString>) -> Div {
    div()
        .font_family(font::MONO)
        .text_size(size::xs())
        .text_color(text::tertiary())
        .child(s.into())
}

/// The tag of a minutes status.
pub fn minutes_status(status: Option<&str>) -> Tag {
    match status {
        Some("valid") => status_tag("valid", Status::Ok),
        Some("invalid") => status_tag("invalid", Status::Danger),
        Some(other) => neutral_tag(other.to_string()),
        None => neutral_tag("no status"),
    }
}

/// The tag of the mark of a priority.
pub fn mark_tag(mark: Option<&str>) -> Tag {
    match mark {
        Some("accepted") => status_tag("accepted", Status::Ok),
        Some("changed") => status_tag("changed", Status::Info),
        Some("rejected") => status_tag("rejected", Status::Danger),
        Some(other) => neutral_tag(other.to_string()),
        None => neutral_tag("unmarked"),
    }
}

/// The tag of a decision status. A decision with no row in the store is "not recorded".
pub fn decision_tag(status: &str, recorded: bool) -> Tag {
    if !recorded && status == "pending" {
        return neutral_tag("not recorded");
    }
    match status {
        "approved" => status_tag("approved", Status::Ok),
        "rejected" => status_tag("rejected", Status::Danger),
        "pending" => status_tag("pending", Status::Warn),
        other => neutral_tag(other.to_string()),
    }
}

/// The meta line of a minutes row: id, date, `3/5 marked` and `heard`.
pub fn minutes_meta(m: &agentsi_proto::desk_api::MinutesRow) -> String {
    let marked = m.marks.accepted + m.marks.changed + m.marks.rejected;
    let total = marked + m.marks.unmarked;
    let mut parts = vec![m.id.clone()];
    parts.extend(m.created_at.as_deref().map(crate::format::date_of));
    if total > 0 {
        parts.push(format!("{marked}/{total} marked"));
    }
    if m.heard {
        parts.push("heard".into());
    }
    parts.join(" · ")
}

/// `Dev` for `dev`, `Dev · backend` for `dev.backend`.
pub fn role_label(role: &str) -> String {
    let (base, rest) = match role.split_once('.') {
        Some((b, r)) => (b, Some(r)),
        None => (role, None),
    };
    let name = match base {
        "pm" => "PM".to_string(),
        "qa" => "QA".to_string(),
        other => {
            let mut c = other.chars();
            c.next().map_or(String::new(), |f| {
                f.to_uppercase().collect::<String>() + c.as_str()
            })
        }
    };
    match rest {
        Some(r) => format!("{name} · {r}"),
        None => name,
    }
}

/// What the last write of a screen did. The screen shows it in one line.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Flash {
    Done(String),
    Failed(String),
}

/// The line that tells the owner what the last write did.
pub fn flash_line(flash: &Flash) -> Div {
    let (text_color, tint, border, message) = match flash {
        Flash::Done(m) => (status::ok(), Status::Ok.tint(), status::ok(), m),
        Flash::Failed(m) => (status::danger(), Status::Danger.tint(), status::danger(), m),
    };
    div()
        .p(space(3))
        .rounded(radius::md())
        .border_1()
        .border_color(border.alpha(status::BORDER_ALPHA))
        .bg(tint)
        .text_size(size::sm())
        .text_color(text_color)
        .child(message.clone())
}

/// A screen that writes through the core: it keeps the line of the last write and
/// refuses a second write while one waits. Its `reload` clears the busy flag.
pub trait Writes: Sized + 'static {
    fn state(&mut self) -> (&mut bool, &mut Option<Flash>);
    /// The core of the screen: its host holds the data that the write changed.
    fn core(&self) -> &Entity<CoreState>;
}

/// Sends one write and shows its outcome. When the core accepts it, `done` is the
/// line to show and each screen of that core reads its data again.
pub fn write_through<T: Writes, R: 'static>(
    this: &mut T,
    call: impl std::future::Future<Output = Result<R, crate::link::CallError>> + 'static,
    done: String,
    cx: &mut Context<T>,
) {
    let core = this.core().clone();
    let (busy, flash) = this.state();
    if *busy {
        return;
    }
    *busy = true;
    *flash = None;
    cx.notify();
    cx.spawn(async move |this, cx| {
        let result = call.await.map(|_| ()).map_err(|e| e.to_string());
        let _ = this.update(cx, |this, cx| {
            let (busy, flash) = this.state();
            // After a write that the core accepted, the screen stays busy until it has read
            // the new data. A key before that would act on the row that the write changed.
            *busy = result.is_ok();
            *flash = Some(match result {
                Ok(()) => Flash::Done(done),
                Err(e) => Flash::Failed(e),
            });
            cx.notify();
        });
        // Each screen of the core reads again, this one too. It cannot run inside the update
        // above.
        cx.update(|cx| Live::reload_core(&core, cx));
    })
    .detach();
}

/// The screens that write, so a palette action finds its target and a write
/// makes each of them on the same core read again. The palette closes with the focus on a pane,
/// so an action cannot rely on the focus. The target is the screen in view that
/// the owner used last. A hidden tab is no target: it can be on another host.
#[derive(Default)]
pub struct Live {
    desks: Vec<WeakEntity<DeskScreen>>,
    minutes: Vec<WeakEntity<MinuteScreen>>,
    lists: Vec<WeakEntity<MinutesScreen>>,
    clock: u64,
}

impl Global for Live {}

impl Live {
    /// A new number, higher than each earlier one: the order of use.
    pub fn tick(cx: &mut App) -> u64 {
        let live = cx.default_global::<Live>();
        live.clock += 1;
        live.clock
    }

    pub fn add_desk(cx: &mut App, screen: WeakEntity<DeskScreen>) {
        let live = cx.default_global::<Live>();
        live.desks.retain(|w| w.upgrade().is_some());
        live.desks.push(screen);
    }

    pub fn add_minute(cx: &mut App, screen: WeakEntity<MinuteScreen>) {
        let live = cx.default_global::<Live>();
        live.minutes.retain(|w| w.upgrade().is_some());
        live.minutes.push(screen);
    }

    pub fn add_list(cx: &mut App, screen: WeakEntity<MinutesScreen>) {
        let live = cx.default_global::<Live>();
        live.lists.retain(|w| w.upgrade().is_some());
        live.lists.push(screen);
    }

    /// The front desk on screen that the owner used last.
    pub fn desk(cx: &App) -> Option<Entity<DeskScreen>> {
        let live = cx.try_global::<Live>()?;
        live.desks
            .iter()
            .filter_map(WeakEntity::upgrade)
            .filter(|d| d.read(cx).shown())
            .max_by_key(|d| d.read(cx).stamp())
    }

    /// The minutes screen on screen that the owner used last.
    pub fn minute(cx: &App) -> Option<Entity<MinuteScreen>> {
        let live = cx.try_global::<Live>()?;
        live.minutes
            .iter()
            .filter_map(WeakEntity::upgrade)
            .filter(|m| m.read(cx).shown())
            .max_by_key(|m| m.read(cx).stamp())
    }

    /// Makes each screen of this module on `core` read it again, after a write. The screens
    /// of another host keep their data: the write did not reach it.
    pub fn reload_core(core: &Entity<CoreState>, cx: &mut App) {
        let Some(live) = cx.try_global::<Live>() else {
            return;
        };
        let id = core.entity_id();
        let desks: Vec<_> = live
            .desks
            .iter()
            .filter_map(WeakEntity::upgrade)
            .filter(|s| s.read(cx).core().entity_id() == id)
            .collect();
        let minutes: Vec<_> = live
            .minutes
            .iter()
            .filter_map(WeakEntity::upgrade)
            .filter(|s| s.read(cx).core().entity_id() == id)
            .collect();
        let lists: Vec<_> = live
            .lists
            .iter()
            .filter_map(WeakEntity::upgrade)
            .filter(|s| s.read(cx).core().entity_id() == id)
            .collect();
        for d in desks {
            d.update(cx, |s, cx| s.reload(cx));
        }
        for m in minutes {
            m.update(cx, |s, cx| s.reload(cx));
        }
        for l in lists {
            l.update(cx, |s, cx| s.reload(cx));
        }
    }
}

/// The width of the content column of a screen: long lines read badly.
pub fn column() -> Div {
    div()
        .debug_selector(|| "screen-column".into())
        .w_full()
        .max_w(control::CONTENT_WIDTH)
        .mx_auto()
        .flex()
        .flex_col()
        .gap(space(8))
        .pad_x(space(8))
        .py(space(8))
}

#[cfg(test)]
mod tests {
    use super::{Load, role_label};

    #[test]
    fn a_role_reads_as_a_label() {
        assert_eq!(role_label("pm"), "PM");
        assert_eq!(role_label("qa"), "QA");
        assert_eq!(role_label("dev"), "Dev");
        assert_eq!(role_label("dev.backend"), "Dev · backend");
        assert_eq!(role_label(""), "");
    }

    #[test]
    fn a_settled_load_replaces_the_old_state() {
        let mut l: Load<u32> = Load::Loading;
        l.settle(Ok(1));
        assert!(matches!(l, Load::Ready(1)));
        l.settle(Err("no".into()));
        assert!(matches!(l, Load::Failed(ref e) if e == "no"));
    }
}
