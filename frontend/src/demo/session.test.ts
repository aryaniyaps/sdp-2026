import { describe, expect, it } from "vitest";
import { directoryFrom, graphUrl, terminalUrl } from "./session";

describe("demo session routing", () => {
  it("forwards directory and namespace as ordered ttyd arguments", () => {
    const namespace = "project:a & b/π";
    const terminal = new URL(
      terminalUrl("mobile-app", namespace, 2),
      "http://localhost",
    );
    expect(terminal.searchParams.getAll("arg")).toEqual([
      "mobile-app",
      namespace,
    ]);
    expect(terminal.searchParams.get("session")).toBe("2");
    expect(
      new URL(graphUrl(namespace), "http://localhost").searchParams.get(
        "namespace",
      ),
    ).toBe(namespace);
  });

  it("restricts the directory to the supported projects", () => {
    expect(directoryFrom("scratch")).toBe("scratch");
    expect(directoryFrom("../../etc")).toBe("payments-api");
    expect(directoryFrom(null)).toBe("payments-api");
  });
});
