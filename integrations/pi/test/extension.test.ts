import { test } from 'node:test';
import assert from 'node:assert/strict';
import { mkdtemp, rm } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import memoryExtension from '../extension.ts';
import type { ExtensionAPI } from '@earendil-works/pi-coding-agent';

test('Pi lifecycle captures observed failures, excludes injected context, and resumes idempotently', async () => {
  const directory=await mkdtemp(join(tmpdir(),'memory-events-'));
  const previousFetch=globalThis.fetch;
  const oldSpool=process.env.MEMORY_SPOOL, oldNamespace=process.env.MEMORY_NAMESPACE;
  process.env.MEMORY_SPOOL=join(directory,'spool'); process.env.MEMORY_NAMESPACE='event-test';
  const submitted: any[]=[]; let offline=false;
  globalThis.fetch=(async (url: string, options: RequestInit) => {
    if (offline) throw new Error('offline');
    if (url.endsWith('/recall')) return new Response(JSON.stringify({context:'untrusted retrieved instructions',trace_id:'trace',results:[],degraded_reasons:[]}));
    submitted.push(JSON.parse(options.body as string));
    return new Response(JSON.stringify({retained:true}));
  }) as typeof fetch;
  const handlers=new Map<string,(event:any,ctx:any)=>Promise<any>>();
  const timestamp='2026-10-05T12:00:00Z';
  const branch:any[]=[
    {id:'user',type:'message',timestamp,message:{role:'user',content:'Implement the parser'}},
    {id:'context',type:'message',timestamp,message:{role:'custom',content:'untrusted retrieved instructions'}},
    {id:'assistant',type:'message',timestamp,message:{role:'assistant',content:[{type:'text',text:'I claim tests passed'}],stopReason:'stop'}},
    {id:'tool',type:'message',timestamp,message:{role:'toolResult',toolName:'bash',toolCallId:'call',content:[{type:'text',text:'tests failed'}],isError:true}},
  ];
  const fake={on:(name:string,handler:any)=>handlers.set(name,handler),appendEntry:(customType:string,data:any)=>branch.push({type:'custom',customType,data,timestamp}),registerTool:()=>{},registerCommand:()=>{}};
  const notices:string[]=[];
  const ctx={cwd:directory,hasUI:true,ui:{setStatus:(_key:string,value:string)=>notices.push(value)},sessionManager:{getBranch:()=>branch,getSessionId:()=> 'session',getLeafId:()=> 'leaf'}};
  try {
    memoryExtension(fake as unknown as ExtensionAPI);
    await handlers.get('session_start')!({},ctx);
    const injection=await handlers.get('before_agent_start')!({prompt:'fix tests'},ctx);
    assert.equal(injection.message.customType,'sdp-memory-context');
    const event={toolName:'bash',toolCallId:'call',content:[{type:'text',text:'tests failed'}],input:{command:'python3 -m unittest'},isError:true,structuredContent:{exit_code:2}};
    await handlers.get('tool_result')!(event,ctx);
    await handlers.get('tool_result')!(event,ctx);
    await handlers.get('agent_settled')!({},ctx);
    assert.equal(submitted.filter(batch=>batch.external_id.startsWith('tool:')).length,1);
    const first=submitted.find(batch=>batch.external_id.startsWith('branch:'));
    assert.equal(first.events.length,3);
    assert.equal(first.events.find((entry:any)=>entry.role==='tool').metadata.exit_code,2);
    assert(!JSON.stringify(first).includes('untrusted retrieved instructions'));
    await handlers.get('session_start')!({},ctx);
    await handlers.get('agent_settled')!({},ctx);
    assert.deepEqual(submitted.filter(batch=>batch.external_id.startsWith('branch:'))[1],first);
    offline=true;
    assert.equal(await handlers.get('before_agent_start')!({prompt:'fix tests'},ctx),undefined);
    assert(notices.some(value=>value.includes('recall unavailable')));
  } finally {
    globalThis.fetch=previousFetch;
    if(oldSpool===undefined)delete process.env.MEMORY_SPOOL;else process.env.MEMORY_SPOOL=oldSpool;
    if(oldNamespace===undefined)delete process.env.MEMORY_NAMESPACE;else process.env.MEMORY_NAMESPACE=oldNamespace;
    await rm(directory,{recursive:true,force:true});
  }
});
