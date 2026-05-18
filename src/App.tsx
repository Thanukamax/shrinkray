import { useEffect, useMemo, useState } from 'react'
import { invoke } from '@tauri-apps/api/core'
import { open } from '@tauri-apps/plugin-dialog'

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

export default function App() {
  const [path, setPath] = useState<string | null>(null)
  const [report, setReport] = useState<AnalysisReport | null>(null)
  const [backup, setBackup] = useState<BackupStatus | null>(null)
  const [restore, setRestore] = useState<RestoreReport | null>(null)
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
  const [previewOnly, setPreviewOnly] = useState<boolean>(() => loadPreviewOnly())
  const [error, setError] = useState<string | null>(null)

  useEffect(() => {
    persistPreviewOnly(previewOnly)
  }, [previewOnly])

  async function pickFolder() {
    setError(null)
    setRestore(null)
    setPlan(null)
    setStripReport(null)
    setRecompressPlan(null)
    setRecompressReport(null)
    setDropLangs(new Set())
    const sel = await open({ directory: true, multiple: false, title: 'Pick a game folder' })
    if (typeof sel === 'string') {
      setPath(sel)
      setReport(null)
      const [st, encs] = await Promise.all([
        invoke<BackupStatus | null>('backup_status', { path: sel }),
        invoke<EncoderAvailability[]>('detect_encoders'),
      ])
      setBackup(st)
      setEncoders(encs)
    }
  }

  async function analyze() {
    if (!path) return
    setPending(true)
    setError(null)
    try {
      const r = await invoke<AnalysisReport>('analyze_folder', { path })
      setReport(r)
      const st = await invoke<BackupStatus | null>('backup_status', { path })
      setBackup(st)
    } catch (e) {
      setError(String(e))
    } finally {
      setPending(false)
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
    <main className="layout">
      <header>
        <h1>shrinkray</h1>
        <span className="muted">UE game folder optimizer · v0.3.0</span>
        <span className={`mode-badge ${previewOnly ? 'mode-preview' : 'mode-write'}`}>
          {previewOnly ? 'preview' : 'WRITE'}
        </span>
      </header>

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
        </div>
        {error && <p className="err">{error}</p>}
      </section>

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

          {languages.length > 0 && (
            <>
              <h2 style={{ marginTop: '1.6rem' }}>
                Languages — pick which to drop ({languages.length} detected)
              </h2>
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
