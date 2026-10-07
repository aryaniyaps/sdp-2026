export const directories = ["payments-api", "mobile-app", "scratch"] as const;
export type Directory = (typeof directories)[number];

export function directoryFrom(value: string | null): Directory {
  return directories.find((directory) => directory === value) ?? "payments-api";
}

export function terminalUrl(
  directory: Directory,
  namespace: string,
  session: number,
) {
  const query = new URLSearchParams();
  // ttyd forwards each repeated arg to pi-session.sh in order.
  query.append("arg", directory);
  query.append("arg", namespace);
  query.set("session", String(session));
  return `/term/?${query}`;
}

export function graphUrl(namespace: string) {
  return `/graph?${new URLSearchParams({ namespace, readonly: "1" })}`;
}
