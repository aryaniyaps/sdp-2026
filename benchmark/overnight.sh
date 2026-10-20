#!/usr/bin/env bash
# Runs the whole test benchmark unattended. Every stage writes one file per unit of work and skips what
# is already on disk, so a failed or interrupted stage is simply run again. The engine is restarted when
# its health check fails. Progress: benchmark/runs/<run>.status and benchmark/runs/<run>.log.
#
#   benchmark/overnight.sh [run-id]        (default locomo-test)
set -uo pipefail
cd "$(dirname "$0")/.."
RUN=${1:-locomo-test}
PROPS=${PROPS:-properties-test}
LOG=benchmark/runs/$RUN.log
STATUS=benchmark/runs/$RUN.status
BASE=http://$(python3 -c "import json;print(json.load(open('benchmark/config.json'))['engine']['bind'])")
mkdir -p benchmark/runs

# Keep the laptop awake for the whole run.
if [ -z "${INHIBITED:-}" ] && command -v systemd-inhibit >/dev/null; then
  exec env INHIBITED=1 systemd-inhibit --what=sleep:idle:handle-lid-switch --who=sdp-benchmark \
    --why="memory benchmark $RUN" "$0" "$@"
fi

note() { echo "$(date -u +%FT%TZ) $*" | tee -a "$STATUS"; }

ensure_engine() {
  curl -fsS --max-time 10 "$BASE/healthz" >/dev/null 2>&1 && return 0
  note "engine down; starting benchmark/engine.sh"
  nohup benchmark/engine.sh >> benchmark/runs/engine.log 2>&1 &
  for _ in $(seq 1 120); do
    curl -fsS --max-time 5 "$BASE/healthz" >/dev/null 2>&1 && { note "engine up"; return 0; }
    sleep 5
  done
  note "engine did not come up"
  return 1
}

# stage NAME COMMAND...: retried until it succeeds, up to 20 times, two minutes apart.
stage() {
  local name=$1; shift
  for attempt in $(seq 1 20); do
    ensure_engine || { sleep 120; continue; }
    note "start $name (attempt $attempt)"
    if "$@" >> "$LOG" 2>&1; then note "done $name"; return 0; fi
    note "failed $name (attempt $attempt); see $LOG"
    sleep 120
  done
  note "GAVE UP $name"
  return 1
}

bench() { python3 benchmark/run.py "$@" --split test --run "$RUN"; }

openai_key_works() {
  python3 - <<'EOF'
import sys, urllib.request
sys.path.insert(0, "benchmark")
import run
try:
    key = run.secret("OPENAI_API_KEY")
    urllib.request.urlopen(urllib.request.Request("https://api.openai.com/v1/models/gpt-4o-mini",
                           headers={"authorization": f"Bearer {key}"}), timeout=30)
except BaseException as err:
    print(f"OpenAI key unusable: {err}")
    sys.exit(1)
EOF
}

note "overnight run $RUN starting at commit $(git rev-parse --short HEAD)"
python3 benchmark/run.py fetch >> "$LOG" 2>&1 || { note "fetch failed"; exit 1; }
if [ ! -d "benchmark/runs/$RUN/ingestion" ]; then
  # A fresh run builds every bank with the model, not from answers cached by dev runs, so its
  # ingestion times are real. A resumed run keeps the cache and its progress.
  rows=$(docker exec sdp-bench-postgres-1 psql -U memory -d memory -tAc "WITH d AS (DELETE FROM model_cache RETURNING 1) SELECT count(*) FROM d")
  note "cleared $rows cached model answers before building memory"
fi
stage ingest bench ingest || exit 1
stage recall bench recall || exit 1

# The property suite needs the GPU; the reader does not, so they run side by side.
( stage properties python3 benchmark/properties.py run --run "$PROPS" --reader ) &
PROPS_PID=$!

for repeat in 0 1 2; do
  stage "answer r$repeat" bench answer --repeat "$repeat"
  stage "judge jev r$repeat" bench judge --judges jev --repeat "$repeat"
done
if openai_key_works >> "$LOG" 2>&1; then
  for repeat in 0 1 2; do
    stage "judge gpt4omini r$repeat" bench judge --judges gpt4omini --repeat "$repeat"
  done
else
  note "SKIPPED gpt-4o-mini judge: OPENAI_API_KEY is not a working OpenAI key; rerun 'bench judge --judges gpt4omini' later"
fi

wait "$PROPS_PID"
python3 benchmark/properties.py report --run "$PROPS" >> "$LOG" 2>&1
stage report python3 benchmark/run.py report --split test --run "$RUN"
stage verify python3 benchmark/run.py verify --split test --run "$RUN"
(cd benchmark && uv run python build_notebook.py --run "$RUN" --properties "$PROPS" --execute >> "../$LOG" 2>&1) \
  && note "notebook built" || note "notebook build failed; see $LOG"
note "overnight run $RUN finished"
