#!/usr/bin/env bash
set -euo pipefail
cd "$(dirname "$0")/.."
docker compose up -d postgres
until docker compose exec -T postgres pg_isready -U memory -d memory >/dev/null 2>&1; do sleep 1; done
docker run --rm --network host -e TEST_DATABASE_URL=postgres://memory:memory@127.0.0.1:55432/memory -v "$PWD:/app" -w /app rust:1.96-bookworm sh -c 'cargo fmt --check && cargo clippy --all-targets -- -D warnings && cargo test --all-targets -- --test-threads=1'
