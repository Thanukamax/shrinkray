import { useMemo, useState } from 'react'
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
}

type ClassCount = { class_name: string; count: number }

type ApplyStripMipsStub = {
  implemented: boolean
  phase: string
  message: string
  backup_required: boolean
  requires: string[]
}

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

export function MipStripPanel() {
  const [pak, setPak] = useState<string | null>(null)
  const [maxDim, setMaxDim] = useState<number>(2048)
  const [game, setGame] = useState<string>('GAME_UE5_LATEST')
  const [dialogOpen, setDialogOpen] = useState(false)
  const [plan, setPlan] = useState<PlanStripMipsResult | null>(null)
  const [loading, setLoading] = useState(false)
  const [error, setError] = useState<string | null>(null)
  const [applyStub, setApplyStub] = useState<ApplyStripMipsStub | null>(null)

  async function tryApply() {
    if (!pak) return
    setApplyStub(null)
    try {
      const r = await invoke<ApplyStripMipsStub>('sidecar_apply_strip_mips', {
        pakPath: pak,
      })
      setApplyStub(r)
    } catch (e) {
      setError(String(e))
    }
  }

  async function runPlan(pakPath: string, dim: number, g: string) {
    setLoading(true)
    setError(null)
    setPlan(null)
    try {
      const r = await invoke<PlanStripMipsResult>('sidecar_plan_strip_mips', {
        pakPath,
        maxDim: dim,
        limit: 5000,
        game: g,
      })
      setPlan(r)
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

  // Aggregate items by pixel format so the user can see which formats
  // dominate the savings — useful for diagnosing whether a game is BC1-heavy
  // (UI / masks) vs BC5-heavy (normal maps) vs BC7 (modern albedos).
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
            Texture mip strip <span className="preview-tag">preview · Phase 2</span>
          </h2>
          <p className="muted small inspector-blurb">
            Pick a readable UE4 .pak and a maximum texture dimension. We walk
            every cooked texture inside, project the savings from capping mip 0
            to that dimension, and list the biggest wins. Read-only — the apply
            path lands once write-side serialization is verified on real games.
          </p>
        </div>
        <button onClick={() => setDialogOpen(true)} disabled={loading} className={!pak ? 'primary' : ''}>
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
                disabled={loading}
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
              disabled={loading}
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
        </>
      )}

      {error && <p className="err">{error}</p>}

      {plan && !error && (
        <>
          <table className="inspector-meta">
            <tbody>
              <tr>
                <th>pak</th>
                <td className="path-small" title={plan.pak_path}>{plan.pak_path}</td>
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
                  <th>mip 0</th>
                  <th>→ keep</th>
                  <th>drop</th>
                  <th className="size-col">save</th>
                </tr>
              </thead>
              <tbody>
                {plan.items.slice(0, 200).map((it, i) => (
                  <tr key={`${it.asset_path}-${i}`}>
                    <td className="path-small" title={it.asset_path}>
                      {it.export_name}
                    </td>
                    <td className="kind">{it.pixel_format}</td>
                    <td>{it.current_mip0_dim}px</td>
                    <td>{it.kept_mip0_dim}px</td>
                    <td>{it.drop_mip_count}</td>
                    <td className="size">{formatBytes(it.save_bytes)}</td>
                  </tr>
                ))}
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
              <button onClick={tryApply} disabled={loading}>
                apply (write to pak)
              </button>
              {applyStub && !applyStub.implemented && (
                <div className="mipstrip-apply-note">
                  <strong>{applyStub.phase} — write-side not yet implemented.</strong>
                  <p className="muted small">{applyStub.message}</p>
                  {applyStub.requires.length > 0 && (
                    <ul className="reasons">
                      {applyStub.requires.map((r) => (
                        <li key={r}>{r}</li>
                      ))}
                    </ul>
                  )}
                </div>
              )}
            </div>
          )}

          {plan.class_histogram && plan.class_histogram.length > 0 && (
            <details className="mipstrip-diag">
              <summary>diagnostic · export classes seen ({plan.class_histogram.length})</summary>
              <ul className="reasons">
                {plan.class_histogram.map((c) => (
                  <li key={c.class_name}>
                    <span className="path-small">{c.class_name}</span>
                    <span className="muted small"> · {c.count.toLocaleString()}</span>
                  </li>
                ))}
              </ul>
              <p className="muted small">
                If "Texture2D" / "TextureCube" appears here but `textures: 0` above,
                CUE4Parse loaded them as untyped UObject — typed cast needs another
                engine version or a derived class match.
              </p>
            </details>
          )}
        </>
      )}

      {!pak && !error && !loading && (
        <p className="placeholder inspector-empty-hint">
          Drop in a readable UE4 .pak (UE5 IoStore paks need retoc, coming in
          a later Phase 2 step). Pamali / asset flips / dev builds are good test
          targets.
        </p>
      )}
    </section>
  )
}
