"""Score the synthetic held-out set through the same Ollama API as the app.

Reports schema acceptance separately from semantic slot accuracy, empty-input
behavior, supporting UUIDs and citations. Saves every raw response. This is a
schema/bootstrap evaluation; it does not establish real conversation accuracy.
"""
from __future__ import annotations

import argparse
import hashlib
import json
import time
import urllib.request
from collections import Counter
from pathlib import Path

import prompt
import rules


def evaluate(row, response):
    gold = json.loads(row['target'])
    task, meta = row['task'], row['meta']
    if task == 'extract':
        actual = rules.validate_extraction(response, meta['raw_events'], meta['existing'])
        expected = gold['claims']
        key = lambda c: (c['subject']['name'].casefold(), c['predicate'], c['value'].casefold(), c['correction'])
        got, wanted = set(map(key, actual)), set(map(key, expected))
        return {'semantic_exact': got == wanted, 'tp': len(got & wanted), 'fp': len(got - wanted),
                'fn': len(wanted - got), 'empty_correct': not actual if not expected else None,
                'secret_leaked': rules.leaks_secret(json.dumps(response))}
    if task == 'consolidate':
        actual = rules.validate_consolidation(response, meta['facts'])
        expected = gold['observations']
        supports = lambda obs: {(o['subject_id'], tuple(sorted(set(o['supports'])))) for o in obs}
        return {'semantic_exact': supports(actual) == supports(expected),
                'empty_correct': not actual if not expected else None}
    actual = rules.validate_reflect(response, set(meta['allowed']))
    return {'semantic_exact': actual['insufficient_evidence'] == gold['insufficient_evidence']
            and set(actual['citations']) == set(gold['citations']),
            'abstention_correct': actual['insufficient_evidence'] == gold['insufficient_evidence']}


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument('--sets', type=Path, default=Path(__file__).parent/'data'/'bootstrap-sets')
    ap.add_argument('--model', default='memex-extractor')
    ap.add_argument('--base', default='http://127.0.0.1:11434')
    ap.add_argument('--out', type=Path, default=Path(__file__).parent/'data'/'eval'/'local-bootstrap')
    ap.add_argument('--limit', type=int)
    ap.add_argument('--cpu', action='store_true', help='keep inference off the GPU during training')
    args = ap.parse_args()
    rows = [json.loads(line) for line in (args.sets/'test.jsonl').read_text().splitlines()]
    if args.limit:
        rows = rows[:args.limit]
    args.out.mkdir(parents=True, exist_ok=True)
    counts, results = Counter(), []
    with (args.out/'responses.jsonl').open('w') as log:
        for index, row in enumerate(rows):
            started, accepted, error = time.monotonic(), False, ''
            user = row['prompt']
            raw = ''
            metrics = {}
            for attempt in range(3):
                body = {'model': args.model, 'system': prompt.SYSTEM, 'prompt': user, 'stream': False,
                        'format': 'json', 'think': False, 'keep_alive': '15m',
                        'options': {'temperature': 0, 'num_ctx': 12288, 'num_predict': 3072}}
                if args.cpu:
                    body['options'].update(num_gpu=0, num_thread=4)
                request = urllib.request.Request(args.base+'/api/generate', json.dumps(body).encode(),
                                                 {'content-type': 'application/json'})
                with urllib.request.urlopen(request, timeout=300) as reply:
                    payload = json.load(reply)
                raw = payload.get('response', '')
                try:
                    if payload.get('done_reason') == 'length':
                        raise ValueError('truncated output')
                    metrics = evaluate(row, json.loads(raw))
                    accepted = True
                    break
                except (json.JSONDecodeError, rules.Invalid, ValueError) as exc:
                    error = str(exc)
                    user = (row['prompt'] + f'\nRepair attempt {attempt}. Previous response: {raw}\n'
                            f'Validation error: {error}. Return complete corrected JSON with exact quotes and IDs from the supplied evidence.')
            result = {'index': index, 'timeline': row['timeline'], 'task': row['task'],
                      'prompt_sha256': hashlib.sha256(row['prompt'].encode()).hexdigest(),
                      'accepted': accepted, 'first_try': accepted and attempt == 0,
                      'attempts': attempt+1, 'seconds': round(time.monotonic()-started, 3),
                      'response': raw, 'error': error if not accepted else '', **metrics}
            results.append(result)
            log.write(json.dumps(result)+'\n'); log.flush()
            counts[row['task']+':total'] += 1
            counts[row['task']+':accepted'] += accepted
            counts[row['task']+':first_try'] += result['first_try']
            counts[row['task']+':semantic_exact'] += bool(metrics.get('semantic_exact'))
            for name in ('tp', 'fp', 'fn', 'secret_leaked'):
                counts[name] += metrics.get(name, 0)
            print(f'{index+1}/{len(rows)} {row["task"]} accepted={accepted} exact={metrics.get("semantic_exact")} {result["seconds"]}s', flush=True)
    summary = {'model': args.model, 'dataset_sha256': hashlib.sha256((args.sets/'test.jsonl').read_bytes()).hexdigest(),
               'scope': 'synthetic held-out project identities; no real-world accuracy claim',
               'cpu_only': args.cpu,
               'count': len(results), 'counts': dict(counts), 'seconds': sum(r['seconds'] for r in results)}
    (args.out/'summary.json').write_text(json.dumps(summary, indent=2)+'\n')
    print(json.dumps(summary, indent=2))


if __name__ == '__main__':
    main()
