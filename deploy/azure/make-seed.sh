#!/usr/bin/env bash
# Build a seed dump holding only the namespace user:dev, from the local memory_app database.
# It works on a scratch copy (memory_seed); the source database is only read.
set -euo pipefail
PG=${PG:-sdp-2026-postgres-1}
OUT=${1:?usage: make-seed.sh <dump file>}
NS=${NS:-user:dev}
psqlc() { docker exec -i "$PG" psql -U memory -v ON_ERROR_STOP=1 "$@"; }
psqlc -d postgres -c 'DROP DATABASE IF EXISTS memory_seed' -c 'CREATE DATABASE memory_seed'
docker exec "$PG" sh -c "pg_dump -U memory memory_app | psql -U memory -q -v ON_ERROR_STOP=1 -d memory_seed" >/dev/null
psqlc -d memory_seed -q <<SQL
BEGIN;
DELETE FROM assertions WHERE namespace <> '$NS';
DELETE FROM episodes WHERE namespace <> '$NS';
DELETE FROM memory_jobs WHERE namespace <> '$NS';
DELETE FROM entity_aliases WHERE entity_id IN (SELECT id FROM entities WHERE namespace <> '$NS');
DELETE FROM entities WHERE namespace <> '$NS';
DELETE FROM operations WHERE namespace <> '$NS';
TRUNCATE legacy_resolution_receipts, memory_relations, memory_version_sources, memory_versions, memories CASCADE;
DELETE FROM chunks WHERE raw_event_id IN (SELECT id FROM raw_events WHERE session_id IN (SELECT id FROM sessions WHERE namespace <> '$NS'));
DELETE FROM raw_events WHERE session_id IN (SELECT id FROM sessions WHERE namespace <> '$NS');
DELETE FROM sessions WHERE namespace <> '$NS';
TRUNCATE model_cache;
COMMIT;
SQL
echo "namespaces left:"; psqlc -d memory_seed -Atc "select namespace||' '||count(*) from sessions group by namespace; select 'assertions '||count(*) from assertions; select 'entities '||count(*) from entities; select 'chunks '||count(*) from chunks; select 'jobs '||status||' '||count(*) from memory_jobs group by status"
docker exec "$PG" pg_dump -U memory -Fc memory_seed > "$OUT"
ls -la "$OUT"
