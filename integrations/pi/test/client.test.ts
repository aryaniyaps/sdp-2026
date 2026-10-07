import { test } from "node:test";
import assert from "node:assert/strict";
import { mkdtemp, readdir, rm } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { createServer } from "node:http";
import {
  AUTOMATIC_RECALL_MAX_DISTANCE,
  MemoryClient,
  observedExitCode,
} from "../client.ts";
test("offline spool survives restart and delivers one idempotent batch", async () => {
  const dir = await mkdtemp(join(tmpdir(), "memory-spool-"));
  const batch = {
    namespace: "test",
    session_id: "s",
    external_id: "turn-1",
    metadata: {},
    events: [
      {
        role: "user",
        content: "Remember SQL",
        occurred_at: "2026-01-01T00:00:00Z",
        metadata: {},
      },
    ],
  };
  const offline = new MemoryClient("http://127.0.0.1:1", dir, 50);
  await offline.enqueue(batch);
  await offline.enqueue(batch);
  assert.equal((await offline.flush()).pending, 1);
  let deliveries = 0;
  const server = createServer((req, res) => {
    let body = "";
    req.on("data", (chunk) => (body += chunk));
    req.on("end", () => {
      assert.deepEqual(JSON.parse(body), batch);
      deliveries++;
      res.setHeader("content-type", "application/json");
      res.end('{"created":true}');
    });
  });
  await new Promise<void>((resolve) => server.listen(0, "127.0.0.1", resolve));
  try {
    const port = (server.address() as { port: number }).port;
    const restarted = new MemoryClient(`http://127.0.0.1:${port}`, dir);
    assert.equal((await restarted.flush()).delivered, 1);
    assert.equal((await restarted.flush()).delivered, 0);
    assert.equal(deliveries, 1);
    assert.deepEqual(await readdir(dir), []);
  } finally {
    server.close();
    await rm(dir, { recursive: true, force: true });
  }
});
test("assistant success claims are not observed exit codes", () => {
  assert.equal(observedExitCode(undefined, false), null);
  assert.equal(observedExitCode({ exitCode: 0 }, false), 0);
  assert.equal(observedExitCode({ exitCode: 2 }, true), 2);
});
test("recall sends max_distance only when asked and refuses a value the server would not accept", async () => {
  const bodies: any[] = [];
  const server = createServer((req, res) => {
    let body = "";
    req.on("data", (chunk) => (body += chunk));
    req.on("end", () => {
      const sent = JSON.parse(body);
      bodies.push(sent);
      res.setHeader("content-type", "application/json");
      res.end(
        JSON.stringify({
          context: "",
          trace_id: "t",
          results: [],
          degraded_reasons: [],
          ...("max_distance" in sent
            ? { max_distance: sent.max_distance }
            : {}),
        }),
      );
    });
  });
  await new Promise<void>((resolve) => server.listen(0, "127.0.0.1", resolve));
  try {
    const port = (server.address() as { port: number }).port;
    const client = new MemoryClient(`http://127.0.0.1:${port}`, tmpdir());
    await client.recall("ns", "plain");
    await client.recall("ns", "bounded", {
      maxDistance: AUTOMATIC_RECALL_MAX_DISTANCE,
    });
    await client.recall("ns", "widest", { maxDistance: 2 });
    assert.equal(AUTOMATIC_RECALL_MAX_DISTANCE, 0.45);
    assert(!("max_distance" in bodies[0]));
    assert.equal(bodies[1].max_distance, 0.45);
    assert.equal(bodies[2].max_distance, 2);
    // JSON would turn NaN into null, which the server reads as no cutoff, so the client refuses it before sending.
    for (const bad of [0, -1, 2.5, NaN, Infinity])
      await assert.rejects(
        client.recall("ns", "bad", { maxDistance: bad }),
        /maxDistance must be greater than 0 and at most 2/,
      );
    assert.equal(bodies.length, 3);
  } finally {
    server.close();
  }
});
test("recall says so when the service did not apply the cutoff it was asked for", async () => {
  // A service built before max_distance existed ignores the field and does not echo it.
  const server = createServer((req, res) => {
    req.resume();
    req.on("end", () => {
      res.setHeader("content-type", "application/json");
      res.end(
        '{"context":"unrelated text","trace_id":"t","results":[],"degraded_reasons":["graph projection has 1 outstanding jobs"]}',
      );
    });
  });
  await new Promise<void>((resolve) => server.listen(0, "127.0.0.1", resolve));
  try {
    const port = (server.address() as { port: number }).port;
    const client = new MemoryClient(`http://127.0.0.1:${port}`, tmpdir());
    const bounded = await client.recall("ns", "bounded", {
      maxDistance: AUTOMATIC_RECALL_MAX_DISTANCE,
    });
    // The reasons the service gave stay, and the missing cutoff is added to them, loudly and with the way out.
    assert.equal(bounded.degraded_reasons.length, 2);
    assert.equal(
      bounded.degraded_reasons[0],
      "graph projection has 1 outstanding jobs",
    );
    assert.match(
      bounded.degraded_reasons[1],
      /did not apply max_distance 0\.45/,
    );
    assert.match(bounded.degraded_reasons[1], /restart the service/);
    // Nothing was asked, so nothing is missing.
    assert.deepEqual((await client.recall("ns", "plain")).degraded_reasons, [
      "graph projection has 1 outstanding jobs",
    ]);
  } finally {
    server.close();
  }
});
