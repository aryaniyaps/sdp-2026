import { createHash, randomUUID } from "node:crypto";
import { mkdir, open, readFile, readdir, rename, unlink } from "node:fs/promises";
import { userInfo } from "node:os";
import { join } from "node:path";

export type EvidenceEvent = { role: string; content: string; occurred_at: string; metadata: Record<string, unknown> };
export type RetainBatch = { namespace: string; session_id: string; external_id: string; events: EvidenceEvent[]; metadata: Record<string, unknown> };
export type RecallResult = { context: string; trace_id: string; degraded_reasons: string[]; results: unknown[] };
export function digest(value: unknown): string { return createHash("sha256").update(JSON.stringify(value)).digest("hex"); }
/** One memory per person: sessions in any directory share it unless MEMORY_NAMESPACE says otherwise. */
export function defaultNamespace(): string {
  const user = process.env.USER || process.env.LOGNAME || userInfo().username;
  if (!user) throw new Error("cannot determine a user name for the default memory namespace; set MEMORY_NAMESPACE");
  return `user:${user}`;
}
export class MemoryClient {
  constructor(readonly base: string, readonly spool: string, readonly timeout = 3000) {}
  async request<T>(path: string, body?: unknown, timeout = this.timeout, abort?: AbortSignal): Promise<T> {
    const response = await fetch(`${this.base.replace(/\/$/, "")}${path}`, {
      method: body === undefined ? "GET" : "POST",
      headers: { "content-type": "application/json" },
      body: body === undefined ? undefined : JSON.stringify(body), signal: abort ? AbortSignal.any([AbortSignal.timeout(timeout), abort]) : AbortSignal.timeout(timeout),
    });
    if (!response.ok) throw new Error(`memory service HTTP ${response.status}: ${(await response.text()).slice(0, 300)}`);
    return await response.json() as T;
  }
  /** `temporal: false` skips the server's LLM date planner, which can take many seconds. `include_raw` keeps recent source text in play when extraction missed or misread it. */
  async recall(namespace: string, query: string, options: { temporal?: boolean; timeout?: number; signal?: AbortSignal } = {}): Promise<RecallResult> {
    return this.request("/api/v2/recall", { namespace, query, max_tokens: 2048, top_k: 10, reference_date: new Date().toISOString(), temporal: options.temporal ?? true, include_raw: true }, options.timeout, options.signal);
  }
  async enqueue(batch: RetainBatch): Promise<void> {
    // Persist before HTTP: a process crash never loses a previously acknowledged local event.
    await mkdir(this.spool, { recursive: true, mode: 0o700 });
    const path = join(this.spool, `${digest([batch.namespace, batch.external_id])}.json`);
    const tmp = `${path}.${randomUUID()}.tmp`;
    const handle = await open(tmp, "wx", 0o600);
    try { await handle.writeFile(JSON.stringify(batch)); await handle.sync(); }
    finally { await handle.close(); }
    await rename(tmp, path);
    const directory = await open(this.spool, "r");
    try { await directory.sync(); } finally { await directory.close(); }
  }
  async flush(): Promise<{ delivered: number; pending: number; errors: string[] }> {
    await mkdir(this.spool, { recursive: true, mode: 0o700 });
    const files = (await readdir(this.spool)).filter(f => f.endsWith(".json")).sort();
    let delivered = 0; const errors: string[] = [];
    for (const file of files) {
      try {
        const path = join(this.spool, file);
        const batch: RetainBatch = JSON.parse(await readFile(path, "utf8"));
        await this.request("/api/v2/retain", batch);
        await unlink(path); delivered++;
      } catch (error) { errors.push(String(error)); }
    }
    return { delivered, pending: files.length - delivered, errors };
  }
}
export function messageText(message: { content?: unknown }): string {
  if (typeof message.content === "string") return message.content;
  if (!Array.isArray(message.content)) return "";
  return message.content.filter(p => p?.type === "text").map(p => p.text).join("\n");
}
export function observedExitCode(details: unknown, isError: boolean): number | null {
  if (details && typeof details === "object" && "exitCode" in details && typeof details.exitCode === "number") return details.exitCode;
  if (details && typeof details === "object" && "exit_code" in details && typeof details.exit_code === "number") return details.exit_code;
  // A non-error result is not necessarily a completed successful command.
  void isError;
  return null;
}
