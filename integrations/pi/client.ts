import { createHash, randomUUID } from "node:crypto";
import { mkdir, open, readFile, readdir, rename, unlink } from "node:fs/promises";
import { userInfo } from "node:os";
import { join } from "node:path";

export type EvidenceEvent = { role: string; content: string; occurred_at: string; metadata: Record<string, unknown> };
export type RetainBatch = { namespace: string; session_id: string; external_id: string; events: EvidenceEvent[]; metadata: Record<string, unknown> };
export type RecallResult = { context: string; trace_id: string; degraded_reasons: string[]; results: unknown[]; max_distance?: number };
export function digest(value: unknown): string { return createHash("sha256").update(JSON.stringify(value)).digest("hex"); }
/** One memory per person: sessions in any directory share it unless MEMORY_NAMESPACE says otherwise. */
export function defaultNamespace(): string {
  const user = process.env.USER || process.env.LOGNAME || userInfo().username;
  if (!user) throw new Error("cannot determine a user name for the default memory namespace; set MEMORY_NAMESPACE");
  return `user:${user}`;
}
/**
 * Cosine distance beyond which the automatic recall before each prompt ignores a vector match, so a
 * prompt that has nothing to do with the stored memory gets little or nothing instead of the nearest
 * unrelated text. Measured with qwen3-embedding:0.6b on one namespace (user:dev, about 320 facts):
 * the relevant facts were at 0.236, 0.300, 0.333, 0.386, 0.388 and 0.439, and the best unrelated
 * hit for a full sentence at 0.445 to 0.564 (paraphrased questions 0.445 to 0.456, small talk 0.509,
 * a statement with nothing relevant stored 0.539). The margin is thin at the top: a coding task that
 * names a file had relevant hits at 0.368 to 0.462, and a wider sample found a relevant fact at 0.465.
 * It does not separate one or two word prompts. "ok" is 0.415 from the nearest fact and 0.340 from
 * the nearest source text, "continue" 0.404 and 0.385, while a short prompt that has an answer is as
 * near ("staging" 0.307, "Meera" 0.448), so those prompts still inject some memory. Another embedding
 * model or another namespace needs its own calibration.
 */
export const AUTOMATIC_RECALL_MAX_DISTANCE = 0.45;
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
  /** `temporal: false` skips the server's LLM date planner, which can take many seconds. `include_raw` keeps recent source text in play when extraction missed or misread it. `maxDistance` is a cosine distance beyond which the server drops vector matches; leave it out for a deliberate question, where the nearest memory is wanted however far it is. */
  async recall(namespace: string, query: string, options: { temporal?: boolean; maxDistance?: number; timeout?: number; signal?: AbortSignal } = {}): Promise<RecallResult> {
    const { maxDistance } = options;
    // JSON turns NaN into null, which the server reads as "not set": refuse here instead of silently recalling everything.
    if (maxDistance !== undefined && !(maxDistance > 0 && maxDistance <= 2)) throw new Error(`maxDistance must be greater than 0 and at most 2, got ${maxDistance}`);
    const result = await this.request<RecallResult>("/api/v2/recall", { namespace, query, max_tokens: 2048, top_k: 10, reference_date: new Date().toISOString(), temporal: options.temporal ?? true, include_raw: true, ...(maxDistance === undefined ? {} : { max_distance: maxDistance }) }, options.timeout, options.signal);
    // The service echoes the cutoff it applied. A service that predates the field ignores it without a word, so say so.
    if (maxDistance !== undefined && result.max_distance !== maxDistance) result.degraded_reasons = [...result.degraded_reasons, `the memory service did not apply max_distance ${maxDistance}, so this recall is not limited by relevance; restart the service so it runs the current build`];
    return result;
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
