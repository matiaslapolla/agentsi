-- The rows that a reflection can change, as one JSON object: the shape of
-- `reflect_sqlite_rows` in support.rs. The TypeScript side of the row check.
select json_build_object(
  'counts', json_build_object(
    'minutes', (select count(*) from minutes),
    'priorities', (select count(*) from priorities),
    'usage', (select count(*) from usage),
    'meeting_log', (select count(*) from meeting_log),
    'snapshots', (select count(*) from snapshots)),
  'memory', coalesce((select json_agg(t order by t.product, t.agent, t.position) from (
    select product, agent, id, position, text, kind, helpful, harmful, source, created_at, retired_at, retired_why
    from memory) t), '[]'::json),
  'reflections', coalesce((select json_agg(t order by t.product, t.position) from (
    select product, id, position, entry from reflections) t), '[]'::json),
  'decisions', coalesce((select json_agg(t order by t.id) from (
    select id, product, minutes_id, text, why, status, decided_at, kind, reflection_id, action
    from decisions) t), '[]'::json),
  'overlays', coalesce((select json_agg(t order by t.product, t.agent) from (
    select product, agent, body from agent_overlays) t), '[]'::json)
);
