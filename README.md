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
| 0 | Scaffold + folder census by extension, rough savings estimate | ✅ |
| 1 | Scan & report + differential backup/restore + loose-file recompression (png/wav/flac → oxipng/opus) + L10N stripping + pak *trimming* (drop entries, not recompress) + dry-run + diff report UI + Lyra crash-watchdog CI | ⏳ |
| 2 | Cooked-asset surgery via bundled .NET sidecar (CUE4Parse + UAssetAPI): texture mip-strip, audio L10N replacement inside paks. `retoc` IoStore support. Vorbis → Opus opt-in. Tier 3 SSIM validation. AES key handling. | ⏳ |
| 3+ | Mesh LOD strip, orphan-asset cross-reference, FFmpeg escape hatch, Bink (if a free path appears), CLI binary alongside GUI | ⏳ |

The original 6-phase roadmap was rewritten after research — see `research/SUMMARY.md` for the full reasoning.

## Stack

- **Tauri v2** + **Bun** + **Vite** + **React 19** + **TypeScript** (mirrors [vn2apk](https://github.com/Thanukamax/vn2apk) + [wgpu-shader-explorer](https://github.com/Thanukamax/wgpu-shader-explorer) pattern)
- **Rust core:** `walkdir`, `tauri-plugin-dialog` (Phase 0). Phase 1 adds `repak` 0.2.3, `image_dds`, `intel_tex_2`, `symphonia`, `opus`, `image-compare`, `rayon`. Optionally `vorbis_rs` for opt-in Vorbis decode.
- **Phase 2 sidecar:** bundled .NET 8 self-contained CLI wrapping CUE4Parse + UAssetAPI, JSON over stdin/stdout. ~70 MB one-time install cost.
- **Shell-out binaries (Phase 1):** `oxipng`. Optional: `mozjpeg`, `cwebp`.
- **Not used:** FFmpeg (license + binary-size cost), `unreal_asset` crate (UE5 coverage too shallow), Bink anything.

## Quick start

```bash
bun install
cd src-tauri && cargo fetch && cd ..
python3 scripts/generate_icons.py    # placeholder icons; one-time
bun run tauri dev                    # window at localhost:1420
bun run tauri build                  # production binary
```

The current build (v0.4.0-dev) does:
- **analyze** — folder census + L10N detection + pak inventory (signed/encrypted/readable) + top 50 fattest files
- **bloat audit** *(new in v0.4)* — read-only multi-detector report surfacing structural inefficiencies: patch overlay accumulation, stale version directories, sharded video paks, oversized chunks, encryption status, editor leftovers, launcher language satellites. Works on encrypted installs (where content surgery is impossible). 0-100 bloat score + Markdown/JSON output.
- **L10N strip + pak trim** — drop dub languages from loose files and from inside paks
- **loose-file recompression** — PNG via `oxipng`, WAV/FLAC → Opus via `opusenc` (both detected at runtime, install hints surfaced if missing)
- **differential backup + restore** — every destructive op is preceded by a `shrinkray_backup/` entry; restore replays the manifest in reverse with hash verification
- **preview-only mode** — default-on for first-time users; hard-disables every apply button
- **CLI** *(new in v0.4)* — `shrinkray audit <path> [--json] [--out FILE]`. Run from a terminal, share the markdown output, paste it into a bug report.

System tool deps (Linux): `opus-tools` (for `opusenc`), optionally `oxipng` (install via `cargo install oxipng` or your distro).

## Layout

```
src/                                # React frontend
crates/
  shrinkray-core/src/               # destructive-write subsystems
    analyze.rs                      # folder census + L10N detection
    pak.rs                          # repak wrapper + pak classification
    backup.rs                       # differential backup + restore
    strip.rs                        # L10N stripping + pak trimming
    recompress.rs                   # PNG/WAV/FLAC recompression via shell-outs
  shrinkray-audit/src/              # read-only bloat audit (v0.4)
  shrinkray-cli/src/                # CLI binary — `shrinkray audit|analyze|...`
src-tauri/src/                      # thin Tauri wrapper around core+audit
  lib.rs                            # tauri::commands wiring
research/                           # research notes (A-E + SUMMARY) — read before contributing
scripts/                            # generate_icons.py, lyra-smoke.sh
.github/workflows/                  # CI (cargo test + clippy + bun build)
```

## Validation against real games

The unit tests build synthetic paks via `repak`'s writer and round-trip them through our trim + backup + restore pipeline (47 tests, all in CI). That gates correctness of the *plumbing* but does not prove shrinkray-modified games still launch.

The one corpus test that would prove that — Lyra Starter Game cooked PC build, optimize, boot, watch for crash — needs an Epic account to download Lyra source and a full Unreal Engine install to cook a binary, neither of which fits the free GitHub runners. Until a self-hosted runner with a pre-cooked corpus exists, **Lyra validation is a manual pre-release step**: see `scripts/lyra-smoke.sh` for the procedure.

## License
**Source-available, no redistribution** — see [LICENSE](LICENSE).

You may clone this repo, read the source, and build shrinkray for your own use. You may not redistribute the source or binaries, mirror the repo, or publish forks/modified versions without prior written permission.

Earlier commits (v0.3.x and prior) were MIT-licensed and remain so for anyone who obtained them under that license; this restriction applies to commits from the relicense onward. The copyright holder reserves the right to relicense future versions under permissive terms at any time.
