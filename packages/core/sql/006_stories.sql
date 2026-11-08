create table stories (
  product text not null,
  id text not null,
  data jsonb not null,
  primary key (product, id)
);

create table story_logs (
  product text not null,
  story_id text not null,
  body text not null default '',
  primary key (product, story_id)
);
