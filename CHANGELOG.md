# Changelog

## v0.5.0 — 2026-05-20

Phase 2 read-side. Cooked texture data is now reachable end-to-end via a
.NET sidecar, with byte-exact mip-strip projection. UI redesigned to a
Win7 Aero aesthetic with a custom title bar and in-app file dialog.

### Phase 2 read-side (new)
- **Mip strip projection**: walk every readable package in a pak, find
  UTexture-derived exports, project the savings from capping each
  texture's top mip dimension. Pamali UE4 demo: 185 textures, 739 MB
  total texture data, **570 MB reclaimable (77%) at a 1024 px cap**.
- **Asset Inspector**: drill into a single cooked `.uasset`, see its
  full export list, custom-version fingerprint, mip table for any
  texture (per-mip dimensions and byte sizes), with paginated entries
  + payload/package filters + path search.
- **Sidecar pipeline**:
  - .NET 8 self-contained binary, ~75 MB. CUE4Parse master vendored as
    a Git submodule at `sidecar/external/CUE4Parse/`.
  - `ZlibHelper.Initialize` candidate-path scan for `libz-ng.so.2`
    (the missing init was the silent blocker on every package load).
  - Correct `Initialize → MountAsync → PostMount → LoadVirtualPaths`
    sequence so `provider.Files` is actually populated.
  - GameFile-overload `TryLoadPackage` instead of string keys to
    bypass CUE4Parse's path-normalization quirks.
  - Reflection via `GetField` (not `GetProperty`) for
    `FTexturePlatformData.SizeX / Mips / PixelFormat` — they're
    public fields, not properties.
- **CUE4Parse patches** (additive, vendored on the submodule branch):
  - `UTexture2D.cs` — `DeserializeCookedPlatformDataLegacy()`
    fallback that parses the UE4.13-era `FIX_WIDE_STRING_CRC` cook
    layout (`3×int32 preamble + SizeX + SizeY + NumSlices +
    FString format + FirstMip + NumMips + Mips`).
  - `FTexturePlatformData.cs` — removed `readonly` on 4 fields so
    the legacy parser can populate them post-construction.
  - `UTexture.cs` — FName/FString probe for older pixel format
    encodings (defensive, helps the master path too).
- **Per-format byte formula**: BC1/BC3/BC4/BC5/BC6H/BC7/DXT1-5,
  ASTC 4×4/6×6/8×8, uncompressed RGBA8/G8/A2B10G10R10/Float
  variants. Matches UE's own on-disk allocation within ~1 %.

### Audit detectors (5 new — now 13 total)
- `shader_rhi_redundancy`: PCD3D_SM5+SM6 / VulkanSM5+SM6 shipping
  side by side. Warning ≥ 200 MB redundant.
- `redist_installer`: bundled UE4PrereqSetup, vc_redist, dxsetup,
  .NET installers under `/Redist/` or at root. Warning ≥ 10 MB.
- `platform_siblings`: `Engine/Binaries/{Win64,Linux,Mac,IOS,...}`
  multi-platform trees. Warning ≥ 500 MB foreign.
- `mod_manager_artifacts`: `.disabled` / `.modtemp` / `.nxm_backup`
  / `.vortex_backup` / `.mohidden` / `.original`. Warning ≥ 50 MB.
- `duplicate_content`: SHA-256 dedup of files ≥ 4 MB (two-stage:
  size-collision filter first, hash second). Warning ≥ 100 MB.
- `cef_locales`: per-locale Chromium Embedded Framework `.pak`
  bundles outside `en-US`/`en-GB`/`en`. Typical AAA install: ~58
  files, ~10–15 MB reclaim.

### Bug fixes
- **CEF pak misclassification**: Chromium Embedded Framework's
  locale `.pak` files were being scanned as UE paks and showing up
  as "58 unreadable" in the encryption detector. `analyze.rs` now
  skips any pak under `/CEF3/` or `/cef/` paths.
- **ANALYZE button stuck loading**: `backup_status` follow-up moved
  out of the try/finally that owns `setPending(false)`;
  `analyze_folder` Rust command now returns `Result<_, String>`
  wrapped in `catch_unwind` so panics surface as IPC errors
  instead of deadlocking the await.
- **Layout "scales weird"**: `.layout` was a `height: 100vh` grid
  with `auto auto auto 1fr` for 9+ children — content past the 4th
  row overflowed and `.report` had `overflow-y: auto` causing
  nested scrollbars. Now flex column, `max-width: 1200px`, single
  body scroll.
- **Tauri capability gaps**: `core:window:allow-{close,minimize,
  toggle-maximize,start-dragging,is-maximized}` were missing, so
  the custom Aero title bar's min/max/close didn't work and
  dragging failed. Permissions added explicitly.

### Win7 Aero UI (new)
- Custom title bar drawn in React (`TitleBar.tsx`) backed by
  `getCurrentWindow().{minimize, toggleMaximize, close,
  startDragging}`. OS decorations disabled via
  `decorations: false` in `tauri.conf.json`.
- **7.css** integrated for native Aero button / fieldset / window
  chrome. `.window.active.glass` for the frosted-glass effect.
- **Aero wallpaper** procedurally generated locally via
  ImageMagick (945 KB JPEG, 1920×1200) — deep blue base + six
  soft bokeh blooms + diagonal sheen + gaussian noise. Bundled in
  `src/assets/` so the backdrop-filter blur has real texture to
  chew on. No Microsoft assets shipped.
- Typography hierarchy: top-level `.report h2` is `1.05rem` weight
  500 non-uppercase (was `0.75rem` uppercase letter-spaced, which
  read as a label, not a heading).
- Inspector empty state with a gradient bg + primary CTA so it's
  no longer mistaken for a label.
- Severity-tinted findings replace the IMPECCABLE-banned
  side-stripe accents.

### In-app file dialog (new)
- `OpenDialog.tsx` — Win7-style Open dialog with breadcrumb path,
  Favorites / Recent / Drives sidebar, file list with double-click
  navigation, ESC-to-cancel, mode toggle (folder vs `.pak` filter).
- Replaces the OS-native dialog so the whole flow stays inside
  the Aero theme. Backed by new Tauri commands `list_dir`,
  `quick_links`, `path_parent`.

### Misc
- Apply path for mip strip wired as a v0.6 stub
  (`sidecar_apply_strip_mips`) returning a structured "not
  implemented, needs UAssetAPI" payload so the UI can render an
  honest "what's next" affordance.
- Layout grid + analyze handler fixes (#1 + #4 from the
  IMPECCABLE audit pass).

### Tests
- 155 tests across the workspace (96 audit + 9 cli + 47 core + 3
  sidecar), all green. 25 new tests covering the 5 new detectors,
  5 covering `cef_locales`.

### Known limitations
- Mip byte sizes come from the BC / ASTC pixel-format formula,
  not from raw `FByteBulkData.Header.ElementCount`. Matches UE's
  own on-disk allocation within ~1 %; raw bulk parsing lands when
  the write-side does.
- UE5 IoStore cooks (`.utoc`/`.ucas`) are still locked. `retoc`
  integration is on the v0.7 roadmap.
- Write-side mip strip not implemented. v0.6 milestone.
- CUE4Parse is now a Git submodule. Fresh clones need
  `git clone --recurse-submodules`, or
  `git submodule update --init` after a plain clone.

## v0.4.0 — 2026-05-19

### Workspace
- Restructured into a Cargo workspace: `shrinkray-core` (existing
  destructive-write subsystems), `shrinkray-audit` (new read-only
  bloat audit), `shrinkray-cli` (new CLI binary), `src-tauri` (now
  a thin Tauri wrapper). One repo, four crates, shared lockfile.

### Audit (new)
- Read-only multi-detector bloat audit. `AuditReport` carries
  per-detector findings + aggregated metrics + a 0-100 bloat score.
  Serialises to JSON or human-readable Markdown.
- Seven detectors shipped:
  - `patch_overlay`: `_P.pak` overlay accumulation (zombie content
    in base paks). Conservative 50%-of-overlay reclaimable estimate;
    Critical when any chunk overlay ratio ≥ 40%.
  - `stale_version_dir`: per-parent stale `X.Y.Z` directories
    (numeric-segment comparison so 3.3.9 < 3.3.11).
  - `sharded_videos`: directories with 20+ subdirs each holding a
    single small pak (the WuWa `Video/Paks/` pattern).
  - `large_chunk`: paks above the 2 GB recommended threshold;
    Critical above 10 GB.
  - `encryption`: classifies every pak via repak; surfaces the
    encrypted percentage as the hard ceiling on content surgery.
    Detects IoStore stubs (.pak paired with .utoc/.ucas) and flags
    them as critical (retoc Phase 2 needed).
  - `editor_leftovers`: pattern-matches loose .pdb / .uproject /
    /Intermediate/ / /Engine/Editor/ leftovers.
  - `launcher_satellite`: per-language .NET satellite directories
    next to a launcher binary.
- Tauri command `audit_folder` + React "Bloat Audit" panel.

### CLI (new)
- `shrinkray audit <path> [--json] [--out FILE]` — clap-derive CLI
  with the first subcommand. Markdown by default, --json for
  machine-readable output, --out writes to file.

### Tests
- 125 tests across the workspace (66 audit + 9 cli + 47 core + 3
  sidecar), all green.

## v0.3.0 — 2026-05-19
- All 6 implementation steps: analyze, differential backup/restore,
  L10N strip + pak trim, loose-file recompression, preview-only
  mode, CI test+clippy+build.
- 47 unit tests passing. GUI boots cleanly after the Tauri v2
  capabilities + plugin config fix.

## v0.1.0 — 2026-05-19 (Phase 0)
- Scaffold: Tauri v2 + Rust + React + folder picker
- `analyze_folder` IPC command — walks tree, classifies textures /
  audio / paks by extension, rough savings estimate
- pak.rs / texture.rs / audio.rs stubs for Phase 1
