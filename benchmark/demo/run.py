#!/usr/bin/env python3
"""Actual three-session Pi coding demonstration, distinct from the benchmark."""
import json, os, pathlib, subprocess, time, urllib.request
ROOT=pathlib.Path(__file__).resolve().parents[2]
OUTPUT=ROOT/'benchmark/runs/pi-coding-demo'
REPO=OUTPUT/'repository'
NAMESPACE='demo:pi-duration-v1'
BASE=os.environ.get('MEMORY_URL','http://127.0.0.1:8080')
def request(path):
    with urllib.request.urlopen(BASE+path,timeout=30) as r:return json.load(r)
def ready():
    while True:
        status=request('/api/v2/status?namespace='+NAMESPACE)
        (OUTPUT/'status.json').write_text(json.dumps(status,indent=2))
        if any(g['status']=='failed' for g in status['jobs']):raise RuntimeError('memory processing failed; inspect saved status and service jobs')
        if status['ready']:return
        print(json.dumps({'stage':'memory_wait','status':status}),flush=True)
        time.sleep(10)
def main():
    OUTPUT.mkdir(parents=True,exist_ok=True)
    if not REPO.exists():
        REPO.mkdir()
        subprocess.run(['git','init','-q'],cwd=REPO,check=True)
        (REPO/'duration.py').write_text('def parse_duration(text):\n    return int(text)\n')
        subprocess.run(['git','add','duration.py'],cwd=REPO,check=True)
        subprocess.run(['git','-c','user.name=Memory Demo','-c','user.email=demo@example.invalid','commit','-qm','Initial duration parser'],cwd=REPO,check=True)
    prompts=[
        'Implement parse_duration(text) in duration.py with a unittest suite. Repository policy: use only the Python standard library, return integer milliseconds, and raise ValueError for negative durations. Support suffixes ms and s. Run the tests and remember the policy and observed test result. Do not add repository instruction files.',
        'A project decision has changed: parse_duration must return integer MICROSECONDS now instead of milliseconds. Preserve the standard-library-only requirement and reject negative durations. Add us support and update ms/s scaling and tests. Run tests and retain this explicit correction. Do not add repository instruction files.',
        'Fix the duration parser so it accepts whitespace around input and rejects malformed strings with ValueError. Use repository memory to preserve prior API decisions. Expand and run the tests. Explain which remembered decision you verified against the code. Do not add repository instruction files.'
    ]
    for index,prompt in enumerate(prompts,1):
        artifact=OUTPUT/f'session-{index}.jsonl'
        if artifact.exists():
            events=[json.loads(line) for line in artifact.read_text().splitlines() if line.startswith('{')]
            if not any(e.get('type')=='agent_end' for e in events):raise RuntimeError('partial session artifact; inspect before retrying')
            ready();continue
        command=['pi','--provider','openai','--model','gpt-5.6-sol','--thinking','medium','--no-skills','--no-prompt-templates','--no-session','--mode','json','-e',str(ROOT/'integrations/pi/extension.ts'),'-p',prompt]
        with artifact.open('w') as out,(OUTPUT/f'session-{index}.stderr').open('w') as err:
            subprocess.run(command,cwd=REPO,env={**os.environ,'MEMORY_NAMESPACE':NAMESPACE,'MEMORY_SPOOL':str(OUTPUT/'spool')},stdout=out,stderr=err,check=True,timeout=900)
        ready()
    result=subprocess.run(['python3','-m','unittest','discover','-v'],cwd=REPO,capture_output=True,text=True)
    (OUTPUT/'verification.json').write_text(json.dumps({'exit_code':result.returncode,'stdout':result.stdout,'stderr':result.stderr},indent=2))
    if result.returncode:raise RuntimeError('demo tests failed')
    print('Three actual Pi sessions completed. Inspect JSONL, tool evidence and final tests; this is not an accuracy benchmark.')
if __name__=='__main__':main()
