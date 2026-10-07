import type { GraphSnapshot, Health, RecallResponse, Trace } from "./types";

export async function request<T>(
  path: string,
  body?: unknown,
  signal?: AbortSignal,
): Promise<T> {
  const response = await fetch(path, {
    method: body === undefined ? "GET" : "POST",
    headers: { Accept: "application/json", "Content-Type": "application/json" },
    body: body === undefined ? undefined : JSON.stringify(body),
    signal: signal
      ? AbortSignal.any([signal, AbortSignal.timeout(30_000)])
      : AbortSignal.timeout(30_000),
  });
  if (!response.ok) {
    const text = await response.text();
    let message = text;
    try {
      message = JSON.parse(text).error ?? text;
    } catch {
      /* Some proxy errors are plain text. */
    }
    throw new Error(message || `Request failed (${response.status}).`);
  }
  return response.json() as Promise<T>;
}

const namespaceQuery = (namespace: string) =>
  new URLSearchParams({ namespace });

export const api = {
  health: (signal?: AbortSignal) =>
    request<Health>("/healthz", undefined, signal),
  graph: (namespace: string, signal?: AbortSignal) =>
    request<GraphSnapshot>(
      `/api/v2/graph?${namespaceQuery(namespace)}`,
      undefined,
      signal,
    ),
  traces: (namespace: string, signal?: AbortSignal) =>
    request<Trace[]>(
      `/api/v2/traces?${namespaceQuery(namespace)}&limit=30`,
      undefined,
      signal,
    ),
  assertion: (namespace: string, id: string, signal?: AbortSignal) =>
    request<unknown>(
      `/api/v2/assertions/${encodeURIComponent(id)}?${namespaceQuery(namespace)}`,
      undefined,
      signal,
    ),
  retain: (namespace: string, session: string, text: string) =>
    request<{ job_id: string }>("/api/v2/retain", {
      namespace,
      session_id: session,
      external_id: crypto.randomUUID(),
      metadata: { source: "dashboard" },
      events: [
        {
          role: "user",
          content: text,
          occurred_at: new Date().toISOString(),
          metadata: {},
        },
      ],
    }),
  recall: (
    namespace: string,
    query: string,
    maxTokens: number,
    graph: boolean,
    observations: boolean,
  ) =>
    request<RecallResponse>("/api/v2/recall", {
      namespace,
      query,
      max_tokens: maxTokens,
      graph,
      observations,
      reference_date: new Date().toISOString(),
    }),
};
