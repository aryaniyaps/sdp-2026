#!/usr/bin/env bash
# Runs the hosted setup on this machine: Pi in a browser terminal next to the live memory graph,
# behind the same proxy and access code, at http://127.0.0.1:18088/ (user reviewer). It is the
# stack of docker-compose.yml with a local proxy address, and it keeps its own databases, so it
# does not touch the memory of scripts/run-memory.sh.
#   run-local.sh [up]   build the image, start everything, print the address (the default)
#   run-local.sh down   stop it, keeping the memory
#   run-local.sh destroy   stop it and delete the memory and the generated files
# Needs Docker Compose 2.24 or newer, openssl, curl, and a Pi that is logged in on this machine. The
# memory worker is the fine-tuned model, served by the stack's own Ollama (CPU unless GPU=1); Pi and
# its login are used only for the terminal.
#   LOCAL_PORT    port on 127.0.0.1                 (default 18088)
#   PI_AGENT_DIR  Pi's agent directory               (default ~/.pi/agent)
#   PI_PROVIDER, PI_MODEL                            (default: Pi's settings.json, for the terminal only)
#   GPU=1         give the stack's Ollama your NVIDIA GPU (needs the NVIDIA container toolkit)
#   SLM_FILE      a GGUF of the worker model you already have (default: downloaded from the release)
#   SEED_DUMP     a dump made by make-seed.sh to start from (default: start empty)
#   SDP_LOCAL_STATE  where the generated files live  (default ~/.local/state/sdp-hosted-local)
set -euo pipefail

HERE=$(cd "$(dirname "$0")" && pwd)
REPO=$(git -C "$HERE" rev-parse --show-toplevel)
STATE=${SDP_LOCAL_STATE:-${XDG_STATE_HOME:-$HOME/.local/state}/sdp-hosted-local}
PORT=${LOCAL_PORT:-18088}
PI_DIR=${PI_AGENT_DIR:-$HOME/.pi/agent}
SEED=${SEED_DUMP:-}
export COMPOSE_PROJECT_NAME=sdp-memory-local
. "$HERE/lib.sh"

dc() { (cd "$STATE" && docker compose "$@"); }

case "${1:-up}" in
  down) dc down; exit 0 ;;
  destroy)
    [ ! -d "$STATE" ] || dc down -v
    # The containers made pi-agent/ belong to uid 10001; take it back so it can be removed.
    if [ -d "$STATE/pi-agent" ]; then
      docker run --rm --user root --entrypoint chown -v "$STATE/pi-agent:/d" sdp-memory-app:local -R "$(id -u):$(id -g)" /d
    fi
    rm -rf "$STATE"
    echo "removed the stack, its memory and $STATE"
    exit 0 ;;
  up) ;;
  *) echo "usage: run-local.sh [up|down|destroy]" >&2; exit 2 ;;
esac

for tool in docker openssl python3 curl; do
  command -v "$tool" > /dev/null || { echo "missing tool: $tool" >&2; exit 1; }
done
[ -f "$PI_DIR/auth.json" ] || { echo "no $PI_DIR/auth.json: log in with Pi first" >&2; exit 1; }
read -r PI_PROVIDER PI_MODEL < <(python3 "$HERE/pi-settings.py" "$PI_DIR/settings.json")
[ -n "${PI_PROVIDER:-}" ] && [ -n "${PI_MODEL:-}" ] || { echo "Set both PI_PROVIDER and PI_MODEL, or configure Pi's default model" >&2; exit 1; }
[ -z "$SEED" ] || [ -f "$SEED" ] || { echo "SEED_DUMP $SEED does not exist" >&2; exit 1; }

echo "Building the image sdp-memory-app:local (the first build compiles the service, several minutes)"
ctx=$(mktemp -d)
trap 'rm -rf "$ctx"' EXIT
pack_context "$ctx"
docker build -t sdp-memory-app:local "$ctx"

umask 077
mkdir -p "$STATE/pi-agent"
if [ ! -f "$STATE/.env" ]; then
  {
    echo "POSTGRES_PASSWORD=$(openssl rand -hex 16)"
    echo "NEO4J_PASSWORD=$(openssl rand -hex 16)"
  } > "$STATE/.env"
fi
grep -E '^(POSTGRES|NEO4J)_PASSWORD=' "$STATE/.env" > "$STATE/.env.new" && mv "$STATE/.env.new" "$STATE/.env"
[ -f "$STATE/access.code" ] || openssl rand -hex 12 > "$STATE/access.code"
hash=$(docker run --rm caddy:2 caddy hash-password --plaintext "$(cat "$STATE/access.code")")

cp "$HERE/docker-compose.yml" "$HERE/pi-session.sh" "$STATE/"
sed -e 's#__SITE_ADDRESS__#:80#' -e "s#__ACCESS_HASH__#$hash#" "$HERE/Caddyfile.template" > "$STATE/Caddyfile"
mkdir -p "$STATE/conf.d"
# A previous run handed this folder to uid 10001, so take it back before refreshing the login.
docker run --rm --user root --entrypoint chown -v "$STATE/pi-agent:/d" sdp-memory-app:local -R "$(id -u):$(id -g)" /d
cp "$PI_DIR/auth.json" "$STATE/pi-agent/auth.json"
printf '{\n  "defaultProvider": "%s",\n  "defaultModel": "%s",\n  "defaultThinkingLevel": "medium",\n  "quietStartup": true,\n  "tuiMode": "regular"\n}\n' \
  "$PI_PROVIDER" "$PI_MODEL" > "$STATE/pi-agent/settings.json"
cat > "$STATE/docker-compose.override.yml" <<YML
services:
  engine:
    image: sdp-memory-app:local
    ports:
      - "127.0.0.1:${LOCAL_ENGINE_PORT:-18080}:8080"
  term:
    image: sdp-memory-app:local
    ports:
      - "127.0.0.1:${LOCAL_TERMINAL_PORT:-7681}:7681"
  caddy:
    ports: !override
      - "127.0.0.1:$PORT:80"
  ollama:
    ports:
      - "127.0.0.1:${LOCAL_OLLAMA_PORT:-18434}:11434"
YML
if [ "${GPU:-0}" = 1 ]; then
  cat >> "$STATE/docker-compose.override.yml" <<YML
    deploy:
      resources:
        reservations:
          devices:
            - driver: nvidia
              count: all
              capabilities: [gpu]
YML
fi
# The files above were made under umask 077, but the terminal runs pi-session.sh as uid 10001 inside the image.
chmod 755 "$STATE/pi-session.sh"
chmod 644 "$STATE/docker-compose.yml" "$STATE/Caddyfile"
chmod 755 "$STATE/conf.d"
# The terminal and the memory worker run as uid 10001 inside the image and read the Pi login from here.
# The files are world readable inside a folder only you can enter.
chmod 755 "$STATE/pi-agent" && chmod 644 "$STATE/pi-agent/auth.json" "$STATE/pi-agent/settings.json"
docker run --rm --user root --entrypoint chown -v "$STATE/pi-agent:/d" sdp-memory-app:local -R 10001:10001 /d
chmod 700 "$STATE"

echo "Starting the databases and pulling the embedding model (about 640 MB the first time)"
dc up -d postgres neo4j ollama
dc up ollama-init
echo "Installing the worker model into the stack's Ollama (about 1.8 GB the first time)"
OLLAMA_URL=http://127.0.0.1:${LOCAL_OLLAMA_PORT:-18434} "$REPO/scripts/fetch-slm.sh"
if [ -n "$SEED" ]; then
  echo "Restoring the seed memory"
  cp "$SEED" "$STATE/seed.dump"
  cp "$HERE/reset-seed.sh" "$STATE/reset-seed.sh"
  dc up -d engine
  SDP_DIR=$STATE SUDO='' "$STATE/reset-seed.sh"
fi
dc up -d

echo "Waiting for the page"
code=$(cat "$STATE/access.code")
for _ in $(seq 1 60); do
  if dc exec -T engine curl -fsS -o /dev/null 'http://127.0.0.1:8080/api/v2/status?namespace=local-readiness' &&
     curl -fsS -o /dev/null -u "reviewer:$code" "http://127.0.0.1:$PORT/demo" &&
     curl -fsS -o /dev/null -u "reviewer:$code" "http://127.0.0.1:$PORT/term/"; then ready=1; break; fi
  sleep 2
done
[ "${ready:-0}" = 1 ] || { echo "the page did not come up; see: cd $STATE && docker compose logs" >&2; exit 1; }
cat <<DONE

Open http://127.0.0.1:$PORT/   user: reviewer   access code file: $STATE/access.code
Stop: deploy/azure/run-local.sh down      Remove everything: deploy/azure/run-local.sh destroy
DONE
