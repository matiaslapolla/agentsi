//! The tables, their columns and their keys, as data. `TABLES` holds the 17
//! tables of Postgres, and the import reads only that list. `LOCAL_TABLES`
//! holds the tables that exist only in SQLite. A test checks both lists
//! against the SQL schema.

/// How a column maps from Postgres to SQLite (the type map of the data spec).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    Text,
    /// `jsonb` or `json` in Postgres, JSON text in SQLite.
    Json,
    /// `timestamptz` in Postgres, ISO 8601 UTC text in SQLite.
    Time,
    /// `boolean` in Postgres, 0 or 1 in SQLite.
    Bool,
    Int,
    /// `numeric` in Postgres, REAL in SQLite.
    Real,
}

#[derive(Debug)]
pub struct Column {
    pub name: &'static str,
    pub kind: Kind,
}

#[derive(Debug)]
pub struct Table {
    pub name: &'static str,
    pub columns: &'static [Column],
    /// The primary key. `usage` has none in Postgres, so it is empty there.
    pub key: &'static [&'static str],
}

impl Table {
    pub fn column(&self, name: &str) -> Option<usize> {
        self.columns.iter().position(|c| c.name == name)
    }
}

const fn c(name: &'static str, kind: Kind) -> Column {
    Column { name, kind }
}

use Kind::*;

/// In import order: `minutes` comes first, because its renamed ids change
/// the rows of the other tables.
pub const TABLES: &[Table] = &[
    Table {
        name: "minutes",
        columns: &[
            c("id", Text),
            c("product", Text),
            c("scope", Text),
            c("question", Text),
            c("status", Text),
            c("created_at", Time),
            c("file", Text),
            c("data", Json),
            c("usage", Json),
        ],
        key: &["id"],
    },
    Table {
        name: "products",
        columns: &[
            c("id", Text),
            c("name", Text),
            c("paths", Json),
            c("status", Text),
        ],
        key: &["id"],
    },
    Table {
        name: "documents",
        columns: &[
            c("id", Int),
            c("product", Text),
            c("scope", Text),
            c("kind", Text),
            c("ref", Text),
            c("title", Text),
            c("body", Text),
            c("updated_at", Time),
        ],
        key: &["id"],
    },
    Table {
        name: "index_state",
        columns: &[
            c("product", Text),
            c("path", Text),
            c("last_commit", Text),
            c("indexed_at", Time),
        ],
        key: &["product", "path"],
    },
    Table {
        name: "priorities",
        columns: &[
            c("minutes_id", Text),
            c("idx", Int),
            c("title", Text),
            c("why", Text),
            c("sources", Json),
            c("owner_role", Text),
            c("mark", Text),
            c("is_new", Bool),
            c("objective", Text),
        ],
        key: &["minutes_id", "idx"],
    },
    Table {
        name: "decisions",
        columns: &[
            c("id", Int),
            c("product", Text),
            c("minutes_id", Text),
            c("text", Text),
            c("why", Text),
            c("status", Text),
            c("decided_at", Time),
            c("kind", Text),
            c("reflection_id", Text),
            c("action", Text),
        ],
        key: &["id"],
    },
    Table {
        name: "usage",
        columns: &[
            c("minutes_id", Text),
            c("role", Text),
            c("model", Text),
            c("input_tokens", Int),
            c("output_tokens", Int),
            c("cache_read", Int),
            c("cache_write", Int),
            c("cost_usd", Real),
            c("duration_ms", Int),
        ],
        key: &[],
    },
    Table {
        name: "stories",
        columns: &[c("product", Text), c("id", Text), c("data", Json)],
        key: &["product", "id"],
    },
    Table {
        name: "story_logs",
        columns: &[c("product", Text), c("story_id", Text), c("body", Text)],
        key: &["product", "story_id"],
    },
    Table {
        name: "memory",
        columns: &[
            c("product", Text),
            c("agent", Text),
            c("id", Text),
            c("position", Int),
            c("text", Text),
            c("kind", Text),
            c("helpful", Int),
            c("harmful", Int),
            c("source", Text),
            c("created_at", Text),
            c("retired_at", Text),
            c("retired_why", Text),
        ],
        key: &["product", "agent", "id"],
    },
    Table {
        name: "agent_overlays",
        columns: &[c("product", Text), c("agent", Text), c("body", Text)],
        key: &["product", "agent"],
    },
    Table {
        name: "briefs",
        columns: &[c("product", Text), c("data", Json)],
        key: &["product"],
    },
    Table {
        name: "meeting_log",
        columns: &[
            c("product", Text),
            c("minutes_id", Text),
            c("position", Int),
            c("entry", Json),
        ],
        key: &["product", "minutes_id"],
    },
    Table {
        name: "snapshots",
        columns: &[c("product", Text), c("minutes_id", Text), c("body", Text)],
        key: &["product", "minutes_id"],
    },
    Table {
        name: "listens",
        columns: &[c("product", Text), c("minutes_id", Text), c("at", Text)],
        key: &["product", "minutes_id"],
    },
    Table {
        name: "reflections",
        columns: &[
            c("product", Text),
            c("id", Text),
            c("position", Int),
            c("entry", Json),
        ],
        key: &["product", "id"],
    },
    Table {
        name: "archive_records",
        columns: &[c("product", Text), c("kind", Text), c("ref", Text)],
        key: &["product", "kind", "ref"],
    },
];

/// The tables that only SQLite has. A module that adds a local table adds it here.
pub const LOCAL_TABLES: &[Table] = &[
    Table {
        name: "tickets",
        columns: &[
            c("product", Text),
            c("id", Text),
            c("scope", Text),
            c("kind", Text),
            c("state", Text),
            c("title", Text),
            c("body", Text),
            c("waiting_on", Text),
            c("assignee", Text),
            c("author", Json),
            c("updated_by", Json),
            c("links", Json),
            c("objective", Text),
            c("created_at", Time),
            c("updated_at", Time),
            c("closed_at", Time),
        ],
        key: &["product", "id"],
    },
    Table {
        name: "desk_sessions",
        columns: &[
            c("product", Text),
            c("session", Text),
            c("created_at", Time),
        ],
        key: &["product"],
    },
    Table {
        name: "desk_items",
        columns: &[
            c("product", Text),
            c("seq", Int),
            c("kind", Text),
            c("body", Json),
            c("sent", Bool),
            c("at", Time),
        ],
        key: &["product", "seq"],
    },
];

/// Each table of SQLite: the tables of Postgres, then the local tables.
pub fn all() -> impl Iterator<Item = &'static Table> {
    TABLES.iter().chain(LOCAL_TABLES)
}

pub fn table(name: &str) -> Option<&'static Table> {
    all().find(|t| t.name == name)
}
