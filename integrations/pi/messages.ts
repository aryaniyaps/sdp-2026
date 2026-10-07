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
