#!/usr/bin/env bash
# Resilient setup. Retries and verifies the real condition (can we import it),
# never a pipeline exit code.
#
# torch comes from PyPI, NOT download.pytorch.org: measured 3.5 MB/s vs 281 B/s
# on this link. PyPI's torch wheel bundles CUDA 12.8, which has sm_120 kernels
# for Blackwell. The verify step at the bottom proves that rather than assuming.
set -u
cd "$(dirname "$0")"
VENV="$PWD/.venv"
PY="$VENV/bin/python"
MODEL="${MODEL:-Qwen/Qwen3-1.7B}"
export UV_HTTP_TIMEOUT=600
export VIRTUAL_ENV="$VENV"

have() { "$PY" -c "import $1" 2>/dev/null; }
step() { echo "[$(date +%H:%M:%S)] $*"; }

# ---------------------------------------------------------------- 1. torch
for i in $(seq 1 20); do
  if have torch; then break; fi
  step "torch attempt $i (pypi)"
  uv pip install torch >/dev/null 2>&1
  have torch || sleep 8
done
if ! have torch; then step "FATAL: torch missing after 20 attempts"; exit 1; fi
step "torch ok: $("$PY" -c 'import torch;print(torch.__version__, torch.version.cuda)')"

# ------------------------------------------------------- 2. training stack
for i in $(seq 1 20); do
  if have transformers && have peft && have trl && have datasets \
     && have accelerate && have bitsandbytes; then break; fi
  step "stack attempt $i"
  uv pip install transformers peft trl datasets accelerate bitsandbytes \
      openai "huggingface_hub[hf_transfer]" >/dev/null 2>&1
  sleep 5
done
have transformers || { step "FATAL: transformers missing"; exit 1; }
step "stack ok"

# ------------------------------------------------------------ 3. base model
step "downloading $MODEL"
MODEL="$MODEL" "$PY" - <<'PY'
import os, time
from huggingface_hub import snapshot_download
m = os.environ["MODEL"]
for i in range(40):
    try:
        p = snapshot_download(m, allow_patterns=["*.json", "*.safetensors", "*.txt",
                                                 "tokenizer*"], max_workers=4)
        print("MODEL AT", p); break
    except Exception as e:
        print(f"  attempt {i+1}: {str(e)[:100]}", flush=True); time.sleep(8)
else:
    raise SystemExit("FATAL: model download failed")
PY

# ---------------------------------------------------------------- 4. verify
step "VERIFYING GPU STACK"
"$PY" - <<'PY'
import torch
print("torch", torch.__version__, "| cuda", torch.version.cuda)
print("arch list:", torch.cuda.get_arch_list())
print("sm_120 present:", "sm_120" in torch.cuda.get_arch_list())
print("device:", torch.cuda.get_device_name(0))
x = torch.randn(1024, 1024, device="cuda", dtype=torch.bfloat16)
print("bf16 matmul ok:", bool(torch.isfinite(x @ x).all().item()))
try:
    import bitsandbytes as bnb
    from bitsandbytes.nn import Linear4bit
    l = Linear4bit(256, 256, compute_dtype=torch.bfloat16).cuda()
    y = l(torch.randn(4, 256, device="cuda", dtype=torch.bfloat16))
    print("bitsandbytes 4-bit on sm_120 ok:", bool(torch.isfinite(y).all().item()))
except Exception as e:
    print("bitsandbytes 4-bit FAILED -> fall back to bf16 LoRA:", str(e)[:160])
PY
step "SETUP COMPLETE"
