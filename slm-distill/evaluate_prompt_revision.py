"""Small human-review development probes; no accuracy or generalization claim."""
import argparse,json,hashlib,subprocess
from pathlib import Path
import prompt
from evaluate_research import run
ROOT=Path(__file__).resolve().parent

def main():
    ap=argparse.ArgumentParser();ap.add_argument('--model',default='qwen3-4b-instruct-baseline');ap.add_argument('--label',default='general-hindsight-v2');args=ap.parse_args()
    old=subprocess.check_output(['git','show','HEAD:src/worker/extract_prompt.txt'],cwd=ROOT,text=True).rstrip('\n')
    oldsys=subprocess.check_output(['git','show','HEAD:src/worker/worker_system.txt'],cwd=ROOT,text=True).strip()
    current=prompt.EXTRACT_TEMPLATE;currentsys=prompt.SYSTEM
    out=ROOT/'data/research-v3/prompt-probes';out.mkdir(parents=True,exist_ok=True)
    cases=json.loads((ROOT/'fixtures/general-memory-prompt-cases.json').read_text())
    for label,template,system in [('coding-original',old,oldsys),(args.label,current,currentsys)]:
        prompt.SYSTEM=system
        records=[]
        for case in cases:
            path=out/f"{label}-{case['id']}.json"
            if path.exists():records.append(json.loads(path.read_text()));continue
            events=[{**e,'occurred_at':'2026-10-08T09:00:00Z','metadata':{}} for e in case['events']]
            user=template.replace('{{EXISTING}}','[]').replace('{{EVENTS}}',prompt.dumps(prompt.wrap_events(events)))
            row={'task':'extract','timeline':case['id'],'prompt':user,'meta':{'raw_events':events,'existing':[]}}
            result={'case':case['id'],'model':args.model,'variant':label,'review_criterion':case['review'],'prompt_sha256':hashlib.sha256(user.encode()).hexdigest(),'system_sha256':hashlib.sha256(system.encode()).hexdigest(),**run(row,args.model)}
            path.write_text(json.dumps(result,indent=2));records.append(result);print(label,case['id'],result['accepted'],flush=True)
        (out/f'{label}-summary.json').write_text(json.dumps({'n':len(records),'accepted':sum(r['accepted'] for r in records),'semantic_review':'manual criteria in each receipt; hard acceptance is not semantic accuracy'},indent=2))
    prompt.SYSTEM=currentsys
if __name__=='__main__':main()
