# Paper track

Long-arc graphics paper on Δ-Codec. Target venues I3D 2027 → HPG → SIGGRAPH 2028.

## Layout

- `draft/` — paper sections (outline, method, biblio seed, figure plan)
- `results/` — bench CSVs + summary markdowns from `bench_real_content` runs
- `corpus/` — test corpus (synthetic + CC0 PBR + uasset test fixtures)
- `figures/` — generated PDF/PNG figures + the render.py script that produces them
- `extraction-plan/` — research docs for spinning the codec out as a public repo (gated on results landing)

## What to do when results land

1. `paper/results/SUMMARY-<date>.md` headline numbers go into outline §4.4 placeholders
2. `paper/figures/render.py paper/results/<csv>` produces Figs 3-8
3. Run with ESRGAN predictor enabled — repeat all numbers; ESRGAN columns get added to tables
4. Write the abstract last; it should fit on a postcard once results are known
5. Decide on extraction (see `extraction-plan/EXTRACTION-PLAN.md`)

## Constraints

- Honest framing only. No "revolutionary". The novelty is narrow: one bitstream serves both lossy + byte-exact restore for cooked-asset workflows. Anyone overclaiming gets dunked by NIC reviewers.
- All corpus content must be paper-publishable (CC0 for PBR, synthetic generated in-repo, UE fixtures are derivative of UAssetAPI's own test data which is MIT).
- DO NOT use Wuthering Waves / closed-source game data for measurements — never paper-publishable, anti-cheat risk on the user's machine.
