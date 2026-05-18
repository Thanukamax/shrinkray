#!/usr/bin/env bash
# Manual pre-release smoke test against a real Lyra cooked PC build.
#
# Why this isn't in CI: the Lyra Starter Game cooked PC build is 5-15 GB,
# requires an Epic account to download, and needs a full Unreal Engine 5
# install to cook a binary we can hand to shrinkray. Neither fits the
# free GitHub Actions runners. Until a self-hosted runner with a pre-
# cooked corpus exists, run this manually before tagging a release.
#
# Prereqs (one-time):
#   1. Download Lyra Starter Game from Epic's Fab marketplace.
#   2. Open the project in UE 5.x.
#   3. Package for the Windows or Linux target (whichever you can launch
#      locally) to a stable path, e.g. ~/games/Lyra.
#   4. Build shrinkray's release binary: `bun run tauri build`.
#
# Usage: scripts/lyra-smoke.sh <path-to-cooked-Lyra-folder>

set -euo pipefail

if [[ $# -lt 1 ]]; then
  echo "usage: $0 <cooked-lyra-folder>" >&2
  exit 1
fi

SRC="$1"
WORK="${TMPDIR:-/tmp}/shrinkray-lyra-smoke"
SHRINKRAY="${SHRINKRAY:-$(pwd)/src-tauri/target/release/shrinkray}"

if [[ ! -x "$SHRINKRAY" ]]; then
  echo "shrinkray binary not found at $SHRINKRAY" >&2
  echo "build it first: bun run tauri build" >&2
  exit 1
fi

echo ">>> Copying Lyra to scratch dir: $WORK"
rm -rf "$WORK"
cp -r "$SRC" "$WORK"

echo ">>> Running analyze pass"
# TODO: once a real CLI subcommand lands, replace this with:
#   "$SHRINKRAY" analyze "$WORK"
# For v0.3.0 the GUI is the only entry point — surface this gap.
echo "    (skipped — no CLI subcommand yet, see issue tracker)"

echo ">>> Lyra binary expected at:"
echo "    Windows: $WORK/Lyra/Binaries/Win64/Lyra-Win64-Shipping.exe"
echo "    Linux:   $WORK/Lyra/Binaries/Linux/Lyra-Linux-Shipping"
echo ""
echo ">>> Launch Lyra, verify it boots into the main menu, then exit."
echo ">>> If crashes appear, restore from \$WORK/../shrinkray_backup/"

# When CLI lands, append:
#   timeout 30s "$WORK/Lyra/Binaries/Linux/Lyra-Linux-Shipping" || \
#     test $? -eq 124  # 124 = clean timeout (no crash in 30s)
