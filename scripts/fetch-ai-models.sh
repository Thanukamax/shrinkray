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
#
# v0.7.0 ships a smoke model (ONNX zoo's sub-pixel CNN super-resolution, 3×,
# 240 KB) for pipeline validation. Real-ESRGAN x4 production models are
# 17-65 MB and the upstream hosting situation is volatile (HF gating,
# release renaming, etc.); for v0.7.1 we'll move the production models to a
# shrinkray-hosted mirror so installs aren't dependent on third-party uptime.
#
# Until then, point SHRINKRAY_AI_MODEL at a local Real-ESRGAN ONNX you've
# downloaded yourself, or run inference with the smoke model for testing.
MODELS=(
  "super-resolution-10.onnx|https://github.com/onnx/models/raw/main/validated/vision/super_resolution/sub_pixel_cnn_2016/model/super-resolution-10.onnx|"
)

MODE="${1:-all}"

fetch() {
  local name="$1" url="$2" expected="$3"
  local dest="${DEST_DIR}/${name}"
  if [[ -f "${dest}" ]]; then
    echo "[fetch-ai-models] ${name} already present, skipping"
    return 0
  fi
  echo "[fetch-ai-models] downloading ${name}"
  echo "[fetch-ai-models]   from: ${url}"
  if ! curl -sL --max-time 120 --fail -o "${dest}" "${url}"; then
    echo "[fetch-ai-models] download FAILED for ${name}" >&2
    rm -f "${dest}"
    return 1
  fi
  # Verify checksum when one is configured. Empty checksum = "no anchor yet,
  # caller's risk" (v0.7.0 ships the smoke model without one because the
  # upstream onnx/models repo bumps occasionally).
  if [[ -n "${expected}" ]]; then
    local got
    got=$(sha256sum "${dest}" | awk '{print $1}')
    if [[ "${got}" != "${expected}" ]]; then
      echo "[fetch-ai-models] checksum mismatch for ${name}" >&2
      echo "  expected: ${expected}" >&2
      echo "  got:      ${got}" >&2
      rm -f "${dest}"
      return 1
    fi
    echo "[fetch-ai-models]   sha256 ok: ${got}"
  else
    local got
    got=$(sha256sum "${dest}" | awk '{print $1}')
    echo "[fetch-ai-models]   sha256: ${got} (no anchor — pin once stable)"
  fi
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
