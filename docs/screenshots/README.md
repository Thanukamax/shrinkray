# Screenshots

Drop PNG screenshots in this folder using the filenames referenced by the
root `README.md`. They'll render automatically — no markdown edits needed.

Expected filenames (1280 × 800 recommended, PNG, keep each under ~500 KB):

| File | What to capture |
|---|---|
| `hero.png` | Main window with a folder loaded, analyze + audit panels visible, Win7 Aero chrome. The marquee shot. |
| `analyze.png` | The analyze report — folder census, top-50 fattest, language detection. |
| `audit.png` | The Bloat Audit panel with a few findings rendered + bloat score. |
| `inspector.png` | Asset Inspector with one cooked `.uasset` opened, mip table visible. |
| `mip-strip.png` | Mip Strip Panel showing the Pamali 570 MB / 77 % projection. |
| `dialog.png` | The in-app Win7-style Open dialog with quick-links sidebar. |

Tips for nicer shots:

- Use the Aero wallpaper background (already on by default).
- Resize the window to a 16:10 ratio before capturing — README cards look
  best at that aspect.
- Crop tightly to the window frame; the title bar is part of the brand.
- Optimize with `oxipng -o 4 --strip safe *.png` before committing
  (shrinkray itself can do this once it can recompress its own README).
