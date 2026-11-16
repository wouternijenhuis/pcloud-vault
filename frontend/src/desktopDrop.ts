import { useEffect, useRef } from 'react'
import { getCurrentWebviewWindow } from '@tauri-apps/api/webviewWindow'
import { isDesktop } from './api'

/** Receives files and folders dropped on the desktop window while `enabled`; the latest handler is always used. */
export function useDesktopDrop(handler: (paths: string[]) => void, enabled: boolean) {
  const latest = useRef(handler)
  latest.current = handler
  useEffect(() => {
    if (!enabled || !isDesktop()) return
    let unlisten: (() => void) | undefined
    let cancelled = false
    getCurrentWebviewWindow().onDragDropEvent(event => {
      if (event.payload.type === 'drop' && event.payload.paths.length) latest.current(event.payload.paths)
    }).then(fn => { if (cancelled) fn(); else unlisten = fn }).catch(() => undefined)
    return () => { cancelled = true; unlisten?.() }
  }, [enabled])
}
