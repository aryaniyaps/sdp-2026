#!/usr/bin/env bash
# Pushes the stack's configuration to the VM that already exists: the compose file, the proxy, the
# environment and the GPU tunnel. It does not touch the databases, the Pi login, the seed or the
# running image tag (deploy.sh owns that), and it restarts only the services whose configuration
# changed. Run it after changing docker-compose.yml, Caddyfile.template or the tunnel settings.
# Needs: az (logged in) and the state that provision.sh left in ~/.sdp-cloud.
#   GPU_TUNNEL   1 (default) enables the tunnel service and route; 0 serves the worker model from the VM's own Ollama
set -euo pipefail

HERE=$(cd "$(dirname "$0")" && pwd)
STATE=${SDP_CLOUD_STATE:-$HOME/.sdp-cloud}
# shellcheck disable=SC1091
. "$STATE/azure.env"
# shellcheck disable=SC2034  # read by render_config in lib.sh
FQDN=$DNS.$LOC.cloudapp.azure.com
# shellcheck source=lib.sh
. "$HERE/lib.sh"

get() { grep -m1 "^$1=" "$STATE/secrets.env" | cut -d= -f2-; }
umask 077
if [ "${GPU_TUNNEL:-1}" = 1 ] && ! grep -q '^GPU_TUNNEL_SECRET=' "$STATE/secrets.env"; then
  echo "GPU_TUNNEL_SECRET=$(openssl rand -hex 24)" >> "$STATE/secrets.env"
fi
# shellcheck disable=SC2034  # read by render_config in lib.sh
hash=$(cat "$STATE/access.hash")
WORK=$(mktemp -d)
trap 'rm -rf "$WORK"' EXIT

cfg=$WORK/cfg
render_config "$cfg"
tar -C "$cfg" -czf "$WORK/cfg.tgz" .
put_file "$WORK/cfg.tgz" /opt/sdp/config.tgz

cat > "$WORK/apply.sh" <<'SH'
set -eu
cd /opt/sdp
# deploy.sh pins the image tag in the compose file; keep it across the refresh.
tag=$(grep -m1 -o 'sdp-memory-app:[A-Za-z0-9._-]*' docker-compose.yml)
cp docker-compose.yml docker-compose.yml.before-sync
tar xzf config.tgz && rm -f config.tgz
sed -i "s#image: sdp-memory-app:[A-Za-z0-9._-]*#image: $tag#" docker-compose.yml
chmod 755 pi-session.sh reset-seed.sh
chmod 644 docker-compose.yml Caddyfile
chmod 755 conf.d && chmod 644 conf.d/* 2>/dev/null || true
chmod 600 .env gpu-tunnel-users.json 2>/dev/null || true
docker compose up -d 2>&1 | tail -n 15
docker compose ps --format 'table {{.Service}}\t{{.Status}}'
# The config travelled in earlier run-command scripts. Remove those copies, but not this run's own
# folder: the agent keeps this script's output there, and deleting it would lose it.
cd /var/lib/waagent/run-command/download && ls -1 | sort -n | head -n -1 | xargs -r rm -rf
echo STEP_OK
SH
on_vm "$WORK/apply.sh"
echo "Configuration applied to $VM"
