# shrinkray — Research Pass Summary (2026-05-18)

Synthesises the 5 research streams (`A_pak_iostore.md`, `B_textures.md`, `C_audio.md`, `D_asset_parsing.md`, `E_validation.md`). Reading order: this file → individual stream notes for sourcing.

## 1. Verdicts at a glance

| Stream | Headline | Phase 1 verdict |
|---|---|---|
| **A — Pak / IoStore** | `repak` 0.2.3 writes uncompressed only. Zstd loses to Oodle Kraken on game data. Free Oodle encode does not exist legally. IoStore is the real UE5 surface, needs separate `retoc` tool. | **Pak/IoStore recompression is dead.** Trim entries (drop unused), don't transcode codecs. |
| **B — Textures** | README is wrong about loose textures — cooked games ship raw BCn blocks inside `.ubulk`. BC3→BC7 is 0% saving. Real wins = mipmap stripping (~75% per texture) + downscaling. Stack: `intel_tex_2` + `image_dds`. | Loose-file recompression is easy; header-coherent mip-strip on cooked is achievable but **B itself warns 1-2 weeks of "game won't load" debugging before reliable**. |
| **C — Audio** | Bink Audio is the UE5 default, not Vorbis. No free Bink encoder, no Rust binding. Vorbis→Opus is ~20-25%, not the README's 30-60%. PCM/FLAC→Opus is the real win (~85-93%) but rare in shipped AAA. L10N stripping is the biggest single lever. | L10N stripping + PCM/FLAC→Opus only. Skip Bink. Vorbis re-encode is opt-in Phase 2. |
| **D — Asset parsing** | `.uasset/.uexp/.ubulk` triplet, ~50 custom-version branches, only CUE4Parse + UAssetAPI track Epic's monthly churn. Pure-Rust `unreal_asset` crate is Astroneer-focused, UE5 coverage shallow. | **Ignore cooked assets entirely in Phase 1.** Phase 2 = .NET sidecar bundling CUE4Parse + UAssetAPI; Rust core stays clean. |
| **E — Validation** | Differential backup (only overwritten bytes) is the realistic default. Skip `.sig`-signed paks. No `--screenshot-and-quit` in shipping UE — Phase 1 launch test is crash-exit-code watchdog only. SSIM via `image-compare`. | 8-item MVP: refuse-without-backup, Tier 1+2 validation, no codec transcoding, skip signed paks, dry-run, diff report, `shrinkray restore`, Lyra CI test. |

## 2. Convergent findings (all 5 streams agree)

Five independent research streams arrived at the same architectural pivot:

1. **Trim, don't transcode.** Whether it's pak compression (A), audio codec (C), or texture format (B), the size wins come from **dropping content** (unused entries, dub languages, mip tails, dev shader caches), not from swapping to a better codec. Modern UE5 already ships near-optimal codecs (Oodle Kraken, Bink, BCn) — beating them in a third-party tool is mostly impossible.

2. **The shipped-asset surgery problem is universal.** Streams B, C, D all hit the same wall: touching content inside cooked `.uasset/.uexp/.ubulk` requires a full UE serializer that tracks ~50 per-subsystem custom versions across every engine release. Only CUE4Parse (read) and UAssetAPI (read/write) exist at that quality. Writing this in pure Rust is a multi-week to multi-month project with constant churn.

3. **Loose files vs cooked files is the real Phase 1 boundary.** Marketplace asset packs, modded games, RPG-Maker-on-UE ports, and dev folders ship loose `.png/.wav/.ogg/.dds` etc. — no parser needed, just `oxipng/cwebp/opus` shell-outs. This is where the 50-80% savings live in shrinkray's stated target archetypes.

4. **Modern UE5 AAA is mostly off-limits in Phase 1.** Most install bytes are inside IoStore `.ucas` containers using Oodle Kraken compression, with content cooked into `.ubulk`. Phase 1 will essentially leave a modern UE5 AAA install alone except for L10N stripping and orphaned-pak trimming.

5. **Backup-and-restore must be Phase 1, not Phase 2.** All streams call this out independently. Differential backup keeps the cost cheap enough to enforce universally.

## 3. The B-vs-D tension — resolved

**B says:** ship header-coherent mip-stripping on cooked `UTexture2D` in Phase 1 (it's 5% of CUE4Parse and the highest size-per-line-of-code win).

**D says:** don't touch cooked assets at all in Phase 1; defer all `.ubulk` work to Phase 2 with the .NET sidecar.

**Resolution: D wins for Phase 1.** Reasoning:

- B itself flags "expect 1-2 weeks of 'this game's textures don't load' debugging before mip stripping is reliable in the wild."
- E's failure-mode table treats `.uasset` GUID drift and bulk-data offset corruption as the highest-severity failures, with no cheap pre-launch signal — exactly the failures partial parser implementations cause.
- Shipping with cooked-asset support that *might* break Fortnite-derived UE branches or UE 5.4+ titles is reputationally worse than shipping with the honest scope: "loose files + L10N stripping today; cooked content in v2."
- The .NET-sidecar approach in D is the correct long-term architecture anyway. Getting Phase 1 out the door without it lets us validate the UX, backup system, and diff report against real users before committing to .NET bundle baggage.

**B's mip-strip work becomes the headline Phase 2 feature.** It's achievable, high-yield, and a clear "v2 worth waiting for" story — but it requires the .NET sidecar (or a hardened native parser) to ship safely.

## 4. Revised Phase 1 plan

shrinkray v0.1 — "Trim what we can see, trust nothing we can't."

### Features (in priority order)

1. **Scan + report** (currently the Phase 0 scaffold's `analyze_folder`). Extend to:
   - Classify by file type (loose vs container, texture vs audio vs other).
   - Detect language folders under `Content/L10N/<lang>/`.
   - Detect signed paks (`.sig` siblings) and flag as untouchable.
   - List top 50 fattest assets and biggest categories.
   - **No writes. Always safe.**

2. **Differential backup system** (from Stream E §5):
   - `shrinkray_backup/manifest.json` records every overwritten byte range with SHA256 + original payload.
   - First-write prompt: (a) full copy, (b) differential (default), (c) abort.
   - `shrinkray restore <folder>` reverses any optimize run with post-restore hash verification.

3. **Loose-file recompression**:
   - PNG: `oxipng` shell-out (lossless re-deflate).
   - JPEG: `mozjpeg` or `cwebp` (if user opts into WebP conversion).
   - WAV/FLAC: `symphonia` decode → `opus` encode @ 96 kbps (BSD bindings, no FFmpeg).
   - DDS / loose BCn: `image_dds` decode → `intel_tex_2` re-encode at lower bpp where alpha is unused.

4. **L10N stripping**:
   - Language picker in UI (multi-select, "keep these").
   - Drops entire `Content/L10N/<lang>/` trees, both loose and inside paks (via `repak` re-emit with dropped entries).
   - Zero quality loss for the user. Often the single biggest win on AAA folders.

5. **Pak trimming** (not recompression):
   - `repak` read pak → drop entries the user excluded (L10N, dev shaders, optional content) → re-emit uncompressed. Even uncompressed re-emit wins if ≥30% of entries are dropped.
   - **Refuse signed paks.** **Refuse encrypted paks unless `--aes-key` supplied or `keys.json` matches.**

6. **Tier 1+2 validation** on every modified asset:
   - Tier 1: SHA256 + size logged to manifest.
   - Tier 2: re-decode every rewritten payload (PNG header parse, WAV header parse, BCn block decode). Any failure auto-reverts that asset.

7. **Dry-run mode** (`--dry-run` / "Preview only" toggle, on by default for first-time users).

8. **Diff report UI**: header card (bytes saved, %, count, duration), category stacked bar, virtualized per-asset table, failures pane, always-visible Restore button.

9. **Crash-watchdog CI test**: Lyra cooked PC build → optimize → boot → 30 s no-crash → exit-code check. Tier 3 SSIM and screenshot-diff are Phase 1.5.

### Explicitly NOT in Phase 1

- Any cooked `.uasset/.uexp/.ubulk` parsing or rewriting (no mip-strip on cooked textures, no audio bulk-data replacement).
- Any pak compression algorithm change (no Oodle→Zstd, no Zlib→Zstd).
- IoStore `.utoc/.ucas` rewriting (read-classify only; the bytes inside `.ucas` are not touched in v1).
- Bink Audio anything (decode requires shelling out to a UE-shipped binary; encode requires SDK license).
- BC3→BC7 transcoding (0% size win anyway).
- ASTC mobile cooks.
- Virtual textures (`VT` assets).

### Stack (additions to Phase 0 scaffold)

- **Rust deps added in Phase 1:** `repak` 0.2.3 (Apache-2.0/MIT), `image` (already in Phase 1 stub), `image_dds`, `intel_tex_2` 0.5.0, `symphonia` (MPL-2.0, decode only), `opus` 0.3.1, optionally `vorbis_rs` 0.5.5, `image-compare` 0.5.0 (SSIM for diff preview).
- **Shell-out binaries bundled or detected:** `oxipng`, `cwebp` (optional), `mozjpeg` (optional).
- **NOT added Phase 1:** any .NET runtime, CUE4Parse, UAssetAPI, `retoc`, `unreal_asset` crate, FFmpeg, Bink anything.

## 5. Revised Phase 2 plan

shrinkray v0.2 — "Cooked content surgery, done safely."

1. **Bundle a .NET self-contained CLI sidecar** (~70 MB) wrapping:
   - CUE4Parse + CUE4Parse-Conversion for read/extract.
   - UAssetAPI for read-modify-write with binary-equality round-trip.
   - JSON-over-stdin/stdout protocol driven by Rust core.
2. **Cooked-texture mip stripping** (Stream B's high-yield feature). Refuses anything with unknown engine version, custom serialization, or virtual textures.
3. **Cooked-audio L10N replacement** (drop bulk data for non-selected language `USoundWave` exports, then re-emit pak).
4. **`retoc` integration** for IoStore `.utoc/.ucas` rewrite — needed for any UE 5.0+ AAA target.
5. **Vorbis → Opus opt-in** with lossy→lossy quality warning. Expected ~20-25%, not 30-60%.
6. **Tier 3 SSIM validation** on every cooked texture rewrite (`image-compare`).
7. **Screenshot-diff launch validation** (Phase 1.5 in the Stream E plan).
8. **AES key handling**: `--aes-key` + FModel-compatible `keys.json`.

### Explicitly deferred to Phase 3+

- BC3→BC7 cooked transcode (still 0% size win on its own; only useful combined with downscale).
- Mesh LOD strip / decimation (Stream B touched, but huge complexity).
- Unused-asset orphan detection (the original README Phase 4; needs cross-pak reference graph, very high false-positive risk).
- Bink Audio anything.
- Movie/Bink video re-encode.
- FFmpeg bundling.

## 6. README corrections to make

The current README (Phase 0 of 6 roadmap) needs updating:

- **Drop "PNG/TGA/BMP/JPG inside paks" from the texture detection prose** — these are not typical for cooked games; rephrase as "loose images in marketplace/modded folders."
- **Reorder Phase 1**: scan-and-report is the v1 headline; pak repack moves to Phase 4 (as "pak trimming") and pak recompression moves to Phase 5+.
- **Replace audio "30-60%" with honest numbers**: ~85-93% on PCM/FLAC (rare in AAA), ~20-25% on Vorbis (with quality cost), 30-60% on L10N stripping (no quality cost, biggest lever).
- **Note IoStore explicitly** — UE5 game folders are mostly `.ucas` not `.pak`; v1 won't touch IoStore at all.
- **Add a "what shrinkray won't do in v1" section**: doesn't touch cooked `.uasset`, doesn't recompress paks, doesn't transcode codecs, doesn't bundle Oodle/Bink encoders.
- **Backup-or-refuse policy** should be on the front page, not buried in docs.

## 7. Open questions for the user

Before I draft a `feat/phase-1` implementation plan / branch, two things to confirm:

1. **README rewrite scope** — comfortable with reordering the roadmap publicly now (signals the project is engineering-led, not vibes-led)? Or wait until Phase 1 ships and rewrite as a "v1 retrospective"?
2. **Phase 1 ordering inside the priority list** — which 3 features should be the MVP for "v0.1.0 ships when these are done"? My pick: scan+report (already half done), differential backup + restore, L10N stripping + pak trimming. Loose-file recompression and the full diff report UI could be v0.2.
3. **Test corpus** — the user has access to asset-flip / modded / AAA UE5 games. Which titles specifically? Need actual install paths for B/C/D verdicts to be stress-tested before v0.1 ships.
