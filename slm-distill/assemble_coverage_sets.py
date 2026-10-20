"""Stage C training sets: the coverage episodes plus a replay of the stage A/B rows.

New rows keep the split their family was given before generation. Replay rows are drawn with a fixed seed,
in the same task mix as the earlier sets, so extraction gains do not cost consolidation or reflection. The
earlier validation set is kept whole, so a regression on the old tasks shows in validation loss.

    python assemble_coverage_sets.py --previous ../path/to/research-v3/continuation-sets
"""
import argparse, collections, hashlib, json, random
from pathlib import Path

ROOT = Path(__file__).parent / 'data/research-v4'


def lines(path):
    return [json.loads(line) for line in Path(path).read_text().splitlines() if line.strip()]


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument('--previous', type=Path, required=True, help='stage B sets (train.jsonl, val.jsonl)')
    ap.add_argument('--replay', type=int, default=600)
    ap.add_argument('--seed', type=int, default=20261020)
    args = ap.parse_args()
    new = {'train': [], 'val': []}
    batches = sorted((ROOT / 'expanded').glob('batch-*.json'))
    for path in batches:
        batch = json.loads(path.read_text())
        new[batch['split']].extend(batch['rows'])
    old_train, old_val = lines(args.previous / 'train.jsonl'), lines(args.previous / 'val.jsonl')
    rng = random.Random(args.seed)
    by_task = collections.defaultdict(list)
    for row in old_train:
        by_task[row['task']].append(row)
    replay = []
    for task, rows in sorted(by_task.items()):
        share = round(args.replay * len(rows) / len(old_train))
        replay.extend(rng.sample(rows, min(share, len(rows))))
    for row in replay:
        row.setdefault('provenance', {})['replay'] = 'stage-b'
    train = new['train'] + replay
    rng.shuffle(train)
    val = new['val'] + old_val
    out = ROOT / 'coverage-sets'
    out.mkdir(parents=True, exist_ok=True)
    manifest = {'batches': len(batches), 'seed': args.seed, 'previous': str(args.previous), 'files': {}}
    for name, rows in (('train', train), ('val', val)):
        text = ''.join(json.dumps(r, ensure_ascii=False) + '\n' for r in rows)
        (out / f'{name}.jsonl').write_text(text)
        manifest['files'][name] = {'rows': len(rows), 'sha256': hashlib.sha256(text.encode()).hexdigest(),
                                   'tasks': dict(collections.Counter(r['task'] for r in rows))}
    manifest['new_rows'] = {k: len(v) for k, v in new.items()}
    manifest['replay_rows'] = len(replay)
    manifest['new_extract_claims'] = {k: sum(len(json.loads(r['target'])['claims']) for r in v if r['task'] == 'extract')
                                      for k, v in new.items()}
    (out / 'manifest.json').write_text(json.dumps(manifest, indent=2) + '\n')
    print(json.dumps(manifest, indent=2))


if __name__ == '__main__':
    main()
