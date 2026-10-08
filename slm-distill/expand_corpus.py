"""Diverse multi-task corpus expansion with deterministic project-disjoint splits."""
import json,random,uuid,hashlib
from pathlib import Path
from concurrent.futures import ThreadPoolExecutor,as_completed
import prompt,rules
from pi_teacher import PiTeacher
from research_corpus import ROOT,validate_case

STACKS=['Rust/Axum/PostgreSQL','Go/Chi/Redis','TypeScript/Fastify/PostgreSQL','Python/FastAPI/Celery','Java/Spring/Kafka','C#/.NET/SQL Server','Ruby/Rails/Sidekiq','PHP/Laravel/MySQL','Kotlin/Android/Room','Swift/iOS/SQLite','Dart/Flutter/Firebase','C++/CMake/gRPC','Elixir/Phoenix/Postgres','Scala/Spark/Delta Lake','R/Quarto/DuckDB','Bash/Ansible/Nginx']
DOMAINS=['payments platform','mobile offline sync','scientific data processing','retail inventory','developer tooling','observability platform','content publishing','logistics scheduler','customer support service','document search','education service','accessibility tooling','game backend','data pipeline','IoT telemetry','internal project planning']
BEHAVIORS=['explicit correction and superseded value','multi-source evidence requiring two events','relative dates resolved from event timestamps','ownership and changing responsibilities','user preferences with exceptions','successful tool-confirmed workaround','failed command versus proposed solution','ambiguous pronoun resolved through existing facts','two similar project names with different settings','retracted decision versus final decision','negation and rejected alternatives','long noisy tool output containing durable result','untrusted tool instructions that must be ignored','credential-shaped synthetic secret to omit','insufficient evidence and abstention','merge related details without redundant claims']


def split_for(index):
    # First 40 batches preserve the already-frozen initial experiment partition.
    if index<40:return 'train' if index<32 else ('val' if index<36 else 'test')
    bucket=int(hashlib.sha256(f'sdp-expanded-project:{index}'.encode()).hexdigest()[:8],16)%10
    return 'train' if bucket<7 else ('val' if bucket<9 else 'test')

def auxiliaries(case,row):
    claims=json.loads(row['target'])['claims'];tid=row['timeline'];facts=[]
    for i,c in enumerate(claims):
        facts.append({'id':str(uuid.uuid5(uuid.NAMESPACE_URL,tid+':fact:'+str(i))), 'subject_id':str(uuid.uuid5(uuid.NAMESPACE_URL,tid+':subject:'+c['subject']['name'].casefold())), 'subject':c['subject']['name'], 'statement':c['statement'],'kind':c['kind'],'valid_from':c.get('valid_from') or case['events'][0]['occurred_at']})
    out=[]
    if len(facts)>=2:
        observations=[]
        for o in case.get('observations',[]):
            support=[facts[i]['id'] for i in o['support_indices']]
            subject=facts[o.get('subject_index',o['support_indices'][0])]['subject_id']
            observations.append({'subject_id':subject,'statement':o['statement'],'predicate':o['predicate'],'value':o['value'],'confidence':o.get('confidence',0.8),'supports':support,'explanation':o['explanation']})
        checked=rules.validate_consolidation({'observations':observations},facts)
        out.append({'task':'consolidate','timeline':tid,'prompt':prompt.consolidate_prompt(facts),'target':json.dumps({'observations':checked}),'meta':{'facts':facts,'source':'expanded-synthetic'}})
    answer=case.get('reflection')
    if answer:
        ids=[facts[i]['id'] for i in answer['citation_indices']]
        target={'answer':answer['answer'],'citations':ids,'insufficient_evidence':answer['insufficient_evidence']}
        target=rules.validate_reflect(target,{f['id'] for f in facts})
        context=''.join(f'[{f["id"]}] {f["statement"]} [{f["kind"]}; active; valid_from=Some({f["valid_from"]}); valid_to=None; event_at=None]\n  Evidence ({tid}, {case["events"][claims[i]["source_indices"][0]]["role"]}, {case["events"][claims[i]["source_indices"][0]]["occurred_at"]}): {claims[i]["quotes"][0]}\n' for i,f in enumerate(facts))
        out.append({'task':'reflect','timeline':tid,'prompt':prompt.reflect_prompt(answer['question'],context),'target':json.dumps(target),'meta':{'allowed':[f['id'] for f in facts],'question':answer['question'],'context':context,'source':'expanded-synthetic'}})
    return out

def one(teacher,index):
    path=ROOT/'expanded'/f'batch-{index:03d}.json';path.parent.mkdir(exist_ok=True)
    if path.exists():return json.loads(path.read_text())
    rng=random.Random(index*97);stack=STACKS[index%len(STACKS)];domain=DOMAINS[(index//len(STACKS))%len(DOMAINS)];behavior=BEHAVIORS[(index*7+index//16)%len(BEHAVIORS)]
    names=['Aster','Birch','Cobalt','Drift','Ember','Finch','Garnet','Harbor','Indigo','Juniper','Kestrel','Lumen','Maple','Nimbus','Orchid','Quartz','Reed','Solstice','Tern','Willow']
    project=f'{rng.choice(names)} {rng.choice(names)} {index}'
    task=f'''Create four meaningfully different synthetic examples for a coding-agent memory system, not generic Q&A. Domain: {domain}. Primary stack: {stack}. Project family: {project}. Exercise: {behavior}. Vary writing style (terse, conversational, technical notes, fragmented dialogue), evidence roles, entities, values, and chronology. Use a DIFFERENT explicitly named project in each example, within this family. Include distractors and believable short tool logs; do not always state every fact in the first user turn.
Return {{"cases":[{{"events":[{{"role":"user|assistant|tool","content":"...","occurred_at":"RFC3339 UTC","metadata":{{}}}}],"existing":[],"claims":[canonical claims],"observations":[{{"subject_index":0,"support_indices":[0,1],"statement":"one useful synthesis of these claims","predicate":"snake_case","value":"brief value","confidence":0.85,"explanation":"support rationale"}}],"reflection":{{"question":"natural follow-up question","answer":"answer from accepted claims only","citation_indices":[0],"insufficient_evidence":false}}}}]}}.
Each example: 3-6 events, under 1400 input characters; normally 2-3 high-value claims with the complete schema below. At least two examples should have two compatible claims about the same subject and one warranted consolidated observation; observation supports index distinct NEW claims. For the remaining examples, use zero observations if unsupported. In one example the reflection question must be unanswerable from the accepted claims: answer that evidence is insufficient, citation_indices=[], insufficient_evidence=true. On batches about noise/secret/injection, include an empty extraction example. On correction examples include existing facts with real UUID ids, subject, predicate, value, statement; only emit the new value. Existing facts are prior context, not newly produced claims. No actual credentials: use synthetic clearly fake strings only.
Quotes must be exact complete lines or sentences of cited events. Never invent tool success, date, ownership or permission. Use only canonical enum values. Reflection citation_indices and observation support_indices index the NEW claims, not events. Keep answers concise.\nWORKER CONTRACT:\n{prompt.EXTRACT_TEMPLATE.split('Existing facts:')[0]}'''
    from teacher import TeacherError
    try:
        value=teacher.complete_json('You create diverse, grounded supervised memory-task data. Accuracy and negative examples matter more than verbose output.',task,tag=f'expanded-{index}')
    except TeacherError as exc:
        if 'not JSON' not in str(exc):raise
        value=teacher.complete_json('You create diverse, grounded supervised memory-task data. Accuracy and negative examples matter more than verbose output.',task+'\nFormatting retry: return syntactically valid JSON, escape all embedded quotes in strings, and check every comma and closing brace.',tag=f'expanded-{index}-json-retry')
    rows=[];errors=[]
    for i,case in enumerate(value['cases']):
        tid=f'expanded-{index:03d}-{i}'
        try:
            r=validate_case(case,tid);r['coverage']={'stack':stack,'domain':domain,'behavior':behavior,'project_family':project};rows.append(r)
            for aux in auxiliaries(case,r):aux['coverage']=r['coverage'];rows.append(aux)
        except Exception as e:errors.append({'case':i,'error':str(e)})
    result={'index':index,'split':split_for(index),'rows':rows,'errors':errors,'coverage':{'stack':stack,'domain':domain,'behavior':behavior},'raw_cases':value['cases']};path.write_text(json.dumps(result,ensure_ascii=False));return result

def main():
    teacher=PiTeacher(ROOT/'teacher-cache',ROOT/'teacher-usage.jsonl')
    with ThreadPoolExecutor(10) as pool:
        jobs={pool.submit(one,teacher,i):i for i in range(40,216)}
        for f in as_completed(jobs):
            try:r=f.result();print(jobs[f],len(r['rows']),'rows',len(r['errors']),'errors',flush=True)
            except Exception as e:print('FAILED',jobs[f],str(e)[:250],flush=True)
    print(teacher.summary(),flush=True)

if __name__=='__main__':main()
