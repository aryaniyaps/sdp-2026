export function messageText(message: { content?: unknown }): string {
  if (typeof message.content === "string") return message.content;
  if (!Array.isArray(message.content)) return "";
  return message.content
    .filter((p) => p?.type === "text")
    .map((p) => p.text)
    .join("\n");
}
export function observedExitCode(
  details: unknown,
  isError: boolean,
): number | null {
  if (
    details &&
    typeof details === "object" &&
    "exitCode" in details &&
    typeof details.exitCode === "number"
  )
    return details.exitCode;
  if (
    details &&
    typeof details === "object" &&
    "exit_code" in details &&
    typeof details.exit_code === "number"
  )
    return details.exit_code;
  // A non-error result is not necessarily a completed successful command.
  void isError;
  return null;
}

// Once recalled evidence enters a branch it can influence later assistant replies too.
// Keep those replies in the ledger, but do not treat them as independent fact sources.
export function memoryDerivedAssistantIds(branch: readonly {
  id: string;
  type: string;
  customType?: string;
  message?: { role: string; customType?: string; toolName?: string };
}[]): Set<string> {
  let recalled = false;
  const ids = new Set<string>();
  for (const entry of branch) {
    const message = entry.message;
    if ((entry.type === "custom_message" && entry.customType === "sdp-memory-context") ||
        (message?.role === "custom" && message.customType === "sdp-memory-context") ||
        (message?.role === "toolResult" && message.toolName === "memory_recall")) {
      recalled = true;
    }
    if (recalled && message?.role === "assistant") ids.add(entry.id);
  }
  return ids;
}
