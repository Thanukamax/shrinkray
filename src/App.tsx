import { useState } from 'react'
import { invoke } from '@tauri-apps/api/core'
import { open } from '@tauri-apps/plugin-dialog'

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

export default function App() {
  const [path, setPath] = useState<string | null>(null)
  const [report, setReport] = useState<AnalysisReport | null>(null)
  const [backup, setBackup] = useState<BackupStatus | null>(null)
  const [restore, setRestore] = useState<RestoreReport | null>(null)
  const [pending, setPending] = useState(false)
  const [restoring, setRestoring] = useState(false)
  const [error, setError] = useState<string | null>(null)

  async function pickFolder() {
    setError(null)
    setRestore(null)
    const sel = await open({ directory: true, multiple: false, title: 'Pick a game folder' })
    if (typeof sel === 'string') {
      setPath(sel)
      setReport(null)
      // Probe for an existing backup in parallel — read-only, safe.
      const st = await invoke<BackupStatus | null>('backup_status', { path: sel })
      setBackup(st)
    }
  }

  async function analyze() {
    if (!path) return
    setPending(true)
    setError(null)
    try {
      const r = await invoke<AnalysisReport>('analyze_folder', { path })
      setReport(r)
      // Re-probe in case the user created one out-of-band.
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
    } catch (e) {
      setError(String(e))
    } finally {
      setRestoring(false)
    }
  }

  const savedPct =
    report && report.total_size > 0
      ? Math.round((report.estimated_l10n_savings / report.total_size) * 100)
      : 0

  const languages = report ? Object.entries(report.languages).sort((a, b) => b[1].size - a[1].size) : []
  const largestLang = languages[0]?.[0]
  const inv = report?.pak_inventory

  return (
    <main className="layout">
      <header>
        <h1>shrinkray</h1>
        <span className="muted">UE game folder optimizer · v0.0.2 · analysis + restore</span>
      </header>

      <section className="drop">
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
          <h2>Backup detected</h2>
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
                  {restore.failures.length > 10 && (
                    <li className="muted small">
                      …and {restore.failures.length - 10} more
                    </li>
                  )}
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
          {languages.length > 1 && (
            <p className="muted small" style={{ marginTop: '0.6rem' }}>
              Ceiling assumes you keep only the largest detected language ({largestLang}).
              Real savings depend on which languages you actually strip in step 3.
            </p>
          )}

          {languages.length > 0 && (
            <>
              <h2 style={{ marginTop: '1.6rem' }}>Languages detected ({languages.length})</h2>
              <table>
                <tbody>
                  {languages.map(([code, cat]) => (
                    <Row
                      key={code}
                      label={code === largestLang ? `${code}  (largest)` : code}
                      value={`${cat.count.toLocaleString()} files · ${formatBytes(cat.size)}`}
                      accent={code === largestLang}
                    />
                  ))}
                </tbody>
              </table>
            </>
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
