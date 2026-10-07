import { randomUUID } from "node:crypto";
import {
  mkdir,
  open,
  readFile,
  readdir,
  rename,
  unlink,
} from "node:fs/promises";
import { join } from "node:path";
import { digest } from "./identity.ts";
import type { RetainBatch } from "./types.ts";

// Evidence reaches disk before HTTP. Retries reuse the same idempotency key.
export class EvidenceSpool {
  constructor(readonly directory: string) {}
  async enqueue(batch: RetainBatch): Promise<void> {
    // Persist before HTTP: a process crash never loses a previously acknowledged local event.
    await mkdir(this.directory, { recursive: true, mode: 0o700 });
    const path = join(
      this.directory,
      `${digest([batch.namespace, batch.external_id])}.json`,
    );
    const tmp = `${path}.${randomUUID()}.tmp`;
    const handle = await open(tmp, "wx", 0o600);
    try {
      await handle.writeFile(JSON.stringify(batch));
      await handle.sync();
    } finally {
      await handle.close();
    }
    await rename(tmp, path);
    const directory = await open(this.directory, "r");
    try {
      await directory.sync();
    } finally {
      await directory.close();
    }
  }
  /** Remove queued batches for one namespace after the server has cleared it. */
  async discardNamespace(namespace: string): Promise<number> {
    await mkdir(this.directory, { recursive: true, mode: 0o700 });
    const files = (await readdir(this.directory)).filter((f) => f.endsWith(".json"));
    let discarded = 0;
    for (const file of files) {
      const path = join(this.directory, file);
      try {
        const batch: RetainBatch = JSON.parse(await readFile(path, "utf8"));
        if (batch.namespace !== namespace) continue;
        await unlink(path);
        discarded++;
      } catch {
        // Keep unreadable spool files for inspection and manual recovery.
      }
    }
    return discarded;
  }
  async flush(deliver: (batch: RetainBatch) => Promise<unknown>): Promise<{
    delivered: number;
    pending: number;
    errors: string[];
  }> {
    await mkdir(this.directory, { recursive: true, mode: 0o700 });
    const files = (await readdir(this.directory))
      .filter((f) => f.endsWith(".json"))
      .sort();
    let delivered = 0;
    const errors: string[] = [];
    for (const file of files) {
      try {
        const path = join(this.directory, file);
        const batch: RetainBatch = JSON.parse(await readFile(path, "utf8"));
        await deliver(batch);
        await unlink(path);
        delivered++;
      } catch (error) {
        errors.push(String(error));
      }
    }
    return { delivered, pending: files.length - delivered, errors };
  }
}
