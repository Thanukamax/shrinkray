#!/usr/bin/env bash
# fetch-pbr-corpus.sh — pull ~20 CC0 PBR texture maps into paper/corpus/pbr-cc0/
#
# All textures are CC0 from AmbientCG (https://ambientcg.com/). AmbientCG
# publishes everything under CC0 1.0 Universal so this corpus is safe to ship
# alongside the paper without per-material attribution. The script picks 1K
# resolution downloads to keep the total under 200 MB; the bench harness
# resizes anyway via --max-dim, so going to 2K buys nothing for measurement.
#
# What's fetched (mix of content classes):
#   - Diffuse / Color maps   (predictable smooth-ish RGB content)
#   - Normal maps            (BC5 territory, the killer case for residuals)
#   - Roughness maps         (single-channel, often near-constant — BC4 floor)
#   - Metalness maps         (similar — BC4)
#
# Picked from materials that span natural (brick, wood, fabric) and
# synthetic (metal plate, paint) so the high-pass energy axis is covered.
#
# Idempotent: existing files are left alone. Re-running is cheap.
#
# Usage:
#   bash scripts/fetch-pbr-corpus.sh           # download missing assets
#   bash scripts/fetch-pbr-corpus.sh --check   # report present/missing, no DL
#   bash scripts/fetch-pbr-corpus.sh --clean   # delete the corpus dir
#
# Notes:
#   - We download per-material zips from AmbientCG and extract just the maps
#     we want (Color, NormalGL, Roughness, Metalness — PNG variants). The
#     other maps in the zip (displacement, AO, etc.) are discarded after
#     extraction.
#   - If AmbientCG is rate-limiting or down, the script logs FAILED for the
#     individual material and moves on. Re-run later to fill gaps.
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "${SCRIPT_DIR}/.." && pwd)"
DEST_DIR="${REPO_ROOT}/paper/corpus/pbr-cc0"
TMP_DIR="${DEST_DIR}/.tmp"

mkdir -p "${DEST_DIR}"

# AmbientCG zip URL pattern:
#   https://ambientcg.com/get?file=<MaterialId>_1K-PNG.zip
#
# Material picks: 6 materials × ~3 maps each = ~18 textures. We discard maps
# we don't want during extraction.
MATERIALS=(
  # ===== Original 6 (2026-05-23 initial bench) =====
  "Bricks075A"      # red brick — mid-frequency repeated structure
  "Wood062"         # wood planks — strong directional grain
  "Fabric010"       # woven fabric — fine high-freq pattern
  "MetalPlates006"  # painted metal plate — hard edges + texture
  "Ground037"       # gravel/dirt — high-entropy natural noise
  "PaintedPlaster017"  # near-uniform — codec floor case

  # ===== Expansion 1 (2026-05-23): targeting routing-classifier signal =====
  # Hypothesis: ESRGAN wins on non-tiled industrial / high-detail content.
  # Bilinear wins on smooth-tiled organic + all roughness.

  # More industrial metals — ESRGAN-friendly hypothesis
  "Metal032"               # brushed/textured bare metal
  "Metal027"               # alt metal surface
  "MetalPlates013"         # different plate geometry
  "DiamondPlate008"        # strong periodic industrial pattern
  "MetalCorrugated001"     # corrugated metal — directional

  # Damaged / high-detail surfaces — ESRGAN-friendly hypothesis
  "Rust004"                # rust patina — high-detail irregular
  "Concrete028"            # damaged/cracked concrete
  "Asphalt022"             # high-frequency particulate

  # Organic / smooth — bilinear-friendly hypothesis
  "WoodFloor046"           # smooth finished wood floor
  "Marble019"              # smooth polished marble
  "Tiles101"               # ceramic tile

  # Mixed — fabric / leather (where Fabric010_NormalGL was an ESRGAN win)
  "Leather011"             # natural leather grain
  "Carpet013"              # carpet fibers
)

# Maps to keep from each zip. Names are AmbientCG's convention.
KEEP_MAPS=( "Color" "NormalGL" "Roughness" "Metalness" )

URL_BASE="https://ambientcg.com/get?file="
MAX_TOTAL_MB=200

MODE="${1:-fetch}"

fetch_one() {
  local mat="$1"
  local zip_name="${mat}_1K-PNG.zip"
  local zip_path="${TMP_DIR}/${zip_name}"
  local marker="${DEST_DIR}/.${mat}.done"
  if [[ -f "${marker}" ]]; then
    echo "[fetch-pbr] ${mat}: already extracted, skipping"
    return 0
  fi
  mkdir -p "${TMP_DIR}"
  echo "[fetch-pbr] ${mat}: downloading ${URL_BASE}${zip_name}"
  # --max-time 60: hard timeout per the experiment-harness brief. AmbientCG
  # zips at 1K are ~5-10 MB so 60s is enough on any normal link; if a mirror
  # hangs we'd rather skip and re-try than block the whole sweep.
  if ! curl -sL --max-time 60 --fail -o "${zip_path}" "${URL_BASE}${zip_name}"; then
    echo "[fetch-pbr] ${mat}: download FAILED" >&2
    rm -f "${zip_path}"
    return 1
  fi
  echo "[fetch-pbr] ${mat}: extracting maps..."
  local extracted=0
  for map in "${KEEP_MAPS[@]}"; do
    # AmbientCG file naming: <MaterialId>_1K-PNG_<MapName>.png
    local member="${mat}_1K-PNG_${map}.png"
    # unzip -p returns 0 even when the member is missing — it just writes
    # nothing. Force the empty-file cleanup unconditionally so the corpus
    # never carries 0-byte PNGs that crash the bench's image decoder.
    unzip -p "${zip_path}" "${member}" > "${DEST_DIR}/${mat}_${map}.png" 2>/dev/null || true
    if [[ -s "${DEST_DIR}/${mat}_${map}.png" ]]; then
      extracted=$((extracted + 1))
    else
      rm -f "${DEST_DIR}/${mat}_${map}.png"
    fi
  done
  rm -f "${zip_path}"
  if [[ "${extracted}" -gt 0 ]]; then
    touch "${marker}"
    echo "[fetch-pbr] ${mat}: extracted ${extracted} map(s)"
  else
    echo "[fetch-pbr] ${mat}: 0 maps extracted (zip layout changed?)" >&2
    return 1
  fi
}

check_only() {
  local present=0 missing=0
  for mat in "${MATERIALS[@]}"; do
    if [[ -f "${DEST_DIR}/.${mat}.done" ]]; then
      present=$((present + 1))
      echo "[check] ${mat}: present"
    else
      missing=$((missing + 1))
      echo "[check] ${mat}: MISSING"
    fi
  done
  echo "[check] ${present}/$((present + missing)) materials present"
}

clean_all() {
  if [[ -d "${DEST_DIR}" ]]; then
    echo "[fetch-pbr] removing ${DEST_DIR}"
    rm -rf "${DEST_DIR}"
  fi
}

write_sources_note() {
  # SOURCES.md is the canonical provenance file the bench summary points at.
  # Mirrors LICENSE.md but is the name the experiment-plan brief uses.
  {
    echo "# PBR corpus — sources"
    echo ""
    echo "All textures here are CC0 1.0 from [AmbientCG](https://ambientcg.com/)."
    echo "Re-fetch with \`bash scripts/fetch-pbr-corpus.sh\`."
    echo ""
    echo "| Material | URL | License |"
    echo "|--|--|--|"
    for mat in "${MATERIALS[@]}"; do
      echo "| ${mat} | https://ambientcg.com/view?id=${mat} | CC0 1.0 |"
    done
  } > "${DEST_DIR}/SOURCES.md"
}

write_license_note() {
  cat > "${DEST_DIR}/LICENSE.md" <<'EOF'
# PBR corpus — license

All textures in this directory are sourced from [AmbientCG](https://ambientcg.com/)
and are released under [CC0 1.0 Universal (Public Domain)](https://creativecommons.org/publicdomain/zero/1.0/).

You may use, modify, and redistribute these files for any purpose, including
commercial use, without attribution. We list the source materials below so the
provenance is traceable.

## Sources

| Material file pattern | Origin |
|--|--|
EOF
  for mat in "${MATERIALS[@]}"; do
    echo "| ${mat}_*.png | https://ambientcg.com/view?id=${mat} |" >> "${DEST_DIR}/LICENSE.md"
  done
  cat >> "${DEST_DIR}/LICENSE.md" <<'EOF'

If you need to re-fetch or audit the corpus, run:

    bash scripts/fetch-pbr-corpus.sh

EOF
}

size_check() {
  if ! command -v du > /dev/null 2>&1; then
    return
  fi
  local mb
  mb=$(du -sm "${DEST_DIR}" 2>/dev/null | awk '{print $1}')
  if [[ -n "${mb}" ]] && [[ "${mb}" -gt "${MAX_TOTAL_MB}" ]]; then
    echo "[fetch-pbr] WARNING: corpus is ${mb} MB (limit ${MAX_TOTAL_MB} MB)" >&2
  else
    echo "[fetch-pbr] corpus size: ${mb} MB"
  fi
}

case "${MODE}" in
  --check)
    check_only
    ;;
  --clean)
    clean_all
    ;;
  fetch|--fetch|all|--all)
    for m in "${MATERIALS[@]}"; do
      fetch_one "${m}" || true
    done
    rm -rf "${TMP_DIR}"
    write_license_note
    write_sources_note
    size_check
    echo "[fetch-pbr] done. Destination: ${DEST_DIR}"
    ;;
  *)
    echo "usage: fetch-pbr-corpus.sh [fetch | --check | --clean]" >&2
    exit 64
    ;;
esac
