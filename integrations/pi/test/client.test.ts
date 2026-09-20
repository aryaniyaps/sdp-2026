import { test } from "node:test";
import assert from "node:assert/strict";
import { mkdtemp, readdir, rm } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { createServer } from "node:http";
import { MemoryClient, observedExitCode } from "../client.ts";
test("offline spool survives restart and delivers one idempotent batch", async () => {
 const dir = await mkdtemp(join(tmpdir(), "memory-spool-"));
 const batch = { namespace: "test", session_id: "s", external_id: "turn-1", metadata: {}, events: [{ role: "user", content: "Remember SQL", occurred_at: "2026-01-01T00:00:00Z", metadata: {} }] };
 const offline = new MemoryClient("http://127.0.0.1:1", dir, 50);
 await offline.enqueue(batch); await offline.enqueue(batch); assert.equal((await offline.flush()).pending, 1);
 let deliveries = 0;
 const server = createServer((req, res) => { let body = ""; req.on("data", chunk => body += chunk); req.on("end", () => { assert.deepEqual(JSON.parse(body), batch); deliveries++; res.setHeader("content-type", "application/json"); res.end('{"created":true}'); }); });
 await new Promise<void>(resolve => server.listen(0, "127.0.0.1", resolve));
 try { const port = (server.address() as {port:number}).port; const restarted = new MemoryClient(`http://127.0.0.1:${port}`, dir); assert.equal((await restarted.flush()).delivered, 1); assert.equal((await restarted.flush()).delivered, 0); assert.equal(deliveries, 1); assert.deepEqual(await readdir(dir), []); }
 finally { server.close(); await rm(dir, {recursive:true,force:true}); }
});
test("assistant success claims are not observed exit codes", () => { assert.equal(observedExitCode(undefined, false), null); assert.equal(observedExitCode({ exitCode: 0 }, false), 0); assert.equal(observedExitCode({ exitCode: 2 }, true), 2); });
