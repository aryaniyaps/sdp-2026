"""Build and execute the evidence report. Does not train models or mutate services."""

import argparse
from pathlib import Path
from textwrap import dedent

import nbformat as nbf
from nbclient import NotebookClient

ROOT = Path(__file__).resolve().parent


def build_notebook():
    """Collect report cells without running experiments."""
    cells = []

    def md(text):
        cells.append(nbf.v4.new_markdown_cell(dedent(text).strip()))

    def code(text):
        cells.append(nbf.v4.new_code_cell(dedent(text).strip()))

    md('''
    # Fine-tuning a small model for memory

    SDP 2026 · 8 October 2026

    The aim was to train a small model to extract facts from conversations, combine related facts, and answer questions with sources. The final model is **Qwen3-4B-Instruct-2507**, fine-tuned with QLoRA on an 8 GB RTX 4070 laptop GPU.

    This notebook follows the iterations using saved results from the training scripts. Each stage helped decide what to try next: coding conversations provided the initial test setting, then the dataset and prompts were extended to everyday conversations. The experiments were AI-assisted and were not all run inside Jupyter.

    Run All reads local results and redraws the plots. It does not train, call the teacher, or change the running service. Saved outputs can be read without the local datasets; missing files are marked unavailable.''')
    code('''
    from pathlib import Path
    from collections import Counter
    from datetime import datetime, timezone
    import json, hashlib, re, ast, sys
    import pandas as pd
    import matplotlib.pyplot as plt
    from IPython.display import display, Markdown, Image
    ROOT = next(p for p in [Path.cwd(), *Path.cwd().parents] if (p / 'Cargo.toml').exists())
    SLM = ROOT / 'slm-distill'
    DATA = SLM / 'data/research-v2'
    FIG = SLM / 'reports/figures'
    FIG.mkdir(parents=True, exist_ok=True)
    def read(path):
        p = Path(path)
        return json.loads(p.read_text()) if p.exists() else None
    def rows(path):
        p = Path(path)
        return [json.loads(line) for line in p.read_text().splitlines() if line.strip()] if p.exists() else []
    def show(value):
        display(pd.DataFrame(value)) if isinstance(value, list) else display(value)
    def digest(path): return hashlib.sha256(Path(path).read_bytes()).hexdigest()
    def missing(label): display(Markdown(f'**Not available locally:** {label}.'))
    def show_acceptance(receipt):
        identity=receipt.get('deployment_identity_before',{})
        show([{'worker_model':receipt.get('worker_model',receipt.get('health',{}).get('worker_model')),
               'passed':receipt.get('passed'),'served_gguf_sha256':identity.get('served_gguf_sha256'),
               'started_at':receipt.get('started_at'),'finished_at':receipt.get('finished_at')}])
        show([{'check':name,'passed':value} for name,value in receipt.get('checks',{}).items()])
        graph=receipt.get('steps',{}).get('correction_graph',{})
        assertions=graph.get('assertions',[])
        observations=[a for a in assertions if a.get('kind')=='observation']
        supports=[e for e in graph.get('edges',[]) if e.get('relation') in ('supports','derives')]
        projection=receipt.get('projection',{})
        show([{'assertions':len(assertions),'active_assertions':sum(a.get('status')=='active' for a in assertions),
               'observations':len(observations),'support_edges':len(supports),
               'observations_with_two_distinct_supports':sum(len({e['to_id'] for e in supports if e['from_id']==a['id']})>=2 for a in observations),
               'projected_nodes':len(projection.get('nodes',[])),'projected_relationships':len(projection.get('relationships',[]))}])
        examples=[]
        for topic in ('location','preference'):
            recalled=receipt.get('steps',{}).get(topic+'_recall',{}).get('results',[])
            hit=next((r for r in recalled if any(source.get('quote') for source in r.get('sources',[]))),None)
            if hit:
                examples.append({'query_topic':topic,'recalled_statement':hit.get('statement'),
                                 'source_quote':next(source['quote'] for source in hit['sources'] if source.get('quote'))})
        if examples: show(examples)
        show([{'query_topic':topic,'answer':receipt.get('steps',{}).get(topic+'_reflect',{}).get('answer'),
               'citation_count':len(receipt.get('steps',{}).get(topic+'_reflect',{}).get('citations',[]))}
              for topic in ('location','preference')])
        if receipt.get('recheck_provenance'): display(receipt['recheck_provenance'])
        if receipt.get('error'): print('Acceptance error:',receipt['error'])
        display(Markdown('The local receipt has the full job, graph and retrieval responses.'))

    print('Report refreshed:', datetime.now(timezone.utc).isoformat())
    print('Python:', sys.version.split()[0])
    ''')
    md('''
    ## 1. What the model needs to do

    | Task | Input | Expected output |
    |---|---|---|
    | Extraction | Conversation events and earlier facts | Facts with exact quotes, source indices, and correction details |
    | Consolidation | Accepted facts | Observations supported by at least two distinct facts |
    | Reflection | A question and retrieved facts | An answer with citations, or insufficient evidence |

    Valid JSON is only the first check. The claims also need to match their sources and cover the relevant facts. The app stores evidence in PostgreSQL and projects it into Neo4j, so the live checks also follow jobs through graph updates and retrieval.''')
    md('''
    ## 2. Iterations

    | Iteration | Observation | Next step |
    |---|---|---|
    | 1.7B bootstrap | Good synthetic scores, poor transfer to conversations | Add teacher-labeled conversations and replay |
    | 1.7B refinement | Source attribution remained unreliable | Pair each quote with its source and constrain IDs |
    | Training memory fixes | Full output logits ran out of GPU memory | Compute answer-only loss in chunks |
    | 4B base | 7,141 tokens caused OOM; 4,091 worked after a dtype fix | Use a 4,096-token training cap |
    | Coding corpus | Established a starting point for software conversations | Extend coverage to sixteen everyday and professional domains |
    | Label review | Review surfaced unsupported labels and ambiguous timestamp flags | Add contract-based adjudication and exclude rejected cases |
    | Final model | Better consolidation and reflection, lower extraction coverage | Record the tradeoff and inspect errors |

    The base model, data and decoder changed between early runs. Those runs do not isolate the effect of model size.''')
    md('''
    ## 3. First run: 1.7B bootstrap

    The first adapter used Qwen3-1.7B with 1,260 training, 140 validation and 280 synthetic test examples. The 28-case comparison below used matching prompts with thinking disabled. The hardware differed, so the latency numbers are not directly comparable.''')
    code('''
    comparison = read(SLM/'data/eval/bootstrap-comparison.json')
    if comparison:
        show([{'model': k, 'n': comparison['n'], **comparison[k]} for k in ('base','bootstrap')])
        print(comparison['scope'])
        print('Latency comparable:', comparison['latency_comparison_valid'])
    for label, path in [('bootstrap synthetic test','bootstrap-student-full'), ('refined synthetic test','refined-bootstrap-full')]:
        result = read(SLM/f'data/eval/{path}/summary.json')
        if result: print(label); display(result)
        else: missing(label)
    ''')
    md('''
    ## 4. Testing on conversations

    The refined 1.7B model accepted only 18 of 33 extraction windows with the earlier decoder. These conversations were later used for debugging, so they are development results.

    The old judge measured support and recall only on accepted windows. That misses failed windows; later coverage scores include them.''')
    code('''
    natural = read(SLM/'data/pi-corpus/eval/local-v1.json')
    if natural:
        e=natural['extract']
        show([{'task':'extract','n':e['windows'],'accepted_fraction':e['accepted'],'first_try_fraction':e['first_try']},
              {'task':'consolidate','n':natural['consolidate']['items'],'accepted_fraction':natural['consolidate']['accepted']},
              {'task':'reflect','n':natural['reflect']['items'],'accepted_fraction':natural['reflect']['accepted']}])
        display(e.get('judge'))
        print('Representative validator failures:')
        for failure in e.get('failures',[])[:3]: print('-', failure)
    else: missing('1.7B naturalistic evaluation')
    ''')
    md('''
    ## 5. Keeping quotes with their sources

    The API stores `source_indices` and `quotes` in separate arrays. During generation, `source_quotes: [{source_index, quote}]` keeps each pair together, then a codec converts it back. Consolidation IDs are limited to the supplied facts. Python and Rust use the shared schema files below.

    Short teacher quotes are widened to an allowed containing span without changing the claim or source index. An exact quote can still be misinterpreted, so semantic review is needed too.''')
    code('''
    schema_files = [ROOT/'src/model/extract_schema.json', ROOT/'src/model/consolidate_schema.json', ROOT/'src/model/paired_sources.txt']
    show([{'file':str(p.relative_to(ROOT)), 'sha256':digest(p), 'bytes':p.stat().st_size} for p in schema_files])
    ''')
    md('''
    ## 6. Model and training setup

    Base: [Qwen3-4B-Instruct-2507](https://huggingface.co/Qwen/Qwen3-4B-Instruct-2507), revision `cdbee75f17c01a7cc42f958dc650907174af0554`, Apache-2.0. It fit the non-thinking instruction and Qwen3 export setup. Qwen3.5-4B was considered, but would have needed a different export path.

    Training uses Transformers, PEFT and bitsandbytes: NF4 with double quantization, bfloat16 compute, LoRA rank 16, alpha 32, dropout 0.05, gradient checkpointing and answer-only loss. The targets are teacher-generated sequences, not teacher logits. See the [PEFT quantization guide](https://huggingface.co/docs/peft/developer_guides/quantization).

    On the 8 GB RTX 4070 laptop GPU, a 7,141-token trial ran out of memory. A 4,091-token trial exposed an fp32/bf16 mismatch; casting the hidden states fixed it. The two-step check used about 6.2 GB peak allocation. Training skips examples over 4,096 tokens instead of cutting off their answers.''')
    md('''
    ## 7. Early coding dataset

    - [SWE-rebench OpenHands traces](https://huggingface.co/datasets/nebius/SWE-rebench-openhands-trajectories), CC-BY-4.0: 144 windows from 52 repositories, split into 40 train / 6 validation / 6 test repositories. The traces were relabeled as memory tasks.
    - Synthetic conversations across software stacks and memory behaviors, labeled by the Pi teacher.
    - Earlier teacher-labeled conversations and a limited amount of deterministic replay.

    Source records keep repository, issue, trajectory, event range and hashes. Multiple tasks from one episode are related examples, not independent conversations. Replay does not add unique examples.

    [LongMemEval](https://github.com/xiaowu0162/LongMemEval) informed the temporal, update and abstention cases. It was not training data, and no LongMemEval score was measured.''')
    code('''
    manifest=read(DATA/'expanded-manifest.json')
    if manifest:
        show([{'split':s,'examples':v['examples'],'episodes':v['episodes'],**v['tasks']} for s,v in manifest['counts'].items()])
        total=sum(v['examples'] for v in manifest['counts'].values())
        print('Total validated rows:',total)
        print('Leakage checks:',manifest['leakage_checks'])
        print('Exact duplicate removals:',len(manifest['duplicates_removed']))
        print('Stage 2 planned exposure:',manifest['stage2'])
        print('Fresh evaluation:',manifest['fresh_evaluation'])
    else: missing('frozen expanded corpus')
    ''')
    code('''
    if manifest:
        fig, axes=plt.subplots(1,2,figsize=(13,5))
        counts=pd.DataFrame({s:v['tasks'] for s,v in manifest['counts'].items()}).T
        counts.plot.bar(stacked=True,ax=axes[0],rot=0,title='Validated task rows by partition')
        coverage=manifest['coverage']['train']
        pd.Series(coverage['stack']).sort_values().plot.barh(ax=axes[1],title='Expanded training rows by stack')
        axes[0].set_ylabel('Rows (not independent conversations)')
        axes[1].set_xlabel('Rows')
        fig.tight_layout(); fig.savefig(FIG/'corpus-coverage.png',dpi=160); plt.show()
        print('Expanded training coverage:', {k:len(v) for k,v in coverage.items()})
        display(pd.DataFrame([{'behavior':k,'training_rows':v} for k,v in coverage['behavior'].items()]))
    ''')
    md('''
    ## 8. Checking the labels

    The same teacher family generated and reviewed the labels. This identified cases for further review but was not independent human review. Rejected cases were removed from all splits.

    A second review checked timestamp flags against the runtime contract: a null timestamp can use the source event time. Other flags identified unsupported test coverage or answers based on facts missing from the input. This led to an adjudication step, with both the original reviews and later decisions kept.''')
    code('''
    audit=read(DATA/'quality-audit.json')
    if audit:
        show([{'reviewed_cases':audit['sampled_cases'],'initial_passes':audit.get('pre_adjudication_accepted'),
               'final_passes':audit['accepted'],'excluded_cases':len(audit['rejected_timelines'])}])
        print(audit['method'])
        rejected=[r for r in audit['results'] if not r['acceptable']]
        show([{'case':r['timeline'],'reason':'; '.join(r.get('issues',[]))} for r in rejected[:5]])
    else: missing('semantic label audit')
    ''')
    md('''
    ## 9. Split checks and an example

    Splits were assigned by repository or scenario family before training. The check below looks for identical prompts across splits. It cannot detect all semantic overlap or pretraining contamination. The public trace sample is also Python-heavy.''')
    code('''
    partitions={s:rows(DATA/f'expanded-canonical/{s}.jsonl') for s in ('train','val','test')}
    if all(partitions.values()):
        hashes={s:{hashlib.sha256(r['prompt'].encode()).hexdigest() for r in rr} for s,rr in partitions.items()}
        for a,b in [('train','val'),('train','test'),('val','test')]:
            assert not hashes[a]&hashes[b], f'Exact prompt overlap: {a}/{b}'
        print('Exact prompt cross-partition checks passed.')
        show([{'split':s,'sha256':digest(DATA/f'expanded-canonical/{s}.jsonl')} for s in partitions])
        example=next((r for r in partitions['train'] if r['timeline'].startswith('expanded-') and r['task']=='extract'),None)
        if example:
            print('Synthetic example:',example['timeline'])
            target=json.loads(example['target'])
            show([{'statement':c['statement'],'correction':c['correction'],'quotes':c['quotes']} for c in target['claims']])
    else: missing('local corpus files for recomputation')
    ''')
    md('''
    ## 10. Training results

    The first 4B stage had 555 training rows: 507 fit the token cap and 48 were skipped. Of the validation rows, 64 fit and 18 were skipped. It ran for one epoch at 5e-5, effective batch size 8, for 64 optimizer steps.

    The next iteration extended the work from coding conversations to general memory. The planned second coding stage was not run; it was replaced by general stage A using the first audited shard, followed by stage B using the remaining accepted rows plus replay. The saved configurations below give the actual counts and exclusions. Loss curves show training progress, not memory accuracy.''')
    code('''
    runs=[('bootstrap 1.7B','local-bootstrap'),('refined 1.7B','local-refined'),('4B stage 1','local-4b-v2'),('4B general stage A','local-4b-general-stage-a'),('4B general-purpose','local-4b-general')]
    status=[]
    fig,ax=plt.subplots(figsize=(9,4)); plotted=False
    for label,folder in runs:
        run=SLM/'out'/folder
        config=read(run/'run-config.json')
        completion=read(run/'training-result.json')
        status.append({'run':label,'adapter_saved':(run/'final/adapter_model.safetensors').exists(),'run_config_saved':config is not None,'completion_receipt':completion is not None})
        if completion:
            print(label+' completion metrics'); display(completion)
        states=list(run.glob('**/trainer_state.json'))
        history=[]
        if states:
            state=max((read(p) for p in states),key=lambda x:x.get('global_step',0))
            history=state.get('log_history',[])
        else:
            log=DATA/'receipts'/f'{folder}.log'
            if log.exists():
                for text in re.findall(r"\\{[^{}]*'loss'[^{}]*\\}",log.read_text()):
                    try: history.append(ast.literal_eval(text))
                    except (ValueError,SyntaxError): pass
        validations=[x for x in history if 'eval_loss' in x]
        if validations: status[-1]['last_eval_loss']=validations[-1]['eval_loss']
        loss=[x for x in history if 'loss' in x]
        if loss:
            ax.plot([x.get('step',i) for i,x in enumerate(loss)],[float(x['loss']) for x in loss],label=label); plotted=True
        if config:
            print(label); display({k:v for k,v in config.items() if k!='exclusions'})
            print('Excluded rows:', {k:len(v) for k,v in config.get('exclusions',{}).items()})
    show(status)
    if plotted:
        ax.set(xlabel='Logged step (index if absent in historical log)',ylabel='Answer-only training loss',title='Training diagnostics; curricula differ')
        ax.legend(); fig.tight_layout(); fig.savefig(FIG/'training-loss.png',dpi=160); plt.show()
    else:
        plt.close(fig); missing('loss history')
    ''')
    md('''
    ## 11. Early model comparisons

    The 1.7B student, 4B base and 4B student use the same constrained decoder and Q8 quantization. The 48-case comparison is development evidence. A separate 60-case held-out subset has 36 extraction, 12 consolidation and 12 reflection examples.

    First-try validity and eventual acceptance are separate scores. Support measures emitted claims; coverage includes failed windows. The teacher family also does the grading, so these scores still need independent review. Timings from different hardware or concurrent workloads are not comparable.''')
    code('''
    evaluation_files=sorted((DATA/'eval').glob('*-summary.json')) if (DATA/'eval').exists() else []
    # Some evaluator versions use the label itself as the summary filename.
    evaluation_files += [p for p in sorted((DATA/'eval').glob('*.json')) if not p.name.endswith(('-summary.json','-semantic.json'))] if (DATA/'eval').exists() else []
    if evaluation_files:
        for p in evaluation_files:
            print(p.name); display(read(p))
    else: missing('completed matched and final student evaluation')
    for p in sorted((DATA/'eval').glob('*-semantic.json')):
        print(p.name); display(read(p))
    ''')
    md('''
    ## 12. Connecting the model to the app

    Structured decoding with the 1.7B bridge model unblocked extraction job `731699d3-52c1-4d08-af65-fa6b2031a8b6` on 8 October. Its payment-gateway fact and graph projection completed. This checked that particular recovery, not the final 4B model.

    The final model has separate checks for extraction, observations, correction history, graph edges, recall and cited answers.''')
    code('''
    receipt=read(DATA/'receipts/final-acceptance.json')
    if receipt: show_acceptance(receipt)
    else: display(Markdown('The next training iteration used the general-memory curriculum instead of coding stage 2. See Section 14 for those model checks.'))
    ''')
    md('''
    ## 13. Prompt iteration: everyday conversations

    The initial prompts provided a coding-focused starting point. The next iteration extended them to everyday conversations while keeping the existing evidence and slot schema.

    [Hindsight, Appendix A.1](https://arxiv.org/pdf/2512.12818v1#page=23) and its [pinned implementation](https://github.com/vectorize-io/hindsight/blob/1152717735237c26986b877789704b89bac89681/hindsight-api-slim/hindsight_api/engine/retain/fact_extraction.py) informed the treatment of context, references and dates. The mapping is in `reports/PROMPT_DESIGN.md`.

    Context-budget tests showed that the 7,016-byte draft left too little room for earlier facts in four cases. The next version shortened the instructions and passed all 17 prompt tests. Further iterations removed the full default-valued example and added per-claim checks, bringing the prompt to 5,649 bytes. All eight development probes passed structural validation, but date and attribution errors remained.''')
    code('''
    probe_dir=SLM/'data/research-v3/prompt-probes'
    probe_results=[]
    for path in sorted(probe_dir.glob('*.json')):
        result=read(path)
        if 'case' in result:
            probe_results.append({'variant':result['variant'],'case':result['case'],'accepted':result['accepted'],
                                  'claims': [c['statement'] for c in result.get('value',{}).get('claims',[])],
                                  'review_criterion':result['review_criterion']})
    if probe_results: show(probe_results)
    else: missing('prompt revision development probes')
    review=read(probe_dir/'manual-review.json')
    if review: display(review)
    ''')
    md('''
    ## 14. General memory dataset and final model

    The replacement dataset covers sixteen domains, including relationships, food, travel, learning, work, routines and software. Scenarios were split by family, then labeled, checked and reviewed. Related tasks from one scenario still count as correlated rows.

    The cells below load the final corpus counts, training configuration, evaluations and live checks. The release is named `aryaniyaps/mem-extractor`; `memex-extractor` in older files is a historical name.''')
    code('''
    GENERAL=SLM/'data/research-v3'
    general_manifest=read(GENERAL/'manifest.json')
    interim=read(GENERAL/'interim-manifest.json')
    if not general_manifest and interim:
        display(Markdown('**Interim audited shard; not the final corpus:**'))
        display({k:interim[k] for k in ('status','counts','coding_replay_rows','leakage_checks') if k in interim})
    if general_manifest:
        show([{'split':split,'rows':value['rows'],'episodes':value['episodes'],**value['tasks'],'coding_fraction':value['coding_fraction']} for split,value in general_manifest['counts'].items()])
        print('Frozen total rows:',sum(v['rows'] for v in general_manifest['counts'].values()))
        print('Audit decisions:',general_manifest.get('label_audit_counts'))
        print('Continuation curriculum:',general_manifest.get('continuation'))
        print('Held-out selection:',general_manifest.get('heldout'))
        print('Leakage checks:',general_manifest.get('leakage_checks'))
        fig,ax=plt.subplots(figsize=(10,6))
        pd.Series(general_manifest['counts']['train']['domains']).sort_values().plot.barh(ax=ax,title='General-purpose training rows across 16 domains')
        ax.set_xlabel('Task rows; multiple tasks can share an episode')
        fig.tight_layout(); fig.savefig(FIG/'general-domain-coverage.png',dpi=160); plt.show()
        print('Accepted episodes:',len(general_manifest.get('accepted_episodes',[])))
        print('Excluded cases:',len(general_manifest.get('excluded',[])))
    else: missing('frozen general-purpose corpus manifest')
    general_audit=read(GENERAL/'quality-audit.json')
    if general_audit:
        display({k:v for k,v in general_audit.items() if k not in ('results','cases','rejected_timelines')})
    else: missing('completed general-purpose semantic audit')
    config=read(SLM/'out/local-4b-general/run-config.json')
    if config: display({k:v for k,v in config.items() if k!='exclusions'})
    else: missing('final general-purpose training configuration')
    print('Final adapter saved:',(SLM/'out/local-4b-general/final/adapter_model.safetensors').exists())
    for label in ('base-general-64','student-general-64','base-external-locomo-16','student-external-locomo-16'):
        for kind in ('summary','semantic'):
            result=read(GENERAL/'eval'/f'{label}-{kind}.json')
            if result:
                print(label,kind); display(result)
            else: missing(f'{label} {kind} evaluation')
    acceptance=read(GENERAL/'final-acceptance.json')
    if acceptance: show_acceptance(acceptance)
    else: missing('general-purpose selected-model live extraction and graph acceptance')
    final_graph=GENERAL/'final-graph.png'
    if final_graph.exists():
        display(Markdown('**Captured final graph UI:**'))
        display(Image(filename=str(final_graph)))
    else: missing('final selected-model graph screenshot')
    release=read(GENERAL/'release-receipt.json')
    if release: display(release)
    else: missing('final Hugging Face release receipt')
    ''')
    md('''
    ## 15. External conversation check

    The external set has sixteen short windows from ten [LoCoMo](https://github.com/snap-research/locomo) conversations ([Maharana et al., 2024](https://aclanthology.org/2024.acl-long.747/)). The data is CC-BY-NC-4.0 and stays local for academic evaluation. It is excluded from training and the model package.

    This is a short-window extraction check, not the official LoCoMo QA benchmark or a long-history retrieval test. Labels and grading still use the same teacher family. Source session times have no timezone; the UTC suffix is an API convention. Details are in `reports/EXTERNAL_LOCOMO.md`.''')
    code('''
    external=read(GENERAL/'external-locomo-manifest.json')
    if external: display(external)
    else: missing('completed external LoCoMo-window label/audit manifest')
    ''')
    md('''
    ## 16. Results and remaining limitations

    | Internal test | Base | Fine-tuned |
    |---|---:|---:|
    | Consolidation | 1/16 | 13/16 |
    | Reflection | 10/16 | 15/16 |
    | Supported extraction claims | 58/63 (92.1%) | 38/42 (90.5%) |
    | Reference facts covered | 50/55 (90.9%) | 41/55 (74.5%) |
    | Annotated corrections recovered | 8/8 | 8/8 |

    The fine-tuned model passed structural checks on 63/64 internal cases. It improved consolidation and reflection but lost extraction coverage. These are small, same-family teacher-graded samples.

    The external results also dropped: support went from 39/44 (88.6%) to 22/26 (84.6%), and coverage from 16/40 (40.0%) to 13/40 (32.5%). Both models passed structural checks on all sixteen windows.

    Manual review identified date-handling limitations: “tomorrow” on June 1 became June 3, and “this Friday” from October 8 became October 17. Some date-only events acquired midnight timestamps. These errors remain; successful graph checks and the citation fix below do not resolve them.''')
    md('''
    ## 17. Serving iteration: citation constraints

    The first final-model pilot passed extraction and graph checks, but reflection returned HTTP 502 because of invalid UUID citations. That result motivated the next serving iteration: restrict citations to IDs in the retrieved evidence. The weights did not change.

    The frozen comparison used the original decoder for both models, so its reflection score stays 15/16. A separate retry of the failed row passed in one attempt with the repaired decoder.

    The second pilot led to a refinement of the relationship check. The original check required a literal name in the statement; the revised check uses the resolved subject and relationship quote. Rechecking the saved outputs resolved that false negative; it was not a new inference run. Both pilot outcomes and subsequent checks are shown below.''')
    code('''
    frozen_decoder=read(GENERAL/'frozen-comparison-decoder.json')
    if frozen_decoder: display(frozen_decoder)
    else: missing('frozen matched-decoder provenance')
    for filename in ('candidate-live-acceptance.json','candidate-live-acceptance-v2.json','candidate-live-acceptance-v3.json'):
        pilot=read(GENERAL/filename)
        if pilot:
            show([{'receipt':filename,'passed':pilot.get('passed'),
                   'completed_checks':len(pilot.get('checks',{})),
                   'passed_checks':sum(v is True for v in pilot.get('checks',{}).values()),
                   'error':pilot.get('error'),
                   'worker_model':pilot.get('worker_model')}])
        else: missing(filename)
    manual_review=read(GENERAL/'student-manual-review.json')
    if manual_review:
        display({k:manual_review[k] for k in ('method','selection','summary','prominent_limitations') if k in manual_review})
    decoder_regression=read(GENERAL/'reflection-decoder-python-check.json')
    if decoder_regression:
        show([{'scope':decoder_regression['scope'],'accepted':decoder_regression['accepted'],
               'attempts':len(decoder_regression.get('attempts',[])),
               'decoder_sha256':decoder_regression['structured_sha256']}])
        display(Markdown('This is a separate regression check on the previously failed row. The frozen reflection score remains 15/16; it is not retroactively changed to 16/16.'))
    ''')
    md(r'''
    ## 18. Running the scripts

    Use the local virtual environment and clear the externally supplied `PYTHONPATH`. The commands here are examples, not executable notebook cells.

    ```bash
    cd slm-distill
    env -u PYTHONPATH .venv/bin/python train.py \
      --base Qwen/Qwen3-4B-Instruct-2507 \
      --revision cdbee75f17c01a7cc42f958dc650907174af0554 \
      --qlora --sets data/research-v3/continuation-sets \
      --adapter out/local-4b-general-stage-a/final --out out/local-4b-general \
      --rank 16 --accum 8 --epochs 1 --max-length 4096 --loss-chunk-size 256 --lr 3e-5 --eval-on-epoch
    ```

    This continues stage A. Use the saved `run-config.json` and frozen inputs to reproduce the run, and choose a new output directory to avoid overwriting it. Teacher generation is separate and uses provider credits.

    From the repository root, rebuild this report with:

    ```bash
    env -u PYTHONPATH slm-distill/.venv/bin/python slm-distill/build_notebook.py --execute
    ```

    The README maps the scripts. Large datasets, checkpoints and teacher caches are ignored by Git; keep their manifests when moving them.''')
    md('''
    ## 19. What remains

    The model fits the laptop and works through the memory pipeline. It is better at consolidation and reflection on the internal sample, but extraction coverage is lower than the base model.

    The next priorities are date handling, missed facts and an independently reviewed test set. Sixteen domains do not establish performance on arbitrary topics, languages or long histories. Teacher-generated labels and teacher grading are also a shared source of bias.''')

    return nbf.v4.new_notebook(
        cells=cells,
        metadata={
            'kernelspec': {
                'display_name': 'SDP distillation (.venv)',
                'language': 'python',
                'name': 'sdp-distill',
            },
            'language_info': {'name': 'python'},
        },
    )


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--execute', action='store_true', help='Refresh outputs from local results')
    args = parser.parse_args()
    notebook = build_notebook()
    if args.execute:
        NotebookClient(
            notebook, timeout=180, kernel_name='sdp-distill',
            resources={'metadata': {'path': str(ROOT.parent)}},
        ).execute()
    path = ROOT / 'Finetuning_Iteration_Report.ipynb'
    nbf.write(notebook, path)
    print(path)


if __name__ == '__main__':
    main()
