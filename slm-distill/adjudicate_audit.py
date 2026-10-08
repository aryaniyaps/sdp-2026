"""Review flagged labels against explicit runtime semantics before excluding them."""
import json
from concurrent.futures import ThreadPoolExecutor
from research_corpus import ROOT
from pi_teacher import PiTeacher
import prompt

def main():
    report=json.loads((ROOT/'quality-audit.json').read_text());teacher=PiTeacher(ROOT/'audit-cache',ROOT/'audit-usage.jsonl')
    def one(result):
        if result['acceptable']:return result
        tid=result['timeline'];_,batch,case_index=tid.split('-');data=json.loads((ROOT/'expanded'/f'batch-{batch}.json').read_text());case=data['raw_cases'][int(case_index)]
        text='''Adjudicate the flagged issues against the exact memory-worker contract below. Some flags may misunderstand default timestamps: valid_from=null intentionally uses the source event timestamp and is NOT a missing-date error. Subjects and aliases can be resolved using the event context; confidence must still reflect whether evidence is user-stated, tool-confirmed, or merely assistant-proposed. Reflection receives NEW accepted claims and their evidence only, not the entire original conversation. Reject substantive factual or citation errors, but not authorized defaults or stylistic preferences. Return {"acceptable":true|false,"issues":["remaining substantive issue"]}.\nCONTRACT:\n'''+prompt.EXTRACT_TEMPLATE+'\nFIRST AUDIT:\n'+json.dumps(result)+'\nEXAMPLE:\n'+json.dumps(case,ensure_ascii=False)
        revised=teacher.complete_json('You review a data-quality decision against the authoritative task contract.',text,tag='audit-adjudication:'+tid)
        if not isinstance(revised.get('acceptable'),bool):raise ValueError('invalid adjudication')
        return {'timeline':tid,**revised,'first_audit':result}
    with ThreadPoolExecutor(10) as pool:results=list(pool.map(one,report['results']))
    report['pre_adjudication_accepted']=report['accepted'];report['results']=results;report['accepted']=sum(r['acceptable'] for r in results);report['rejected_timelines']=[r['timeline'] for r in results if not r['acceptable']];report['adjudication']='flags reviewed against full worker contract, including timestamp defaults'
    (ROOT/'quality-audit-before-adjudication.json').write_text((ROOT/'quality-audit.json').read_text());(ROOT/'quality-audit.json').write_text(json.dumps(report,indent=2));print(report['accepted'],report['sampled_cases'],flush=True)

if __name__=='__main__':main()
