#!/usr/bin/env bash
# fetch-ai-models.sh — download the ONNX models v0.7's AI restore path needs.
#
# Scaffold target only. v0.7 itself will wire the ONNX runtime (`ort` crate)
# and invoke these models per texture; this script just gets them onto disk so
# the dev workflow exists before that integration lands.
#
# Models we plan to ship:
#   * Real-ESRGAN x4 general (diffuse / generic 4× upscale, ~65 MB)
#   * Real-ESRGAN-General-x4-v3 (smaller, ~17 MB — bundle-friendly fallback)
#
# Destination: src-tauri/binaries/ai-models/  (alongside the sidecar binary;
# `Sidecar::locate()`-style search will find them via the same env var trick.)
#
# Usage:
#   bash scripts/fetch-ai-models.sh           # download both models
#   bash scripts/fetch-ai-models.sh --small   # download only the 17 MB model
#   bash scripts/fetch-ai-models.sh --check   # verify checksums, no download
#
# The model URLs use the standard upstream ONNX release artifacts. If we ship
# this for real in v1.0 we'll mirror them on shrinkray's release page so we
# don't depend on third-party hosting being up at install time.
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "${SCRIPT_DIR}/.." && pwd)"
DEST_DIR="${REPO_ROOT}/src-tauri/binaries/ai-models"

mkdir -p "${DEST_DIR}"

# Format: name|url|sha256
# SHA256 of zeros = "scaffold placeholder, replace before v0.7 release".
MODELS=(
  "realesrgan-x4-general.onnx|https://example.invalid/realesrgan-x4-general.onnx|0000000000000000000000000000000000000000000000000000000000000000"
  "realesrgan-general-x4-v3.onnx|https://example.invalid/realesrgan-general-x4-v3.onnx|0000000000000000000000000000000000000000000000000000000000000000"
)

MODE="${1:-all}"

fetch() {
  local name="$1" url="$2" expected="$3"
  local dest="${DEST_DIR}/${name}"
  if [[ -f "${dest}" ]]; then
    echo "[fetch-ai-models] ${name} already present, skipping"
    return 0
  fi
  echo "[fetch-ai-models] scaffold placeholder for ${name}"
  echo "[fetch-ai-models]   real URL/checksum land with v0.7 release"
  echo "[fetch-ai-models]   would have fetched: ${url}"
  echo "[fetch-ai-models]   expected sha256:   ${expected}"
  # Write a sentinel so downstream code can detect scaffold-stage state.
  printf 'scaffold-placeholder\nname=%s\nurl=%s\n' "${name}" "${url}" > "${dest}.placeholder"
}

check_only() {
  for entry in "${MODELS[@]}"; do
    IFS='|' read -r name url expected <<< "${entry}"
    local f="${DEST_DIR}/${name}"
    if [[ -f "${f}" ]]; then
      echo "[check] ${name}: present (skipping checksum — scaffold)"
    elif [[ -f "${f}.placeholder" ]]; then
      echo "[check] ${name}: scaffold placeholder"
    else
      echo "[check] ${name}: MISSING"
    fi
  done
}

case "${MODE}" in
  --check)
    check_only
    ;;
  --small)
    IFS='|' read -r n u s <<< "${MODELS[1]}"
    fetch "$n" "$u" "$s"
    ;;
  all|--all)
    for entry in "${MODELS[@]}"; do
      IFS='|' read -r n u s <<< "${entry}"
      fetch "$n" "$u" "$s"
    done
    ;;
  *)
    echo "usage: fetch-ai-models.sh [--all | --small | --check]" >&2
    exit 64
    ;;
esac

echo "[fetch-ai-models] done. Destination: ${DEST_DIR}"
