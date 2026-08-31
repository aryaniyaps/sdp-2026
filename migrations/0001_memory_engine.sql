CREATE EXTENSION IF NOT EXISTS vector;
CREATE EXTENSION IF NOT EXISTS pgcrypto;

CREATE TABLE sessions (
  id uuid PRIMARY KEY DEFAULT gen_random_uuid(),
  namespace text NOT NULL CHECK (length(trim(namespace)) > 0),
  external_id text NOT NULL CHECK (length(trim(external_id)) > 0),
  created_at timestamptz NOT NULL DEFAULT now(),
  UNIQUE(namespace, external_id)
);

CREATE TABLE raw_events (
  id uuid PRIMARY KEY DEFAULT gen_random_uuid(),
  session_id uuid NOT NULL REFERENCES sessions(id),
  role text NOT NULL CHECK (role IN ('user','assistant','system')),
  content text NOT NULL CHECK (length(trim(content)) > 0),
  occurred_at timestamptz NOT NULL,
  processing_state text NOT NULL DEFAULT 'pending' CHECK (processing_state IN ('pending','processed','failed')),
  extraction_error text,
  created_at timestamptz NOT NULL DEFAULT now()
);

CREATE TABLE chunks (
  id uuid PRIMARY KEY DEFAULT gen_random_uuid(),
  raw_event_id uuid NOT NULL REFERENCES raw_events(id),
  ordinal integer NOT NULL CHECK (ordinal >= 0),
  content text NOT NULL,
  created_at timestamptz NOT NULL DEFAULT now(),
  UNIQUE(raw_event_id, ordinal)
);

CREATE TABLE memories (
  id uuid PRIMARY KEY DEFAULT gen_random_uuid(),
  namespace text NOT NULL,
  canonical_key text NOT NULL,
  subject text NOT NULL,
  predicate text NOT NULL,
  created_at timestamptz NOT NULL DEFAULT now(),
  UNIQUE(namespace, canonical_key)
);

CREATE TABLE memory_versions (
  id uuid PRIMARY KEY DEFAULT gen_random_uuid(),
  memory_id uuid NOT NULL REFERENCES memories(id),
  version integer NOT NULL CHECK (version > 0),
  value text NOT NULL,
  normalized_value text NOT NULL,
  statement text NOT NULL,
  kind text NOT NULL,
  status text NOT NULL CHECK (status IN ('active','superseded')),
  valid_from timestamptz NOT NULL,
  valid_to timestamptz,
  extractor_version text NOT NULL,
  embedding vector(1024),
  search_vector tsvector GENERATED ALWAYS AS (to_tsvector('english', coalesce(statement, '') || ' ' || coalesce(value, ''))) STORED,
  created_at timestamptz NOT NULL DEFAULT now(),
  UNIQUE(memory_id, version),
  CHECK ((status = 'active' AND valid_to IS NULL) OR (status = 'superseded' AND valid_to IS NOT NULL))
);
CREATE UNIQUE INDEX one_active_version_per_memory ON memory_versions(memory_id) WHERE status = 'active';
CREATE INDEX memory_versions_search_idx ON memory_versions USING gin(search_vector);
CREATE INDEX memory_versions_embedding_idx ON memory_versions USING hnsw (embedding vector_cosine_ops);

CREATE TABLE memory_version_sources (
  memory_version_id uuid NOT NULL REFERENCES memory_versions(id),
  chunk_id uuid NOT NULL REFERENCES chunks(id),
  PRIMARY KEY(memory_version_id, chunk_id)
);

CREATE TABLE memory_relations (
  id uuid PRIMARY KEY DEFAULT gen_random_uuid(),
  from_version_id uuid NOT NULL REFERENCES memory_versions(id),
  to_version_id uuid NOT NULL REFERENCES memory_versions(id),
  relation_type text NOT NULL CHECK (relation_type IN ('supersedes','extends','derives')),
  created_at timestamptz NOT NULL DEFAULT now(),
  UNIQUE(from_version_id, to_version_id, relation_type),
  CHECK (from_version_id <> to_version_id)
);

