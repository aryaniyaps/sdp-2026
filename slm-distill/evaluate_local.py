"""Evaluate the local teacher corpus with engine limits and saved raw receipts.

    python slm-distill/evaluate_local.py --model memex-extractor:local-v1 --label local-v1

Uses the existing evaluator and Pi teacher judge. Report judge agreement separately
from measured schema acceptance; the judge is not an independent human review.
"""
from pathlib import Path
import hashlib
import json
import sys
import threading

import evaluate
import ollama_client
import prompt
from pi_teacher import PiTeacher


def main():
    root = Path(__file__).parent/'data'/'pi-corpus'
    evaluate.DATA = root
    evaluate.EMBED_MODEL = 'qwen3-embedding-cpu'
    evaluate.Teacher = PiTeacher
    if '--sets' not in sys.argv:
        sys.argv += ['--sets', str(root/'sets')]
    if '--ollama' not in sys.argv:
        sys.argv += ['--ollama', 'http://127.0.0.1:11434']
    if '--workers' not in sys.argv:
        sys.argv += ['--workers', '1']
    label = sys.argv[sys.argv.index('--label')+1]
    folder = root/'eval'; folder.mkdir(exist_ok=True)
    original, lock = ollama_client.generate, threading.Lock()
    with (folder/(label+'-responses.jsonl')).open('w') as log:
        def checked(base, model, user, **kwargs):
            ctx, cap = kwargs.get('num_ctx', 12288), kwargs.get('num_predict', 3072)
            error = ''
            if prompt.estimate_tokens(user) + cap > ctx:
                out = {'response': '', 'wall_seconds': 0, 'eval_count': 0}
                error = 'engine prompt preflight rejected this input'
            else:
                out = original(base, model, user, **kwargs)
                if out.get('prompt_eval_count') is None or out['prompt_eval_count'] + 16 >= ctx:
                    error = 'missing prompt count or truncated prompt'
                elif out.get('done_reason') == 'length':
                    error = 'truncated answer'
            with lock:
                log.write(json.dumps({'model': model, 'prompt_sha256': hashlib.sha256(user.encode()).hexdigest(),
                                      'engine_error': error, 'payload': out})+'\n'); log.flush()
            if error:
                out = {**out, 'response': ''}
            return out
        ollama_client.generate = checked
        evaluate.main()


if __name__ == '__main__':
    main()
