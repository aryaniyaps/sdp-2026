// Mirrors the starter catalog seeded by migration 0012; replaced by the live catalog after loading.
export const sampleProjects = ["payments-api", "mobile-app", "scratch"];

export type Directory = string;
export function directoryFrom(value: string | null): Directory {
  return value && /^[a-zA-Z0-9][a-zA-Z0-9_-]{0,63}$/.test(value)
    ? value
    : "payments-api";
}
export function terminalUrl(
  directory: Directory,
  namespace: string,
  session: string | number,
) {
  const query = new URLSearchParams();
  query.append("arg", directory);
  query.append("arg", namespace);
  query.append("arg", String(session));
  return `/term/?${query}`;
}
export function graphUrl(namespace: string) {
  return `/graph?${new URLSearchParams({ namespace, readonly: "1" })}`;
}
export type Session = {
  id: string;
  project: string;
  namespace: string;
  revision: number;
  applied_revision: number;
};
export async function dashboardRequest<T>(
  path: string,
  body?: unknown,
): Promise<T> {
  const response = await fetch(`/api/v2/${path}`, {
    method: body === undefined ? "GET" : "POST",
    headers: { "content-type": "application/json" },
    body: body === undefined ? undefined : JSON.stringify(body),
    cache: "no-store",
    signal: AbortSignal.timeout(10_000),
  });
  if (!response.ok)
    throw new Error(
      `Dashboard request failed (${response.status}): ${await response.text()}`,
    );
  return response.json() as Promise<T>;
}
