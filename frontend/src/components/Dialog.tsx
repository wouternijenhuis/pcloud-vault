import { useEffect, useRef, type FormEvent, type ReactNode } from 'react'
import { X } from 'lucide-react'

type Props = { title: string; description?: string; children: ReactNode; onClose: () => void; onSubmit?: (event: FormEvent<HTMLFormElement>) => void; submitLabel?: string; busy?: boolean; danger?: boolean }
export function Dialog({ title, description, children, onClose, onSubmit, submitLabel, busy = false, danger = false }: Props) {
  const ref = useRef<HTMLDivElement>(null)
  useEffect(() => {
    const previous = document.activeElement as HTMLElement | null
    ref.current?.querySelector<HTMLElement>('input,button,select')?.focus()
    const key = (event: KeyboardEvent) => { if (event.key === 'Escape' && !busy) onClose() }
    window.addEventListener('keydown', key)
    return () => { window.removeEventListener('keydown', key); previous?.focus() }
  }, [onClose, busy])
  const contents = <>
    <div className="dialog-heading"><div><h2>{title}</h2>{description && <p>{description}</p>}</div><button className="icon-button" aria-label="Close dialog" onClick={onClose}><X size={18} /></button></div>
    <div className="dialog-body">{children}</div>
    {submitLabel && <div className="dialog-actions"><button className="button button-subtle" type="button" onClick={onClose} disabled={busy}>Cancel</button><button className={`button ${danger ? 'button-danger' : 'button-primary'}`} type="submit" disabled={busy}>{busy ? 'Please wait…' : submitLabel}</button></div>}
  </>
  return <div className="dialog-backdrop" onMouseDown={event => { if (event.target === event.currentTarget && !busy) onClose() }}>
    <div className="dialog" role="dialog" aria-modal="true" aria-labelledby="dialog-title" ref={ref}>
      {onSubmit ? <form onSubmit={onSubmit}><h2 id="dialog-title" className="sr-only">{title}</h2>{contents}</form> : <><h2 id="dialog-title" className="sr-only">{title}</h2>{contents}</>}
    </div>
  </div>
}
