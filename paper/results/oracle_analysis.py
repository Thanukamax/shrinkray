#!/usr/bin/env python3
"""
Oracle routing analysis — for each texture, pick min(bilinear, ESRGAN) ratio.
That's the upper bound any per-texture router could achieve.

Reads the two paired CSVs from 2026-05-23 and writes a markdown summary.
"""
import csv
import statistics as st
import sys
from pathlib import Path

HERE = Path(__file__).parent
# CLI: oracle_analysis.py [bilinear_csv] [esrgan_csv] [out_md]
# Defaults to the initial 19-sample paired run.
BILINEAR_CSV = Path(sys.argv[1]) if len(sys.argv) > 1 else HERE / "bench-bilinear-pbr-q1-pixel-512px-4x.csv"
ESRGAN_CSV = Path(sys.argv[2]) if len(sys.argv) > 2 else HERE / "bench-esrgan-pbr-q1-pixel-512px-4x.csv"
OUT_MD = Path(sys.argv[3]) if len(sys.argv) > 3 else HERE / "SUMMARY-oracle-routing-2026-05-23.md"


def load(p: Path):
    out = {}
    with p.open() as f:
        for row in csv.DictReader(f):
            out[row["label"]] = {
                "ratio": float(row["ratio_vs_baseline"]),
                "lossless": row["lossless_pass"] == "true",
                "hp_energy": float(row["hp_energy"]),
                "baseline_bytes": int(row["baseline_bytes"]),
                "residual_bytes": int(row["residual_bytes"]),
            }
    return out


def classify(label: str) -> str:
    n = label.lower()
    if "color" in n:
        return "color"
    if "normalgl" in n or "normal" in n:
        return "normal"
    if "roughness" in n:
        return "roughness"
    if "metalness" in n:
        return "metalness"
    return "other"


def main():
    bilinear = load(BILINEAR_CSV)
    esrgan = load(ESRGAN_CSV)
    common = sorted(set(bilinear) & set(esrgan))
    if not common:
        sys.exit("no overlapping textures")

    rows = []
    for label in common:
        b = bilinear[label]["ratio"]
        e = esrgan[label]["ratio"]
        oracle = min(b, e)
        winner = "bilinear" if b <= e else "esrgan"
        rows.append({
            "label": label,
            "class": classify(label),
            "bilinear": b,
            "esrgan": e,
            "oracle": oracle,
            "winner": winner,
            "hp_energy": bilinear[label]["hp_energy"],
        })

    # Per-class medians
    classes = sorted({r["class"] for r in rows})
    per_class = {}
    for c in classes:
        cls_rows = [r for r in rows if r["class"] == c]
        per_class[c] = {
            "n": len(cls_rows),
            "bilinear_median": st.median(r["bilinear"] for r in cls_rows),
            "esrgan_median": st.median(r["esrgan"] for r in cls_rows),
            "oracle_median": st.median(r["oracle"] for r in cls_rows),
            "esrgan_wins": sum(1 for r in cls_rows if r["winner"] == "esrgan"),
            "bilinear_wins": sum(1 for r in cls_rows if r["winner"] == "bilinear"),
        }

    overall = {
        "n": len(rows),
        "bilinear_median": st.median(r["bilinear"] for r in rows),
        "esrgan_median": st.median(r["esrgan"] for r in rows),
        "oracle_median": st.median(r["oracle"] for r in rows),
        "esrgan_wins": sum(1 for r in rows if r["winner"] == "esrgan"),
        "bilinear_wins": sum(1 for r in rows if r["winner"] == "bilinear"),
    }

    # Lift: how much does oracle beat the best single predictor?
    forced_bilinear_median = overall["bilinear_median"]
    forced_esrgan_median = overall["esrgan_median"]
    best_forced = min(forced_bilinear_median, forced_esrgan_median)
    oracle_lift_abs = best_forced - overall["oracle_median"]
    oracle_lift_pct = oracle_lift_abs / best_forced * 100

    # Compaction: can hp_energy predict the winner?
    bilinear_winners_hp = [r["hp_energy"] for r in rows if r["winner"] == "bilinear"]
    esrgan_winners_hp = [r["hp_energy"] for r in rows if r["winner"] == "esrgan"]

    # ----- write markdown -----
    out = OUT_MD
    with out.open("w") as f:
        f.write("# Oracle routing analysis — 2026-05-23\n\n")
        f.write(
            "For each PBR texture, pick `min(bilinear_ratio, esrgan_ratio)` as the\n"
            "*oracle* — the best achievable ratio if the router knew the right\n"
            "predictor in advance. This is the upper bound any data-driven router\n"
            "(probe-based, content-classifier, learned) can chase.\n\n"
        )

        f.write("## Aggregate\n\n")
        f.write("| Strategy | Median ratio | Bilinear-wins | ESRGAN-wins |\n")
        f.write("|----------|-------------:|---:|---:|\n")
        f.write(f"| Forced bilinear  | {overall['bilinear_median']:.4f} | — | — |\n")
        f.write(f"| Forced ESRGAN    | {overall['esrgan_median']:.4f} | — | — |\n")
        f.write(
            f"| **Oracle (best-of-both)** | "
            f"**{overall['oracle_median']:.4f}** | "
            f"{overall['bilinear_wins']}/{overall['n']} | "
            f"{overall['esrgan_wins']}/{overall['n']} |\n\n"
        )
        f.write(
            f"**Oracle lift vs best forced predictor (bilinear): "
            f"{oracle_lift_abs:.4f} absolute ({oracle_lift_pct:.1f}% relative).**\n\n"
        )

        f.write("## Per-class breakdown\n\n")
        f.write("| Class | n | Bilinear | ESRGAN | **Oracle** | ESRGAN wins |\n")
        f.write("|-------|--:|---------:|-------:|-----------:|------------:|\n")
        for c in ["color", "normal", "roughness", "metalness"]:
            if c not in per_class:
                continue
            d = per_class[c]
            f.write(
                f"| {c} | {d['n']} | {d['bilinear_median']:.4f} | "
                f"{d['esrgan_median']:.4f} | **{d['oracle_median']:.4f}** | "
                f"{d['esrgan_wins']}/{d['n']} |\n"
            )
        f.write("\n")

        f.write("## Per-texture pairs (oracle winner highlighted)\n\n")
        f.write("| Texture | Class | Bilinear | ESRGAN | Oracle | Winner | hp_energy |\n")
        f.write("|---------|-------|---------:|-------:|-------:|:-------|----------:|\n")
        for r in sorted(rows, key=lambda x: (x["class"], x["label"])):
            mark = "🟢 esrgan" if r["winner"] == "esrgan" else "bilinear"
            f.write(
                f"| {r['label']} | {r['class']} | {r['bilinear']:.4f} | "
                f"{r['esrgan']:.4f} | **{r['oracle']:.4f}** | {mark} | "
                f"{r['hp_energy']:.2f} |\n"
            )
        f.write("\n")

        f.write("## Can `hp_energy` (existing probe signal) predict the winner?\n\n")
        if bilinear_winners_hp:
            f.write(
                f"- Bilinear winners (n={len(bilinear_winners_hp)}): "
                f"hp_energy range [{min(bilinear_winners_hp):.2f}, "
                f"{max(bilinear_winners_hp):.2f}], "
                f"median {st.median(bilinear_winners_hp):.2f}\n"
            )
        if esrgan_winners_hp:
            f.write(
                f"- ESRGAN winners (n={len(esrgan_winners_hp)}): "
                f"hp_energy range [{min(esrgan_winners_hp):.2f}, "
                f"{max(esrgan_winners_hp):.2f}], "
                f"median {st.median(esrgan_winners_hp):.2f}\n\n"
            )

        # Threshold analysis: is there a clean hp split?
        if bilinear_winners_hp and esrgan_winners_hp:
            all_hp = sorted(set(bilinear_winners_hp + esrgan_winners_hp))
            best_t = None
            best_acc = -1.0
            for i in range(len(all_hp) - 1):
                t = (all_hp[i] + all_hp[i + 1]) / 2
                # Rule: hp >= t → ESRGAN, else bilinear
                correct = sum(1 for r in rows if (r["hp_energy"] >= t) == (r["winner"] == "esrgan"))
                acc = correct / len(rows)
                if acc > best_acc:
                    best_acc, best_t = acc, t
            f.write(
                f"**Best threshold on hp_energy alone:** t = {best_t:.2f} → "
                f"{best_acc*100:.1f}% routing accuracy. "
                f"(Random baseline: {max(overall['bilinear_wins'], overall['esrgan_wins']) / overall['n'] * 100:.1f}% "
                f"by always picking the majority winner.)\n\n"
            )

        f.write("## Interpretation\n\n")
        if oracle_lift_pct > 5:
            f.write(
                f"The oracle outperforms forced bilinear by {oracle_lift_pct:.1f}% "
                f"on the median PBR texture. **A real router worth building.**\n\n"
            )
        else:
            f.write(
                f"The oracle gains only {oracle_lift_pct:.1f}% over forced bilinear "
                f"on the median. **Not worth a routing layer at this corpus size** — "
                f"forced-bilinear is within shouting distance of optimal here, "
                f"and routing adds complexity for little win. The right experiment "
                f"is to broaden the corpus toward content classes where ESRGAN "
                f"wins (MetalPlates-style industrial diffuse, fine-detail normals) "
                f"and re-measure.\n\n"
            )
        f.write(
            "Either way, the **bitstream-level support for per-texture predictor "
            "choice remains the architectural contribution** — what changes with "
            "this data is whether a router *needs to be built today* or stays as "
            "future work.\n"
        )

    print(f"wrote {out}")


if __name__ == "__main__":
    main()
