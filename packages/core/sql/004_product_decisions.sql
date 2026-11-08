alter table decisions drop constraint decisions_kind_check;
alter table decisions add constraint decisions_kind_check check (kind in ('minutes', 'reflection', 'product'));
