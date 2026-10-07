#!/usr/bin/env bash
# Build a seed dump holding only the namespace user:dev, from the local memory_app database.
# It works on a scratch copy (memory_seed); the source database is only read.
set -euo pipefail
REPO=$(git -C "$(dirname "$0")" rev-parse --show-toplevel)
PG=${PG:-$(docker compose -f "$REPO/docker-compose.yml" ps -q postgres)}
[ -n "$PG" ] || { echo "Start the root PostgreSQL service first" >&2; exit 1; }
SOURCE_DATABASE=${SOURCE_DATABASE:-memory_app}
OUT=${1:?usage: make-seed.sh <dump file>}
NS=${NS:-user:dev}
psqlc() { docker exec -i "$PG" psql -U memory -v ON_ERROR_STOP=1 "$@"; }
psqlc -d postgres -c 'DROP DATABASE IF EXISTS memory_seed' -c 'CREATE DATABASE memory_seed'
docker exec "$PG" pg_dump -U memory "$SOURCE_DATABASE" | psqlc -d memory_seed -q >/dev/null
psqlc -d memory_seed -q -v ns="$NS" <<'SQL'
BEGIN;
DELETE FROM assertions WHERE namespace <> :'ns';
DELETE FROM episodes WHERE namespace <> :'ns';
DELETE FROM memory_jobs WHERE namespace <> :'ns';
DELETE FROM entity_aliases WHERE entity_id IN (SELECT id FROM entities WHERE namespace <> :'ns');
DELETE FROM entities WHERE namespace <> :'ns';
DELETE FROM operations WHERE namespace <> :'ns';
TRUNCATE legacy_resolution_receipts, memory_relations, memory_version_sources, memory_versions, memories CASCADE;
DELETE FROM chunks WHERE raw_event_id IN (SELECT id FROM raw_events WHERE session_id IN (SELECT id FROM sessions WHERE namespace <> :'ns'));
DELETE FROM raw_events WHERE session_id IN (SELECT id FROM sessions WHERE namespace <> :'ns');
DELETE FROM sessions WHERE namespace <> :'ns';
TRUNCATE model_cache;
COMMIT;
SQL
echo "namespaces left:"; psqlc -d memory_seed -Atc "select namespace||' '||count(*) from sessions group by namespace; select 'assertions '||count(*) from assertions; select 'entities '||count(*) from entities; select 'chunks '||count(*) from chunks; select 'jobs '||status||' '||count(*) from memory_jobs group by status"
docker exec "$PG" pg_dump -U memory -Fc memory_seed > "$OUT"
ls -la "$OUT"
