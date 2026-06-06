# shrinkray — PRODUCT.md

## Register

**Product.** shrinkray is a desktop tool whose UI serves the product, not vice
versa. The window is what users click in; the chrome, copy, and layout exist to
make optimization workflows tractable for a technical operator.

## Users

The primary user is an **engineer or technical artist** who knows their way
around an Unreal Engine cooked content tree, owns at least one game install
they want to shrink, and is comfortable reading hex sizes, BC pixel formats,
mip pyramids, and `TC_*` compression-settings classes. Secondary user is a
**games-research lecturer or peer reviewer** evaluating shrinkray as research
artifact — they will not click every button, but the UI must signal rigor,
falsifiable claims, and real measurements at a glance.

Context of use: a single-monitor laptop on a desk, indoor light, an hour-long
optimization or audit session that may include drilling into one specific
texture's mip table. Not a phone surface, not a mobile context, never
right-clicked.

## Product purpose

shrinkray reads cooked Unreal Engine pak files, projects byte-exact reversible
savings, audits structural bloat in game folders, and (separately) demonstrates
Δ-Codec — a novel texture compression scheme that ships both an AI prediction
and a byte-exact residual in one bitstream. The UI must make all three
capabilities visible without nesting them three levels deep.

## Brand personality

- **Distinctive, not trendy.** Win7 Aero chrome is intentional and on-brand —
  it sets shrinkray apart from every dashboard SaaS aesthetic in the room.
- **Measurement-forward.** Every claim on screen is backed by a number with a
  unit. No marketing copy.
- **Honest about scope.** Preview-only by default; "not yet implemented"
  labelled as such; bench results show losses as well as wins.
- **Confident but not flashy.** No animations beyond functional feedback. No
  gradients beyond the Aero chrome itself.

## Tone of UI copy

Technical, direct, dry-with-affection. Sentence case for headers. "would
shrink", "reclaimable", "byte-exact" — concrete verbs and adjectives. Never
"unleash your game's potential" or "revolutionary". When something fails it
says exactly what's missing ("backup required before write mode"); when
something works it shows numbers, not adjectives.

## Anti-references

- Modern dashboard SaaS (Linear / Notion / Vercel). Clean, sans-everything,
  white-on-white. Not shrinkray.
- Dark-mode developer tools (VS Code, Insomnia). Functional but interchangeable.
- Game launcher chrome (Steam, Epic Launcher). Heavy gradients, hover-glow,
  hero art. Wrong vibe entirely.
- Glassmorphism / neumorphism / 2020-era flat-with-shadows. shrinkray's
  glass is Win7 Aero glass; the difference matters.

## Strategic design principles

1. **Density is the affordance.** A serious operator will scan a screen
   listing 50 textures with their classes and savings columns. Don't hide
   information behind tabs that exist just to make the layout feel tidy.
2. **Win7 chrome on every panel, every modal.** Inconsistency breaks the
   character. Aero gradient title bars, beveled buttons, inset shadows,
   `Segoe UI` body.
3. **Numbers before adjectives.** "84.0 MB save · 98.4%" beats "Massive
   reclaim potential". Always.
4. **Live results over canned screenshots.** The Δ-Codec panel must run a
   real bench when the user clicks; the texture mip strip must scan the real
   pak. Demo trust comes from latency that proves work is happening.
5. **Preview-by-default for destructive ops.** The mode badge in the title
   row exists because the operator must know whether the next click writes to
   disk.
6. **Color is a status channel, not decoration.** Green = OK / saved. Orange
   = warn / projection. Red = danger / destructive. Blue = info / inspection.
   Never use color to add visual interest.

## What success looks like

A reviewer opens the app, picks a real game folder, sees real numbers within
30 seconds (analyze + mip strip projection), runs the Δ-Codec bench live, and
concludes the codec claim is falsifiable, measured, and reproducible — without
the operator having to narrate over generic UI.

## Tech context

- Tauri v2 + React + Vite frontend at `src/`
- `7.css` (Win7 chrome library, in `node_modules`) for native button + window
  semantics
- Aero wallpaper (ImageMagick-generated, vendored at `src/assets/wallpaper.jpg`)
  for the backdrop
- Custom title bar in `src/TitleBar.tsx` because Tauri's default chrome is
  wrong for the aesthetic
- Stylesheet at `src/styles.css`, layered on top of 7.css
