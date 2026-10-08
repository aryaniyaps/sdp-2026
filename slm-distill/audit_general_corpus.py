"""Audit completed general-purpose batches while generation continues.

Same teacher family as generation: a quality filter, not human ground truth.
"""
import argparse,json,time
from concurrent.futures import ThreadPoolExecutor,as_completed
from pathlib import Path
import prompt
from pi_teacher import PiTeacher
ROOT=Path(__file__).parent/'data/research-v3'

def audit(teacher,path):
    data=json.loads(path.read_text());dest=ROOT/'audit'/path.name
    if dest.exists():return json.loads(dest.read_text())
    indices=sorted({int(r['timeline'].rsplit('-',1)[1]) for r in data['rows']})
    cases=[{'index':i,'case':data['raw_cases'][i]} for i in indices]
    instruction='''Audit every supplied synthetic episode and ALL its labels. Return {"cases":[{"index":0,"acceptable":true,"issues":[]}]}, exactly one verdict per supplied index.
Check claims for exact source attribution and entailment, consistent subject/predicate/value/statement, relationship direction, dates without invented precision, explicit correction flags, preference exceptions, negation and rejected suggestions. When identity and value need two events, both must be cited. Existing facts may resolve references but are not new evidence. Check observations against ONLY their support_indices NEW claims: at least two distinct supports, no invented causality. Check reflection against ONLY NEW claims and their quoted evidence, with correct citation_indices and insufficiency. Do not permit reflection to use unextracted source details.
A null valid_from is VALID: the system defaults to mention time. A null event_at is REQUIRED when only a calendar date/month or uncertain time is supported. Resolve unambiguous relative dates in the statement but do not require invented UTC clock precision. Review meaningful correctness, not style. Empty claims/observations are valid when evidence is insufficient. Reject the whole episode for a substantive error; explain precisely.\nCONTRACT:\n'''+prompt.EXTRACT_TEMPLATE.split('Existing facts: {{EXISTING}}')[0]+'\nCASES:\n'+json.dumps(cases,ensure_ascii=False)
    result=teacher.complete_json('Audit memory training labels against evidence. Do not assume generated labels are correct.',instruction,tag=f"general-audit-{data['index']}")
    verdicts=result.get('cases',[])
    if sorted(v['index'] for v in verdicts)!=indices or any(type(v.get('acceptable')) is not bool for v in verdicts):raise ValueError('Incomplete audit')
    result={'batch':data['index'],'method':'same-family teacher semantic audit against current contract','cases':verdicts}
    dest.write_text(json.dumps(result,indent=2));return result

def main():
    ap=argparse.ArgumentParser();ap.add_argument('--watch',action='store_true');ap.add_argument('--batches',type=int,default=320);args=ap.parse_args()
    (ROOT/'audit').mkdir(exist_ok=True);teacher=PiTeacher(ROOT/'audit-cache',ROOT/'audit-usage.jsonl');failed={};deadline=time.monotonic()+7200
    while True:
        files=sorted((ROOT/'expanded').glob('batch-*.json'))
        todo=[p for p in files if not (ROOT/'audit'/p.name).exists() and failed.get(p.name,0)<2]
        with ThreadPoolExecutor(6) as pool:
            jobs={pool.submit(audit,teacher,p):p for p in todo}
            for f in as_completed(jobs):
                p=jobs[f]
                try:r=f.result();print(p.name,sum(c['acceptable'] for c in r['cases']),'/',len(r['cases']),flush=True)
                except Exception as e:failed[p.name]=failed.get(p.name,0)+1;print('ERROR',p.name,str(e)[:300],flush=True)
        if not args.watch or (len(files)==args.batches and all((ROOT/'audit'/p.name).exists() for p in files)):break
        if time.monotonic()>deadline:raise TimeoutError('generation/audit incomplete; preserve completed receipts')
        time.sleep(10)
    receipts=[json.loads(p.read_text()) for p in sorted((ROOT/'audit').glob('batch-*.json'))]
    entries=[{'timeline':f"general-{r['batch']:03d}-{c['index']}",**c} for r in receipts for c in r['cases']]
    report={'method':'all structurally accepted general synthetic episodes, same teacher family; not human validation','batches':len(receipts),'reviewed':len(entries),'accepted':sum(e['acceptable'] for e in entries),'rejected_timelines':[e['timeline'] for e in entries if not e['acceptable']],'results':entries}
    (ROOT/'quality-audit.json').write_text(json.dumps(report,indent=2));print('Audit complete',report['accepted'],'/',report['reviewed'],flush=True)
if __name__=='__main__':main()
