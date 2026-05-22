# Figures — plan

**Status:** plan only. Actual figures generated after bench run lands in `paper/results/`.
**Tool:** matplotlib via a small Python harness reading the CSV from `paper/results/`.
**Style:** monochrome+1-accent, vector PDF, 88mm one-column / 180mm full-page widths.

## Fig. 1 — Concept diagram (hand-drawn, vector)

The two camps and the codec's position:
- Left: lossy AI restore (small, bytes change)
- Right: full backup (big, byte-exact)
- Center: Δ-Codec (small + byte-exact via residual)

Single panel, no data. Drawn in Excalidraw or Figma; export SVG → embed.

## Fig. 2 — Bitstream layout

Boxes-and-bytes diagram of `DeltaBitstream`. Annotate which fields are predictor-dependent, which are quantization-dependent.

## Fig. 3 — Headline scatter (the money plot)

X: `log10(baseline_bytes)` per texture
Y: `ratio_vs_baseline` at `quant_step=1`
Color: content class (smooth / textured / noise / normals / hard-edges / PBR-diffuse / PBR-normal / PBR-roughness)
Marker: predictor (○ bilinear, ▲ ESRGAN, ★ adaptive)
Reference lines: y=1.0 (break-even with backup), y=0.5 (50% savings), y=0.25 (4× savings)

Caption: "Single-bitstream byte-exact restore disk overhead vs full-backup baseline. Lower = better. Adaptive routing covers content classes neither space handles alone."

## Fig. 4 — Per-class boxplots

X: content class
Y: ratio_vs_baseline
Three boxes per class: pixel-only, BC-only, adaptive
Annotation: significance markers (Mann-Whitney) where adaptive wins

## Fig. 5 — Predictor ablation

X: quant_step ∈ {1, 2, 4, 8, 16}
Y left: total codec bytes (log)
Y right: PSNR
Two lines per quant_step: bilinear vs ESRGAN
Single content class (PBR-diffuse), median + IQR shaded

## Fig. 6 — Probe threshold visualization

Histogram of `high_pass_energy` across corpus
Two distributions: textures where pixel-space wins, textures where BC-byte wins
Vertical line at `HIGH_PASS_THRESHOLD=25`
Marginal: ratio improvement from adaptive vs forced choice

## Fig. 7 — Wall-clock cost

Bar chart: encode + decode time per MP for each predictor
Annotate predictor-call as the dominant cost

## Fig. 8 (appendix) — Per-channel residual entropy

For one PBR diffuse + one PBR normal, show entropy of R/G/B/A residuals separately.
Motivates the §5 future-work item on channel-adaptive `quant_step`.

## Generation script

`paper/figures/render.py` (to be written):
- argparse `--csv paper/results/bench-<date>.csv` `--out paper/figures/`
- Functions: `fig_3_scatter()`, `fig_4_boxplots()`, etc.
- Saves both PNG (for repo preview) and PDF (for the paper)
- Deterministic output (seed all RNGs even if not used)
