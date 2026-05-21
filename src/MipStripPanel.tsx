import { useEffect, useMemo, useState } from 'react'
import { invoke } from '@tauri-apps/api/core'
import { OpenDialog } from './OpenDialog'

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

export function MipStripPanel() {
  const [pak, setPak] = useState<string | null>(null)
  const [maxDim, setMaxDim] = useState<number>(2048)
  const [game, setGame] = useState<string>('GAME_UE5_LATEST')
  const [dialogOpen, setDialogOpen] = useState(false)
  const [plan, setPlan] = useState<PlanStripMipsResult | null>(null)
  const [aiPlan, setAiPlan] = useState<RestoreAiPlan | null>(null)
  const [loading, setLoading] = useState(false)
  const [error, setError] = useState<string | null>(null)
  const [applyResult, setApplyResult] = useState<ApplyStripMipsResult | null>(null)
  const [applying, setApplying] = useState(false)
  const [exemptNormals, setExemptNormals] = useState(true)

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
    setApplying(true)
    setError(null)
    try {
      // Filter: drop NoStrip-class textures (data textures / lookups) and,
      // when exemptNormals is on, drop ExactBackup-class too (normals etc).
      // Leave that filtering to v0.6.0 final — for rc1 we send all items and
      // let the sidecar's parser decide what it can handle. The skip-reason
      // diagnostic surfaces both classifier-driven skips and parser-stage skips.
      const targets = plan.items
        .filter((it) => {
          const c = classByAsset.get(it.asset_path)
          if (c === 'no_strip') return false
          if (exemptNormals && c === 'exact_backup') return false
          return true
        })
        .map((it) => ({ asset_path: it.asset_path, max_dim: maxDim }))

      const r = await invoke<ApplyStripMipsResult>('sidecar_apply_strip_mips', {
        pakPath: pak,
        targets,
        game,
        engineVersion: uassetapiVerFor(game),
      })
      setApplyResult(r)
    } catch (e) {
      setError(String(e))
    } finally {
      setApplying(false)
    }
  }

  async function runPlan(pakPath: string, dim: number, g: string) {
    setLoading(true)
    setError(null)
    setPlan(null)
    setAiPlan(null)
    setApplyResult(null)
    try {
      const r = await invoke<PlanStripMipsResult>('sidecar_plan_strip_mips', {
        pakPath,
        maxDim: dim,
        limit: 5000,
        game: g,
      })
      setPlan(r)
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
    }
  }

  function onPakChosen(p: string) {
    setDialogOpen(false)
    setPak(p)
    runPlan(p, maxDim, game)
  }

  function onDimChange(dim: number) {
    setMaxDim(dim)
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

  // Skip-reason histogram for the apply result. Multiple textures can share
  // the same reason ("inline payload mip", "asset not found", etc); collapse
  // them so the UI doesn't render hundreds of identical lines.
  const skipHistogram = useMemo(() => {
    if (!applyResult) return [] as { reason: string; count: number }[]
    const m = new Map<string, number>()
    for (const s of applyResult.skipped) {
      m.set(s.reason, (m.get(s.reason) ?? 0) + 1)
    }
    return Array.from(m.entries())
      .map(([reason, count]) => ({ reason, count }))
      .sort((a, b) => b.count - a.count)
  }, [applyResult])

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
            <span className="preview-tag">v0.6.0-rc1 · apply path in flight</span>
          </h2>
          <p className="muted small inspector-blurb">
            Pick a readable UE4 .pak and a maximum texture dimension. The plan
            walks every cooked texture, projects savings, and routes each
            texture through the classifier (AI re-expand vs exact backup vs
            skip). v0.6.0-rc1 wires the apply path end-to-end; per-mip parser
            for UE4.22 cooks is still being pinned down — skipped textures
            surface their reason inline.
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
                <td>{plan.texture_count.toLocaleString()}</td>
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
                {applying ? 'applying…' : 'apply (write to pak)'}
              </button>
              <p className="muted small">
                v0.6.0-rc1: applier framework is wired. Targets that hit the
                in-flight per-mip parser path will land in "skipped" with a
                diagnostic reason rather than corrupting the pak.
              </p>

              {applyResult && (
                <div className="mipstrip-apply-note">
                  <strong>
                    applied: {applyResult.applied.length} · skipped:{' '}
                    {applyResult.skipped.length} · saved:{' '}
                    {formatBytes(applyResult.total_saved_bytes)}
                  </strong>
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
                  {applyResult.applied.length === 0 && (
                    <p className="muted small">
                      Nothing applied yet. The framework is wired end-to-end
                      (extract → parse → splice → regen → write back); the
                      per-mip layout pin-down lands in v0.6.0 final. Original
                      pak is untouched.
                    </p>
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
