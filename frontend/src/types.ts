export interface Health {
  database: boolean;
  embedder: boolean;
  worker_model?: string;
}

export interface Assertion {
  id: string;
  statement: string;
  kind: string;
  status: string;
  confidence: number;
  valid_from: string | null;
  valid_to: string | null;
  recorded_at: string;
}

export interface JobCount {
  kind: string;
  status: string;
  count: number;
}

export interface GraphSnapshot {
  entities: { id: string; name: string }[];
  assertions: Assertion[];
  edges: { from_id: string; to_id: string; relation: string }[];
  status: {
    ready: boolean;
    outstanding_jobs: number;
    stale_observations: number;
    last_projection_completed_at: string | null;
    jobs: JobCount[];
  };
}

export interface Trace {
  id: string;
  operation_type: string;
  status: string;
  duration_ms: number;
  started_at: string;
  request: unknown;
  steps: {
    ordinal: number;
    stage: string;
    status: string;
    duration_ms: number;
    details: unknown;
  }[];
}

export interface RecallResponse {
  context: string;
  estimated_tokens: number;
  elapsed_ms: number;
  trace_id: string;
  degraded_reasons: string[];
  ranking: {
    id: string;
    statement: string;
    status: string;
    ranks: Record<string, number>;
    paths: string[][];
  }[];
}
