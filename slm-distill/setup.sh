#!/usr/bin/env bash
# Creates slm-distill/.venv with the pinned packages, fetches the pinned llama.cpp converter and
# checks that the GPU stack works. Needs uv, Python 3.12 and an NVIDIA GPU with a recent driver.
# torch comes from PyPI: its wheel bundles CUDA 13, which has kernels for Blackwell (sm_120).
set -euo pipefail
cd "$(dirname "$0")"
command -v uv > /dev/null || { echo "uv is required: https://docs.astral.sh/uv/" >&2; exit 1; }
[ -d .venv ] || uv venv --python 3.12 .venv
VIRTUAL_ENV="$PWD/.venv" uv pip install -r requirements.txt
tools/fetch-llamacpp.sh
VIRTUAL_ENV="$PWD/.venv" uv pip install -e .tools/llama.cpp/gguf-py
.venv/bin/python - <<'PY'
import torch
assert torch.cuda.is_available(), "no CUDA device visible to torch"
print("torch", torch.__version__, "cuda", torch.version.cuda, "|", torch.cuda.get_device_name(0))
x = torch.randn(1024, 1024, device="cuda", dtype=torch.bfloat16)
assert bool(torch.isfinite(x @ x).all()), "bf16 matmul failed"
print("bf16 matmul ok")
PY
.venv/bin/python -m pytest -q --ignore=v1 .
echo "setup complete: use .venv/bin/python"
