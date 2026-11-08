-- Minutes, usage, the meeting log, listens, reflections, snapshots and archive records move from the data root into Postgres.
-- `data` holds the minutes JSON as it was written; `usage` holds the per-role usage array of the meeting.
-- Rows written before this migration have both null until `agentsi data import` fills them from the old files.
alter table minutes add column data jsonb;
alter table minutes add column usage jsonb;

create table meeting_log (
  product text not null,
  minutes_id text not null,
  position bigserial,
  entry jsonb not null,
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
  position int not null,
  entry jsonb not null,
  primary key (product, id)
);
create index reflections_product_idx on reflections (product, position);

-- One row per archived thing. `kind` is minutes, decision, story or product (`ref` is '' for the product).
create table archive_records (
  product text not null,
  kind text not null check (kind in ('minutes', 'decision', 'story', 'product')),
  ref text not null,
  primary key (product, kind, ref)
);
