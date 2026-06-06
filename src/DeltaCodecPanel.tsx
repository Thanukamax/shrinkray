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
