#!/usr/bin/env bash
# Put the hosted memory back to the seed (the user:dev namespace as it was built from the project docs).
# Run on the VM, where the stack lives in /opt/sdp. Anything told to Pi since then is removed.
# run-local.sh runs it too, with SDP_DIR pointing at its folder and SUDO set to nothing.
set -euo pipefail
cd "${SDP_DIR:-/opt/sdp}"
# shellcheck disable=SC2086
dc() { ${SUDO-sudo} docker compose "$@"; }
dc stop term engine
dc exec -T postgres psql -U memory -d postgres -v ON_ERROR_STOP=1 -c 'DROP DATABASE memory_app' -c 'CREATE DATABASE memory_app'
dc exec -T postgres pg_restore -U memory -d memory_app --no-owner --exit-on-error < seed.dump
dc start engine
until dc exec -T engine curl -fsS http://127.0.0.1:8080/healthz >/dev/null 2>&1; do sleep 2; done
dc exec -T engine curl -fsS -X POST http://127.0.0.1:8080/api/v2/graph/rebuild -H 'content-type: application/json' -d '{"namespace":"user:dev"}'
dc start term
echo; echo "seed restored; the graph projection is rebuilding (about 10 seconds)"
