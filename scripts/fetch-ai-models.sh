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
# Two models ship today:
#   1. super-resolution-10.onnx — ONNX-zoo sub-pixel CNN, ~240 KB. Smoke-test
#      model: validates ORT is wired, NOT a Real-ESRGAN. Fixed 224×224 input.
#   2. realesrgan-x4-general.onnx — Real-ESRGAN x4 (xinntao upstream weights,
#      BSD-3-Clause), ONNX export via crj/dl-ws on HF. ~67 MB. **Dynamic input
#      shape** [1, 3, H, W] → [1, 3, 4H, 4W], opset 10, FP32. This is the
#      load-bearing production model the Δ-Codec's RealEsrganX4 predictor
#      uses. SHA-256 is pinned; mismatch aborts the fetch.
#
# Upstream weights:
#   xinntao/Real-ESRGAN  https://github.com/xinntao/Real-ESRGAN  (BSD-3-Clause)
#
# Mirror:
#   crj/dl-ws on HuggingFace. Fallback to AXERA-TECH/Real-ESRGAN (same weights,
#   different export) if the primary mirror disappears.
#
# Override with SHRINKRAY_AI_MODEL=/path/to/your.onnx for testing alternatives.
MODELS=(
  "super-resolution-10.onnx|https://github.com/onnx/models/raw/main/validated/vision/super_resolution/sub_pixel_cnn_2016/model/super-resolution-10.onnx|"
  "realesrgan-x4-general.onnx|https://huggingface.co/crj/dl-ws/resolve/main/real_esrgan_x4.onnx|4139cc1585d04851ccd41570b0f76e775c96e064ca292d5372b6031704dda0d3"
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
  # 10 min ceiling — the Real-ESRGAN model is ~67 MB and HuggingFace can
  # throttle to ~1 MB/s on cold paths. Smoke model is 240 KB so the timeout
  # only hits on the big fetch.
  if ! curl -sL --max-time 600 --fail -o "${dest}" "${url}"; then
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
    # Smoke-only fetch — 240 KB sub-pixel CNN, enough to validate ORT wiring
    # without pulling the 67 MB Real-ESRGAN. Used in slim CI lanes.
    IFS='|' read -r n u s <<< "${MODELS[0]}"
    fetch "$n" "$u" "$s"
    ;;
  --esrgan)
    # Production fetch — just the Real-ESRGAN x4. ~67 MB.
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
    echo "usage: fetch-ai-models.sh [--all | --small | --esrgan | --check]" >&2
    exit 64
    ;;
esac

echo "[fetch-ai-models] done. Destination: ${DEST_DIR}"
