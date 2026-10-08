#!/usr/bin/env bash
# Upload only a checksum-verified directory created by stage_release.py.
# The repository stays private until a separate remote-verification/publication step.
# Usage: slm-distill/release.sh STAGED_DIRECTORY [--verify-only]
set -euo pipefail
repo_root=$(cd "$(dirname "$0")/.." && pwd)
exec env -u PYTHONPATH "${SLM_PYTHON:-$repo_root/slm-distill/.venv/bin/python}" \
  "$repo_root/slm-distill/publish_release.py" "$@"
