#!/bin/bash
# Started by ttyd for every browser connection: one fresh Pi session in one of three fixed directories.
# Provider, model and credentials come from the mounted Pi agent directory (auth.json, settings.json).
dir="${1:-payments-api}"
case "$dir" in payments-api|mobile-app|scratch) ;; *) dir=payments-api ;; esac
mkdir -p "/work/$dir" && cd "/work/$dir" || exit 1
[ -f README.md ] || printf '# %s\n' "$dir" > README.md
export USER=dev LOGNAME=dev
tools=()
# Set PI_TOOLS=off on the term service to remove the shell and file tools.
[ "${PI_TOOLS:-on}" = "off" ] && tools=(--no-builtin-tools)
exec pi -e /opt/memory-pi/extension.ts "${tools[@]}" --no-extensions --no-skills --no-prompt-templates --no-context-files \
  --session-dir "/tmp/pi-sessions/$dir"
