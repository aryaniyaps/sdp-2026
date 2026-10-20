#!/usr/bin/env bash
# Runs the memory engine for the benchmark: its own Postgres and Neo4j (benchmark/docker-compose.bench.yml),
# the GPU embedder, and the worker named in benchmark/config.json. Nothing is read from .env.
set -euo pipefail
cd "$(dirname "$0")/.."
config() { python3 -c "import json,sys;print(json.load(open('benchmark/config.json'))['engine'][sys.argv[1]])" "$1"; }
export CARGO_TARGET_DIR="${CARGO_TARGET_DIR:-$HOME/.cache/sdp-2026-target}"
docker compose -p sdp-bench -f benchmark/docker-compose.bench.yml up -d --wait >/dev/null
[ -f frontend/dist/index.html ] || (cd frontend && npm ci && npm run build)
cargo build --release --locked
for model in "$(config worker_model)" "$(config embedding_model)"; do
  curl -fsS http://127.0.0.1:11434/api/show -d "{\"model\":\"$model\"}" >/dev/null || {
    echo "Ollama is missing $model; run scripts/fetch-slm.sh (worker) or ollama pull (embedder)" >&2; exit 1; }
done
mkdir -p benchmark/runs
exec env -i HOME="$HOME" PATH="$PATH" \
  DATABASE_URL=postgres://memory:memory@127.0.0.1:56432/memory \
  NEO4J_URI=http://127.0.0.1:17474 NEO4J_USER=neo4j NEO4J_PASSWORD=password \
  OLLAMA_URL=http://127.0.0.1:11434 EXTRACTION_OLLAMA_URL=http://127.0.0.1:11434 \
  EXTRACTION_MODEL="$(config worker_model)" EMBEDDING_MODEL="$(config embedding_model)" \
  OLLAMA_NUM_CTX="$(config num_ctx)" OLLAMA_NUM_PREDICT="$(config num_predict)" \
  MEMORY_WORKER_CONCURRENCY="$(config worker_concurrency)" BIND_ADDR="$(config bind)" \
  "$CARGO_TARGET_DIR/release/memory-engine"
