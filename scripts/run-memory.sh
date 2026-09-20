#!/usr/bin/env bash
set -euo pipefail
cd "$(dirname "$0")/.."
command -v pi >/dev/null || { echo 'Pi is required for the subscription memory worker.'; exit 1; }
docker compose up -d postgres neo4j
until docker compose exec -T postgres pg_isready -U memory -d memory >/dev/null 2>&1; do sleep 1; done
if ! docker compose exec -T postgres psql -U memory -d postgres -tAc "SELECT 1 FROM pg_database WHERE datname='memory_app'" | rg -q 1; then
  docker compose exec -T postgres psql -U memory -d postgres -c 'CREATE DATABASE memory_app'
fi
export DATABASE_URL="${DATABASE_URL:-postgres://memory:memory@127.0.0.1:55432/memory_app}"
export NEO4J_URI="${NEO4J_URI:-http://127.0.0.1:7474}"
export PI_PROVIDER="${PI_PROVIDER:-openai}"
export PI_MODEL="${PI_MODEL:-gpt-5.6-sol}"
export OLLAMA_URL="${OLLAMA_URL:-http://127.0.0.1:11434}"
export EMBEDDING_MODEL="${EMBEDDING_MODEL:-qwen3-embedding:0.6b}"
if ! curl -fsS --max-time 3 "$OLLAMA_URL/api/version" >/dev/null; then
  docker compose --profile local-models up -d ollama
  until curl -fsS --max-time 3 "$OLLAMA_URL/api/version" >/dev/null; do sleep 1; done
fi
curl -fsS "$OLLAMA_URL/api/pull" -H 'content-type: application/json' -d '{"model":"qwen3-embedding:0.6b","stream":false}' >/dev/null
docker run --rm -v sdp_cargo:/usr/local/cargo -v "$PWD:/app" -w /app rust:1.96-bookworm cargo build --locked
echo 'Memory API and observatory: http://127.0.0.1:8080'
exec target/debug/memory-engine
