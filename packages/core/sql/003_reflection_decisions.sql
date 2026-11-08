alter table decisions add column kind text not null default 'minutes' check (kind in ('minutes', 'reflection'));
alter table decisions add column reflection_id text;
alter table decisions drop constraint decisions_status_check;
alter table decisions add constraint decisions_status_check check (status in ('pending', 'approved', 'rejected', 'stale'));
