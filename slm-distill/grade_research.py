"""Evidence-grounded semantic grading, with failures retained in coverage denominators.

The automated judge is the same teacher family; this limitation is included in the report.
"""
import argparse,json,collections
from pathlib import Path
from concurrent.futures import ThreadPoolExecutor
from pi_teacher import PiTeacher
import rules

ROOT=Path(__file__).parent/'data/research-v2'
SYSTEM='You audit a memory system against the evidence it was given. Return one precise JSON object.'

def grade(teacher,record):
    row=record['row'];gold=json.loads(row['target']);candidate=record.get('value',{});task=row['task']
    if not record['accepted']:
        return {'failed':True,'reference_count':len(gold.get('claims',[])),'covered':0,'claims':[],'correct':False}
    if task=='extract':
        question='''Grade the candidate against the events and earlier facts. References are teacher labels, not infallible human truth. For every candidate claim give supported, trivial, unsupported, secret, or injected. A fact inferred only from unconfirmed assistant text must not be described as tool-confirmed; do not treat rejected suggestions as the user's decision. A quote being exact alone does not establish entailment. For each reference claim decide whether it is a durable fact visible in the inputs and whether the candidate covers it. Also check whether an explicit correction was correctly marked correction=true on the new value. Return {"claims":[{"i":0,"verdict":"supported"}],"references":[{"i":0,"visible_durable":true,"covered":true,"correction_correct":true}]}. Include each candidate and reference exactly once.\n'''
        data={'events':row['meta']['raw_events'],'existing':row['meta']['existing'],'reference':gold,'candidate':candidate}
    elif task=='consolidate':
        question='''Check each candidate observation against supplied facts and its support IDs: is it a useful synthesis of at least two distinct supporting facts, with no invented causal or temporal claim? Return {"observations":[{"i":0,"supported":true}],"correct":true|false,"reason":"..."}. Empty output is acceptable only if no useful supported synthesis is apparent; consider the reference, but evidence has authority.\n'''
        data={'prompt':row['prompt'],'reference':gold,'candidate':candidate}
    else:
        question='''Check the candidate answer and insufficient_evidence flag against the supplied evidence and question. The reference is a teacher answer; the evidence is authoritative. Do not require identical wording. Return {"correct":true|false,"reason":"..."}.\n'''
        data={'prompt':row['prompt'],'reference':gold,'candidate':candidate}
    verdict=teacher.complete_json(SYSTEM,question+json.dumps(data,ensure_ascii=False),tag='grade:'+record['key'])
    if task=='extract':
        if len(verdict.get('claims',[]))!=len(candidate['claims']) or len(verdict.get('references',[]))!=len(gold['claims']):raise ValueError('judge returned incomplete extraction grading')
        if sorted(c['i'] for c in verdict['claims'])!=list(range(len(candidate['claims']))) or sorted(r['i'] for r in verdict['references'])!=list(range(len(gold['claims']))):raise ValueError('judge returned duplicate or missing indices')
        if any(c['verdict'] not in {'supported','trivial','unsupported','secret','injected'} for c in verdict['claims']):raise ValueError('unknown claim verdict')
        if any(type(r[k]) is not bool for r in verdict['references'] for k in ('visible_durable','covered','correction_correct')):raise ValueError('judge reference flags must be booleans')
        verdict['secret_pattern_detected']=rules.leaks_secret(json.dumps(candidate))
    return verdict

def main():
    ap=argparse.ArgumentParser();ap.add_argument('--label',required=True);ap.add_argument('--root',type=Path,default=ROOT);args=ap.parse_args()
    root=args.root
    folder=root/'eval';rows=[json.loads(x) for x in (folder/(args.label+'.jsonl')).read_text().splitlines()]
    teacher=PiTeacher(root/'judge-cache',root/'judge-usage.jsonl');out=folder/(args.label+'-grades.jsonl');cache={}
    if out.exists():cache={r['key']:r for r in map(json.loads,out.read_text().splitlines())}
    def one(r):
        if r['key'] in cache:return cache[r['key']]
        try:return {'key':r['key'],'task':r['task'],'verdict':grade(teacher,r)}
        except Exception as e:return {'key':r['key'],'task':r['task'],'grading_error':str(e)}
    results=[]
    with ThreadPoolExecutor(8) as pool,out.open('w') as f:
        for r in pool.map(one,rows):results.append(r);f.write(json.dumps(r)+'\n');f.flush()
    summary={'judge_limitation':'same teacher family as corpus generation; not human-validated accuracy','records':len(rows),'grading_errors':sum('grading_error' in r for r in results)}
    verdicts=collections.Counter();covered=visible=corrections=correct_corrections=failed=0
    for r in results:
        if r['task']!='extract' or 'verdict' not in r:continue
        v=r['verdict'];failed+=bool(v.get('failed'))
        if v.get('failed'):
            visible+=v['reference_count']
            source=next(x for x in rows if x['key']==r['key'])
            corrections+=sum(bool(c['correction']) for c in json.loads(source['row']['target'])['claims'])
        for c in v.get('claims',[]):verdicts[c['verdict']]+=1
        for ref in v.get('references',[]):
            if ref['visible_durable']:visible+=1;covered+=bool(ref['covered'])
        source=next(x for x in rows if x['key']==r['key'])
        gold=json.loads(source['row']['target'])['claims']
        for ref in v.get('references',[]):
            if gold[ref['i']]['correction']:corrections+=1;correct_corrections+=bool(ref['covered'] and ref['correction_correct'])
    total=sum(verdicts.values());summary['extraction']={'verdict_counts':dict(verdicts),'supported_fraction':verdicts['supported']/total if total else None,'reference_covered':covered,'reference_count':visible,'reference_coverage':covered/visible if visible else None,'failed_windows':failed,'correct_corrections':correct_corrections,'reference_corrections':corrections}
    for task in ('consolidate','reflect'):
        rs=[r for r in results if r['task']==task and 'verdict' in r];summary[task]={'n':len(rs),'correct':sum(bool(r['verdict'].get('correct')) for r in rs)}
    (folder/(args.label+'-semantic.json')).write_text(json.dumps(summary,indent=2));print(json.dumps(summary,indent=2))

if __name__=='__main__':main()
