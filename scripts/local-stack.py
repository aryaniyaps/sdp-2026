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


def run(*args, capture=False):
    return subprocess.run(args, check=True, text=True, stdout=subprocess.PIPE if capture else None).stdout


def dc(*args, capture=False):
    return run(*compose, *args, capture=capture)


def request(url, payload=None, timeout=10):
    args = ['exec', '-T', 'engine', 'curl', '-fsS', '--max-time', str(timeout), url]
    if payload is not None:
        args += ['-H', 'content-type: application/json', '-d', json.dumps(payload)]
    return json.loads(dc(*args, capture=True))


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
        try:
            request(url + '/api/show', {'model': model})
        except subprocess.CalledProcessError:
            raise RuntimeError(f'Cannot load installed model {model}; start its local Ollama or reinstall it while online') from None
        print('Installed locally: ' + model, flush=True)
    ports = services['caddy']['ports']
    port = next(str(p['published']) for p in ports if p['target'] == 80)
    code = (state / 'access.code').read_text().strip()
    for path in ['/demo', '/term/', '/api/v2/projects']:
        for attempt in range(15):
            try:
                run('curl', '-fsS', '--max-time', '5', '-o', '/dev/null', '-u', 'reviewer:' + code,
                    f'http://127.0.0.1:{port}{path}')
                break
            except subprocess.CalledProcessError:
                time.sleep(2)
        else:
            raise RuntimeError(f'Browser endpoint {path} did not become ready on port {port}')
    print(f'Ready: http://127.0.0.1:{port}/ (user reviewer; access code: {state / "access.code"})')
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
