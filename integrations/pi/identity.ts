import { createHash } from "node:crypto";
import { basename } from "node:path";

export function digest(value: unknown): string {
  return createHash("sha256").update(JSON.stringify(value)).digest("hex");
}
/** Repository-root identity keeps subdirectories together and unrelated checkouts apart. */
export function defaultNamespace(root = process.cwd()): string {
  return `project:${basename(root)}:${digest(root).slice(0, 12)}`;
}
