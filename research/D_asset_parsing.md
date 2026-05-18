# Stream D — UE Cooked Asset Parsing (`.uasset` / `.uexp` / `.ubulk`)

Streams B (textures) and C (audio) both hit the same wall: once a UE game is cooked
and shipped, content lives inside the `.uasset` + `.uexp` + `.ubulk` triplet (often
further wrapped in `.pak` or UE5 IoStore `.utoc`/`.ucas`). To touch shipped content
we either parse those files ourselves, vendor a parser, or shell out to one that
already exists. This note picks a verdict.

## The Triplet: What Each File Holds

A cooked UE package is split into three on-disk artefacts that reference each other
purely by byte offsets [Source: https://yonenki.day/en/tech/uasset-format-basics/]:

- **`.uasset`** — the package *header*. Starts with magic `0x9E2A83C1`, then an
  `FPackageFileSummary` (~44 fields: UE4 version, UE5 version, ~50 custom-system
  versions, plus offsets to every other table). After the summary come the
  **Name Map** (de-duplicated `FString` table), **Import Map** (external object
  references via `FObjectImport`), **Export Map** (`FObjectExport` records — each
  with a `SerialOffset`/`SerialSize` pointing into the export blob), and optional
  metadata/thumbnails.
- **`.uexp`** — the *export blob*. Pre-UE4.15 this was concatenated to the end of
  `.uasset`; from 4.15 onward it ships as a sibling file so the engine can mmap
  the small header without touching the large payload [Source: https://forums.unrealengine.com/t/what-are-uexp-and-ubulk-files/390611].
  Each `FObjectExport.SerialOffset` (in the *combined* virtual stream) tells you
  where that export's serialized properties live.
- **`.ubulk`** — *bulk data*: mip chains, raw audio samples, anything large/
  streamable. Each `FByteBulkData` block carries `BulkDataFlags`, `ElementCount`,
  `SizeOnDisk`, and `BulkDataOffsetInFile`. Flags decide 32 vs 64-bit sizes
  (`BULKDATA_Size64Bit`), compression, and whether the payload is *inline* (inside
  `.uexp`), *end-of-file* (`.uasset` tail), or *external* (`.ubulk` /
  `.uptnl` optional payload) [Source: https://yonenki.day/en/tech/uasset-format-basics/].

UE source-of-truth files: `PackageFileSummary.cpp` (`FPackageFileSummary`),
`UObjectGlobals.cpp` / `Package.cpp` (`FObjectExport`, `FObjectImport`),
`BulkData.cpp` (`FByteBulkData`).

`FPackageIndex` is the glue: a signed int32 where `0` = null, negative `-N` =
`Import[N-1]`, positive `+N` = `Export[N-1]`. Resolving an export's class through
this is how you tell *what type* of object you're looking at — `UTexture2D`,
`USoundWave`, `UStaticMesh`, etc.

For our use case the chain is:
`Export.ClassIndex → Import → "/Script/Engine.Texture2D"` (or `SoundWave`) →
seek to `SerialOffset` in `.uexp` → parse object header → read `FTexturePlatformData`
(textures) or `FStreamedAudioPlatformData` (audio) → reach the `FByteBulkData` that
points into `.ubulk` [Source: https://github.com/BlueRaja/UModel/blob/master/Unreal/UnTexture4.cpp].

## Versioning Hell

There is no single "UE asset format" — there are dozens, gated by:

1. **`FileVersionUE4`** (legacy, frozen at 522 in UE5).
2. **`FileVersionUE5`** — `EUnrealEngineObjectUE5Version` ticks for every breaking
   change (`INITIAL_VERSION`, `NAMES_REFERENCED_FROM_EXPORT_DATA`, `PAYLOAD_TOC`,
   `OPTIONAL_RESOURCES`, `LARGE_WORLD_COORDINATES`, `DATA_RESOURCES`, etc.).
3. **~50 Custom Versions** — each subsystem (animation, materials, niagara,
   landscape, Fortnite main branch, …) has its own GUID + integer pair serialized
   as `Count(i32) + (Guid + Version)*Count`. Serialization code branches on these:
   `if (Ar.CustomVer(FEditorObjectVersion::GUID) >= SomeValue)` [Source: https://yonenki.day/en/tech/uasset-format-basics/].

A parser written against UE4.27 will *partially* read UE5.4 assets — the summary
loads, names load, but the moment you hit a property serialized with a newer
custom-version branch the byte stream desyncs silently. Bulk data layout itself
changed in UE5.0 (`PAYLOAD_TOC`, virtualized bulk data) and again in UE5.4
(`DATA_RESOURCES`).

How the giants handle it: **CUE4Parse** ships a giant `EGame` enum (one entry per
released game / engine fork) and an `EUnrealEngineObjectUE5Version`-aware reader
that picks branches per custom version [Source: https://github.com/FabianFG/CUE4Parse].
**UAssetAPI** uses `ObjectVersion` + `ObjectVersionUE5` enums plus per-system
`CustomVersion` overrides and a "binary equality" round-trip test as its
correctness gate [Source: https://atenfyr.github.io/UAssetAPI/api/uassetapi.uasset.html].
**unreal_asset (Rust)** has an `EngineVersion` enum but its examples still use
`VER_UE4_25`; coverage thins out fast past UE4.27 and stops cold before modern UE5.

## Existing Parsers — Survey

| Tool | Lang | License | Stars | Last push | Scope | Verdict |
|---|---|---|---|---|---|---|
| **CUE4Parse** | C# (.NET) | Apache-2.0 | 530 | 2026-05-17 | Full read + conversion; powers FModel | Gold standard, actively tracks new UE versions [Source: https://github.com/FabianFG/CUE4Parse] |
| **CUE4Parse-Conversion** | C# | Apache-2.0 | — | active | Texture→PNG/DDS, audio→WAV/OGG export | Sister project, what we'd actually call |
| **UAssetAPI** | C# (.NET) | MIT | 441 | 2026-05-17 | Read **and write**, UE 4.13 → 5.7, binary-equal round-trip | The only mature read/write stack [Source: https://github.com/atenfyr/UAssetAPI] |
| **unreal_asset** (AstroTechies) | Rust | MIT | 84 | 2025-11-28 | Read + some write; Astroneer-centric (UE 4.23–4.27) | Closest to "native Rust", but UE5 coverage is weak and last UE5-focused work is months-stale [Source: https://github.com/AstroTechies/unrealmodding] |
| **uasset-rs** (jorgenpt) | Rust | MIT | low | quiet | Header-only parse, UE4.10–4.26 | Toy crate — header summary only, no exports/bulk |
| **UEViewer / umodel** | C++ | MIT | 2846 | 2024-03-16 | Read-only viewer/export, pre-IoStore biased | Mature for UE3/UE4 textures+meshes; UE5 / IoStore lags |
| **repak** (trumank) | Rust | Apache-2.0 | 499 | 2026-02-20 | `.pak` container only, no asset internals | Pairs perfectly with whatever asset parser we pick [Source: https://github.com/trumank/repak] |
| **retoc** (trumank) | Rust | MIT | 171 | 2026-04-30 | UE5 IoStore `.utoc`/`.ucas` pack/unpack | Needed for any UE5 game shipping IoStore-only |

## What shrinkray Actually Needs (Phase 2)

For shrinking we don't need to *understand* the asset — we only need to:

1. **Classify**: is this `FObjectExport` a `UTexture2D` or a `USoundWave`?
2. **Locate**: where in `.uexp` or `.ubulk` does its bulk payload start, what
   size, what pixel format / sample rate?
3. **Replace**: hand the bytes to an external encoder (BC7 → BC1, OGG-recompress,
   downsample, drop a mip), then write them back.
4. **Patch**: update `SerialSize` on the export, update every affected
   `FByteBulkData.SizeOnDisk` / `BulkDataOffsetInFile`, rewrite the summary's
   `BulkDataStartOffset`, rewrite the `.pak`/IoStore index.

Step 4 is the deal-breaker. Step 1–3 are 5% of CUE4Parse; step 4 is what
UAssetAPI's whole "binary equality round-trip" infrastructure exists to make safe.

## Build vs Borrow vs Shell Out

- **Build (pure Rust, from scratch)** — write our own minimal parser targeting
  `FPackageFileSummary` + Texture2D/SoundWave exports. ~3–6 weeks of work just to
  cover UE4.25–5.4; every new game found in the wild becomes a bug report. We'd
  ship a tool that breaks on Fortnite 36.0 the day after release.
- **Borrow (`unreal_asset` crate)** — pure Rust, MIT, in-process, no runtime
  deps. But: UE5 coverage is shallow, the texture/audio export paths aren't its
  focus (it was built to mod *Astroneer*), and write support is partial. We'd
  inherit a maintenance burden we can't repay alone.
- **Shell out (CUE4Parse via bundled `dotnet` or UAssetAPI CLI)** — ship a
  ~70MB .NET runtime + the parser DLL, drive it via JSON over stdin/stdout.
  Adds Apache-2.0 (CUE4Parse) or MIT (UAssetAPI) — both shrinkray-compatible.
  Massive surface area we don't have to write. IPC overhead is irrelevant
  because we batch by-package. Downside: bundle size, .NET surprise on Linux
  users, slower per-call startup.

## Encryption Keys

Many shipped paks are AES-256 encrypted with a per-game (often per-patch) key.
Tools don't crack these — they crowdsource them. The ecosystem:

- `FModel/Unreal-Game-Keys` — small curated list for games with stable keys
  [Source: https://github.com/FModel/Unreal-Game-Keys].
- `dippyshere/fortnite-aes-archive` — historical Fortnite main + dynamic keys.
- `fnlookup.github.io/aes/` — live Fortnite key tracker.
- FModel pulls from these and lets users paste keys into its AES Manager.

shrinkray should **not** ship a baked-in key DB (becomes stale, legally noisy).
Phase 2 approach: a `--aes-key <hex>` flag plus an optional `keys.json` file in
the project dir, structurally compatible with FModel's format so users can paste.
If we can't decrypt, we degrade gracefully: skip the encrypted pak, still shrink
everything else.

## Loose / Uncooked Asset Folders (Phase 1 Target)

Three big categories ship with non-cooked, easy-pickings content:

- **Marketplace asset packs** before integration: a `Content/` tree of `.uasset`
  + sibling source files (`.png`, `.psd`, `.wav`, `.fbx` in `RawAssets/` or next
  to the cooked file).
- **Modded games / RPG Maker MV→UE / Ren'Py-on-UE ports**: often dump raw
  textures and audio as loose files; the game streams them at runtime via
  custom loaders.
- **In-development project folders** users hand to shrinkray for size-checking.

For these, Phase 1 doesn't need any uasset parsing at all — file extension is
the classifier, and re-encoding is a `oxipng` / `cwebp` / `oggenc` shell-out.
This is the bulk of the "easy wins" promised in the README's 30–60% target
(50–80% on asset-flip games).

## Phase 1 Architecture Verdict

**Phase 1 (now):** *ignore cooked assets entirely*. Walk the folder tree, target
loose `.png` / `.tga` / `.wav` / `.ogg` / `.bmp` / `.uncompressed` files. Shell
out to vendored binaries (`oxipng`, `cwebp`, `ffmpeg`, `oggenc`). Ship a working
tool in days, hit 50–80% on asset-flip UE games, build trust.

**Phase 2 (cooked assets):** **shell out** to a bundled CUE4Parse-based helper
for *read/extract*, and **borrow** UAssetAPI (also via bundled .NET helper) for
the read-modify-write round-trip on textures and audio bulk data. Reasoning:

1. We need correctness on hundreds of UE versions; only CUE4Parse and UAssetAPI
   currently track that.
2. The .NET runtime bundle (~70MB self-contained) is one-time cost; shrinkray
   is already a desktop app (Tauri), users expect a real install.
3. The Rust `unreal_asset` crate is tempting but its UE5 / texture / audio
   coverage isn't there, and we'd be the second-largest user driving it forward
   — that's a research project, not a feature.
4. `repak` (Rust) + `retoc` (Rust) **do** cover the container layer well; we can
   stay native-Rust for `.pak` / IoStore wrapping and only shell out for the
   per-asset header/bulk-data surgery.

Net architecture: Rust core (Tauri), Rust `repak`/`retoc` for containers, bundled
.NET CLI wrapping CUE4Parse + UAssetAPI for asset-level read/write, native
shell-outs (`oxipng`, `ffmpeg`, etc.) for the actual pixel/sample re-encoding.
This is the same separation FModel/UModel themselves use — let the people who
already track Epic's monthly format churn do that job; we own the UX and the
optimization pipeline.

**Bottom line for Streams B and C:** assume cooked-asset support is *not*
available in Phase 1. Design your encoders to operate on loose files, then in
Phase 2 the same encoders get called by a `cue4parse-extract → encode →
uassetapi-repack` pipeline. Don't write a uasset parser. Don't depend on
`unreal_asset` crate. Plan for a .NET sidecar.
