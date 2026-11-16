import { useCallback, useState } from 'react'
import { Check, FolderOpen } from 'lucide-react'
import { Dialog } from './Dialog'
import { DestinationPicker, defaultTarget } from './DestinationPicker'
import { ProtectionBadge } from './ProtectionBadge'
import type { ExtraArguments, JobKind, KnownFolders, Protection, Target } from '../types'

type Props = {
  kind: JobKind
  initialPath?: string | null
  protection: Protection | null
  knownFolders: KnownFolders | null
  busy: boolean
  pickFolder: () => Promise<string | null>
  onSubmit: (args: ExtraArguments['add_job']) => Promise<boolean>
  onClose: () => void
  onOpenSettings: () => void
}

export const knownFolderList = (known: KnownFolders | null) => [
  { label: 'Documents', path: known?.documents ?? null },
  { label: 'Pictures', path: known?.pictures ?? null },
  { label: 'Desktop', path: known?.desktop ?? null },
].filter((item): item is { label: string; path: string } => !!item.path)

/** One plain sentence saying where a folder's files end up and who can read them. */
export function jobSummary(kind: JobKind, localPath: string, target: Target) {
  const where = target.location === 'vault'
    ? `Files in ${localPath} will be encrypted before they leave this computer and stored in ${target.label}. Only you can read them.`
    : `Files in ${localPath} will be stored unencrypted in ${target.label}; pCloud can read them.`
  return `${where} ${kind === 'sync' ? 'Changes and deletions flow both ways.' : 'This is a one-way copy.'}`
}

const STEPS = ['Folder', 'Destination', 'Options'] as const

/** Add a backup or sync in three steps: local folder, destination, options with a plain summary. */
export function FolderJobDialog({ kind, initialPath = null, protection, knownFolders, busy, pickFolder, onSubmit, onClose, onOpenSettings }: Props) {
  const [step, setStep] = useState(initialPath ? 1 : 0)
  const [localPath, setLocalPath] = useState<string | null>(initialPath)
  const [target, setTarget] = useState<Target>(() => defaultTarget(protection))
  const [mirrorDeletions, setMirrorDeletions] = useState(false)
  const [picking, setPicking] = useState(false)
  const changeTarget = useCallback((value: Target) => setTarget(value), [])
  const noun = kind === 'sync' ? 'sync' : 'backup'
  const choose = async () => {
    setPicking(true)
    try { const path = await pickFolder(); if (path) setLocalPath(path) } finally { setPicking(false) }
  }
  const submit = async () => {
    if (!localPath) return
    const done = await onSubmit({ kind, localPath, location: target.location, folderId: target.folderId, mirrorDeletions: kind === 'backup' && mirrorDeletions })
    if (done) onClose()
  }
  return <Dialog title={kind === 'sync' ? 'Sync a folder' : 'Back up a folder'} description={kind === 'sync' ? 'Keep the same files on this computer and in the cloud.' : 'Keep a copy of a folder from this computer in the cloud.'} onClose={onClose} busy={busy}>
    <ol className="dialog-steps" aria-label="Steps">{STEPS.map((label, index) => <li key={label} className={index === step ? 'current' : index < step ? 'done' : ''} aria-current={index === step ? 'step' : undefined}><span>{index < step ? <Check size={11} /> : index + 1}</span>{label}</li>)}</ol>
    {step === 0 && <div className="job-step">
      <h3>Which folder on this computer?</h3>
      {knownFolderList(knownFolders).length > 0 && <div className="chip-row" aria-label="Suggested folders">{knownFolderList(knownFolders).map(item => <button key={item.label} type="button" className={`chip ${localPath === item.path ? 'selected' : ''}`} aria-pressed={localPath === item.path} onClick={() => setLocalPath(item.path)}><FolderOpen size={14} /> {item.label}</button>)}</div>}
      <button type="button" className="button button-subtle" onClick={() => void choose()} disabled={picking}><FolderOpen size={15} /> {picking ? 'Choosing…' : 'Choose a folder…'}</button>
      {localPath && <p className="chosen-path">Selected: <strong>{localPath}</strong></p>}
    </div>}
    {step === 1 && <div className="job-step">
      <h3>Where should it go?</h3>
      <DestinationPicker protection={protection} value={target} onChange={changeTarget} folders onOpenSettings={() => { onClose(); onOpenSettings() }} />
    </div>}
    {step === 2 && localPath && <div className="job-step">
      <h3>Check and start</h3>
      <div className={`job-summary ${target.location === 'vault' ? 'enc' : 'plain'}`}><ProtectionBadge location={target.location} /><p>{jobSummary(kind, localPath, target)}</p></div>
      {kind === 'backup'
        ? <label className="check-label"><input type="checkbox" checked={mirrorDeletions} onChange={event => setMirrorDeletions(event.target.checked)} /><span><strong>Mirror deletions</strong> — when you delete a file on this computer, delete it from the backup too. Off: the backup keeps everything.</span></label>
        : <p className="hint">Files you delete on this computer are removed in the cloud too, and the other way around. Files removed from this computer by the sync go to the app's recycle folder for 30 days.</p>}
    </div>}
    <div className="dialog-actions inline">
      {step > 0 ? <button type="button" className="button button-subtle" onClick={() => setStep(step - 1)} disabled={busy}>Back</button> : <button type="button" className="button button-subtle" onClick={onClose} disabled={busy}>Cancel</button>}
      {step < 2 ? <button type="button" className="button button-primary" onClick={() => setStep(step + 1)} disabled={!localPath}>Next</button>
        : <button type="button" className="button button-primary" onClick={() => void submit()} disabled={busy || !localPath}>{busy ? 'Please wait…' : `Start ${noun}`}</button>}
    </div>
  </Dialog>
}
