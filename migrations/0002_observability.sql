CREATE TABLE operations (
  id uuid PRIMARY KEY,
  operation_type text NOT NULL CHECK (operation_type IN ('ingest','search')),
  namespace text NOT NULL,
  session_id uuid REFERENCES sessions(id),
  event_id uuid REFERENCES raw_events(id),
  status text NOT NULL CHECK (status IN ('succeeded','failed','degraded')),
  started_at timestamptz NOT NULL,
  finished_at timestamptz NOT NULL,
  duration_ms bigint NOT NULL CHECK (duration_ms >= 0),
  degraded_mode boolean NOT NULL DEFAULT false,
  request jsonb NOT NULL,
  result jsonb,
  error text,
  created_at timestamptz NOT NULL DEFAULT now()
);

CREATE TABLE operation_steps (
  id bigserial PRIMARY KEY,
  operation_id uuid NOT NULL REFERENCES operations(id) ON DELETE CASCADE,
  ordinal integer NOT NULL CHECK (ordinal >= 0),
  stage text NOT NULL,
  status text NOT NULL CHECK (status IN ('ok','error','degraded','skipped')),
  duration_ms bigint NOT NULL CHECK (duration_ms >= 0),
  details jsonb NOT NULL DEFAULT '{}'::jsonb,
  UNIQUE(operation_id, ordinal)
);

CREATE INDEX operations_namespace_started_idx ON operations(namespace, started_at DESC);
CREATE INDEX operation_steps_operation_idx ON operation_steps(operation_id, ordinal);

