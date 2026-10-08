"""Build a larger, provenance-tracked memory corpus; split public data by repository.

Public coding traces are relabeled as memory extraction, never used as raw chat targets.
Synthetic cases deliberately cover updates, abstention, source roles and tool noise.
"""
from pathlib import Path
from concurrent.futures import ThreadPoolExecutor, as_completed
import json, hashlib, random, uuid, argparse
import prompt, rules, structured
from pi_teacher import PiTeacher

ROOT=Path(__file__).parent/'data'/'research-v2'
TOPICS=['package manager replacement','database migration and rollback','service ports and Docker networking','dependency versions and compatibility','user coding preferences','CI failures and confirmed fixes','authentication provider correction','deployment approvals and environments','project ownership and deadlines','GPU memory and training settings','test runner and flaky test workaround','API schema and pagination','knowledge graph projection and source of truth','user retracts an earlier claim','unconfirmed assistant suggestion versus tool evidence','code-only tool noise with no durable memory']

def validate_case(case,tid):
    events=case['events'];existing=case.get('existing',[])
    for e in events:
        e.setdefault('occurred_at','2026-04-15T10:00:00Z');e.setdefault('metadata',{})
    claims=rules.validate_extraction({'claims':case['claims']},events,existing)
    # Training mirrors the serving window exactly.
    compact=[prompt.compact_event(e) for e in events]
    user=prompt.extract_prompt(existing,prompt.wrap_events(compact))
    return {'task':'extract','timeline':tid,'prompt':user,'target':json.dumps({'claims':claims}), 'meta':{'raw_events':events,'existing':existing,'source':'research-v2'}}

def generate_batch(teacher,index):
    path=ROOT/'synthetic'/f'batch-{index:03d}.json';path.parent.mkdir(exist_ok=True)
    if path.exists():return json.loads(path.read_text())
    topic=TOPICS[index%len(TOPICS)];project=f'Northstar-{index:03d}'
    instruction=f'''Create four DISTINCT compact coding-session memory extraction examples about {topic}. Project name is {project}; use different specific software stacks, values, speaking styles and event sequences. These are synthetic fixtures. Return {{"cases":[{{"events":[{{"role":"user|assistant|tool","content":"...","occurred_at":"2026-04-15T10:00:00Z","metadata":{{}}}}],"existing":[],"claims":[...]}}]}}.
Each case has 3-6 events, total input under 1800 characters, and 0-3 high-value claims. Include one correction with existing facts (real generated UUID ids, subject/name, predicate, value, statement), one user decision with tool evidence, one unconfirmed assistant suggestion that must not be treated as an established fact, and one abstention/noise or synthetic secret case. For secrets use ONLY obviously synthetic sk-test-... strings. Never use real credentials. Every source quote must be an exact full line or sentence of its cited event. Claims use the complete canonical schema from the worker instructions below. Distinguish statements the user rejected from their chosen final decision. Existing fact objects have id,subject,predicate,value,statement; related references only their exact UUIDs. Do not use invented enum values.
WORKER INSTRUCTIONS:\n{prompt.EXTRACT_TEMPLATE.split('Existing facts:')[0]}'''
    value=teacher.complete_json('You design precise supervised learning examples for a memory extraction worker.',instruction,tag=f'research-synthetic-{index}')
    rows=[];errors=[]
    for i,case in enumerate(value['cases']):
        try:rows.append(validate_case(case,f'research-synthetic-{index:03d}-{i}'))
        except Exception as e:errors.append(str(e))
    result={'rows':rows,'errors':errors,'index':index};path.write_text(json.dumps(result,ensure_ascii=False));return result

def public_label(teacher,item):
    tid=item['timeline'];path=ROOT/'public-labels'/f'{tid}.json';path.parent.mkdir(exist_ok=True)
    if path.exists():return json.loads(path.read_text())
    user=prompt.extract_prompt([],prompt.wrap_events(item['events']))
    wire,_,paired=structured.prepare(user)
    instruction=wire+'\nFor this public coding trace, extract at most three high-value project facts or goals. Skip code implementation details, generated file contents, and unverified completion claims. Copy an exact complete source line/sentence for each quote. Empty claims is correct when there is no durable memory.'
    error=None
    for attempt in range(3):
        value=teacher.complete_json(prompt.SYSTEM,instruction,tag=tid)
        try:
            value=structured.canonicalize(value,paired)
            claims=rules.validate_extraction(value,item['events'],[])
            break
        except Exception as exc:
            error=str(exc)
            instruction=wire+'\nPrevious response failed validation: '+error+'\nReturn a corrected complete JSON object. Use source_quotes pairs and exact source text. At most three durable claims. Previous response: '+json.dumps(value)
    else:
        raise ValueError(error)
    row={'task':'extract','timeline':tid,'prompt':user,'target':json.dumps({'claims':claims}), 'meta':{'raw_events':item['events'],'existing':[],'source':item['provenance']},'split':item['split']}
    path.write_text(json.dumps(row,ensure_ascii=False));return row

def to_wire(row):
    # Decoder chooses complete lines/sentences. Widen a teacher's shorter exact quote to
    # the shortest available containing span, retaining the same source and statement.
    row={**row};target=json.loads(row['target'])
    if row['task']=='extract':
        _,tail=structured.first_json(row['prompt'].split('\nExisting facts: ',1)[1]);events,_=structured.first_json(tail.lstrip()[len('Events: '):])
        by_index={e['source_index']:e['event']['content'] for e in events}
        for c in target['claims']:
            c['quotes']=[min((s for s in structured.quote_candidates(by_index[i]) if q in s),key=len) for i,q in zip(c['source_indices'],c['quotes'],strict=True)]
    row['target']=json.dumps(target,ensure_ascii=False)
    return structured.training_row(row)

def main():
    ap=argparse.ArgumentParser();ap.add_argument('--synthetic-batches',type=int,default=40);args=ap.parse_args()
    ROOT.mkdir(exist_ok=True);teacher=PiTeacher(ROOT/'teacher-cache',ROOT/'teacher-usage.jsonl')
    tasks=[];failures=[]
    with ThreadPoolExecutor(8) as pool:
        jobs={pool.submit(generate_batch,teacher,i):('synthetic',i) for i in range(args.synthetic_batches)}
        p=ROOT/'public-windows.jsonl'
        if p.exists():
            for line in p.read_text().splitlines():
                item=json.loads(line);jobs[pool.submit(public_label,teacher,item)]=('public',item['timeline'])
        for future in as_completed(jobs):
            kind,key=jobs[future]
            try:
                result=future.result();print(kind,key,'ok',len(result.get('rows',[])) if kind=='synthetic' else '',flush=True)
            except Exception as exc:
                failures.append({'kind':kind,'key':key,'error':str(exc)});print('FAILED',kind,key,str(exc)[:200],flush=True)
    rows={'train':[],'val':[],'test':[]}
    old=Path(__file__).parent/'data'
    for split in rows:
        rows[split]+=[json.loads(l) for l in (old/'pi-corpus'/'sets'/f'{split}.jsonl').read_text().splitlines()]
    replay=[json.loads(l) for l in (old/'bootstrap-sets'/'train.jsonl').read_text().splitlines()];random.Random(13).shuffle(replay);rows['train']+=replay[:126]
    for path in sorted((ROOT/'synthetic').glob('*.json')):
        result=json.loads(path.read_text());i=result['index'];split='train' if i<32 else ('val' if i<36 else 'test');rows[split]+=result['rows']
        failures += [{'kind':'synthetic-validation','key':i,'error':e} for e in result['errors']]
    for path in sorted((ROOT/'public-labels').glob('*.json')):
        row=json.loads(path.read_text());rows[row['split']].append(row)
    sets=ROOT/'sets';sets.mkdir(exist_ok=True);canonical=ROOT/'canonical';canonical.mkdir(exist_ok=True)
    manifest={'failures':failures,'counts':{},'teacher':teacher.summary(),'sources':{'public':'https://huggingface.co/datasets/nebius/SWE-rebench-openhands-trajectories','license':'CC-BY-4.0','authors':'Nebius; Trofimova et al. 2025'},'protocol':'public repositories and synthetic project names disjoint by split; previous test used during 1.7B diagnosis is development, new test cases untouched'}
    for split,rs in rows.items():
        accepted=[];canonical_rows=[]
        for r in rs:
            try:accepted.append(to_wire(r));canonical_rows.append(r)
            except Exception as e:failures.append({'kind':'wire-conversion','key':r['timeline'],'error':str(e)})
        random.Random(13).shuffle(accepted)
        text=''.join(json.dumps(r,ensure_ascii=False)+'\n' for r in accepted);(sets/f'{split}.jsonl').write_text(text)
        (canonical/f'{split}.jsonl').write_text(''.join(json.dumps(r,ensure_ascii=False)+'\n' for r in canonical_rows))
        manifest['counts'][split]={'rows':len(accepted),'sha256':hashlib.sha256(text.encode()).hexdigest()}
    (ROOT/'manifest.json').write_text(json.dumps(manifest,indent=2));print(json.dumps(manifest['counts']),flush=True)

if __name__=='__main__':main()
