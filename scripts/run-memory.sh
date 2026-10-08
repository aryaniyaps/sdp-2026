#!/usr/bin/env bash
set -euo pipefail
cd "$(dirname "$0")/.."
(cd frontend && npm ci && npm run build)
docker compose up -d postgres neo4j
until docker compose exec -T postgres pg_isready -U memory -d memory >/dev/null 2>&1; do sleep 1; done
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
  docker compose "${compose_files[@]}" --profile local-models up -d ollama
  until curl -fsS --max-time 3 "$OLLAMA_URL/api/version" >/dev/null; do sleep 1; done
fi
curl -fsS "$OLLAMA_URL/api/pull" -H 'content-type: application/json' -d '{"model":"qwen3-embedding:0.6b","stream":false}' >/dev/null
# The memory worker is the fine-tuned student model. It is installed once from the release (about 4.3 GB)
# into the Ollama that serves it, which can be another machine: set EXTRACTION_OLLAMA_URL.
export EXTRACTION_MODEL="${EXTRACTION_MODEL:-mem-extractor}"
export EXTRACTION_OLLAMA_URL="${EXTRACTION_OLLAMA_URL:-$OLLAMA_URL}"
OLLAMA_URL="$EXTRACTION_OLLAMA_URL" scripts/fetch-slm.sh
# Host cargo is used when it is the pinned toolchain. Building in Docker leaves root-owned files in target/.
wanted=$(sed -n 's/^channel *= *"\(.*\)"/\1/p' rust-toolchain.toml)
if command -v cargo >/dev/null && [ "$(cargo --version | awk '{print $2}')" = "$wanted" ]; then
  echo "Building with host cargo $wanted"
  cargo build --release --locked
else
  echo "Building in Docker (host cargo is not $wanted)"
  docker run --rm -v sdp_cargo:/usr/local/cargo -v "$PWD:/app" -w /app rust:1.96-bookworm cargo build --release --locked
fi
echo "Worker: ollama/$EXTRACTION_MODEL at $EXTRACTION_OLLAMA_URL"
echo 'Memory API and observatory: http://127.0.0.1:8080'
exec target/release/memory-engine
