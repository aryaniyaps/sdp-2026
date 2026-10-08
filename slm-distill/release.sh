#!/usr/bin/env bash
# Publishes the served model as a GitHub release, which scripts/fetch-slm.sh downloads.
#   slm-distill/release.sh <tag> <path/to/memex-extractor-q8_0.gguf> [notes file]
# The GGUF is the one export.py --gguf-out copied from the model Ollama serves, so what is
# published is byte for byte what was evaluated. The release is public when the repository is.
set -euo pipefail
tag=${1:?usage: release.sh <tag> <gguf> [notes file]}
file=${2:?usage: release.sh <tag> <gguf> [notes file]}
notes=${3:-}
repo=${SLM_REPO:-aryaniyaps/sdp-2026}
[ -f "$file" ] || { echo "$file does not exist" >&2; exit 1; }
dir=$(mktemp -d)
trap 'rm -rf "$dir"' EXIT
asset=memex-extractor-q8_0.gguf
ln -s "$(realpath "$file")" "$dir/$asset"
(cd "$dir" && sha256sum -L "$asset" > SHA256SUMS && cat SHA256SUMS)
args=(--repo "$repo" --title "Memory worker model $tag" --target "$(git rev-parse HEAD)")
if [ -n "$notes" ]; then args+=(--notes-file "$notes"); else args+=(--notes "Fine-tuned Qwen3-1.7B memory worker (GGUF, q8_0). Install with scripts/fetch-slm.sh."); fi
gh release create "$tag" "${args[@]}" "$dir/$asset" "$dir/SHA256SUMS"
echo "published https://github.com/$repo/releases/tag/$tag"
