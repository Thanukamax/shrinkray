# shrinkray — UI source bundle

All UI-relevant source for the redesign, concatenated for upload to claude.ai.
Pair with `docs/ui-design-prompt.md` (the brief) and `docs/architecture-diagrams.md`.
Generated from commit ad39a1a. Backend/IPC is out of scope — TS/TSX/CSS only.

## Contents
PRODUCT.md · index.html · src/main.tsx · src/App.tsx · src/TitleBar.tsx · src/OpenDialog.tsx · src/AssetInspector.tsx · src/MipStripPanel.tsx · src/DeltaCodecPanel.tsx · src/styles.css

---

## `PRODUCT.md`

````md
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

````

## `index.html`

````html
<!doctype html>
<html lang="en">
  <head>
    <meta charset="UTF-8" />
    <meta name="viewport" content="width=device-width, initial-scale=1.0" />
    <title>shrinkray</title>
  </head>
  <body>
    <div id="root"></div>
    <script type="module" src="/src/main.tsx"></script>
  </body>
</html>

````

## `src/main.tsx`

````tsx
import React from 'react'
import ReactDOM from 'react-dom/client'
import App from './App'
import '7.css/dist/7.css'
import './styles.css'

ReactDOM.createRoot(document.getElementById('root')!).render(
  <React.StrictMode>
    <App />
  </React.StrictMode>,
)

````

## `src/App.tsx`

````tsx
import { useEffect, useMemo, useState } from 'react'
import { invoke } from '@tauri-apps/api/core'
import { AssetInspector } from './AssetInspector'
import { MipStripPanel } from './MipStripPanel'
import { DeltaCodecPanel } from './DeltaCodecPanel'
import { TitleBar } from './TitleBar'
import { OpenDialog } from './OpenDialog'

const RECENT_FOLDERS_KEY = 'shrinkray.recent-folders'
function loadRecent(): string[] {
  try {
    const raw = localStorage.getItem(RECENT_FOLDERS_KEY)
    return raw ? JSON.parse(raw) : []
  } catch {
    return []
  }
}
function pushRecent(p: string) {
  try {
    const cur = loadRecent().filter((x) => x !== p)
    cur.unshift(p)
    localStorage.setItem(RECENT_FOLDERS_KEY, JSON.stringify(cur.slice(0, 8)))
  } catch {}
}

const PREVIEW_ONLY_KEY = 'shrinkray.preview-only'
const SEEN_KEY = 'shrinkray.seen'

function loadPreviewOnly(): boolean {
  try {
    const seen = localStorage.getItem(SEEN_KEY)
    if (seen !== 'true') {
      // First run: be safe — default to preview-only.
      return true
    }
    return localStorage.getItem(PREVIEW_ONLY_KEY) === 'true'
  } catch {
    return true
  }
}

function persistPreviewOnly(value: boolean) {
  try {
    localStorage.setItem(PREVIEW_ONLY_KEY, value ? 'true' : 'false')
    localStorage.setItem(SEEN_KEY, 'true')
  } catch {
    /* ignore — incognito or storage-disabled */
  }
}

type Category = { count: number; size: number }
type FatFile = { path: string; size: number; kind: string }
type UnreadablePak = { path: string; reason: string }
type PakInventory = {
  readable: string[]
  signed: string[]
  encrypted: string[]
  unreadable: UnreadablePak[]
}
type AnalysisReport = {
  root: string
  total_files: number
  total_size: number
  textures: Category
  audio: Category
  paks: Category
  languages: Record<string, Category>
  pak_inventory: PakInventory
  top_files: FatFile[]
  estimated_l10n_savings: number
}

type BackupStatus = {
  backup_dir: string
  created_at: number
  shrinkray_version: string
  mode: 'differential' | 'full'
  entry_count: number
}

type DeltaCodecClassBreakdown = {
  compression_settings: string
  texture_count: number
  baseline_bytes: number
  projected_bytes: number
  ratio: number
}

type DeltaCodecProjection = {
  current_backup_bytes: number
  projected_delta_codec_bytes: number
  savings_bytes: number
  ratio: number
  texture_count: number
  class_breakdown: DeltaCodecClassBreakdown[]
  bench_ratios_used: [string, number][]
}

type RestoreReport = {
  restored: string[]
  failures: { path: string; reason: string }[]
}

type PlannedFile = { path: string; size: number; language: string }
type PlannedPakChange = {
  pak: string
  dropped_entries: number
  kept_entries: number
  becomes_empty: boolean
}
type StripPlan = {
  root: string
  drop_languages: string[]
  loose_files: PlannedFile[]
  pak_changes: PlannedPakChange[]
  skipped_signed_paks: string[]
  skipped_encrypted_paks: string[]
  skipped_unreadable_paks: string[]
  total_loose_bytes: number
}

type PakRewrite = {
  pak: string
  original_size: number
  new_size: number
  dropped_entries: number
}
type StripFailure = { path: string; reason: string }
type StripReport = {
  deleted_files: string[]
  rewritten_paks: PakRewrite[]
  deleted_paks: string[]
  failures: StripFailure[]
  total_bytes_saved: number
}

type EncoderAvailability = {
  encoder: 'oxipng' | 'opusenc'
  available: boolean
  version: string | null
  install_hint: string
}
type RecompressKind = 'png' | 'wav' | 'flac'
type PlannedItem = {
  path: string
  kind: RecompressKind
  encoder: 'oxipng' | 'opusenc'
  size: number
}
type RecompressPlan = {
  root: string
  items: PlannedItem[]
  total_input_bytes: number
  missing_encoders: ('oxipng' | 'opusenc')[]
}
type RecompressResult = {
  path: string
  kind: RecompressKind
  original_size: number
  new_size: number
  bytes_saved: number
  new_path: string
}
type RecompressFailure = { path: string; kind: RecompressKind; reason: string }
type RecompressReport = {
  recompressed: RecompressResult[]
  skipped_no_improvement: string[]
  failures: RecompressFailure[]
  total_bytes_saved: number
}

type AuditSeverity = 'info' | 'warning' | 'critical'
type AuditCategory =
  | 'patch_overlay'
  | 'stale_version_dir'
  | 'sharded_videos'
  | 'large_chunk'
  | 'encryption'
  | 'editor_leftovers'
  | 'launcher_satellite'
  | 'chunking_quality'
  | 'shader_rhi_redundancy'
  | 'redist_installer'
  | 'platform_siblings'
  | 'duplicate_content'
  | 'mod_manager_artifacts'
  | 'cef_locales'
type AuditEvidence = { path: string; size_bytes: number; note?: string }
type AuditFinding = {
  detector: string
  category: AuditCategory
  severity: AuditSeverity
  title: string
  summary: string
  evidence: AuditEvidence[]
  reclaimable_bytes?: number
  recommendation: string
}
type AuditAggregate = {
  total_findings: number
  findings_by_severity: Partial<Record<AuditSeverity, number>>
  reclaimable_by_category: Partial<Record<AuditCategory, number>>
  total_reclaimable_bytes: number
  total_reclaimable_pct: number
  bloat_score: number
}
type AuditMeta = {
  schema_version: number
  tool_version: string
  generated_at: string
  detectors: string[]
}
type AuditReport = {
  root: string
  total_size_bytes: number
  findings: AuditFinding[]
  aggregate: AuditAggregate
  meta: AuditMeta
}

const CATEGORY_LABEL: Record<AuditCategory, string> = {
  patch_overlay: 'Patch overlay accumulation',
  stale_version_dir: 'Stale version directories',
  sharded_videos: 'Sharded video paks',
  large_chunk: 'Oversized pak chunks',
  encryption: 'Pak encryption status',
  editor_leftovers: 'Editor leftovers',
  launcher_satellite: 'Launcher language satellites',
  chunking_quality: 'Chunking strategy',
  shader_rhi_redundancy: 'Shader-cache RHI redundancy',
  redist_installer: 'Redistributable installers',
  platform_siblings: 'Multi-platform binaries',
  duplicate_content: 'Duplicate content',
  mod_manager_artifacts: 'Mod-manager leftovers',
  cef_locales: 'CEF locale bundles',
}

function scoreLabel(score: number): string {
  if (score < 20) return 'clean'
  if (score < 50) return 'mild'
  if (score < 80) return 'structural bloat'
  return 'severe'
}

export default function App() {
  const [path, setPath] = useState<string | null>(null)
  const [report, setReport] = useState<AnalysisReport | null>(null)
  const [backup, setBackup] = useState<BackupStatus | null>(null)
  const [restore, setRestore] = useState<RestoreReport | null>(null)
  const [deltaCodecProjection, setDeltaCodecProjection] =
    useState<DeltaCodecProjection | null>(null)
  const [dropLangs, setDropLangs] = useState<Set<string>>(new Set())
  const [plan, setPlan] = useState<StripPlan | null>(null)
  const [stripReport, setStripReport] = useState<StripReport | null>(null)
  const [pending, setPending] = useState(false)
  const [restoring, setRestoring] = useState(false)
  const [planning, setPlanning] = useState(false)
  const [applying, setApplying] = useState(false)
  const [encoders, setEncoders] = useState<EncoderAvailability[]>([])
  const [recompressPlan, setRecompressPlan] = useState<RecompressPlan | null>(null)
  const [recompressReport, setRecompressReport] = useState<RecompressReport | null>(null)
  const [planningRecompress, setPlanningRecompress] = useState(false)
  const [recompressing, setRecompressing] = useState(false)
  const [auditing, setAuditing] = useState(false)
  const [audit, setAudit] = useState<AuditReport | null>(null)
  const [previewOnly, setPreviewOnly] = useState<boolean>(() => loadPreviewOnly())
  const [error, setError] = useState<string | null>(null)

  useEffect(() => {
    persistPreviewOnly(previewOnly)
  }, [previewOnly])

  // Re-fetch the Δ-Codec projection whenever the backup state changes —
  // covers folder load, ensure_backup, post-strip, post-restore.
  useEffect(() => {
    let cancelled = false
    async function pull() {
      if (!path || !backup || backup.entry_count === 0) {
        setDeltaCodecProjection(null)
        return
      }
      try {
        const proj = await invoke<DeltaCodecProjection | null>(
          'delta_codec_project_backup',
          { path },
        )
        if (!cancelled) setDeltaCodecProjection(proj)
      } catch {
        if (!cancelled) setDeltaCodecProjection(null)
      }
    }
    pull()
    return () => {
      cancelled = true
    }
  }, [path, backup])

  const [folderDialogOpen, setFolderDialogOpen] = useState(false)

  function pickFolder() {
    setFolderDialogOpen(true)
  }

  async function onFolderChosen(sel: string) {
    setFolderDialogOpen(false)
    setError(null)
    setRestore(null)
    setPlan(null)
    setStripReport(null)
    setRecompressPlan(null)
    setRecompressReport(null)
    setAudit(null)
    setDropLangs(new Set())
    setPath(sel)
    setReport(null)
    pushRecent(sel)
    const [st, encs] = await Promise.all([
      invoke<BackupStatus | null>('backup_status', { path: sel }),
      invoke<EncoderAvailability[]>('detect_encoders'),
    ])
    setBackup(st)
    setEncoders(encs)
  }

  async function runAudit() {
    if (!path) return
    setAuditing(true)
    setError(null)
    try {
      const r = await invoke<AuditReport>('audit_folder', { path })
      setAudit(r)
    } catch (e) {
      setError(String(e))
    } finally {
      setAuditing(false)
    }
  }

  async function analyze() {
    if (!path) return
    setPending(true)
    setError(null)
    try {
      const r = await invoke<AnalysisReport>('analyze_folder', { path })
      setReport(r)
    } catch (e) {
      setError(String(e))
    } finally {
      setPending(false)
    }
    // Backup status is a follow-up probe — its latency must never block
    // the analyze button's pending state.
    try {
      const st = await invoke<BackupStatus | null>('backup_status', { path })
      setBackup(st)
    } catch {
      /* ignore — backup status is informational */
    }
  }

  async function restoreFolder() {
    if (!path || !backup) return
    setRestoring(true)
    setError(null)
    setRestore(null)
    try {
      const result = await invoke<RestoreReport>('restore_folder', { path })
      setRestore(result)
      // After restore, re-probe everything.
      const st = await invoke<BackupStatus | null>('backup_status', { path })
      setBackup(st)
    } catch (e) {
      setError(String(e))
    } finally {
      setRestoring(false)
    }
  }

  function toggleDrop(lang: string) {
    setDropLangs((cur) => {
      const next = new Set(cur)
      if (next.has(lang)) next.delete(lang)
      else next.add(lang)
      return next
    })
    setPlan(null)
    setStripReport(null)
  }

  async function runPlan() {
    if (!path || dropLangs.size === 0) return
    setPlanning(true)
    setError(null)
    setStripReport(null)
    try {
      const p = await invoke<StripPlan>('plan_strip', {
        path,
        dropLanguages: Array.from(dropLangs),
      })
      setPlan(p)
    } catch (e) {
      setError(String(e))
    } finally {
      setPlanning(false)
    }
  }

  async function runRecompressPlan() {
    if (!path) return
    setPlanningRecompress(true)
    setError(null)
    setRecompressReport(null)
    try {
      const p = await invoke<RecompressPlan>('plan_recompress', { path })
      setRecompressPlan(p)
    } catch (e) {
      setError(String(e))
    } finally {
      setPlanningRecompress(false)
    }
  }

  async function applyRecompress() {
    if (!path || !recompressPlan || recompressPlan.items.length === 0 || previewOnly) return
    const summary =
      `About to recompress ${recompressPlan.items.length} loose files in ${path}\n` +
      `  ${formatBytes(recompressPlan.total_input_bytes)} input bytes\n` +
      `  WAV/FLAC → .opus  (existing files deleted, savings ~85%)\n` +
      `  PNG → re-optimised in place (lossless, savings ~10-30%)\n\n` +
      (backup
        ? `Existing backup will record every change.`
        : `A differential backup will be created first.`) +
      `\n\nContinue?`
    if (!window.confirm(summary)) return

    setRecompressing(true)
    setError(null)
    try {
      if (!backup) {
        const fresh = await invoke<BackupStatus>('ensure_backup', { path })
        setBackup(fresh)
      }
      const r = await invoke<RecompressReport>('apply_recompress', { path })
      setRecompressReport(r)
      const [fresh, st] = await Promise.all([
        invoke<AnalysisReport>('analyze_folder', { path }),
        invoke<BackupStatus | null>('backup_status', { path }),
      ])
      setReport(fresh)
      setBackup(st)
      setRecompressPlan(null)
    } catch (e) {
      setError(String(e))
    } finally {
      setRecompressing(false)
    }
  }

  async function applyStrip() {
    if (!path || dropLangs.size === 0 || !plan || previewOnly) return
    const summary =
      `About to drop ${dropLangs.size} language(s): ${Array.from(dropLangs).join(', ')}\n` +
      `  ${plan.loose_files.length} loose files\n` +
      `  ${plan.pak_changes.length} pak${plan.pak_changes.length === 1 ? '' : 's'} to rewrite/delete\n` +
      `  ~${formatBytes(plan.total_loose_bytes)} loose savings (+ pak savings vary)\n\n` +
      (backup
        ? `Existing backup will record every change.`
        : `A differential backup will be created at ${path}/../shrinkray_backup/ first.`) +
      `\n\nContinue?`
    if (!window.confirm(summary)) return

    setApplying(true)
    setError(null)
    setStripReport(null)
    try {
      if (!backup) {
        const fresh = await invoke<BackupStatus>('ensure_backup', { path })
        setBackup(fresh)
      }
      const r = await invoke<StripReport>('apply_strip', {
        path,
        dropLanguages: Array.from(dropLangs),
      })
      setStripReport(r)
      // Re-probe analysis + backup so the UI reflects the new state.
      const fresh = await invoke<AnalysisReport>('analyze_folder', { path })
      setReport(fresh)
      const st = await invoke<BackupStatus | null>('backup_status', { path })
      setBackup(st)
      setPlan(null)
    } catch (e) {
      setError(String(e))
    } finally {
      setApplying(false)
    }
  }

  const savedPct =
    report && report.total_size > 0
      ? Math.round((report.estimated_l10n_savings / report.total_size) * 100)
      : 0

  const languages = useMemo(
    () =>
      report
        ? Object.entries(report.languages).sort((a, b) => b[1].size - a[1].size)
        : [],
    [report],
  )
  const largestLang = languages[0]?.[0]
  const inv = report?.pak_inventory

  return (
    <div className="window app-shell active glass">
      <TitleBar title="shrinkray" subtitle="UE game folder optimizer · v0.7.3" />
      {folderDialogOpen && (
        <OpenDialog
          mode="folder"
          initialPath={path}
          recent={loadRecent()}
          onConfirm={onFolderChosen}
          onCancel={() => setFolderDialogOpen(false)}
        />
      )}
      <main className="window-body layout">
        <div className="wizard-intro">
          <p className="wizard-intro-blurb">
            Trim, audit, and inspect your Unreal Engine game folders. Preview-only by default.
          </p>
          <span className={`mode-badge ${previewOnly ? 'mode-preview' : 'mode-write'}`}>
            {previewOnly ? 'preview mode' : 'WRITE mode'}
          </span>
        </div>

      <section className="drop">
        <label className="preview-toggle">
          <input
            type="checkbox"
            checked={previewOnly}
            onChange={(e) => setPreviewOnly(e.target.checked)}
          />
          <span>
            preview-only mode
            <span className="muted small">
              {' '}
              — when checked, apply buttons are disabled and no writes happen
            </span>
          </span>
        </label>
        {path ? (
          <p className="path" title={path}>
            {path}
          </p>
        ) : (
          <p className="placeholder">Pick a game folder to analyze.</p>
        )}
        <div className="actions">
          <button onClick={pickFolder}>{path ? 'change folder' : 'pick folder'}</button>
          {path && (
            <button className="primary" onClick={analyze} disabled={pending}>
              {pending ? 'analyzing…' : 'analyze'}
            </button>
          )}
          {path && (
            <button onClick={runAudit} disabled={auditing}>
              {auditing ? 'auditing…' : 'bloat audit'}
            </button>
          )}
        </div>
        {error && <p className="err">{error}</p>}
      </section>

      {audit && <AuditCard report={audit} />}

      <AssetInspector />

      <MipStripPanel folderPath={path} backupLoaded={!!backup} previewOnly={previewOnly} />

      <DeltaCodecPanel />

      {backup && (
        <section className="report backup-card">
          <h2>Backup</h2>
          <table>
            <tbody>
              <Row label="created" value={formatTimestamp(backup.created_at)} />
              <Row label="mode" value={backup.mode} />
              <Row label="recorded edits" value={backup.entry_count.toLocaleString()} />
              <Row label="written by" value={`shrinkray ${backup.shrinkray_version}`} />
              <Row label="backup dir" value={backup.backup_dir} />
            </tbody>
          </table>

          {deltaCodecProjection && deltaCodecProjection.texture_count > 0 && (
            <div
              style={{
                marginTop: '0.9rem',
                padding: '0.7rem 0.9rem',
                background: 'rgba(64, 200, 140, 0.08)',
                border: '1px solid rgba(120, 220, 160, 0.3)',
                borderRadius: '4px',
              }}
            >
              <p style={{ margin: 0, fontWeight: 600, color: '#9efc8c' }}>
                Δ-Codec projection
              </p>
              <p className="muted small" style={{ marginTop: '0.3rem' }}>
                Same backup, encoded via Δ-Codec sidecar instead of full bytes. Bench-validated
                per-class ratios (see <code>docs/delta-codec-spec.md</code>).
              </p>
              <table style={{ marginTop: '0.5rem' }}>
                <tbody>
                  <Row
                    label="current backup"
                    value={formatBytes(deltaCodecProjection.current_backup_bytes)}
                  />
                  <Row
                    label="with Δ-Codec"
                    value={formatBytes(deltaCodecProjection.projected_delta_codec_bytes)}
                  />
                  <Row
                    label="savings"
                    value={`${formatBytes(deltaCodecProjection.savings_bytes)} (${((1 - deltaCodecProjection.ratio) * 100).toFixed(1)}% smaller)`}
                  />
                  <Row
                    label="textures covered"
                    value={deltaCodecProjection.texture_count.toLocaleString()}
                  />
                </tbody>
              </table>
              {deltaCodecProjection.class_breakdown.length > 0 && (
                <details style={{ marginTop: '0.5rem' }}>
                  <summary className="muted small">per-class breakdown</summary>
                  <table style={{ marginTop: '0.4rem' }}>
                    <thead>
                      <tr>
                        <th style={{ textAlign: 'left' }}>compression</th>
                        <th style={{ textAlign: 'right' }}>count</th>
                        <th style={{ textAlign: 'right' }}>baseline</th>
                        <th style={{ textAlign: 'right' }}>Δ-projected</th>
                        <th style={{ textAlign: 'right' }}>ratio</th>
                      </tr>
                    </thead>
                    <tbody>
                      {deltaCodecProjection.class_breakdown.map((c) => (
                        <tr key={c.compression_settings}>
                          <td>{c.compression_settings}</td>
                          <td style={{ textAlign: 'right' }}>{c.texture_count}</td>
                          <td style={{ textAlign: 'right' }}>{formatBytes(c.baseline_bytes)}</td>
                          <td style={{ textAlign: 'right' }}>{formatBytes(c.projected_bytes)}</td>
                          <td
                            style={{
                              textAlign: 'right',
                              color: c.ratio < 1.0 ? '#9efc8c' : '#ffb14e',
                            }}
                          >
                            {c.ratio.toFixed(2)}×
                          </td>
                        </tr>
                      ))}
                    </tbody>
                  </table>
                </details>
              )}
            </div>
          )}
          <div className="actions" style={{ marginTop: '0.9rem' }}>
            <button onClick={restoreFolder} disabled={restoring || backup.entry_count === 0}>
              {restoring ? 'restoring…' : 'restore from backup'}
            </button>
          </div>
          {restore && (
            <div style={{ marginTop: '0.9rem' }}>
              <p className="muted small">
                Restored {restore.restored.length.toLocaleString()} of{' '}
                {(restore.restored.length + restore.failures.length).toLocaleString()} entries.
              </p>
              {restore.failures.length > 0 && (
                <ul className="reasons">
                  {restore.failures.slice(0, 10).map((f) => (
                    <li key={f.path} className="err">
                      <span className="path-small">{f.path}</span>
                      <span className="muted small"> — {f.reason}</span>
                    </li>
                  ))}
                </ul>
              )}
            </div>
          )}
        </section>
      )}

      {report && (
        <section className="report">
          <h2>Census</h2>
          <table>
            <tbody>
              <Row label="total files" value={report.total_files.toLocaleString()} />
              <Row label="total size" value={formatBytes(report.total_size)} />
              <Row
                label="textures"
                value={`${report.textures.count.toLocaleString()} · ${formatBytes(report.textures.size)}`}
              />
              <Row
                label="audio"
                value={`${report.audio.count.toLocaleString()} · ${formatBytes(report.audio.size)}`}
              />
              <Row
                label="paks"
                value={`${report.paks.count.toLocaleString()} · ${formatBytes(report.paks.size)}`}
              />
              {languages.length > 1 && (
                <Row
                  label="l10n savings ceiling"
                  value={`${formatBytes(report.estimated_l10n_savings)}  (~${savedPct}% of folder)`}
                  accent
                />
              )}
            </tbody>
          </table>

          <h2 style={{ marginTop: '1.6rem' }}>
            {languages.length > 0
              ? `Languages — pick which to drop (${languages.length} detected)`
              : 'L10N strip'}
          </h2>
          {languages.length === 0 ? (
            <p className="muted">
              No L10N folders detected on this install — nothing to strip.
              shrinkray looks for BCP-47-style language codes (e.g.{' '}
              <code>Lang_en</code>, <code>L10N/de</code>, <code>fr-FR</code>) in
              loose paths and inside paks. Single-language games and tightly
              cooked indies will land here.
            </p>
          ) : (
            <>
              <table>
                <tbody>
                  {languages.map(([code, cat]) => {
                    const dropping = dropLangs.has(code)
                    return (
                      <tr key={code} className={dropping ? 'dropping' : ''}>
                        <td>
                          <label className="lang-row">
                            <input
                              type="checkbox"
                              checked={dropping}
                              onChange={() => toggleDrop(code)}
                              disabled={applying}
                            />
                            <span className={code === largestLang ? 'accent' : ''}>
                              drop {code}
                              {code === largestLang ? '  (largest — keep?)' : ''}
                            </span>
                          </label>
                        </td>
                        <td>
                          {cat.count.toLocaleString()} files · {formatBytes(cat.size)}
                        </td>
                      </tr>
                    )
                  })}
                </tbody>
              </table>
              <div className="actions" style={{ marginTop: '0.9rem' }}>
                <button
                  onClick={runPlan}
                  disabled={planning || applying || dropLangs.size === 0}
                >
                  {planning ? 'planning…' : 'preview'}
                </button>
                <button
                  className="primary destructive"
                  onClick={applyStrip}
                  disabled={
                    applying || dropLangs.size === 0 || !plan || previewOnly
                  }
                  title={
                    previewOnly
                      ? 'preview-only mode is on — uncheck the toggle at the top to enable writes'
                      : !plan
                        ? 'preview first'
                        : ''
                  }
                >
                  {applying
                    ? 'applying…'
                    : previewOnly
                      ? 'apply (preview-only mode)'
                      : backup
                        ? 'apply (write to backup + folder)'
                        : 'apply (create backup + write)'}
                </button>
              </div>
            </>
          )}

          {plan && (
            <section className="plan-card">
              <h2 style={{ marginTop: '1.6rem' }}>
                Plan — drop {plan.drop_languages.join(', ')}
              </h2>
              <table>
                <tbody>
                  <Row
                    label="loose files to delete"
                    value={`${plan.loose_files.length.toLocaleString()} · ${formatBytes(plan.total_loose_bytes)}`}
                    accent
                  />
                  <Row
                    label="paks to rewrite or delete"
                    value={plan.pak_changes.length.toLocaleString()}
                    accent={plan.pak_changes.length > 0}
                  />
                  {plan.skipped_signed_paks.length > 0 && (
                    <Row
                      label="signed paks skipped"
                      value={plan.skipped_signed_paks.length.toLocaleString()}
                    />
                  )}
                  {plan.skipped_encrypted_paks.length > 0 && (
                    <Row
                      label="encrypted paks skipped"
                      value={plan.skipped_encrypted_paks.length.toLocaleString()}
                    />
                  )}
                </tbody>
              </table>
            </section>
          )}

          {stripReport && (
            <section className="plan-card">
              <h2 style={{ marginTop: '1.6rem' }}>Strip applied</h2>
              <table>
                <tbody>
                  <Row
                    label="loose files deleted"
                    value={stripReport.deleted_files.length.toLocaleString()}
                    accent
                  />
                  <Row
                    label="paks rewritten"
                    value={stripReport.rewritten_paks.length.toLocaleString()}
                    accent
                  />
                  <Row
                    label="paks deleted (became empty)"
                    value={stripReport.deleted_paks.length.toLocaleString()}
                    accent={stripReport.deleted_paks.length > 0}
                  />
                  <Row
                    label="bytes saved"
                    value={formatBytes(stripReport.total_bytes_saved)}
                    accent
                  />
                  {stripReport.failures.length > 0 && (
                    <Row
                      label="failures"
                      value={stripReport.failures.length.toLocaleString()}
                    />
                  )}
                </tbody>
              </table>
              {stripReport.failures.length > 0 && (
                <ul className="reasons">
                  {stripReport.failures.slice(0, 10).map((f) => (
                    <li key={f.path} className="err">
                      <span className="path-small">{f.path}</span>
                      <span className="muted small"> — {f.reason}</span>
                    </li>
                  ))}
                </ul>
              )}
            </section>
          )}

          <h2 style={{ marginTop: '1.6rem' }}>Loose-file recompression</h2>
          <table>
            <tbody>
              {encoders.map((e) => (
                <Row
                  key={e.encoder}
                  label={e.encoder}
                  value={
                    e.available
                      ? `available · ${e.version ?? 'unknown version'}`
                      : `missing · ${e.install_hint}`
                  }
                  accent={!e.available}
                />
              ))}
            </tbody>
          </table>
          <div className="actions" style={{ marginTop: '0.9rem' }}>
            <button
              onClick={runRecompressPlan}
              disabled={planningRecompress || recompressing}
            >
              {planningRecompress ? 'scanning…' : 'find loose files'}
            </button>
            <button
              className="primary destructive"
              onClick={applyRecompress}
              disabled={
                recompressing ||
                !recompressPlan ||
                recompressPlan.items.length === 0 ||
                previewOnly
              }
              title={
                previewOnly
                  ? 'preview-only mode is on — uncheck the toggle at the top to enable writes'
                  : !recompressPlan
                    ? 'scan first'
                    : ''
              }
            >
              {recompressing
                ? 'recompressing…'
                : previewOnly
                  ? 'recompress (preview-only mode)'
                  : backup
                    ? 'recompress (write to backup + folder)'
                    : 'recompress (create backup + write)'}
            </button>
          </div>

          {recompressPlan && (
            <section className="plan-card">
              <h2 style={{ marginTop: '1.6rem' }}>Recompress plan</h2>
              <table>
                <tbody>
                  <Row
                    label="files to process"
                    value={recompressPlan.items.length.toLocaleString()}
                    accent
                  />
                  <Row
                    label="total input bytes"
                    value={formatBytes(recompressPlan.total_input_bytes)}
                  />
                  <Row
                    label="png"
                    value={recompressPlan.items
                      .filter((i) => i.kind === 'png')
                      .length.toLocaleString()}
                  />
                  <Row
                    label="wav"
                    value={recompressPlan.items
                      .filter((i) => i.kind === 'wav')
                      .length.toLocaleString()}
                  />
                  <Row
                    label="flac"
                    value={recompressPlan.items
                      .filter((i) => i.kind === 'flac')
                      .length.toLocaleString()}
                  />
                  {recompressPlan.missing_encoders.length > 0 && (
                    <Row
                      label="missing encoders"
                      value={recompressPlan.missing_encoders.join(', ')}
                      accent
                    />
                  )}
                </tbody>
              </table>
            </section>
          )}

          {recompressReport && (
            <section className="plan-card">
              <h2 style={{ marginTop: '1.6rem' }}>Recompress applied</h2>
              <table>
                <tbody>
                  <Row
                    label="files recompressed"
                    value={recompressReport.recompressed.length.toLocaleString()}
                    accent
                  />
                  <Row
                    label="skipped (no improvement)"
                    value={recompressReport.skipped_no_improvement.length.toLocaleString()}
                  />
                  <Row
                    label="bytes saved"
                    value={formatBytes(recompressReport.total_bytes_saved)}
                    accent
                  />
                  {recompressReport.failures.length > 0 && (
                    <Row
                      label="failures"
                      value={recompressReport.failures.length.toLocaleString()}
                    />
                  )}
                </tbody>
              </table>
              {recompressReport.failures.length > 0 && (
                <details style={{ marginTop: '0.6rem' }}>
                  <summary className="muted small">
                    {recompressReport.failures.length} failed — click to expand
                  </summary>
                  <ul className="reasons">
                    {recompressReport.failures.slice(0, 20).map((f) => (
                      <li key={f.path} className="err">
                        <span className="path-small">{f.path}</span>
                        <span className="muted small"> — {f.reason}</span>
                      </li>
                    ))}
                  </ul>
                </details>
              )}
            </section>
          )}

          {inv && report.paks.count > 0 && (
            <>
              <h2 style={{ marginTop: '1.6rem' }}>Pak inventory</h2>
              <table>
                <tbody>
                  <Row label="readable" value={inv.readable.length.toLocaleString()} />
                  <Row
                    label="signed (untouchable)"
                    value={inv.signed.length.toLocaleString()}
                    accent={inv.signed.length > 0}
                  />
                  <Row
                    label="encrypted (needs key)"
                    value={inv.encrypted.length.toLocaleString()}
                    accent={inv.encrypted.length > 0}
                  />
                  <Row
                    label="unreadable"
                    value={inv.unreadable.length.toLocaleString()}
                    accent={inv.unreadable.length > 0}
                  />
                </tbody>
              </table>
              {inv.unreadable.length > 0 && (
                <details style={{ marginTop: '0.6rem' }}>
                  <summary className="muted small">
                    {inv.unreadable.length} unreadable — click to expand
                  </summary>
                  <ul className="reasons">
                    {inv.unreadable.slice(0, 20).map((u) => (
                      <li key={u.path}>
                        <span className="path-small">{u.path}</span>
                        <span className="muted small"> — {u.reason}</span>
                      </li>
                    ))}
                    {inv.unreadable.length > 20 && (
                      <li className="muted small">…and {inv.unreadable.length - 20} more</li>
                    )}
                  </ul>
                </details>
              )}
            </>
          )}

          {report.top_files.length > 0 && (
            <>
              <h2 style={{ marginTop: '1.6rem' }}>
                Top {report.top_files.length} fattest files
              </h2>
              <table className="top">
                <tbody>
                  {report.top_files.map((f) => (
                    <tr key={f.path}>
                      <td className="kind">{f.kind}</td>
                      <td className="path-small" title={f.path}>{f.path}</td>
                      <td className="size">{formatBytes(f.size)}</td>
                    </tr>
                  ))}
                </tbody>
              </table>
            </>
          )}
        </section>
      )}
      </main>
    </div>
  )
}

function Row({
  label,
  value,
  accent,
}: {
  label: string
  value: string
  accent?: boolean
}) {
  return (
    <tr>
      <td>{label}</td>
      <td className={accent ? 'accent' : ''}>{value}</td>
    </tr>
  )
}

function AuditCard({ report }: { report: AuditReport }) {
  const ag = report.aggregate
  const grouped: Record<AuditSeverity, AuditFinding[]> = {
    critical: [],
    warning: [],
    info: [],
  }
  for (const f of report.findings) grouped[f.severity].push(f)

  const reclaimableRows = Object.entries(ag.reclaimable_by_category)
    .filter(([, v]) => (v ?? 0) > 0)
    .sort((a, b) => (b[1] ?? 0) - (a[1] ?? 0))

  return (
    <section className="report audit-card">
      <div className="audit-header">
        <h2>Bloat Audit</h2>
        <span className="muted small">
          {report.meta.detectors.length} detectors · {report.findings.length} findings
        </span>
      </div>

      <div className="score-row">
        <div className={`score-pill score-${scoreBucket(ag.bloat_score)}`}>
          <span className="score-value">{ag.bloat_score}</span>
          <span className="score-out">/100</span>
          <span className="score-label">{scoreLabel(ag.bloat_score)}</span>
        </div>
        <div className="score-side">
          <div>
            <span className="muted small">total</span>
            <strong>{formatBytes(report.total_size_bytes)}</strong>
          </div>
          {ag.total_reclaimable_bytes > 0 && (
            <div>
              <span className="muted small">reclaimable</span>
              <strong className="accent">
                {formatBytes(ag.total_reclaimable_bytes)} ({ag.total_reclaimable_pct.toFixed(1)}%)
              </strong>
            </div>
          )}
        </div>
      </div>

      {reclaimableRows.length > 0 && (
        <>
          <h3 className="audit-h3">Reclaimable by category</h3>
          <table>
            <tbody>
              {reclaimableRows.map(([cat, bytes]) => (
                <tr key={cat}>
                  <td>{CATEGORY_LABEL[cat as AuditCategory] ?? cat}</td>
                  <td className="accent">{formatBytes(bytes ?? 0)}</td>
                </tr>
              ))}
            </tbody>
          </table>
        </>
      )}

      {report.findings.length === 0 && (
        <p className="muted" style={{ marginTop: '0.9rem' }}>
          No findings — this install looks clean to shrinkray.
        </p>
      )}

      {(['critical', 'warning', 'info'] as AuditSeverity[]).map((sev) =>
        grouped[sev].length === 0 ? null : (
          <div key={sev} className="finding-group">
            <h3 className="audit-h3">
              {sev[0].toUpperCase() + sev.slice(1)} findings ({grouped[sev].length})
            </h3>
            {grouped[sev].map((f, i) => (
              <FindingCard key={`${sev}-${i}`} f={f} />
            ))}
          </div>
        ),
      )}
    </section>
  )
}

function FindingCard({ f }: { f: AuditFinding }) {
  return (
    <article className={`finding finding-${f.severity}`}>
      <header className="finding-head">
        <span className={`sev-pill sev-${f.severity}`}>{f.severity}</span>
        <span className="finding-title">{f.title}</span>
      </header>
      <p className="finding-summary">{f.summary}</p>
      {f.reclaimable_bytes != null && f.reclaimable_bytes > 0 && (
        <p className="finding-meta">
          <span className="muted small">reclaimable:</span>{' '}
          <strong className="accent">{formatBytes(f.reclaimable_bytes)}</strong>
        </p>
      )}
      {f.evidence.length > 0 && (
        <details>
          <summary className="muted small">
            {f.evidence.length} evidence item{f.evidence.length === 1 ? '' : 's'}
          </summary>
          <ul className="reasons">
            {f.evidence
              .slice()
              .sort((a, b) => b.size_bytes - a.size_bytes)
              .slice(0, 8)
              .map((ev) => (
                <li key={ev.path}>
                  <span className="path-small">{ev.path}</span>
                  <span className="muted small"> — {formatBytes(ev.size_bytes)}</span>
                  {ev.note && <span className="muted small"> · {ev.note}</span>}
                </li>
              ))}
            {f.evidence.length > 8 && (
              <li className="muted small">…and {f.evidence.length - 8} more</li>
            )}
          </ul>
        </details>
      )}
      <p className="finding-rec">
        <span className="muted small">recommendation:</span> {f.recommendation}
      </p>
    </article>
  )
}

function scoreBucket(score: number): 'clean' | 'mild' | 'structural' | 'severe' {
  if (score < 20) return 'clean'
  if (score < 50) return 'mild'
  if (score < 80) return 'structural'
  return 'severe'
}

function formatTimestamp(unixSeconds: number): string {
  if (!unixSeconds) return 'unknown'
  return new Date(unixSeconds * 1000).toLocaleString()
}

function formatBytes(b: number): string {
  if (!b) return '0 B'
  const u = ['B', 'KB', 'MB', 'GB', 'TB']
  let i = 0
  let n = b
  while (n >= 1024 && i < u.length - 1) {
    n /= 1024
    i++
  }
  return `${n.toFixed(n < 10 ? 2 : 1)} ${u[i]}`
}

````

## `src/TitleBar.tsx`

````tsx
import { getCurrentWindow } from '@tauri-apps/api/window'

const win = getCurrentWindow()

export function TitleBar({ title, subtitle }: { title: string; subtitle?: string }) {
  return (
    <div className="title-bar" data-tauri-drag-region>
      <div className="title-bar-text" data-tauri-drag-region>
        {title}
        {subtitle && (
          <span className="title-bar-subtitle" data-tauri-drag-region>
            {' '}
            {subtitle}
          </span>
        )}
      </div>
      <div className="title-bar-controls">
        <button aria-label="Minimize" onClick={() => win.minimize()} />
        <button aria-label="Maximize" onClick={() => win.toggleMaximize()} />
        <button aria-label="Close" onClick={() => win.close()} />
      </div>
    </div>
  )
}

````

## `src/OpenDialog.tsx`

````tsx
import { useCallback, useEffect, useMemo, useRef, useState } from 'react'
import { invoke } from '@tauri-apps/api/core'

export type OpenDialogMode = 'folder' | 'pak'

type DirEntry = {
  name: string
  path: string
  is_dir: boolean
  size: number
  modified: number
  extension: string
}
type QuickLink = { label: string; path: string; kind: string }

type Props = {
  mode: OpenDialogMode
  initialPath?: string | null
  recent?: string[]
  onConfirm: (path: string) => void
  onCancel: () => void
}

function formatBytes(b: number): string {
  if (!b) return ''
  const u = ['B', 'KB', 'MB', 'GB', 'TB']
  let i = 0
  let n = b
  while (n >= 1024 && i < u.length - 1) {
    n /= 1024
    i++
  }
  return `${n.toFixed(n < 10 && i > 0 ? 2 : 0)} ${u[i]}`
}

function formatModified(unix: number): string {
  if (!unix) return ''
  return new Date(unix * 1000).toLocaleString()
}

function breadcrumbs(path: string): { label: string; path: string }[] {
  if (!path) return []
  const parts = path.split('/').filter(Boolean)
  const acc: { label: string; path: string }[] = [{ label: '/', path: '/' }]
  let cur = ''
  for (const p of parts) {
    cur += '/' + p
    acc.push({ label: p, path: cur })
  }
  return acc
}

export function OpenDialog({ mode, initialPath, recent, onConfirm, onCancel }: Props) {
  const [cwd, setCwd] = useState<string>(initialPath || '/')
  const [entries, setEntries] = useState<DirEntry[]>([])
  const [quickLinks, setQuickLinks] = useState<QuickLink[]>([])
  const [selected, setSelected] = useState<DirEntry | null>(null)
  const [error, setError] = useState<string | null>(null)
  const [loading, setLoading] = useState(false)
  const [search, setSearch] = useState('')
  const listRef = useRef<HTMLDivElement | null>(null)

  const wantsFolder = mode === 'folder'
  const filterLabel = wantsFolder ? 'Folders' : '.pak files'

  // Load quick links once.
  useEffect(() => {
    invoke<QuickLink[]>('quick_links')
      .then(setQuickLinks)
      .catch((e) => console.error('quick_links failed', e))
  }, [])

  // Default cwd: home dir if none provided.
  useEffect(() => {
    if (initialPath) return
    if (quickLinks.length === 0) return
    const home = quickLinks.find((l) => l.kind === 'home')
    if (home) setCwd(home.path)
  }, [quickLinks, initialPath])

  // Load directory contents whenever cwd changes.
  const refresh = useCallback(
    async (path: string) => {
      setLoading(true)
      setError(null)
      setSelected(null)
      try {
        const list = await invoke<DirEntry[]>('list_dir', { path })
        const filtered = list.filter((e) => {
          if (e.is_dir) return true
          if (wantsFolder) return false
          return e.extension === 'pak'
        })
        // Folders first, then files. Alpha within each.
        filtered.sort((a, b) => {
          if (a.is_dir !== b.is_dir) return a.is_dir ? -1 : 1
          return a.name.localeCompare(b.name)
        })
        setEntries(filtered)
      } catch (e) {
        setError(String(e))
        setEntries([])
      } finally {
        setLoading(false)
      }
    },
    [wantsFolder],
  )

  useEffect(() => {
    if (cwd) refresh(cwd)
  }, [cwd, refresh])

  // Esc to cancel, Enter to confirm.
  useEffect(() => {
    function onKey(ev: KeyboardEvent) {
      if (ev.key === 'Escape') {
        ev.preventDefault()
        onCancel()
      }
    }
    window.addEventListener('keydown', onKey)
    return () => window.removeEventListener('keydown', onKey)
  }, [onCancel])

  const goUp = async () => {
    const parent = await invoke<string | null>('path_parent', { path: cwd })
    if (parent) setCwd(parent)
  }

  const onEntryClick = (e: DirEntry) => setSelected(e)
  const onEntryDoubleClick = (e: DirEntry) => {
    if (e.is_dir) {
      setCwd(e.path)
    } else if (!wantsFolder) {
      onConfirm(e.path)
    }
  }

  const onOpen = () => {
    if (wantsFolder) {
      // Folder mode: open the selected dir OR the current cwd.
      const chosen = selected?.is_dir ? selected.path : cwd
      if (chosen) onConfirm(chosen)
    } else {
      if (selected && !selected.is_dir) onConfirm(selected.path)
    }
  }

  const visible = useMemo(() => {
    const q = search.trim().toLowerCase()
    if (!q) return entries
    return entries.filter((e) => e.name.toLowerCase().includes(q))
  }, [entries, search])

  const crumbs = breadcrumbs(cwd)

  return (
    <div className="open-dialog-backdrop" onClick={(e) => e.target === e.currentTarget && onCancel()}>
      <div className="window open-dialog active glass" role="dialog" aria-label="Open">
        <div className="title-bar">
          <div className="title-bar-text">
            Open {wantsFolder ? 'folder' : '.pak file'}
          </div>
          <div className="title-bar-controls">
            <button aria-label="Close" onClick={onCancel} />
          </div>
        </div>
        <div className="window-body">
          {/* Path bar */}
          <div className="od-pathbar">
            <button onClick={goUp} title="Up one level" className="od-up">↑</button>
            <div className="od-breadcrumbs">
              {crumbs.map((c, i) => (
                <span key={c.path + i} className="od-crumb">
                  <button onClick={() => setCwd(c.path)}>{c.label}</button>
                  {i < crumbs.length - 1 && <span className="od-crumb-sep">›</span>}
                </span>
              ))}
            </div>
            <input
              className="od-search"
              type="text"
              placeholder="Search this folder"
              value={search}
              onChange={(e) => setSearch(e.target.value)}
            />
          </div>

          <div className="od-body">
            {/* Sidebar */}
            <aside className="od-sidebar">
              <div className="od-sidebar-group">
                <h4>Favorites</h4>
                <ul>
                  {quickLinks
                    .filter((l) => l.kind !== 'drive')
                    .map((l) => (
                      <li key={l.path}>
                        <button onClick={() => setCwd(l.path)} title={l.path}>
                          {l.label}
                        </button>
                      </li>
                    ))}
                </ul>
              </div>
              {recent && recent.length > 0 && (
                <div className="od-sidebar-group">
                  <h4>Recent</h4>
                  <ul>
                    {recent.slice(0, 6).map((r) => (
                      <li key={r}>
                        <button onClick={() => setCwd(r)} title={r}>
                          {r.split('/').pop() || r}
                        </button>
                      </li>
                    ))}
                  </ul>
                </div>
              )}
              {quickLinks.some((l) => l.kind === 'drive') && (
                <div className="od-sidebar-group">
                  <h4>Drives</h4>
                  <ul>
                    {quickLinks
                      .filter((l) => l.kind === 'drive')
                      .map((l) => (
                        <li key={l.path}>
                          <button onClick={() => setCwd(l.path)} title={l.path}>
                            {l.label}
                          </button>
                        </li>
                      ))}
                  </ul>
                </div>
              )}
            </aside>

            {/* File list */}
            <div className="od-list" ref={listRef}>
              {loading && <p className="od-empty">Loading…</p>}
              {error && <p className="od-error">{error}</p>}
              {!loading && !error && visible.length === 0 && (
                <p className="od-empty">
                  {search
                    ? 'No matches in this folder.'
                    : wantsFolder
                    ? 'No subfolders here.'
                    : 'No .pak files in this folder.'}
                </p>
              )}
              {!loading && !error && visible.length > 0 && (
                <table className="od-table">
                  <thead>
                    <tr>
                      <th className="od-col-name">Name</th>
                      <th className="od-col-modified">Date modified</th>
                      <th className="od-col-size">Size</th>
                    </tr>
                  </thead>
                  <tbody>
                    {visible.map((e) => (
                      <tr
                        key={e.path}
                        className={selected?.path === e.path ? 'od-row-on' : ''}
                        onClick={() => onEntryClick(e)}
                        onDoubleClick={() => onEntryDoubleClick(e)}
                      >
                        <td className="od-col-name">
                          <span className="od-icon">{e.is_dir ? '📁' : '📦'}</span>
                          <span className="od-name">{e.name}</span>
                        </td>
                        <td className="od-col-modified">{formatModified(e.modified)}</td>
                        <td className="od-col-size">
                          {e.is_dir ? '' : formatBytes(e.size)}
                        </td>
                      </tr>
                    ))}
                  </tbody>
                </table>
              )}
            </div>
          </div>

          {/* Footer: filename + filter + buttons */}
          <div className="od-footer">
            <div className="od-filename-row">
              <label>File name:</label>
              <input
                type="text"
                className="od-filename"
                readOnly
                value={
                  selected
                    ? selected.path
                    : wantsFolder
                    ? cwd
                    : ''
                }
              />
            </div>
            <div className="od-filename-row">
              <label>Files of type:</label>
              <input type="text" className="od-filename" readOnly value={filterLabel} />
            </div>
            <div className="od-buttons">
              <button
                className="primary"
                onClick={onOpen}
                disabled={
                  wantsFolder
                    ? !cwd
                    : !selected || selected.is_dir
                }
              >
                Open
              </button>
              <button onClick={onCancel}>Cancel</button>
            </div>
          </div>
        </div>
      </div>
    </div>
  )
}

````

## `src/AssetInspector.tsx`

````tsx
import { useMemo, useState } from 'react'
import { invoke } from '@tauri-apps/api/core'
import { OpenDialog } from './OpenDialog'

export type AssetEntry = {
  path: string
  size: number
  extension: string
  compression: string
  encrypted: boolean
  is_package: boolean
  is_payload: boolean
}

export type ListAssetsResult = {
  pak_path: string
  mount_point: string
  entry_count: number
  encrypted: boolean
  game: string
  entries: AssetEntry[]
  truncated: boolean
}

export type ExportInfo = { name: string; class_name: string; serial_size: number }
export type ImportInfo = { object_name: string; class_name: string; outer_package: string }
export type CustomVersionEntry = { key: string; version: number }
export type MipDescriptor = {
  index: number
  width: number
  height: number
  byte_size: number
}
export type TextureInfo = {
  name: string
  class_name: string
  pixel_format: string
  mip_count: number
  mips: MipDescriptor[]
  total_bytes: number
}
export type InspectAssetResult = {
  pak_path: string
  asset_path: string
  name_count: number
  import_count: number
  export_count: number
  file_version_ue: string
  custom_versions: CustomVersionEntry[]
  exports: ExportInfo[]
  imports: ImportInfo[]
  textures: TextureInfo[]
}

type Filter = 'all' | 'package' | 'payload' | 'audio' | 'texture' | 'other'

const PACKAGE_EXTS = new Set(['.uasset', '.uexp', '.umap', '.ubulk'])
const AUDIO_EXTS = new Set(['.wav', '.flac', '.ogg', '.opus', '.bnk', '.bin'])
const TEXTURE_EXTS = new Set(['.png', '.jpg', '.jpeg', '.tga', '.dds'])

function classify(e: AssetEntry): Filter {
  if (e.is_payload) return 'payload'
  if (e.is_package) return 'package'
  const ext = e.extension.toLowerCase()
  if (PACKAGE_EXTS.has(ext)) return 'package'
  if (AUDIO_EXTS.has(ext)) return 'audio'
  if (TEXTURE_EXTS.has(ext)) return 'texture'
  return 'other'
}

function formatBytes(b: number): string {
  if (!b) return '0 B'
  const u = ['B', 'KB', 'MB', 'GB', 'TB']
  let i = 0
  let n = b
  while (n >= 1024 && i < u.length - 1) {
    n /= 1024
    i++
  }
  return `${n.toFixed(n < 10 ? 2 : 1)} ${u[i]}`
}

export function AssetInspector() {
  const [result, setResult] = useState<ListAssetsResult | null>(null)
  const [loading, setLoading] = useState(false)
  const [error, setError] = useState<string | null>(null)
  const [filter, setFilter] = useState<Filter>('all')
  const [query, setQuery] = useState('')
  const [detail, setDetail] = useState<InspectAssetResult | null>(null)
  const [detailFor, setDetailFor] = useState<string | null>(null)
  const [loadingDetail, setLoadingDetail] = useState(false)
  const [detailError, setDetailError] = useState<string | null>(null)

  const [pakDialogOpen, setPakDialogOpen] = useState(false)

  function pickPak() {
    setError(null)
    setPakDialogOpen(true)
  }

  async function onPakChosen(sel: string) {
    setPakDialogOpen(false)
    setLoading(true)
    setDetail(null)
    setDetailFor(null)
    setDetailError(null)
    try {
      const r = await invoke<ListAssetsResult>('sidecar_list_assets', {
        pakPath: sel,
        limit: 5000,
      })
      setResult(r)
      setFilter('all')
      setQuery('')
    } catch (e) {
      setError(String(e))
      setResult(null)
    } finally {
      setLoading(false)
    }
  }

  async function inspectAsset(assetPath: string) {
    if (!result) return
    setLoadingDetail(true)
    setDetailFor(assetPath)
    setDetail(null)
    setDetailError(null)
    try {
      const r = await invoke<InspectAssetResult>('sidecar_inspect_asset', {
        pakPath: result.pak_path,
        assetPath,
      })
      setDetail(r)
    } catch (e) {
      setDetailError(String(e))
    } finally {
      setLoadingDetail(false)
    }
  }

  const counts = useMemo(() => {
    if (!result) return null
    const c: Record<Filter, number> = {
      all: result.entries.length,
      package: 0,
      payload: 0,
      audio: 0,
      texture: 0,
      other: 0,
    }
    for (const e of result.entries) c[classify(e)]++
    return c
  }, [result])

  const visible = useMemo(() => {
    if (!result) return []
    const q = query.trim().toLowerCase()
    return result.entries
      .filter((e) => filter === 'all' || classify(e) === filter)
      .filter((e) => !q || e.path.toLowerCase().includes(q))
      .slice() // copy before sort
      .sort((a, b) => b.size - a.size)
      .slice(0, 300)
  }, [result, filter, query])

  const empty = !result && !error && !loading
  return (
    <section className={`report inspector-card${empty ? ' inspector-card--empty' : ''}`}>
      {pakDialogOpen && (
        <OpenDialog
          mode="pak"
          initialPath={result?.pak_path
            ? result.pak_path.replace(/\/[^/]+$/, '')
            : null}
          onConfirm={onPakChosen}
          onCancel={() => setPakDialogOpen(false)}
        />
      )}
      <header className="inspector-head">
        <div>
          <h2>
            Inspect a pak <span className="preview-tag">preview · Phase 2</span>
          </h2>
          <p className="muted small inspector-blurb">
            Opens any readable .pak via CUE4Parse and lists every cooked entry.
            Building block for in-pak rewriting (mip stripping, audio re-encode,
            L10N inside encrypted-sibling paks) coming in v0.5.
          </p>
        </div>
        <button
          onClick={pickPak}
          disabled={loading}
          className={empty ? 'primary' : ''}
        >
          {loading ? 'reading…' : result ? 'pick different pak' : 'choose .pak'}
        </button>
      </header>

      {error && <p className="err">{error}</p>}

      {result && !error && (
        <>
          <table className="inspector-meta">
            <tbody>
              <tr>
                <th>pak</th>
                <td className="path-small" title={result.pak_path}>
                  {result.pak_path}
                </td>
              </tr>
              <tr>
                <th>mount point</th>
                <td className="path-small">{result.mount_point || '—'}</td>
              </tr>
              <tr>
                <th>entries</th>
                <td>
                  {result.entry_count.toLocaleString()}{' '}
                  {result.truncated && (
                    <span className="muted small">
                      (showing first {result.entries.length.toLocaleString()})
                    </span>
                  )}
                </td>
              </tr>
              <tr>
                <th>encryption</th>
                <td>
                  {result.encrypted ? (
                    <span className="badge-warn">AES-encrypted (key required)</span>
                  ) : (
                    <span className="badge-ok">readable</span>
                  )}
                </td>
              </tr>
              <tr>
                <th>parser version</th>
                <td className="muted small">{result.game}</td>
              </tr>
            </tbody>
          </table>

          {result.encrypted ? (
            <p className="muted small" style={{ marginTop: '0.8rem' }}>
              This pak's index is AES-encrypted. Phase 2 will accept{' '}
              <code>--aes-key</code> + FModel-style <code>keys.json</code> to
              unlock per-game content.
            </p>
          ) : (
            <>
              <div className="inspector-filters" style={{ marginTop: '0.9rem' }}>
                {(['all', 'package', 'payload', 'audio', 'texture', 'other'] as Filter[]).map(
                  (k) => (
                    <button
                      key={k}
                      className={`chip ${filter === k ? 'chip-on' : ''}`}
                      onClick={() => setFilter(k)}
                    >
                      {k} {counts ? `(${counts[k].toLocaleString()})` : ''}
                    </button>
                  ),
                )}
                <input
                  className="inspector-search"
                  placeholder="filter by path…"
                  value={query}
                  onChange={(e) => setQuery(e.target.value)}
                />
              </div>

              <table className="inspector-table">
                <thead>
                  <tr>
                    <th>path</th>
                    <th>kind</th>
                    <th>compression</th>
                    <th className="size-col">size</th>
                    <th></th>
                  </tr>
                </thead>
                <tbody>
                  {visible.map((e, i) => {
                    const inspectable = e.is_package || e.path.toLowerCase().endsWith('.uasset')
                      || e.path.toLowerCase().endsWith('.umap')
                    const isSelected = detailFor === e.path
                    return (
                      <tr
                        key={`${e.path}-${i}`}
                        className={isSelected ? 'inspector-row-on' : ''}
                      >
                        <td className="path-small" title={e.path}>
                          {e.path}
                        </td>
                        <td className="kind">
                          {e.is_payload
                            ? 'payload'
                            : e.is_package
                            ? 'package'
                            : e.extension || '—'}
                        </td>
                        <td className="muted small">{e.compression}</td>
                        <td className="size">{formatBytes(e.size)}</td>
                        <td>
                          {inspectable && (
                            <button
                              className="chip"
                              onClick={() => inspectAsset(e.path)}
                              disabled={loadingDetail && detailFor === e.path}
                            >
                              {loadingDetail && detailFor === e.path
                                ? 'reading…'
                                : 'inspect'}
                            </button>
                          )}
                        </td>
                      </tr>
                    )
                  })}
                </tbody>
              </table>
              {visible.length === 0 && (
                <p className="muted small">no entries match this filter</p>
              )}

              {detailFor && (
                <div className="inspector-detail">
                  <div className="inspector-detail-head">
                    <h3>
                      package detail
                      <span className="preview-tag">Phase 2 read-side</span>
                    </h3>
                    <button className="chip" onClick={() => { setDetail(null); setDetailFor(null); setDetailError(null); }}>close</button>
                  </div>
                  <p className="path-small inspector-detail-path">{detailFor}</p>
                  {detailError && <p className="err">{detailError}</p>}
                  {loadingDetail && <p className="muted small">parsing package…</p>}
                  {detail && !detailError && (
                    <>
                      <table className="inspector-meta">
                        <tbody>
                          <tr>
                            <th>UE version</th>
                            <td className="muted small">{detail.file_version_ue}</td>
                          </tr>
                          <tr>
                            <th>names</th>
                            <td>{detail.name_count.toLocaleString()}</td>
                          </tr>
                          <tr>
                            <th>imports</th>
                            <td>
                              {detail.import_count.toLocaleString()}{' '}
                              <span className="muted small">
                                (cross-package deps)
                              </span>
                            </td>
                          </tr>
                          <tr>
                            <th>exports</th>
                            <td>
                              {detail.export_count.toLocaleString()}{' '}
                              <span className="muted small">
                                (objects inside this package)
                              </span>
                            </td>
                          </tr>
                          <tr>
                            <th>custom versions</th>
                            <td>{detail.custom_versions.length}</td>
                          </tr>
                        </tbody>
                      </table>

                      {detail.exports.length > 0 && (
                        <>
                          <h4 className="inspector-detail-sub">exports</h4>
                          <table className="inspector-table">
                            <thead>
                              <tr>
                                <th>name</th>
                                <th>class</th>
                              </tr>
                            </thead>
                            <tbody>
                              {detail.exports.slice(0, 30).map((ex, i) => (
                                <tr key={i}>
                                  <td className="path-small">{ex.name}</td>
                                  <td className="muted small">{ex.class_name}</td>
                                </tr>
                              ))}
                            </tbody>
                          </table>
                          {detail.exports.length > 30 && (
                            <p className="muted small">
                              …and {detail.exports.length - 30} more
                            </p>
                          )}
                        </>
                      )}

                      {detail.textures && detail.textures.length > 0 && (
                        <>
                          {detail.textures.slice(0, 6).map((t, ti) => (
                            <div key={ti} className="texture-block">
                              <h4 className="inspector-detail-sub">
                                texture · {t.name}{' '}
                                <span className="muted small">
                                  {t.class_name} · {t.pixel_format} · {t.mip_count} mip(s) · {formatBytes(t.total_bytes)}
                                </span>
                              </h4>
                              <table className="inspector-table mip-table">
                                <thead>
                                  <tr>
                                    <th>mip</th>
                                    <th>w</th>
                                    <th>h</th>
                                    <th className="size-col">bytes</th>
                                  </tr>
                                </thead>
                                <tbody>
                                  {t.mips.map((m) => (
                                    <tr key={m.index}>
                                      <td>{m.index}</td>
                                      <td>{m.width}</td>
                                      <td>{m.height}</td>
                                      <td className="size">{formatBytes(m.byte_size)}</td>
                                    </tr>
                                  ))}
                                </tbody>
                              </table>
                            </div>
                          ))}
                          {detail.textures.length > 6 && (
                            <p className="muted small">
                              …and {detail.textures.length - 6} more texture(s)
                            </p>
                          )}
                        </>
                      )}

                      {detail.custom_versions.length > 0 && (
                        <>
                          <h4 className="inspector-detail-sub">custom versions</h4>
                          <ul className="reasons">
                            {detail.custom_versions.slice(0, 12).map((cv, i) => (
                              <li key={i}>
                                <span className="path-small">{cv.key}</span>
                                <span className="muted small"> · v{cv.version}</span>
                              </li>
                            ))}
                            {detail.custom_versions.length > 12 && (
                              <li className="muted small">
                                …and {detail.custom_versions.length - 12} more
                              </li>
                            )}
                          </ul>
                        </>
                      )}
                    </>
                  )}
                </div>
              )}
            </>
          )}
        </>
      )}

      {!result && !error && !loading && (
        <p className="placeholder inspector-empty-hint">
          Works on modded paks, asset flips, dev builds, and cooked UE4 or UE5
          games. Encrypted paks surface the AES affordance instead of erroring.
        </p>
      )}
    </section>
  )
}

````

## `src/MipStripPanel.tsx`

````tsx
import { useEffect, useMemo, useRef, useState } from 'react'
import { invoke } from '@tauri-apps/api/core'
import { listen, type UnlistenFn } from '@tauri-apps/api/event'
import { OpenDialog } from './OpenDialog'

type StripMipsLevel = { w: number; h: number; bytes: number }

type StripMipsItem = {
  asset_path: string
  export_name: string
  class_name: string
  pixel_format: string
  current_mip0_dim: number
  kept_mip0_dim: number
  drop_mip_count: number
  kept_mip_count: number
  save_bytes: number
  original_bytes: number
  compression_settings: string | null
  /// v0.7.2: full mip pyramid. When present, `onDimChange` re-projects this
  /// item locally instead of round-tripping to the sidecar planner.
  mips?: StripMipsLevel[]
}

type ClassCount = { class_name: string; count: number }

type PlanStripMipsResult = {
  pak_path: string
  max_dim: number
  scanned_assets: number
  texture_count: number
  items: StripMipsItem[]
  total_save_bytes: number
  total_texture_bytes: number
  truncated: boolean
  class_histogram?: ClassCount[]
}

type RestoreClass = 'ai_upscale' | 'exact_backup' | 'no_strip'

type PlannedTexture = {
  asset_path: string
  export_name: string
  class: RestoreClass
  pixel_format: string
}

type RestoreAiPlan = {
  policy: string
  textures: PlannedTexture[]
  ai_count: number
  backup_count: number
  skip_count: number
  executor_ready: boolean
  executor_phase: string
  executor_notes: string[]
}

type StripAppliedFile = { pak_path: string; bytes_base64: string }

type StripAppliedTexture = {
  asset_path: string
  export_name: string
  drop_mip_count: number
  kept_mip_count: number
  original_top_dim: number
  kept_top_dim: number
  saved_bytes: number
  files: StripAppliedFile[]
  original_files: StripAppliedFile[]
}

type StripSkipped = { asset_path: string; reason: string }

type ApplyStripMipsResult = {
  pak_path: string
  engine_version: string
  applied: StripAppliedTexture[]
  skipped: StripSkipped[]
  total_saved_bytes: number
}

// v0.6.1: shape returned by the new apply_strip_mips_to_folder command.
// Mirrors shrinkray_core::texture_strip::PakStripReport.
type TextureStripRecord = {
  asset_path: string
  export_name: string
  original_top_dim: number
  stripped_top_dim: number
  drop_mip_count: number
  kept_mip_count: number
  pixel_format: string
  compression_settings: string | null
  saved_bytes: number
}

type PakStripReport = {
  pak: string
  original_size: number
  new_size: number
  applied: TextureStripRecord[]
  skipped: StripSkipped[]
  total_saved_bytes: number
}

// v0.7.2 strip-progress event payloads — emitted from the C# sidecar via
// ProgressEmitter.Emit. Two shapes share the channel: one per texture during
// apply, one per ~10 packages during plan. Discriminate on `op`.
type ApplyProgress = {
  op: 'apply_strip_mips'
  current: number
  total: number
  asset_path: string
  status: 'applied' | 'skipped'
  saved_bytes: number
  reason: string | null
}

type PlanProgress = {
  op: 'plan_strip_mips'
  current: number
  total: number
  asset_path: string
}

type StripProgress = ApplyProgress | PlanProgress

const MAX_DIMS = [4096, 2048, 1024, 512]

const GAME_VERSIONS = [
  { value: 'GAME_UE5_LATEST', label: 'UE5 (latest)' },
  { value: 'GAME_UE5_6', label: 'UE5.6' },
  { value: 'GAME_UE5_5', label: 'UE5.5' },
  { value: 'GAME_UE5_4', label: 'UE5.4' },
  { value: 'GAME_UE5_3', label: 'UE5.3' },
  { value: 'GAME_UE5_0', label: 'UE5.0' },
  { value: 'GAME_UE4_LATEST', label: 'UE4 (latest)' },
  { value: 'GAME_UE4_27', label: 'UE4.27' },
  { value: 'GAME_UE4_25', label: 'UE4.25' },
  { value: 'GAME_UE4_22', label: 'UE4.22 (Pamali-era)' },
]

// Engine-version → UAssetAPI EngineVersion string. The .NET sidecar takes a
// UAssetAPI enum name (VER_UE4_22 etc) for the write-side parser. Best-effort
// mapping; falls back to AUTOMATIC if unknown.
function uassetapiVerFor(game: string): string {
  const m: Record<string, string> = {
    GAME_UE4_22: 'VER_UE4_22',
    GAME_UE4_25: 'VER_UE4_25',
    GAME_UE4_27: 'VER_UE4_27',
    GAME_UE4_LATEST: 'VER_UE4_27',
    GAME_UE5_0: 'VER_UE5_0',
    GAME_UE5_3: 'VER_UE5_3',
    GAME_UE5_4: 'VER_UE5_4',
    GAME_UE5_5: 'VER_UE5_5',
    GAME_UE5_6: 'VER_UE5_6',
    GAME_UE5_LATEST: 'VER_UE5_6',
  }
  return m[game] ?? 'VER_UE4_AUTOMATIC_VERSION'
}

// v0.7.2: re-project a cached StripMipsItem at a new max_dim. Returns null
// when the texture has no mip small enough or is already at/below cap — same
// "drop this from the list" signal the sidecar produces via `save_bytes <= 0`.
// Mirrors the C# `ProjectStripFromUObject` formula 1:1; if either side ever
// changes the math, update both.
function reproject(item: StripMipsItem, maxDim: number): StripMipsItem | null {
  if (!item.mips || item.mips.length === 0) return item
  const mips = item.mips
  let firstKept = mips.length - 1
  for (let i = 0; i < mips.length; i++) {
    if (Math.max(mips[i].w, mips[i].h) <= maxDim) {
      firstKept = i
      break
    }
  }
  if (firstKept <= 0) return null
  let saveBytes = 0
  for (let i = 0; i < firstKept; i++) saveBytes += mips[i].bytes
  if (saveBytes <= 0) return null
  const kept = mips[firstKept]
  return {
    ...item,
    drop_mip_count: firstKept,
    kept_mip_count: mips.length - firstKept,
    kept_mip0_dim: Math.max(kept.w, kept.h),
    save_bytes: saveBytes,
  }
}

function reprojectPlan(plan: PlanStripMipsResult, maxDim: number): PlanStripMipsResult {
  const items: StripMipsItem[] = []
  let totalSave = 0
  let totalTex = 0
  for (const item of plan.items) {
    const repro = reproject(item, maxDim)
    if (!repro) continue
    items.push(repro)
    totalSave += repro.save_bytes
    // total_texture_bytes counts only textures we'd actually strip — keeps
    // the "save / texture bytes" ratio meaningful (the original v0.6 UX).
    totalTex += repro.original_bytes
  }
  items.sort((a, b) => b.save_bytes - a.save_bytes)
  return {
    ...plan,
    max_dim: maxDim,
    items,
    total_save_bytes: totalSave,
    total_texture_bytes: totalTex,
  }
}

function formatBytes(b: number): string {
  if (!b) return '0 B'
  const u = ['B', 'KB', 'MB', 'GB', 'TB']
  let i = 0
  let n = b
  while (n >= 1024 && i < u.length - 1) {
    n /= 1024
    i++
  }
  return `${n.toFixed(n < 10 && i > 0 ? 2 : 1)} ${u[i]}`
}

function classLabel(c: RestoreClass): string {
  switch (c) {
    case 'ai_upscale':
      return 'AI'
    case 'exact_backup':
      return 'backup'
    case 'no_strip':
      return 'skip'
  }
}

function classClass(c: RestoreClass): string {
  switch (c) {
    case 'ai_upscale':
      return 'restore-ai'
    case 'exact_backup':
      return 'restore-backup'
    case 'no_strip':
      return 'restore-skip'
  }
}

type MipStripPanelProps = {
  /// Game folder root chosen at the App level. When set + backupLoaded is true
  /// + previewOnly is false, the "apply" button triggers the real write-back
  /// flow (apply_strip_mips_to_folder) instead of the bytes-only sidecar call.
  folderPath: string | null
  /// Whether a shrinkray_backup exists for `folderPath`. Gates the write path.
  backupLoaded: boolean
  /// Global preview-only toggle in the App header. When true, the apply
  /// button stays read-only (sidecar bytes only, no disk write).
  previewOnly: boolean
}

export function MipStripPanel({ folderPath, backupLoaded, previewOnly }: MipStripPanelProps) {
  const [pak, setPak] = useState<string | null>(null)
  const [maxDim, setMaxDim] = useState<number>(2048)
  const [game, setGame] = useState<string>('GAME_UE5_LATEST')
  const [dialogOpen, setDialogOpen] = useState(false)
  const [plan, setPlan] = useState<PlanStripMipsResult | null>(null)
  const [aiPlan, setAiPlan] = useState<RestoreAiPlan | null>(null)
  const [loading, setLoading] = useState(false)
  const [error, setError] = useState<string | null>(null)
  const [applyResult, setApplyResult] = useState<ApplyStripMipsResult | null>(null)
  const [writeReport, setWriteReport] = useState<PakStripReport | null>(null)
  const [applying, setApplying] = useState(false)
  const [exemptNormals, setExemptNormals] = useState(true)

  // v0.7.2 streamed-progress state — one update per texture the sidecar
  // finishes. progress goes back to null once apply settles so the UI can
  // re-show the static result summary.
  const [progress, setProgress] = useState<StripProgress | null>(null)
  const [progressStartedAt, setProgressStartedAt] = useState<number | null>(null)
  const [elapsedTick, setElapsedTick] = useState(0)
  const unlistenRef = useRef<UnlistenFn | null>(null)

  // Subscribe to strip-progress events whenever we're running a sidecar
  // operation (planning OR applying). Clean up between runs so we don't leak
  // listeners across folder/pak swaps.
  const busy = loading || applying
  useEffect(() => {
    if (!busy) return
    let cancelled = false
    listen<StripProgress>('strip-progress', (event) => {
      if (cancelled) return
      setProgress(event.payload)
    }).then((un) => {
      if (cancelled) un()
      else unlistenRef.current = un
    })
    return () => {
      cancelled = true
      if (unlistenRef.current) {
        unlistenRef.current()
        unlistenRef.current = null
      }
    }
  }, [busy])

  // Ticking elapsed timer so the UI doesn't look frozen between progress
  // events (some textures take seconds and stderr-quiet stretches feel like
  // hangs without a live counter).
  useEffect(() => {
    if (!busy) return
    const id = window.setInterval(() => setElapsedTick((t) => t + 1), 500)
    return () => window.clearInterval(id)
  }, [busy])

  // Write mode is enabled only when the App has a folder + a loaded backup
  // and the user hasn't toggled the global preview-only switch on.
  const canWriteToDisk = !!folderPath && backupLoaded && !previewOnly

  // Map asset_path → RestoreClass, derived from aiPlan. Used to render the
  // routing per row + to filter the apply targets (skip "no_strip" textures).
  const classByAsset = useMemo(() => {
    const m = new Map<string, RestoreClass>()
    if (aiPlan) {
      for (const t of aiPlan.textures) m.set(t.asset_path, t.class)
    }
    return m
  }, [aiPlan])

  async function tryApply() {
    if (!pak || !plan) return
    setApplyResult(null)
    setWriteReport(null)
    setProgress(null)
    setProgressStartedAt(Date.now())
    setElapsedTick(0)
    setApplying(true)
    setError(null)
    try {
      // Drop NoStrip-class textures (lookups / data) unconditionally; drop
      // ExactBackup-class (normals etc) when the user has the exempt toggle
      // on (default). Surviving targets get either the bytes-only sidecar
      // call (preview) or the real disk-write command (write mode).
      const targets = plan.items
        .filter((it) => {
          const c = classByAsset.get(it.asset_path)
          if (c === 'no_strip') return false
          if (exemptNormals && c === 'exact_backup') return false
          return true
        })
        .map((it) => ({ asset_path: it.asset_path, max_dim: maxDim }))

      if (canWriteToDisk && folderPath) {
        const r = await invoke<PakStripReport>('apply_strip_mips_to_folder', {
          folderPath,
          pakPath: pak,
          targets,
          game,
          engineVersion: uassetapiVerFor(game),
        })
        setWriteReport(r)
      } else {
        const r = await invoke<ApplyStripMipsResult>('sidecar_apply_strip_mips', {
          pakPath: pak,
          targets,
          game,
          engineVersion: uassetapiVerFor(game),
        })
        setApplyResult(r)
      }
    } catch (e) {
      setError(String(e))
    } finally {
      setApplying(false)
      setProgressStartedAt(null)
      // Keep `progress` set briefly so the user sees the final "N/N done"
      // before it collapses to the result summary; the result-summary block
      // takes over rendering once writeReport / applyResult is non-null.
    }
  }

  async function runPlan(pakPath: string, dim: number, g: string) {
    setLoading(true)
    setError(null)
    setPlan(null)
    setAiPlan(null)
    setApplyResult(null)
    setProgress(null)
    setProgressStartedAt(Date.now())
    setElapsedTick(0)
    try {
      const raw = await invoke<PlanStripMipsResult>('sidecar_plan_strip_mips', {
        pakPath,
        maxDim: dim,
        limit: 5000,
        game: g,
      })
      // v0.7.2: the planner returns every cooked texture (including ones
      // that wouldn't save bytes at this cap) so client-side reprojection
      // works at other caps. Filter to displayable items via the same
      // reproject pass that handles dim-change.
      setPlan(reprojectPlan(raw, dim))
      // Second call: ask the classifier for per-texture restore routing.
      // Same pak + max_dim so the planner items line up 1:1 with the AI plan.
      try {
        const ai = await invoke<RestoreAiPlan>('sidecar_plan_restore_ai', {
          pakPath,
          maxDim: dim,
          limit: 5000,
          game: g,
          policy: 'smart',
        })
        setAiPlan(ai)
      } catch (_e) {
        // Don't fail the whole panel if the AI plan can't be computed —
        // restore classes are presentational, the planner data is the load-bearing piece.
        setAiPlan(null)
      }
    } catch (e) {
      setError(String(e))
    } finally {
      setLoading(false)
      // Don't clear progress here — leave the final tick visible briefly
      // for the user; runPlan rendering the plan table replaces it.
    }
  }

  function onPakChosen(p: string) {
    setDialogOpen(false)
    setPak(p)
    runPlan(p, maxDim, game)
  }

  function onDimChange(dim: number) {
    setMaxDim(dim)
    // v0.7.2: if the cached plan has per-mip data, re-project locally — no
    // sidecar round-trip, response is instant. Re-plan only when we don't
    // have mip data yet (legacy planner response).
    //
    // We deliberately don't re-call the AI restore classifier here: per-row
    // routing (AI / backup / skip) is dim-independent at the texture level
    // (it's driven by CompressionSettings + pixel_format + name, not the
    // cap), so the existing aiPlan stays correct enough. The aggregate
    // counts can drift slightly when textures move in/out of the "would be
    // stripped" set, but that's cosmetic and not worth a fresh pak walk.
    if (plan && plan.items.length > 0 && plan.items[0].mips && plan.items[0].mips.length > 0) {
      setPlan(reprojectPlan(plan, dim))
      return
    }
    if (pak) runPlan(pak, dim, game)
  }

  function onGameChange(g: string) {
    setGame(g)
    if (pak) runPlan(pak, maxDim, g)
  }

  const savePct =
    plan && plan.total_texture_bytes > 0
      ? (plan.total_save_bytes / plan.total_texture_bytes) * 100
      : 0

  const formatBreakdown = useMemo(() => {
    if (!plan) return [] as { format: string; count: number; save: number }[]
    const m = new Map<string, { count: number; save: number }>()
    for (const it of plan.items) {
      const cur = m.get(it.pixel_format) ?? { count: 0, save: 0 }
      cur.count++
      cur.save += it.save_bytes
      m.set(it.pixel_format, cur)
    }
    return Array.from(m.entries())
      .map(([format, v]) => ({ format, ...v }))
      .sort((a, b) => b.save - a.save)
  }, [plan])

  // Skip-reason histogram for the apply result (either preview or write mode
  // — same shape on `skipped`). Multiple textures can share the same reason
  // ("inline payload mip", "asset not found", etc); collapse so the UI doesn't
  // render hundreds of identical lines.
  const skipHistogram = useMemo(() => {
    const source = writeReport?.skipped ?? applyResult?.skipped ?? []
    if (source.length === 0) return [] as { reason: string; count: number }[]
    const m = new Map<string, number>()
    for (const s of source) m.set(s.reason, (m.get(s.reason) ?? 0) + 1)
    return Array.from(m.entries())
      .map(([reason, count]) => ({ reason, count }))
      .sort((a, b) => b.count - a.count)
  }, [applyResult, writeReport])

  useEffect(() => {
    if (plan?.pak_path) document.title = `shrinkray · ${plan.pak_path.split('/').pop()}`
  }, [plan?.pak_path])

  return (
    <section className="report mipstrip-card">
      {dialogOpen && (
        <OpenDialog
          mode="pak"
          initialPath={pak ? pak.replace(/\/[^/]+$/, '') : null}
          onConfirm={onPakChosen}
          onCancel={() => setDialogOpen(false)}
        />
      )}
      <header className="inspector-head">
        <div>
          <h2>
            Texture mip strip{' '}
            <span className="preview-tag">v0.6.2 · real shrinkage</span>
          </h2>
          <p className="muted small inspector-blurb">
            Pick a readable UE4 .pak and a maximum texture dimension. The plan
            walks every cooked texture, projects savings, and routes each
            texture through the classifier (AI re-expand vs exact backup vs
            skip). With a game folder + loaded backup, "apply" rewrites the
            pak on disk and records strip metadata for v0.7's AI restore. v0.6.2
            ships a patched repak so cooked AAA paks actually get smaller
            (Pamali T_hairMask03 strip: 13 MB net shrinkage on disk).
          </p>
        </div>
        <button
          onClick={() => setDialogOpen(true)}
          disabled={loading || applying}
          className={!pak ? 'primary' : ''}
        >
          {loading ? 'planning…' : pak ? 'pick different pak' : 'choose .pak'}
        </button>
      </header>

      {pak && (
        <>
          <div className="mipstrip-controls">
            <span className="muted small">cap mip 0 at:</span>
            {MAX_DIMS.map((d) => (
              <button
                key={d}
                className={`chip ${maxDim === d ? 'chip-on' : ''}`}
                onClick={() => onDimChange(d)}
                disabled={loading || applying}
              >
                {d}px
              </button>
            ))}
          </div>
          <div className="mipstrip-controls">
            <span className="muted small">engine version:</span>
            <select
              className="mipstrip-game"
              value={game}
              onChange={(e) => onGameChange(e.target.value)}
              disabled={loading || applying}
            >
              {GAME_VERSIONS.map((g) => (
                <option key={g.value} value={g.value}>
                  {g.label}
                </option>
              ))}
            </select>
            <span className="muted small">
              ← if textures show as 0 on a UE4 game, switch to a UE4 version
            </span>
          </div>
          <div className="mipstrip-controls">
            <label className="muted small">
              <input
                type="checkbox"
                checked={exemptNormals}
                onChange={(e) => setExemptNormals(e.target.checked)}
                disabled={applying}
              />{' '}
              exempt normals + data textures (recommended — these need byte-exact restore)
            </label>
          </div>
        </>
      )}

      {error && <p className="err">{error}</p>}

      {busy && progress && (() => {
        void elapsedTick
        const elapsedMs = progressStartedAt ? Date.now() - progressStartedAt : 0
        const elapsedSec = Math.max(0, Math.floor(elapsedMs / 1000))
        const pct = progress.total > 0 ? (progress.current / progress.total) * 100 : 0
        const etaSec = progress.current > 0 && progress.current < progress.total
          ? Math.round((elapsedMs / progress.current) * (progress.total - progress.current) / 1000)
          : null
        const op = progress.op === 'apply_strip_mips' ? 'applying' : 'planning'
        const statusPrefix = progress.op === 'apply_strip_mips'
          ? (progress.status === 'skipped' ? '↷ ' : '✓ ')
          : '· '
        return (
          <div className="mipstrip-progress" role="status" aria-live="polite">
            <div className="mipstrip-progress-bar" aria-hidden>
              <div
                className="mipstrip-progress-fill"
                style={{ width: `${pct.toFixed(1)}%` }}
              />
            </div>
            <div className="muted small mipstrip-progress-meta">
              <span title={progress.asset_path}>
                {op} {progress.current}/{progress.total}: {statusPrefix}
                {progress.asset_path.split('/').pop()}
              </span>
              <span>
                {elapsedSec}s elapsed
                {etaSec !== null && ` · ~${etaSec}s remaining`}
              </span>
            </div>
          </div>
        )
      })()}

      {plan && !error && (
        <>
          <table className="inspector-meta">
            <tbody>
              <tr>
                <th>pak</th>
                <td className="path-small" title={plan.pak_path}>
                  {plan.pak_path}
                </td>
              </tr>
              <tr>
                <th>scanned</th>
                <td>
                  {plan.scanned_assets.toLocaleString()} asset(s){' '}
                  {plan.truncated && (
                    <span className="muted small">(truncated — bump limit if needed)</span>
                  )}
                </td>
              </tr>
              <tr>
                <th>textures</th>
                <td>
                  {plan.items.length.toLocaleString()} would shrink
                  {plan.texture_count > plan.items.length && (
                    <span className="muted small">
                      {' '}· {plan.texture_count.toLocaleString()} total cooked
                    </span>
                  )}
                </td>
              </tr>
              <tr>
                <th>total texture bytes</th>
                <td>{formatBytes(plan.total_texture_bytes)}</td>
              </tr>
              <tr>
                <th>projected save</th>
                <td className="accent">
                  {formatBytes(plan.total_save_bytes)} ({savePct.toFixed(1)}%)
                </td>
              </tr>
              {aiPlan && (
                <tr>
                  <th>restore plan</th>
                  <td>
                    <span className="restore-ai">AI: {aiPlan.ai_count}</span>{' '}
                    ·{' '}
                    <span className="restore-backup">
                      backup: {aiPlan.backup_count}
                    </span>{' '}
                    · <span className="restore-skip">skip: {aiPlan.skip_count}</span>{' '}
                    <span className="muted small">
                      (executor in {aiPlan.executor_phase})
                    </span>
                  </td>
                </tr>
              )}
            </tbody>
          </table>

          {formatBreakdown.length > 0 && (
            <div className="mipstrip-formats">
              <h4>By pixel format</h4>
              <table className="inspector-table">
                <thead>
                  <tr>
                    <th>format</th>
                    <th>textures</th>
                    <th className="size-col">save</th>
                  </tr>
                </thead>
                <tbody>
                  {formatBreakdown.map((f) => (
                    <tr key={f.format}>
                      <td className="kind">{f.format}</td>
                      <td>{f.count.toLocaleString()}</td>
                      <td className="size">{formatBytes(f.save)}</td>
                    </tr>
                  ))}
                </tbody>
              </table>
            </div>
          )}

          {plan.items.length > 0 ? (
            <table className="inspector-table mipstrip-items">
              <thead>
                <tr>
                  <th>asset</th>
                  <th>format</th>
                  <th>compression</th>
                  <th>restore</th>
                  <th>mip 0</th>
                  <th>→ keep</th>
                  <th>drop</th>
                  <th className="size-col">save</th>
                </tr>
              </thead>
              <tbody>
                {plan.items.slice(0, 200).map((it, i) => {
                  const c = classByAsset.get(it.asset_path)
                  return (
                    <tr key={`${it.asset_path}-${i}`}>
                      <td className="path-small" title={it.asset_path}>
                        {it.export_name}
                      </td>
                      <td className="kind">{it.pixel_format}</td>
                      <td className="kind">
                        {it.compression_settings ?? <span className="muted">—</span>}
                      </td>
                      <td className={c ? classClass(c) : ''}>
                        {c ? classLabel(c) : '?'}
                      </td>
                      <td>{it.current_mip0_dim}px</td>
                      <td>{it.kept_mip0_dim}px</td>
                      <td>{it.drop_mip_count}</td>
                      <td className="size">{formatBytes(it.save_bytes)}</td>
                    </tr>
                  )
                })}
              </tbody>
            </table>
          ) : (
            <p className="muted small">
              No textures in this pak exceed the {maxDim}px cap — nothing to
              strip. Try a lower cap.
            </p>
          )}
          {plan.items.length > 200 && (
            <p className="muted small">
              Showing top 200 of {plan.items.length} items by savings.
            </p>
          )}

          {plan.items.length > 0 && (
            <div className="mipstrip-apply">
              <button onClick={tryApply} disabled={loading || applying} className="primary">
                {applying
                  ? progress && progress.op === 'apply_strip_mips'
                    ? `${canWriteToDisk ? 'writing' : 'computing'} ${progress.current}/${progress.total}…`
                    : canWriteToDisk
                      ? 'writing to pak…'
                      : 'computing…'
                  : canWriteToDisk
                    ? 'apply (write to pak)'
                    : 'apply (preview — no disk write)'}
              </button>
              <p className="muted small">
                {canWriteToDisk ? (
                  <>
                    Write mode is live. Backed up at the folder root —
                    <code> shrinkray restore</code> reverts the pak byte-exact
                    if you change your mind.
                  </>
                ) : (
                  <>
                    {!folderPath
                      ? 'Pick a game folder in the panel above to enable write mode.'
                      : !backupLoaded
                        ? 'Backup is required before write mode. Click "ensure backup" above.'
                        : 'Preview-only mode is on. Toggle it off in the header to write to disk.'}
                  </>
                )}
              </p>

              {writeReport && (
                <div className="mipstrip-apply-note">
                  <strong>
                    rewrote pak · applied: {writeReport.applied.length} ·
                    skipped: {writeReport.skipped.length} · saved:{' '}
                    {formatBytes(writeReport.total_saved_bytes)}
                  </strong>
                  <p className="muted small">
                    {formatBytes(writeReport.original_size)} →{' '}
                    {formatBytes(writeReport.new_size)} on disk
                    {writeReport.applied.length > 0 && (
                      <>
                        {' · '}backup manifest carries {writeReport.applied.length}{' '}
                        strip record(s) for v0.7 restore.
                      </>
                    )}
                  </p>
                  {skipHistogram.length > 0 && (
                    <details>
                      <summary>
                        skip reasons ({skipHistogram.length} distinct)
                      </summary>
                      <ul className="reasons">
                        {skipHistogram.map((s) => (
                          <li key={s.reason}>
                            <span className="muted small">
                              {s.count.toLocaleString()} ×
                            </span>{' '}
                            {s.reason}
                          </li>
                        ))}
                      </ul>
                    </details>
                  )}
                </div>
              )}

              {applyResult && !writeReport && (
                <div className="mipstrip-apply-note">
                  <strong>
                    preview · applied: {applyResult.applied.length} · skipped:{' '}
                    {applyResult.skipped.length} · would save:{' '}
                    {formatBytes(applyResult.total_saved_bytes)}
                  </strong>
                  <p className="muted small">
                    Bytes computed but not written to disk. Disable preview
                    mode + ensure a backup is loaded to commit.
                  </p>
                  {skipHistogram.length > 0 && (
                    <details>
                      <summary>
                        skip reasons ({skipHistogram.length} distinct)
                      </summary>
                      <ul className="reasons">
                        {skipHistogram.map((s) => (
                          <li key={s.reason}>
                            <span className="muted small">
                              {s.count.toLocaleString()} ×
                            </span>{' '}
                            {s.reason}
                          </li>
                        ))}
                      </ul>
                    </details>
                  )}
                </div>
              )}
            </div>
          )}

          {aiPlan && aiPlan.executor_notes.length > 0 && (
            <details className="mipstrip-diag">
              <summary>v0.7 AI restore executor — pending notes</summary>
              <ul className="reasons">
                {aiPlan.executor_notes.map((n) => (
                  <li key={n}>{n}</li>
                ))}
              </ul>
            </details>
          )}

          {plan.class_histogram && plan.class_histogram.length > 0 && (
            <details className="mipstrip-diag">
              <summary>
                diagnostic · export classes seen ({plan.class_histogram.length})
              </summary>
              <ul className="reasons">
                {plan.class_histogram.map((c) => (
                  <li key={c.class_name}>
                    <span className="path-small">{c.class_name}</span>
                    <span className="muted small"> · {c.count.toLocaleString()}</span>
                  </li>
                ))}
              </ul>
              <p className="muted small">
                If "Texture2D" / "TextureCube" appears here but `textures: 0`
                above, CUE4Parse loaded them as untyped UObject — typed cast
                needs another engine version or a derived class match.
              </p>
            </details>
          )}
        </>
      )}

      {!pak && !error && !loading && (
        <p className="placeholder inspector-empty-hint">
          Drop in a readable UE4 .pak (UE5 IoStore paks need retoc, coming in
          a later Phase 2 step). Pamali / asset flips / dev builds are good
          test targets.
        </p>
      )}
    </section>
  )
}

````

## `src/DeltaCodecPanel.tsx`

````tsx
import { useState, type CSSProperties } from 'react'
import { invoke } from '@tauri-apps/api/core'
import { open } from '@tauri-apps/plugin-dialog'

type Row = {
  sample: string
  predictor: string
  quant_step: number
  top_mip_bytes: number
  low_mip_bytes: number
  residual_zst_bytes: number
  delta_total_bytes: number
  ratio: number
  max_channel_error: number
  byte_exact: boolean
}

type BenchResult = {
  rows: Row[]
  best_lossless_ratio: number
  lossless_runs: number
  total_runs: number
  spec_version: string
}

function fmtBytes(n: number): string {
  if (n < 1024) return `${n} B`
  if (n < 1024 * 1024) return `${(n / 1024).toFixed(1)} KiB`
  return `${(n / (1024 * 1024)).toFixed(2)} MiB`
}

export function DeltaCodecPanel() {
  const [result, setResult] = useState<BenchResult | null>(null)
  const [pending, setPending] = useState(false)
  const [err, setErr] = useState<string | null>(null)
  const [lastLabel, setLastLabel] = useState<string | null>(null)
  const [downsample, setDownsample] = useState<2 | 4>(2)

  async function runSynthetic() {
    setPending(true)
    setErr(null)
    try {
      const r = await invoke<BenchResult>('delta_codec_run_synthetic_bench', { downsample })
      setResult(r)
      setLastLabel('synthetic content classes')
    } catch (e: unknown) {
      setErr(String(e))
    } finally {
      setPending(false)
    }
  }

  async function runOnFile() {
    setErr(null)
    const picked = await open({
      multiple: false,
      directory: false,
      filters: [{ name: 'image', extensions: ['png', 'jpg', 'jpeg', 'webp', 'bmp'] }],
    })
    if (!picked || typeof picked !== 'string') return
    setPending(true)
    try {
      const r = await invoke<BenchResult>('delta_codec_run_file_bench', { path: picked, downsample })
      setResult(r)
      setLastLabel(picked.split('/').pop() ?? picked)
    } catch (e: unknown) {
      setErr(String(e))
    } finally {
      setPending(false)
    }
  }

  const segBtn = (active: boolean): CSSProperties => ({
    padding: '0.3rem 0.6rem',
    fontSize: '0.8rem',
    cursor: pending ? 'default' : 'pointer',
    border: '1px solid rgba(255,255,255,0.18)',
    background: active ? 'rgba(120,180,255,0.22)' : 'transparent',
    color: active ? '#cfe' : 'inherit',
    fontWeight: active ? 600 : 400,
  })

  return (
    <section className="report">
      <h2>Δ-Codec — Byte-Exact AI Compression</h2>
      <p className="muted small">
        A new bitstream that ships both an AI-predicted high mip AND a compressed residual.
        Restore = predict + apply residual. Byte-exact verified via SHA-256 of the reconstructed
        RGBA. Industry says you pick one of "lossy-small" or "byte-exact." We're testing whether
        you can have both.
      </p>
      <div
        className="actions"
        style={{ display: 'flex', gap: '0.6rem', marginTop: '0.6rem', alignItems: 'center', flexWrap: 'wrap' }}
      >
        <button onClick={runSynthetic} disabled={pending}>
          {pending ? 'running…' : 'run synthetic bench'}
        </button>
        <button onClick={runOnFile} disabled={pending}>
          run on image…
        </button>
        <div
          role="group"
          aria-label="downsample factor"
          style={{ display: 'inline-flex', marginLeft: 'auto', borderRadius: 3, overflow: 'hidden' }}
        >
          <button style={segBtn(downsample === 2)} disabled={pending} onClick={() => setDownsample(2)}>
            2× · bilinear
          </button>
          <button style={segBtn(downsample === 4)} disabled={pending} onClick={() => setDownsample(4)}>
            4× · bilinear + ESRGAN
          </button>
        </div>
      </div>
      <p className="muted small" style={{ marginTop: '0.4rem' }}>
        Downsample sets how hard the low mip is shrunk before prediction. <b>4×</b> is the realistic
        shrinkray strip and the only ratio Real-ESRGAN can upscale, so it runs bilinear and ESRGAN
        paired (image bench only) — ESRGAN inference makes that run slower.
      </p>

      {err && (
        <p className="err small" style={{ marginTop: '0.6rem' }}>
          {err}
        </p>
      )}

      {result && (
        <div style={{ marginTop: '0.9rem' }}>
          {lastLabel && (
            <p className="muted small" style={{ marginBottom: '0.4rem' }}>
              Sample: <span style={{ color: '#cfe' }}>{lastLabel}</span> · spec {result.spec_version}
            </p>
          )}
          <div style={{ overflowX: 'auto' }}>
            <table className="delta-codec-table">
              <thead>
                <tr>
                  <th>sample</th>
                  <th>predictor</th>
                  <th>q</th>
                  <th>top mip</th>
                  <th>low mip</th>
                  <th>residual</th>
                  <th>Δ total</th>
                  <th>ratio</th>
                  <th>max err</th>
                  <th>byte-exact</th>
                </tr>
              </thead>
              <tbody>
                {result.rows.map((r, i) => (
                  <tr key={i} className={r.quant_step === 1 ? 'lossless-row' : 'lossy-row'}>
                    <td>{r.sample}</td>
                    <td style={{ color: r.predictor === 'esrgan' ? '#d8b4fe' : '#9fc4e8' }}>
                      {r.predictor}
                    </td>
                    <td style={{ textAlign: 'right' }}>{r.quant_step}</td>
                    <td style={{ textAlign: 'right' }}>{fmtBytes(r.top_mip_bytes)}</td>
                    <td style={{ textAlign: 'right' }}>{fmtBytes(r.low_mip_bytes)}</td>
                    <td style={{ textAlign: 'right' }}>{fmtBytes(r.residual_zst_bytes)}</td>
                    <td style={{ textAlign: 'right' }}>{fmtBytes(r.delta_total_bytes)}</td>
                    <td
                      style={{
                        textAlign: 'right',
                        color: r.ratio < 1.0 ? '#9efc8c' : '#ffb14e',
                        fontWeight: 600,
                      }}
                    >
                      {r.ratio.toFixed(2)}×
                    </td>
                    <td style={{ textAlign: 'right' }}>{r.max_channel_error}</td>
                    <td
                      style={{
                        textAlign: 'center',
                        color: r.byte_exact ? '#9efc8c' : '#888',
                      }}
                    >
                      {r.byte_exact ? 'YES' : '—'}
                    </td>
                  </tr>
                ))}
              </tbody>
            </table>
          </div>
          <p className="muted small" style={{ marginTop: '0.6rem' }}>
            {result.lossless_runs} byte-exact run(s) · best byte-exact ratio{' '}
            <span style={{ color: '#9efc8c', fontWeight: 600 }}>
              {result.best_lossless_ratio.toFixed(2)}×
            </span>{' '}
            of the ExactBackup baseline (the q=1 oracle — smallest across predictors). q&gt;1 rows
            trade away byte-exactness and don&apos;t count as wins. Note: this ratio includes the
            low mip; in the strip workflow that mip already lives in the pak, so the true marginal
            cost of reversibility is the <b>residual</b> column alone.
          </p>
        </div>
      )}
    </section>
  )
}

````

## `src/styles.css`

````css
/* shrinkray — sits on top of 7.css. We only add what 7.css doesn't already do:
 *  - Aero title-bar subtitle + draggable region
 *  - Shrinkray-specific severity / score / inspector chroma
 *  - Wizard-intro band + mode badge
 *  - Folder picker styling
 */

:root {
  --sr-orange: #c96a18;
  --sr-orange-soft: #fdf1e3;
  --sr-orange-border: #e0a25c;
  --sr-info-soft: #e8f1fc;
  --sr-info-border: #95bce0;
  --sr-warn: #b75f00;
  --sr-warn-soft: #fff4e0;
  --sr-warn-border: #d4ab63;
  --sr-err: #c00000;
  --sr-err-soft: #fbecec;
  --sr-err-border: #d9a8a8;
  --sr-ok: #2a6e2a;
  --sr-ok-soft: #e7f3e0;
  --sr-ok-border: #95b29a;
  --sr-accent-soft: #d6e6f7;
}

* { box-sizing: border-box; }
html, body, #root {
  margin: 0;
  padding: 0;
  height: 100%;
  font: 9pt 'Segoe UI', 'SegoeUI', 'Noto Sans', sans-serif;
  color: #000;
}

/* Aero-style wallpaper — original 1920×1200 JPEG generated locally via
 * ImageMagick (deep blue base + soft bokeh blooms + sheen + noise). Layered
 * with a faint diagonal sheen + grain so the backdrop-filter glass blur has
 * meaningful texture to bite into. */
body {
  background:
    /* Diagonal sheen — gives the glass a moving-light feel */
    linear-gradient(112deg,
      rgba(255,255,255,0.10) 0%,
      rgba(255,255,255,0.0) 25%,
      rgba(255,255,255,0.14) 45%,
      rgba(255,255,255,0.0) 65%) fixed,
    url('./assets/wallpaper.jpg') center center / cover fixed,
    #133f78;
}

/* The whole app sits inside the .window shell which 7.css decorates with the
 * Aero gradient + drop shadow. We make it fill the OS window. */
.app-shell {
  position: fixed;
  inset: 0;
  display: flex;
  flex-direction: column;
  border-radius: 0;  /* fills entire OS window — no rounded corner gap */
}
.app-shell::before { border-radius: 0; }
.app-shell > .title-bar {
  flex: 0 0 auto;
  cursor: default;
  user-select: none;
}
.app-shell > .window-body {
  flex: 1 1 auto;
  overflow: auto;
  margin: 6px 6px 6px 6px;
}

.title-bar-subtitle {
  font-weight: 400;
  font-size: 0.85em;
  opacity: 0.85;
  margin-left: 0.4em;
}

.layout {
  display: flex;
  flex-direction: column;
  gap: 12px;
  padding: 12px 14px 20px;
}

/* Wizard-intro band right under the title bar. */
.wizard-intro {
  display: flex;
  align-items: center;
  justify-content: space-between;
  gap: 1rem;
  padding: 8px 12px;
  background: linear-gradient(180deg, #ffffff 0%, #eef3fa 100%);
  border: 1px solid #c5d3e6;
  border-radius: 3px;
  box-shadow: inset 0 1px 0 rgba(255,255,255,0.9);
}
.wizard-intro-blurb {
  margin: 0;
  font-size: 0.95em;
  color: #1a3a6b;
}

/* Mode badge — Aero status pill. */
.mode-badge {
  padding: 2px 10px;
  border-radius: 9px;
  font-size: 0.78em;
  font-weight: 600;
  letter-spacing: 0;
  text-transform: none;
  box-shadow: inset 0 1px 0 rgba(255,255,255,0.7);
  white-space: nowrap;
}
.mode-badge.mode-preview {
  background: var(--sr-ok-soft);
  color: var(--sr-ok);
  border: 1px solid var(--sr-ok-border);
}
.mode-badge.mode-write {
  background: var(--sr-err-soft);
  color: var(--sr-err);
  border: 1px solid var(--sr-err-border);
}

/* Folder picker / drop zone. */
.drop {
  background: #fff;
  border: 1px solid #cdd7db;
  border-radius: 3px;
  padding: 10px 12px;
  display: flex;
  flex-direction: column;
  gap: 8px;
  box-shadow: inset 0 0 0 1px #fff, 0 1px 2px rgba(0,0,0,0.05);
}
.placeholder { color: #585858; font-style: italic; margin: 0; }
.path {
  font-family: 'Consolas', 'Liberation Mono', monospace;
  font-size: 0.85em;
  color: #1a1a1a;
  background: #f6f7f8;
  border: 1px solid #d8d8d8;
  border-radius: 2px;
  padding: 4px 7px;
  box-shadow: inset 0 1px 0 rgba(0,0,0,0.04);
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
  margin: 0;
}
.actions { display: flex; gap: 6px; flex-wrap: wrap; }
.preview-toggle {
  display: inline-flex;
  align-items: center;
  gap: 6px;
  cursor: pointer;
  user-select: none;
  font-size: 0.92em;
  padding-bottom: 6px;
  border-bottom: 1px dotted #d8d8d8;
}

/* Group-box .report sections — match 7.css fieldset look but accept multiple
 * h2s inside (existing markup has subsection h2s within one card). */
.report {
  background: #fff;
  border: 1px solid #cdd7db;
  border-radius: 3px;
  padding: 12px 14px;
  box-shadow: inset 0 0 0 1px #fff, 0 1px 2px rgba(0,0,0,0.04);
  margin: 0;
}
.report h2 {
  font-size: 1em;
  font-weight: 600;
  letter-spacing: 0;
  text-transform: none;
  color: #1a3a6b;
  padding-bottom: 4px;
  margin: 0 0 8px;
  border-bottom: 1px solid #d8e1ec;
}
.report > h2:first-child { margin-top: 0; }
.report table {
  width: 100%;
  border-collapse: collapse;
  font-size: 0.92em;
}
.report td {
  padding: 4px 0;
  border-bottom: 1px dotted #d8d8d8;
}
.report td:first-child { color: #585858; }
.report td:last-child { text-align: right; font-variant-numeric: tabular-nums; }
.report td.accent { color: #1b6fd4; font-weight: 600; }

.err {
  color: var(--sr-err);
  font-size: 0.92em;
  background: var(--sr-err-soft);
  border: 1px solid var(--sr-err-border);
  border-radius: 3px;
  padding: 6px 8px;
  white-space: pre-wrap;
  margin: 0 0 8px;
}

/* Top-files table — minor tightening. */
.report table.top td { padding: 3px 4px; vertical-align: top; }
.report table.top td.kind {
  color: #585858;
  font-size: 0.85em;
  width: 4.5rem;
  letter-spacing: 0;
  text-transform: none;
}
.report table.top td.path-small,
.path-small {
  font-family: 'Consolas', 'Liberation Mono', monospace;
  font-size: 0.88em;
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
  max-width: 100%;
}
.report table.top td.size {
  text-align: right;
  font-variant-numeric: tabular-nums;
  color: #155bb0;
  font-weight: 600;
  white-space: nowrap;
  width: 6rem;
}

.reasons {
  list-style: none;
  margin: 4px 0 0;
  padding-left: 6px;
  border-left: 1px solid #d8d8d8;
}
.reasons li { padding: 3px 0 3px 7px; font-size: 0.88em; }

.muted { color: #585858; font-size: 0.92em; }
.small { font-size: 0.85em; }

details summary { cursor: pointer; user-select: none; }

/* Lang drop checkboxes. */
.lang-row { display: inline-flex; align-items: center; gap: 6px; cursor: pointer; user-select: none; }
.lang-row input[type='checkbox'] { accent-color: #1b6fd4; }
tr.dropping td { color: #1a1a1a; }

/* ---- Bloat audit ----------------------------------------------------- */

.audit-card { border-color: #9bb5d8; }
.audit-card h2 { color: #1b6fd4; border-bottom-color: #cad9eb; }
.audit-header {
  display: flex;
  align-items: baseline;
  justify-content: space-between;
  margin-bottom: 4px;
}
.audit-h3 {
  font-size: 0.95em;
  font-weight: 600;
  letter-spacing: 0;
  text-transform: none;
  color: #2c2c2c;
  margin: 12px 0 6px;
  padding-bottom: 3px;
  border-bottom: 1px dotted #d8d8d8;
}

.score-row {
  display: flex;
  align-items: center;
  gap: 16px;
  padding: 8px 0;
  border-bottom: 1px dotted #d8d8d8;
}
.score-pill {
  display: inline-flex;
  align-items: baseline;
  gap: 6px;
  padding: 7px 12px;
  border-radius: 4px;
  border: 1px solid #b8b8b8;
  background: #f7f9fc;
  box-shadow: inset 0 1px 0 rgba(255,255,255,0.7), 0 1px 0 rgba(0,0,0,0.03);
}
.score-pill .score-value { font-size: 1.9em; font-weight: 700; line-height: 1; }
.score-pill .score-out { color: #585858; font-size: 0.9em; }
.score-pill .score-label {
  margin-left: 8px;
  font-size: 0.95em;
  font-weight: 600;
  letter-spacing: 0;
  text-transform: none;
}
.score-clean      { color: var(--sr-ok);   border-color: var(--sr-ok-border);   background: var(--sr-ok-soft); }
.score-mild       { color: #836b00;        border-color: #c9b66a;               background: #fbf3cb; }
.score-structural { color: var(--sr-warn); border-color: var(--sr-warn-border); background: var(--sr-warn-soft); }
.score-severe     { color: var(--sr-err);  border-color: var(--sr-err-border);  background: var(--sr-err-soft); }

.score-side { display: flex; gap: 16px; flex-direction: column; font-size: 0.92em; }
.score-side > div { display: flex; gap: 6px; align-items: baseline; }

.finding-group { margin-top: 12px; }
.finding {
  background: #f7f9fc;
  border: 1px solid #d8d8d8;
  border-radius: 3px;
  padding: 8px 10px;
  margin-bottom: 6px;
  box-shadow: inset 0 1px 0 rgba(255,255,255,0.7);
}
.finding-info     { background: var(--sr-info-soft); border-color: #b8d2ee; }
.finding-warning  { background: var(--sr-warn-soft); border-color: #e1c79a; }
.finding-critical { background: var(--sr-err-soft);  border-color: var(--sr-err-border); }

.finding-head { display: flex; align-items: center; gap: 6px; margin-bottom: 5px; }
.finding-title { font-size: 0.98em; font-weight: 600; flex: 1; color: #1a1a1a; }

.sev-pill {
  padding: 1px 6px;
  border-radius: 2px;
  font-size: 0.8em;
  letter-spacing: 0;
  text-transform: none;
  font-weight: 600;
  box-shadow: inset 0 1px 0 rgba(255,255,255,0.6);
}
.sev-info     { background: var(--sr-accent-soft); color: #155bb0;        border: 1px solid var(--sr-info-border); }
.sev-warning  { background: #f7e7c4;               color: var(--sr-warn); border: 1px solid var(--sr-warn-border); }
.sev-critical { background: #f5cccc;               color: var(--sr-err);  border: 1px solid #cf8b8b; }

.finding-summary { font-size: 0.92em; line-height: 1.55; color: #1a1a1a; margin: 0 0 5px; }
.finding-meta { font-size: 0.9em; margin: 0 0 5px; }
.finding-rec {
  font-size: 0.88em;
  line-height: 1.55;
  color: #2c2c2c;
  margin-top: 6px;
  padding-top: 5px;
  border-top: 1px dotted rgba(0,0,0,0.12);
}
.finding-rec .muted { color: #585858; }

/* Backup — green-tinted because it's safety. */
.backup-card { border-color: var(--sr-ok-border); }
.backup-card h2 { color: var(--sr-ok); border-bottom-color: #c5dac9; }

/* Plan card — orange confirmation. */
.plan-card {
  margin-top: 12px;
  padding: 10px 12px;
  border: 1px solid var(--sr-orange-border);
  border-radius: 3px;
  background: var(--sr-orange-soft);
  box-shadow: inset 0 1px 0 rgba(255,255,255,0.6);
}
.plan-card h2 { margin-top: 0 !important; color: var(--sr-orange); border-bottom-color: #f1d4a8; }

/* ---- Asset Inspector ------------------------------------------------- */

.inspector-card { border-color: var(--sr-orange-border); }
.inspector-card h2 { color: var(--sr-orange); border-bottom-color: #f1d4a8; }
.inspector-card--empty {
  padding: 18px 16px 22px;
  background: linear-gradient(180deg, var(--sr-orange-soft) 0%, #ffffff 70%);
}
.inspector-head {
  display: flex;
  align-items: flex-start;
  justify-content: space-between;
  gap: 14px;
  margin-bottom: 6px;
}
.inspector-card--empty .inspector-head { margin-bottom: 0; }
.inspector-card--empty .inspector-head > div { max-width: 60ch; }
.inspector-head h2 { margin-bottom: 5px; padding-bottom: 4px; }

.preview-tag {
  display: inline-block;
  margin-left: 7px;
  padding: 1px 6px;
  background: #f7e7c4;
  color: var(--sr-orange);
  border: 1px solid #d4ab63;
  border-radius: 2px;
  font-size: 0.75em;
  letter-spacing: 0;
  text-transform: none;
  font-weight: 600;
  vertical-align: middle;
  box-shadow: inset 0 1px 0 rgba(255,255,255,0.5);
}
.inspector-blurb { max-width: 60ch; line-height: 1.55; margin: 0; }
.inspector-empty-hint {
  margin: 14px 0 0;
  padding-top: 11px;
  border-top: 1px dotted #d8d8d8;
  font-size: 0.92em;
  line-height: 1.55;
  max-width: 60ch;
  color: #585858;
}

.inspector-meta { width: 100%; border-collapse: collapse; font-size: 0.92em; margin-top: 4px; }
.inspector-meta th {
  text-align: left;
  color: #585858;
  font-weight: 400;
  letter-spacing: 0;
  padding: 4px 11px 4px 0;
  width: 10rem;
  vertical-align: top;
  border-bottom: 1px dotted #d8d8d8;
}
.inspector-meta td {
  padding: 4px 0;
  border-bottom: 1px dotted #d8d8d8;
  text-align: left !important;
}

.badge-ok {
  display: inline-block;
  padding: 1px 7px;
  background: var(--sr-ok-soft);
  color: var(--sr-ok);
  border: 1px solid var(--sr-ok-border);
  border-radius: 2px;
  font-size: 0.85em;
  letter-spacing: 0;
  text-transform: none;
  font-weight: 600;
}
.badge-warn {
  display: inline-block;
  padding: 1px 7px;
  background: var(--sr-warn-soft);
  color: var(--sr-warn);
  border: 1px solid var(--sr-warn-border);
  border-radius: 2px;
  font-size: 0.85em;
  letter-spacing: 0;
  text-transform: none;
  font-weight: 600;
}

.inspector-filters {
  display: flex;
  flex-wrap: wrap;
  align-items: center;
  gap: 5px;
  margin: 10px 0 6px;
}
/* .chip uses 7.css's native <button> styling — we only flag the active state. */
.chip { min-width: 0 !important; font-size: 0.86em !important; padding: 2px 8px !important; }
.chip-on {
  background: linear-gradient(180deg, #5995d8 0%, #2870c5 100%) !important;
  color: #fff !important;
  border-color: #155bb0 !important;
  text-shadow: 0 -1px 0 rgba(0,0,0,0.25) !important;
}
.inspector-search {
  margin-left: auto;
  background: #fff;
  color: #1a1a1a;
  border: 1px solid #8b8b8b;
  border-radius: 2px;
  padding: 3px 7px;
  font-family: inherit;
  font-size: 0.92em;
  min-width: 20ch;
  box-shadow: inset 0 1px 0 rgba(0,0,0,0.04);
}
.inspector-search:focus {
  outline: none;
  border-color: #1b6fd4;
  box-shadow: inset 0 1px 0 rgba(0,0,0,0.04), 0 0 0 2px rgba(27,111,212,0.2);
}

.inspector-table {
  width: 100%;
  border-collapse: collapse;
  font-size: 0.92em;
  margin-top: 4px;
}
.inspector-table thead th {
  text-align: left;
  color: #2c2c2c;
  font-weight: 600;
  font-size: 0.88em;
  letter-spacing: 0;
  text-transform: none;
  padding: 5px 8px 5px 0;
  background: linear-gradient(180deg, #f3f6fa 0%, #e6eaef 100%);
  border-top: 1px solid #b8b8b8;
  border-bottom: 1px solid #b8b8b8;
}
.inspector-table thead th:first-child { padding-left: 7px; }
.inspector-table thead th.size-col { text-align: right; padding-right: 7px; }
.inspector-table tbody tr:nth-child(even) td { background: #f7f9fc; }
.inspector-table tbody tr:hover td { background: #e8f0fb; }
.inspector-table td {
  padding: 4px 8px 4px 0;
  border-bottom: 1px dotted #d8d8d8;
}
.inspector-table td:first-child { padding-left: 7px; }
.inspector-table td.kind { color: #585858; font-size: 0.88em; letter-spacing: 0; text-transform: none; }
.inspector-table td.size { text-align: right; font-variant-numeric: tabular-nums; padding-right: 7px; }
.inspector-row-on td {
  background: var(--sr-accent-soft) !important;
  box-shadow: inset 2px 0 0 #1b6fd4;
}

.inspector-detail {
  margin-top: 12px;
  padding: 11px 13px;
  background: var(--sr-orange-soft);
  border: 1px solid var(--sr-orange-border);
  border-radius: 3px;
  box-shadow: inset 0 1px 0 rgba(255,255,255,0.6);
}
.inspector-detail-head {
  display: flex;
  align-items: center;
  justify-content: space-between;
  gap: 11px;
  margin-bottom: 5px;
}
.inspector-detail-head h3 {
  font-size: 0.98em;
  font-weight: 600;
  letter-spacing: 0;
  text-transform: none;
  color: var(--sr-orange);
  margin: 0;
}
.inspector-detail-path { margin: 0 0 8px; color: #1a1a1a; word-break: break-all; }
.inspector-detail-sub {
  margin: 11px 0 5px;
  font-size: 0.92em;
  font-weight: 600;
  letter-spacing: 0;
  text-transform: none;
  color: #2c2c2c;
  border-bottom: 1px dotted rgba(0,0,0,0.12);
  padding-bottom: 3px;
}

.texture-block { margin-top: 10px; }
.mip-table { font-size: 0.85em; max-width: 36rem; }
.mip-table th, .mip-table td { padding: 2px 8px 2px 0; }
.mip-table th:first-child, .mip-table td:first-child { width: 3rem; padding-left: 6px; }

/* Mip-strip panel — Phase 2 read-side. */
.mipstrip-card { border-color: var(--sr-orange-border); }
.mipstrip-card h2 { color: var(--sr-orange); border-bottom-color: #f1d4a8; }
.mipstrip-controls {
  display: flex;
  align-items: center;
  gap: 8px;
  margin: 12px 0 10px;
  padding: 6px 8px;
  background: var(--sr-orange-soft);
  border: 1px solid #f1d4a8;
  border-radius: 3px;
}
.mipstrip-items {
  margin-top: 8px;
}
.mipstrip-items th, .mipstrip-items td {
  padding: 3px 8px 3px 0;
}
.mipstrip-items td.size { color: var(--sr-orange); font-weight: 600; }
.mipstrip-formats {
  margin-top: 12px;
  padding: 8px 10px;
  background: #f8f9fb;
  border: 1px solid #d8dde3;
  border-radius: 3px;
}
.mipstrip-formats h4 {
  margin: 0 0 6px;
  font-size: 0.88em;
  font-weight: 600;
  color: #1a3a6b;
}
.mipstrip-formats table { max-width: 36rem; }
.mipstrip-formats td.size { color: var(--sr-orange); font-weight: 600; }

.mipstrip-apply { margin-top: 14px; }
.mipstrip-apply-note {
  margin-top: 10px;
  padding: 10px 12px;
  background: var(--sr-warn-soft);
  border: 1px solid var(--sr-warn-border);
  border-radius: 3px;
}
.mipstrip-apply-note strong {
  display: block;
  color: var(--sr-warn);
  margin-bottom: 4px;
}
.mipstrip-apply-note p { margin: 0; }
.mipstrip-apply-note ul { margin-top: 6px; padding-left: 8px; }
.mipstrip-apply-note details { margin-top: 8px; }
.mipstrip-apply-note summary { cursor: pointer; }

/* v0.7.2 live apply progress. One row per texture finished arrives via the
 * `strip-progress` Tauri event from the sidecar's ProgressEmitter; the bar
 * + filename + elapsed/ETA line live below the apply button until the run
 * settles and the result-summary block takes over. */
.mipstrip-progress {
  margin-top: 8px;
  display: flex;
  flex-direction: column;
  gap: 4px;
}
.mipstrip-progress-bar {
  width: 100%;
  height: 6px;
  background: rgba(0, 0, 0, 0.08);
  border: 1px solid var(--sr-border, rgba(0, 0, 0, 0.15));
  border-radius: 3px;
  overflow: hidden;
}
.mipstrip-progress-fill {
  height: 100%;
  background: var(--sr-accent, #4a7c8a);
  transition: width 120ms linear;
}
.mipstrip-progress-meta {
  display: flex;
  justify-content: space-between;
  gap: 12px;
  white-space: nowrap;
  overflow: hidden;
}
.mipstrip-progress-meta > span:first-child {
  overflow: hidden;
  text-overflow: ellipsis;
  flex: 1 1 auto;
}
.mipstrip-progress-meta > span:last-child {
  flex: 0 0 auto;
}

/* Restore class pills in the planner table — colour-codes which textures
 * become AI-restorable / backup-required / skipped per the classifier. */
.restore-ai      { color: #1f6f3b; font-weight: 600; }
.restore-backup  { color: #6f4d1f; font-weight: 600; }
.restore-skip    { color: #6f1f1f; font-weight: 600; }
.mipstrip-game {
  font-family: inherit;
  font-size: 0.9em;
  padding: 2px 6px;
  border: 1px solid #8b8b8b;
  border-radius: 2px;
  background: #fff;
  color: #1a1a1a;
}
.mipstrip-game:focus {
  outline: none;
  border-color: #1b6fd4;
  box-shadow: 0 0 0 2px rgba(27,111,212,0.2);
}

/* ---- Win7 Open dialog ----------------------------------------------- */

.open-dialog-backdrop {
  position: fixed;
  inset: 0;
  background: rgba(20, 30, 50, 0.45);
  display: flex;
  align-items: center;
  justify-content: center;
  z-index: 1000;
}
/* 7.css hides .window[role=dialog] by default — undo so our React-controlled
 * mount is what decides visibility. */
.open-dialog.window[role='dialog'] {
  position: static;
  left: auto;
  top: auto;
  transform: none;
  opacity: 1;
  visibility: visible;
  z-index: auto;
  transition: none;
}
.open-dialog {
  width: 780px;
  max-width: 92vw;
  max-height: 86vh;
  display: flex;
  flex-direction: column;
}
.open-dialog > .window-body {
  display: flex;
  flex-direction: column;
  padding: 8px;
  gap: 6px;
  flex: 1 1 auto;
  min-height: 0;
}

.od-pathbar {
  display: flex;
  align-items: center;
  gap: 6px;
  padding: 4px;
  background: linear-gradient(180deg, #f3f6fa 0%, #e6eaef 100%);
  border: 1px solid #c3cdd6;
  border-radius: 3px;
}
.od-up {
  min-width: 0 !important;
  padding: 2px 8px !important;
  font-weight: 700;
}
.od-breadcrumbs {
  flex: 1 1 auto;
  display: flex;
  align-items: center;
  gap: 2px;
  font-size: 0.9em;
  overflow: hidden;
  white-space: nowrap;
}
.od-crumb { display: inline-flex; align-items: center; gap: 2px; }
.od-crumb button {
  min-width: 0 !important;
  padding: 2px 6px !important;
  background: transparent !important;
  border: 1px solid transparent !important;
  box-shadow: none !important;
  font-size: 1em !important;
  color: #1a3a6b !important;
}
.od-crumb button:hover {
  background: #eaf3fb !important;
  border-color: #cad9eb !important;
}
.od-crumb-sep { color: #888; padding: 0 2px; }
.od-search {
  min-width: 22ch;
  font-family: inherit;
  font-size: 0.9em;
  padding: 3px 6px;
  border: 1px solid #8b8b8b;
  border-radius: 2px;
  background: #fff;
  box-shadow: inset 0 1px 0 rgba(0,0,0,0.04);
}
.od-search:focus {
  outline: none;
  border-color: #1b6fd4;
  box-shadow: inset 0 1px 0 rgba(0,0,0,0.04), 0 0 0 2px rgba(27,111,212,0.2);
}

.od-body {
  display: flex;
  gap: 6px;
  flex: 1 1 auto;
  min-height: 280px;
}

.od-sidebar {
  width: 170px;
  flex-shrink: 0;
  background: #fff;
  border: 1px solid #c3cdd6;
  border-radius: 3px;
  padding: 6px;
  overflow-y: auto;
}
.od-sidebar-group { margin-bottom: 8px; }
.od-sidebar-group h4 {
  font-size: 0.82em;
  font-weight: 600;
  color: #1a3a6b;
  text-transform: none;
  letter-spacing: 0;
  margin: 4px 0 4px;
  padding-bottom: 2px;
  border-bottom: 1px dotted #d8e1ec;
}
.od-sidebar-group ul { list-style: none; margin: 0; padding: 0; }
.od-sidebar-group li { margin: 1px 0; }
.od-sidebar-group li button {
  width: 100%;
  text-align: left;
  background: transparent !important;
  border: 1px solid transparent !important;
  box-shadow: none !important;
  min-width: 0 !important;
  padding: 3px 6px !important;
  font-size: 0.9em !important;
  color: #1a1a1a !important;
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}
.od-sidebar-group li button:hover {
  background: #eaf3fb !important;
  border-color: #cad9eb !important;
}

.od-list {
  flex: 1 1 auto;
  background: #fff;
  border: 1px solid #c3cdd6;
  border-radius: 3px;
  overflow-y: auto;
  min-height: 0;
}

.od-table {
  width: 100%;
  border-collapse: collapse;
  font-size: 0.9em;
}
.od-table thead {
  position: sticky;
  top: 0;
  z-index: 1;
}
.od-table thead th {
  text-align: left;
  background: linear-gradient(180deg, #f3f6fa 0%, #e6eaef 100%);
  border-bottom: 1px solid #c3cdd6;
  padding: 5px 8px;
  font-weight: 600;
  color: #1a3a6b;
}
.od-table tbody td {
  padding: 4px 8px;
  border-bottom: 1px dotted #e3e7ec;
  cursor: default;
  user-select: none;
}
.od-table tbody tr:hover td { background: #eaf3fb; }
.od-table tbody tr.od-row-on td {
  background: #cce4f7;
  color: #000;
}
.od-col-name {
  display: flex;
  align-items: center;
  gap: 6px;
}
.od-table .od-col-modified { width: 12rem; color: #585858; font-size: 0.9em; }
.od-table .od-col-size {
  width: 5rem;
  text-align: right;
  font-variant-numeric: tabular-nums;
}
.od-icon { font-size: 1.05em; }
.od-name {
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}
.od-empty, .od-error {
  padding: 16px;
  font-size: 0.92em;
  text-align: center;
}
.od-empty { color: #585858; font-style: italic; }
.od-error { color: var(--sr-err); }

.od-footer {
  display: flex;
  flex-direction: column;
  gap: 4px;
  padding-top: 6px;
  border-top: 1px solid #d8d8d8;
}
.od-filename-row {
  display: flex;
  align-items: center;
  gap: 8px;
  font-size: 0.92em;
}
.od-filename-row label {
  width: 7rem;
  color: #1a1a1a;
}
.od-filename {
  flex: 1 1 auto;
  font-family: inherit;
  font-size: 0.9em;
  padding: 3px 6px;
  border: 1px solid #8b8b8b;
  border-radius: 2px;
  background: #fff;
  box-shadow: inset 0 1px 0 rgba(0,0,0,0.04);
  color: #1a1a1a;
  overflow: hidden;
  text-overflow: ellipsis;
}
.od-buttons {
  display: flex;
  justify-content: flex-end;
  gap: 6px;
  margin-top: 6px;
}
.od-buttons button { min-width: 88px; }

````

