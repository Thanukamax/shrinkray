# Δ-Codec — combined AI-fast / byte-exact texture compression

**Status:** v0.7.4-alpha prototype, 2026-05-22.
**Crate:** `crates/shrinkray-delta-codec`.
**Experiment binary:** `cargo run -p shrinkray-delta-codec --example delta-codec-bench`.

## The claim

Industry compression tools force a choice between two incompatible goals:

| Approach | Disk savings | Byte-exact restore? | Anti-cheat safe? |
|---|---|---|---|
| Mip strip + ExactBackup (shrinkray v0.7.3) | Negative — backup eats the save | Yes | Yes |
| AI re-expansion (Real-ESRGAN, Cupscale, FitGirl-style) | Large | No (hallucinated pixels) | No (hashes drift) |
| Oodle Texture (RAD/Epic) | Moderate | Yes | Yes | (paid, license-locked, requires engine integration) |

The literature treats *byte-exact* and *significantly smaller than the original* as mutually exclusive for content-aware lossy codecs. Δ-Codec is the falsifiable claim that they are not.

**Δ-Codec ships, in one bitstream, both:**
1. A *prediction* of the original top mip, produced cheaply from a kept low mip via an upscaler (bilinear baseline; Real-ESRGAN production), and
2. A *residual* — the per-pixel difference between the original and the prediction — compressed losslessly with zstd.

Restore re-runs the predictor and adds the residual back. The reconstruction is **byte-exact in RGBA space** when `quant_step == 1`, and lossy-but-bounded for larger steps. SHA-256 of the restored RGBA must equal the recorded original hash; the encoder records this hash at compress time.

## Why this is novel

To the best of our knowledge, no published prior work combines:

- Per-class restore routing keyed on `UTexture.CompressionSettings`
- Predictor + residual coding *in one bitstream* aimed at distribution
- Byte-exact verification via per-texture hash receipt
- Targeting cooked UE pak content (vs. raw .uasset or runtime GPU paths)

Closest prior art:
- **Video residual codecs (H.264, AV1):** per-frame delta coding, not per-asset, no class routing, no hash receipt.
- **NVIDIA Neural Texture Compression (SIGGRAPH 2023):** runtime GPU decode path requires custom hardware support and engine integration. Δ-Codec targets install-time decode to standard BCn, no engine modification.
- **JPEG XL progressive coding:** lossy-only, no byte-exact mode for the high-quality tier.
- **Cupscale / chaiNNer / Real-ESRGAN tooling:** pure AI upscale, no residual, no hash, no byte-exactness.

## Bitstream

```rust
pub struct DeltaBitstream {
    pub magic: u32,                 // 0x44434443 "DCDC"
    pub version: u16,               // 1
    pub predictor: PredictorId,     // Bilinear | RealEsrganX4 | Onnx4x{sha256}
    pub quant_step: u8,             // 1 = lossless, >1 = lossy
    pub low_w: u32,
    pub low_h: u32,
    pub top_w: u32,
    pub top_h: u32,
    pub low_mip_rgba: Vec<u8>,      // kept on disk, fed to predictor
    pub residual_zst: Vec<u8>,      // zstd-19 over int16 LE per channel
    pub original_sha256: Option<[u8; 32]>, // anti-cheat receipt
}
```

Predictor identity is part of the bitstream — decoder refuses to run with a mismatched predictor (the residual would land on the wrong baseline).

## Encode

```
encode(top_mip, low_mip, predictor, q):
    pred           = predictor.predict(low_mip → top_dims)
    residual_int16 = top_mip - pred                       // per channel, signed
    residual_quant = round(residual_int16 / q)            // identity when q=1
    residual_zst   = zstd(LE16(residual_quant))
    sha            = sha256(top_mip)                      // anti-cheat receipt
    emit bitstream
```

## Decode

```
decode(bitstream, predictor):
    assert predictor.id == bitstream.predictor             // catch baseline drift
    pred           = predictor.predict(bitstream.low_mip → top_dims)
    residual_quant = LE16⁻¹(zstd⁻¹(bitstream.residual_zst))
    residual_int16 = residual_quant * q                    // identity when q=1
    restored       = clamp(pred + residual_int16, 0..255)
    if bitstream.sha is set:
        assert sha256(restored) == bitstream.sha           // anti-cheat receipt
    return restored
```

## Measured results (2026-05-22, bilinear baseline predictor)

Important framing: shrinkray's strip operation leaves the low mip *inside* the
stripped pak. Δ-Codec only ships the *residual* as a sidecar — the low mip is
not paid for twice. The `payload_ratio` column therefore compares the residual
+ overhead against the ExactBackup payload that would otherwise be stored.

Two codec variants benched:
- **pixel** — residual in RGBA pixel space; baseline = top mip RGBA bytes; byte-exact reconstruction is in RGBA space.
- **bc3-byte** — residual in BC3 byte space (after deterministic BC encode on both sides); baseline = top mip BC3 bytes; byte-exact reconstruction is on the actual on-disk BC bytes (the stronger anti-cheat claim).

### Synthetic 256×256 samples

| Content | Space | q | Baseline | Residual payload | Payload ratio | H bits/byte | zstd eff | Byte-exact |
|---|---|---|---|---|---|---|---|---|
| smooth gradient | pixel | 1 | 256.0 KiB | 12.5 KiB | **0.05×** | 0.925 | 4.75× | RGBA ✅ |
| smooth gradient | pixel | 2 | 256.0 KiB | 12.4 KiB | 0.05× | 0.925 | 4.76× | (lossy) |
| smooth gradient | pixel | 4 | 256.0 KiB | 51 B | 0.00× | 0.000 | 0.15× | (lossy) |
| smooth gradient | bc3-byte | 1 | 64.0 KiB | 8.2 KiB | 0.13× | 0.861 | 1.69× | BC bytes ✅ |
| textured gradient | pixel | 1 | 256.0 KiB | 35.3 KiB | **0.14×** | 3.019 | 5.47× | RGBA ✅ |
| textured gradient | bc3-byte | 1 | 64.0 KiB | 9.4 KiB | 0.15× | 2.463 | 4.19× | BC bytes ✅ |
| high freq noise | pixel | 1 | 256.0 KiB | 274.0 KiB | 1.07× | 4.460 | 1.04× | RGBA ✅ |
| **high freq noise** | **bc3-byte** | 1 | 64.0 KiB | 44.5 KiB | **0.70×** | 3.259 | 1.17× | BC bytes ✅ |

### Real photographic content (Aero wallpaper @ 1024×1024)

| Space | q | Baseline | Residual | Ratio | Byte-exact |
|---|---|---|---|---|---|
| pixel | 1 | 4.00 MiB | 1.41 MiB | **0.35×** | RGBA ✅ |

### Key findings

1. **Sidecar framing exposes the real wins.** Earlier framing put 0.30× on smooth; the sidecar framing (residual-only, since low mip lives in the stripped pak) lands **0.05× on smooth** — a 95% saving over ExactBackup with byte-exact reconstruction.

2. **Pixel-space wins on predictable content. BC-byte-space wins on noisy content.** This was not obvious in advance. BC's quantization absorbs prediction error into block-coded deltas, so residual entropy stays bounded even on noise. Pixel-space loses on noise (1.07× — net worse than backup). BC-byte-space stays at 0.70× on the *same* noisy input.

3. **Implication:** a per-texture codec-space selector based on a cheap entropy probe is the next research move (see G13 in the gap inventory). It pushes ALL content into the "wins" column.

4. **zstd efficiency above 1.0× on every payable sample** indicates inter-byte correlations zstd is exploiting beyond byte-marginal entropy. A context-adaptive entropy coder (BCM, FSE-tuned) likely improves headlines further.

## G4 verified — image_dds is byte-deterministic

For the file-byte anti-cheat-safety claim to hold, the BC encoder must produce
identical bytes across invocations. `tests/g4_bc_determinism.rs` confirms
this for image_dds 0.7 across BC1, BC3, BC5, BC7, at both Fast and Slow
quality settings, on gradient AND noise inputs. 7/7 deterministic.

## Expected lift with a real predictor

The bilinear baseline is the floor. We expect Real-ESRGAN x4 to reduce residual entropy substantially because:

1. Bilinear underpredicts high-frequency detail. ESRGAN hallucinates plausible detail in the right places, so the *signed* residual averages closer to zero with smaller absolute values.
2. Smaller residual values quantize to a narrower symbol set, which zstd compresses harder.

Tentative target: 0.40-0.55× ExactBackup at q=1 (byte-exact) on real game textures. Measurement TBD on Pamali T_hairMask03 once the sidecar+ONNX integration lands (v0.7.4-c milestone).

## Failure modes (each is data, not failure)

1. **Residual entropy too high.** If real BC3-decoded textures look more like "high_freq_noise" than "textured_gradient", Δ-Codec loses. This tells us the BC quantization noise dominates the predictor error — the predictor needs to operate in *block* space, not pixel space. Publishable as a negative result + next-step direction.

2. **Predictor drift across builds.** If the model weights aren't byte-identical between encode and decode (different ORT version, GPU vs CPU, fused vs unfused conv), the residual lands on a different baseline and reconstruction breaks. Mitigation: pin model SHA in the bitstream `predictor` field. Decoder refuses to run with the wrong predictor. Publishable as a deployment-engineering finding.

3. **Per-texture inference cost.** Even cheap upscalers cost 30-80ms on CPU. A 2000-texture AAA install = ~3 min restore time. Slower than ExactBackup's `cp` of payloads. Mitigation: GPU batching at restore time; or pre-staging the restore to a temp dir.

4. **Anti-cheat hash-equality is *RGBA*-exact, not *BC-bytes*-exact.** Standard BC encoders are non-deterministic (block search heuristics vary). To get *file-byte* exactness we need a deterministic BC encoder. `intel_tex_2` is deterministic given fixed quality settings; `image_dds` is on the path. Open question: does the on-disk pak hash actually need to match the original, or is per-asset hash sufficient? Online AAA games hash whole paks; singleplayer doesn't.

## Roadmap

- **v0.7.4 (this branch):** Δ-Codec crate ships with pixel-space + BC-byte-residual variants, bilinear baseline, bench harness, entropy report, determinism tests, live demo panel in the Tauri app.
- **v0.7.5:** wire Real-ESRGAN as a `Predictor` impl. Run Pamali T_hairMask03 end-to-end. Measure real-game residual entropy across both variants.
- **v0.8.0:** **G13 — adaptive codec-space selector.** Cheap entropy/edge probe per texture routes between pixel-space (smooth content) and BC-byte-space (noisy content). Per-game savings should improve uniformly because nothing pays the "wrong-space" penalty.
- **v0.8.x:** integrate Δ-Codec into the manifest/restore pipeline. `shrinkray strip --codec delta-lossless` writes the residual into the backup payload; `shrinkray restore --codec delta-lossless` reconstructs from it. Optional perceptual quantization (G2) for lossy mode.
- **v0.9.0:** Lyra Starter Game end-to-end measurement (paper-track constraint #5). Then second game (asset-flip indie) to demonstrate cross-style generalization.
- **Paper draft target (post-Lyra-validation):** I3D 2026 short / HPG tools track / GDC research talk. Companion short note: "Anti-cheat-safe game asset compression via deterministic-BC + residual coding."

## Related

- `crates/shrinkray-delta-codec/src/lib.rs` — the encode/decode/quantize/dequantize/predictor-trait surface.
- `crates/shrinkray-delta-codec/examples/delta_codec_bench.rs` — measurement harness.
- `crates/shrinkray-core/src/inference.rs` — the ESRGAN ONNX runtime that will plug into `Predictor`.
- `docs/delta-codec-spec.md` — this file.

## Provenance

Authored 2026-05-22 during a v0.7.4 push. Falsifiable claim, measured result, byte-exact verified by SHA-256.
