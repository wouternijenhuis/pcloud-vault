import { useCallback, useRef, useState } from 'react'
import { CloudUpload, FolderUp, LockKeyhole, Upload } from 'lucide-react'
import { runExtra, uploadDemoFiles } from '../api'
import { DestinationPicker, defaultTarget } from '../components/DestinationPicker'
import { PageHeader } from '../components/Gates'
import { ProtectionBadge } from '../components/ProtectionBadge'
import { TransferList } from '../components/StatusBar'
import { useDesktopDrop } from '../desktopDrop'
import type { AppStatus, Protection, Target, Transfer } from '../types'
import type { Perform } from '../ui'

type Props = {
  status: AppStatus | null
  protection: Protection | null
  transfers: Transfer[]
  desktop: boolean
  busy: boolean
  perform: Perform
  openSettings: () => void
  openConnect: () => void
  openUnlock: () => void
}

export function UploadPage({ status, protection, transfers, desktop, busy, perform, openSettings, openConnect, openUnlock }: Props) {
  const [target, setTarget] = useState<Target>(() => defaultTarget(protection))
  const [dragging, setDragging] = useState(false)
  const fileInput = useRef<HTMLInputElement>(null)
  const changeTarget = useCallback((value: Target) => setTarget(value), [])
  const encrypted = target.location === 'vault'
  const blocked = !status?.connected ? 'connect' : encrypted && !status.unlocked ? 'unlock' : null
  const where = { location: target.location, folderId: target.folderId }
  const done = encrypted ? `Uploading to ${target.label}, encrypted` : `Uploading to ${target.label}, not encrypted`
  useDesktopDrop(paths => { if (!blocked) void perform(() => runExtra('upload_paths', { paths, ...where }), done) }, true)
  const demoUpload = (files: FileList | null) => {
    if (!files?.length) return
    const list = Array.from(files)
    void perform(() => uploadDemoFiles(list, target.location, target.folderId), done).then(() => { if (fileInput.current) fileInput.current.value = '' })
  }
  const browseFiles = () => {
    if (blocked || busy) return
    if (!desktop) { fileInput.current?.click(); return }
    void perform(() => runExtra('upload_files', where), done)
  }
  const browseFolder = () => { if (!blocked && !busy) void perform(() => runExtra('upload_folder', where), done) }
  const uploads = transfers.filter(item => item.direction === 'upload')

  return <div className="page upload-page">
    <PageHeader title="Upload" description="Send files and folders from this computer to the cloud. The originals stay where they are." />
    <section className="upload-destination" aria-label="Upload destination">
      <h3>Upload to</h3>
      <DestinationPicker protection={protection} value={target} onChange={changeTarget} folders onOpenSettings={openSettings} />
    </section>
    <div className={`dropzone ${encrypted ? 'enc' : 'plain'} ${dragging ? 'dragging' : ''} ${blocked ? 'blocked' : ''}`}
      onDragOver={event => { if (!desktop) { event.preventDefault(); setDragging(true) } }}
      onDragLeave={() => setDragging(false)}
      onDrop={event => { if (!desktop) { event.preventDefault(); setDragging(false); if (!blocked) demoUpload(event.dataTransfer.files) } }}>
      <CloudUpload size={46} strokeWidth={1.5} aria-hidden="true" className="dropzone-icon" />
      <h3>Upload your files</h3>
      <p>Drop files and folders here or click Browse</p>
      <p className={`dropzone-protection ${encrypted ? 'enc' : 'plain'}`}><ProtectionBadge location={target.location} /> {encrypted ? 'Files are encrypted before they leave this computer.' : 'Files are uploaded as they are. pCloud can read them.'}</p>
      {blocked === 'connect' ? <button type="button" className="button button-primary" onClick={openConnect}>Connect pCloud to upload</button>
        : blocked === 'unlock' ? <button type="button" className="button button-primary" onClick={openUnlock}><LockKeyhole size={15} /> Unlock the Vault to upload</button>
          : <div className="dropzone-actions">
            <button type="button" className="button button-primary" onClick={browseFiles} disabled={busy}><Upload size={15} /> Browse files</button>
            <button type="button" className="button button-subtle" onClick={browseFolder} disabled={busy}><FolderUp size={15} /> Browse folder</button>
          </div>}
      {!desktop && <input ref={fileInput} type="file" multiple hidden onChange={event => demoUpload(event.target.files)} />}
    </div>
    <section className="queue" aria-label="Upload queue">
      <h3>Queue</h3>
      <TransferList transfers={uploads} empty="Nothing is uploading. Files you add appear here." />
    </section>
  </div>
}
