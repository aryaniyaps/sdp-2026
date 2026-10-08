#!/usr/bin/env bash
set -euo pipefail
cd "$(dirname "$0")/.."
mode=${1:-up}
case "$mode" in up|prepare) ;; *) echo 'usage: run-memory.sh [up|prepare]' >&2; exit 2 ;; esac
if [ "$mode" = prepare ]; then
  (cd frontend && npm ci && npm run build)
else
  [ -x target/release/memory-engine ] || { echo 'Run ./scripts/run-memory.sh prepare while online first' >&2; exit 1; }
fi
if [ "$mode" = prepare ]; then
  docker compose pull postgres neo4j
  if ! curl -fsS --max-time 3 "${OLLAMA_URL:-http://127.0.0.1:11434}/api/version" >/dev/null; then
    docker compose --profile local-models pull ollama
  fi
fi
docker compose up -d --pull never --no-build postgres neo4j
for attempt in $(seq 1 60); do
  docker compose exec -T postgres pg_isready -U memory -d memory >/dev/null 2>&1 && break
  sleep 1
done
docker compose exec -T postgres pg_isready -U memory -d memory >/dev/null
if ! docker compose exec -T postgres psql -U memory -d postgres -tAc "SELECT 1 FROM pg_database WHERE datname='memory_app'" | rg -q 1; then
  docker compose exec -T postgres psql -U memory -d postgres -c 'CREATE DATABASE memory_app'
fi
export DATABASE_URL="${DATABASE_URL:-postgres://memory:memory@127.0.0.1:55432/memory_app}"
export NEO4J_URI="${NEO4J_URI:-http://127.0.0.1:7474}"
export OLLAMA_URL="${OLLAMA_URL:-http://127.0.0.1:11434}"
export EMBEDDING_MODEL="${EMBEDDING_MODEL:-qwen3-embedding:0.6b}"
# GPU=1 gives the Ollama container the NVIDIA GPU (needs the NVIDIA container toolkit). A host Ollama
# already answering on OLLAMA_URL is used as it is.
compose_files=(-f docker-compose.yml)
[ "${GPU:-0}" = 1 ] && compose_files+=(-f docker-compose.gpu.yml)
if ! curl -fsS --max-time 3 "$OLLAMA_URL/api/version" >/dev/null; then
  docker compose "${compose_files[@]}" --profile local-models up -d --pull never --no-build ollama
  for attempt in $(seq 1 60); do
    curl -fsS --max-time 3 "$OLLAMA_URL/api/version" >/dev/null && break
    sleep 1
  done
  curl -fsS --max-time 3 "$OLLAMA_URL/api/version" >/dev/null
fi
if ! curl -fsS "$OLLAMA_URL/api/show" -d "{\"model\":\"$EMBEDDING_MODEL\"}" >/dev/null; then
  [ "$mode" = prepare ] || { echo "Missing $EMBEDDING_MODEL; run prepare while online" >&2; exit 1; }
  curl -fsS "$OLLAMA_URL/api/pull" -d "{\"model\":\"$EMBEDDING_MODEL\",\"stream\":false}" >/dev/null
fi
# The memory worker is the fine-tuned student model. It is installed once from the release (about 4.3 GB)
# into the Ollama that serves it, which can be another machine: set EXTRACTION_OLLAMA_URL.
export EXTRACTION_MODEL="${EXTRACTION_MODEL:-mem-extractor}"
export EXTRACTION_OLLAMA_URL="${EXTRACTION_OLLAMA_URL:-$OLLAMA_URL}"
if [ "$mode" = prepare ]; then
  OLLAMA_URL="$EXTRACTION_OLLAMA_URL" scripts/fetch-slm.sh
else
  curl -fsS "$EXTRACTION_OLLAMA_URL/api/show" -d "{\"model\":\"$EXTRACTION_MODEL\"}" >/dev/null || {
    echo "Missing worker $EXTRACTION_MODEL; run prepare while online" >&2; exit 1;
  }
fi
if [ "$mode" = prepare ]; then
# Host cargo is used when it is the pinned toolchain. Building in Docker leaves root-owned files in target/.
wanted=$(sed -n 's/^channel *= *"\(.*\)"/\1/p' rust-toolchain.toml)
if command -v cargo >/dev/null && [ "$(cargo --version | awk '{print $2}')" = "$wanted" ]; then
  echo "Building with host cargo $wanted"
  cargo build --release --locked
else
  echo "Building in Docker (host cargo is not $wanted)"
  docker run --rm -v sdp_cargo:/usr/local/cargo -v "$PWD:/app" -w /app rust:1.96-bookworm cargo build --release --locked
fi
fi
echo "Worker: ollama/$EXTRACTION_MODEL at $EXTRACTION_OLLAMA_URL"
echo "Memory API and observatory: http://${BIND_ADDR:-127.0.0.1:8080}"
exec target/release/memory-engine
