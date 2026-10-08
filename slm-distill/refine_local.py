"""Wait for complete training timelines and validation labels, then refine locally.

Run from the repository root alongside run_pi_corpus.py. Test timelines are
excluded. A small deterministic bootstrap replay protects the learned schema.
"""
import json,time,shutil,subprocess,os,random
from pathlib import Path
root=Path('slm-distill/data/pi-corpus'); train=[f'tl{i:04d}' for i in range(16)]; stages=['labeled','turns','aux']
while True:
 valid=[f'tl{i:04d}' for i in range(260,264) if all((root/s/f'tl{i:04d}.json').is_file() for s in stages)]
 if valid and all((root/s/(tid+'.json')).is_file() for tid in train for s in stages):break
 time.sleep(20)
selected=train+valid; snapshot=Path('slm-distill/data/refinement-corpus'); sets=Path('slm-distill/data/refinement-sets')
for stage in stages:
 (snapshot/stage).mkdir(parents=True,exist_ok=True)
 for tid in selected:shutil.copyfile(root/stage/(tid+'.json'),snapshot/stage/(tid+'.json'))
env=dict(os.environ);env.pop('PYTHONPATH',None);env['HF_HUB_OFFLINE']='1';env['PYTORCH_ALLOC_CONF']='expandable_segments:True'
python='slm-distill/.venv/bin/python'
subprocess.run([python,'slm-distill/build_dataset.py','--labeled',str(snapshot/'labeled'),'--turns',str(snapshot/'turns'),'--aux',str(snapshot/'aux'),'--out',str(sets)],check=True,env=env)
natural=(sets/'train.jsonl').read_text().splitlines(); replay=random.Random(13).sample(Path('slm-distill/data/bootstrap-sets/train.jsonl').read_text().splitlines(),126)
rows=natural+replay;random.Random(13).shuffle(rows);(sets/'train.jsonl').write_text('\n'.join(rows)+'\n')
manifest={'train_timelines':train,'validation_timelines':valid,'natural_train_examples':len(natural),'bootstrap_replay_examples':len(replay),'test_examples_used':0,'seed':13,'epochs':4,'learning_rate':0.0001}
(sets/'manifest.json').write_text(json.dumps(manifest,indent=2));print(manifest,flush=True)
subprocess.run([python,'slm-distill/train.py','--sets',str(sets),'--adapter','slm-distill/out/local-bootstrap/final','--out','slm-distill/out/local-refined','--epochs','4','--lr','0.0001'],check=True,env=env)
