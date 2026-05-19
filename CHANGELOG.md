# Changelog

## Unreleased — v0.4.0-dev

### Workspace
- Restructured into a Cargo workspace: `shrinkray-core` (existing
  destructive-write subsystems), `shrinkray-audit` (new read-only
  bloat audit), `shrinkray-cli` (new CLI binary), `src-tauri` (now
  a thin Tauri wrapper). One repo, four crates, shared lockfile.

### Audit (new)
- Read-only multi-detector bloat audit. `AuditReport` carries
  per-detector findings + aggregated metrics + a 0-100 bloat score.
  Serialises to JSON or human-readable Markdown.
- Six detectors shipped:
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
  - `editor_leftovers`: pattern-matches loose .pdb / .uproject /
    /Intermediate/ / /Engine/Editor/ leftovers.
  - `launcher_satellite`: per-language .NET satellite directories
    next to a launcher binary.
- Tauri command `audit_folder` + React "Bloat Audit" panel
  (functional UI only — IMPECCABLE design pass deferred to v0.5).

### CLI (new)
- `shrinkray audit <path> [--json] [--out FILE]` — clap-derive CLI
  with the first subcommand. Markdown by default, --json for
  machine-readable output, --out writes to file.

### Tests
- 120 tests across the workspace (47 core + 64 audit + 9 cli),
  all green. New end-to-end integration test builds a WuWa-shaped
  fixture and exercises all 6 detectors at once.

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
