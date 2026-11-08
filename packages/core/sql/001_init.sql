create table products (
  id text primary key,
  name text,
  paths jsonb,
  status text
);

create table documents (
  id bigserial primary key,
  product text not null,
  scope text,
  kind text not null check (kind in ('doc','journal','commit','issue','minutes','memory')),
  ref text not null,
  title text,
  body text,
  updated_at timestamptz,
  tsv tsvector generated always as (
    setweight(to_tsvector('simple', coalesce(title, '')), 'A') ||
    setweight(to_tsvector('simple', coalesce(body, '')), 'B')
  ) stored,
  unique (product, kind, ref)
);
create index documents_tsv_idx on documents using gin (tsv);
create index documents_product_kind_idx on documents (product, kind);
create index documents_product_scope_idx on documents (product, scope);

create table index_state (
  product text not null,
  path text not null,
  last_commit text,
  indexed_at timestamptz,
  primary key (product, path)
);

create table minutes (
  id text primary key,
  product text not null,
  scope text,
  question text,
  status text check (status in ('valid','invalid')),
  created_at timestamptz,
  file text
);
create index minutes_product_idx on minutes (product, created_at desc);

create table priorities (
  minutes_id text not null,
  idx int not null,
  title text,
  why text,
  sources jsonb,
  owner_role text,
  mark text check (mark in ('accepted','changed','rejected')),
  is_new boolean default false,
  primary key (minutes_id, idx)
);

create table decisions (
  id bigserial primary key,
  product text not null,
  minutes_id text,
  text text,
  why text,
  status text default 'pending' check (status in ('pending','approved','rejected')),
  decided_at timestamptz
);
create index decisions_product_status_idx on decisions (product, status);

create table usage (
  minutes_id text not null,
  role text,
  model text,
  input_tokens int,
  output_tokens int,
  cache_read int,
  cache_write int,
  cost_usd numeric,
  duration_ms int
);
create index usage_minutes_idx on usage (minutes_id);
