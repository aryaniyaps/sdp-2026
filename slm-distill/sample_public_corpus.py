"""Sample public SWE trajectories with repository-level split and source receipts."""
import json,hashlib,random
from pathlib import Path
import requests,prompt
ROOT=Path(__file__).parent/'data'/'research-v2';ROOT.mkdir(exist_ok=True)
source=Path('/tmp/sdp-public-trajectories.json')
if not source.exists():
 r=requests.get('https://datasets-server.huggingface.co/rows',params={'dataset':'nebius/SWE-rebench-openhands-trajectories','config':'default','split':'train','offset':0,'length':100},timeout=120);r.raise_for_status();source.write_text(r.text)
data=json.loads(source.read_text());by_repo={}
for entry in data['rows']:
 if entry.get('truncated_cells'):continue
 row=entry['row'];by_repo.setdefault(row['repo'],row)
repos=sorted(by_repo);random.Random(37).shuffle(repos)
output=[]
for i,repo in enumerate(repos[:52]):
 split='train' if i<40 else ('val' if i<46 else 'test');row=by_repo[repo]
 events=[]
 for message in row['trajectory']:
  role=message['role']
  if role not in ('user','assistant','tool'):continue
  text=message.get('content') or ''
  if message.get('tool_calls'):text+='\nTool calls: '+json.dumps(message['tool_calls'])
  if not text.strip():continue
  events.append(prompt.compact_event({'role':role,'content':text,'occurred_at':'2026-01-01T00:00:00Z','metadata':{'repository':repo,'source_timestamp':'unavailable; fixed import timestamp'}}))
 if len(events)<3:continue
 starts=list(dict.fromkeys([0,max(0,len(events)//2-2),max(0,len(events)-5)]))[:3 if split=='train' else 2]
 for j,start in enumerate(starts):
  window=events[start:start+5]
  # Keep the project identity in metadata; do not invent successful execution evidence.
  tid='public-'+hashlib.sha256((row['trajectory_id']+':'+str(start)).encode()).hexdigest()[:16]
  output.append({'timeline':tid,'split':split,'events':window,'provenance':{'dataset':'nebius/SWE-rebench-openhands-trajectories','license':'CC-BY-4.0','authors':'Nebius; Trofimova et al. 2025','repo':repo,'trajectory_id':row['trajectory_id'],'instance_id':row['instance_id'],'source_rows_sha256':hashlib.sha256(source.read_bytes()).hexdigest(),'range':[start,start+len(window)]}})
(ROOT/'public-source.json').write_bytes(source.read_bytes())
(ROOT/'public-windows.jsonl').write_text(''.join(json.dumps(r,ensure_ascii=False)+'\n' for r in output))
from collections import Counter
print(Counter(r['split'] for r in output),len(repos),'repositories')
