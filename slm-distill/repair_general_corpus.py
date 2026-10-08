"""Repair rejected labels without changing source evidence, then independently re-audit.

Original generation and audit receipts remain immutable. Same-family review is not
human validation. No student responses or scores are available to this pipeline.
"""
import argparse,json,time,hashlib
from concurrent.futures import ThreadPoolExecutor,as_completed
from pathlib import Path
import prompt
from pi_teacher import PiTeacher
from research_corpus import validate_case
from expand_corpus import auxiliaries
ROOT=Path(__file__).parent/'data/research-v3'

def repair(teacher,path):
    dest=ROOT/'repairs'/path.name
    if dest.exists():
        previous=json.loads(dest.read_text())
        if previous['original_audit_sha256']==hashlib.sha256(path.read_bytes()).hexdigest():return previous
        superseded=ROOT/'repairs-superseded';superseded.mkdir(exist_ok=True)
        dest.rename(superseded/(dest.stem+'-'+hashlib.sha256(dest.read_bytes()).hexdigest()[:12]+'.json'))
    audit=json.loads(path.read_text());data=json.loads((ROOT/'expanded'/path.name).read_text())
    rejected=[v for v in audit['cases'] if not v['acceptable']]
    result={'batch':data['index'],'original_audit_sha256':hashlib.sha256(path.read_bytes()).hexdigest(),'audit_snapshot':audit,'cases':[]}
    if not rejected:dest.write_text(json.dumps(result));return result
    supplied=[{'index':v['index'],'issues':v['issues'],'case':data['raw_cases'][v['index']]} for v in rejected]
    instruction='''Correct the LABELS of each supplied memory episode using the audit feedback. Return {"cases":[{"index":0,"claims":[],"observations":[],"reflection":{}}]}. Keep the source events and existing facts exactly unchanged; they are immutable evidence. Retain every useful relationship/preference/event that the contract requires, with exact source quotations and all context-bearing source indices. Correct observations to use only their cited NEW claims and reflection only NEW claims plus their quoted evidence. Observation support_indices and reflection citation_indices index the revised claims. Do not invent missing evidence to satisfy the old label; delete unsupported assertions. Preserve the reflection question, answer insufficiently if required. Null valid_from is valid. Event_at must be null unless an exact timestamp is supported. Return only revised labels, not modified events. CONTRACT:\n'''+prompt.EXTRACT_TEMPLATE.split('Existing facts: {{EXISTING}}')[0]+'\nEPISODES:\n'+json.dumps(supplied,ensure_ascii=False)
    labels=teacher.complete_json('Repair synthetic memory labels carefully against immutable evidence.',instruction,tag=f"general-repair-{data['index']}")['cases']
    if sorted(x['index'] for x in labels)!=sorted(x['index'] for x in rejected):raise ValueError('Incomplete repair')
    candidates=[]
    for label in labels:
        i=label['index'];original=data['raw_cases'][i];case={**original,**{k:label[k] for k in ('claims','observations','reflection')}}
        # The question is immutable input, not a label the repair model may edit.
        case['reflection']={**case['reflection'],'question':original['reflection']['question']}
        try:
            row=validate_case(case,f"general-{data['index']:03d}-{i}");rows=[row,*auxiliaries(case,row)]
            for r in rows:r['coverage']=data['coverage'];r['meta']['source']='general-synthetic-v3-repaired'
            candidates.append({'index':i,'case':case,'rows':rows})
        except Exception as exc:result['cases'].append({'index':i,'acceptable':False,'issues':['Structural repair failure: '+str(exc)]})
    if candidates:
        instruction='''Independently audit ALL labels in each supplied episode. Return {"cases":[{"index":0,"acceptable":true,"issues":[]}]}, exactly one verdict each. Check source entailment, exact quotation, source attribution, cross-event identity citation, relationship direction, qualified preferences, dates, corrections, omissions of important explicit relationships. Observations use ONLY distinct supporting NEW claims, no additional event details. Reflection uses ONLY NEW claims and their quoted evidence, with correct citations/insufficiency. Null valid_from is valid. Calendar-only dates need null event_at. Reject substantive errors, not stylistic preferences. CONTRACT:\n'''+prompt.EXTRACT_TEMPLATE.split('Existing facts: {{EXISTING}}')[0]+'\nCASES:\n'+json.dumps([{'index':c['index'],'case':c['case']} for c in candidates],ensure_ascii=False)
        verdicts=teacher.complete_json('Audit memory labels against evidence. Do not assume labels are correct.',instruction,tag=f"general-repair-review-{data['index']}")['cases']
        if sorted(v['index'] for v in verdicts)!=sorted(c['index'] for c in candidates):raise ValueError('Incomplete repair audit')
        byindex={v['index']:v for v in verdicts}
        for c in candidates:
            verdict=byindex[c['index']]
            if type(verdict.get('acceptable')) is not bool:raise ValueError('Invalid verdict')
            result['cases'].append({**c,**verdict})
    dest.write_text(json.dumps(result,ensure_ascii=False,indent=2));return result

def main():
    ap=argparse.ArgumentParser();ap.add_argument('--watch',action='store_true');ap.add_argument('--workers',type=int,default=10);args=ap.parse_args()
    (ROOT/'repairs').mkdir(exist_ok=True);teacher=PiTeacher(ROOT/'repair-cache',ROOT/'repair-usage.jsonl');failed={};deadline=time.monotonic()+7200
    while True:
        files=sorted((ROOT/'audit').glob('batch-*.json'));todo=[p for p in files if not (ROOT/'repairs'/p.name).exists() and failed.get(p.name,0)<2]
        with ThreadPoolExecutor(args.workers) as pool:
            jobs={pool.submit(repair,teacher,p):p for p in todo}
            for f in as_completed(jobs):
                p=jobs[f]
                try:r=f.result();print(p.name,sum(c['acceptable'] for c in r['cases']),'repaired',flush=True)
                except Exception as exc:failed[p.name]=failed.get(p.name,0)+1;print('ERROR',p.name,str(exc),flush=True)
        if not args.watch or (len(files)==320 and all((ROOT/'repairs'/p.name).exists() for p in files)):break
        if time.monotonic()>deadline:raise TimeoutError('Incomplete audit/repair')
        time.sleep(10)
if __name__=='__main__':main()
