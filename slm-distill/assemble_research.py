"""Freeze the expanded corpus, audit split isolation, and prepare incremental training."""
import json,hashlib,random,collections
from pathlib import Path
from research_corpus import ROOT,to_wire


def digest(row):return hashlib.sha256(row['prompt'].encode()).hexdigest()

def main():
    initial={s:[json.loads(x) for x in (ROOT/'canonical'/f'{s}.jsonl').read_text().splitlines()] for s in ('train','val','test')}
    expanded={s:[] for s in initial};errors=[];batches=[]
    batch_data=[json.loads(p.read_text()) for p in sorted((ROOT/'expanded').glob('batch-*.json'))]
    assert len(batch_data)==176, f"wait for all expansion batches: {len(batch_data)}/176"
    # Stratify by stack while keeping every project family wholly in one partition.
    # This assignment is frozen before second-stage training or fresh-set evaluation.
    groups=collections.defaultdict(list)
    for data in batch_data:groups[data['coverage']['stack']].append(data)
    assignment={}
    for stack, group in groups.items():
        group.sort(key=lambda b: hashlib.sha256(f"split-v2:{b['index']}".encode()).hexdigest())
        for position,data in enumerate(group):
            split='test' if position<2 else ('val' if position<4 else 'train')
            assignment[str(data['index'])]=split
            batches.append(data['index']);expanded[split]+=data['rows']
            errors += [{'batch':data['index'],**e} for e in data['errors']]
    rows={s:initial[s]+expanded[s] for s in initial}
    audit_path=ROOT/'quality-audit.json'
    audit=json.loads(audit_path.read_text()) if audit_path.exists() else {}
    rejected=set(audit.get('rejected_timelines',[]))
    rows={split:[r for r in rs if r['timeline'] not in rejected] for split,rs in rows.items()}
    # Test partition wins if exact prompt overlap is found, then validation.
    seen=set();duplicates=[]
    for split in ('test','val','train'):
        unique=[]
        for r in rows[split]:
            key=digest(r)
            if key in seen:duplicates.append({'split':split,'timeline':r['timeline'],'sha256':key});continue
            seen.add(key);unique.append(r)
        rows[split]=unique
    for a,b in [('train','val'),('train','test'),('val','test')]:
        families=lambda rs:{r['timeline'].rsplit('-',1)[0] if r['timeline'].startswith(('expanded-','research-synthetic-')) else r['timeline'] for r in rs}
        assert not families(rows[a])&families(rows[b]),f'project family overlap {a}/{b}'
        repos=lambda rs:{r['meta']['source']['repo'] for r in rs if isinstance(r.get('meta',{}).get('source'),dict)}
        assert not repos(rows[a])&repos(rows[b]),f'public repository overlap {a}/{b}'
    folder=ROOT/'expanded-canonical';folder.mkdir(exist_ok=True);wire=ROOT/'expanded-sets';wire.mkdir(exist_ok=True)
    report={'quality_audit':audit,'batches':batches,'expanded_split_assignment':assignment,'target_validation_errors':errors,'duplicates_removed':duplicates,'counts':{},'coverage':{},'transformation_exclusions':[],'leakage_checks':'exact prompt, project family, public repository: passed'}
    for split,rs in rows.items():
        kept=[];encoded=[]
        for r in rs:
            try:encoded.append(to_wire(r));kept.append(r)
            except Exception as e:report['transformation_exclusions'].append({'split':split,'timeline':r['timeline'],'error':str(e)})
        rows[split]=kept
        for dest,records in [(folder,kept),(wire,encoded)]:
            text=''.join(json.dumps(r,ensure_ascii=False)+'\n' for r in records);(dest/f'{split}.jsonl').write_text(text)
        report['counts'][split]={'examples':len(kept),'tasks':dict(collections.Counter(r['task'] for r in kept)),'episodes':len({r['timeline'] for r in kept}),'sha256':hashlib.sha256((wire/f'{split}.jsonl').read_bytes()).hexdigest()}
        report['coverage'][split]={field:dict(collections.Counter(r['coverage'][field] for r in kept if 'coverage' in r)) for field in ('stack','domain','behavior')}
    total=sum(x['examples'] for x in report['counts'].values())
    assert total>=2000,f'only {total} examples; wait for corpus expansion'
    stage2=ROOT/'stage2-sets';stage2.mkdir(exist_ok=True)
    new=[r for r in rows['train'] if r['timeline'].startswith('expanded-')]
    replay=[r for r in rows['train'] if not r['timeline'].startswith('expanded-')];random.Random(29).shuffle(replay)
    train=new+replay[:128];random.Random(29).shuffle(train)
    for split,rs in [('train',train),('val',rows['val'])]:
        (stage2/f'{split}.jsonl').write_text(''.join(json.dumps(to_wire(r),ensure_ascii=False)+'\n' for r in rs))
    report['stage2']={'new_training_examples':len(new),'replay_examples':min(128,len(replay))}
    # Fixed, stratified evaluation sample; no model output informs selection.
    rng=random.Random(713);fresh=[]
    for task,count in [('extract',36),('consolidate',12),('reflect',12)]:
        pool=[r for r in rows['test'] if r['task']==task and r['timeline'].startswith(('expanded-','public-'))];rng.shuffle(pool)
        # Prefer independent episodes within each task.
        fresh+=pool[:count]
    (ROOT/'fresh-evaluation-60.jsonl').write_text(''.join(json.dumps(r,ensure_ascii=False)+'\n' for r in fresh))
    report['fresh_evaluation']={'n':len(fresh),'sha256':hashlib.sha256((ROOT/'fresh-evaluation-60.jsonl').read_bytes()).hexdigest()}
    (ROOT/'expanded-manifest.json').write_text(json.dumps(report,indent=2));print(json.dumps({'total':total,'counts':report['counts'],'stage2':report['stage2']},indent=2))

if __name__=='__main__':main()
