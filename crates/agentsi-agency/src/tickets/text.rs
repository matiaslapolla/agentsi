//! The ticket line and the Markdown of `ticket.get`.

use agentsi_proto::ticket_api::{Author, Ticket};

/// The ticket line: `<id> [<kind>[, waiting on <waiting_on>]] <title>[ (for <assignee>)]`.
pub fn line(t: &Ticket) -> String {
    t.line()
}

fn who(a: &Author) -> String {
    if a.id.is_empty() {
        a.by.as_str().to_string()
    } else {
        format!("{} {}", a.by.as_str(), a.id)
    }
}

pub fn markdown(t: &Ticket) -> String {
    let state = match &t.waiting_on {
        Some(w) => format!("{} (on {w})", t.state.as_str()),
        None => t.state.as_str().to_string(),
    };
    let links = if t.links.is_empty() {
        "none".to_string()
    } else {
        t.links
            .iter()
            .map(|l| format!("{} {}", l.kind.as_str(), l.reference))
            .collect::<Vec<_>>()
            .join(", ")
    };
    let mut lines = vec![
        format!("# {} {}", t.id, t.title),
        String::new(),
        format!(
            "- Kind: {} · State: {state} · Assignee: {}",
            t.kind.as_str(),
            t.assignee.as_deref().unwrap_or("none")
        ),
        format!(
            "- Author: {} · Last write: {}",
            who(&t.author),
            who(&t.updated_by)
        ),
        format!("- Links: {links}"),
    ];
    let place: Vec<String> = [("Scope", &t.scope), ("Objective", &t.objective)]
        .into_iter()
        .filter_map(|(k, v)| v.as_ref().map(|v| format!("{k}: {v}")))
        .collect();
    if !place.is_empty() {
        lines.push(format!("- {}", place.join(" · ")));
    }
    let closed = t
        .closed_at
        .as_ref()
        .map(|c| format!(" · Closed: {c}"))
        .unwrap_or_default();
    lines.push(format!(
        "- Created: {} · Updated: {}{closed}",
        t.created_at, t.updated_at
    ));
    let body = t.body.trim_end();
    if !body.is_empty() {
        lines.extend([String::new(), body.to_string()]);
    }
    lines.join("\n") + "\n"
}
