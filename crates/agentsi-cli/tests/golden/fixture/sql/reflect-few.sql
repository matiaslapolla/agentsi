-- The log keeps its failures, but two minutes lose their body, so replay has too few cases.
update minutes set data = null where id in ('rf-03', 'rf-05');
