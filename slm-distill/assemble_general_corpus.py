"""Freeze audited general-purpose corpus with family-disjoint splits and provenance."""
import argparse,collections,hashlib,json,random
from pathlib import Path
import prompt
from research_corpus import to_wire
ROOT=Path(__file__).parent/'data/research-v3'
SPLITS=('train','val','test')

def sha(data):return hashlib.sha256(data).hexdigest()
def write(path,rows):
    path.parent.mkdir(parents=True,exist_ok=True);path.write_text(''.join(json.dumps(r,ensure_ascii=False)+'\n' for r in rows));return sha(path.read_bytes())
def render(r):
    r={**r,'meta':dict(r['meta'])};m=r['meta'];r['meta']['original_prompt_sha256']=sha(r['prompt'].encode())
    if r['task']=='extract':r['prompt']=prompt.extract_prompt(m['existing'],prompt.wrap_events([prompt.compact_event(e) for e in m['raw_events']]))
    elif r['task']=='consolidate':r['prompt']=prompt.consolidate_prompt(m['facts'])
    elif r['task']=='reflect':r['prompt']=prompt.reflect_prompt(m['question'],m['context'])
    else:raise ValueError(r['task'])
    return r

def main():
    ap=argparse.ArgumentParser();ap.add_argument('--partial',action='store_true');args=ap.parse_args();prefix='interim-' if args.partial else ''
    rows={s:[] for s in SPLITS};excluded=[];accepted=[];batches=list(sorted((ROOT/'expanded').glob('batch-*.json')));pending=[];structural_errors=[]
    for p in batches:
        apath=ROOT/'audit'/p.name;rpath=ROOT/'repairs'/p.name
        if not apath.exists() or not rpath.exists():pending.append(p.name);continue
        data=json.loads(p.read_text());structural_errors.extend({'batch':data['index'],**e} for e in data['errors']);audit=json.loads(apath.read_text());repairs=json.loads(rpath.read_text());repair={c['index']:c for c in repairs['cases']}
        assert repairs['original_audit_sha256']==sha(apath.read_bytes()),f'Audit changed after repair: {p.name}'
        for verdict in audit['cases']:
            i=verdict['index'];tid=f"general-{data['index']:03d}-{i}"
            if verdict['acceptable']:chosen=[r for r in data['rows'] if r['timeline']==tid];origin='original-audit-pass'
            elif repair.get(i,{}).get('acceptable'):chosen=repair[i]['rows'];origin='repaired-and-reviewed'
            else:excluded.append({'timeline':tid,'issues':repair.get(i,verdict).get('issues',[])});continue
            for r in chosen:
                r=render(r);r['provenance']={'generation_contract_sha256':data['contract_sha256'],'generation_system_sha256':data['system_sha256'],'batch_sha256':sha(p.read_bytes()),'audit_sha256':sha(apath.read_bytes()),'repair_sha256':sha(rpath.read_bytes()),'label_selection':origin}
                try:to_wire(r)
                except Exception as exc:excluded.append({'timeline':tid,'task':r['task'],'wire_error':str(exc)});continue
                rows[data['split']].append(r)
            accepted.append({'timeline':tid,'split':data['split'],'label_selection':origin})
    if not args.partial:assert len(batches)==320 and not pending,f'Incomplete corpus: {len(batches)} batches, {len(pending)} pending audits/repairs'
    # Only accepted v2 expanded synthetic training rows: no private Pi traces or public copyrighted input.
    replay_path=ROOT.parent/'research-v2/expanded-canonical/train.jsonl';replay=[]
    if replay_path.exists():
        candidates=[json.loads(l) for l in replay_path.read_text().splitlines()];candidates=[r for r in candidates if r['timeline'].startswith('expanded-')]
        random.Random(715).shuffle(candidates)
        # Account for the general corpus's own technical domain, keeping ALL coding <=20%.
        general=len(rows['train']);coding=sum(r['coverage']['domain']=='software and technical projects' for r in rows['train']);limit=max(0,int((.20*general-coding)/.80))
        for r in candidates:
            if len(replay)>=min(128,limit):break
            try:r=render(r);to_wire(r)
            except Exception:continue
            r['coverage']={**r.get('coverage',{}),'domain':'software and technical projects','family':r['timeline'].rsplit('-',1)[0]};r['provenance']={'source':'research-v2 audited expanded canonical training replay','source_file_sha256':sha(replay_path.read_bytes())};replay.append(r)
    rows['train']+=replay
    seen=set();duplicates=[]
    for split in ('test','val','train'):
        kept=[]
        for r in rows[split]:
            digest=sha(r['prompt'].encode())
            if digest in seen:duplicates.append({'split':split,'timeline':r['timeline'],'task':r['task']});continue
            seen.add(digest);kept.append(r)
        rows[split]=kept
    families={s:{r['coverage']['family'] for r in rs} for s,rs in rows.items()}
    for a,b in [('train','val'),('train','test'),('val','test')]:assert not families[a]&families[b],f'Family leakage {a}/{b}'
    label_counts=dict(collections.Counter(e['label_selection'] for e in accepted))
    report={'label_audit_counts':label_counts,'raw_generated_episodes':sum(len(json.loads(p.read_text())['raw_cases']) for p in batches),'status':'audited interim subset' if args.partial else 'frozen','method':'synthetic labels, same-family semantic audit, rejected labels repaired against immutable evidence and re-audited; not human validation','generated_batches':len(batches),'pending_batches':pending,'generation_contract_hashes':sorted({r['provenance']['generation_contract_sha256'] for rs in rows.values() for r in rs if 'generation_contract_sha256' in r['provenance']}),'current_prompt_hashes':{k:sha(getattr(prompt,k).encode()) for k in ('SYSTEM','EXTRACT_TEMPLATE','CONSOLIDATE_TEMPLATE','REFLECT_TEMPLATE')},'accepted_episodes':accepted,'excluded':excluded,'structural_generation_errors':structural_errors,'duplicates':duplicates,'coding_replay_rows':len(replay),'counts':{},'leakage_checks':'family, exact prompt disjoint; all task variants of one episode retain pre-generation family split'}
    for s,rs in rows.items():
        random.Random(719).shuffle(rs);canonical_hash=write(ROOT/(prefix+'canonical')/(s+'.jsonl'),rs);wire_hash=write(ROOT/(prefix+'sets')/(s+'.jsonl'),[to_wire(r) for r in rs])
        coding=sum(r['coverage']['domain']=='software and technical projects' for r in rs)
        report['counts'][s]={'rows':len(rs),'episodes':len({r['timeline'] for r in rs}),'families':len(families[s]),'tasks':dict(collections.Counter(r['task'] for r in rs)),'empty_extractions':sum(r['task']=='extract' and not json.loads(r['target'])['claims'] for r in rs),'insufficient_reflections':sum(r['task']=='reflect' and json.loads(r['target'])['insufficient_evidence'] for r in rs),'domains':dict(collections.Counter(r['coverage']['domain'] for r in rs)),'behaviors':dict(collections.Counter(r['coverage'].get('behavior','replay') for r in rs)),'coding_fraction':coding/len(rs) if rs else 0,'canonical_sha256':canonical_hash,'wire_sha256':wire_hash}
    if not args.partial:
        assert sum(len(rs) for rs in rows.values())>=2000,'Fewer than 2000 audited examples; do not silently weaken target'
        fresh=[];used_episodes=set()
        for domain in sorted({r['coverage']['domain'] for r in rows['test']}):
            for task,n in [('consolidate',1),('reflect',1),('extract',2)]:
                pool=[r for r in rows['test'] if r['coverage']['domain']==domain and r['task']==task and r['timeline'] not in used_episodes];pool.sort(key=lambda r:sha(('heldout:'+r['timeline']+r['task']).encode()));assert len(pool)>=n,(domain,task,len(pool));fresh+=pool[:n];used_episodes.update(r['timeline'] for r in pool[:n])
        assert len(fresh)==len(used_episodes)==64
        report['heldout']={'rows':64,'unique_episodes':len(used_episodes),'sha256':write(ROOT/'fresh-evaluation-64.jsonl',fresh),'selection':'2 extraction + 1 consolidation + 1 reflection per 16 domains, distinct episodes; deterministic before student outputs'}
        trained_receipt=ROOT/'interim-trained-rows.json'
        if trained_receipt.exists():
            trained=json.loads(trained_receipt.read_text());previous={(r['timeline'],r['task']) for r in trained['rows']};remaining=[r for r in rows['train'] if (r['timeline'],r['task']) not in previous]
            prior=[r for r in rows['train'] if (r['timeline'],r['task']) in previous];random.Random(721).shuffle(prior);second=remaining+prior[:min(128,len(remaining)//4)]
            report['continuation']={'new_rows':len(remaining),'replay_rows':len(second)-len(remaining),'train_sha256':write(ROOT/'continuation-sets/train.jsonl',[to_wire(r) for r in second]),'val_sha256':write(ROOT/'continuation-sets/val.jsonl',[to_wire(r) for r in rows['val']])}
    (ROOT/(prefix+'manifest.json')).write_text(json.dumps(report,indent=2));print(json.dumps(report['counts'],indent=2))
if __name__=='__main__':main()
