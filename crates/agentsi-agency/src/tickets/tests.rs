use super::*;
use crate::registry::Product;
use agentsi_proto::ticket_api::{
    AuthorBy, LinkType, TicketAddParams, TicketMoveParams, TicketUpdateParams,
};
use agentsi_store::Store;
use write::{add, move_to, update};

const T0: i64 = 1_788_000_000_000;

fn product() -> Product {
    Product {
        id: "p".into(),
        name: "P".into(),
        paths: vec![],
        status: "active".into(),
        roles: vec!["pm".into(), "dev.backend".into()],
    }
}

fn store() -> (tempfile::TempDir, Store) {
    let dir = tempfile::tempdir().unwrap();
    let store = Store::open(&dir.path().join("agentsi.db")).unwrap();
    store
        .with_writer(|c| {
            c.execute_batch(
                r#"insert into stories values ('p', 'S014', '{}'), ('q', 'S015', '{}');
                   insert into minutes (id, product, created_at, data) values
                     ('m1', 'p', '2026-11-01T00:00:00.000Z', '{}'),
                     ('mq', 'q', '2026-11-01T00:00:00.000Z', '{}');
                   insert into priorities (minutes_id, idx) values ('m1', 0), ('mq', 0);
                   insert into decisions (id, product, text) values (42, 'p', 'd'), (43, 'q', 'd');
                   insert into briefs values ('p', '{"objectives":[{"id":"o1"}]}');"#,
            )?;
            Ok(())
        })
        .unwrap();
    (dir, store)
}

fn owner() -> Author {
    Author {
        by: AuthorBy::Owner,
        id: "Matias".into(),
    }
}

fn session(id: &str) -> Author {
    Author {
        by: AuthorBy::Session,
        id: id.into(),
    }
}

fn add_params(title: &str, author: Author) -> TicketAddParams {
    TicketAddParams {
        product: "p".into(),
        kind: TicketKind::Todo,
        title: title.into(),
        body: None,
        assignee: None,
        links: None,
        scope: None,
        objective: None,
        author,
    }
}

fn do_add(store: &Store, p: TicketAddParams) -> Result<Ticket> {
    store
        .try_write(move |tx| add(tx, &product(), &p, T0))
        .unwrap()
}

fn do_move(
    store: &Store,
    id: &str,
    state: TicketState,
    waiting_on: Option<&str>,
    by: Author,
) -> Result<write::Moved> {
    let p = TicketMoveParams {
        product: "p".into(),
        id: id.into(),
        state,
        waiting_on: waiting_on.map(String::from),
        by,
    };
    store
        .try_write(move |tx| move_to(tx, &product(), &p, T0 + 1000))
        .unwrap()
}

fn update_params(id: &str, by: Author) -> TicketUpdateParams {
    TicketUpdateParams {
        product: "p".into(),
        id: id.into(),
        title: None,
        body: None,
        kind: None,
        assignee: None,
        links: None,
        scope: None,
        objective: None,
        by,
    }
}

fn do_update(store: &Store, p: TicketUpdateParams) -> Result<Ticket> {
    store
        .try_write(move |tx| update(tx, &product(), &p, T0 + 2000))
        .unwrap()
}

fn link(kind: LinkType, r: &str) -> Link {
    Link {
        kind,
        reference: r.into(),
    }
}

#[test]
fn tickets_add_takes_the_next_id_and_the_open_state() {
    let (_dir, store) = store();
    let a = do_add(&store, add_params("  First  ", owner())).unwrap();
    assert_eq!((a.id.as_str(), a.title.as_str()), ("C001", "First"));
    assert_eq!(a.state, TicketState::Open);
    assert_eq!(a.author, a.updated_by);
    assert_eq!(a.created_at, "2026-08-29T10:40:00.000Z");
    let b = do_add(&store, add_params("Second", owner())).unwrap();
    assert_eq!(b.id, "C002");
    let got = store.read(|c| Ok(get(c, "p", "C001"))).unwrap().unwrap();
    assert_eq!(got, a);
}

#[test]
fn tickets_the_id_after_c999_is_c1000_and_each_product_counts_alone() {
    let (_dir, store) = store();
    store
        .with_writer(|c| {
            c.execute(
                "insert into tickets (product, id, kind, state, title, author, updated_by, created_at, updated_at)
                 values ('p', 'C999', 'note', 'done', 't', '{\"by\":\"owner\",\"id\":\"o\"}', '{\"by\":\"owner\",\"id\":\"o\"}', 'x', 'x')",
                [],
            )?;
            Ok(())
        })
        .unwrap();
    assert_eq!(
        do_add(&store, add_params("t", owner())).unwrap().id,
        "C1000"
    );
    assert_eq!(
        do_add(&store, add_params("t", owner())).unwrap().id,
        "C1001"
    );
    let other = Product {
        id: "q".into(),
        ..product()
    };
    let p = TicketAddParams {
        product: "q".into(),
        ..add_params("t", owner())
    };
    let q = store
        .try_write(move |tx| add(tx, &other, &p, T0))
        .unwrap()
        .unwrap();
    assert_eq!(q.id, "C001");
}

#[test]
fn tickets_bad_values_are_refused_and_write_nothing() {
    let (_dir, store) = store();
    let invalid = |p: TicketAddParams| matches!(do_add(&store, p), Err(Error::Invalid(_)));
    assert!(invalid(add_params("   ", owner())));
    assert!(invalid(add_params(&"x".repeat(121), owner())));
    assert!(do_add(&store, add_params(&"é".repeat(120), owner())).is_ok());
    assert!(invalid(TicketAddParams {
        assignee: Some("seo".into()),
        ..add_params("t", owner())
    }));
    assert!(invalid(add_params("t", session("  "))));
    assert!(matches!(
        do_add(
            &store,
            TicketAddParams {
                objective: Some("o9".into()),
                ..add_params("t", owner())
            }
        ),
        Err(Error::NotFound(_))
    ));
    let n: i64 = store
        .read(|c| Ok(c.query_row("select count(*) from tickets", [], |r| r.get(0))?))
        .unwrap();
    assert_eq!(n, 1);
}

#[test]
fn tickets_the_assignee_is_owner_claude_or_an_agent_of_the_roster() {
    let (_dir, store) = store();
    for a in ["owner", "claude", "dev.backend", "pm"] {
        let t = do_add(
            &store,
            TicketAddParams {
                assignee: Some(a.into()),
                ..add_params("t", owner())
            },
        )
        .unwrap();
        assert_eq!(t.assignee.as_deref(), Some(a));
    }
    let t = do_add(
        &store,
        TicketAddParams {
            assignee: Some(" ".into()),
            ..add_params("t", owner())
        },
    )
    .unwrap();
    assert_eq!(t.assignee, None);
}

#[test]
fn tickets_each_link_type_is_checked() {
    let (_dir, store) = store();
    do_add(&store, add_params("target", owner())).unwrap();
    let ok = [
        link(LinkType::Story, "S014"),
        link(LinkType::Decision, "42"),
        link(LinkType::Minutes, "m1"),
        link(LinkType::Priority, "m1#0"),
        link(LinkType::Ticket, "C001"),
        link(LinkType::Url, "https://example.com/x"),
        link(LinkType::Commit, "1a2b3c4"),
        link(LinkType::Path, "src/main.rs"),
        link(LinkType::Issue, "12"),
    ];
    let t = do_add(
        &store,
        TicketAddParams {
            links: Some(ok.to_vec()),
            ..add_params("t", owner())
        },
    )
    .unwrap();
    assert_eq!(t.links, ok);
    let refused = |l: Link| {
        do_add(
            &store,
            TicketAddParams {
                links: Some(vec![l]),
                ..add_params("t", owner())
            },
        )
        .unwrap_err()
    };
    let not_found = [
        link(LinkType::Story, "S015"),
        link(LinkType::Decision, "43"),
        link(LinkType::Minutes, "mq"),
        link(LinkType::Priority, "m1#1"),
        link(LinkType::Priority, "mq#0"),
        link(LinkType::Ticket, "C099"),
    ];
    for l in not_found {
        assert!(matches!(refused(l.clone()), Error::NotFound(_)), "{l:?}");
    }
    let invalid = [
        link(LinkType::Decision, "x"),
        link(LinkType::Priority, "m1"),
        link(LinkType::Url, "ftp://x"),
        link(LinkType::Commit, " "),
    ];
    for l in invalid {
        assert!(matches!(refused(l.clone()), Error::Invalid(_)), "{l:?}");
    }
}

#[test]
fn tickets_each_move_of_the_state_table() {
    let (_dir, store) = store();
    do_add(&store, add_params("t", owner())).unwrap();
    let mv = |state, w: Option<&str>| do_move(&store, "C001", state, w, owner());

    assert!(matches!(
        mv(TicketState::Waiting, None),
        Err(Error::Invalid(_))
    ));
    let t = mv(TicketState::Waiting, Some("Matias")).unwrap().ticket;
    assert_eq!(
        (t.state, t.waiting_on.as_deref()),
        (TicketState::Waiting, Some("Matias"))
    );

    let t = mv(TicketState::Open, None).unwrap().ticket;
    assert_eq!((t.state, t.waiting_on), (TicketState::Open, None));

    mv(TicketState::Waiting, Some("CI")).unwrap();
    let t = mv(TicketState::Done, None).unwrap().ticket;
    assert_eq!(t.state, TicketState::Done);
    assert_eq!(t.waiting_on, None);
    assert!(t.closed_at.is_some());

    for bad in [TicketState::Waiting, TicketState::Dropped] {
        assert!(
            matches!(
                mv(bad, (bad == TicketState::Waiting).then_some("x")),
                Err(Error::Invalid(_))
            ),
            "{bad:?}"
        );
    }
    let t = mv(TicketState::Open, None).unwrap().ticket;
    assert_eq!((t.state, t.closed_at), (TicketState::Open, None));

    let t = mv(TicketState::Dropped, None).unwrap().ticket;
    assert!(t.closed_at.is_some());
    assert!(matches!(
        mv(TicketState::Done, None),
        Err(Error::Invalid(_))
    ));
    assert!(matches!(
        mv(TicketState::Open, Some("x")),
        Err(Error::Invalid(_))
    ));
}

#[test]
fn tickets_a_move_to_the_same_state_changes_only_a_new_waiting_on() {
    let (_dir, store) = store();
    do_add(&store, add_params("t", owner())).unwrap();
    let same = do_move(&store, "C001", TicketState::Open, None, owner()).unwrap();
    assert!(!same.changed);
    assert_eq!(same.ticket.updated_at, "2026-08-29T10:40:00.000Z");
    do_move(&store, "C001", TicketState::Waiting, Some("A"), owner()).unwrap();
    assert!(
        !do_move(&store, "C001", TicketState::Waiting, None, owner())
            .unwrap()
            .changed
    );
    assert!(
        !do_move(&store, "C001", TicketState::Waiting, Some("A"), owner())
            .unwrap()
            .changed
    );
    let again = do_move(&store, "C001", TicketState::Waiting, Some("B"), owner()).unwrap();
    assert!(again.changed);
    assert_eq!(again.ticket.waiting_on.as_deref(), Some("B"));
}

#[test]
fn tickets_a_session_changes_only_its_own_tickets_and_the_tickets_of_claude() {
    let (_dir, store) = store();
    do_add(&store, add_params("of the owner", owner())).unwrap();
    do_add(&store, add_params("of s1", session("s1"))).unwrap();
    do_add(
        &store,
        TicketAddParams {
            assignee: Some("claude".into()),
            ..add_params("for claude", owner())
        },
    )
    .unwrap();
    do_add(&store, add_params("of unknown", session("unknown"))).unwrap();

    let conflict = |id: &str, who: &str| {
        let e = do_move(&store, id, TicketState::Dropped, None, session(who)).err();
        match e {
            Some(Error::Conflict(m)) => Some(m),
            _ => None,
        }
    };
    assert_eq!(
        conflict("C001", "s1").as_deref(),
        Some("Ticket C001 belongs to the owner.")
    );
    assert_eq!(
        conflict("C002", "s2").as_deref(),
        Some("Ticket C002 belongs to the owner.")
    );
    assert_eq!(
        conflict("C004", "unknown").as_deref(),
        Some("Ticket C004 belongs to the owner.")
    );
    assert!(conflict("C002", "s1").is_none());
    assert!(conflict("C003", "s9").is_none());

    let mut p = update_params("C001", session("s1"));
    p.title = Some("taken".into());
    assert!(matches!(do_update(&store, p), Err(Error::Conflict(_))));
    let t = store.read(|c| Ok(get(c, "p", "C001"))).unwrap().unwrap();
    assert_eq!(
        (t.title.as_str(), t.state),
        ("of the owner", TicketState::Open)
    );

    // The owner changes each ticket.
    let mut p = update_params("C002", owner());
    p.title = Some("renamed".into());
    let t = do_update(&store, p).unwrap();
    assert_eq!(t.updated_by, owner());
    assert_eq!(t.author, session("s1"));
}

#[test]
fn tickets_update_changes_only_the_given_fields_and_empty_clears() {
    let (_dir, store) = store();
    do_add(
        &store,
        TicketAddParams {
            body: Some("b".into()),
            assignee: Some("pm".into()),
            scope: Some("web".into()),
            objective: Some("o1".into()),
            links: Some(vec![link(LinkType::Story, "S014")]),
            ..add_params("t", owner())
        },
    )
    .unwrap();
    let mut p = update_params("C001", owner());
    p.kind = Some(TicketKind::Question);
    let t = do_update(&store, p).unwrap();
    assert_eq!(t.kind, TicketKind::Question);
    assert_eq!((t.body.as_str(), t.assignee.as_deref()), ("b", Some("pm")));
    assert_eq!(t.updated_at, "2026-08-29T10:40:02.000Z");

    let mut p = update_params("C001", owner());
    p.assignee = Some(String::new());
    p.scope = Some(String::new());
    p.objective = Some(String::new());
    p.links = Some(vec![]);
    let t = do_update(&store, p).unwrap();
    assert_eq!((t.assignee, t.scope, t.objective), (None, None, None));
    assert!(t.links.is_empty());
    assert_eq!(t.body, "b");

    let p = update_params("C404", owner());
    assert!(matches!(do_update(&store, p), Err(Error::NotFound(_))));
}

#[test]
fn tickets_list_orders_by_state_then_newest_then_highest_id_and_filters() {
    let (_dir, store) = store();
    for (title, author) in [
        ("a", owner()),
        ("b", session("s1")),
        ("c", owner()),
        ("d", owner()),
    ] {
        do_add(&store, add_params(title, author)).unwrap();
    }
    do_move(&store, "C001", TicketState::Done, None, owner()).unwrap();
    do_move(&store, "C002", TicketState::Waiting, Some("x"), owner()).unwrap();
    do_move(&store, "C004", TicketState::Dropped, None, owner()).unwrap();
    store
        .with_writer(|c| {
            c.execute("update tickets set links = '[{\"type\":\"story\",\"ref\":\"S014\"}]' where id = 'C003'", [])?;
            Ok(())
        })
        .unwrap();
    let ids = |p: TicketListParams| -> Vec<String> {
        store
            .read(move |c| Ok(list(c, &p)))
            .unwrap()
            .unwrap()
            .into_iter()
            .map(|t| t.id)
            .collect()
    };
    let base = || TicketListParams {
        product: "p".into(),
        ..Default::default()
    };
    assert_eq!(ids(base()), ["C003", "C002", "C001"]);
    assert_eq!(
        ids(TicketListParams {
            states: Some(TicketState::ALL.to_vec()),
            ..base()
        }),
        ["C003", "C002", "C001", "C004"]
    );
    assert_eq!(
        ids(TicketListParams {
            author: Some(AuthorBy::Session),
            ..base()
        }),
        ["C002"]
    );
    assert_eq!(
        ids(TicketListParams {
            link: Some(link(LinkType::Story, "S014")),
            ..base()
        }),
        ["C003"]
    );
    assert_eq!(
        ids(TicketListParams {
            limit: Some(1),
            ..base()
        }),
        ["C003"]
    );
    let bad = store
        .read(|c| {
            Ok(list(
                c,
                &TicketListParams {
                    limit: Some(201),
                    ..base()
                },
            ))
        })
        .unwrap();
    assert!(matches!(bad, Err(Error::Invalid(_))));
    // The same updated_at: the highest id comes first.
    do_add(&store, add_params("e", owner())).unwrap();
    do_add(&store, add_params("f", owner())).unwrap();
    assert_eq!(ids(base())[..3], ["C006", "C005", "C003"]);
}

#[test]
fn tickets_open_keeps_the_scope_and_the_tickets_with_no_scope() {
    let (_dir, store) = store();
    for scope in [None, Some("web"), Some("app")] {
        do_add(
            &store,
            TicketAddParams {
                scope: scope.map(String::from),
                ..add_params("t", owner())
            },
        )
        .unwrap();
    }
    let ids = |scope: Option<&'static str>| -> Vec<String> {
        store
            .read(move |c| Ok(open(c, "p", scope)))
            .unwrap()
            .unwrap()
            .into_iter()
            .map(|t| t.id)
            .collect()
    };
    assert_eq!(ids(None), ["C003", "C002", "C001"]);
    assert_eq!(ids(Some("web")), ["C002", "C001"]);
}

#[test]
fn tickets_the_line_and_the_markdown() {
    let (_dir, store) = store();
    do_add(
        &store,
        TicketAddParams {
            body: Some("Run the bundle.\n".into()),
            assignee: Some("owner".into()),
            links: Some(vec![
                link(LinkType::Story, "S014"),
                link(LinkType::Commit, "1a2b3c4"),
            ]),
            ..add_params("Install the aarch64 bundle", session("3f2a"))
        },
    )
    .unwrap();
    let t = do_move(
        &store,
        "C001",
        TicketState::Waiting,
        Some("Matias"),
        owner(),
    )
    .unwrap()
    .ticket;
    assert_eq!(
        text::line(&t),
        "C001 [todo, waiting on Matias] Install the aarch64 bundle (for owner)"
    );
    assert_eq!(
        text::markdown(&t),
        "# C001 Install the aarch64 bundle

- Kind: todo · State: waiting (on Matias) · Assignee: owner
- Author: session 3f2a · Last write: owner Matias
- Links: story S014, commit 1a2b3c4
- Created: 2026-08-29T10:40:00.000Z · Updated: 2026-08-29T10:40:01.000Z

Run the bundle.
"
    );
    let plain = do_add(&store, add_params("Plain", owner())).unwrap();
    assert_eq!(text::line(&plain), "C002 [todo] Plain");
}
