#!/usr/bin/env python3
"""Start/check the prepared browser stack without builds, pulls or config regeneration."""
import json
import os
from pathlib import Path
import subprocess
import sys
import time

state = Path(os.environ.get('SDP_LOCAL_STATE', Path(os.environ.get('XDG_STATE_HOME', Path.home() / '.local/state')) / 'sdp-hosted-local'))
compose = ['docker', 'compose', '-p', os.environ.get('SDP_LOCAL_PROJECT', 'sdp-memory-local'), '--project-directory', str(state)]


def run(*args, capture=False, stdin=None):
    return subprocess.run(args, check=True, text=True, input=stdin, stdout=subprocess.PIPE if capture else None).stdout


def dc(*args, capture=False, stdin=None):
    return run(*compose, *args, capture=capture, stdin=stdin)


def request(url, payload=None, timeout=10):
    args = ['exec', '-T', 'engine', 'curl', '-fsS', '--max-time', str(timeout), url]
    if payload is not None:
        args += ['-H', 'content-type: application/json', '-d', json.dumps(payload)]
    return json.loads(dc(*args, capture=True))


REPAIR_SQL = """
-- A worker renews its 15 minute lease every 30s. A running job silent for a minute belongs to a dead engine
-- (reboot, crash, restart); expire it so it is picked up now instead of after the lease runs out.
UPDATE memory_namespace_leases SET lease_until = now() - interval '1 second'
 WHERE job_id IN (SELECT id FROM memory_jobs WHERE status = 'running' AND lease_until < now() + interval '14 minutes');
UPDATE memory_jobs SET lease_until = now() - interval '1 second'
 WHERE status = 'running' AND lease_until > now() AND lease_until < now() + interval '14 minutes';
DELETE FROM memory_namespace_leases l WHERE NOT EXISTS
 (SELECT 1 FROM memory_jobs j WHERE j.id = l.job_id AND j.status = 'running');
-- A failed job blocks every later job of its namespace; give recent ones a fresh set of attempts.
UPDATE memory_jobs SET status = 'pending', attempts = 0, available_at = now(), error = NULL
 WHERE status = 'failed' AND created_at > now() - interval '7 days';
"""


def repair_queue():
    """Unstick the job queue after a reboot or crash. Never fatal: startup continues if it cannot run."""
    try:
        dc('exec', '-T', 'postgres', 'sh', '-c', 'psql -q -v ON_ERROR_STOP=1 -U "$POSTGRES_USER" "$POSTGRES_DB"',
           capture=True, stdin=REPAIR_SQL)
        print('Job queue checked: stale leases released, failed jobs re-queued', flush=True)
    except (subprocess.CalledProcessError, OSError):
        print('Warning: could not check the job queue', flush=True)


def warm_worker_model(url, model, num_ctx):
    """Load the worker model now, so the first extraction of the demo does not pay a cold load."""
    try:
        request(url + '/api/generate', {'model': model, 'keep_alive': '1h', 'options': {'num_ctx': num_ctx}}, timeout=240)
        print('Worker model loaded: ' + model, flush=True)
    except (subprocess.CalledProcessError, ValueError):
        print('Warning: could not preload ' + model + '; the first extraction will load it', flush=True)


def main():
    mode = sys.argv[1] if len(sys.argv) > 1 else 'up'
    if not (state / 'docker-compose.yml').is_file():
        raise RuntimeError('No prepared stack. While online run ./deploy/azure/run-local.sh prepare')
    config = json.loads(dc('config', '--format', 'json', capture=True))
    services = config['services']
    active = [name for name, service in services.items()
              if not service.get('profiles') and name != 'ollama-init']
    for name in active:
        image = services[name]['image']
        try:
            run('docker', 'image', 'inspect', image, capture=True)
        except subprocess.CalledProcessError:
            raise RuntimeError(f'Missing local image {image}; run prepare while online') from None
    if mode == 'up':
        dc('up', '-d', '--pull', 'never', '--no-build', *active)
    elif mode != 'check':
        raise RuntimeError('usage: local-stack.py [up|check]')
    for attempt in range(60):
        try:
            health = request('http://127.0.0.1:8080/healthz')
            if health.get('database') and health.get('embedder') and not health.get('degraded_mode'):
                break
        except (subprocess.CalledProcessError, ValueError):
            pass
        time.sleep(2)
    else:
        raise RuntimeError('Memory API is not healthy; inspect docker compose logs in ' + str(state))
    env = services['engine']['environment']
    ollama = env.get('OLLAMA_URL', 'http://ollama:11434').rstrip('/')
    worker_url = env.get('EXTRACTION_OLLAMA_URL', ollama).rstrip('/')
    for url, model in [(ollama, env.get('EMBEDDING_MODEL', 'qwen3-embedding:0.6b')),
                       (worker_url, env.get('EXTRACTION_MODEL', 'mem-extractor'))]:
        for attempt in range(30):  # Ollama may still be starting right after a boot
            try:
                request(url + '/api/show', {'model': model})
                break
            except subprocess.CalledProcessError:
                time.sleep(2)
        else:
            raise RuntimeError(f'Cannot load installed model {model}; start its local Ollama or reinstall it while online') from None
        print('Installed locally: ' + model, flush=True)
    repair_queue()
    warm_worker_model(worker_url, env.get('EXTRACTION_MODEL', 'mem-extractor'), health.get('worker_num_ctx') or 12288)
    ports = services['caddy']['ports']
    port = next(str(p['published']) for p in ports if p['target'] == 80)
    entry = f'http://127.0.0.1:{port}/'
    redirect = run('curl', '-sS', '--max-time', '5', '-o', '/dev/null',
                   '-w', '%{http_code} %{redirect_url}', entry, capture=True)
    if redirect != f'302 {entry}demo':
        raise RuntimeError('Dashboard root must redirect to /demo on port ' + port)
    for path in ['/demo', '/term/', '/api/v2/projects']:
        for attempt in range(15):
            try:
                run('curl', '-fsS', '--max-time', '5', '-o', '/dev/null',
                    f'http://127.0.0.1:{port}{path}')
                break
            except subprocess.CalledProcessError:
                time.sleep(2)
        else:
            raise RuntimeError(f'Browser endpoint {path} did not become ready on port {port}')
    print(f'Ready: http://127.0.0.1:{port}/')
    print('No builds or downloads performed. Pi provider requests still require internet.')


if __name__ == '__main__':
    try:
        main()
    except (RuntimeError, subprocess.CalledProcessError, OSError, KeyError) as exc:
        # Do not print commands/config: they can contain credentials.
        print(f'Local startup/check failed ({type(exc).__name__}). Check local images, Ollama models and stack logs.', file=sys.stderr)
        if isinstance(exc, RuntimeError):
            print(str(exc), file=sys.stderr)
        sys.exit(1)
