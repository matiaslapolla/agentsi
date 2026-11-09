//! How a screen asks the shell to open another screen. A screen is a view inside a
//! panel, and the shell owns the dock, so the screen sends a request on a channel
//! that the shell reads.

use futures::channel::mpsc;
use gpui_kit::{App, Entity, Global};

use crate::model::{CoreState, ProductRef};
use crate::screens::{ScreenKind, ScreenSpec};

/// Which screen to open, or select when it is open already.
pub type ScreenRequest = ScreenSpec;

impl ScreenSpec {
    pub fn minutes(product: &str) -> Self {
        ScreenSpec {
            kind: ScreenKind::Minutes,
            host: None,
            product: product.into(),
            arg: None,
        }
    }

    /// The front desk of a product of any host.
    pub fn desk(product: &ProductRef) -> Self {
        ScreenSpec {
            kind: ScreenKind::Desk,
            host: product.host.clone(),
            product: product.id.clone(),
            arg: None,
        }
    }

    /// The same screen on `host`. The constructors name a screen of this device.
    pub fn on(self, host: Option<String>) -> Self {
        ScreenSpec { host, ..self }
    }

    pub fn roles(product: &str) -> Self {
        ScreenSpec {
            kind: ScreenKind::Roles,
            host: None,
            product: product.into(),
            arg: None,
        }
    }

    pub fn stories(product: &str) -> Self {
        ScreenSpec {
            kind: ScreenKind::Stories,
            host: None,
            product: product.into(),
            arg: None,
        }
    }

    pub fn story(product: &str, id: &str) -> Self {
        ScreenSpec {
            kind: ScreenKind::Story,
            host: None,
            product: product.into(),
            arg: Some(id.into()),
        }
    }

    /// The setup screen. An empty `product` registers a new one.
    pub fn setup(product: &str) -> Self {
        ScreenSpec {
            kind: ScreenKind::Setup,
            host: None,
            product: product.into(),
            arg: None,
        }
    }

    pub fn tickets(product: &str) -> Self {
        ScreenSpec {
            kind: ScreenKind::Tickets,
            host: None,
            product: product.into(),
            arg: None,
        }
    }

    pub fn ticket(product: &str, id: &str) -> Self {
        ScreenSpec {
            kind: ScreenKind::Ticket,
            host: None,
            product: product.into(),
            arg: Some(id.into()),
        }
    }

    /// The front desk of a product of this device.
    pub fn front_desk(product: &str) -> Self {
        ScreenSpec {
            kind: ScreenKind::Desk,
            host: None,
            product: product.into(),
            arg: None,
        }
    }

    pub fn git(product: &str) -> Self {
        ScreenSpec {
            kind: ScreenKind::Git,
            host: None,
            product: product.into(),
            arg: None,
        }
    }

    pub fn meeting(product: &str) -> Self {
        ScreenSpec {
            kind: ScreenKind::Meeting,
            host: None,
            product: product.into(),
            arg: None,
        }
    }

    pub fn memory(product: &str) -> Self {
        ScreenSpec {
            kind: ScreenKind::Memory,
            host: None,
            product: product.into(),
            arg: None,
        }
    }

    /// The settings screen, which belongs to no product.
    pub fn settings() -> Self {
        ScreenSpec {
            kind: ScreenKind::Settings,
            host: None,
            product: String::new(),
            arg: None,
        }
    }

    /// The guide, which belongs to no product.
    pub fn guide() -> Self {
        ScreenSpec {
            kind: ScreenKind::Guide,
            host: None,
            product: String::new(),
            arg: None,
        }
    }

    /// The call of the minutes `id`.
    pub fn call(product: &str, id: &str) -> Self {
        ScreenSpec {
            kind: ScreenKind::Call,
            host: None,
            product: product.into(),
            arg: Some(id.into()),
        }
    }

    pub fn minute(product: &str, id: &str) -> Self {
        ScreenSpec {
            kind: ScreenKind::Minute,
            host: None,
            product: product.into(),
            arg: Some(id.into()),
        }
    }
}

/// The sender side, kept as a global so that a screen that the layout restored can use it.
pub struct Nav(mpsc::UnboundedSender<ScreenRequest>);

impl Global for Nav {}

impl Nav {
    pub fn channel() -> (Nav, mpsc::UnboundedReceiver<ScreenRequest>) {
        let (tx, rx) = mpsc::unbounded();
        (Nav(tx), rx)
    }

    /// Asks the shell to open `request` on the host of `core`, the core of the asking
    /// screen. Thus a screen of a host never opens a screen of the same id on another host.
    /// Without a shell, nothing happens.
    pub fn open(cx: &App, core: &Entity<CoreState>, request: ScreenRequest) {
        if let Some(nav) = cx.try_global::<Nav>() {
            let host = core.read(cx).remote.clone();
            let _ = nav.0.unbounded_send(request.on(host));
        }
    }
}

/// A request to open a Claude pane with a text, in the first path of a product.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ClaudeRun {
    pub host: Option<String>,
    pub product: String,
    pub text: String,
}

/// The prompt argument for `claude`. A role wrote the action text, so a text that starts with
/// `-` would read as an option. The prefix keeps it a positional argument.
pub fn claude_prompt(text: &str) -> String {
    if text.trim_start().starts_with('-') {
        format!("Action: {text}")
    } else {
        text.to_string()
    }
}

/// The sender side of `ClaudeRun`. The owner's click on "Run in Claude" is the approval, so
/// the shell opens the pane at once and the app runs nothing by itself.
pub struct Runs(mpsc::UnboundedSender<ClaudeRun>);

impl Global for Runs {}

impl Runs {
    pub fn channel() -> (Runs, mpsc::UnboundedReceiver<ClaudeRun>) {
        let (tx, rx) = mpsc::unbounded();
        (Runs(tx), rx)
    }

    /// Asks the shell to open `claude <text>` for `product` on the host of `core`.
    pub fn run(cx: &App, core: &Entity<CoreState>, product: &str, text: &str) {
        if let Some(runs) = cx.try_global::<Runs>() {
            let host = core.read(cx).remote.clone();
            let _ = runs.0.unbounded_send(ClaudeRun {
                host,
                product: product.into(),
                text: text.into(),
            });
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_action_text_never_reads_as_an_option_of_claude() {
        assert_eq!(
            claude_prompt("--dangerously-skip-permissions"),
            "Action: --dangerously-skip-permissions"
        );
        assert_eq!(claude_prompt("  -p x"), "Action:   -p x");
        assert_eq!(claude_prompt("Fix the build"), "Fix the build");
    }

    #[test]
    fn a_request_for_one_minutes_carries_its_id() {
        let r = ScreenRequest::minute("demo", "m1");
        assert_eq!((r.kind, r.arg.as_deref()), (ScreenKind::Minute, Some("m1")));
        let r = ScreenRequest::minutes("demo");
        assert_eq!((r.kind, r.arg), (ScreenKind::Minutes, None));
    }

    #[test]
    fn the_shell_reads_what_a_screen_sends() {
        let (nav, mut rx) = Nav::channel();
        nav.0
            .unbounded_send(ScreenRequest::minutes("demo"))
            .unwrap();
        assert_eq!(rx.try_recv().unwrap(), ScreenRequest::minutes("demo"));
    }
}
