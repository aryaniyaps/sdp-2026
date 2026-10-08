"""Resumable matched-protocol evaluation on canonical rows, with raw evidence receipts."""
import argparse,json,hashlib,time,collections
from pathlib import Path
import ollama_client,rules,structured

def check(row,value):
    if row['task']=='extract':return {'claims':rules.validate_extraction(value,row['meta']['raw_events'],row['meta']['existing'])}
    if row['task']=='consolidate':
        facts,_=structured.first_json(row['prompt'].split(' Facts: ',1)[1]);return {'observations':rules.validate_consolidation(value,facts)}
    return rules.validate_reflect(value,set(row['meta']['allowed']))

def run(row,model):
    original=row['prompt'];user=original;attempts=[]
    for i in range(3):
        result={};error=''
        try:
            result=ollama_client.generate('http://127.0.0.1:11434',model,user)
            if result.get('done_reason')=='length':raise ValueError('truncated output')
            if result.get('prompt_eval_count',999999)+16>=12288:raise ValueError('truncated prompt')
            value=check(row,json.loads(result['response']))
            attempts.append({'payload':result});return {'accepted':True,'value':value,'attempts':attempts}
        except Exception as e:error=str(e)
        attempts.append({'payload':result,'error':error})
        user=original+'\nPrevious response failed validation: '+error+'\nPrevious response: '+result.get('response','')+'\nReturn the complete corrected JSON.'
    return {'accepted':False,'attempts':attempts,'error':error}

def main():
    ap=argparse.ArgumentParser();ap.add_argument('--model',required=True);ap.add_argument('--label',required=True);ap.add_argument('--rows',type=Path,required=True);ap.add_argument('--output-folder',type=Path);args=ap.parse_args()
    root=args.output_folder or Path(__file__).parent/'data/research-v2/eval';root.mkdir(parents=True,exist_ok=True)
    out=root/(args.label+'.jsonl');seen={}
    if out.exists():
        for line in out.read_text().splitlines():
            v=json.loads(line)
            if v['model'] != args.model: raise ValueError('Cannot resume receipt with a different model')
            seen[v['key']]=v
    rows=[json.loads(l) for l in args.rows.read_text().splitlines()]
    with out.open('a') as f:
        for i,row in enumerate(rows):
            key=hashlib.sha256(row['prompt'].encode()).hexdigest()
            if key not in seen:
                result=run(row,args.model);record={'key':key,'model':args.model,'task':row['task'],'timeline':row['timeline'],'row':row,**result};f.write(json.dumps(record,ensure_ascii=False)+'\n');f.flush();seen[key]=record
            r=seen[key];print(i+1,len(rows),row['task'],r['accepted'],len(r['attempts']),flush=True)
    counts={}
    for task in ['extract','consolidate','reflect']:
        rs=[r for r in seen.values() if r['task']==task];counts[task]={'n':len(rs),'accepted':sum(r['accepted'] for r in rs),'first_try':sum(r['accepted'] and len(r['attempts'])==1 for r in rs)}
    (root/(args.label+'-summary.json')).write_text(json.dumps(counts,indent=2));print(counts,flush=True)

if __name__=='__main__':main()
