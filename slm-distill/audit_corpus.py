"""Second-pass semantic audit of a stratified training-label sample (same teacher)."""
import json,hashlib,argparse
from pathlib import Path
from concurrent.futures import ThreadPoolExecutor,as_completed
from research_corpus import ROOT
from pi_teacher import PiTeacher


def main():
    ap=argparse.ArgumentParser();ap.add_argument("--all", action="store_true");args=ap.parse_args()
    manifest=json.loads((ROOT/'expanded-manifest.json').read_text());groups={}
    for p in sorted((ROOT/'expanded').glob('batch-*.json')):
        data=json.loads(p.read_text())
        if not args.all and manifest['expanded_split_assignment'][str(data['index'])]!='train':continue
        stack=data['coverage']['stack']
        for i,case in enumerate(data['raw_cases']):
            tid=f"expanded-{data['index']:03d}-{i}"
            if any(r['timeline']==tid for r in data['rows']):groups.setdefault(stack,[]).append((tid,case))
    selected=[]
    for stack,items in groups.items():
        items.sort(key=lambda x:hashlib.sha256(('audit:'+x[0]).encode()).hexdigest());selected += items if args.all else items[:2]
    teacher=PiTeacher(ROOT/'audit-cache',ROOT/'audit-usage.jsonl');folder=ROOT/'audit';folder.mkdir(exist_ok=True)
    def one(item):
        tid,case=item;p=folder/f'{tid}.json'
        if p.exists():return json.loads(p.read_text())
        question='''Audit this synthetic training example against its own source events and prior facts. Check every extracted claim for source entailment, correct speaker attribution, durable usefulness, current versus rejected values, dates and secrets. Check every proposed observation is a nontrivial supported synthesis of its support_indices (which index claims). Check the reflection answer and insufficiency flag against NEW claims only; its citation_indices index claims. Do not reject natural paraphrases or longer exact quotes. Reject substantive false, unsupported, overconfident assistant-only, wrongly dated or contradictory labels. Return {"acceptable":true|false,"issues":["precise issue"]}. This is a quality audit, not a request to improve style.\nExample:\n'''+json.dumps(case,ensure_ascii=False)
        result=teacher.complete_json('You independently scrutinize memory-worker training labels against supplied evidence. Be precise.',question,tag='label-audit:'+tid)
        if not isinstance(result.get('acceptable'),bool):raise ValueError('audit verdict must be boolean')
        out={'timeline':tid,**result};p.write_text(json.dumps(out,indent=2));return out
    out=[]
    with ThreadPoolExecutor(12) as pool:
        for r in pool.map(one,selected):out.append(r);print(r['timeline'],r['acceptable'],r['issues'],flush=True)
    report={'method':('all expanded cases' if args.all else 'stratified training sample')+'; second-pass review by same teacher, not independent human annotation','sampled_cases':len(out),'accepted':sum(r['acceptable'] for r in out),'rejected_timelines':[r['timeline'] for r in out if not r['acceptable']],'results':out}
    (ROOT/'quality-audit.json').write_text(json.dumps(report,indent=2));print('accepted',report['accepted'],'/',len(out),flush=True)

if __name__=='__main__':main()
