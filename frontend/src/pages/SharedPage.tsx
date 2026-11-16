import { useEffect, useState } from 'react'
import { Check, Copy, ExternalLink, File, Folder, Link2, ShieldCheck, Trash2, Users, X } from 'lucide-react'
import { listLinks, listShares, runExtra } from '../api'
import { CloudGate, PageHeader } from '../components/Gates'
import { ProtectionBadge } from '../components/ProtectionBadge'
import { copyText, errorMessage, timeLabel, plural } from '../format'
import type { AppStatus, PublicLink, Share } from '../types'
import type { Perform } from '../ui'
import { VAULT_SHARE_NOTE } from './FilesPage'

type Props = { status: AppStatus | null; busy: boolean; dataVersion: number; perform: Perform; announce: (message: string) => void; openConnect: () => void }

export function SharedPage({ status, busy, dataVersion, perform, announce, openConnect }: Props) {
  const [tab, setTab] = useState<'links' | 'folders'>('links')
  const [links, setLinks] = useState<PublicLink[]>([])
  const [shares, setShares] = useState<Share[]>([])
  const [problem, setProblem] = useState('')
  const connected = status?.connected ?? false
  useEffect(() => {
    if (!connected) return
    let cancelled = false
    Promise.all([listLinks(), listShares()])
      .then(([linkItems, shareItems]) => { if (!cancelled) { setLinks(linkItems); setShares(shareItems); setProblem('') } })
      .catch(reason => { if (!cancelled) setProblem(errorMessage(reason, 'Could not load shared items.')) })
    return () => { cancelled = true }
  }, [connected, dataVersion])
  const header = <PageHeader title="Shared" description="Links and folders you share from pCloud Files, and folders others share with you." />
  if (!connected) return <div className="page">{header}<CloudGate connect={openConnect} what="share files and folders" /></div>
  const copy = (url: string) => void copyText(url).then(ok => announce(ok ? 'Link copied' : 'Could not copy the link'))
  const incoming = shares.filter(share => share.direction === 'incoming')
  const outgoing = shares.filter(share => share.direction === 'outgoing')
  const shareRow = (share: Share) => <li key={share.id} className="list-row">
    <span className="list-icon"><Folder size={17} aria-hidden="true" /></span>
    <div className="list-text"><strong>{share.folderName}</strong><small>{share.direction === 'incoming' ? `From ${share.email}` : `With ${share.email}`} · {share.permissions === 'edit' ? 'Can edit' : 'Can view'} · {share.state === 'pending' ? 'Waiting for an answer' : 'Active'}</small></div>
    <ProtectionBadge location="cloud" />
    <div className="list-actions">
      {share.direction === 'incoming' && share.state === 'pending' ? <>
        <button type="button" className="button button-primary" disabled={busy} onClick={() => void perform(() => runExtra('accept_share', { id: share.id }), 'Share accepted')}><Check size={14} /> Accept</button>
        <button type="button" className="button button-subtle" disabled={busy} onClick={() => void perform(() => runExtra('decline_share', { id: share.id }), 'Share declined')}><X size={14} /> Decline</button>
      </> : <button type="button" className="button button-subtle" disabled={busy} onClick={() => void perform(() => runExtra('remove_share', { id: share.id }), share.direction === 'incoming' ? 'You left the shared folder' : 'Sharing stopped')}>{share.direction === 'incoming' ? 'Leave' : 'Stop sharing'}</button>}
    </div>
  </li>
  return <div className="page">
    {header}
    <div className="notice enc-note"><ShieldCheck size={15} aria-hidden="true" /><span>{VAULT_SHARE_NOTE}</span></div>
    <div className="segmented" role="group" aria-label="Shared items"><button type="button" aria-pressed={tab === 'links'} className={tab === 'links' ? 'active' : ''} onClick={() => setTab('links')}><Link2 size={15} aria-hidden="true" />Links</button><button type="button" aria-pressed={tab === 'folders'} className={tab === 'folders' ? 'active' : ''} onClick={() => setTab('folders')}><Users size={15} aria-hidden="true" />Shared folders</button></div>
    {problem && <p className="mini-empty" role="alert">{problem}</p>}
    {tab === 'links' ? <section className="card-list" aria-label="Links">
      {links.length === 0 ? <p className="mini-empty">No links yet. In pCloud Files, select a file or folder and choose Get link.</p>
        : <ul className="list">{links.map(link => <li key={link.id} className="list-row">
          <span className="list-icon">{link.isFolder ? <Folder size={17} aria-hidden="true" /> : <File size={17} aria-hidden="true" />}</span>
          <div className="list-text"><strong>{link.name}</strong><small>Created {timeLabel(link.created)} · {plural(link.downloads, 'download')} · <span className="mono">{link.url}</span></small></div>
          <ProtectionBadge location="cloud" />
          <div className="list-actions">
            <button type="button" className="button button-subtle" onClick={() => copy(link.url)} aria-label={`Copy link to ${link.name}`}><Copy size={14} /> Copy</button>
            <button type="button" className="button button-subtle" onClick={() => void perform(() => runExtra('open_url', { url: link.url }))} aria-label={`Open link to ${link.name}`}><ExternalLink size={14} /> Open</button>
            <button type="button" className="button button-ghost-danger" disabled={busy} onClick={() => void perform(() => runExtra('delete_link', { id: link.id }), 'Link deleted')} aria-label={`Delete link to ${link.name}`}><Trash2 size={14} /> Delete</button>
          </div>
        </li>)}</ul>}
    </section> : <section className="card-list" aria-label="Shared folders">
      <h3>Shared with you</h3>
      {incoming.length ? <ul className="list">{incoming.map(shareRow)}</ul> : <p className="mini-empty">Nobody has shared a folder with you.</p>}
      <h3>Shared by you</h3>
      {outgoing.length ? <ul className="list">{outgoing.map(shareRow)}</ul> : <p className="mini-empty">You are not sharing any folders. In pCloud Files, select a folder and choose Share folder.</p>}
    </section>}
  </div>
}
