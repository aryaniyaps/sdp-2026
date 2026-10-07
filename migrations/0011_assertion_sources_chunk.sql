-- The graph view asks, for a chunk, which assertions cite it and which episode holds it. Both
-- primary keys start with the other column, so without these indexes each lookup scans the table.
CREATE INDEX assertion_sources_chunk ON assertion_sources(chunk_id);
CREATE INDEX episode_sources_chunk ON episode_sources(chunk_id);
