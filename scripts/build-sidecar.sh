#!/usr/bin/env bash
# Build the .NET sidecar (CUE4Parse + UAssetAPI host) as a self-contained
# single-file binary. Output lives in sidecar/ShrinkraySidecar/publish/ and is
# also copied to src-tauri/binaries/sidecar/ so a packaged Tauri build can
# resolve it next to the main executable.
#
# Requires:
#   - dotnet 8 SDK on PATH (or DOTNET_ROOT set to ~/.dotnet)
#   - executed from the repo root or anywhere — uses the script's own dir
set -euo pipefail

cd "$(dirname "$0")/.."
REPO_ROOT="$(pwd)"
PROJ="$REPO_ROOT/sidecar/ShrinkraySidecar"
OUT="$PROJ/publish"
DEST="$REPO_ROOT/src-tauri/binaries/sidecar"

# Surface ~/.dotnet if user hasn't put it on PATH globally.
if ! command -v dotnet >/dev/null 2>&1 && [ -x "$HOME/.dotnet/dotnet" ]; then
  export DOTNET_ROOT="$HOME/.dotnet"
  export PATH="$HOME/.dotnet:$PATH"
fi

RID="${RID:-linux-x64}"
echo "==> Publishing sidecar for $RID"
dotnet publish "$PROJ" -c Release -r "$RID" --self-contained -o "$OUT" --nologo

echo "==> Copying to $DEST"
mkdir -p "$DEST"
rm -rf "$DEST"/*
cp -a "$OUT"/. "$DEST"/

echo "==> Done. Binary: $DEST/shrinkray-sidecar"
ls -lah "$DEST" | head -10
