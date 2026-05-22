# Oracle routing analysis — 2026-05-23

For each PBR texture, pick `min(bilinear_ratio, esrgan_ratio)` as the
*oracle* — the best achievable ratio if the router knew the right
predictor in advance. This is the upper bound any data-driven router
(probe-based, content-classifier, learned) can chase.

## Aggregate

| Strategy | Median ratio | Bilinear-wins | ESRGAN-wins |
|----------|-------------:|---:|---:|
| Forced bilinear  | 0.2453 | — | — |
| Forced ESRGAN    | 0.3984 | — | — |
| **Oracle (best-of-both)** | **0.2323** | 50/56 | 6/56 |

**Oracle lift vs best forced predictor (bilinear): 0.0130 absolute (5.3% relative).**

## Per-class breakdown

| Class | n | Bilinear | ESRGAN | **Oracle** | ESRGAN wins |
|-------|--:|---------:|-------:|-----------:|------------:|
| color | 17 | 0.4293 | 0.4797 | **0.4293** | 1/17 |
| normal | 17 | 0.5943 | 0.6169 | **0.5943** | 4/17 |
| roughness | 17 | 0.1942 | 0.3536 | **0.1942** | 0/17 |
| metalness | 5 | 0.0421 | 0.0223 | **0.0223** | 1/5 |

## Per-texture pairs (oracle winner highlighted)

| Texture | Class | Bilinear | ESRGAN | Oracle | Winner | hp_energy |
|---------|-------|---------:|-------:|-------:|:-------|----------:|
| Asphalt022_Color | color | 0.4383 | 0.4757 | **0.4383** | bilinear | 3.82 |
| Bricks075A_Color | color | 0.6303 | 0.6878 | **0.6303** | bilinear | 7.13 |
| Carpet013_Color | color | 0.6675 | 0.6787 | **0.6675** | bilinear | 23.33 |
| Concrete028_Color | color | 0.3191 | 0.4600 | **0.3191** | bilinear | 1.68 |
| Fabric010_Color | color | 0.4436 | 0.6309 | **0.4436** | bilinear | 6.09 |
| Ground037_Color | color | 0.7459 | 0.7594 | **0.7459** | bilinear | 12.22 |
| Leather011_Color | color | 0.2542 | 0.3160 | **0.2542** | bilinear | 0.85 |
| Marble019_Color | color | 0.3124 | 0.3826 | **0.3124** | bilinear | 3.70 |
| Metal027_Color | color | 0.2668 | 0.2771 | **0.2668** | bilinear | 1.69 |
| Metal032_Color | color | 0.1917 | 0.2245 | **0.1917** | bilinear | 0.84 |
| MetalPlates006_Color | color | 0.2174 | 0.1320 | **0.1320** | 🟢 esrgan | 2.95 |
| MetalPlates013_Color | color | 0.4836 | 0.6038 | **0.4836** | bilinear | 2.96 |
| PaintedPlaster017_Color | color | 0.2282 | 0.2915 | **0.2282** | bilinear | 1.55 |
| Rust004_Color | color | 0.4293 | 0.4797 | **0.4293** | bilinear | 1.92 |
| Tiles101_Color | color | 0.4603 | 0.6521 | **0.4603** | bilinear | 19.92 |
| Wood062_Color | color | 0.4935 | 0.5654 | **0.4935** | bilinear | 7.06 |
| WoodFloor046_Color | color | 0.4292 | 0.5962 | **0.4292** | bilinear | 2.95 |
| Metal027_Metalness | metalness | 0.0001 | 0.0004 | **0.0001** | bilinear | 0.00 |
| Metal032_Metalness | metalness | 0.0001 | 0.0004 | **0.0001** | bilinear | 0.00 |
| MetalPlates006_Metalness | metalness | 0.0421 | 0.0223 | **0.0223** | 🟢 esrgan | 1.36 |
| MetalPlates013_Metalness | metalness | 0.1958 | 0.3034 | **0.1958** | bilinear | 6.30 |
| Rust004_Metalness | metalness | 0.1247 | 0.1640 | **0.1247** | bilinear | 1.09 |
| Asphalt022_NormalGL | normal | 0.6598 | 0.6686 | **0.6598** | bilinear | 6.74 |
| Bricks075A_NormalGL | normal | 0.7752 | 0.8051 | **0.7752** | bilinear | 8.75 |
| Carpet013_NormalGL | normal | 0.8001 | 0.8030 | **0.8001** | bilinear | 18.07 |
| Concrete028_NormalGL | normal | 0.5943 | 0.6241 | **0.5943** | bilinear | 3.85 |
| Fabric010_NormalGL | normal | 0.6647 | 0.6169 | **0.6169** | 🟢 esrgan | 4.24 |
| Ground037_NormalGL | normal | 0.7458 | 0.7497 | **0.7458** | bilinear | 11.54 |
| Leather011_NormalGL | normal | 0.4672 | 0.5421 | **0.4672** | bilinear | 2.04 |
| Marble019_NormalGL | normal | 0.0991 | 0.1047 | **0.0991** | bilinear | 0.32 |
| Metal027_NormalGL | normal | 0.2717 | 0.2866 | **0.2717** | bilinear | 1.15 |
| Metal032_NormalGL | normal | 0.1006 | 0.1064 | **0.1006** | bilinear | 0.33 |
| MetalPlates006_NormalGL | normal | 0.3008 | 0.1707 | **0.1707** | 🟢 esrgan | 7.52 |
| MetalPlates013_NormalGL | normal | 0.4277 | 0.4636 | **0.4277** | bilinear | 1.63 |
| PaintedPlaster017_NormalGL | normal | 0.6847 | 0.6828 | **0.6828** | 🟢 esrgan | 8.43 |
| Rust004_NormalGL | normal | 0.6057 | 0.6191 | **0.6057** | bilinear | 4.73 |
| Tiles101_NormalGL | normal | 0.3041 | 0.2925 | **0.2925** | 🟢 esrgan | 2.27 |
| Wood062_NormalGL | normal | 0.7703 | 0.7715 | **0.7703** | bilinear | 13.20 |
| WoodFloor046_NormalGL | normal | 0.1026 | 0.1150 | **0.1026** | bilinear | 0.44 |
| Asphalt022_Roughness | roughness | 0.2148 | 0.3316 | **0.2148** | bilinear | 5.07 |
| Bricks075A_Roughness | roughness | 0.2221 | 0.5043 | **0.2221** | bilinear | 4.09 |
| Carpet013_Roughness | roughness | 0.1942 | 0.3199 | **0.1942** | bilinear | 3.77 |
| Concrete028_Roughness | roughness | 0.1998 | 0.4141 | **0.1998** | bilinear | 3.37 |
| Fabric010_Roughness | roughness | 0.1603 | 0.4605 | **0.1603** | bilinear | 2.21 |
| Ground037_Roughness | roughness | 0.2118 | 0.3281 | **0.2118** | bilinear | 4.50 |
| Leather011_Roughness | roughness | 0.1739 | 0.4752 | **0.1739** | bilinear | 2.16 |
| Marble019_Roughness | roughness | 0.1954 | 0.3776 | **0.1954** | bilinear | 2.78 |
| Metal027_Roughness | roughness | 0.1510 | 0.2509 | **0.1510** | bilinear | 2.13 |
| Metal032_Roughness | roughness | 0.1062 | 0.1584 | **0.1062** | bilinear | 0.86 |
| MetalPlates006_Roughness | roughness | 0.1516 | 0.3546 | **0.1516** | bilinear | 5.90 |
| MetalPlates013_Roughness | roughness | 0.2364 | 0.6200 | **0.2364** | bilinear | 5.18 |
| PaintedPlaster017_Roughness | roughness | 0.1164 | 0.2664 | **0.1164** | bilinear | 0.78 |
| Rust004_Roughness | roughness | 0.2157 | 0.4796 | **0.2157** | bilinear | 3.57 |
| Tiles101_Roughness | roughness | 0.1635 | 0.3536 | **0.1635** | bilinear | 6.04 |
| Wood062_Roughness | roughness | 0.2248 | 0.3466 | **0.2248** | bilinear | 7.55 |
| WoodFloor046_Roughness | roughness | 0.1163 | 0.1640 | **0.1163** | bilinear | 0.90 |

## Can `hp_energy` (existing probe signal) predict the winner?

- Bilinear winners (n=50): hp_energy range [0.00, 23.33], median 3.47
- ESRGAN winners (n=6): hp_energy range [1.36, 8.43], median 3.59

**Best threshold on hp_energy alone:** t = 21.63 → 87.5% routing accuracy. (Random baseline: 89.3% by always picking the majority winner.)

## Interpretation

The oracle outperforms forced bilinear by 5.3% on the median PBR texture. **A real router worth building.**

Either way, the **bitstream-level support for per-texture predictor choice remains the architectural contribution** — what changes with this data is whether a router *needs to be built today* or stays as future work.
