create table memory (
  product text not null,
  agent text not null,
  id text not null,
  position int not null,
  text text not null,
  kind text not null,
  helpful int not null default 0,
  harmful int not null default 0,
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
  data json not null
);
