import { useEffect, useLayoutEffect, useRef, useState, type ReactNode } from 'react'

export type MenuItem = { label: string; icon?: ReactNode; run: () => void; danger?: boolean }

type Props = { x: number; y: number; label: string; items: MenuItem[]; onClose: () => void }

/** A keyboard-accessible context menu, kept inside the window and closed by Escape, outside clicks or blur. */
export function ContextMenu({ x, y, label, items, onClose }: Props) {
  const ref = useRef<HTMLDivElement>(null)
  const [position, setPosition] = useState({ left: x, top: y })

  useLayoutEffect(() => {
    const menu = ref.current
    if (!menu) return
    const box = menu.getBoundingClientRect()
    setPosition({
      left: Math.max(8, Math.min(x, window.innerWidth - box.width - 8)),
      top: Math.max(8, Math.min(y, window.innerHeight - box.height - 8)),
    })
    menu.querySelector<HTMLElement>('[role="menuitem"]')?.focus()
  }, [x, y])

  useEffect(() => {
    const previous = document.activeElement as HTMLElement | null
    const outside = (event: MouseEvent) => { if (!ref.current?.contains(event.target as Node)) onClose() }
    const keys = (event: KeyboardEvent) => {
      if (event.key === 'Escape' || event.key === 'Tab') { event.preventDefault(); onClose(); return }
      if (event.key !== 'ArrowDown' && event.key !== 'ArrowUp' && event.key !== 'Home' && event.key !== 'End') return
      event.preventDefault()
      const options = [...(ref.current?.querySelectorAll<HTMLElement>('[role="menuitem"]') ?? [])]
      const index = options.indexOf(document.activeElement as HTMLElement)
      const next = event.key === 'Home' ? 0 : event.key === 'End' ? options.length - 1
        : (index + (event.key === 'ArrowDown' ? 1 : -1) + options.length) % options.length
      options[next]?.focus()
    }
    window.addEventListener('mousedown', outside)
    window.addEventListener('keydown', keys)
    window.addEventListener('blur', onClose)
    window.addEventListener('resize', onClose)
    return () => {
      window.removeEventListener('mousedown', outside)
      window.removeEventListener('keydown', keys)
      window.removeEventListener('blur', onClose)
      window.removeEventListener('resize', onClose)
      previous?.focus()
    }
  }, [onClose])

  return <div ref={ref} className="context-menu" role="menu" aria-label={label} style={position} onContextMenu={event => event.preventDefault()}>
    {items.map(item => <button key={item.label} type="button" role="menuitem" tabIndex={-1} className={item.danger ? 'danger' : undefined} onClick={() => { onClose(); item.run() }}>
      {item.icon}<span>{item.label}</span>
    </button>)}
  </div>
}
