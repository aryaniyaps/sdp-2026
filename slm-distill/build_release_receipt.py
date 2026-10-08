"""Create a public aggregate receipt from completed local artifacts; never publish raw cases."""
import argparse, hashlib, json
from pathlib import Path
from collections import Counter
from datetime import datetime, timezone

HERE=Path(__file__).resolve().parent

def read(path):
    if not path.is_file(): raise FileNotFoundError(f'Required final artifact missing: {path}')
    return json.loads(path.read_text())

def sha(path):
    h=hashlib.sha256()
    with path.open('rb') as f:
        for part in iter(lambda:f.read(8*1024*1024),b''):h.update(part)
    return h.hexdigest()

def stage(path):
    c=read(path/'run-config.json'); args=dict(c['arguments'])
    for key in ('sets','adapter','out'):
        if args.get(key):
            value=Path(args[key])
            try: args[key]=str(value.resolve().relative_to(HERE))
            except ValueError: args[key]=value.name
    return {'run':path.name,'arguments':args,'counts':c.get('counts',{}),
            'tokens':c.get('tokens',{}),'excluded_counts':c.get('excluded_counts') or {k:len(v) for k,v in c.get('exclusions',{}).items()},
            'dataset_sha256':c.get('dataset_sha256',{}),
            'system_sha256':c.get('system_sha256'),'extraction_template_sha256':c.get('extraction_template_sha256'),
            'run_config_sha256':sha(path/'run-config.json'),
            'training_result':read(path/'training-result.json') if (path/'training-result.json').exists() else None}

def checks_only(value):
    # Preserve check outcomes and numeric aggregates, never arbitrary narrative/content.
    if isinstance(value,(bool,int,float)) or value is None:return value
    if isinstance(value,dict):return {k:checks_only(v) for k,v in value.items() if isinstance(v,(dict,bool,int,float)) or v is None}
    return None

REQUIRED_CHECKS={
    'durable_assertions','current_chennai','correction_history','old_city_not_active',
    'sister_relationship','no_unsupported_geographic_addition',
    'derived_observation_with_two_supports','neo4j_projection',
    'answer_does_not_assert_old_city_current',
    *(name+suffix for name in ('location','preference') for suffix in
      ('_recall_source_evidence','_cited_answer','_citations_have_source_evidence'))}
EVALUATIONS={'base-general-64':64,'student-general-64':64,
             'base-external-locomo-16':16,'student-external-locomo-16':16}

def validate_acceptance(receipt,model):
    checks=receipt.get('checks',{})
    missing=REQUIRED_CHECKS-set(checks)
    if missing:raise ValueError(f'Missing required live acceptance checks: {sorted(missing)}')
    if receipt.get('passed') is not True or any(v is not True for v in checks.values()):
        raise ValueError('Final live acceptance did not pass every check')
    if receipt.get('worker_model')!=model:raise ValueError('Live acceptance used a different worker model')
    if receipt.get('error'):raise ValueError('Final live acceptance contains an error')

def load_evaluations(root):
    evaluation={}
    for label,count in EVALUATIONS.items():
        summary=read(root/'eval'/f'{label}-summary.json')
        semantic=read(root/'eval'/f'{label}-semantic.json')
        if set(summary)!={'extract','consolidate','reflect'}:raise ValueError(f'{label}: incomplete task summary')
        for task,value in summary.items():
            if any(type(value.get(k)) is not int for k in ('n','accepted','first_try')):
                raise ValueError(f'{label}/{task}: missing integer counts')
            if not 0<=value['first_try']<=value['accepted']<=value['n']:
                raise ValueError(f'{label}/{task}: inconsistent counts')
        if sum(v['n'] for v in summary.values())!=count:raise ValueError(f'{label}: expected {count} evaluated cases')
        if semantic.get('records')!=count or semantic.get('grading_errors')!=0:
            raise ValueError(f'{label}: semantic grading incomplete or has errors')
        evaluation[label+'-summary']=summary
        evaluation[label+'-semantic']=semantic
    return evaluation

def main():
    p=argparse.ArgumentParser()
    p.add_argument('--root',type=Path,default=HERE/'data/research-v3')
    p.add_argument('--run',type=Path,default=HERE/'out/local-4b-general')
    p.add_argument('--gguf',type=Path)
    p.add_argument('--model',default='mem-extractor:general-v1')
    a=p.parse_args();root=a.root;gguf=a.gguf or a.run/'mem-extractor-q8_0.gguf'
    adapter=a.run/'final/adapter_model.safetensors'
    for file in [adapter,gguf]:
        if not file.is_file() or file.stat().st_size<1024*1024:raise ValueError(f'Final model artifact absent/incomplete: {file}')
    completion=read(a.run/'training-result.json')
    if completion.get('global_step',0)<=0 or not completion.get('metrics',{}).get('train_runtime'):
        raise ValueError('Final training completion metrics are missing')
    manifest=read(root/'manifest.json');acceptance=read(root/'final-acceptance.json')
    validate_acceptance(acceptance,a.model)
    if manifest.get('pending_batches'):raise ValueError('Final corpus still contains pending batches')
    evaluation=load_evaluations(root)
    lineage=[HERE/'out/local-4b-v2',HERE/'out/local-4b-general-stage-a',a.run]
    accepted=manifest.get('accepted_episodes',[])
    corpus={k:manifest[k] for k in ('status','method','generated_batches','counts','coding_replay_rows','leakage_checks','current_prompt_hashes','generation_contract_hashes','label_audit_counts','raw_generated_episodes','heldout','continuation') if k in manifest}
    corpus.update({'accepted_episode_count':len(accepted),'label_selection_counts':dict(Counter(x.get('label_selection','unspecified') for x in accepted)),
                   'excluded_count':len(manifest.get('excluded',[])),'duplicate_count':len(manifest.get('duplicates',[])) if isinstance(manifest.get('duplicates',[]),list) else manifest['duplicates'],
                   'manifest_sha256':sha(root/'manifest.json'),
                   'inherited_public_data':{'source':'nebius/SWE-rebench-openhands-trajectories','license':'CC-BY-4.0','relabeled_training_windows_before_token_filter':118,'stage':'local-4b-v2','note':'Inherited through warm-start lineage; stage-1 token filtering may exclude some rows.'}})
    receipt={'model_id':'aryaniyaps/mem-extractor','created_at':datetime.now(timezone.utc).isoformat(),
      'training':{'lineage':[stage(x) for x in lineage],'counting_note':'Stage row exposures overlap; do not sum them as unique conversations.'},
      'corpus':corpus,'evaluation':evaluation,
      'live_acceptance':{'receipt_sha256':sha(root/'final-acceptance.json'),'worker_model':acceptance['worker_model'],'passed':True,'checks':checks_only(acceptance['checks'])},
      'artifacts':{'adapter':{'bytes':adapter.stat().st_size,'sha256':sha(adapter)},'gguf':{'bytes':gguf.stat().st_size,'sha256':sha(gguf)}},
      'limitations':['English synthetic scenarios and a limited public coding slice do not establish universal domain or language generalization.',
        'Labels and semantic audits use the same teacher family; reported scores are not independent human accuracy estimates.',
        'Exact source quotes and schema validity do not guarantee entailment, correct temporal reasoning or complete recall.',
        'Training examples are capped at 4096 tokens; a larger serving context is not evidence of learned long-context performance.',
        'Multiple tasks per episode and warm-start replay are correlated; training exposure is not unique dataset quantity.']}
    destination=root/'release-receipt.json';destination.write_text(json.dumps(receipt,indent=2));print(destination)

if __name__=='__main__':main()
