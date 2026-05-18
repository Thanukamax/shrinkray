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
bun run tauri dev      # window at localhost:1420
bun run tauri build    # production binary
```

Drop a UE game folder in the picker. v0 walks the tree and reports a census. Real trimming lands in v1.

## Layout

```
src/                   # React frontend (folder picker, report panel)
src-tauri/
  src/main.rs          # bin entry → lib::run()
  src/lib.rs           # Tauri builder + analyze_folder command
  src/analyze.rs       # folder census (v0)
  src/pak.rs           # repak wrapper (Phase 1 stub)
  src/texture.rs       # loose texture encode (Phase 1 stub)
  src/audio.rs         # loose audio encode (Phase 1 stub)
  Cargo.toml
  tauri.conf.json
research/              # research notes (A-E + SUMMARY) — read before contributing
.github/workflows/build.yml
```

## License
MIT. See [LICENSE](LICENSE).
