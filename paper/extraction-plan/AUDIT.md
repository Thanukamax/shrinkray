# AUDIT — `shrinkray-delta-codec` dependency + coupling review

Pre-extraction audit. Goal: enumerate every dep + every UE/shrinkray-internal
coupling point in the crate, so we know exactly what has to move, what has to
get renamed, and what has to be generalized before the crate can stand alone
as a public library. Read-only — no edits made.

Source: `crates/shrinkray-delta-codec/Cargo.toml` + the three source files
(`src/lib.rs`, `src/bc_residual.rs`, `src/probe.rs`) + `tests/g4_bc_determinism.rs`
+ `examples/delta_codec_bench.rs`.

---

## 1. `[dependencies]` — runtime deps

| Dep | Source | Workspace? | Purpose | Generic vs UE-specific | Extraction concern |
|-----|--------|------------|---------|------------------------|---------------------|
| `serde` | crates.io `1` (workspace) | yes | `Serialize`/`Deserialize` derives on the bitstream structs + the `PredictorId` enum | generic | drop workspace ref, pin direct (`serde = { version = "1", features = ["derive"] }`) |
| `anyhow` | crates.io `1` (workspace) | yes | error type used throughout (`Result<T>`) | generic | drop workspace ref, pin direct. Optional follow-up: swap to `thiserror` + a crate-local error enum for a tighter public API. Leave for v0.2. |
| `sha2` | crates.io `0.10` (workspace) | yes | the SHA-256 hash receipt that powers the "byte-exact verified" claim | generic | drop workspace ref, pin direct |
| `image_dds` | crates.io `0.7` (workspace) | yes | BC1/3/5/7 encode + decode under `bc_residual.rs`; the G4 determinism tests probe this lib's stability | generic (BC is a Direct3D/DDS standard, not UE) | drop workspace ref, pin direct. **One real concern**: `image_dds` itself depends on `intel_tex_2`-class native code paths in some feature combos — keep the default feature set and add a CI matrix entry to verify it builds clean on Linux/macOS/Windows GHA runners before tagging 0.1 |
| `zstd` | crates.io `0.13` (direct) | no | terminal entropy coder on the residual stream | generic | already direct; nothing to do |

**Verdict:** every runtime dep is general-purpose. Nothing in the dependency
graph ties the codec to UE pak / iostore / repak / CUE4Parse / shrinkray-core.
That part of the story is clean.

## 2. `[dev-dependencies]`

| Dep | Source | Purpose | Concern |
|-----|--------|---------|---------|
| `image` | workspace `0.25` w/ `png,jpeg` | example harness loads optional user-supplied PNG/JPG; not used in unit tests | drop workspace ref, pin direct; consider gating the `delta-codec-bench` example behind a feature so vanilla `cargo build` of consumers doesn't pull `image` transitively. (`cargo` already does this for examples by default — verify.) |
| `zstd` | `0.13` | redundantly listed; already a runtime dep | strip the dev-dep entry; runtime alone covers tests |

## 3. Workspace ties to break

The `Cargo.toml` currently inherits four fields from `[workspace.package]` via
`workspace = true`:

- `edition.workspace = true` → pin to `edition = "2021"`
- `license-file.workspace = true` → **changes** (the shrinkray repo ships a
  source-available "No Redistribution" license; the public crate must NOT
  inherit that — see `LICENSE-DECISION.md`)
- `authors.workspace = true` → pin to `authors = ["Thanuka Sehasna Perera"]`
  (or whichever identity the public crate ships under)
- `version` is currently `0.7.4-alpha`, hard-coded — but it tracks shrinkray's
  cadence. Reset to `0.1.0` on first publish so semver starts from a clean
  number that reflects the standalone crate's surface stability, not
  shrinkray's internal milestone counter.

Also remove the workspace deps (`serde`, `anyhow`, `sha2`, `image_dds`,
`image`) and re-pin each. A standalone library should NOT depend on a parent
workspace.

## 4. UE / shrinkray internal coupling — line-by-line

These are the actual rope-strands tying the crate back to the host repo.
Listed in descending order of severity.

### 4.1 `BcResidualFormat` enum (HIGH — needs a rename + a docstring rewrite)

`src/bc_residual.rs:32-49`. The variants are `Bc1 / Bc3 / Bc5 / Bc7`, and the
doc comment literally says "mirrors `shrinkray_core::bcn::BcFormat`". The
enum itself is fine — BC1/3/5/7 are DirectDraw/D3D standards, not UE-specific
— but:

- The doc comment will dangle once we cut the cord. Rewrite to reference
  Microsoft's BCn spec instead of shrinkray-core.
- Consider renaming to just `BcFormat` (it stops needing the "Residual" prefix
  once it's no longer being deconflicted with a parent crate's twin).
- Add the missing variants the public probably wants: `Bc4` (single-channel),
  `Bc6h` (HDR). `image_dds` supports both; the crate currently only exposes
  what shrinkray needs.

### 4.2 `probe::CodecSpace::Bc3Byte` variant name (MEDIUM — UE-flavoured leak)

`src/probe.rs:24`. The enum is `{ Pixel, Bc3Byte }`. The name `Bc3Byte` is
honest — the probe is currently tuned + benched against BC3 specifically
because that's what UE diffuse textures cook to — but a public crate
shouldn't bake one BC mode into the type system. Rename to `BcByte` (format-
agnostic), and let the caller select the BC format separately. The threshold
constant `HIGH_PASS_THRESHOLD = 25.0` was empirically tuned against shrinkray
bench samples (`smooth_gradient`, `textured_gradient`, `high_freq_noise`);
flag that in a doc-comment as a default that callers should re-tune for their
own content distribution, not a universal truth.

### 4.3 `PredictorId::RealEsrganX4` variant (MEDIUM — UE-AI-restore-shaped)

`src/lib.rs:48-49`. This variant exists because shrinkray ships a specific
Real-ESRGAN ONNX model in `shrinkray-core::inference`. A public crate
shouldn't promote one third-party model to a first-class enum variant.

Recommended generalization:
- Keep the variant for backwards-compat **OR** collapse it into the
  existing `Onnx4x { sha256: String }` variant. Net: the enum becomes
  `{ Bilinear, External { id: String } }` or
  `{ Bilinear, Onnx { sha256: String } }`.
- Either way, the API surface gets a docstring that says "this crate doesn't
  ship a neural predictor; bring your own, key it by hash". Honest and
  matches reality.

### 4.4 Comment-level UE references (LOW — docstring polish)

Strings to find-and-soften before publishing. None of these are load-bearing
code, just framing:

- `src/lib.rs:1-22` — module doc references "Real-ESRGAN family", "FitGirl
  repacks", "pak rewriters", and "shrinkray-core::inference". The pitch is
  good but needs to be reframed for a generic audience: keep the
  "anti-cheat-safe restore + smaller-than-backup" thesis, drop the named
  shrinkray subsystem.
- `src/lib.rs:19-20` — "production wires a 4× Real-ESRGAN ONNX inference via
  `shrinkray-core::inference`". Replace with a generic "your predictor wires
  in here" pointer.
- `src/bc_residual.rs:1-21` — references "cooked game pak" and "anti-cheat".
  The anti-cheat phrasing is fine for the README pitch but in a module
  docstring it reads provincial. Trim to "the use case is downstream
  byte-exact constraints, e.g. anti-cheat hash checks, file integrity
  scanners, deterministic build outputs".
- `src/bc_residual.rs:31` — "mirrors `shrinkray_core::bcn::BcFormat` but we
  keep it private so the codec crate stays decoupled from shrinkray-core."
  This is the exact line that betrays the extraction. Rewrite as "BC format
  selection. Mirrors the relevant Microsoft BCn formats; consumers may pass
  any variant supported by `image_dds`."
- `src/probe.rs:1-13` — "every texture in a pak". Generalize to "every
  texture in a batch".

### 4.5 Bench harness (`examples/delta_codec_bench.rs`) — LOW

References "UE diffuse format" (line ~131), "cooked diffuse texture" (~370),
and the synthetic-samples narrative is shrinkray-shaped. The example is
useful as a self-contained reproducer; just reword the inline comments.

## 5. What's NOT coupled (good news section)

- No `repak`/`CUE4Parse`/`UAssetAPI` dependency. The codec operates on raw
  RGBA + BC bytes — it doesn't know what a `.uasset` is. This is exactly
  the right shape for a standalone crate.
- No filesystem layer. Pure in-memory codec.
- No shrinkray-core import. The crate already lives behind a clean boundary.
- No async runtime, no tokio, no GPU dep. The README pitch ("synchronous,
  CPU-bound, predictor-agnostic") is honest.
- No `unsafe`. Easy to audit + easy to license.
- The G4 determinism tests stand on their own — they probe `image_dds`'s
  encoder, which is the property the BC-byte variant rests on. They keep
  working unchanged post-extraction.

## 6. Pre-publish checklist (referenced from `EXTRACTION-PLAN.md`)

- [ ] Rewrite the four docstrings flagged in §4.4.
- [ ] Decide rename for `BcResidualFormat` → `BcFormat` (yes/no).
- [ ] Decide whether `PredictorId::RealEsrganX4` survives or collapses
      into `Onnx { sha256 }`.
- [ ] Decide rename `CodecSpace::Bc3Byte` → `CodecSpace::BcByte`.
- [ ] Drop workspace inheritance in `Cargo.toml`; pin each dep direct.
- [ ] Reset version to `0.1.0`.
- [ ] Replace `license-file = "LICENSE"` with `license = "<SPDX>"` per
      `LICENSE-DECISION.md`.
- [ ] Verify `cargo doc --no-deps` produces clean output with the new
      docstrings.
- [ ] Verify `cargo build` + `cargo test` + `cargo clippy` clean on
      Linux/macOS/Windows.
