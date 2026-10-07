import { EvidenceSpool } from "./spool.ts";
import type { RecallResult, RetainBatch } from "./types.ts";

export type { EvidenceEvent, RetainBatch, RecallResult } from "./types.ts";
export { defaultNamespace, digest } from "./identity.ts";
export { messageText, observedExitCode } from "./messages.ts";

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
  private readonly queue: EvidenceSpool;

  constructor(
    readonly base: string,
    readonly spool: string,
    readonly timeout = 3000,
  ) {
    this.queue = new EvidenceSpool(spool);
  }
  async request<T>(
    path: string,
    body?: unknown,
    timeout = this.timeout,
    abort?: AbortSignal,
  ): Promise<T> {
    const response = await fetch(`${this.base.replace(/\/$/, "")}${path}`, {
      method: body === undefined ? "GET" : "POST",
      headers: { "content-type": "application/json" },
      body: body === undefined ? undefined : JSON.stringify(body),
      signal: abort
        ? AbortSignal.any([AbortSignal.timeout(timeout), abort])
        : AbortSignal.timeout(timeout),
    });
    if (!response.ok)
      throw new Error(
        `memory service HTTP ${response.status}: ${(await response.text()).slice(0, 300)}`,
      );
    return (await response.json()) as T;
  }
  /** `temporal: false` skips the server's LLM date planner, which can take many seconds. `include_raw` keeps recent source text in play when extraction missed or misread it. `maxDistance` is a cosine distance beyond which the server drops vector matches; leave it out for a deliberate question, where the nearest memory is wanted however far it is. */
  async recall(
    namespace: string,
    query: string,
    options: {
      temporal?: boolean;
      maxDistance?: number;
      timeout?: number;
      signal?: AbortSignal;
    } = {},
  ): Promise<RecallResult> {
    const { maxDistance } = options;
    // JSON turns NaN into null, which the server reads as "not set": refuse here instead of silently recalling everything.
    if (maxDistance !== undefined && !(maxDistance > 0 && maxDistance <= 2))
      throw new Error(
        `maxDistance must be greater than 0 and at most 2, got ${maxDistance}`,
      );
    const result = await this.request<RecallResult>(
      "/api/v2/recall",
      {
        namespace,
        query,
        max_tokens: 2048,
        top_k: 10,
        reference_date: new Date().toISOString(),
        temporal: options.temporal ?? true,
        include_raw: true,
        ...(maxDistance === undefined ? {} : { max_distance: maxDistance }),
      },
      options.timeout,
      options.signal,
    );
    // The service echoes the cutoff it applied. A service that predates the field ignores it without a word, so say so.
    if (maxDistance !== undefined && result.max_distance !== maxDistance)
      result.degraded_reasons = [
        ...result.degraded_reasons,
        `the memory service did not apply max_distance ${maxDistance}, so this recall is not limited by relevance; restart the service so it runs the current build`,
      ];
    return result;
  }
  async enqueue(batch: RetainBatch): Promise<void> {
    return this.queue.enqueue(batch);
  }

  async flush() {
    return this.queue.flush((batch) => this.request("/api/v2/retain", batch));
  }

  async discardNamespace(namespace: string) {
    return this.queue.discardNamespace(namespace);
  }
}
