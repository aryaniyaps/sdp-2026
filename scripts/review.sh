#!/usr/bin/env bash
set -euo pipefail
cd "$(dirname "$0")/.."
command -v docker >/dev/null || { echo "Docker is required"; exit 1; }
command -v ollama >/dev/null || { echo "Ollama is required"; exit 1; }
docker compose up -d postgres
echo "Waiting for PostgreSQL..."
until docker compose exec -T postgres pg_isready -U memory -d memory >/dev/null 2>&1; do sleep 1; done
if ! ollama list | awk 'NR>1 {print $1}' | grep -Fxq qwen3-embedding:0.6b; then ollama pull qwen3-embedding:0.6b; fi
scripts/fetch-slm.sh
echo "Pre-warming Ollama models..."
# The warm-up must use the context the service will ask for, or Ollama reloads the model on the first real call.
curl -fsS http://127.0.0.1:11434/api/generate -d '{"model":"memex-extractor","prompt":"ready","stream":false,"keep_alive":"10m","options":{"num_predict":1,"num_ctx":'"${OLLAMA_NUM_CTX:-12288}"'}}' >/dev/null
curl -fsS http://127.0.0.1:11434/api/embed -d '{"model":"qwen3-embedding:0.6b","input":"ready","dimensions":1024,"keep_alive":"10m"}' >/dev/null
cp -n .env.example .env 2>/dev/null || true
echo "Building and starting API..."
docker compose --profile full up -d --build api
echo "UI:      http://127.0.0.1:8080"
echo "Swagger: http://127.0.0.1:8080/swagger-ui/"
echo "Health:  http://127.0.0.1:8080/healthz"
echo "Metrics: http://127.0.0.1:8080/metrics"
echo "Logs:    docker compose logs -f api"
