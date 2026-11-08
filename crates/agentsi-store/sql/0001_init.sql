-- The final shape of the 8 Postgres migrations of packages/core/sql/.
-- Type map: docs/specs/agentsi-desktop-data.md. JSON is TEXT with json_valid,
-- timestamptz is ISO 8601 UTC TEXT, boolean is 0 or 1, numeric is REAL.

create table products (
  id text primary key,
  name text,
  paths text check (json_valid(paths)),
  status text
);

create table documents (
  id integer primary key autoincrement,
  product text not null,
  scope text,
  kind text not null check (kind in ('doc','journal','commit','issue','minutes','memory')),
  ref text not null,
  title text,
  body text,
  updated_at text,
  unique (product, kind, ref)
);
create index documents_product_kind_idx on documents (product, kind);
create index documents_product_scope_idx on documents (product, scope);

-- Takes the place of the GIN index documents_tsv_idx.
create virtual table documents_fts using fts5(
  title, body,
  content='documents', content_rowid='id',
  tokenize='unicode61 remove_diacritics 2'
);
create trigger documents_fts_insert after insert on documents begin
  insert into documents_fts (rowid, title, body) values (new.id, new.title, new.body);
end;
create trigger documents_fts_delete after delete on documents begin
  insert into documents_fts (documents_fts, rowid, title, body) values ('delete', old.id, old.title, old.body);
end;
create trigger documents_fts_update after update on documents begin
  insert into documents_fts (documents_fts, rowid, title, body) values ('delete', old.id, old.title, old.body);
  insert into documents_fts (rowid, title, body) values (new.id, new.title, new.body);
end;

create table index_state (
  product text not null,
  path text not null,
  last_commit text,
  indexed_at text,
  primary key (product, path)
);

create table minutes (
  id text primary key,
  product text not null,
  scope text,
  question text,
  status text check (status in ('valid','invalid')),
  created_at text,
  file text,
  data text check (json_valid(data)),
  usage text check (json_valid(usage))
);
create index minutes_product_idx on minutes (product, created_at desc);

create table priorities (
  minutes_id text not null,
  idx integer not null,
  title text,
  why text,
  sources text check (json_valid(sources)),
  owner_role text,
  mark text check (mark in ('accepted','changed','rejected')),
  is_new integer default 0 check (is_new in (0, 1)),
  objective text,
  primary key (minutes_id, idx)
);

create table decisions (
  id integer primary key autoincrement,
  product text not null,
  minutes_id text,
  text text,
  why text,
  status text default 'pending' check (status in ('pending','approved','rejected','stale')),
  decided_at text,
  kind text not null default 'minutes' check (kind in ('minutes','reflection','product')),
  reflection_id text,
  action text
);
create index decisions_product_status_idx on decisions (product, status);

create table usage (
  minutes_id text not null,
  role text,
  model text,
  input_tokens integer,
  output_tokens integer,
  cache_read integer,
  cache_write integer,
  cost_usd real,
  duration_ms integer
);
create index usage_minutes_idx on usage (minutes_id);

create table stories (
  product text not null,
  id text not null,
  data text not null check (json_valid(data)),
  primary key (product, id)
);

create table story_logs (
  product text not null,
  story_id text not null,
  body text not null default '',
  primary key (product, story_id)
);

create table memory (
  product text not null,
  agent text not null,
  id text not null,
  position integer not null,
  text text not null,
  kind text not null,
  helpful integer not null default 0,
  harmful integer not null default 0,
  source text not null,
  created_at text not null,
  retired_at text,
  retired_why text,
  primary key (product, agent, id)
);
create index memory_product_agent_idx on memory (product, agent, position);

create table agent_overlays (
  product text not null,
  agent text not null,
  body text not null,
  primary key (product, agent)
);

create table briefs (
  product text primary key,
  data text not null check (json_valid(data))
);

-- position was a bigserial: the store writes max + 1.
create table meeting_log (
  product text not null,
  minutes_id text not null,
  position integer not null,
  entry text not null check (json_valid(entry)),
  primary key (product, minutes_id)
);
create index meeting_log_product_idx on meeting_log (product, position);

create table snapshots (
  product text not null,
  minutes_id text not null,
  body text not null,
  primary key (product, minutes_id)
);

create table listens (
  product text not null,
  minutes_id text not null,
  at text not null,
  primary key (product, minutes_id)
);

create table reflections (
  product text not null,
  id text not null,
  position integer not null,
  entry text not null check (json_valid(entry)),
  primary key (product, id)
);
create index reflections_product_idx on reflections (product, position);

create table archive_records (
  product text not null,
  kind text not null check (kind in ('minutes','decision','story','product')),
  ref text not null,
  primary key (product, kind, ref)
);
