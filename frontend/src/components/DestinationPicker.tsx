import { useEffect, useRef, useState } from 'react'
import { ChevronRight, Cloud, Folder, FolderUp, ShieldCheck } from 'lucide-react'
import { listEntries } from '../api'
import { lockedLocation, rootLabel } from '../format'
import type { Location, Protection, Target } from '../types'

type Props = {
  protection: Protection | null
  value: Target
  onChange: (value: Target) => void
  /** Show the folder chooser for the selected side. */
  folders?: boolean
  onOpenSettings?: () => void
}

export const rootTarget = (location: Location): Target => ({ location, folderId: null, label: rootLabel(location) })
/** The destination a new item starts with, honoring the protection mode. */
export const defaultTarget = (protection: Protection | null): Target => rootTarget(lockedLocation(protection) ?? 'vault')

const LOCK_REASON: Record<Location, string> = {
  vault: 'Everything is encrypted, so new files always go to the Vault.',
  cloud: 'You chose to encrypt only the Vault, so new files go to pCloud unencrypted.',
}

/** Two large cards make the encrypted/unencrypted choice explicit, then an optional folder chooser. */
export function DestinationPicker({ protection, value, onChange, folders = false, onOpenSettings }: Props) {
  const locked = lockedLocation(protection)
  useEffect(() => { if (locked && value.location !== locked) onChange(rootTarget(locked)) }, [locked, value.location, onChange])
  const card = (location: Location) => {
    const encrypted = location === 'vault'
    const selected = value.location === location
    const disabled = locked !== null && locked !== location
    return <button type="button" role="radio" aria-checked={selected} disabled={disabled}
      className={`destination-card ${encrypted ? 'enc' : 'plain'} ${selected ? 'selected' : ''}`}
      onClick={() => { if (!selected) onChange(rootTarget(location)) }}>
      <span className="destination-icon">{encrypted ? <ShieldCheck size={20} /> : <Cloud size={20} />}</span>
      <span className="destination-text">
        <strong>{encrypted ? 'Vault — encrypted' : 'pCloud — not encrypted'}</strong>
        <small>{encrypted ? 'Encrypted on this computer. Only you can read these files.' : 'Stored as is. pCloud can read these files.'}</small>
      </span>
    </button>
  }
  return <div className="destination-picker">
    <div className="destination-cards" role="radiogroup" aria-label="Destination">{card('vault')}{card('cloud')}</div>
    {locked && <p className="destination-lock">{LOCK_REASON[locked]}{onOpenSettings && <> <button type="button" className="link-button" onClick={onOpenSettings}>Change in Settings</button></>}</p>}
    {folders && <FolderChooser value={value} onChange={onChange} />}
  </div>
}

type Crumb = { id: string; name: string }

/** Browses folders on one side (Vault or pCloud); the folder being shown is the chosen one. */
export function FolderChooser({ value, onChange }: { value: Target; onChange: (value: Target) => void }) {
  const [path, setPath] = useState<Crumb[]>([])
  const [folders, setFolders] = useState<Crumb[]>([])
  const [problem, setProblem] = useState('')
  const request = useRef(0)
  const location = value.location
  const parentId = path.at(-1)?.id ?? null
  useEffect(() => { setPath([]) }, [location])
  useEffect(() => {
    const id = ++request.current
    setProblem('')
    listEntries(location, parentId, null, 'files')
      .then(items => { if (id === request.current) setFolders((Array.isArray(items) ? items : []).filter(item => item.kind === 'folder').map(item => ({ id: item.id, name: item.name }))) })
      .catch(() => { if (id === request.current) { setFolders([]); setProblem(location === 'vault' ? 'Unlock the Vault to choose a folder in it.' : 'Connect pCloud to choose a folder in it.') } })
  }, [location, parentId])
  const go = (next: Crumb[]) => {
    setPath(next)
    const label = [rootLabel(location), ...next.map(item => item.name)].join(' / ')
    onChange({ location, folderId: next.at(-1)?.id ?? null, label })
  }
  return <div className="folder-chooser">
    <div className="folder-chooser-head">
      <span>Folder: <strong>{value.label}</strong></span>
      {path.length > 0 && <button type="button" className="link-button" onClick={() => go(path.slice(0, -1))}><FolderUp size={14} /> Up one level</button>}
    </div>
    {problem ? <p className="folder-chooser-empty">{problem}</p>
      : folders.length === 0 ? <p className="folder-chooser-empty">No folders inside. Files go straight into {value.label}.</p>
        : <ul className="folder-chooser-list" aria-label={`Folders in ${value.label}`}>{folders.map(item => <li key={item.id}>
          <button type="button" onClick={() => go([...path, item])} aria-label={`Open ${item.name}`}><Folder size={15} /><span>{item.name}</span><ChevronRight size={14} /></button>
        </li>)}</ul>}
  </div>
}
