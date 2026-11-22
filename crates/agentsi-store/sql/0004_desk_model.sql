-- The model and the effort of the desk session of each product. Null: the role default.
-- Spec: docs/specs/agentsi-desktop-desk-model.md.

alter table desk_sessions add column desk_model text;
alter table desk_sessions add column desk_effort text;
