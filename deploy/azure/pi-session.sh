#!/bin/bash
# Started by ttyd for every browser connection: one fresh Pi session in a validated project directory.
# Provider, model and credentials come from the mounted Pi agent directory (auth.json, settings.json).
dir="${1:-payments-api}"
if [[ ! "$dir" =~ ^[a-zA-Z0-9][a-zA-Z0-9_-]{0,63}$ ]]; then
  echo "Invalid project directory" >&2; exit 1
fi
mkdir -p "/work/$dir" && cd "/work/$dir" || exit 1
[ -f README.md ] || printf '# %s\n' "$dir" > README.md
export USER=dev LOGNAME=dev
export MEMORY_NAMESPACE="${2:-project:$dir}"
export MEMORY_DASHBOARD_SESSION="${3:-}"
tools=()
# Set PI_TOOLS=off on the term service to remove the shell and file tools.
[ "${PI_TOOLS:-on}" = "off" ] && tools=(--no-builtin-tools)
exec pi -e /opt/memory-pi/extension.ts "${tools[@]}" --no-extensions --no-skills --no-prompt-templates --no-context-files \
  --session-dir "/tmp/pi-sessions/$dir"
