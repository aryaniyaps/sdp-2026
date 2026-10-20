#!/usr/bin/env bash
# From the end of stage C training to a finished test run, unattended:
#   1. export the stage C adapter as mem-extractor:general-v2,
#   2. rebuild the two dev conversations with v1 and with v2 and evaluate both on the same code,
#   3. choose the worker by the rule fixed before either result was seen (v2 if its dev enhanced accuracy
#      is at least v1's), record the decision and commit the configuration,
#   4. run the full test benchmark (benchmark/overnight.sh).
# Progress: benchmark/runs/pipeline-v2.status. Every step skips work already on disk.
set -uo pipefail
cd "$(dirname "$0")/.."
ROOT=$PWD
STATUS=benchmark/runs/pipeline-v2.status
LOG=benchmark/runs/pipeline-v2.log
DISTILL=${DISTILL:-$HOME/orca/sdp-2026/slm-distill}
PY=$DISTILL/.venv/bin/python
ADAPTER=$ROOT/slm-distill/out/local-4b-coverage/final
V1=mem-extractor:general-v1
V2=mem-extractor:general-v2
DEV="enhanced,assertions_only,raw_hybrid"
note() { echo "$(date -u +%FT%TZ) $*" | tee -a "$STATUS"; }
fail() { note "FAILED: $*"; exit 1; }

restart_engine() {  # $1: worker model
  local pid
  pid=$(ss -ltnp | grep 127.0.0.1:8090 | grep -o 'pid=[0-9]*' | cut -d= -f2)
  [ -n "$pid" ] && kill "$pid" && sleep 3
  WORKER_MODEL=$1 setsid nohup benchmark/engine.sh >> benchmark/runs/engine.log 2>&1 < /dev/null &
  for _ in $(seq 1 120); do
    curl -fsS -m 3 127.0.0.1:8090/healthz 2>/dev/null | grep -q "\"ollama/$1\"" && return 0
    sleep 5
  done
  fail "engine did not come up with $1"
}

note "pipeline starting at commit $(git rev-parse --short HEAD)"
while pgrep -f "train.py --qlora --eval-on-epoch" >/dev/null; do sleep 60; done
[ -f "$ADAPTER/adapter_config.json" ] || fail "training left no adapter at $ADAPTER"
note "training finished: $(grep -aoE "'eval_loss': '[0-9.]+'" slm-distill/data/research-v4/train.log | tail -1)"

if ! curl -fsS 127.0.0.1:11434/api/show -d "{\"model\":\"$V2\"}" >/dev/null 2>&1; then
  note "exporting $V2"
  (cd "$DISTILL" && HF_HUB_OFFLINE=1 "$PY" export.py --base Qwen/Qwen3-4B-Instruct-2507 \
     --revision cdbee75f17c01a7cc42f958dc650907174af0554 --adapter "$ADAPTER" --name "$V2" --quant q8_0 \
     --gguf-out "$ROOT/slm-distill/out/local-4b-coverage/mem-extractor-q8_0.gguf") >> "$LOG" 2>&1 \
     || fail "export, see $LOG"
fi
note "$V2 served; GGUF sha256 $(sha256sum slm-distill/out/local-4b-coverage/mem-extractor-q8_0.gguf | cut -c1-16)"

for model in "$V1" "$V2"; do
  run=locomo-dev-v3-${model##*:}
  restart_engine "$model"
  note "dev $run: building memory"
  for step in ingest recall answer judge; do
    extra=""; [ "$step" = judge ] && extra="--judges jev"
    for attempt in 1 2 3; do
      BENCH_WORKER_MODEL=$model python3 benchmark/run.py "$step" --split dev --run "$run" --conditions "$DEV" $extra \
        >> "$LOG" 2>&1 && break
      note "dev $run $step attempt $attempt failed"; sleep 60
      [ "$attempt" = 3 ] && fail "dev $run $step"
    done
  done
  BENCH_WORKER_MODEL=$model python3 benchmark/run.py report --split dev --run "$run" > /dev/null 2>&1
  note "dev $run done"
done

choice=$(python3 - <<'EOF'
import json, subprocess, sys
sys.path.insert(0, "benchmark")
acc = {}
for v in ("general-v1", "general-v2"):
    r = json.load(open(f"benchmark/runs/locomo-dev-v3-{v}/report.json"))
    acc[v] = {c: r["conditions"][c]["jev_accuracy"] for c in ("enhanced", "assertions_only", "raw_hybrid")}
# Rule fixed before either result existed: v2 replaces v1 if its dev enhanced accuracy is at least v1's.
chosen = "general-v2" if acc["general-v2"]["enhanced"] >= acc["general-v1"]["enhanced"] else "general-v1"
p = "benchmark/iterations.json"; d = json.load(open(p))
d["iterations"].append({
    "version": "v6-extractor-coverage",
    "date": "2026-10-21",
    "dev_run": "locomo-dev-v3-general-v1 and locomo-dev-v3-general-v2",
    "finding": "Extraction coverage was the main fault of the v1 worker (iteration v5): its training episodes asked for two or three claims each.",
    "change": "Stage C QLoRA from the released adapter on 1,119 new coverage rows (480 two-person conversations labelled with every durable fact by Claude Sonnet 5.5, slm-distill/expand_coverage_corpus.py) plus 600 replayed stage B rows, training cap raised from 4,096 to 6,144 tokens so the new rows fit. Rule fixed in benchmark/pipeline-v2.sh before the comparison: v2 is used if its dev enhanced accuracy is at least v1's.",
    "files": ["slm-distill/expand_coverage_corpus.py", "slm-distill/assemble_coverage_sets.py", "benchmark/pipeline-v2.sh"],
    "dev_before": {"worker": "general-v1", **acc["general-v1"]},
    "dev_after": {"worker": "general-v2", **acc["general-v2"]},
    "decision": f"mem-extractor:{chosen}",
})
open(p, "w").write(json.dumps(d, indent=2) + "\n")
c = json.load(open("benchmark/config.json"))
c["engine"]["worker_model"] = f"mem-extractor:{chosen}"
open("benchmark/config.json", "w").write(json.dumps(c, indent=2) + "\n")
print(chosen, json.dumps(acc))
EOF
) || fail "comparing dev runs"
note "decision: $choice"

git add benchmark/config.json benchmark/iterations.json
git commit -q -F - <<EOF || fail "commit"
Freeze the test configuration after the extractor comparison

Dev comparison of the v1 and stage C (v2) workers on the same code chose
mem-extractor:${choice%% *}; numbers are in benchmark/iterations.json.

Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>
EOF
note "committed $(git rev-parse --short HEAD)"

restart_engine "mem-extractor:${choice%% *}"
note "starting the test run"
benchmark/overnight.sh locomo-final
note "pipeline finished"
