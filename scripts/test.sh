#!/usr/bin/env bash
set -euo pipefail
cd "$(dirname "$0")/.."
docker compose up -d postgres neo4j
until docker compose exec -T postgres pg_isready -U memory -d memory >/dev/null 2>&1; do sleep 1; done
until curl -fsS --max-time 3 -u neo4j:password http://127.0.0.1:7474/db/neo4j/tx/commit -H 'content-type: application/json' -d '{"statements":[{"statement":"RETURN 1"}]}' | python3 -c 'import json,sys; sys.exit(bool(json.load(sys.stdin).get("errors")))' >/dev/null 2>&1; do sleep 1; done
if ! docker compose exec -T postgres psql -U memory -d postgres -tAc "SELECT 1 FROM pg_database WHERE datname='memory_test'" | rg -q 1; then
  docker compose exec -T postgres psql -U memory -d postgres -c 'CREATE DATABASE memory_test'
fi
docker run --rm --network host -e TEST_DATABASE_URL=postgres://memory:memory@127.0.0.1:55432/memory_test -e TEST_NEO4J_URL=http://127.0.0.1:7474 -v sdp_cargo:/usr/local/cargo -v "$PWD:/app" -w /app rust:1.96-bookworm sh -c 'cargo fmt --check && cargo clippy --all-targets -- -D warnings && cargo test --all-targets -- --test-threads=1'
python3 -m unittest discover -s benchmark -p 'test_*.py'
(cd integrations/pi && npm ci --legacy-peer-deps && npm run check && npm test)
