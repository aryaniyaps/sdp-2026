export type EvidenceEvent = {
  role: string;
  content: string;
  occurred_at: string;
  metadata: Record<string, unknown>;
};
export type RetainBatch = {
  namespace: string;
  session_id: string;
  external_id: string;
  events: EvidenceEvent[];
  metadata: Record<string, unknown>;
};
export type RecallResult = {
  context: string;
  trace_id: string;
  degraded_reasons: string[];
  results: unknown[];
  max_distance?: number;
};
