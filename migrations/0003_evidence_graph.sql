-- PostgreSQL owns evidence and graph semantics; Neo4j is a replayable projection.
ALTER TABLE raw_events DROP CONSTRAINT raw_events_role_check;
ALTER TABLE raw_events ADD CHECK (role IN ('user','assistant','system','tool'));
ALTER TABLE raw_events ADD COLUMN metadata jsonb NOT NULL DEFAULT '{}';
CREATE TABLE episodes (
 id uuid PRIMARY KEY DEFAULT gen_random_uuid(), namespace text NOT NULL,
 session_id uuid NOT NULL REFERENCES sessions(id), external_id text NOT NULL,
 content_hash text NOT NULL, metadata jsonb NOT NULL DEFAULT '{}', created_at timestamptz NOT NULL DEFAULT now(),
 UNIQUE(namespace, external_id)
);
CREATE TABLE episode_sources (
 episode_id uuid NOT NULL REFERENCES episodes(id) ON DELETE CASCADE,
 chunk_id uuid NOT NULL REFERENCES chunks(id), ordinal integer NOT NULL,
 PRIMARY KEY(episode_id,chunk_id), UNIQUE(episode_id,ordinal)
);
CREATE TABLE entities (
 id uuid PRIMARY KEY DEFAULT gen_random_uuid(), namespace text NOT NULL,
 entity_type text NOT NULL, name text NOT NULL, normalized_name text NOT NULL,
 UNIQUE(namespace,entity_type,normalized_name), UNIQUE(namespace,id)
);
CREATE TABLE entity_aliases (
 namespace text NOT NULL, entity_id uuid NOT NULL, alias text NOT NULL,
 evidence_chunk_id uuid NOT NULL REFERENCES chunks(id),
 FOREIGN KEY(namespace,entity_id) REFERENCES entities(namespace,id),
 PRIMARY KEY(entity_id,alias,evidence_chunk_id)
);
CREATE INDEX entity_alias_lookup ON entity_aliases(namespace,alias);
CREATE TABLE assertions (
 id uuid PRIMARY KEY DEFAULT gen_random_uuid(), namespace text NOT NULL,
 subject_id uuid NOT NULL, predicate text NOT NULL, value text NOT NULL,
 normalized_value text NOT NULL, statement text NOT NULL,
 slot_key text NOT NULL, cardinality text NOT NULL CHECK(cardinality IN ('single','multiple','event')),
 kind text NOT NULL CHECK(kind IN ('fact','preference','profile','goal','episode','observation','procedure','other')),
 status text NOT NULL CHECK(status IN ('active','superseded','contested','stale','retracted')),
 valid_from timestamptz NOT NULL, valid_to timestamptz,
 event_at timestamptz, recorded_at timestamptz NOT NULL DEFAULT now(),
 confidence real NOT NULL CHECK(confidence BETWEEN 0 AND 1),
 model text NOT NULL, episode_id uuid REFERENCES episodes(id),
 embedding vector(1024), search_vector tsvector GENERATED ALWAYS AS (to_tsvector('english',statement)) STORED,
 FOREIGN KEY(namespace,subject_id) REFERENCES entities(namespace,id), UNIQUE(namespace,id),
 CHECK(valid_to IS NULL OR valid_to >= valid_from)
);
CREATE UNIQUE INDEX assertions_single_active ON assertions(namespace,slot_key)
 WHERE cardinality='single' AND status='active';
CREATE INDEX assertions_namespace ON assertions(namespace,status);
CREATE INDEX assertions_slot ON assertions(namespace,slot_key);
CREATE INDEX assertions_search ON assertions USING gin(search_vector);
CREATE INDEX assertions_embedding ON assertions USING hnsw(embedding vector_cosine_ops);
CREATE TABLE assertion_sources (
 assertion_id uuid NOT NULL REFERENCES assertions(id) ON DELETE CASCADE,
 chunk_id uuid NOT NULL REFERENCES chunks(id), quote text NOT NULL,
 PRIMARY KEY(assertion_id,chunk_id)
);
CREATE TABLE assertion_entities (
 namespace text NOT NULL, assertion_id uuid NOT NULL, entity_id uuid NOT NULL,
 FOREIGN KEY(namespace,assertion_id) REFERENCES assertions(namespace,id) ON DELETE CASCADE,
 FOREIGN KEY(namespace,entity_id) REFERENCES entities(namespace,id),
 PRIMARY KEY(assertion_id,entity_id)
);
CREATE TABLE assertion_edges (
 namespace text NOT NULL, from_id uuid NOT NULL, to_id uuid NOT NULL,
 relation text NOT NULL CHECK(relation IN ('supersedes','extends','contradicts','supports','derives','causes')),
 explanation text NOT NULL, created_at timestamptz NOT NULL DEFAULT now(),
 FOREIGN KEY(namespace,from_id) REFERENCES assertions(namespace,id) ON DELETE CASCADE,
 FOREIGN KEY(namespace,to_id) REFERENCES assertions(namespace,id) ON DELETE CASCADE,
 PRIMARY KEY(from_id,to_id,relation), CHECK(from_id<>to_id)
);
CREATE INDEX assertion_edges_reverse ON assertion_edges(namespace,to_id,relation);
CREATE TABLE memory_jobs (
 id uuid PRIMARY KEY DEFAULT gen_random_uuid(), namespace text NOT NULL,
 kind text NOT NULL CHECK(kind IN ('extract','consolidate','project','rebuild','clear_graph')),
 dedupe_key text NOT NULL UNIQUE, payload jsonb NOT NULL,
 status text NOT NULL DEFAULT 'pending' CHECK(status IN ('pending','running','succeeded','failed')),
 attempts integer NOT NULL DEFAULT 0, max_attempts integer NOT NULL DEFAULT 5,
 lease_token uuid, lease_until timestamptz, available_at timestamptz NOT NULL DEFAULT now(),
 result jsonb, error text, created_at timestamptz NOT NULL DEFAULT now(), completed_at timestamptz
);
CREATE INDEX memory_jobs_claim ON memory_jobs(kind,available_at,created_at) WHERE status IN ('pending','running');
CREATE TABLE model_cache (
 cache_key text PRIMARY KEY, model text NOT NULL, prompt_version text NOT NULL,
 response jsonb NOT NULL, created_at timestamptz NOT NULL DEFAULT now()
);
-- Preserve legacy version IDs and source provenance, including closed validity windows.
INSERT INTO entities(namespace,entity_type,name,normalized_name)
 SELECT DISTINCT ON (namespace,lower(trim(subject))) namespace,'person',subject,lower(trim(subject))
 FROM memories ORDER BY namespace,lower(trim(subject)),created_at;
INSERT INTO assertions(id,namespace,subject_id,predicate,value,normalized_value,statement,slot_key,cardinality,kind,status,valid_from,valid_to,recorded_at,confidence,model,embedding)
 SELECT v.id,m.namespace,e.id,m.predicate,v.value,v.normalized_value,v.statement,m.canonical_key,'single',v.kind,v.status,v.valid_from,v.valid_to,v.created_at,1,v.extractor_version,v.embedding
 FROM memory_versions v JOIN memories m ON m.id=v.memory_id JOIN entities e ON e.namespace=m.namespace AND e.entity_type='person' AND e.normalized_name=lower(trim(m.subject));
INSERT INTO assertion_sources SELECT s.memory_version_id,s.chunk_id,c.content FROM memory_version_sources s JOIN chunks c ON c.id=s.chunk_id;
INSERT INTO assertion_entities SELECT namespace,id,subject_id FROM assertions;
INSERT INTO assertion_edges SELECT a.namespace,r.from_version_id,r.to_version_id,r.relation_type,'legacy version migration',r.created_at FROM memory_relations r JOIN assertions a ON a.id=r.from_version_id;
INSERT INTO memory_jobs(namespace,kind,dedupe_key,payload)
 SELECT namespace,'project','backfill:'||id,jsonb_build_object('assertion_id',id) FROM assertions;
