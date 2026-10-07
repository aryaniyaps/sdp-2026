import { createHash } from "node:crypto";
import { userInfo } from "node:os";

export function digest(value: unknown): string {
  return createHash("sha256").update(JSON.stringify(value)).digest("hex");
}
/** One memory per person: sessions in any directory share it unless MEMORY_NAMESPACE says otherwise. */
export function defaultNamespace(): string {
  const user = process.env.USER || process.env.LOGNAME || userInfo().username;
  if (!user)
    throw new Error(
      "cannot determine a user name for the default memory namespace; set MEMORY_NAMESPACE",
    );
  return `user:${user}`;
}
