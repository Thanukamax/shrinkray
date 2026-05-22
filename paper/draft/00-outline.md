# Δ-Codec paper — outline (working draft)

**Status:** skeleton. Section bodies arrive after measurement run lands in `paper/results/`.
**Target venues:** I3D 2027 (short paper) → HPG 2027 → SIGGRAPH 2028 (full paper if results extend).

---

## Title (working)

> One Bitstream, Two Restores: AI-Restorable and Byte-Exact Mip Recovery for Cooked Game Assets

Alt:
- "Δ-Codec: Predictor + Residual Coding for Anti-Cheat-Safe Texture Backups"
- "Quantized Residual Coding over AI Mip Predictions"

## Abstract (200 words, write last)

Frame: prior art forces a pick between lossy AI restore (FitGirl-style repacks; bytes change; anti-cheat flags) and full byte-exact backup (every byte stripped is stored elsewhere; net disk savings ≈ 0). We present a single bitstream that satisfies both. A low-mip is kept on disk; a residual encodes the per-pixel (or per-BC-byte) delta against a predictor's reconstruction from that low mip. With `quant_step = 1` reconstruction is byte-exact; with `quant_step > 1` it degrades smoothly. An adaptive selector (high-pass energy probe) chooses pixel-space vs BC-byte-space residuals per texture.

Headline numbers (bilinear predictor, 2026-05-23 bench, n=51, ESRGAN pending):
- **byte-exact restore at 10.5% of full-backup size on the median** (pixel-space, q=1)
- **51 / 51 lossless reconstructions** — zero codec-broken failures
- PBR diffuse median: **38.9%** of backup (2.5× win); PBR normal: 69.4% (1.4× — ESRGAN target)
- High-freq noise: 97.4% (within slack of entropy floor — codec doesn't leak compression)
- Adaptive routing (G13) needed for normals — BC5 / BC1 / BC7 all exceed 1.0× there
- **ESRGAN does NOT beat bilinear at scale.** 56-sample PBR run: bilinear wins **50/56 (89%)**, ESRGAN wins 6/56 (11%), oracle lift just **5.3%** over forced bilinear. Initial 19-sample suggested 24% lift but expansion proved it small-sample bias.
- **The paper's contribution is the byte-exact codec + predictor-agnostic format**, not any specific predictor. 126/126 lossless across all runs.
- **Bilinear is the strong baseline.** PBR content is engineered to be smooth/tileable, which is exactly what bilinear is mathematically optimal for.
- **hp_energy probe doesn't route bilinear-vs-ESRGAN** — paper-worthy negative result.
- Single bitstream serves byte-exact + lossy + per-texture predictor — three knobs from one format

## 1. Introduction

- The two camps and why they hate each other (anti-cheat vs disk savings)
- Why nobody fused them: AI predictors weren't reliable enough until recently
- Our contribution: not the predictor, not the residual coding — the *combination* with one bitstream serving both restores
- Three concrete claims:
  1. Adding a residual lane to AI restore makes byte-exact recovery possible without a full backup
  2. The residual stays small enough to beat full-backup on natural-content textures
  3. Adaptive routing (pixel vs BC-byte) covers content classes neither space handles alone

## 2. Prior art

- **Residual coding in video** — H.264 onward; what's different here (single-shot, not temporal)
- **Neural image compression** — Toderici, Ballé, JPEG-AI WG; residual + learned codec but goal is rate-distortion not byte-exact
- **JPEG XL lossless JPEG repack** — closest analog conceptually; recover original JPEG bytes from a smaller container
- **JPEG2000 scalable coding** — progressive lossless on top of lossy preview
- **rsync / xdelta / bsdiff** — delta against a known baseline; tree where ours is forest
- **Game asset repacking (FitGirl, DODI)** — lossy only, content-aware but no exact-restore guarantee
- **What we add**: per-texture adaptive routing + the "AI predictor as a free baseline" framing for cooked-asset workflows

## 3. Δ-Codec design

### 3.1 Bitstream layout

- Magic, version, predictor id, quant_step, low-mip dims + bytes, residual zst, optional sha256 receipt
- BC variant: same but residual is over BC-encoded bytes

### 3.2 Encode

- predictor(low) → predicted_high
- residual = original_high − predicted_high (i16 per channel)
- quantize(residual, step) → zstd
- record hash if requested

### 3.3 Decode

- predictor(low) → predicted_high (must be deterministic + match encoder)
- read residual, unzstd, dequantize, add back
- verify hash if present

### 3.4 Predictor agnosticism

- Trait-based; bilinear baseline + ESRGAN-x4 production
- BC determinism story (G4 milestone): with a deterministic BC encoder, even the BC-byte variant is byte-exact

### 3.5 Adaptive codec-space selector (G13)

- High-pass energy probe O(pixels), no allocations
- Threshold tuned empirically; per-texture routing

## 4. Evaluation

### 4.1 Corpus

- Synthetic content classes (smooth / textured / noise / normals / hard edges) — 30+ images
- CC0 PBR set (AmbientCG/Polyhaven) — ~20 textures, diffuse + normal + roughness
- UAssetAPI test fixtures (real UE-cooked bytes, limited corpus)
- (Future: Lyra cooked PC build — needs Epic account + UE5 + manual cook)

### 4.2 Baselines

- Full-backup (the bytes shrinkray would otherwise store elsewhere)
- Pure zstd-19 on original mip (no prediction)
- Bilinear-only Δ-codec
- ESRGAN-only Δ-codec
- Adaptive (probe-routed)

### 4.3 Metrics

- `ratio = (low_mip_bytes + residual_bytes) / baseline_bytes` at quant_step=1 (byte-exact mode)
- PSNR / SSIM at quant_step ∈ {2,4,8,16} (lossy mode)
- Lossless verdict (decoded bytes == original bytes)
- Encode/decode wall-clock per MP

### 4.4 Results

**Bilinear predictor run (2026-05-23, n=51):** see `paper/results/SUMMARY-2026-05-23.md` and `paper/results/bench-1779474831.csv`.

Findings:

1. **Pixel-space dominates** on smooth + textured content (median 0.10× at q=1). Codec is real on bilinear alone.
2. **BC3 leads block formats** (median 0.15×). BC7 is weakest (0.65×) — already-tight baseline leaves less to win.
3. **PBR normal maps are the codec's hardest content class** (0.69× pixel-space; >1.0× for BC1/BC5/BC7). The bilinear predictor cannot model per-pixel normal-vector noise; ESRGAN may or may not help — open question for the predictor-comparison ablation.
4. **High-frequency noise hits 0.97×** — within slack of the entropy floor. Codec is not leaking compression on incompressible content.
5. **q-sweep**: q=1 median 0.105, q=2 median 0.098, q=4 median 0.036. Lossy mode is gravy on top of the byte-exact win.
6. **BC5 on normals exceeds 1.0×** (1.22 median). Either we route around it (probe + pixel-space override) or motivate a normal-aware predictor in §5 future work.

**ESRGAN run (2026-05-23):** see `paper/results/SUMMARY-esrgan-vs-bilinear-2026-05-23.md` (initial 19-sample) and `paper/results/SUMMARY-oracle-routing-v2-2026-05-23.md` (expanded 56-sample).

**Hypothesis was wrong, and it falsifies more cleanly at scale.** ESRGAN underperforms bilinear on **50/56 PBR textures (89%)**. The 6 ESRGAN wins concentrate in: MetalPlates006 (3 of 6), specific normal maps with fine surface detail (Fabric, PaintedPlaster, Tiles), and one metalness map. Roughness goes **17/17 bilinear**.

**Oracle-routing analysis (best of bilinear or ESRGAN per texture):**
- 19-sample: 24.2% relative lift over forced bilinear (looked promising)
- 56-sample: **5.3% relative lift** (small-sample bias revealed — not worth a routing layer by itself)

**`HIGH_PASS_THRESHOLD` probe cannot route bilinear-vs-ESRGAN.** Best single-threshold routing accuracy is **87.5%**, vs majority-baseline (always-bilinear) **89.3%**. The probe signal that worked for pixel-vs-BC-byte does not generalize to predictor choice.

**Revised paper framing.** The contribution is NOT "ESRGAN beats bilinear" or "smart router beats either predictor." The contributions that survive empirical scrutiny are:
1. **Byte-exact restore at q=1 across 126 total measurements with zero failures.** Rock-solid.
2. **Predictor-agnostic bitstream architecture** — the format supports future game-content-trained predictors without recompiling the decoder.
3. **Bilinear is a shockingly strong baseline** for 1K-scale PBR game content because PBR is engineered to be smooth/tileable. This is the empirical finding the paper actually owns.
4. **Negative result on hp_energy as a routing signal** — useful for the future-work section + as a cautionary tale for anyone designing similar probes.

Plots to generate from CSVs:
- Fig. 3 — scatter: log baseline vs ratio, colored by content class, marker by predictor
- Fig. 4 — per-class boxplots: pixel / BC3 / adaptive
- Fig. 5 — quant-step sweep with PSNR overlay (ESRGAN run only)
- Fig. 6 — high-pass-energy histogram with `HIGH_PASS_THRESHOLD=25` line
- Fig. 8 (appendix) — per-channel entropy on PBR diffuse + PBR normal (motivates channel-adaptive `quant_step` in §5)

## 5. Limitations

- High-frequency content (foliage, grain) — residual blows up; codec collapses to backup-equivalent
- Custom UE compression settings (normal maps, masks) need channel-aware predictor — not addressed
- ESRGAN deterministic-enough for byte-exact? Verify across CPU/GPU runs in ablations
- Encode cost dominated by predictor; not real-time

## 6. Conclusion

- Not "we solved compression." We found a useful point in the design space.
- Niche but real: anti-cheat-safe + sub-backup overhead.
- Reference implementation public at [TBD repo URL once extracted]

## Appendix candidates

- Full BC determinism proof / test setup (G4)
- Probe threshold derivation (G13)
- ONNX model checksums + URLs used
