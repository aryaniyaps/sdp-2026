# shellcheck shell=bash
# shellcheck disable=SC2154  # render_config reads hash, FQDN and get from the script that sources this file
# Shared by deploy.sh, provision.sh and run-local.sh. Source it, do not run it.
# Needs REPO (the repository root); on_vm and put_file also need RG and VM.

# Runs a script on the VM as root through the Azure agent (no SSH) and fails unless the script
# reached its last line. run-command reports success even when the script failed, so every script
# must end by printing STEP_OK. Prints the output cut to 300 columns.
on_vm() {
  local out
  out=$(az vm run-command invoke -g "$RG" -n "$VM" --command-id RunShellScript --scripts @"$1" \
    --query 'value[0].message' -o tsv)
  printf '%s\n' "$out" | cut -c1-300
  printf '%s' "$out" | grep -q '^STEP_OK$' || { echo "failed on the VM (step $1)" >&2; return 1; }
}

# Copies a local file to a path on the VM. A run-command script is limited to 256 KB, so the file
# goes over base64 in chunks of 150 KB and is decoded when the last one has arrived.
put_file() {
  local src=$1 dest=$2 tmp i n=0 total
  tmp=$(mktemp -d)
  base64 -w0 "$src" > "$tmp/all.b64"
  split -b 150000 -d -a 4 "$tmp/all.b64" "$tmp/chunk."
  total=$(find "$tmp" -name 'chunk.*' | wc -l)
  for i in "$tmp"/chunk.*; do
    n=$((n + 1))
    {
      echo 'set -eu'
      echo "mkdir -p \"\$(dirname '$dest')\""
      if [ "$n" = 1 ]; then echo ": > '$dest.b64'"; fi
      echo "cat >> '$dest.b64' <<'CHUNK'"
      cat "$i"; echo
      echo 'CHUNK'
      if [ "$n" = "$total" ]; then echo "base64 -d '$dest.b64' > '$dest' && rm -f '$dest.b64'"; fi
      echo 'echo STEP_OK'
    } > "$tmp/send.sh"
    echo "  $(basename "$dest") part $n of $total" >&2
    on_vm "$tmp/send.sh" > /dev/null || { rm -rf "$tmp"; return 1; }
  done
  rm -rf "$tmp"
}

# Assembles the Docker build context of the hosted image in the empty directory $1: only what the
# image needs, taken from the working tree.
pack_context() {
  local ctx=$1
  mkdir -p "$ctx/deploy/azure" "$ctx/integrations"
  cp "$REPO/Cargo.toml" "$REPO/Cargo.lock" "$REPO/rust-toolchain.toml" "$ctx/"
  cp -r "$REPO/migrations" "$REPO/src" "$ctx/"
  cp -r "$REPO/integrations/pi" "$ctx/integrations/pi"
  cp -r "$REPO/frontend" "$ctx/frontend"
  rm -rf "$ctx/frontend/node_modules" "$ctx/frontend/dist" "$ctx/frontend/test-results" "$ctx/frontend/playwright-report"
  rm -rf "$ctx/integrations/pi/node_modules" "$ctx/integrations/pi/test"
  cp "$REPO/deploy/azure/Dockerfile" "$ctx/Dockerfile"
  cp "$REPO/deploy/azure/pi-session.sh" "$ctx/deploy/azure/pi-session.sh"
}

# Writes the stack's configuration files into the directory $1: compose file, proxy, environment, and
# (when GPU_TUNNEL is 1) the tunnel's auth file and proxy route. Needs HERE, FQDN and hash (the Caddy
# password hash), and a `get NAME` function that reads a generated secret. Does not write the Pi
# login or the seed: those are uploaded once and left alone afterwards.
render_config() {
  local cfg=$1
  mkdir -p "$cfg/conf.d"
  cp "$HERE/docker-compose.yml" "$HERE/pi-session.sh" "$HERE/reset-seed.sh" "$cfg/"
  sed -e "s#__SITE_ADDRESS__#$FQDN#" -e "s#__ACCESS_HASH__#$hash#" "$HERE/Caddyfile.template" > "$cfg/Caddyfile"
  {
    echo "POSTGRES_PASSWORD=$(get POSTGRES_PASSWORD)"
    echo "NEO4J_PASSWORD=$(get NEO4J_PASSWORD)"
    if [ "${GPU_TUNNEL:-1}" = 1 ]; then
      echo "COMPOSE_PROFILES=gpu-tunnel"
      echo "EXTRACTION_OLLAMA_URL=http://gpu-tunnel:11436"
    fi
  } > "$cfg/.env"
  if [ "${GPU_TUNNEL:-1}" = 1 ]; then
    printf '{"gpu:%s":["^R:0.0.0.0:11436$"]}\n' "$(get GPU_TUNNEL_SECRET)" > "$cfg/gpu-tunnel-users.json"
    cp "$HERE/gpu-tunnel.caddy" "$cfg/conf.d/gpu-tunnel.caddy"
  fi
}
