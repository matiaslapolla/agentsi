-- Reflection decisions whose reflection has no proposal.
insert into decisions (id, product, minutes_id, text, why, status, decided_at, kind, reflection_id, action) values (30, 'alpha', null, 'Change the prompt of dev', 'Its reflection is gone.', 'pending', null, 'reflection', 'r-gone', null);
insert into decisions (id, product, minutes_id, text, why, status, decided_at, kind, reflection_id, action) values (31, 'alpha', null, 'Change the prompt of qa', 'Its reflection made no proposal.', 'pending', null, 'reflection', 'r-seed', null);
