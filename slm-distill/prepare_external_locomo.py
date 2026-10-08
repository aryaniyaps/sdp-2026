"""Local academic held-out extraction diagnostic; never used for training or public raw-data upload."""
from pathlib import Path
from concurrent.futures import ThreadPoolExecutor, as_completed
from datetime import datetime
import json,hashlib,urllib.request
import prompt,rules,structured
from pi_teacher import PiTeacher
ROOT=Path(__file__).parent/'data/research-v3'
DATA=ROOT/'external-locomo'
REV='3eb6f2c585f5e1699204e3c3bdf7adc5c28cb376'
URL=f'https://raw.githubusercontent.com/snap-research/locomo/{REV}/data/locomo10.json'

def sha(text):return hashlib.sha256(text.encode()).hexdigest()
def selection():
    dest=DATA/'frozen-selection.json'
    if dest.exists():return json.loads(dest.read_text())
    source=DATA/'locomo10.curl.json';raw=source.read_text();conversations=json.loads(raw)
    all_selected=[]
    for conv in sorted(conversations,key=lambda c:sha('locomo-v1:'+c['sample_id'])):
        candidates=[];body=conv['conversation']
        for session,turns in body.items():
            if not isinstance(turns,list) or not session.startswith('session_'):continue
            if len(turns)<6:continue
            key=f"{conv['sample_id']}:{session}"
            # Choose a contiguous six-turn window without consulting labels or models.
            start=int(sha(key)[:8],16)%(len(turns)-5);window=turns[start:start+6]
            content=[f"{t['speaker']}: {t['text']}" for t in window]
            if not 400<=sum(map(len,content))<=2600:continue
            date=body[session+'_date_time'];dt=datetime.strptime(date,'%I:%M %p on %d %B, %Y')
            # No timezone supplied upstream; UTC suffix is only an API normalization convention.
            events=[{'role':'user','content':text,'occurred_at':dt.isoformat()+'Z','metadata':{'speaker':turn['speaker'],'upstream_dialog_id':turn['dia_id']}} for text,turn in zip(content,window)]
            candidates.append({'timeline':f"locomo-{conv['sample_id']}-{session}",'events':events,'provenance':{'url':URL,'revision':REV,'license':'CC-BY-NC-4.0','conversation':conv['sample_id'],'session':session,'turn_range_zero_based':[start,start+6],'dialog_ids':[t['dia_id'] for t in window],'source_date_time':date,'window_sha256':sha(json.dumps(window,sort_keys=True))}})
        candidates.sort(key=lambda x:sha('session:'+x['timeline']));all_selected.append(candidates[:2])
    selected=[items[round_] for round_ in range(2) for items in all_selected if len(items)>round_][:16]
    assert len(selected)==16 and len({x['timeline'] for x in selected})==16
    value={'selection_rule':'SHA256 locomo-v1 conversation ordering; per-conversation hashed session ordering; contiguous six-turn hashed start; 400-2600 characters; round-robin across conversations; frozen before model outputs','source_sha256':sha(raw),'windows':selected}
    dest.write_text(json.dumps(value,indent=2));return value

def label(teacher,item):
    path=DATA/'labels'/f"{item['timeline']}.json"
    if path.exists():return json.loads(path.read_text())
    events=item['events'];user=prompt.extract_prompt([],prompt.wrap_events([prompt.compact_event(e) for e in events]));wire,_,paired=structured.prepare(user)
    base=wire+'\nLabel this HELD-OUT external conversation slice with at most three high-value durable claims. Both named speakers are real conversation participants in this source; role=user is the adapter representation. Do not invent image content or missing earlier context. Session timestamps have no upstream timezone: do not infer exact UTC event times. Empty claims is valid. Use exact whole-line/sentence source_quotes.'
    instruction=base;attempts=[]
    for i in range(4):
        target=teacher.complete_json(prompt.SYSTEM,instruction,tag=item['timeline']+f'-label-{i}')
        try:
            canonical=structured.canonicalize(target,paired);claims=rules.validate_extraction(canonical,events,[])
            audit=teacher.complete_json('Independently inspect proposed memory labels against the supplied evidence. Be precise, not stylistic.',
                'Return {"acceptable":true,"issues":[]}. Check each claim is entailed, belongs to the correct named speaker, preserves important exceptions, cites all necessary events, contains no fabricated temporal precision or image contents, has consistent slot/value/statement and correct correction flag. No earlier facts exist. null valid_from is permitted; null event_at is appropriate absent exact time. Do not require exhaustive extraction; at most three high-value claims are requested.\nCONTRACT:\n'+prompt.EXTRACT_TEMPLATE.split('Existing facts:')[0]+'\nEVENTS:\n'+json.dumps(events)+'\nLABELS:\n'+json.dumps({'claims':claims}),tag=item['timeline']+f'-audit-{i}')
            if type(audit.get('acceptable')) is not bool:raise ValueError('invalid audit verdict')
            attempts.append({'target':canonical,'audit':audit})
            if not audit['acceptable']:raise ValueError('Semantic audit: '+json.dumps(audit['issues']))
            row={'task':'extract','timeline':item['timeline'],'prompt':user,'target':json.dumps({'claims':claims}), 'meta':{'raw_events':events,'existing':[],'source':item['provenance']},'split':'external-test'}
            result={'row':row,'attempts':attempts,'audit_method':'same-family separate teacher review, not human ground truth'};path.write_text(json.dumps(result,indent=2));return result
        except Exception as e:
            attempts.append({'error':str(e)})
            instruction=base+'\nCorrect the preceding label errors, without changing evidence. Error: '+str(e)+'\nPrevious labels: '+json.dumps(target)
    (DATA/'labels'/f"{item['timeline']}.failed.json").write_text(json.dumps(attempts,indent=2));raise ValueError(item['timeline']+' failed four label/audit attempts')

def main():
    (DATA/'labels').mkdir(parents=True,exist_ok=True)
    source=DATA/'locomo10.curl.json'
    if not source.exists():
        with urllib.request.urlopen(URL,timeout=45) as response: source.write_bytes(response.read())
    license_path=DATA/'LICENSE.txt'
    if not license_path.exists():
        with urllib.request.urlopen(f'https://raw.githubusercontent.com/snap-research/locomo/{REV}/LICENSE.txt',timeout=30) as response: license_path.write_bytes(response.read())
    assert 'Attribution-NonCommercial 4.0' in license_path.read_text()
    selected=selection();teacher=PiTeacher(DATA/'teacher-cache',DATA/'teacher-usage.jsonl');results={};failures=[]
    with ThreadPoolExecutor(4) as pool:
        jobs={pool.submit(label,teacher,item):item['timeline'] for item in selected['windows']}
        for future in as_completed(jobs):
            tid=jobs[future]
            try:results[tid]=future.result();print(tid,'audited',flush=True)
            except Exception as e:failures.append(str(e));print(str(e),flush=True)
    if failures:raise ValueError(failures)
    rows=[results[x['timeline']]['row'] for x in selected['windows']];text=''.join(json.dumps(r,ensure_ascii=False)+'\n' for r in rows)
    (ROOT/'external-locomo-16.jsonl').write_text(text)
    manifest={k:v for k,v in selected.items() if k!='windows'}
    manifest.update({'rows':16,'distinct_conversations':len({r['meta']['source']['conversation'] for r in rows}),'distinct_sessions':16,'revision':REV,'source_url':URL,'license':'CC-BY-NC-4.0','local_academic_evaluation_only':True,'training_use':False,'official_locomo_qa_score':False,'file_sha256':sha(text),'audit':'All 16 labels structurally validated and same-family semantically audited; repaired labels retain original attempts. Not independent human gold.','timezone_note':'Unzoned source session wall time normalized with Z solely to satisfy API; no claim of actual UTC timezone.','teacher':teacher.summary()})
    (ROOT/'external-locomo-manifest.json').write_text(json.dumps(manifest,indent=2));print('READY',manifest,flush=True)
if __name__=='__main__':main()
