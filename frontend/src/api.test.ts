import { afterEach, describe, expect, it, vi } from "vitest";
import { api, request } from "./api";

afterEach(() => vi.unstubAllGlobals());

describe("API client", () => {
  it("reports JSON and plain text server errors", async () => {
    const fetch = vi
      .fn()
      .mockResolvedValueOnce(
        new Response(JSON.stringify({ error: "Namespace is required" }), {
          status: 400,
        }),
      )
      .mockResolvedValueOnce(
        new Response("Proxy unavailable", { status: 502 }),
      );
    vi.stubGlobal("fetch", fetch);
    await expect(request("/api/v2/recall")).rejects.toThrow(
      "Namespace is required",
    );
    await expect(request("/api/v2/recall")).rejects.toThrow(
      "Proxy unavailable",
    );
  });

  it("encodes namespace characters without changing the query", async () => {
    const fetch = vi.fn().mockResolvedValue(new Response(JSON.stringify([])));
    vi.stubGlobal("fetch", fetch);
    await api.traces("user:student&other=value");
    const url = new URL(fetch.mock.calls[0][0], "http://localhost");
    expect(url.searchParams.get("namespace")).toBe("user:student&other=value");
    expect(url.searchParams.has("other")).toBe(false);
  });
});
