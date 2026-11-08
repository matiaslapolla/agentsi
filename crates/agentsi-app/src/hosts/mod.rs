//! SSH hosts: the products of each host in the sidebar, "device off", and the voice of
//! each remote core through the local pill. Spec: `docs/specs/agentsi-desktop-protocol.md`, SSH.
//!
//! Each host has a thread (`conn`) and a `CoreState` of its own, which its screens and panes
//! hold. The threads report changes to one pump on the UI thread. It keeps the `Hosts`
//! entity, which the sidebar and the settings screen observe, and the `CoreState` of the host.
//! So a host change redraws no pane of another host.

pub mod config;
pub mod conn;

use std::path::PathBuf;

use agentsi_proto::desk_api::ProductInfo;
use futures::StreamExt;
use futures::channel::mpsc;
use gpui_kit::*;

use crate::model::{CoreState, Cores, HostState, take_event, take_up};
use crate::screens::meeting::{MeetingFeed, Meetings};
use conn::{Change, Host, Limits, Update, spawn_dial, ssh_argv};

/// What the window shows of one SSH host.
#[derive(Debug, Clone, PartialEq)]
pub struct RemoteHost {
    pub name: String,
    pub ssh: String,
    pub state: HostState,
    /// The products of the last connection. They stay while the host is off.
    pub products: Vec<ProductInfo>,
    pub note: Option<String>,
}

#[derive(Debug, Default)]
pub struct Hosts {
    /// `hosts.toml`, or `None` when the config folder is unknown.
    pub file: Option<PathBuf>,
    /// Why the file was not read.
    pub error: Option<String>,
    pub hosts: Vec<RemoteHost>,
}

impl Hosts {
    /// Takes one report of a host thread. Returns false when nothing changed.
    pub fn apply(&mut self, u: Update) -> bool {
        let Some(h) = self.hosts.get_mut(u.host) else {
            return false;
        };
        match u.change {
            Change::State(s) if h.state != s => h.state = s,
            Change::Products(p) if h.products != p => h.products = p,
            Change::Note(n) if h.note != n => h.note = n,
            _ => return false,
        }
        true
    }
}

/// The hosts of this window, set at start.
pub struct HostsGlobal(pub Entity<Hosts>);

impl Global for HostsGlobal {}

/// Takes one report of a host thread into the state of its core.
fn apply_to_core(core: &Entity<CoreState>, change: &Change, cx: &mut AsyncApp) {
    match change {
        Change::Link(l) => core.update(cx, |s, _| s.link = l.0.clone()),
        Change::Event(n) => take_event(core, n, cx),
        Change::Products(p) => core.update(cx, |s, cx| s.set_products(p.clone(), cx)),
        Change::State(state) => {
            let up = core.update(cx, |s, cx| {
                if s.host == *state {
                    return false;
                }
                s.host = state.clone();
                if matches!(state, HostState::Off(_)) {
                    s.meetings_lost(cx);
                }
                cx.notify();
                *state == HostState::Up
            });
            if up {
                take_up(core, cx);
            }
        }
        Change::Note(_) => {}
    }
}

/// Reads `hosts.toml`, makes the core state of each host and starts its thread. `local` is
/// the state of the local core, which gets the voice events.
pub fn start(local: Entity<CoreState>, cx: &mut App) -> (Entity<Hosts>, Cores) {
    let local_link = local.read(cx).link.clone();
    let file = config::path();
    let (configs, error) = match file.as_deref().map(config::read) {
        Some(Ok(c)) => (c, None),
        Some(Err(e)) => {
            eprintln!("agentsi-app: no SSH hosts: {e}");
            (Vec::new(), Some(e))
        }
        None => (Vec::new(), None),
    };
    let hosts = Hosts {
        file: file.clone(),
        error,
        hosts: configs
            .iter()
            .map(|c| RemoteHost {
                name: c.name.clone(),
                ssh: c.ssh.clone(),
                state: HostState::Connecting,
                products: c.products.clone(),
                note: None,
            })
            .collect(),
    };
    let entity = cx.new(|_| hosts);
    let feed = cx.new(|_| MeetingFeed(Meetings::default()));
    local.update(cx, |s, _| s.meetings = Some(feed));
    let mut cores = Cores::new(local);
    for c in &configs {
        let feed = cx.new(|_| MeetingFeed(Meetings::default()));
        let core = cx.new(|_| CoreState {
            meetings: Some(feed),
            ..CoreState::remote(&c.name, c.products.clone())
        });
        cores.remote.push((c.name.clone(), core));
    }
    let (tx, mut rx) = mpsc::unbounded::<Update>();
    for (index, c) in configs.into_iter().enumerate() {
        let host = Host::new(
            index,
            c.name.clone(),
            file.clone(),
            c.products.clone(),
            local_link.clone(),
            Limits::default(),
            spawn_dial(ssh_argv(&c)),
            tx.clone(),
        );
        if let Err(e) = host.spawn() {
            let off = HostState::Off(format!("no thread for the host: {e}"));
            cores.remote[index]
                .1
                .update(cx, |s, _| s.host = off.clone());
            entity.update(cx, |h, _| {
                h.apply(Update {
                    host: index,
                    change: Change::State(off),
                })
            });
        }
    }
    drop(tx);
    let e = entity.clone();
    let remote: Vec<Entity<CoreState>> = cores.remote.iter().map(|(_, c)| c.clone()).collect();
    cx.spawn(async move |cx| {
        while let Some(u) = rx.next().await {
            if let Some(core) = remote.get(u.host) {
                apply_to_core(core, &u.change, cx);
            }
            e.update(cx, |h, cx| {
                if h.apply(u) {
                    cx.notify();
                }
            });
        }
    })
    .detach();
    (entity, cores)
}

#[cfg(test)]
mod tests {
    use std::cell::RefCell;
    use std::rc::Rc;

    use agentsi_proto::rpc::Notification;
    use serde_json::json;

    use super::conn::Linked;
    use super::{Change, HostState, Hosts, RemoteHost, Update, apply_to_core};
    use crate::link::Link;
    use crate::model::two_cores::{methods, two_cores};
    use crate::model::{CoreEvent, CoreState};
    use gpui::TestAppContext;
    use gpui_kit::Entity;

    fn watch(core: &Entity<CoreState>, cx: &mut TestAppContext) -> Rc<RefCell<Vec<CoreEvent>>> {
        let seen: Rc<RefCell<Vec<CoreEvent>>> = Rc::default();
        let out = seen.clone();
        cx.update(|cx| {
            cx.subscribe(core, move |_, e: &CoreEvent, _| out.borrow_mut().push(*e))
                .detach()
        });
        seen
    }

    #[gpui::test]
    fn a_data_event_of_a_host_reaches_only_the_core_of_that_host(cx: &mut TestAppContext) {
        let t = two_cores(cx);
        let studio = t.cores.of(Some("studio")).unwrap().clone();
        let (on_studio, on_local) = (watch(&studio, cx), watch(&t.cores.local, cx));
        let n: Notification = serde_json::from_value(json!({
            "jsonrpc": "2.0", "method": "data.changed",
            "params": {"product": "demo", "kind": "minutes"}
        }))
        .unwrap();
        apply_to_core(&studio, &Change::Event(n), &mut cx.to_async());
        cx.run_until_parked();
        assert_eq!(*on_studio.borrow(), [CoreEvent::DataChanged]);
        assert!(on_local.borrow().is_empty());
    }

    #[gpui::test]
    fn a_new_connection_replaces_the_link_and_a_loss_keeps_the_dead_one(cx: &mut TestAppContext) {
        let mut t = two_cores(cx);
        let studio = t.cores.of(Some("studio")).unwrap().clone();
        let seen = watch(&studio, cx);
        let mut a = cx.to_async();
        apply_to_core(
            &studio,
            &Change::State(HostState::Off("gone".into())),
            &mut a,
        );
        let dead = Link::lost("gone");
        studio.update(cx, |s, _| s.link = dead.clone());
        assert!(seen.borrow().is_empty());

        // A link with no reader thread: the test scheduler aborts on a wake from another thread.
        let (end, mut fake) = std::os::unix::net::UnixStream::pair().unwrap();
        fake.set_nonblocking(true).unwrap();
        let link = std::sync::Arc::new(Link::for_tests(end));
        apply_to_core(&studio, &Change::Link(Linked(link.clone())), &mut a);
        assert!(studio.read_with(cx, |s, _| std::sync::Arc::ptr_eq(&s.link, &link)));
        assert!(
            seen.borrow().is_empty(),
            "the host is up only with its state"
        );
        apply_to_core(&studio, &Change::State(HostState::Up), &mut a);
        assert_eq!(*seen.borrow(), [CoreEvent::Up]);
        // The lists of the new connection go to the new link, not to the old socket: the
        // terminals, and the story sessions of the one product.
        assert_eq!(methods(&mut fake), ["terminal.list", "story.list"]);
        assert_eq!(methods(&mut t.studio), Vec::<String>::new());
        // The same state again changes nothing.
        apply_to_core(&studio, &Change::State(HostState::Up), &mut a);
        assert_eq!(seen.borrow().len(), 1);
    }

    #[test]
    fn apply_reports_only_changes() {
        let mut h = Hosts {
            hosts: vec![RemoteHost {
                name: "a".into(),
                ssh: "a".into(),
                state: HostState::Connecting,
                products: Vec::new(),
                note: None,
            }],
            ..Hosts::default()
        };
        let off = |why: &str| Update {
            host: 0,
            change: Change::State(HostState::Off(why.into())),
        };
        assert!(h.apply(off("gone")));
        assert!(!h.apply(off("gone")));
        assert!(h.apply(off("other")));
        assert!(!h.apply(Update {
            host: 3,
            change: Change::Note(None)
        }));
    }
}
