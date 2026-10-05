ALTER TABLE chunks ADD COLUMN embedding vector(1024);
ALTER TABLE chunks ADD COLUMN search_vector tsvector GENERATED ALWAYS AS (to_tsvector('english',content)) STORED;
CREATE INDEX chunks_search ON chunks USING gin(search_vector);
CREATE INDEX chunks_embedding ON chunks USING hnsw(embedding vector_cosine_ops);
