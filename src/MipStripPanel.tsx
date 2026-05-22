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
