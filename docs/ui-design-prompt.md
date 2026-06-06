# shrinkray — UI design prompt (paste into Claude)

A ready-to-use brief for redesigning/rebuilding the shrinkray UI with Claude
(Claude Code or claude.ai). Copy everything inside the rule below. Grounded in
`PRODUCT.md`, `src/styles.css` (the `--sr-*` tokens), and the real component +
IPC inventory as of v0.7.3.

---

You are redesigning the UI of **shrinkray**, a Tauri v2 desktop app that shrinks
Unreal Engine game folders. Make it look and feel *finished* — a precise,
measurement-forward instrument for a technical operator — without changing what
any button does. This is a **reskin + interaction-polish** pass, not a rewrite.

## Before you write any code
1. Read `PRODUCT.md`, `src/styles.css`, `src/App.tsx`, and every component in
   `src/*.tsx`. Build a screen inventory first; show it to me before editing.
2. Invoke the design skills and treat them as required tooling, not optional:
   - **ui-ux-pro-max** — palette, type pairing, spacing, layout, component states.
   - **impeccable** — run an audit pass on each screen; fix anti-patterns before
     declaring anything done.
   - **design-motion-principles** — invoke BEFORE writing any transition/animation.
3. Propose a short plan (token changes + per-component changes). Wait for my OK.

## Who it's for (do not drift from this)
A games **engineer / technical artist** reading hex sizes, BC pixel formats, mip
pyramids, and `TC_*` compression classes during an hour-long optimization or
audit session on a single-monitor laptop. Secondary user: a **research reviewer**
who must see rigor, falsifiable claims, and real measurements at a glance.
Never a phone surface. Never right-clicked.

## Brand & tone (from PRODUCT.md — enforce it)
- **Win7 Aero glass is intentional and on-brand.** Real `backdrop-filter` glass
  over the app's generated wallpaper — NOT trendy glassmorphism, NOT a SaaS
  dashboard, NOT dark-mode dev-tool chrome, NOT game-launcher gradients/hero art.
  Those are explicit anti-references; if a screen starts looking like Linear,
  Vercel, Steam, or VS Code, you've gone wrong.
- **Measurement-forward.** Every claim on screen is a number with a unit. No
  marketing copy. Never "unleash your game's potential."
- **Honest about scope.** Preview-only by default; "not yet implemented" stays
  labelled as such; bench tables show losses as well as wins.
- **Confident, not flashy.** No animation beyond functional feedback. No
  gradients beyond the Aero chrome itself.
- Copy: technical, direct, dry-with-affection. Sentence case headers. Concrete
  verbs/adjectives ("would shrink", "reclaimable", "byte-exact"). Failure states
  say exactly what's missing ("backup required before write mode").

## Design system (extend the existing tokens, don't replace them)
- Keep `src/styles.css` as the single stylesheet and keep the `--sr-*` custom
  properties as the source of truth. Current palette: warm orange brand
  (`--sr-orange #c96a18`), plus status families ok/warn/err/info each with
  `soft`/`border` variants. Light theme (white → `#eef3fa` gradients).
- Data/numbers in monospace (`Consolas, 'Liberation Mono', monospace`); chrome
  text in a clean sans. Keep the two registers distinct.
- **Density is a feature.** This is an instrument — tight, scannable rows, strong
  numeric alignment (right-align numbers, decimal-consistent), clear table
  hierarchy. Don't pad it out into a marketing page.
- Status color is semantic and consistent everywhere: green = ok/safe/byte-exact,
  amber = caution/structural, red = severe/error, blue = info. A reader should
  learn the legend once.

## Screens & components to cover (all of them)
- **TitleBar** — custom Win7 chrome + window controls; the app identity bar.
- **OpenDialog** — the in-app file/folder picker (no native dialog).
- **Main analyze panel** — pick/change folder, Analyze (primary), Audit; the
  analysis report (sizes, categories, reclaimable totals).
- **AuditCard** — read-only bloat audit: findings by severity + reclaimable by
  category; severity badges (info/warning/critical), score states.
- **AssetInspector** — drill into one cooked `.uasset`: export list, custom-version
  fingerprint, per-mip dimension/byte table, pagination, payload/package filters,
  path search. The most data-dense screen — get its table right.
- **MipStripPanel** — mip-strip projection + apply (preview-gated, backup-gated).
- **DeltaCodecPanel** — the Δ-Codec demo: predictor column, q-step rows, 2×/4×
  downsample toggle, ratio/max-err/byte-exact columns, oracle summary.
- **Backup / restore / recompress / strip** blocks — status rows, plan vs report,
  the backup-or-refuse + preview-mode affordances.
- **Row** (label/value) primitive — used everywhere; make it the spine of the
  measurement-forward look.
- **Global states** — empty (no folder picked), loading/progress (long ops:
  analyze, strip, restore, ESRGAN inference), error, and "not implemented yet".

## Interaction & motion
- Functional feedback only: button press, hover affordance, progress for
  long-running ops (strip/restore stream progress; ESRGAN runs take seconds —
  show a determinate or clearly-busy state, never a dead frozen window).
- No decorative motion, parallax, or entrance animations. Respect
  `prefers-reduced-motion`. Run **design-motion-principles** before adding any.

## Hard guardrails (do NOT)
- **Do not rename or change any Tauri `invoke()` command or its payload shape.**
  Preserve: `analyze_folder`, `audit_folder`, `backup_status`, `detect_encoders`,
  `plan_strip`, `apply_strip`, `plan_recompress`, `apply_recompress`,
  `ensure_backup`, `restore_folder`, `delta_codec_project_backup`,
  `delta_codec_run_synthetic_bench`, `delta_codec_run_file_bench`, and the asset
  inspector commands. The Rust backend is out of scope — TS/TSX/CSS only.
- Don't remove the preview-mode-default or backup-or-refuse affordances.
- Don't add heavy UI dependencies. Prefer the existing stack (React 19 + TS +
  Vite, hand-written CSS). No component megaframeworks, no CSS-in-JS runtime.
- Don't introduce dark mode or a SaaS aesthetic. Don't add marketing copy.
- Don't hide losses — bench tables and projections must still show when something
  inflates or when ESRGAN loses to bilinear.

## Deliverables
1. Screen inventory + redesign plan (await my approval).
2. Updated `src/styles.css` (extended tokens, component classes) + per-component
   TSX edits, smallest diffs that achieve the look.
3. An IMPECCABLE audit summary per screen (what was wrong, what you fixed).
4. `bun run tauri dev` runs; `npx tsc --noEmit` is clean; no console errors.

## Definition of done
Every screen looks finished on a single 1080p/1440p monitor; numbers are aligned
and unit-labelled; the Aero identity is unmistakable and distinct from any SaaS
dashboard; states (empty/loading/error/not-implemented) are all designed, not
afterthoughts; motion is functional-only; IPC contracts untouched; typecheck and
dev build green.
