//! The tickets part of the front desk: the open tickets for the owner and the session tickets
//! of the last day, 8 rows at most. A click opens a ticket. A button opens the Tickets screen.
//! It is one child view of the front desk, so the desk module keeps its own file.

use agentsi_proto::ticket_api::{AuthorBy, Ticket, TicketList, TicketListParams, TicketState};
use agentsi_ui::components::{Button, ListRow};
use agentsi_ui::tokens::{size, space, text};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;

use super::parts::section;
use super::ticket::{kind_tag, state_tag};
use super::tickets::meta;
use crate::format;
use crate::model::CoreState;
use crate::nav::{Nav, ScreenRequest};

/// The most rows that the front desk shows.
const ROWS: usize = 8;
const DAY: i64 = 86_400;

/// The tickets to show, in the order of `ticket.list`: open tickets assigned to the owner, and
/// open tickets that a session wrote in the last 24 hours.
pub fn shown(tickets: &[Ticket], now: i64) -> Vec<&Ticket> {
    tickets
        .iter()
        .filter(|t| t.state.is_open())
        .filter(|t| {
            t.assignee.as_deref() == Some("owner")
                || (t.author.by == AuthorBy::Session
                    && format::unix_secs(&t.created_at).is_none_or(|c| c >= now - DAY))
        })
        .take(ROWS)
        .collect()
}

pub struct DeskTickets {
    product: String,
    core: Entity<CoreState>,
    tickets: Vec<Ticket>,
}

impl DeskTickets {
    /// The desk calls `reload` after it asks for its own data.
    pub fn new(product: String, core: Entity<CoreState>, _: &mut Context<Self>) -> Self {
        DeskTickets {
            product,
            core,
            tickets: Vec::new(),
        }
    }

    pub fn reload(&mut self, cx: &mut Context<Self>) {
        let call = self
            .core
            .read(cx)
            .link
            .call::<TicketList>(&TicketListParams {
                product: self.product.clone(),
                states: Some(vec![TicketState::Open, TicketState::Waiting]),
                limit: Some(200),
                ..TicketListParams::default()
            });
        cx.spawn(async move |this, cx| {
            // A core that cannot read the tickets leaves the part empty. The desk shows the rest.
            let tickets = call.await.map(|r| r.tickets).unwrap_or_default();
            let _ = this.update(cx, |this, cx| {
                if this.tickets != tickets {
                    this.tickets = tickets;
                    cx.notify();
                }
            });
        })
        .detach();
    }
}

impl Render for DeskTickets {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let now = format::now_secs();
        let rows = shown(&self.tickets, now);
        let count = rows.len();
        let product = self.product.clone();
        div()
            .flex()
            .flex_col()
            .gap(space(3))
            .child(
                div()
                    .flex()
                    .items_center()
                    .justify_between()
                    .child(section("Tickets", Some(count)))
                    .child(
                        Button::new("desk-all-tickets")
                            .secondary()
                            .small()
                            .label("All tickets")
                            .on_click(cx.listener(move |this, _, _, cx| {
                                Nav::open(cx, &this.core, ScreenRequest::tickets(&product));
                            })),
                    ),
            )
            .when(rows.is_empty(), |el| {
                el.child(
                    div()
                        .text_size(size::sm())
                        .text_color(text::tertiary())
                        .child("No ticket waits for you."),
                )
            })
            .children(rows.into_iter().enumerate().map(|(i, t)| {
                let id = t.id.clone();
                ListRow::new(("desk-ticket", i), t.title.clone())
                    .meta(meta(t, now))
                    .trailing(
                        div()
                            .flex()
                            .gap(space(2))
                            .child(kind_tag(t.kind))
                            .child(state_tag(t)),
                    )
                    .on_click(cx.listener(move |this, _, _, cx| {
                        Nav::open(cx, &this.core, ScreenRequest::ticket(&this.product, &id));
                    }))
            }))
    }
}

#[cfg(test)]
mod tests {
    use super::shown;
    use crate::format;
    use agentsi_proto::ticket_api::Ticket;
    use serde_json::json;

    fn ticket(id: &str, state: &str, assignee: Option<&str>, by: &str, created: &str) -> Ticket {
        serde_json::from_value(json!({
            "product": "demo", "id": id, "scope": null, "kind": "todo", "state": state,
            "title": id, "body": "", "waiting_on": null, "assignee": assignee,
            "author": {"by": by, "id": ""}, "updated_by": {"by": by, "id": ""},
            "links": [], "objective": null,
            "created_at": created, "updated_at": created, "closed_at": null
        }))
        .unwrap()
    }

    #[test]
    fn the_desk_shows_the_tickets_for_the_owner_and_the_session_tickets_of_the_last_day() {
        let now = format::unix_secs("2026-11-09T12:00:00.000Z").unwrap();
        let tickets = vec![
            ticket(
                "C001",
                "open",
                Some("owner"),
                "owner",
                "2026-10-01T10:00:00.000Z",
            ),
            ticket("C002", "open", None, "owner", "2026-11-09T10:00:00.000Z"),
            ticket("C003", "open", None, "session", "2026-11-09T10:00:00.000Z"),
            ticket(
                "C004",
                "waiting",
                None,
                "session",
                "2026-11-01T10:00:00.000Z",
            ),
            ticket(
                "C005",
                "done",
                Some("owner"),
                "owner",
                "2026-11-09T10:00:00.000Z",
            ),
            ticket(
                "C006",
                "open",
                Some("claude"),
                "owner",
                "2026-11-09T10:00:00.000Z",
            ),
        ];
        let ids: Vec<_> = shown(&tickets, now).iter().map(|t| t.id.as_str()).collect();
        assert_eq!(ids, ["C001", "C003"]);
    }

    #[test]
    fn the_desk_shows_eight_rows_at_most() {
        let now = format::unix_secs("2026-11-09T12:00:00.000Z").unwrap();
        let tickets: Vec<_> = (1..=12)
            .map(|n| {
                ticket(
                    &format!("C{n:03}"),
                    "open",
                    Some("owner"),
                    "owner",
                    "2026-11-09T10:00:00.000Z",
                )
            })
            .collect();
        assert_eq!(shown(&tickets, now).len(), 8);
    }
}
