import { useCallback, useEffect, useState, type FormEvent } from 'react'
import { Copy, Inbox, Plus, ShieldCheck, Trash2 } from 'lucide-react'
import { listFileRequests, runExtra } from '../api'
import { Dialog } from '../components/Dialog'
import { FolderChooser, rootTarget } from '../components/DestinationPicker'
import { CloudGate, PageHeader } from '../components/Gates'
import { ProtectionBadge } from '../components/ProtectionBadge'
import { copyText, errorMessage, plural, timeLabel } from '../format'
import type { AppStatus, FileRequest, Protection, Target } from '../types'
import type { Perform } from '../ui'

type Props = { status: AppStatus | null; protection: Protection | null; busy: boolean; dataVersion: number; perform: Perform; announce: (message: string) => void; openConnect: () => void }
export const FILE_REQUEST_NOTE = 'Files arrive unencrypted on pCloud first. The app encrypts and moves them when it runs.'

export function FileRequestPage({ status, protection, busy, dataVersion, perform, announce, openConnect }: Props) {
  const [requests, setRequests] = useState<FileRequest[]>([])
  const [problem, setProblem] = useState('')
  const [creating, setCreating] = useState(false)
  const connected = status?.connected ?? false
  useEffect(() => {
    if (!connected) return
    let cancelled = false
    listFileRequests().then(items => { if (!cancelled) { setRequests(items); setProblem('') } }).catch(reason => { if (!cancelled) setProblem(errorMessage(reason, 'Could not load file requests.')) })
    return () => { cancelled = true }
  }, [connected, dataVersion])
  const header = <PageHeader title="File request" description="Let anyone upload files to you with a link, without a pCloud account." actions={connected ? <button type="button" className="button button-primary" onClick={() => setCreating(true)}><Plus size={16} /> New file request</button> : undefined} />
  if (!connected) return <div className="page">{header}<CloudGate connect={openConnect} what="receive files with a link" /></div>
  return <div className="page">
    {header}
    {problem && <p className="mini-empty" role="alert">{problem}</p>}
    {requests.length === 0 ? <div className="empty-state"><img src="/art-empty.svg" alt="" /><h3>No file requests</h3><p>Create a link that others can use to send you files.</p></div>
      : <ul className="list card-list" aria-label="File requests">{requests.map(request => <li key={request.id} className="list-row">
        <span className="list-icon"><Inbox size={17} aria-hidden="true" /></span>
        <div className="list-text">
          <strong>{request.folderName}</strong>
          <small>{plural(request.filesReceived, 'file')} received{request.maxFiles ? ` of ${request.maxFiles}` : ''} · {request.expires ? `Expires ${timeLabel(request.expires)}` : 'No expiry'}{request.comment ? ` · “${request.comment}”` : ''}</small>
          <small>{request.encrypt ? `Files arrive unencrypted in pCloud. The app then encrypts them into ${request.vaultFolderName ? `Vault / ${request.vaultFolderName}` : 'the Vault'} and moves the plain copy to the pCloud trash.` : 'Received files stay in pCloud, not encrypted.'}</small>
        </div>
        {/* Uploads always land unencrypted first, so the arrival folder is labelled as such. */}
        <ProtectionBadge location="cloud" />
        {request.encrypt && <ProtectionBadge location="vault" />}
        <div className="list-actions">
          <button type="button" className="button button-subtle" aria-label={`Copy link for ${request.folderName}`} onClick={() => void copyText(request.url).then(ok => announce(ok ? 'Link copied' : 'Could not copy the link'))}><Copy size={14} /> Copy link</button>
          <button type="button" className="button button-ghost-danger" disabled={busy} aria-label={`Delete file request for ${request.folderName}`} onClick={() => void perform(() => runExtra('delete_file_request', { id: request.id }), 'File request deleted')}><Trash2 size={14} /> Delete</button>
        </div>
      </li>)}</ul>}
    {creating && <FileRequestDialog protection={protection} busy={busy} onClose={() => setCreating(false)} perform={perform} />}
  </div>
}

export function FileRequestDialog({ protection, busy, onClose, perform }: { protection: Protection | null; busy: boolean; onClose: () => void; perform: Perform }) {
  const forced = protection === 'everything'
  const [folder, setFolder] = useState<Target>(rootTarget('cloud'))
  const [vaultFolder, setVaultFolder] = useState<Target>(rootTarget('vault'))
  const [comment, setComment] = useState('')
  const [expiresDays, setExpiresDays] = useState('7')
  const [maxFiles, setMaxFiles] = useState('')
  const [encrypt, setEncrypt] = useState(protection !== 'vaultOnly')
  const changeFolder = useCallback((value: Target) => setFolder(value), [])
  const changeVaultFolder = useCallback((value: Target) => setVaultFolder(value), [])
  const willEncrypt = forced || encrypt
  const submit = async (event: FormEvent<HTMLFormElement>) => {
    event.preventDefault()
    if (!folder.folderId) return
    const folderId = folder.folderId
    const done = await perform(() => runExtra('create_file_request', {
      folderId, comment: comment.trim(), expiresDays: expiresDays ? Number(expiresDays) : null, maxFiles: maxFiles ? Number(maxFiles) : null,
      encrypt: willEncrypt, vaultFolderId: willEncrypt ? vaultFolder.folderId : null,
    }), 'File request created')
    if (done) onClose()
  }
  return <Dialog title="New file request" description="People with the link can upload files into a pCloud folder you choose." onClose={onClose} onSubmit={event => void submit(event)} submitLabel="Create file request" busy={busy}>
    <div className="field-label">pCloud folder for received files<FolderChooser value={folder} onChange={changeFolder} /></div>
    {!folder.folderId && <p className="hint">Open a folder above to choose it.</p>}
    <label className="field-label">Message for uploaders<input value={comment} maxLength={300} onChange={event => setComment(event.target.value)} placeholder="e.g. Please upload your signed forms." /></label>
    <div className="field-columns">
      <label className="field-label">Expires after (days)<input type="number" min={1} max={365} value={expiresDays} onChange={event => setExpiresDays(event.target.value)} placeholder="Never" /></label>
      <label className="field-label">Maximum files<input type="number" min={1} max={10000} value={maxFiles} onChange={event => setMaxFiles(event.target.value)} placeholder="No limit" /></label>
    </div>
    <label className="switch-label"><input type="checkbox" role="switch" checked={willEncrypt} disabled={forced} onChange={event => setEncrypt(event.target.checked)} /><span><strong>Move received files into the Vault</strong>{forced ? ' — always on, because everything is encrypted.' : ''}</span></label>
    {willEncrypt && <div className="field-label">Vault folder<FolderChooser value={vaultFolder} onChange={changeVaultFolder} /></div>}
    <p className="privacy-note"><ShieldCheck size={15} /> {FILE_REQUEST_NOTE}</p>
  </Dialog>
}
