"""CPU-only exact tokenizer accounting, without truncating examples or loading weights."""
import argparse,collections,hashlib,json
from pathlib import Path
from transformers import AutoTokenizer
import prompt

def main():
    ap=argparse.ArgumentParser();ap.add_argument('--sets',type=Path,required=True);ap.add_argument('--max-length',type=int,default=4096);args=ap.parse_args()
    tok=AutoTokenizer.from_pretrained('Qwen/Qwen3-4B-Instruct-2507',revision='cdbee75f17c01a7cc42f958dc650907174af0554',local_files_only=True)
    result={'max_length':args.max_length,'base':'Qwen/Qwen3-4B-Instruct-2507','revision':'cdbee75f17c01a7cc42f958dc650907174af0554','splits':{}}
    for split in ('train','val','test'):
        path=args.sets/(split+'.jsonl')
        if not path.exists():continue
        retained=[];excluded=[]
        for r in map(json.loads,path.read_text().splitlines()):
            tokens=len(tok(prompt.chat_prompt(tok,r['prompt']),add_special_tokens=False)['input_ids'])+len(tok(r['target']+'<|im_end|>',add_special_tokens=False)['input_ids'])
            entry={'timeline':r['timeline'],'task':r['task'],'tokens':tokens,'domain':r.get('coverage',{}).get('domain'),'prompt_sha256':hashlib.sha256(r['prompt'].encode()).hexdigest()}
            (retained if tokens<=args.max_length else excluded).append(entry)
        result['splits'][split]={'source_sha256':hashlib.sha256(path.read_bytes()).hexdigest(),'retained':len(retained),'excluded':excluded,'maximum_retained_tokens':max((r['tokens'] for r in retained),default=0),'retained_tasks':dict(collections.Counter(r['task'] for r in retained)),'retained_domains':dict(collections.Counter(r['domain'] for r in retained)),'rows':retained}
    dest=args.sets.parent/(args.sets.name+'-token-stats.json');dest.write_text(json.dumps(result,indent=2));print(json.dumps({s:{'retained':r['retained'],'excluded':len(r['excluded'])} for s,r in result['splits'].items()}))
if __name__=='__main__':main()
