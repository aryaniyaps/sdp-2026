import { test } from 'node:test';
import assert from 'node:assert/strict';
import { mkdtemp, mkdir, rm } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import memoryExtension from '../extension.ts';
import { AUTOMATIC_RECALL_MAX_DISTANCE } from '../client.ts';
import type { ExtensionAPI } from '@earendil-works/pi-coding-agent';

type Handlers = Map<string, (event: any, ctx: any) => Promise<any>>;
function session(cwd: string, id: string) {
  const handlers: Handlers = new Map();
  const branch: any[] = [];
  const fake = { on: (name: string, handler: any) => handlers.set(name, handler), appendEntry: (customType: string, data: any) => branch.push({ type: 'custom', customType, data }), registerTool: () => {}, registerCommand: () => {} };
  memoryExtension(fake as unknown as ExtensionAPI);
  const ctx = { cwd, hasUI: false, sessionManager: { getBranch: () => branch, getSessionId: () => id, getLeafId: () => 'leaf' } };
  return { handlers, ctx };
}

test('sessions in different directories share one namespace and one spool', async () => {
  const root = await mkdtemp(join(tmpdir(), 'memory-namespace-'));
  const saved = { fetch: globalThis.fetch, home: process.env.HOME, user: process.env.USER, namespace: process.env.MEMORY_NAMESPACE, spool: process.env.MEMORY_SPOOL, stderr: process.stderr.write };
  delete process.env.MEMORY_NAMESPACE; delete process.env.MEMORY_SPOOL;
  process.env.HOME = join(root, 'home'); process.env.USER = 'tester';
  process.stderr.write = (() => true) as typeof process.stderr.write;
  const a = join(root, 'project-a'), b = join(root, 'elsewhere', 'project-b');
  await mkdir(a, { recursive: true }); await mkdir(b, { recursive: true });
  const recalls: any[] = []; const retained: any[] = []; let offline = true;
  globalThis.fetch = (async (url: string, options: RequestInit) => {
    const body = JSON.parse((options?.body as string) ?? 'null');
    if (url.endsWith('/recall')) { recalls.push(body); return new Response(JSON.stringify({ context: 'earlier fact', trace_id: 't', results: [], degraded_reasons: [], max_distance: body.max_distance })); }
    if (offline) throw new Error('offline');
    retained.push(body);
    return new Response(JSON.stringify({ retained: true }));
  }) as typeof fetch;
  try {
    const first = session(a, 'session-a');
    await first.handlers.get('session_start')!({}, first.ctx);
    const injected = await first.handlers.get('before_agent_start')!({ prompt: 'what did I say?' }, first.ctx);
    // The first directory queues evidence while the service is down.
    await first.handlers.get('tool_result')!({ toolName: 'bash', toolCallId: 'c1', content: [{ type: 'text', text: 'remembered in A' }], input: {}, isError: false }, first.ctx);
    assert.equal(retained.length, 0);

    const second = session(b, 'session-b');
    offline = false;
    await second.handlers.get('session_start')!({}, second.ctx);
    const injectedB = await second.handlers.get('before_agent_start')!({ prompt: 'what did I say?' }, second.ctx);

    assert.equal(injected.message.details.namespace, 'user:tester');
    assert.equal(injectedB.message.details.namespace, 'user:tester');
    assert.deepEqual(recalls.map(r => r.namespace), ['user:tester', 'user:tester']);
    // Automatic recall must not wait for the server's LLM date planner.
    assert(recalls.every(r => r.temporal === false));
    // Interactive recall keeps raw evidence in play so an extraction mistake cannot hide it.
    assert(recalls.every(r => r.include_raw === true));
    // Automatic recall drops vector matches beyond the calibrated distance, so an unrelated prompt injects nothing.
    assert.equal(AUTOMATIC_RECALL_MAX_DISTANCE, 0.45);
    assert(recalls.every(r => r.max_distance === 0.45));
    // Directory B delivered the evidence directory A had queued.
    assert.equal(retained.length, 1);
    assert.equal(retained[0].namespace, 'user:tester');
    assert.equal(retained[0].session_id, 'session-a');
    // An empty prompt does not ask the service to search for nothing.
    assert.equal(await second.handlers.get('before_agent_start')!({ prompt: '  ' }, second.ctx), undefined);
    assert.equal(recalls.length, 2);
  } finally {
    globalThis.fetch = saved.fetch; process.stderr.write = saved.stderr;
    for (const [key, value] of [['HOME', saved.home], ['USER', saved.user], ['MEMORY_NAMESPACE', saved.namespace], ['MEMORY_SPOOL', saved.spool]] as const) { if (value === undefined) delete process.env[key]; else process.env[key] = value; }
    await rm(root, { recursive: true, force: true });
  }
});

test('MEMORY_NAMESPACE still selects a separate memory', async () => {
  const root = await mkdtemp(join(tmpdir(), 'memory-namespace-override-'));
  const saved = { fetch: globalThis.fetch, namespace: process.env.MEMORY_NAMESPACE, spool: process.env.MEMORY_SPOOL };
  process.env.MEMORY_NAMESPACE = 'project:private'; process.env.MEMORY_SPOOL = join(root, 'spool');
  const recalls: any[] = [];
  globalThis.fetch = (async (_url: string, options: RequestInit) => { const body = JSON.parse(options.body as string); recalls.push(body); return new Response(JSON.stringify({ context: 'x', trace_id: 't', results: [], degraded_reasons: [], max_distance: body.max_distance })); }) as typeof fetch;
  try {
    const s = session(root, 'override');
    await s.handlers.get('session_start')!({}, s.ctx);
    await s.handlers.get('before_agent_start')!({ prompt: 'hello' }, s.ctx);
    assert.equal(recalls[0].namespace, 'project:private');
  } finally {
    globalThis.fetch = saved.fetch;
    for (const [key, value] of [['MEMORY_NAMESPACE', saved.namespace], ['MEMORY_SPOOL', saved.spool]] as const) { if (value === undefined) delete process.env[key]; else process.env[key] = value; }
    await rm(root, { recursive: true, force: true });
  }
});

test('automatic recall sets a distance cutoff and the explicit memory_recall tool does not', async () => {
  const root = await mkdtemp(join(tmpdir(), 'memory-distance-'));
  const saved = { fetch: globalThis.fetch, namespace: process.env.MEMORY_NAMESPACE, spool: process.env.MEMORY_SPOOL };
  process.env.MEMORY_NAMESPACE = 'project:distance'; process.env.MEMORY_SPOOL = join(root, 'spool');
  const recalls: any[] = [];
  globalThis.fetch = (async (_url: string, options: RequestInit) => { const body = JSON.parse(options.body as string); recalls.push(body); return new Response(JSON.stringify({ context: '', trace_id: 't', results: [], degraded_reasons: [], ...('max_distance' in body ? { max_distance: body.max_distance } : {}) })); }) as typeof fetch;
  const handlers: Handlers = new Map(); const tools = new Map<string, any>(); const branch: any[] = [];
  const fake = { on: (name: string, handler: any) => handlers.set(name, handler), appendEntry: (customType: string, data: any) => branch.push({ type: 'custom', customType, data }), registerTool: (tool: any) => tools.set(tool.name, tool), registerCommand: () => {} };
  const ctx = { cwd: root, hasUI: false, sessionManager: { getBranch: () => branch, getSessionId: () => 'distance', getLeafId: () => 'leaf' } };
  try {
    memoryExtension(fake as unknown as ExtensionAPI);
    await handlers.get('session_start')!({}, ctx);
    // Nothing is injected when the server found nothing close enough.
    assert.equal(await handlers.get('before_agent_start')!({ prompt: 'thanks, that makes sense' }, ctx), undefined);
    await tools.get('memory_recall').execute('call', { query: 'what is the release branch?' }, undefined, undefined, ctx);
    assert.equal(recalls.length, 2);
    assert.equal(recalls[0].query, 'thanks, that makes sense');
    assert.equal(recalls[0].max_distance, AUTOMATIC_RECALL_MAX_DISTANCE);
    assert.equal(recalls[1].query, 'what is the release branch?');
    // The model asked a deliberate question: no cutoff, so the field is absent, not null.
    assert(!('max_distance' in recalls[1]));
    assert.equal(recalls[1].include_raw, true);
    assert.equal(recalls[1].temporal, true);
  } finally {
    globalThis.fetch = saved.fetch;
    for (const [key, value] of [['MEMORY_NAMESPACE', saved.namespace], ['MEMORY_SPOOL', saved.spool]] as const) { if (value === undefined) delete process.env[key]; else process.env[key] = value; }
    await rm(root, { recursive: true, force: true });
  }
});

test('automatic recall warns when the memory service ignores the cutoff, as a service built before it does', async () => {
  const root = await mkdtemp(join(tmpdir(), 'memory-old-service-'));
  const saved = { fetch: globalThis.fetch, namespace: process.env.MEMORY_NAMESPACE, spool: process.env.MEMORY_SPOOL };
  process.env.MEMORY_NAMESPACE = 'project:old-service'; process.env.MEMORY_SPOOL = join(root, 'spool');
  // The reply carries no max_distance: the old service accepted the field and ignored it.
  globalThis.fetch = (async () => new Response(JSON.stringify({ context: 'unrelated text', trace_id: 't', results: [], degraded_reasons: [] }))) as typeof fetch;
  const handlers: Handlers = new Map(); const branch: any[] = []; const notices: string[] = [];
  const fake = { on: (name: string, handler: any) => handlers.set(name, handler), appendEntry: (customType: string, data: any) => branch.push({ type: 'custom', customType, data }), registerTool: () => {}, registerCommand: () => {} };
  const ctx = { cwd: root, hasUI: true, ui: { setStatus: (_key: string, value: string) => notices.push(value) }, sessionManager: { getBranch: () => branch, getSessionId: () => 'old-service', getLeafId: () => 'leaf' } };
  try {
    memoryExtension(fake as unknown as ExtensionAPI);
    await handlers.get('session_start')!({}, ctx);
    const injected = await handlers.get('before_agent_start')!({ prompt: 'what is the release branch called?' }, ctx);
    // The recall still happens as it did before the cutoff existed, but the user is told why it is not limited.
    assert.equal(injected.message.customType, 'sdp-memory-context');
    assert(notices.some(value => value.includes('did not apply max_distance 0.45') && value.includes('restart the service')), notices.join('|'));
  } finally {
    globalThis.fetch = saved.fetch;
    for (const [key, value] of [['MEMORY_NAMESPACE', saved.namespace], ['MEMORY_SPOOL', saved.spool]] as const) { if (value === undefined) delete process.env[key]; else process.env[key] = value; }
    await rm(root, { recursive: true, force: true });
  }
});
