"""Upload an explicit stage_release.py directory; never publish an arbitrary checkout."""
import argparse
import hashlib
import json
import re
from pathlib import Path

REQUIRED = {
    'README.md', 'LICENSE', 'Modelfile', 'release-receipt.json',
    'mem-extractor-q8_0.gguf', 'adapter/adapter_config.json',
    'adapter/adapter_model.safetensors', 'inference/prompt.py',
    'inference/structured.py', 'inference/run.py',
    'src/model/extract_schema.json', 'src/model/consolidate_schema.json',
    'src/model/paired_sources.txt', 'src/worker/worker_system.txt',
    'src/worker/extract_prompt.txt', 'src/worker/consolidate_prompt.txt',
    'src/v2/reflect_prompt.txt', 'documentation/PROMPT_DESIGN.md',
    'documentation/RESEARCH_PROTOCOL.md',
    'documentation/Finetuning_Iteration_Report.ipynb',
}


def verify(directory):
    directory = directory.resolve(strict=True)
    manifest = directory / 'SHA256SUMS'
    listed = {}
    for line in manifest.read_text().splitlines():
        match = re.fullmatch(r'([0-9a-f]{64})  (.+)', line)
        if not match or match[2] in listed:
            raise ValueError('Malformed or duplicate checksum manifest entry')
        listed[match[2]] = match[1]
    if set(listed) != REQUIRED:
        raise ValueError('Staging manifest differs from the explicit public-file allowlist')
    actual = {str(p.relative_to(directory)) for p in directory.rglob('*') if p.is_file()}
    if actual != REQUIRED | {'SHA256SUMS'}:
        raise ValueError('Unlisted files in staging directory; restage into a clean directory')
    if any(p.is_symlink() for p in directory.rglob('*')):
        raise ValueError('Staging symlinks are not allowed')
    for name, expected in listed.items():
        h = hashlib.sha256()
        with (directory / name).open('rb') as f:
            for chunk in iter(lambda: f.read(8 * 1024 * 1024), b''):
                h.update(chunk)
        if h.hexdigest() != expected:
            raise ValueError(f'Checksum mismatch: {name}')
    return directory, sorted(REQUIRED | {'SHA256SUMS'})


def main():
    p = argparse.ArgumentParser(description=__doc__)
    p.add_argument('directory', type=Path)
    p.add_argument('--repo', default='aryaniyaps/mem-extractor')
    p.add_argument('--verify-only', action='store_true')
    a = p.parse_args()
    directory, files = verify(a.directory)
    if a.verify_only:
        print(json.dumps({'verified': True, 'files': len(files), 'directory': str(directory)}))
        return
    from huggingface_hub import HfApi
    api = HfApi()
    api.create_repo(a.repo, repo_type='model', private=True, exist_ok=True)
    if not api.repo_info(a.repo, repo_type='model').private:
        raise ValueError('Expected a private staging repository; verify publication authority separately')
    result = api.upload_folder(repo_id=a.repo, repo_type='model', folder_path=str(directory),
                               allow_patterns=files,
                               commit_message='Upload verified mem-extractor release artifacts')
    print(json.dumps({'repo': a.repo, 'commit': result.oid, 'url': result.commit_url,
                      'private': True, 'next': 'Verify remote artifact hashes before making public.'}, indent=2))


if __name__ == '__main__':
    main()
