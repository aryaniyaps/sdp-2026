"""Builds and executes Benchmark_Report.ipynb from stored run artifacts. Calls no model and changes no service.

    cd benchmark && uv run python build_notebook.py --execute
"""

import argparse
from pathlib import Path
from textwrap import dedent

import nbformat as nbf
from nbclient import NotebookClient

HERE = Path(__file__).resolve().parent


def build(test_run, properties_run):
    cells = []

    def md(text):
        cells.append(nbf.v4.new_markdown_cell(dedent(text).strip()))

    def code(text):
        cells.append(nbf.v4.new_code_cell(dedent(text).strip()))

    md(f'''
    # Memory Engine benchmark

    This notebook is the evidence for the claim that the memory engine is useful to an agent. Every number below is
    recomputed here from the stored run records in `benchmark/runs/`. No model is called and no service is touched,
    so *Run All* takes seconds.

    **What was measured.** LoCoMo [Maharana et al., ACL 2024] has ten long two-person conversations (272 sessions)
    and 1,540 scored questions about them. Each conversation is retained into the engine as its own memory bank.
    Each question is then answered by the same reader model under six conditions that differ only in what the reader is shown:

    | condition | what the reader sees |
    |---|---|
    | `enhanced` | the full engine: extracted assertions with source quotes, inferences, graph, temporal planning |
    | `raw_hybrid` | the engine's lexical + vector search over raw source chunks (plain RAG) |
    | `recent_window` | the newest 8,192 tokens of the conversation, the same budget as the engine: what an agent without memory keeps |
    | `full_context` | the whole conversation (an upper reference, and the most expensive) |
    | `no_memory` | the question only |
    | `no_graph`, `no_consolidation` | `enhanced` without one module (300-question stratified sample) |

    Two judges from different model families grade every answer with Mem0's LoCoMo rubric: TypeSafe **Jev**, a decision
    model (primary), and **gpt-4o-mini**, the judge Mem0 used. Mem0's token F1 and BLEU-1 need no model at all.
    A separate deterministic property suite checks knowledge updates, relative dates, abstention, attribution and
    credential filtering on the engine's stored state.

    **How to check it.** `benchmark/README.md` shows how to re-run any stage. `run.py verify` rebuilds every stored prompt from
    the stored retrieval and the pinned dataset, and `verify --live N` re-asks the models on N random items. The
    spot-check section below prints random items in full so a reader can grade them by hand.
    ''')
    code(f'''
    from pathlib import Path
    import json, hashlib, random, collections, subprocess
    import pandas as pd
    import matplotlib.pyplot as plt
    from IPython.display import display, Markdown
    pd.set_option('display.max_colwidth', 120)
    BENCH = next(p for p in [Path.cwd(), *Path.cwd().parents] if (p / 'benchmark/run.py').exists()) / 'benchmark'
    RUN = BENCH / 'runs' / {test_run!r}
    PROPS = BENCH / 'runs' / {properties_run!r}
    CONFIG = json.loads((BENCH / 'config.json').read_text())
    def read(p): return json.loads(Path(p).read_text())
    def sha(p): return hashlib.sha256(Path(p).read_bytes()).hexdigest()
    report = read(RUN / 'report.json')
    manifest = read(RUN / 'manifest.json')
    items = pd.DataFrame([json.loads(l) for l in (RUN / 'items.jsonl').read_text().splitlines()])
    ORDER = [c for c in CONFIG['conditions'] if c in report['conditions']]
    ''')

    md('## 1. Provenance\n\nThe run manifest freezes the code fingerprint, configuration, dataset checksum, model digests and hardware. '
       'The checks below recompute the checksums from the files on disk.')
    code('''
    data = BENCH / 'data' / CONFIG['dataset']['file']
    vendor = read(BENCH / 'vendor/mem0/manifest.json')
    checks = {
        'dataset sha256 matches the pin': data.exists() and sha(data) == CONFIG['dataset']['sha256'],
        'dataset pin matches the run manifest': manifest['dataset_sha256'] == CONFIG['dataset']['sha256'],
        'source archive matches the manifest': sha(RUN / 'source.tar.gz') == manifest['source_archive_sha256'],
        **{f'vendored mem0/{f} matches upstream pin': sha(BENCH / 'vendor/mem0' / f) == e['sha256'] for f, e in vendor['files'].items()},
    }
    display(pd.DataFrame({'check': list(checks), 'passed': list(checks.values())}))
    display(pd.DataFrame([{
        'run': RUN.name, 'split': manifest['split'], 'created': manifest['created_at'], 'commit': manifest['commit'],
        'code_fingerprint': manifest['code_fingerprint'][:16], 'worker': manifest['service']['worker_model'],
        'worker digest': (manifest.get('ollama_digests', {}).get(CONFIG['engine']['worker_model']) or '')[:16],
        'embedder': manifest['service']['embedding_model'], 'reader': f"{CONFIG['reader']['provider']}/{CONFIG['reader']['model']}",
        'gpu': manifest['hardware']['gpu'], 'items': len(manifest['item_ids'])}]).T.rename(columns={0: 'value'}))
    ''')

    md('## 2. Dataset')
    code('''
    import sys; sys.path.insert(0, str(BENCH))
    import run as bench
    banks, all_items = bench.load()
    sessions = [s for b in banks.values() for s in b['sessions']]
    turns = [t for s in sessions for t in s['turns']]
    display(pd.DataFrame([{'conversations': len(banks), 'sessions': len(sessions), 'turns': len(turns),
                           'characters': sum(len(t['content']) for t in turns),
                           'scored questions': len(all_items)}]))
    display(pd.Series(collections.Counter(i['category'] for i in all_items), name='questions').to_frame())
    ''')

    md('''
    ## 3. Pre-registered success criteria

    These thresholds were committed to `benchmark/config.json` before the test run; `git log -p benchmark/config.json`
    shows the order. They are evaluated on the primary judge.
    ''')
    code('''
    display(pd.Series({k: v for k, v in CONFIG['criteria'].items() if k != 'note'}, name='threshold').to_frame())
    display(pd.Series(report['criteria'], name='result').to_frame())
    ''')

    md('## 4. Main results')
    code('''
    rows = []
    for c in ORDER:
        s = report['conditions'][c]
        rows.append({'condition': c, 'n': s['answered'], 'Jev accuracy': s.get('jev_accuracy'),
                     'gpt-4o-mini accuracy': s.get('gpt4omini_accuracy'), 'F1': s['f1'], 'BLEU-1': s['bleu1'],
                     'context tokens (mean)': s['context_tokens_mean'],
                     'recall p50 ms': s.get('recall_ms_p50'), 'recall p95 ms': s.get('recall_ms_p95'),
                     'reader p50 s': s['reader_seconds_p50']})
    main = pd.DataFrame(rows).set_index('condition')
    display(main.style.format(precision=3, na_rep='–'))
    ''')
    code('''
    fig, axes = plt.subplots(1, 2, figsize=(12, 4))
    main[['Jev accuracy', 'gpt-4o-mini accuracy', 'F1']].plot.bar(ax=axes[0], rot=20)
    axes[0].set_ylim(0, 1); axes[0].set_title('Answer quality by condition'); axes[0].legend(loc='upper left')
    for c in ORDER:
        axes[1].scatter(main.loc[c, 'context tokens (mean)'], main.loc[c, 'Jev accuracy'], s=60)
        axes[1].annotate(c, (main.loc[c, 'context tokens (mean)'], main.loc[c, 'Jev accuracy']), xytext=(5, 5), textcoords='offset points')
    axes[1].set_xscale('symlog'); axes[1].set_xlabel('reader context tokens per question (mean)'); axes[1].set_ylabel('Jev accuracy')
    axes[1].set_title('Accuracy against context cost'); plt.tight_layout(); plt.show()
    ''')

    md('## 5. Paired differences against `enhanced`\n\nPaired bootstrap (2,000 resamples) over the same questions; positive means `enhanced` is better.')
    code('''
    rows = []
    for judge, table in report['paired_vs_enhanced'].items():
        for c, ci in table.items():
            if ci: rows.append({'judge': judge, 'baseline': c, 'n': ci['n'], 'enhanced minus baseline': ci['difference'],
                                'per-question ci95': f"[{ci['ci95'][0]:+.3f}, {ci['ci95'][1]:+.3f}]",
                                'per-conversation ci95': f"[{ci['cluster_ci95'][0]:+.3f}, {ci['cluster_ci95'][1]:+.3f}]"})
    display(pd.DataFrame(rows).style.format(precision=3))
    ''')
    md('''
    ### Held-out conversations

    Faults were diagnosed on conv-26 and conv-30 only (see section 11). The same table for the other eight
    conversations, which no fix ever looked at:
    ''')
    code('''
    if 'held_out' in report:
        h = report['held_out']
        display(Markdown('Conversations: ' + ', '.join(h['conversations'])))
        display(pd.DataFrame([{'condition': c, 'n': h['conditions'][c]['answered'], 'Jev': h['conditions'][c].get('jev_accuracy'),
                               'gpt-4o-mini': h['conditions'][c].get('gpt4omini_accuracy'), 'F1': h['conditions'][c]['f1']}
                              for c in ORDER if c in h['conditions']]).set_index('condition').style.format(precision=3))
        display(pd.Series(h['criteria'], name='criteria on held-out').to_frame())
    ''')

    md('''
    ### Run-to-run noise

    Repeats 1 and 2 re-ask the reader on the same 300-question sample. If the spread here is small next to the
    differences above, those differences are not reader noise.
    ''')
    code('''
    display(pd.DataFrame([{'condition': c, 'questions': v['questions'],
                           **{f'repeat {k}': a for k, a in enumerate(v['accuracy_by_repeat'])},
                           'stdev': v['accuracy_stdev'], 'verdict flip rate': v['verdict_flip_rate']}
                          for c, v in report['repeats'].items()]).set_index('condition').style.format(precision=3, na_rep='–'))
    ''')

    md('## 6. By question category')
    code('''
    cat = pd.DataFrame([{'condition': c, 'category': k, 'n': v['n'], 'Jev': v.get('jev_accuracy'), 'gpt-4o-mini': v.get('gpt4omini_accuracy'), 'F1': v['f1']}
                        for c in ORDER for k, v in report['conditions'][c]['categories'].items()])
    pivot = cat.pivot(index='category', columns='condition', values='Jev')[ORDER]
    display(pivot.style.format(precision=3, na_rep='–'))
    pivot.plot.bar(figsize=(11, 4), rot=0, title='Jev accuracy by category'); plt.ylim(0, 1); plt.tight_layout(); plt.show()
    ''')

    md('## 7. Retrieval quality and provenance\n\nLoCoMo labels the dialogue turns that answer each question. '
       'Packed recall is the share of those evidence sessions and turns whose text reaches the reader. Every source quote returned '
       'with a recall result is checked against the original dataset turn it cites.')
    code('''
    rows = []
    for c in ORDER:
        s = report['conditions'][c]
        if 'retrieval' in s:
            rows.append({'condition': c, **s['retrieval'], 'quotes checked': s['quotes_checked'], 'quote exactness': s['quote_exactness']})
    display(pd.DataFrame(rows).set_index('condition').style.format(precision=3))
    ''')

    md('## 8. Judge agreement\n\nThe two judges never see the condition. Cohen’s kappa measures their agreement beyond chance.')
    code('''
    if report['judge_agreement'] and {'jev', 'gpt4omini'} <= set(items.columns):
        display(pd.Series(report['judge_agreement'], name='value').to_frame())
        both = items.dropna(subset=['jev', 'gpt4omini'])
        display(pd.crosstab(both['jev'], both['gpt4omini'], rownames=['Jev'], colnames=['gpt-4o-mini']))
    else:
        display(Markdown('Only one judge has graded this run.'))
    ''')

    md('## 9. Cost of building memory\n\nIngestion runs once per conversation on the 8 GB laptop GPU; recall is per question.')
    code('''
    ing = report['ingestion']
    display(pd.DataFrame([{**{k: v for k, v in ing.items() if k != 'worker_ms'},
                           **{f'{k} (GPU-busy hours)': v / 3.6e6 for k, v in ing['worker_ms'].items()}}]).T.rename(columns={0: 'value'}))
    ''')

    md('## 10. Deterministic property suite\n\nScripted conversations checked against the engine’s stored state and responses; no judge is involved. '
       'The reader comparison asks the same questions over three contexts and scores answers by string match.')
    code('''
    if (PROPS / 'summary.json').exists():
        props = read(PROPS / 'summary.json')
        display(pd.DataFrame([{'check': k, **v, 'rate': v['passed'] / v['n']} for k, v in props['checks'].items()]).set_index('check').style.format({'rate': '{:.2f}'}))
        display(pd.DataFrame([{'family': f, 'condition': c, **v, 'rate': v['correct'] / v['n']}
                              for f, t in props['reader'].items() for c, v in t.items()]).pivot(index='family', columns='condition', values='rate'))
    else:
        display(Markdown('**Property suite results are not available locally.**'))
    ''')

    md('## 11. Iterations on the dev split\n\n`benchmark/iterations.json` records each change made after a dev run, why, and the dev numbers before and after. '
       'The test run used only the final version.')
    code('''
    it = read(BENCH / 'iterations.json')
    display(pd.DataFrame(it['iterations']).set_index('version'))
    ''')

    md('## 12. Spot check\n\nTen random questions, shown in full, so a reader can grade them by hand. Re-run the cell for another sample.')
    code('''
    rng = random.Random()
    for qid in rng.sample(sorted(items['id'].unique()), 10):
        rows = items[items['id'] == qid]
        first = rows.iloc[0]
        display(Markdown(f"**{qid}** ({first['category']}) · *{first['question']}* · gold: **{first['answer']}**"))
        display(rows[[c for c in ['condition', 'hypothesis', 'jev', 'gpt4omini', 'f1'] if c in rows]].set_index('condition'))
        recall = RUN / 'recall' / 'enhanced' / f'{qid}.json'
        if recall.exists():
            print(read(recall)['context'][:1500], '\\n...')
    ''')

    md('''
    ## 13. Comparison with published systems

    Published LoCoMo results from the Mem0 paper (Chhikara et al., 2025, Tables 1 and 2), checked against the paper and
    stored in `benchmark/published.json`. They answer with GPT-4o-mini and this benchmark answers with Claude Haiku 5.5,
    so absolute scores are not comparable. The fair comparison is each system's distance from its own full-context
    baseline and the context it needs.
    ''')
    code('''
    pub = read(BENCH / 'published.json')
    ours = report['conditions']
    rows = [{'system': f'{k} (published)', 'judge accuracy': v, 'gap to own full context': v - pub['locomo_overall_j']['Full-context'],
             'context tokens': pub['locomo_context_tokens'].get(k.split(' (')[0])}
            for k, v in pub['locomo_overall_j'].items()]
    fc = ours['full_context']['gpt4omini_accuracy'] if ours['full_context'].get('gpt4omini_accuracy') is not None else ours['full_context']['jev_accuracy']
    for c in ('full_context', 'enhanced', 'raw_hybrid', 'recent_window'):
        if c in ours:
            a = ours[c].get('gpt4omini_accuracy') if ours[c].get('gpt4omini_accuracy') is not None else ours[c]['jev_accuracy']
            rows.append({'system': f'this engine: {c}', 'judge accuracy': 100 * a, 'gap to own full context': 100 * (a - fc),
                         'context tokens': ours[c]['context_tokens_mean']})
    display(Markdown(pub['caveat']))
    display(pd.DataFrame(rows).set_index('system').style.format(precision=1, na_rep='–'))
    ''')

    md('## 14. Verification')
    code('''
    v = RUN / 'verification.json'
    display(pd.Series({k: (len(x) if isinstance(x, list) else x) for k, x in read(v).items() if k != 'live'}, name='value').to_frame() if v.exists()
            else Markdown('Run `python3 benchmark/run.py verify --split test` to produce verification.json.'))
    if v.exists() and 'live' in read(v):
        display(pd.DataFrame(read(v)['live']).T)
    ''')
    return cells


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("--run", default="locomo-final")
    parser.add_argument("--properties", default="properties-final")
    parser.add_argument("--execute", action="store_true")
    args = parser.parse_args()
    notebook = nbf.v4.new_notebook()
    notebook.cells = build(args.run, args.properties)
    notebook.metadata["kernelspec"] = {"name": "python3", "display_name": "Python 3", "language": "python"}
    path = HERE / "Benchmark_Report.ipynb"
    if args.execute:
        NotebookClient(notebook, timeout=600, kernel_name="python3", resources={"metadata": {"path": str(HERE)}}).execute()
    nbf.write(notebook, path)
    print(path)


if __name__ == "__main__":
    main()
