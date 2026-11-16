import { useEffect, useRef } from 'react'
import { ArrowDownToLine, ArrowUpFromLine, CircleAlert, X } from 'lucide-react'
import { ProtectionBadge } from './ProtectionBadge'
import { dateTimeLabel, formatSize, plural } from '../format'
import type { Activity, Transfer } from '../types'

const isRunning = (item: Transfer) => item.state === 'queued' || item.state === 'active'

export function transferCounts(transfers: Transfer[], pendingCount: number) {
  const downloads = transfers.filter(item => item.direction === 'download' && isRunning(item)).length
  const uploads = transfers.filter(item => item.direction === 'upload' && isRunning(item)).length + Math.max(0, pendingCount)
  return { downloads, uploads }
}

type Props = { transfers: Transfer[]; pendingCount: number; open: boolean; onToggle: () => void }

/** Left: downloads, right: uploads — the same plain summary as the official pCloud app. */
export function StatusBar({ transfers, pendingCount, open, onToggle }: Props) {
  const { downloads, uploads } = transferCounts(transfers, pendingCount)
  const failed = transfers.some(item => item.state === 'failed')
  return <footer className="statusbar">
    <button type="button" className="status-button" aria-expanded={open} aria-controls="activity-drawer" onClick={onToggle}>
      <ArrowDownToLine size={14} aria-hidden="true" /><span>{downloads ? `Downloading ${plural(downloads, 'file')}…` : 'Everything downloaded'}</span>
    </button>
    {failed && <button type="button" className="status-button status-failed" onClick={onToggle}><CircleAlert size={14} aria-hidden="true" /> Some transfers failed</button>}
    <button type="button" className="status-button" aria-expanded={open} aria-controls="activity-drawer" onClick={onToggle}>
      <span>{uploads ? `Uploading ${plural(uploads, 'file')}…` : 'Everything uploaded'}</span><ArrowUpFromLine size={14} aria-hidden="true" />
    </button>
  </footer>
}

export function TransferList({ transfers, empty }: { transfers: Transfer[]; empty: string }) {
  if (!transfers.length) return <p className="mini-empty">{empty}</p>
  return <ul className="transfer-list">{transfers.map(item => {
    // Backup and sync jobs report no byte total; show only their state, never "0 B of 0 B".
    const sized = item.total > 0
    const percent = sized ? Math.min(100, Math.round(item.done / item.total * 100)) : item.state === 'done' ? 100 : 0
    const state = item.state === 'queued' ? 'Waiting' : item.state === 'active' ? (sized ? `${percent}%` : 'Working…') : item.state === 'done' ? 'Done' : item.error ?? 'Failed'
    return <li key={item.id} className={`transfer-item ${item.state}`}>
      <div className="transfer-row">
        {item.direction === 'upload' ? <ArrowUpFromLine size={14} aria-label="Upload" /> : <ArrowDownToLine size={14} aria-label="Download" />}
        <strong title={item.name}>{item.name}</strong>
        <ProtectionBadge location={item.location} />
        <span className="transfer-state">{state}</span>
      </div>
      <div className="progress" role="progressbar" aria-label={`${item.name} progress`} aria-valuemin={0} aria-valuemax={100} aria-valuenow={percent}><i style={{ width: `${percent}%` }} /></div>
      {sized && <small>{formatSize(item.done)} of {formatSize(item.total)}</small>}
    </li>
  })}</ul>
}

type DrawerProps = { transfers: Transfer[]; activity: Activity[]; onClose: () => void; onClear: () => void }

/** Activity & transfers: what is moving now and what happened recently. */
export function ActivityDrawer({ transfers, activity, onClose, onClear }: DrawerProps) {
  const ref = useRef<HTMLElement>(null)
  useEffect(() => {
    ref.current?.querySelector<HTMLElement>('button')?.focus()
    const key = (event: KeyboardEvent) => { if (event.key === 'Escape') onClose() }
    window.addEventListener('keydown', key)
    return () => window.removeEventListener('keydown', key)
  }, [onClose])
  const finished = transfers.some(item => item.state === 'done' || item.state === 'failed')
  return <aside id="activity-drawer" className="drawer" aria-label="Activity and transfers" ref={ref}>
    <div className="drawer-head"><h2>Activity &amp; transfers</h2><button type="button" className="icon-button" aria-label="Close activity" onClick={onClose}><X size={17} /></button></div>
    <section className="drawer-section">
      <div className="drawer-section-title"><h3>Transfers</h3>{finished && <button type="button" className="link-button" onClick={onClear}>Clear finished</button>}</div>
      <TransferList transfers={transfers} empty="Nothing is uploading or downloading." />
    </section>
    <section className="drawer-section">
      <div className="drawer-section-title"><h3>Recent activity</h3></div>
      {activity.length ? <ul className="activity-list">{activity.map(item => <li className="activity-item" key={item.id}><span className={`activity-dot ${item.kind}`} /><div><strong>{item.message}</strong><small>{dateTimeLabel(item.time)}</small></div></li>)}</ul>
        : <p className="mini-empty">Nothing here yet. File and sync updates will show up here.</p>}
    </section>
  </aside>
}
