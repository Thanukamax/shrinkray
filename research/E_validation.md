# E. Validation Strategy

Single biggest risk for shrinkray: "we made the game smaller and now it crashes / shows checkerboards / has no audio." Every other design decision must defer to keeping that ratio at zero across the test corpus. This note covers what we have to catch, how cheaply we can catch it, what we ship in Phase 1, and the open-source titles we use as a fixed CI corpus.

## 1. Failure modes and their cheapest signal

For each failure mode the cheapest pre-launch signal is listed. If the cheap signal can rule it out, we never need the 5-minute launch test for that pak.

| Failure mode | Cheapest pre-launch signal |
|---|---|
| Game won't launch (pak hash mismatch) | UE often signs the global pak. Phase 1 must detect `pakchunk0-WindowsNoEditor.sig` siblings and **skip** any pak with a matching `.sig`. If signed and we touched it, we already know the game won't launch. |
| Crash at level load (bad bulk-data offset) | Re-parse the rewritten pak with `repak info` + per-entry CRC. UE stores per-record hashes in the pak index; mismatched offset means the index points outside the new file. Catchable in milliseconds per pak. [Source: https://github.com/trumank/repak] |
| Checkerboard / missing textures (decoder mismatch) | Re-decode the rewritten `.ubulk` mip0 through a BCn decoder; if it fails to decode or yields all-zero pixels, abort. Cheap. Pixel-level SSIM only needed for quality regressions, not correctness. |
| Silent / distorted audio | After re-encoding `.uexp` audio payloads, parse the WAV/OGG header. Sample-rate, channel count, and duration must match the original within rounding. ~1 ms per asset; catches 95% of audio breakage without PEAQ. |
| Stutter on load (slow zstd path) | UE's runtime zstd path is markedly slower than Oodle Kraken/Mermaid: Kraken ~1.5 GB/s, Mermaid/Selkie faster, vs zlib ~300 MB/s on the same i7-8750H. ZSTD also gives smaller gains on UE's small-block pak layout. [Source: https://en.imzlp.com/posts/30732/] We **do not transcode Oodle → zstd in Phase 1**. If a pak is already Oodle, leave the compression algorithm alone and only repack what we can re-derive. |
| Save game won't load (GUID drift) | Asset GUIDs live in the `.uasset` header; never touch the header. Rewrite only `.ubulk` (texture mips) and audio payloads. If we restrict ourselves to payload bytes, GUID drift is structurally impossible. |

Rule of thumb: any optimization where the cheap signal cannot exist (no parseable header, no per-entry hash) is a Phase 2 feature.

## 2. Per-asset validation tiers

Three tiers, applied in order, short-circuit on the cheapest one that catches the bug.

- **Tier 1 — file-level (always on, ~µs):** SHA256 + byte size of original and rewritten payload. Logged into `shrinkray_backup/manifest.json`. This is also what `shrinkray restore` reads.
- **Tier 2 — format re-decode (always on, ~ms):** Texture path: BC1/3/5/7 decode the new mip0 with a pure-Rust BCn decoder. Audio path: parse RIFF/Ogg header; assert sample rate, channel count, duration delta < 1 ms. Any parse failure aborts the asset and **leaves the original in place**.
- **Tier 3 — pixel-level (opt-in, ~10-100 ms):** SSIM between original-decoded-BCn and rewritten-decoded-BCn. Use `image-compare` (MIT, v0.5.0, Aug 2025, rayon-parallel, has both SSIM and a YUV hybrid metric). [Source: https://lib.rs/crates/image-compare] DSSIM is more perceptually accurate (multiscale, L\*a\*b\*) but is AGPL/commercial dual-licensed — avoid for a MIT/Apache shrinkray. [Source: https://github.com/kornelski/dssim] Threshold: abort if SSIM < 0.95 on any 2K+ texture. For reference, BC7 and ASTC 4×4 normally land at PSNR > 42 dB which is well above that threshold; we catch only genuine breakage. [Source: https://aras-p.info/blog/2020/12/08/Texture-Compression-in-2020/]

Audio perceptual diff (PEAQ-equivalent) is explicitly **out of scope**. Sample-rate/channels/duration match is the proxy.

## 3. Launch validation — the only real test

No tier replaces actually booting the game. Plan:

1. Spawn the game binary, set a 30-second crash watchdog. Exit code != 0 or process death = fail.
2. After 15 s, capture the active window with `maim -i $(xdotool getactivewindow)` on Linux; on Windows use `nircmd savescreenshot` or the Windows.Graphics.Capture API. Compare to a baseline PNG with SSIM tolerance ≥ 0.9 (loose — main menus animate). [Source: https://linuxconfig.org/how-to-take-screenshots-using-maim-on-linux]
3. Optional Phase 2: scripted "press Enter, wait 30 s, screenshot" via `xdotool key`. Validates first level loads.
4. Idle 60 s, sample process RSS and exit code. Any crash → fail.

UE games do **not** generally expose `--screenshot-and-quit` for shipping builds; the editor has it, the cooked build does not. Epic's internal "Screenshot Comparison Tool" is editor-only. [Source: https://dev.epicgames.com/documentation/en-us/unreal-engine/screenshot-comparison-tool-in-unreal-engine] For commercial titles we are stuck with external screenshot tooling.

**Phase 1 stance:** automate the watchdog + crash-exit-code check only. Screenshot diffing is Phase 1.5. Human spot-check is acceptable for the first ship.

## 4. Reference CI corpus

Free, redistributable, packageable for PC, large enough to exercise the optimizer:

1. **Lyra Starter Game** — Epic's flagship UE5 sample, free on Fab/Marketplace, fully cookable to PC, contains BCn textures, Oodle paks, and sound cues. Primary CI target. [Source: https://www.fab.com/listings/93faede1-4434-47c0-85f1-bf27c0820ad0] [Source: https://dev.epicgames.com/documentation/en-us/unreal-engine/lyra-sample-game-in-unreal-engine]
2. **EpicSurvivalGame (tomlooman)** — pure-C++ UE sample, MIT-style permissive use, smaller footprint, good for fast CI runs. [Source: https://github.com/tomlooman/EpicSurvivalGame]
3. **Bomber / Eternal Crusade Resurrection / Aura** — community open-source UE5 projects; useful as a third datapoint to ensure we are not overfitting validation to Epic-cooked output.

**Do not** include archived Fortnite builds or any other Epic-published commercial binary — Epic's EULA forbids redistribution and reverse-engineering tooling targeted at their shipped games. Same goes for ripped AAA titles; the user's "modern AAA UE5" corpus is local-only and must never be checked into the CI repo.

## 5. "Backup or refuse" policy

Three options evaluated:

- **Take our own copy first** — safest, but a 100 GB AAA folder doubles disk usage. Reject as default.
- **Require `--i-have-backup` flag** — user-hostile and unenforceable.
- **Detect `shrinkray_backup/` sibling folder** — recommended. shrinkray refuses to write unless `shrinkray_backup/` exists at the target's parent and contains a `manifest.json` produced by us. On first run we offer an interactive choice: (a) make a full copy now, (b) make a *differential* backup that stores only the bytes we are about to overwrite (Tier 1 manifest + original payload blobs), or (c) abort.

The **differential backup** is the realistic default. We only overwrite specific `.ubulk` ranges and audio payloads, so the backup is on the order of "bytes we touched", typically 5-15% of the folder. This makes the safety net cheap enough to be the always-on default.

## 6. Dry-run mode

`shrinkray optimize --dry-run <folder>` runs Tiers 1+2 against a virtual rewrite, accumulating would-be-savings per category (textures, audio, padding, duplicate assets) and emits a JSON + human report. No writes, no backup needed.

In the Tauri UI this maps to a **"Preview only"** toggle pinned next to the "Optimize" button. Default ON for first-time users (detected via absence of `~/.config/shrinkray/state.json`); off thereafter. The preview produces the same diff report UI as a real run, with a banner "Nothing was written" and a one-click "Apply these changes" CTA.

## 7. Diff report UI

After any run (dry or real) the frontend renders a single scrollable report:

- **Header card** — total bytes saved, % reduction, asset count modified, duration.
- **Category breakdown** — horizontal stacked bar: textures / audio / pak padding / duplicate removal. Click a segment to filter.
- **Per-asset table** — virtualized list (5k+ rows possible): path, original size, new size, delta, Tier 2 status (OK / parse-fail / skipped), Tier 3 SSIM if computed.
- **Failures pane** — collapsed by default, expands red if non-zero. Each failure has "reason" + a "revert this asset" button.
- **Footer** — "Restore backup" button always present, links to `shrinkray restore <folder>`.

Use IMPECCABLE audit pass on this view before shipping — it is the only screen most users will ever judge the tool by.

## 8. Reversibility — `shrinkray restore`

Phase 1, not Phase 2. The differential backup format from §5 is designed exactly for this: walk `shrinkray_backup/manifest.json`, for each entry seek to the recorded offset in the target file and write back the original payload bytes, then truncate or extend the file to the original length. Restoring is sequential file IO with no decompression — should run faster than the original optimize pass. Verify by hashing each file post-restore against the manifest's `original_sha256`. If any hash mismatches we surface a hard error and leave the folder in a known-bad state rather than silently completing.

`shrinkray restore` must be wired to a button in the diff report UI from day one, not buried in a CLI flag.

## Phase 1 validation MVP

The minimum that must ship with the first "Optimize" button:

1. **Refuse-without-backup** policy via differential `shrinkray_backup/manifest.json`, with interactive offer to create one.
2. **Tier 1 (hash + size) and Tier 2 (re-decode / header re-parse)** validation on every modified asset; any Tier 2 failure auto-reverts that asset and logs it.
3. **No Oodle → zstd transcoding.** Keep the original pak's compression algorithm. Only repack assets whose compression we can faithfully reproduce (uncompressed payload edits inside an otherwise-untouched pak via repak's in-place index rewrite). [Source: https://deepwiki.com/trumank/repak/4.1-oodle-integration]
4. **Skip signed paks** (`.sig` sibling present) entirely.
5. **`--dry-run` flag** and Tauri "Preview only" toggle, default-on for first-time users.
6. **Diff report UI** with category breakdown, per-asset table, failures pane, and always-visible Restore button.
7. **`shrinkray restore <folder>`** command + UI button, with post-restore hash verification.
8. **Crash watchdog launch test** scripted against the Lyra cooked build in CI: optimize → boot → 30 s no-crash → exit-code check. Tier 3 SSIM and screenshot-diff are Phase 1.5.

Anything beyond this list — Oodle re-encode, audio transcoding, multi-pak dedupe, perceptual audio metrics — is Phase 2 and must not gate the first ship.
