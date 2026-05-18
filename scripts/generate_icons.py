#!/usr/bin/env python3
"""Regenerate shrinkray's app icons at the sizes Tauri's bundler expects.

These are placeholders — replace with real branded artwork when ready.
Output: src-tauri/icons/{32x32.png, 128x128.png, 128x128@2x.png, icon.ico, icon.icns}
"""
from pathlib import Path
from PIL import Image, ImageDraw

ROOT = Path(__file__).resolve().parent.parent
OUT = ROOT / "src-tauri" / "icons"
OUT.mkdir(parents=True, exist_ok=True)


def make(size: int) -> Image.Image:
    img = Image.new("RGBA", (size, size), (12, 38, 50, 255))
    d = ImageDraw.Draw(img)
    margin = max(2, size // 6)
    top = (size // 2, margin)
    left = (margin, size - margin)
    right = (size - margin, size - margin)
    bottom = (size // 2, int(size * 0.65))
    d.polygon([top, right, bottom, left], fill=(240, 245, 255, 255))
    return img


for size, name in [(32, "32x32.png"), (128, "128x128.png"), (256, "128x128@2x.png")]:
    make(size).save(OUT / name, "PNG")

make(256).save(
    OUT / "icon.ico",
    "ICO",
    sizes=[(16, 16), (32, 32), (48, 48), (64, 64), (128, 128), (256, 256)],
)
make(512).save(OUT / "icon.icns", "ICNS")

for f in sorted(OUT.iterdir()):
    print(f"  {f.name:20} {f.stat().st_size:>7} bytes")
