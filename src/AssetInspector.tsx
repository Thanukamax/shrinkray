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
