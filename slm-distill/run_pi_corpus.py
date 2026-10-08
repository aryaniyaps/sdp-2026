"""Generate and validate an isolated compact teacher corpus, resumably."""
from __future__ import annotations
import json
import sys
from concurrent.futures import ThreadPoolExecutor, as_completed
from pathlib import Path

from pi_teacher import PiTeacher, configure_stage
import scenarios
import label
import label_turns
import aux_tasks
import build_dataset

ROOT = Path(__file__).parent/'data'/'pi-corpus'
INDEXES = [*range(16), *range(260, 264), *range(280, 284)]


def main():
    folders = {k: ROOT/k for k in ('timelines', 'labeled', 'turns', 'aux', 'sets')}
    for folder in folders.values():
        folder.mkdir(parents=True, exist_ok=True)
    # Reuse the already verified first timeline without another model request.
    first = Path(__file__).parent/'data'/'teacher-timelines'/'tl0000.json'
    if first.exists() and not (folders['timelines']/first.name).exists():
        (folders['timelines']/first.name).write_bytes(first.read_bytes())
    configure_stage(scenarios)
    teacher = PiTeacher(ROOT/'teacher-cache', ROOT/'teacher-usage.jsonl')

    def one(index):
        tid = f'tl{index:04d}'
        status = scenarios.generate_one(teacher, index, folders['timelines'])
        print(tid, 'generated:', status, flush=True)
        path = folders['timelines']/f'{tid}.json'
        timeline = json.loads(path.read_text())
        print(tid, 'whole:', label.label_timeline(teacher, timeline, folders['labeled']), flush=True)
        whole = json.loads((folders['labeled']/path.name).read_text())
        print(tid, 'turns:', label_turns.label_timeline(teacher, timeline, whole, folders['turns']), flush=True)
        print(tid, 'aux:', aux_tasks.process(teacher, path, folders['labeled'], folders['aux']), flush=True)

    failures = []
    with ThreadPoolExecutor(6) as pool:
        jobs = {pool.submit(one, i): i for i in INDEXES}
        for future in as_completed(jobs):
            try:
                future.result()
            except Exception as exc:
                failures.append({'index': jobs[future], 'error': str(exc)[:500]})
                print('FAILED', jobs[future], str(exc)[:300], flush=True)
    manifest = {'provider': teacher.provider, 'model': teacher.model, 'compact_sessions': True,
                'indexes': INDEXES, 'failures': failures, 'summary': teacher.summary()}
    (ROOT/'manifest.json').write_text(json.dumps(manifest, indent=2)+'\n')
    print(teacher.summary(), flush=True)
    if failures:
        sys.exit(f'{len(failures)} timelines failed; progress is cached for a resumable retry')
    sys.argv = ['build_dataset', '--labeled', str(folders['labeled']), '--turns', str(folders['turns']),
                '--aux', str(folders['aux']), '--out', str(folders['sets'])]
    build_dataset.main()


if __name__ == '__main__':
    main()
