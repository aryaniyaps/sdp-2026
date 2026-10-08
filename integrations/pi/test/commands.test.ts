import { test } from "node:test";
import assert from "node:assert/strict";
import { mkdtemp, rm } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join } from "node:path";
import memoryExtension from "../extension.ts";
import type { ExtensionAPI } from "@earendil-works/pi-coding-agent";

test("namespace changes and clears never replay old conversation into memory", async () => {
  const root = await mkdtemp(join(tmpdir(), "pi-commands-"));
  const saved = {
    fetch: globalThis.fetch,
    namespace: process.env.MEMORY_NAMESPACE,
    spool: process.env.MEMORY_SPOOL,
  };
  process.env.MEMORY_NAMESPACE = "project:a";
  process.env.MEMORY_SPOOL = root;
  const requests: { url: string; body: any }[] = [];
  globalThis.fetch = (async (url: string, options: RequestInit) => {
    requests.push({
      url,
      body: options?.body ? JSON.parse(String(options.body)) : undefined,
    });
    return new Response(
      JSON.stringify({ graph: { cleared: true }, retained: true }),
    );
  }) as typeof fetch;
  const handlers = new Map<string, any>(),
    commands = new Map<string, any>();
  const branch: any[] = [];
  let typed = "";
  const ctx: any = {
    cwd: root,
    hasUI: true,
    ui: { input: async () => typed, notify: () => {}, setStatus: () => {} },
    sessionManager: {
      getBranch: () => branch,
      getSessionId: () => "commands",
      getLeafId: () => branch.at(-1)?.id,
    },
  };
  const pi = {
    on: (name: string, fn: any) => handlers.set(name, fn),
    registerCommand: (name: string, command: any) =>
      commands.set(name, command.handler),
    registerTool: () => {},
    appendEntry: (customType: string, data: unknown) =>
      branch.push({ type: "custom", customType, data }),
  } as unknown as ExtensionAPI;
  const message = (id: string, content: string) =>
    branch.push({
      id,
      type: "message",
      timestamp: new Date().toISOString(),
      message: { role: "user", content },
    });
  try {
    memoryExtension(pi);
    await handlers.get("session_start")({}, ctx);
    message("old", "private to A");
    await handlers.get("agent_settled")({}, ctx);
    await commands.get("memory-namespace")("project:b", ctx);
    message("new", "private to B");
    await handlers.get("agent_settled")({}, ctx);
    const retained = requests.filter((r) => r.url.endsWith("/retain"));
    assert.equal(retained.length, 2);
    assert.equal(retained[1].body.namespace, "project:b");
    assert.deepEqual(
      retained[1].body.events.map((e: any) => e.content),
      ["private to B"],
    );
    typed = "wrong";
    await commands.get("memory-clear")("", ctx);
    assert(!requests.some((r) => r.url.endsWith("/clear")));
    // Even an as-yet unretained message must not resurrect after clear.
    message("unretained", "erase this too");
    typed = "project:b";
    await commands.get("memory-clear")("", ctx);
    await handlers.get("agent_settled")({}, ctx);
    assert.equal(requests.filter((r) => r.url.endsWith("/retain")).length, 2);
    assert.deepEqual(requests.find((r) => r.url.endsWith("/clear"))?.body, {
      namespace: "project:b",
      confirm: "project:b",
    });
    message("fresh", "after clear");
    await handlers.get("agent_settled")({}, ctx);
    assert.deepEqual(
      requests
        .filter((r) => r.url.endsWith("/retain"))
        .at(-1)!
        .body.events.map((e: any) => e.content),
      ["after clear"],
    );
  } finally {
    globalThis.fetch = saved.fetch;
    for (const [key, value] of [
      ["MEMORY_NAMESPACE", saved.namespace],
      ["MEMORY_SPOOL", saved.spool],
    ]) {
      if (value === undefined) delete process.env[key!];
      else process.env[key!] = value;
    }
    await rm(root, { recursive: true, force: true });
  }
});

test("dashboard namespace changes immediately during an active turn without moving its evidence", async () => {
  const root = await mkdtemp(join(tmpdir(), "pi-dashboard-"));
  const keys = ["MEMORY_NAMESPACE", "MEMORY_SPOOL", "MEMORY_DASHBOARD_SESSION"];
  const saved = keys.map((key) => process.env[key]);
  const oldFetch = globalThis.fetch;
  process.env.MEMORY_NAMESPACE = "project:a";
  process.env.MEMORY_SPOOL = root;
  process.env.MEMORY_DASHBOARD_SESSION = "test-binding";
  let state = { namespace: "project:a", revision: 0 };
  let applied = -1;
  let finishRecall: (() => void) | undefined;
  const retained: any[] = [];
  globalThis.fetch = (async (url: string, options: RequestInit) => {
    const body = options?.body ? JSON.parse(String(options.body)) : undefined;
    let result: unknown = {};
    if (url.endsWith("/test-binding")) result = state;
    else if (url.endsWith("/ack")) {
      applied = body.revision;
      result = { applied: true };
    } else if (url.endsWith("/recall")) {
      if (body.namespace === "project:a")
        await new Promise<void>((resolve) => {
          finishRecall = resolve;
        });
      result = {
        context: body.namespace === "project:a" ? "old namespace facts" : "",
        degraded_reasons: [],
        max_distance: body.max_distance,
      };
    } else if (url.endsWith("/retain")) retained.push(body);
    return new Response(JSON.stringify(result));
  }) as typeof fetch;
  const handlers = new Map<string, any>();
  const branch: any[] = [];
  const ctx: any = {
    cwd: root,
    hasUI: false,
    sessionManager: {
      getBranch: () => branch,
      getSessionId: () => "handoff",
      getLeafId: () => branch.filter((e) => e.id).at(-1)?.id,
    },
  };
  const pi = {
    on: (name: string, fn: any) => handlers.set(name, fn),
    registerCommand: () => {},
    registerTool: () => {},
    appendEntry: (customType: string, data: unknown) =>
      branch.push({ type: "custom", customType, data }),
  } as unknown as ExtensionAPI;
  const message = (id: string) =>
    branch.push({
      id,
      type: "message",
      timestamp: new Date().toISOString(),
      message: { role: "user", content: id },
    });
  try {
    memoryExtension(pi);
    await handlers.get("session_start")({}, ctx);
    message("old-turn");
    const recall = handlers.get("before_agent_start")(
      { prompt: "old-turn" },
      ctx,
    );
    for (let attempt = 0; !finishRecall && attempt < 20; attempt++)
      await new Promise((resolve) => setTimeout(resolve, 10));
    assert(finishRecall, "recall request must be in flight");
    state = { namespace: "project:b", revision: 1 };
    for (let attempt = 0; Number(applied) !== 1 && attempt < 10; attempt++)
      await new Promise((resolve) => setTimeout(resolve, 100));
    assert.equal(
      applied,
      1,
      "namespace must be acknowledged before the active turn finishes",
    );
    assert.equal(process.env.MEMORY_NAMESPACE, "project:b");
    finishRecall();
    assert.equal(
      await recall,
      undefined,
      "a stale recall must not inject the previous namespace's facts",
    );
    // Rapid switching back must not overwrite the active turn's retention cursor.
    for (const next of [
      { namespace: "project:a", revision: 2 },
      { namespace: "project:b", revision: 3 },
    ]) {
      state = next;
      for (
        let attempt = 0;
        Number(applied) !== next.revision && attempt < 10;
        attempt++
      )
        await new Promise((resolve) => setTimeout(resolve, 100));
      assert.equal(applied, next.revision);
    }
    await handlers.get("agent_settled")({}, ctx);
    message("new-turn");
    await handlers.get("before_agent_start")({ prompt: "new-turn" }, ctx);
    await handlers.get("agent_settled")({}, ctx);
    assert.deepEqual(
      retained.map((batch) => [
        batch.namespace,
        batch.events.map((e: any) => e.content),
      ]),
      [
        ["project:a", ["old-turn"]],
        ["project:b", ["new-turn"]],
      ],
    );
  } finally {
    await handlers.get("session_shutdown")?.({}, ctx);
    globalThis.fetch = oldFetch;
    keys.forEach((key, i) => {
      if (saved[i] === undefined) delete process.env[key];
      else process.env[key] = saved[i];
    });
    await rm(root, { recursive: true, force: true });
  }
});
