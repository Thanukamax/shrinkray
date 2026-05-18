# B. Textures: Cooked Format, BCn Recompression, and Phase 1 Scope

Scope: what shrinkray actually has to touch on disk to shrink UE4/UE5 textures, what the `intel_tex_2` Rust crate gives us, and where the realistic yield lives.

## 1. How UE stores cooked textures

A cooked texture asset in a shipped UE game lands on disk as a triplet inside the project's `.pak` (or `.utoc/.ucas` IoStore) container:

- `*.uasset` - package header / UObject metadata (FName table, exports, imports).
- `*.uexp` - the export data the header was stripped from (so the loader can mmap headers cheaply, then stream data later) [Source: https://forums.unrealengine.com/t/what-are-uexp-and-ubulk-files/390611].
- `*.ubulk` - bulk payload (the actual mip pixel bytes for streamed textures). For non-streaming or small mips, bulk data can also be appended to `.uasset` / `.uexp` rather than split out [Source: https://www.gildor.org/projects/umodel/faq].

The `UTexture2D` export embeds an `FTexturePlatformData` block with `SizeX`, `SizeY`, a packed `NumSlices/cubemap/optionalData/CPUCopy` field, a `PixelFormat` (FName / FString depending on game), and an `FTexture2DMipMap[]` array. Each mip carries an `FByteBulkData` descriptor: a flag word, an element count, a size on disk, and an offset that points either into the same archive or into a sibling `.ubulk` [Source: https://github.com/FabianFG/CUE4Parse/blob/master/CUE4Parse/UE4/Assets/Exports/Texture/FTexturePlatformData.cs].

What is in those bytes is **raw BCn block data, not a DDS file**. There is no DDS magic, no DX10 header, no padding. UE writes blocks tightly packed in mip order. Tools like UModel / CUE4Parse / UE4-DDS-Tools wrap them in a synthetic DDS header on export [Source: https://github.com/matyalatte/UE4-DDS-Tools]. Mobile cooks can carry ASTC / ETC2 instead, which UModel notes specifically because they cannot round-trip through DDS containers [Source: https://www.gildor.org/projects/umodel/faq].

**README correction.** The shrinkray README implies we will find loose `.dds/.png/.tga/.bmp/.jpg` inside pak files. That is almost never true for a shipped game. Loose images only appear in (a) source projects before cooking, (b) Marketplace asset zips, (c) some mod packages, and (d) games that ship pre-cook assets next to a stub project. PNG is a *source* format; cooked = DDS-style BCn blocks inside `.ubulk` [Source: https://forums.unrealengine.com/t/loading-loose-files-uasset-uexp-ubulk/2681518]. Treat loose image files as the exception, not the target.

## 2. BCn formats in practice

All BCn formats are fixed-rate block compression on 4x4 pixel tiles:

| Format | bpp | Job | Notes |
|---|---|---|---|
| BC1 (DXT1) | 4 | RGB, no/1-bit alpha | Smallest. Banding on gradients [Source: https://en.wikipedia.org/wiki/S3_Texture_Compression] |
| BC3 (DXT5) | 8 | RGBA | UE4 default for albedo with alpha |
| BC4 | 4 | single channel | greyscale masks |
| BC5 | 8 | two channel | tangent-space normals (XY, reconstruct Z) |
| BC6H | 8 | HDR RGB FP16 | skyboxes, lightmaps |
| BC7 | 8 | RGB/RGBA, high quality | same size as BC3, much better quality |

Realistic recompression yield per technique on a typical 2k RGBA texture (~5.5 MB at BC3 with full mip chain):

- **BC3 -> BC7** at the same resolution: **0% size delta**. Both are 8 bpp. BC7 is a *quality* upgrade, not a size win. Useful only if we are also downsizing or if we are replacing higher-bpp uncompressed assets [Source: https://en.wikipedia.org/wiki/S3_Texture_Compression].
- **BC3 -> BC1** where alpha is unused: **~50%** per texture. Detecting "alpha is solid 1.0" is the hard part; encoder must decode first, scan alpha, then re-encode.
- **Strip top mip (2k -> 1k logical)**: **~75%** of that texture's footprint. Mip 0 alone is 3/4 of the full chain (1 + 1/4 + 1/16 + ...).
- **Downscale 2k -> 1k then re-encode**: same 75% saving but you keep a complete chain so streaming behaves.
- **Strip language-specific UI textures**: 100% of those files; biggest blunt-instrument win.

The actually-shrinking moves are mip stripping and downscaling. Format swaps are quality plays, not size plays - unless we are going from an uncompressed (32 bpp) source to BC7 (8 bpp), which would not happen on already-cooked content.

## 3. Mipmap stripping

UE cooks textures with a full mip chain down to 1x1 by default. The streamer loads the smallest first and walks up until it hits the pool budget; `r.Streaming.MipBias` and per-LODGroup `LODBias` clamp where it stops [Source: https://dev.epicgames.com/documentation/en-us/unreal-engine/texture-streaming-configuration-in-unreal-engine]. Cooker-side, `MaxTextureSize`, `LODBias`, and `NumMipsInTail` decide what actually gets serialized.

If we strip the top mip **and rewrite the `FTexturePlatformData` header** so `SizeX/SizeY` and the mip array length agree, the engine treats the texture as if it had been cooked smaller - safe. If we strip the top mip bytes but **leave the header claiming 2048x2048**, the streamer will attempt to upload a non-existent mip, which is a known crash pattern; community-reported `MipValueMode = MipBias` with bias >= 4 also crashes [Source: https://forums.unrealengine.com/t/texture-mipmap-setting-negative-lod-bias/387731]. So: header-coherent stripping is required, not optional.

Visible cost: identical to shipping with `LODBias=1`. UI and detail textures show it; ground textures and far props rarely do.

## 4. `intel_tex_2` capabilities

Latest: **0.5.0**, published 2025-07-02, ~33k downloads/month, actively maintained by Traverse Research; bindings over Intel's ISPC texture compressor [Source: https://lib.rs/crates/intel_tex_2]. Encoders: BC1, BC3, BC4, BC5, BC6H, BC7, ETC1, ASTC (in progress). Input is an `RgbaSurface { data, width, height, stride }` (or `RgSurface` / `RSurface`). Quality presets exist per format (e.g. `bc7::alpha_basic_settings()` vs `alpha_slow_settings()`).

What it does not do: **no decoder for any format**, no ETC2, no KTX/DDS container handling. Cannot read a BC3 block.

## 5. The decoder problem

To do BC3 -> BC7 (or BC3 -> downscaled BC3) we must decode first. Rust options, ranked by fit:

1. **`image_dds`** (Traverse-Research-adjacent, ScanMountGoat). Encode via `intel-tex-rs-2`, **decode via a safe Rust port of `bcdec`**. Handles all BCn we care about, can work on raw block bytes (not just DDS-wrapped) [Source: https://github.com/ScanMountGoat/image_dds]. This is the obvious pick - it pairs the encoder we already want with a decoder, in one crate, by an active author.
2. **`texture2ddecoder`** - pure Rust, decodes BC3/4/5/6/7 plus mobile formats (ASTC, ETC). Decode-only. Good fallback for ASTC mobile cooks [Source: https://crates.io/crates/texture2ddecoder].
3. **`texpresso`** - pure-Rust libsquish port, encode + decode for BC1-7 + ETC2 + ASTC, but throughput is much lower than ISPC; treat as a portability fallback [Source: https://github.com/jansol/texpresso].
4. **`bcndecode`** - older C-binding decoder, narrower scope, not needed if `image_dds` is in.

**Recommended Phase 1 stack:** `image_dds` (covers decode + the `intel_tex_2` it wraps) plus `texture2ddecoder` for ASTC mobile fallback. Drop the README's standalone `texpresso` dep unless we hit a real portability case.

## 6. Actually parsing `.ubulk`

The hard path: open `.uasset`, locate the `UTexture2D` export, parse `FTexturePlatformData`, walk `FTexture2DMipMap[]`, follow each mip's `FByteBulkData` offset into `.ubulk`, decode, re-encode/strip/resize, write new bytes, **rewrite every changed offset and size in the header**, fix `SizeX/SizeY` and mip count, and keep alignment that the runtime expects. CUE4Parse does this in C# across hundreds of engine-version branches [Source: https://github.com/FabianFG/CUE4Parse]. No Rust crate currently does it end-to-end at production quality - `repak` reads pak containers but does not parse `UTexture2D`.

Phase 1 reality: writing a correct UE asset serializer in Rust is a multi-week project on its own, with per-engine-version quirks (Fortnite branches, custom versions, Oodle). Doing it half-right will corrupt shipped games.

## 7. Quality validation

For pre/post comparison after recompression:

- **`dssim`** by kornelski - multiscale SSIM in L*a*b*, fast, the de-facto Rust SSIM [Source: https://github.com/kornelski/dssim].
- **`image-compare`** - SSIM + RMS + hybrid + histogram, rayon-parallel, CPU-only [Source: https://lib.rs/crates/image-compare].
- For a "render a frame" sanity check, we cannot cheaply boot the game engine. Best we can do in Phase 1 is render the *texture itself* (decoded mip 0, side-by-side original vs recompressed, with a third pane showing absolute diff x8). That is enough to catch BC1-alpha-loss and BC7-mode-collapse cases by eye.

## Phase 1 scope verdict

**Ship loose-texture handling AND header-coherent mip stripping. Defer full `.ubulk` rewriting to Phase 2.**

Concretely for Phase 1:

1. **Loose image files** inside paks (`.dds/.png/.tga` in dev/Marketplace/modded paks) - decode with `image` + `image_dds`, re-encode to BC7 or BC1 with `intel_tex_2`, write back. Low yield on shipped AAA, real yield on the asset-flip / RPG-Maker-port archetype the README already targets. Easy and safe.
2. **Mip-tail stripping on cooked textures** - this is the highest size-per-line-of-code win, but it requires touching the `FTexturePlatformData` header. Scope it tight: parse only enough of `UTexture2D` to read `PixelFormat`, `SizeX/Y`, and the mip array; only strip whole trailing entries; rewrite the header in place; refuse anything with unknown engine version, custom serialization, or virtual textures. This is a fraction of what CUE4Parse does, and is achievable.
3. **Defer**: full BC3 -> BC7 transcode of cooked textures, ASTC mobile cooks, IoStore container rewriting, and virtual texture (`VT`) assets. Phase 2.

Honesty on uncertainty: I have not opened a real shipped pak from `.uexp/.ubulk` to confirm exact offset semantics across UE 5.3 / 5.4 / 5.5; CUE4Parse's source is the ground truth and we should mirror its parser, not invent ours. Expect 1-2 weeks of "this game's textures don't load" debugging before mip stripping is reliable in the wild.
