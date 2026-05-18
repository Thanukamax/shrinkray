import { useState } from 'react'
import { invoke } from '@tauri-apps/api/core'
import { open } from '@tauri-apps/plugin-dialog'

type Category = { count: number; size: number }
type AnalysisReport = {
  total_files: number
  total_size: number
  textures: Category
  audio: Category
  paks: Category
  estimated_savings: number
}

export default function App() {
  const [path, setPath] = useState<string | null>(null)
  const [report, setReport] = useState<AnalysisReport | null>(null)
  const [pending, setPending] = useState(false)
  const [error, setError] = useState<string | null>(null)

  async function pickFolder() {
    setError(null)
    const sel = await open({ directory: true, multiple: false, title: 'Pick a game folder' })
    if (typeof sel === 'string') {
      setPath(sel)
      setReport(null)
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
  }

  const savedPct =
    report && report.total_size > 0
      ? Math.round((report.estimated_savings / report.total_size) * 100)
      : 0

  return (
    <main className="layout">
      <header>
        <h1>shrinkray</h1>
        <span className="muted">UE game folder optimizer · phase 0</span>
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
              <Row
                label="estimated savings"
                value={`${formatBytes(report.estimated_savings)}  (~${savedPct}%)`}
                accent
              />
            </tbody>
          </table>
          <p className="muted small">
            Phase 0 estimate based on a fixed mix per category. Real per-asset estimates land in
            Phase 1 once textures and audio can be introspected.
          </p>
        </section>
      )}
    </main>
  )
}

function Row({ label, value, accent }: { label: string; value: string; accent?: boolean }) {
  return (
    <tr>
      <td>{label}</td>
      <td className={accent ? 'accent' : ''}>{value}</td>
    </tr>
  )
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
