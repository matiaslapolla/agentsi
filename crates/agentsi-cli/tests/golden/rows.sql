-- The rows that a write command can change, as one JSON object. The TypeScript
-- side of the row check: `make-golden.sh` runs it on Postgres after each write
-- case. `support.rs::sqlite_rows` gives the same shape from SQLite.
select json_build_object(
  'minutes', (select coalesce(json_agg(json_build_object('id', id, 'data', data) order by id), '[]'::json) from minutes),
  'priorities', (select coalesce(json_agg(json_build_object('minutes_id', minutes_id, 'idx', idx, 'mark', mark, 'is_new', is_new) order by minutes_id, idx), '[]'::json) from priorities),
  'decisions', (select coalesce(json_agg(json_build_object('id', id, 'product', product, 'minutes_id', minutes_id, 'text', text, 'why', why, 'status', status, 'decided_at', decided_at, 'kind', kind, 'reflection_id', reflection_id, 'action', action) order by id), '[]'::json) from decisions),
  'archive_records', (select coalesce(json_agg(json_build_object('product', product, 'kind', kind, 'ref', ref) order by product, kind, ref), '[]'::json) from archive_records)
);
