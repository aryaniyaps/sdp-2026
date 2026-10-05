-- An observation may cite several distinct passages from the same source chunk.
ALTER TABLE assertion_sources DROP CONSTRAINT assertion_sources_pkey;
ALTER TABLE assertion_sources ADD PRIMARY KEY(assertion_id,chunk_id,quote);
WITH RECURSIVE support_paths(observation_id,support_id) AS (
 SELECT from_id,to_id FROM assertion_edges WHERE relation IN ('supports','derives')
 UNION
 SELECT p.observation_id,e.to_id FROM support_paths p JOIN assertion_edges e ON e.from_id=p.support_id WHERE e.relation IN ('supports','derives')
)
INSERT INTO assertion_sources SELECT p.observation_id,s.chunk_id,s.quote FROM support_paths p JOIN assertion_sources s ON s.assertion_id=p.support_id ON CONFLICT DO NOTHING;
