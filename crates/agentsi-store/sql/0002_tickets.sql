-- The tracker: tickets of a product. Spec: docs/specs/agentsi-desktop-tracker.md.
-- A local table: Postgres has no copy of it, so the import never reads it.

create table tickets (
  product text not null,
  id text not null check (id glob 'C[0-9][0-9][0-9]*'),
  scope text,
  kind text not null check (kind in ('note','todo','question')),
  state text not null check (state in ('open','waiting','done','dropped')),
  title text not null,
  body text not null default '',
  waiting_on text,
  assignee text,
  author text not null check (json_valid(author)),
  updated_by text not null check (json_valid(updated_by)),
  links text not null default '[]' check (json_valid(links)),
  objective text,
  created_at text not null,
  updated_at text not null,
  closed_at text,
  primary key (product, id)
);
create index tickets_product_state_idx on tickets (product, state);
