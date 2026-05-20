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
