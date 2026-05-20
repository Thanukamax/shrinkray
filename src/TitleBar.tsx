import { getCurrentWindow } from '@tauri-apps/api/window'

const win = getCurrentWindow()

export function TitleBar({ title, subtitle }: { title: string; subtitle?: string }) {
  return (
    <div className="title-bar" data-tauri-drag-region>
      <div className="title-bar-text" data-tauri-drag-region>
        {title}
        {subtitle && (
          <span className="title-bar-subtitle" data-tauri-drag-region>
            {' '}
            {subtitle}
          </span>
        )}
      </div>
      <div className="title-bar-controls">
        <button aria-label="Minimize" onClick={() => win.minimize()} />
        <button aria-label="Maximize" onClick={() => win.toggleMaximize()} />
        <button aria-label="Close" onClick={() => win.close()} />
      </div>
    </div>
  )
}
