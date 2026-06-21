# F — Mip-strip runtime effect: the gating experiment

**Status:** designed, not yet run (blocked on dGPU visibility — see Prereqs).
**Date:** 2026-06-09

## The one question this answers

> Does asset-level top-mip stripping deliver a runtime VRAM / streaming win
> **meaningfully beyond the free `r.Streaming.MipBias` ini lever** — enough to
> justify pitching shrinkray as a performance tool, not just a disk tool?

Everything downstream (a "Performance mode" that targets non-streaming textures,
the paper angle) hinges on this. If a one-line ini gets ~90% of the runtime win,
shrinkray's runtime story collapses back to **disk + IO-bandwidth** (still honest,
still useful) and we pitch it that way.

### What we already know (don't re-derive)
- Mip0 = 75% of a texture's chain bytes. Arithmetic, not in question.
- Disk saving is real and shipped. Not what this tests.
- UE texture streaming is **async** → pool starvation degrades to *blurry pop-in*,
  NOT frame hitches. The canonical "UE traversal stutter" is shader/PSO + GC +
  level-streaming on the CPU — top-mip stripping does **nothing** for that.
  So frametime is a *secondary* signal here; the primary signals are VRAM
  footprint and streaming-pool behaviour.

## Salvageable thesis (what we're actually testing)

| Lever | Disk | VRAM | PCIe/IO bytes |
|---|---|---|---|
| `r.Streaming.MipBias 1` (free ini) | ❌ | ✅ | ❌ (full bytes still on disk) |
| shrinkray strip top mip | ✅ | ✅ | ✅ (bytes don't exist) |

The unique column is **IO bytes**. The test must isolate whether that third
column produces a runtime difference the ini lever can't, on a VRAM-starved card.

## Test subjects

Use the **local disposable demos** — unoptimized asset-flip archetype, where the
effect should be largest, and safe to mutate:
- `~/Downloads/Misc/Test/AUGUST NIGHT/`  (UnrealGame-Win64-Shipping)
- `~/Downloads/Misc/Test/Test 2/pjtRedLipstickDemo/`

**OFF-LIMITS without explicit per-game approval:** Stellar Blade, Black Myth:
Wukong (real installs on /media/zzz). Do not strip these as part of the gate.

n=1 game is a **signal, not proof** (see feedback_codec_bench_sample_size). If the
signal is positive, widen to ≥3 games before any public/paper claim.

## Prereqs

1. **dGPU visible.** `supergfxctl -g` must report `Hybrid`, `nvidia-smi` must work.
   `always_reboot:true` is set, so `supergfxctl -m Hybrid` reboots cleanly.
   The whole point is the 4 GB RTX 3050 Ti starved regime — do NOT measure on the iGPU.
2. **MangoHud** installed (frametime + VRAM + FPS → CSV under Proton).
3. shrinkray release binary built (`bun run tauri build`) for the GUI strip step.

## The 4 arms

Strip operates on a **copy** per arm (shrinkray backs up anyway). Same game, same
build, same hardware, same traversal path.

| Arm | Assets | ini | Tests |
|---|---|---|---|
| **A. stock** | original | default | baseline |
| **B. ini lever** | original | `r.Streaming.MipBias=1` | the free alternative |
| **C. strip** | shrinkray top-mip stripped | default | shrinkray alone |
| **D. both** | stripped | `r.Streaming.MipBias=1` | interaction / ceiling |

The decisive comparison is **B vs C**: same VRAM target, but C also shrank the
on-disk/IO bytes. If C ≈ B on every runtime metric, the IO column doesn't matter
at runtime and the gate FAILS (→ disk-only pitch).

### Setting the ini lever (shipping build, no editor)
Add to `<Game>/<Project>/Saved/Config/Windows/Engine.ini`:
```
[ConsoleVariables]
r.Streaming.MipBias=1
```
If the shipping build strips console/CVars, fall back to launch arg
`-execcmds="r.Streaming.MipBias 1"`. If neither takes (verify with `stat streaming`),
note it — the ini-lever arm is then untestable and B vs C can't be isolated.

### Forcing the starved regime
To make pool pressure visible on a 4 GB card, optionally cap the pool so stock
overflows: `r.Streaming.PoolSize=1500` (MB). Keep identical across all 4 arms.

## Measurement

Per arm, **3 runs**, same fixed traversal path (pick a route that loads new texture
sets — walk through 2–3 distinct areas). Wrap the Proton launch:

```bash
MANGOHUD_CONFIG=fps,frametime,vram,gpu_mem_clock,log_interval=100,output_folder=/tmp/mip-bench \
  mangohud %command%        # in Steam launch options, or `mangohud <binary>` direct
```

Capture, per run:
- **VRAM** — MangoHud `vram` column (peak + steady-state), cross-checked with a
  `nvidia-smi --query-gpu=memory.used --format=csv -l 1` poll logged to file.
- **Streaming pool** — in-game console `stat streaming`: Pool Used / Wanted, and any
  "Texture streaming pool over N MB" warning. Screenshot or note per area.
- **Frametime** — MangoHud CSV: median + 1%-low + max hitch. (Secondary — expect
  little movement unless genuinely IO/VRAM-bound.)
- **Visual** — same screenshot spot each arm; eyeball pop-in / blur (C and D will be
  lower texture quality by construction — that's the tradeoff, record it).

Analysis script (CLI `audit` is read-only and scriptable):
```bash
shrinkray audit "<arm-folder>"   # disk + asset stats per arm
# + a small parser over /tmp/mip-bench/*.csv for VRAM/frametime percentiles
```

## Pass / fail (decides the next two weeks)

**PASS (runtime story is real → build Performance mode):**
- C shows materially lower steady-state VRAM AND/OR fewer pool-overflow events
  than **B**, not just than A — i.e. the IO column buys something the ini can't; OR
- under the capped-pool starved regime, C resolves textures / avoids pop-in where
  B still thrashes.

**FAIL (collapse to disk + IO pitch, drop the "FPS booster" framing):**
- C ≈ B on VRAM, pool, and frametime → the win is entirely the free ini lever,
  shrinkray's runtime delta is noise. Disk saving remains the (honest) product.

Either outcome is a publishable result and kills Gemini's unmeasured "bulletproof".

## Honesty guards
- Never report frametime gains as "fixed traversal stutter" — distinguish
  pop-in (async, expected) from hitching (CPU/PSO, untouched by this).
- Record that C/D are a **quality reduction** (Ultra→High equivalent), not free.
- n=1 → "signal". Widen before any claim leaves this repo.
