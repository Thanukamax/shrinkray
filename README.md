# shrinkray

> Cut Unreal Engine game folder size by trimming what you don't need. Drop a folder, get a smaller folder.

## Why

UE games ship with predictable bloat — but **not the bloat most tools promise to fix.** A 2026-05-18 research pass (see `research/SUMMARY.md`) found that modern UE5 already ships near-optimal codecs (Oodle Kraken, Bink Audio, BCn textures) that a third-party tool cannot meaningfully beat. The real size wins come from **dropping content**, not transcoding it:

- **Unused languages** — a fully-voiced AAA UE5 game ships 4-8 dub languages; you only play one. Often **30-60% of the audio bucket** alone, zero quality loss.
- **Orphan pak entries** — dev/debug paks, duplicated shader caches, leftover platform variants.
- **Loose marketplace / modded assets** — uncooked `.png/.wav/.flac` re-encodable with no parser needed.
- **Asset-flip / RPG-Maker-on-UE bloat** — sometimes 80% of the install is content that's never referenced.

Existing tooling (UnrealPak, umodel, FModel, UAssetGUI, `repak`) **reads** UE assets but doesn't apply optimizations. shrinkray is the integrated "analyze → trim → smaller game" workflow, with a backup-or-refuse policy from day one.

## Realistic targets

| Game type | Typical savings | Where they come from |
|---|---|---|
| Asset-flip indie / RPG Maker UE ports | 50-80% | Loose-file recompression + orphan trimming |
| Heavily-modded titles | 30-60% | Uncompressed audio re-encode + language stripping |
| Mid-tier indie | 20-40% | Language stripping + dev/debug pak removal |
| Already-optimized AAA | 5-15% | Language stripping (the bulk) + orphan trimming |

These are the upper-realistic numbers. The lower bound on a modern UE5 single-language AAA install is often <5%. shrinkray will surface the predicted delta in the diff report before any write.

## What shrinkray will NOT do (v1)

Honesty about scope, because half the existing UE-tool ecosystem overpromises:

- **No pak recompression.** Oodle Kraken (the UE5 default) beats Zstd on both ratio and decode speed for game data. Free Oodle encoders don't legally exist. Transcoding to Zstd would actually inflate or stutter.
- **No cooked `.uasset/.uexp/.ubulk` surgery.** Correctly round-tripping cooked UE assets needs a serializer that tracks ~50 per-subsystem custom versions across every engine release. Only CUE4Parse (read) and UAssetAPI (read/write) do that today. v2 will ship those as a bundled .NET sidecar.
- **No Vorbis re-encode by default.** Vorbis → Opus is realistically ~20-25% per file with a lossy → lossy quality cost — opt-in only in v2.
- **No Bink Audio anything.** No free encoder, no Rust binding.
- **No BC3 → BC7 transcode.** Same 8 bpp, 0% size win.
- **No IoStore `.utoc/.ucas` rewriting in v1.** Most UE5 AAA install bytes live in IoStore; v1 reports them but leaves them alone. v2 wires in `retoc`.

If the size you wanted came from those features, v1 is not for you yet.

## Safety model

shrinkray refuses to write without a backup. On first run it offers:

- **Differential backup** (default) — records only the bytes about to be overwritten. Typically 5-15% of the folder.
- **Full copy** — safe but expensive on a 100 GB AAA folder.
- **Abort.**

`shrinkray restore <folder>` undoes any optimize run with post-restore hash verification. Wired to a button in the diff report from day one. Signed paks (`.sig` sibling present) and undecryptable AES paks are skipped, never partially modified.

## Roadmap

| Phase | What | State |
|---|---|---|
| 0 | Scaffold + folder census by extension, rough savings estimate | ✅ v0.1 |
| 1 | Scan & report + differential backup/restore + loose-file recompression (png/wav/flac → oxipng/opus) + L10N stripping + pak *trimming* (drop entries, not recompress) + dry-run + diff report UI | ✅ v0.3 |
| 1.5 | Read-only multi-detector bloat audit (13 detectors), Win7 Aero UI redesign, in-app file dialog, custom title bar | ✅ v0.4 + v0.5 |
| 2 read-side | Cooked-asset inspection via bundled .NET sidecar (CUE4Parse master, vendored): texture mip-strip projection (byte-exact via per-format formula), per-package inspection (mip table, custom-version fingerprint, export list) | ✅ v0.5 |
| 2 write-side | Cooked `.uasset/.uexp/.ubulk` rewrite — texture mip strip apply, audio L10N replacement inside paks. Needs UAssetAPI bundle. | ⏳ v0.6 |
| 2 IoStore | `retoc` integration for UE5 AAA `.utoc`/`.ucas` containers. AES key handling. | ⏳ v0.7 |
| 3+ | Vorbis → Opus opt-in, SSIM validation, mesh LOD strip, orphan-asset cross-reference, screenshot-diff launch validation | ⏳ |

The original 6-phase roadmap was rewritten after research — see `research/SUMMARY.md` for the full reasoning.

## Stack

- **Tauri v2** + **Bun** + **Vite** + **React 19** + **TypeScript** (mirrors [vn2apk](https://github.com/Thanukamax/vn2apk) + [wgpu-shader-explorer](https://github.com/Thanukamax/wgpu-shader-explorer) pattern)
- **Rust core:** `walkdir`, `tauri-plugin-dialog`, `repak` 0.2.3, `sha2`, `dirs`. Phase 1 adds `image_dds`, `intel_tex_2`, `symphonia`, `opus`, `image-compare`, `rayon` opt-in.
- **Phase 2 sidecar:** .NET 8 self-contained CLI vendoring **CUE4Parse master** (Apache 2.0) as a Git submodule at `sidecar/external/CUE4Parse/`. JSON-over-stdin IPC. ~75 MB published binary. Three additive patches to CUE4Parse to handle UE4.13-era cook layouts — see `CHANGELOG.md` v0.5.0.
- **Frontend:** 7.css for native Win7 Aero chrome (`.window`, `.title-bar`, fieldset/button styling).
- **Shell-out binaries:** `oxipng`. Optional: `mozjpeg`, `cwebp`.
- **Not used (yet):** FFmpeg, `unreal_asset` crate, Bink anything. UAssetAPI bundle pending v0.6 write-side.

## Quick start

```bash
# Clone WITH submodules (CUE4Parse lives under sidecar/external/CUE4Parse)
git clone --recurse-submodules https://github.com/Thanukamax/shrinkray
cd shrinkray

# Or if you already cloned without --recurse-submodules:
git submodule update --init

bun install
cd src-tauri && cargo fetch && cd ..

# Build the .NET sidecar (requires dotnet 8 SDK)
bash scripts/build-sidecar.sh

python3 scripts/generate_icons.py    # placeholder icons; one-time
bun run tauri dev                    # window at localhost:1420
bun run tauri build                  # production binary
```

The current build (v0.5.0) does:
- **analyze** — folder census + L10N detection + pak inventory (signed / encrypted / readable / IoStore stub) + top 50 fattest files. CEF locale `.pak` files correctly filtered out.
- **bloat audit** — read-only multi-detector report surfacing structural inefficiencies. 13 detectors:
  - `patch_overlay`, `stale_version_dir`, `sharded_videos`, `large_chunk`, `encryption` (with IoStore stub detection), `editor_leftovers`, `launcher_satellite` *(v0.4)*
  - `shader_rhi_redundancy`, `redist_installer`, `platform_siblings`, `mod_manager_artifacts`, `duplicate_content` (SHA-256), `cef_locales` *(v0.5)*
  - 0–100 bloat score + Markdown/JSON output. Works on encrypted installs (where content surgery is impossible).
- **asset inspector** *(v0.5)* — drill into a single cooked `.uasset` via the .NET sidecar: full export list, class names, custom-version fingerprint, mip table per texture (per-mip dimensions + byte sizes), with pagination, search, and payload/package filters.
- **texture mip strip projection** *(v0.5)* — walk every UTexture-derived export in a readable pak, project the savings from capping mip 0 dimension. Per-format BC/ASTC byte formula. **Pamali UE4 demo: 185 textures → 570 MB save at 1024 px cap (77 %).** Read-only; write-side lands v0.6 via UAssetAPI.
- **L10N strip + pak trim** — drop dub languages from loose files and from inside paks
- **loose-file recompression** — PNG via `oxipng`, WAV/FLAC → Opus via `opusenc` (both detected at runtime, install hints surfaced if missing)
- **differential backup + restore** — every destructive op is preceded by a `shrinkray_backup/` entry; restore replays the manifest in reverse with hash verification
- **preview-only mode** — default-on for first-time users; hard-disables every apply button
- **Win7 Aero UI** *(v0.5)* — custom title bar (Tauri OS decorations off), 7.css window chrome, frosted-glass title bar, in-app Win7-style file dialog (replaces the OS-native picker), procedurally-generated Aero wallpaper.
- **CLI** — `shrinkray audit <path> [--json] [--out FILE]`. Run from a terminal, share the markdown output, paste it into a bug report.

System tool deps (Linux): `dotnet-sdk-8.0`, `opus-tools` (for `opusenc`), `libz-ng.so.2` (Fedora/Nobara ships it; Debian/Ubuntu: `apt install libz-ng`). Optionally `oxipng` (via `cargo install oxipng` or your distro). Recommended for sidecar native helpers: `cmake` + a C/C++ toolchain (Linux build works without them — the native blob is optional).

## Building the AppImage (Linux)

`bun run tauri build --bundles appimage` will produce `target/release/bundle/appimage/shrinkray_<version>_amd64.AppImage` (~109 MB, self-contained, runs on any glibc-based x86_64 distro from ~2022 onward).

Required system packages (Fedora 41+/Nobara 43, names approximate on other distros):

```bash
sudo dnf install \
  webkit2gtk4.1-devel gtk3-devel libsoup3-devel \
  openssl-devel librsvg2-devel \
  fuse fuse-libs file desktop-file-utils \
  patchelf gobject-introspection-devel
```

Then build with `NO_STRIP=true` to skip linuxdeploy's bundled `strip` binary, which is too old to handle modern ELF `.relr.dyn` relocation sections (present in every Fedora 41+/Nobara 43 system library):

```bash
NO_STRIP=true bun run tauri build --bundles appimage
```

Without `NO_STRIP=true`, the bundle step prints hundreds of `unknown type [0x13] section .relr.dyn` errors and exits non-zero — the AppDir is still fully populated, but linuxdeploy never invokes its appimage plugin. The flag is harmless: shipped libs come pre-stripped from `/lib64`, and the final AppImage is squashfs-compressed regardless.

Known limitations:
- The .NET sidecar (`src-tauri/binaries/sidecar/`) and AI models (`src-tauri/binaries/ai-models/`) are **not yet bundled** into the AppImage — features that depend on them (asset inspector, AI restore) will need the sidecar installed alongside or a follow-up `resources` entry in `src-tauri/tauri.conf.json`.
- AppImage runtime needs FUSE2 (`fuse-libs` on Fedora). On hosts without FUSE, run with `--appimage-extract-and-run`.

## Layout

```
src/                                # React frontend
  App.tsx                           # top-level shell + sections
  TitleBar.tsx                      # custom Aero title bar (v0.5)
  OpenDialog.tsx                    # in-app Win7 Open dialog (v0.5)
  AssetInspector.tsx                # cooked .uasset drill-down (v0.5)
  MipStripPanel.tsx                 # texture mip-strip projection (v0.5)
  assets/wallpaper.jpg              # procedurally-generated Aero bg (v0.5)
crates/
  shrinkray-core/src/               # destructive-write subsystems
    analyze.rs                      # folder census + L10N detection + CEF filter
    pak.rs                          # repak wrapper + pak classification
    backup.rs                       # differential backup + restore
    strip.rs                        # L10N stripping + pak trimming
    recompress.rs                   # PNG/WAV/FLAC recompression via shell-outs
  shrinkray-audit/src/              # read-only bloat audit (v0.4 + 6 detectors v0.5)
    detectors/                      # one .rs per detector, 13 total
  shrinkray-sidecar/src/            # Rust IPC client + types for the .NET sidecar
  shrinkray-cli/src/                # CLI binary — `shrinkray audit|analyze|...`
sidecar/
  ShrinkraySidecar/                 # .NET 8 sidecar (CUE4Parse host)
    AssetInspector.cs               # per-package inspection + mip table
    AssetLister.cs                  # pak entry enumeration via PakFileReader
    StripMipsPlanner.cs             # walk + project mip-strip savings (v0.5)
    Program.cs                      # JSON-over-stdin dispatcher
  external/CUE4Parse/               # Git submodule — CUE4Parse master + 3 patches
src-tauri/src/                      # thin Tauri wrapper around core+audit+sidecar
  lib.rs                            # tauri::commands wiring
research/                           # research notes (A-E + SUMMARY) — read before contributing
scripts/                            # generate_icons.py, build-sidecar.sh, lyra-smoke.sh
.github/workflows/                  # CI (cargo test + clippy + bun build)
```

## Validation against real games

The unit tests build synthetic paks via `repak`'s writer and round-trip them through our trim + backup + restore pipeline (47 tests, all in CI). That gates correctness of the *plumbing* but does not prove shrinkray-modified games still launch.

The one corpus test that would prove that — Lyra Starter Game cooked PC build, optimize, boot, watch for crash — needs an Epic account to download Lyra source and a full Unreal Engine install to cook a binary, neither of which fits the free GitHub runners. Until a self-hosted runner with a pre-cooked corpus exists, **Lyra validation is a manual pre-release step**: see `scripts/lyra-smoke.sh` for the procedure.

## License
**Source-available, no redistribution** — see [LICENSE](LICENSE).

You may clone this repo, read the source, and build shrinkray for your own use. You may not redistribute the source or binaries, mirror the repo, or publish forks/modified versions without prior written permission.

Earlier commits (v0.3.x and prior) were MIT-licensed and remain so for anyone who obtained them under that license; this restriction applies to commits from the relicense onward. The copyright holder reserves the right to relicense future versions under permissive terms at any time.
