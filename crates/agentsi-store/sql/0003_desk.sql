-- The desk conversation: one desk session for each product and its stored items.
-- Spec: docs/specs/agentsi-desktop-conversation.md. Local tables: the import never reads them.

create table desk_sessions (
  product text primary key,
  session text not null unique,
  created_at text not null
);

create table desk_items (
  product text not null,
  seq integer not null,
  kind text not null check (kind in ('owner','reply','tool','view','note','end','error')),
  body text not null check (json_valid(body)),
  sent integer not null default 0 check (sent in (0, 1)),
  at text not null,
  primary key (product, seq)
);
