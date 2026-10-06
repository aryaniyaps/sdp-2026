#!/usr/bin/env bash
# Wait for the weights, then train, then evaluate. Safe to re-run: each stage
# checks whether its output already exists.
set -u
cd "$(dirname "$0")"
PY="$PWD/.venv/bin/python"
step() { echo "[$(date +%H:%M:%S)] $*"; }

# ------------------------------------------------- wait for a complete model
step "waiting for Qwen3-1.7B weights"
for i in $(seq 1 240); do
  if "$PY" - <<'PY' 2>/dev/null
from huggingface_hub import snapshot_download
snapshot_download("Qwen/Qwen3-1.7B",
                  allow_patterns=["*.json","*.safetensors","*.txt","tokenizer*"],
                  local_files_only=True)
PY
  then step "weights complete"; break; fi
  sleep 15
done
"$PY" - <<'PY' || { echo "FATAL: weights still incomplete"; exit 1; }
from huggingface_hub import snapshot_download
p = snapshot_download("Qwen/Qwen3-1.7B",
                      allow_patterns=["*.json","*.safetensors","*.txt","tokenizer*"],
                      local_files_only=True)
print("  at", p)
PY

# ------------------------------------------------------------------- train
if [ -d "out/qwen3-1.7b-memex-lora/final" ]; then
  step "adapter already exists, skipping training"
else
  step "TRAINING"
  "$PY" train_lora.py --epochs 2 2>&1 | grep -v -e UserWarning -e "cpu = _conv"
  [ -d "out/qwen3-1.7b-memex-lora/final" ] || { echo "FATAL: training produced no adapter"; exit 1; }
fi

# -------------------------------------------------------------------- eval
step "EVALUATING (base zero-shot vs distilled, 142 held-out windows)"
"$PY" eval.py 2>&1 | grep -v -e UserWarning -e "cpu = _conv"
step "PIPELINE COMPLETE"
