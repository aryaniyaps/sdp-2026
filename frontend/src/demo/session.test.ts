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
      "2",
    ]);

    expect(
      new URL(graphUrl(namespace), "http://localhost").searchParams.get(
        "namespace",
      ),
    ).toBe(namespace);
  });

  it("accepts new project names and rejects path traversal", () => {
    expect(directoryFrom("new-project_2")).toBe("new-project_2");
    expect(directoryFrom("scratch")).toBe("scratch");
    expect(directoryFrom("../../etc")).toBe("payments-api");
    expect(directoryFrom(null)).toBe("payments-api");
  });
});
