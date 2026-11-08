-- The rows that a meeting writes, as one JSON object: the shape of `meeting_sqlite_rows` in support.rs.
-- The new minutes have an id that starts with the year. The fixture ids do not.
select json_build_object(
  'counts', json_build_object(
    'minutes', (select count(*) from minutes),
    'priorities', (select count(*) from priorities),
    'decisions', (select count(*) from decisions),
    'usage', (select count(*) from usage),
    'memory', (select count(*) from memory),
    'meeting_log', (select count(*) from meeting_log),
    'snapshots', (select count(*) from snapshots),
    'documents', (select count(*) from documents)),
  'minutes', coalesce((select json_agg(t order by t.id) from (
    select id, product, scope, question, status,
      to_char(created_at at time zone 'utc', 'YYYY-MM-DD"T"HH24:MI:SS.MS"Z"') as created_at, data, usage
    from minutes where id like '20%') t), '[]'::json),
  'priorities', coalesce((select json_agg(t order by t.minutes_id, t.idx) from (
    select minutes_id, idx, title, why, sources, owner_role, objective, mark, is_new
    from priorities where minutes_id like '20%') t), '[]'::json),
  'decisions', coalesce((select json_agg(t order by t.minutes_id, t.text) from (
    select product, minutes_id, text, why, action, status, decided_at, kind, reflection_id
    from decisions where minutes_id like '20%') t), '[]'::json),
  'usage', coalesce((select json_agg(t order by t.minutes_id, t.role) from (
    select minutes_id, role, model, input_tokens, output_tokens, cache_read, cache_write, cost_usd, duration_ms
    from usage where minutes_id like '20%') t), '[]'::json),
  'memory', coalesce((select json_agg(t order by t.product, t.agent, t.position) from (
    select product, agent, id, position, text, kind, helpful, harmful, source, created_at, retired_at, retired_why
    from memory where source like '20%') t), '[]'::json),
  'meeting_log', coalesce((select json_agg(t order by t.minutes_id) from (
    select product, minutes_id, entry from meeting_log where minutes_id like '20%') t), '[]'::json),
  'snapshots', coalesce((select json_agg(t order by t.minutes_id) from (
    select product, minutes_id, body from snapshots where minutes_id like '20%') t), '[]'::json),
  'documents', coalesce((select json_agg(t order by t.kind, t.ref) from (
    select kind, ref, title, body from documents
    where (kind = 'minutes' and ref like '20%') or kind = 'memory') t), '[]'::json)
);
