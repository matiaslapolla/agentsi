//! `meeting.start`, `meeting.ask`, `meeting.cancel` and `minutes.check`: the
//! methods of `agentsi meet`, `ask` and `check`. The logic is in
//! `agentsi_agency::meeting`. A meeting takes minutes, so it runs off the event
//! loop, and its events reach each subscriber of the topic `meeting`.

use super::{Ctx, Router};
use crate::state::Core;
use agentsi_agency::Error as AgencyError;
use agentsi_agency::meeting::running::{self, Entry, Refused};
use agentsi_agency::meeting::{
    self, Env, Error as MeetingError, MeetingOptions, Sink, ask_role, run_meeting,
};
use agentsi_agency::registry::Registry;
use agentsi_proto::RpcError;
use agentsi_proto::events::{DataChanged, DataChangedParams, MeetingEvent, MeetingEventParams};
use agentsi_proto::meeting_api::{
    MeetingAsk, MeetingAskParams, MeetingAskResult, MeetingCancel, MeetingCancelParams,
    MeetingCancelResult, MeetingProgress, MeetingProgressParams, MeetingStart, MeetingStartParams,
    MeetingStartResult, MinutesCheck, MinutesCheckParams, MinutesCheckResult,
};
use agentsi_proto::rpc::ErrorKind;
use agentsi_store::{Store, read};
use serde_json::Value;
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

/// The program of each role. Tests put a fake one first on the `PATH`.
const CLAUDE: &str = "claude";
/// A line of a role can hold a whole file. A longer line stays out of the events, so
/// no subscriber queue holds it. The result line never needs this size.
const MAX_EVENT_LINE: usize = 1 << 20;

static NEXT_NAME: AtomicU64 = AtomicU64::new(1);

/// The registry entry of a new meeting. The blocking work must own it, so the
/// entry lives as long as the Claude runs, also when the client goes away.
fn register_meeting(name: Option<String>, product: &str) -> Result<Entry, RpcError> {
    let name = name.unwrap_or_else(|| {
        format!(
            "meeting-{}-{}",
            std::process::id(),
            NEXT_NAME.fetch_add(1, Ordering::Relaxed)
        )
    });
    Entry::for_product(name.clone(), product).map_err(|refused| match refused {
        Refused::NameTaken => RpcError::new(
            ErrorKind::Conflict,
            format!("A meeting named {name} runs already."),
            "Pick another name, or leave the name out.",
        ),
        Refused::CoreStops => RpcError::new(
            ErrorKind::Unavailable,
            "The core is stopping.",
            "Start the meeting again when the core runs.",
        ),
    })
}

pub fn register(r: Router) -> Router {
    r.add(MeetingStart, start)
        .add(MeetingAsk, ask)
        .add(MeetingCancel, cancel)
        .add(MinutesCheck, check)
}

fn rpc(e: MeetingError) -> RpcError {
    match e {
        MeetingError::Agency(AgencyError::NotFound(m)) => RpcError::new(
            ErrorKind::NotFound,
            m,
            "Use product.whoami with list: true for the products of the registry.",
        ),
        MeetingError::Agency(AgencyError::Unavailable(m)) => RpcError::new(
            ErrorKind::Unavailable,
            m,
            "Set AGENTSI_HOME to the data root. It holds agentsi.config.json and agentsi.db.",
        ),
        MeetingError::Invalid(m) => RpcError::new(
            ErrorKind::Invalid,
            m,
            "Run agentsi roles validate for the state of the roster.",
        ),
        MeetingError::Conflict(m) => RpcError::new(
            ErrorKind::Conflict,
            m,
            "Nothing was written. Run the meeting again when the product is ready.",
        ),
        MeetingError::Failed(m) => RpcError::internal(m),
    }
}

/// What a handler works with.
struct Work {
    root: PathBuf,
    registry: Registry,
    store: Arc<Store>,
}

fn work(core: &Core) -> meeting::Result<Work> {
    let root = agentsi_store::paths::data_root(None)?;
    let registry = Registry::load(&root)?;
    let store = core.store()?.ok_or_else(|| {
        AgencyError::Unavailable("the data root has no agentsi.db. Import the data first.".into())
    })?;
    Ok(Work {
        root,
        registry,
        store,
    })
}

/// Runs `f` off the event loop.
async fn blocking<T: Send + 'static>(
    ctx: &Ctx,
    f: impl FnOnce(Arc<Core>, Work) -> meeting::Result<T> + Send + 'static,
) -> Result<T, RpcError> {
    let core = ctx.core.clone();
    crate::blocking::spawn(move || {
        let w = work(&core).map_err(rpc)?;
        f(core, w).map_err(rpc)
    })
    .await
    .map_err(|e| RpcError::internal(e.to_string()))?
}

/// Publishes the events of one meeting.
struct Events {
    core: Arc<Core>,
    meeting: String,
}

impl Sink for Events {
    fn progress(&self, event: MeetingProgressParams) {
        self.core.publish::<MeetingProgress>(&event);
    }

    fn line(&self, role: &str, line: &str) {
        if line.len() > MAX_EVENT_LINE {
            return;
        }
        self.core.publish::<MeetingEvent>(&MeetingEventParams {
            meeting: self.meeting.clone(),
            role: role.into(),
            line: line.into(),
        });
    }
}

async fn start(ctx: Ctx, p: MeetingStartParams) -> Result<MeetingStartResult, RpcError> {
    let entry = register_meeting(p.meeting.clone(), &p.product)?;
    let name = entry.name.clone();
    let outcome = blocking(&ctx, move |core, w| {
        let events = Events {
            core: core.clone(),
            meeting: entry.name.clone(),
        };
        let result = run_meeting(
            &Env {
                store: &w.store,
                root: &w.root,
                registry: &w.registry,
                sink: &events,
                cancel: &entry.cancel,
                claude: CLAUDE,
            },
            &MeetingOptions {
                meeting: entry.name.clone(),
                product: p.product.clone(),
                scope: p.scope,
                question: p.question,
                roles: p.roles.unwrap_or_default(),
                frozen: p.frozen,
            },
        )?;
        core.publish::<DataChanged>(&DataChangedParams {
            product: p.product,
            kind: "minutes".into(),
        });
        Ok(result)
    })
    .await?;
    Ok(MeetingStartResult {
        meeting: name,
        minutes: outcome.minutes,
        usage: outcome.usage,
        total_cost_usd: outcome.total_cost_usd,
    })
}

async fn ask(ctx: Ctx, p: MeetingAskParams) -> Result<MeetingAskResult, RpcError> {
    let entry = register_meeting(p.meeting.clone(), &p.product)?;
    let outcome = blocking(&ctx, move |core, w| {
        let events = Events {
            core,
            meeting: entry.name.clone(),
        };
        ask_role(
            &Env {
                store: &w.store,
                root: &w.root,
                registry: &w.registry,
                sink: &events,
                cancel: &entry.cancel,
                claude: CLAUDE,
            },
            &p.product,
            p.scope.as_deref(),
            &p.role,
            &p.question,
        )
    })
    .await?;
    Ok(MeetingAskResult {
        answer: outcome.answer,
        usage: outcome.usage,
        warnings: outcome.warnings,
    })
}

async fn cancel(_ctx: Ctx, p: MeetingCancelParams) -> Result<MeetingCancelResult, RpcError> {
    Ok(MeetingCancelResult {
        canceled: running::cancel(&p.meeting),
    })
}

async fn check(ctx: Ctx, p: MinutesCheckParams) -> Result<MinutesCheckResult, RpcError> {
    blocking(&ctx, move |_, w| {
        let id = p.minutes.get("product").and_then(Value::as_str);
        let product = w
            .registry
            .product(id.unwrap_or("undefined"))
            .map_err(MeetingError::Agency)?
            .clone();
        let mut out = None;
        w.store.read(|c| {
            let brief = read::brief(c, &product.id)?;
            out = Some(meeting::check::check(
                c,
                &p.minutes,
                &product,
                p.roles.as_deref(),
                brief.as_ref(),
            ));
            Ok(())
        })?;
        Ok(out.expect("the read ran"))
    })
    .await
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_name_runs_once_and_is_free_again_after_the_end() {
        let first = register_meeting(Some("same".into()), "p").unwrap();
        let second = register_meeting(Some("same".into()), "p");
        assert!(second.is_err());
        drop(first);
        assert!(register_meeting(Some("same".into()), "p").is_ok());
    }

    #[test]
    fn a_core_makes_a_name_for_each_meeting() {
        let (a, b) = (
            register_meeting(None, "p").unwrap(),
            register_meeting(None, "p").unwrap(),
        );
        assert_ne!(a.name, b.name);
    }
}
