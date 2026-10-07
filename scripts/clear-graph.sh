#!/usr/bin/env bash
# Deletes everything one namespace knows (facts, entities, memories, source text, jobs) and empties its Neo4j graph.
# Other namespaces are not touched. Traces stay.
# Usage: scripts/clear-graph.sh NAMESPACE [--yes]      BASE_URL defaults to http://127.0.0.1:8080
set -euo pipefail
ns=${1:?usage: scripts/clear-graph.sh NAMESPACE [--yes]}
base=${BASE_URL:-http://127.0.0.1:8080}
if [ "${2:-}" != "--yes" ]; then
  read -r -p "Delete everything the namespace '$ns' knows? Type the namespace to confirm: " typed
  [ "$typed" = "$ns" ] || { echo 'Not cleared: what you typed does not match the namespace.' >&2; exit 1; }
fi
body=$(python3 -c 'import json,sys; print(json.dumps({"namespace":sys.argv[1],"confirm":sys.argv[1]}))' "$ns")
reply=$(curl -sS -w '\n%{http_code}' -X POST "$base/api/v2/graph/clear" -H 'content-type: application/json' -d "$body")
code=${reply##*$'\n'}
echo "${reply%$'\n'*}" | python3 -m json.tool || echo "${reply%$'\n'*}"
[ "$code" = 200 ] || { echo "Not cleared: HTTP $code" >&2; exit 1; }
