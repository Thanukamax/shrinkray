import { useState } from 'react'
import { invoke } from '@tauri-apps/api/core'
import { open } from '@tauri-apps/plugin-dialog'

type Row = {
  sample: string
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

  async function runSynthetic() {
    setPending(true)
    setErr(null)
    try {
      const r = await invoke<BenchResult>('delta_codec_run_synthetic_bench')
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
      const r = await invoke<BenchResult>('delta_codec_run_file_bench', { path: picked })
      setResult(r)
      setLastLabel(picked.split('/').pop() ?? picked)
    } catch (e: unknown) {
      setErr(String(e))
    } finally {
      setPending(false)
    }
  }

  return (
    <section className="report">
      <h2>Δ-Codec — Byte-Exact AI Compression</h2>
      <p className="muted small">
        A new bitstream that ships both an AI-predicted high mip AND a compressed residual.
        Restore = predict + apply residual. Byte-exact verified via SHA-256 of the reconstructed
        RGBA. Industry says you pick one of "lossy-small" or "byte-exact." We're testing whether
        you can have both.
      </p>
      <div className="actions">
        <button onClick={runSynthetic} disabled={pending}>
          {pending ? 'running…' : 'run synthetic bench'}
        </button>
        <button onClick={runOnFile} disabled={pending}>
          run on image…
        </button>
      </div>

      {err && <p className="err small">{err}</p>}

      {result && (
        <div className="delta-codec-result">
          {lastLabel && (
            <p className="muted small">
              Sample: <span className="delta-codec-sample-label">{lastLabel}</span> · spec{' '}
              {result.spec_version}
            </p>
          )}
          <div className="mipstrip-overflow">
            <table className="delta-codec-table">
              <thead>
                <tr>
                  <th>sample</th>
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
                    <td className="num">{r.quant_step}</td>
                    <td className="num">{fmtBytes(r.top_mip_bytes)}</td>
                    <td className="num">{fmtBytes(r.low_mip_bytes)}</td>
                    <td className="num">{fmtBytes(r.residual_zst_bytes)}</td>
                    <td className="num">{fmtBytes(r.delta_total_bytes)}</td>
                    <td className={`ratio ${r.ratio < 1.0 ? 'ratio-win' : 'ratio-loss'}`}>
                      {r.ratio.toFixed(2)}×
                    </td>
                    <td className="num">{r.max_channel_error}</td>
                    <td
                      className={`byte-exact-cell ${r.byte_exact ? 'byte-exact-yes' : 'byte-exact-no'}`}
                    >
                      {r.byte_exact ? 'YES' : '—'}
                    </td>
                  </tr>
                ))}
              </tbody>
            </table>
          </div>
          <p className="muted small">
            {result.lossless_runs} byte-exact run(s) · best ratio{' '}
            <span className="delta-codec-summary-ratio">
              {result.best_lossless_ratio.toFixed(2)}×
            </span>{' '}
            of the ExactBackup baseline. q=1 is lossless; q=2/4 trade bounded error for further
            residual shrink. Any ratio &lt; 1.0× beats full backup on disk.
          </p>
        </div>
      )}
    </section>
  )
}
