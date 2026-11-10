//! `ticket.promote` with a fake PM: a story promote links the story and closes the ticket,
//! a failed `story.new` leaves the ticket as it was, and a decision promote adds a pending
//! decision in the same transaction.

use agentsi_agency::registry::Product;
use agentsi_agency::story::claude::Call;
use agentsi_agency::story::propose::ProposeCtx;
use agentsi_agency::tickets::{self, Error, write};
use agentsi_proto::agency_api::Owner;
use agentsi_proto::story_api::{StoryConfig, StoryUsage};
use agentsi_proto::ticket_api::{
    Author, AuthorBy, Link, LinkType, TicketAddParams, TicketKind, TicketMoveParams, TicketState,
};
use agentsi_store::Store;
use serde_json::{Value, json};
use std::sync::{Arc, Mutex};

const T0: i64 = 1_788_000_000_000;

struct Fixture {
    _dir: tempfile::TempDir,
    ctx: ProposeCtx,
    prompts: Arc<Mutex<Vec<String>>>,
}

fn owner() -> Author {
    Author {
        by: AuthorBy::Owner,
        id: "Ada".into(),
    }
}

fn story(title: &str) -> Value {
    let criterion =
        json!({"given": "g", "when": "w", "then": "t", "test": "test/a.test.ts", "checks": "c"});
    json!({"stories": [{"type": "story", "title": title, "as_a": "owner", "i_want": "x", "so_that": "y",
        "criteria": [criterion, criterion], "owner_role": "dev", "objective": null,
        "sources": [{"type": "path", "ref": "src/app.ts"}]}]})
}

/// A PM that answers with `output`, or fails when it is `None`.
fn fixture(output: Option<Value>) -> Fixture {
    let dir = tempfile::tempdir().unwrap();
    let base = dir.path().canonicalize().unwrap();
    let (root, repo) = (base.join("data"), base.join("repo"));
    std::fs::create_dir_all(&root).unwrap();
    std::fs::create_dir_all(repo.join("src")).unwrap();
    std::fs::write(repo.join("src/app.ts"), "export {}\n").unwrap();
    let store = Arc::new(Store::open(&root.join("agentsi.db")).unwrap());
    let product = Product {
        id: "p".into(),
        name: "P".into(),
        paths: vec![repo.to_string_lossy().into()],
        status: "active".into(),
        roles: vec!["pm".into(), "dev".into()],
    };
    let config = StoryConfig {
        repo: repo.to_string_lossy().into(),
        test: "node --test {files}".into(),
        gate: vec!["true".into()],
        budget_usd: 5.0,
    };
    let block = Owner {
        name: "Ada".into(),
        language: "English".into(),
    };
    let mut ctx = ProposeCtx::new(store, root, product, config, block);
    ctx.snapshot = Some("# Snapshot of p\n\nNothing yet.".into());
    let prompts: Arc<Mutex<Vec<String>>> = Arc::default();
    let seen = prompts.clone();
    ctx.caller = Arc::new(move |c: &Call, role: &str| {
        seen.lock().unwrap().push(c.prompt.clone());
        match &output {
            Some(v) => Ok((
                v.clone(),
                StoryUsage {
                    role: role.into(),
                    model: "opus".into(),
                    input_tokens: 1,
                    output_tokens: 1,
                    cache_read: 0,
                    cache_write: 0,
                    cost_usd: 0.25,
                    duration_ms: 10,
                },
            )),
            None => Err("claude exited with 1".into()),
        }
    });
    let f = Fixture {
        _dir: dir,
        ctx,
        prompts,
    };
    let product = f.ctx.product.clone();
    f.ctx
        .store
        .try_write(move |tx| {
            let p = TicketAddParams {
                product: "p".into(),
                kind: TicketKind::Todo,
                title: "Show the result".into(),
                body: Some("The coach wants the score.".into()),
                assignee: None,
                links: None,
                scope: None,
                objective: None,
                author: owner(),
            };
            write::add(tx, &product, &p, T0)
        })
        .unwrap()
        .unwrap();
    f
}

fn ticket(f: &Fixture) -> agentsi_proto::ticket_api::Ticket {
    f.ctx
        .store
        .read(|c| Ok(tickets::get(c, "p", "C001")))
        .unwrap()
        .unwrap()
}

#[test]
fn a_story_promote_links_the_new_story_and_closes_the_ticket() {
    let f = fixture(Some(story("Show the result")));
    let (t, s) = write::promote_story(&f.ctx, "C001", &owner(), T0 + 1000).unwrap();
    assert_eq!(s.title, "Show the result");
    assert_eq!(t.state, TicketState::Done);
    assert!(t.closed_at.is_some());
    assert_eq!(
        t.links,
        [Link {
            kind: LinkType::Story,
            reference: s.id.clone()
        }]
    );
    assert_eq!(ticket(&f), t);
    let prompts = f.prompts.lock().unwrap();
    assert!(
        prompts[0].contains("Show the result\n\nThe coach wants the score."),
        "{}",
        prompts[0]
    );
    // A closed ticket cannot become a story again.
    drop(prompts);
    let again = write::promote_story(&f.ctx, "C001", &owner(), T0 + 2000).unwrap_err();
    assert!(matches!(again.error, Error::Invalid(_)), "{again:?}");
    assert_eq!(again.story, None);
}

#[test]
fn a_failed_story_new_leaves_the_ticket_as_it_was() {
    let f = fixture(None);
    let before = ticket(&f);
    let err = write::promote_story(&f.ctx, "C001", &owner(), T0 + 1000).unwrap_err();
    assert!(!matches!(err.error, Error::Conflict(_)), "{err:?}");
    assert_eq!(err.story, None);
    assert_eq!(ticket(&f), before);
    let stories: i64 = f
        .ctx
        .store
        .read(|c| Ok(c.query_row("select count(*) from stories", [], |r| r.get(0))?))
        .unwrap();
    assert_eq!(stories, 0);
}

#[test]
fn a_ticket_moved_during_the_pm_call_stays_and_the_error_names_the_new_story() {
    let mut f = fixture(Some(story("Show the result")));
    let (store, product, pm) = (
        f.ctx.store.clone(),
        f.ctx.product.clone(),
        f.ctx.caller.clone(),
    );
    f.ctx.caller = Arc::new(move |c: &Call, role: &str| {
        let product = product.clone();
        store
            .try_write(move |tx| {
                let p = TicketMoveParams {
                    product: "p".into(),
                    id: "C001".into(),
                    state: TicketState::Dropped,
                    waiting_on: None,
                    by: owner(),
                };
                write::move_to(tx, &product, &p, T0 + 500)
            })
            .unwrap()
            .unwrap();
        pm(c, role)
    });
    let err = write::promote_story(&f.ctx, "C001", &owner(), T0 + 1000).unwrap_err();
    let story = err.story.clone().expect("the PM call saved a story");
    match &err.error {
        Error::Conflict(m) => assert!(m.contains(&format!("Story {story} was saved")), "{m}"),
        other => panic!("{other:?}"),
    }
    let t = ticket(&f);
    assert_eq!(t.state, TicketState::Dropped);
    assert!(t.links.is_empty(), "{:?}", t.links);
    let stories: i64 = f
        .ctx
        .store
        .read(|c| Ok(c.query_row("select count(*) from stories", [], |r| r.get(0))?))
        .unwrap();
    assert_eq!(stories, 1);
}

#[test]
fn a_session_cannot_promote_a_ticket_of_the_owner_and_no_pm_runs() {
    let f = fixture(Some(story("x")));
    let s1 = Author {
        by: AuthorBy::Session,
        id: "s1".into(),
    };
    let err = write::promote_story(&f.ctx, "C001", &s1, T0).unwrap_err();
    assert_eq!(
        err.error,
        Error::Conflict("Ticket C001 belongs to the owner.".into())
    );
    assert!(f.prompts.lock().unwrap().is_empty());
}

#[test]
fn a_decision_promote_adds_a_pending_decision_and_closes_the_ticket() {
    let f = fixture(None);
    let (t, id) = f
        .ctx
        .store
        .try_write(|tx| write::promote_decision(tx, "p", "C001", &owner(), T0 + 1000))
        .unwrap()
        .unwrap();
    assert_eq!(t.state, TicketState::Done);
    assert_eq!(t.links[0].reference, id.to_string());
    let row: (String, String, String, String) = f
        .ctx
        .store
        .read(move |c| {
            Ok(c.query_row(
                "select text, why, status, kind from decisions where id = ?1",
                [id],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)),
            )?)
        })
        .unwrap();
    assert_eq!(
        row,
        (
            "Show the result".into(),
            "The coach wants the score.".into(),
            "pending".into(),
            "product".into()
        )
    );
}
