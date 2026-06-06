# Oracle routing analysis — 2026-05-23

For each PBR texture, pick `min(bilinear_ratio, esrgan_ratio)` as the
*oracle* — the best achievable ratio if the router knew the right
predictor in advance. This is the upper bound any data-driven router
(probe-based, content-classifier, learned) can chase.

## Aggregate

| Strategy | Median ratio | Bilinear-wins | ESRGAN-wins |
|----------|-------------:|---:|---:|
| Forced bilinear  | 0.3008 | — | — |
| Forced ESRGAN    | 0.5043 | — | — |
| **Oracle (best-of-both)** | **0.2282** | 14/19 | 5/19 |

**Oracle lift vs best forced predictor (bilinear): 0.0727 absolute (24.2% relative).**

## Per-class breakdown

| Class | n | Bilinear | ESRGAN | **Oracle** | ESRGAN wins |
|-------|--:|---------:|-------:|-----------:|------------:|
| color | 6 | 0.4686 | 0.5982 | **0.4686** | 1/6 |
| normal | 6 | 0.7152 | 0.7162 | **0.7143** | 3/6 |
| roughness | 6 | 0.1860 | 0.3506 | **0.1860** | 0/6 |
| metalness | 1 | 0.0421 | 0.0223 | **0.0223** | 1/1 |

## Per-texture pairs (oracle winner highlighted)

| Texture | Class | Bilinear | ESRGAN | Oracle | Winner | hp_energy |
|---------|-------|---------:|-------:|-------:|:-------|----------:|
| Bricks075A_Color | color | 0.6303 | 0.6878 | **0.6303** | bilinear | 7.13 |
| Fabric010_Color | color | 0.4436 | 0.6309 | **0.4436** | bilinear | 6.09 |
| Ground037_Color | color | 0.7459 | 0.7594 | **0.7459** | bilinear | 12.22 |
| MetalPlates006_Color | color | 0.2174 | 0.1320 | **0.1320** | 🟢 esrgan | 2.95 |
| PaintedPlaster017_Color | color | 0.2282 | 0.2915 | **0.2282** | bilinear | 1.55 |
| Wood062_Color | color | 0.4935 | 0.5654 | **0.4935** | bilinear | 7.06 |
| MetalPlates006_Metalness | metalness | 0.0421 | 0.0223 | **0.0223** | 🟢 esrgan | 1.36 |
| Bricks075A_NormalGL | normal | 0.7752 | 0.8051 | **0.7752** | bilinear | 8.75 |
| Fabric010_NormalGL | normal | 0.6647 | 0.6169 | **0.6169** | 🟢 esrgan | 4.24 |
| Ground037_NormalGL | normal | 0.7458 | 0.7497 | **0.7458** | bilinear | 11.54 |
| MetalPlates006_NormalGL | normal | 0.3008 | 0.1707 | **0.1707** | 🟢 esrgan | 7.52 |
| PaintedPlaster017_NormalGL | normal | 0.6847 | 0.6828 | **0.6828** | 🟢 esrgan | 8.43 |
| Wood062_NormalGL | normal | 0.7703 | 0.7715 | **0.7703** | bilinear | 13.20 |
| Bricks075A_Roughness | roughness | 0.2221 | 0.5043 | **0.2221** | bilinear | 4.09 |
| Fabric010_Roughness | roughness | 0.1603 | 0.4605 | **0.1603** | bilinear | 2.21 |
| Ground037_Roughness | roughness | 0.2118 | 0.3281 | **0.2118** | bilinear | 4.50 |
| MetalPlates006_Roughness | roughness | 0.1516 | 0.3546 | **0.1516** | bilinear | 5.90 |
| PaintedPlaster017_Roughness | roughness | 0.1164 | 0.2664 | **0.1164** | bilinear | 0.78 |
| Wood062_Roughness | roughness | 0.2248 | 0.3466 | **0.2248** | bilinear | 7.55 |

## Can `hp_energy` (existing probe signal) predict the winner?

- Bilinear winners (n=14): hp_energy range [0.78, 13.20], median 6.58
- ESRGAN winners (n=5): hp_energy range [1.36, 8.43], median 4.24

**Best threshold on hp_energy alone:** t = 12.71 → 68.4% routing accuracy. (Random baseline: 73.7% by always picking the majority winner.)

## Interpretation

The oracle outperforms forced bilinear by 24.2% on the median PBR texture. **A real router worth building.**

Either way, the **bitstream-level support for per-texture predictor choice remains the architectural contribution** — what changes with this data is whether a router *needs to be built today* or stays as future work.
