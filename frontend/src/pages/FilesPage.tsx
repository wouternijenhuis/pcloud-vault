import { useCallback, useEffect, useRef, useState, type FormEvent } from 'react'
import {
  ArrowDownToLine, ArrowLeft, Check, ChevronRight, Cloud, Copy, Eye, File, FileText, Folder, FolderPlus, Grid2X2, HardDrive, Image,
  LayoutList, Link2, LockKeyhole, Plus, Share2, ShieldCheck, Star, Trash2, Upload, Wifi, X,
} from 'lucide-react'
import { importDemoFiles, importPaths, listEntries, runAction, runExtra } from '../api'
import { ContextMenu, type MenuItem } from '../components/ContextMenu'
import { Dialog } from '../components/Dialog'
import { CloudGate, LoadingGate, PageHeader, SetupGate, UnlockGate } from '../components/Gates'
import { ProtectionBadge } from '../components/ProtectionBadge'
import { useDesktopDrop } from '../desktopDrop'
import { copyText, errorMessage, formatSize, isImageName, timeLabel } from '../format'
import type { AppStatus, Entry, Location, PublicLink, View } from '../types'
import type { Perform, PreviewItem } from '../ui'

type Modal = 'new' | 'rename' | 'move' | 'delete' | 'share' | 'link' | null
type Props = {
  location: Location
  status: AppStatus | null
  query: string
  desktop: boolean
  busy: boolean
  dataVersion: number
  perform: Perform
  announce: (message: string) => void
  setError: (message: string) => void
  openPreview: (item: PreviewItem) => void
  openConnect: () => void
  openVaultDialog: (kind: 'create' | 'unlock') => void
  localOnly: boolean
  workLocally: () => void
}
const VAULT_VIEWS: { id: View; label: string; icon: typeof Star }[] = [
  { id: 'files', label: 'All files', icon: ShieldCheck }, { id: 'favorites', label: 'Favorites', icon: Star },
  { id: 'offline', label: 'Available offline', icon: HardDrive }, { id: 'trash', label: 'Trash', icon: Trash2 },
]
export const VAULT_SHARE_NOTE = "Vault files can't be shared: sharing would let pCloud read them. To share a Vault file, export a copy and upload it to pCloud Files."

/** The file browser used for both the encrypted Vault and plain pCloud Files. */
export function FilesPage({ location, status, query, desktop, busy, dataVersion, perform, announce, setError, openPreview, openConnect, openVaultDialog, localOnly, workLocally }: Props) {
  const vault = location === 'vault'
  const [view, setView] = useState<View>('files')
  const [path, setPath] = useState<Entry[]>([])
  const [entries, setEntries] = useState<Entry[]>([])
  const [selected, setSelected] = useState<string[]>([])
  const [current, setCurrent] = useState<Entry | null>(null)
  const [menu, setMenu] = useState<{ x: number; y: number; entry: Entry } | null>(null)
  const [modal, setModal] = useState<Modal>(null)
  const [formName, setFormName] = useState('')
  const [shareEmail, setShareEmail] = useState('')
  const [sharePermission, setSharePermission] = useState<'view' | 'edit'>('view')
  const [link, setLink] = useState<PublicLink | null>(null)
  const [grid, setGrid] = useState(false)
  const entriesRequest = useRef(0)
  const fileInput = useRef<HTMLInputElement>(null)
  const parentId = path.at(-1)?.id ?? null
  const unlocked = status?.unlocked ?? false
  const available = !!status && (vault ? status.vaultExists && unlocked : status.connected)

  const refresh = useCallback(async () => {
    if (!available) { entriesRequest.current++; setEntries([]); return }
    const request = ++entriesRequest.current
    try { const result = await listEntries(location, parentId, query || null, view); if (request === entriesRequest.current) setEntries(Array.isArray(result) ? result : []) }
    catch (reason) { if (request === entriesRequest.current) setError(errorMessage(reason, 'Could not load files.')) }
  }, [available, location, parentId, query, view, setError])
  useEffect(() => { void refresh(); const id = window.setInterval(() => { void refresh() }, 5000); return () => { window.clearInterval(id); entriesRequest.current++ } }, [refresh, dataVersion])
  // Locking hides every vault name at once, including the folder path and selection.
  useEffect(() => { if (vault && !unlocked) { entriesRequest.current++; setEntries([]); setPath([]); setSelected([]); setCurrent(null); setModal(null); setMenu(null) } }, [vault, unlocked])
  useDesktopDrop(paths => { void perform(() => importPaths(paths, parentId), 'Files added to import queue') }, vault && unlocked)

  const reset = () => { entriesRequest.current++; setEntries([]); setSelected([]) }
  const switchView = (next: View) => { reset(); setView(next); setPath([]) }
  const goToPath = (index: number) => { reset(); setPath(path.slice(0, index + 1)) }
  const goToRoot = () => { reset(); setPath([]) }
  const goUp = () => { reset(); setPath(path.slice(0, -1)) }
  const openEntry = (entry: Entry) => {
    if (entry.kind === 'folder') { reset(); setPath(items => [...items, entry]) }
    else openPreview({ location, id: entry.id, name: entry.name, size: entry.size, modified: entry.modified })
  }
  const selectedEntry = entries.find(item => selected.includes(item.id)) ?? null
  // Drop selected items that a refresh or search no longer shows, so actions match the bar.
  useEffect(() => { setSelected(value => { const kept = value.filter(id => entries.some(item => item.id === id)); return kept.length === value.length ? value : kept }) }, [entries])
  // The selection drives the selection bar; `current` only pins the item an open dialog acts on.
  const activeEntry = current ?? selectedEntry
  // Items an open move/trash dialog acts on: the pinned item, or every selected item.
  const targets = current ? [current] : entries.filter(item => selected.includes(item.id))
  const selectEntry = (entry: Entry, event: React.MouseEvent) => {
    setCurrent(null)
    if (vault && (event.ctrlKey || event.metaKey)) setSelected(value => value.includes(entry.id) ? value.filter(id => id !== entry.id) : [...value, entry.id])
    else setSelected(value => value.length === 1 && value[0] === entry.id ? [] : [entry.id])
  }
  const act = async (task: () => Promise<unknown>, success: string) => { const done = await perform(task, success); if (done) setSelected([]); return done }
  const exportEntry = async (entry: Entry) => {
    if (!desktop) { announce('Export copies are available in the desktop app'); return }
    await act(() => runAction('download_entry', { location, id: entry.id }), 'Export complete')
  }
  const getLink = async (entry: Entry) => {
    const result: { link?: PublicLink } = {}
    if (await perform(async () => { result.link = await runExtra('create_link', { id: entry.id }) }, 'Link created') && result.link) { setLink(result.link); setCurrent(entry); setModal('link') }
  }
  const renameAction = (entry: Entry): MenuItem => ({ label: 'Rename', run: () => { setCurrent(entry); setFormName(entry.name); setModal('rename') } })
  const moveAction = (entry: Entry, count = 1): MenuItem => ({ label: 'Move', run: () => { setCurrent(count === 1 ? entry : null); setFormName(''); setModal('move') } })
  const trashAction = (entry: Entry, count: number, label: string): MenuItem => ({ label, icon: <Trash2 size={15} />, danger: true, run: () => { setCurrent(count === 1 ? entry : null); setModal('delete') } })
  /** The items an action applies to: the clicked item, or every selected item. */
  const itemsFor = (entry: Entry, count: number) => count === 1 ? [entry] : entries.filter(item => selected.includes(item.id))
  /** Runs a per-item command and reports once. */
  const forEach = (items: Entry[], command: (item: Entry) => Promise<unknown>, success: string) => void act(async () => {
    for (const item of items) await command(item)
  }, success)
  /** Favorite and Offline set one state for all items (they toggle per item, so skip those already there). */
  const setFlag = (items: Entry[], flag: 'starred' | 'pinned', command: 'toggle_favorite' | 'toggle_pin', success: string) => {
    const target = !items.every(item => item[flag])
    forEach(items.filter(item => item[flag] !== target), item => runAction(command, { id: item.id }), success)
  }
  /** One action list for the selection bar and the right-click menu, so both always match. */
  const entryActions = (entry: Entry | null, count: number): MenuItem[] => {
    if (!entry) return []
    const items = itemsFor(entry, count)
    const allStarred = items.every(item => item.starred)
    const allPinned = items.every(item => item.pinned)
    const exportCopy: MenuItem[] = count === 1 ? [{ label: 'Export copy', icon: <ArrowDownToLine size={15} />, run: () => void exportEntry(entry) }] : []
    if (!vault) return [
      ...exportCopy,
      ...(count === 1 ? [{ label: 'Get link', icon: <Link2 size={15} />, run: () => void getLink(entry) }] : []),
      ...(count === 1 && entry.kind === 'folder' ? [{ label: 'Share folder', icon: <Share2 size={15} />, run: () => { setCurrent(entry); setShareEmail(''); setSharePermission('view'); setModal('share') } }] : []),
      ...(count === 1 && entry.kind === 'file' ? [{ label: 'Copy into Vault', icon: <LockKeyhole size={15} />, run: () => void act(() => runAction('import_cloud_file', { id: entry.id, parentId: null }), 'Copied into your Vault, encrypted. The original stays in pCloud.') }] : []),
      ...(count === 1 ? [renameAction(entry)] : []),
      moveAction(entry, count),
      trashAction(entry, count, 'Move to pCloud trash'),
    ]
    if (view === 'trash') return [{ label: 'Restore', run: () => forEach(items, item => runAction('restore_entry', { location, id: item.id }), items.length === 1 ? 'Item restored' : `${items.length} items restored`) }]
    return [
      ...exportCopy,
      ...(count === 1 ? [renameAction(entry)] : []),
      moveAction(entry, count),
      { label: allStarred ? 'Remove from favorites' : 'Add to favorites', icon: <Star size={15} />, run: () => setFlag(items, 'starred', 'toggle_favorite', allStarred ? 'Removed from favorites' : 'Added to favorites') },
      { label: allPinned ? 'Remove offline copy' : 'Keep offline', icon: <HardDrive size={15} />, run: () => setFlag(items, 'pinned', 'toggle_pin', allPinned ? 'Offline copy removed' : 'Kept offline') },
      trashAction(entry, count, 'Move to trash'),
    ]
  }
  const menuItems = (entry: Entry): MenuItem[] => {
    const open: MenuItem = { label: entry.kind === 'folder' ? 'Open' : 'Preview', icon: entry.kind === 'folder' ? <Folder size={15} /> : <Eye size={15} />, run: () => openEntry(entry) }
    // Right-clicking an item of a multi-selection acts on the whole selection, like the bar.
    const count = selected.length > 1 && selected.includes(entry.id) ? selected.length : 1
    return [...(view === 'trash' || count > 1 ? [] : [open]), ...entryActions(entry, count)]
  }
  const openMenu = (entry: Entry, x: number, y: number) => { setCurrent(null); setSelected(value => value.includes(entry.id) ? value : [entry.id]); setMenu({ x, y, entry }) }
  const closeMenu = useCallback(() => setMenu(null), [])
  const closeModal = useCallback(() => { if (!busy) { setModal(null); setCurrent(null); setLink(null) } }, [busy])
  const doImport = async (files: FileList | null) => {
    if (!files?.length || !vault) return
    const list = Array.from(files)
    await perform(() => importDemoFiles(list, parentId), `${list.length} file${list.length > 1 ? 's' : ''} added to demo`)
    if (fileInput.current) fileInput.current.value = ''
  }
  const browseFiles = () => {
    if (!vault || busy) return
    if (!desktop) { fileInput.current?.click(); return }
    void perform(() => runAction('import_files', { parentId }), 'Import started')
  }
  const onSubmit = async (event: FormEvent<HTMLFormElement>) => {
    event.preventDefault()
    const target = activeEntry
    if (modal === 'new') await perform(() => runAction('new_folder', { location, parentId, name: formName.trim() }), 'Folder created')
    if (modal === 'rename' && target) await act(() => runAction('rename_entry', { location, id: target.id, name: formName.trim() }), 'Name updated')
    const items = targets
    if (modal === 'move' && items.length) await act(async () => { for (const item of items) await runAction('move_entry', { location, id: item.id, parentId: formName || null }) }, items.length === 1 ? 'Item moved' : `${items.length} items moved`)
    if (modal === 'delete' && items.length) await act(async () => { for (const item of items) await runAction('trash_entry', { location, id: item.id }) }, vault ? 'Moved to trash' : 'Moved to pCloud trash')
    if (modal === 'share' && target) await perform(() => runExtra('share_folder', { id: target.id, email: shareEmail.trim(), permissions: sharePermission }), 'Invitation sent')
    setFormName(''); setModal(null); setCurrent(null)
  }

  const header = <PageHeader title={vault ? 'Vault' : 'pCloud Files'} badge={<ProtectionBadge location={location} />}
    description={vault ? 'Encrypted on this computer before anything is uploaded. Only you can read these files.' : 'Your regular pCloud files. They are not encrypted: pCloud can read them.'}
    actions={available ? <button type="button" className="button button-primary" onClick={() => { setFormName(''); setModal('new') }}><Plus size={16} /> New</button> : undefined} />
  if (!status) return <div className="page">{header}<LoadingGate /></div>
  if (!vault && !status.connected) return <div className="page">{header}<CloudGate connect={openConnect} /></div>
  if (vault && status.vaultExists && !unlocked) return <div className="page">{header}<UnlockGate unlock={() => openVaultDialog('unlock')} /></div>
  if (vault && !status.vaultExists && !localOnly) return <div className="page">{header}<SetupGate connected={status.connected} connect={openConnect} create={() => openVaultDialog('create')} workLocally={workLocally} /></div>

  const folder = path.at(-1) ?? null
  const allFolders = entries.filter(item => item.kind === 'folder')
  return <div className="page files-page" onDragOver={event => { if (!desktop && vault) event.preventDefault() }} onDrop={event => { if (!desktop && vault) { event.preventDefault(); void doImport(event.dataTransfer.files) } }}>
    {header}
    <div className={`location-band ${vault ? 'enc' : 'plain'}`}>{vault ? <ShieldCheck size={15} aria-hidden="true" /> : <Cloud size={15} aria-hidden="true" />}<span>{vault ? 'You are in the Vault. Everything here is encrypted.' : 'You are in pCloud Files. Nothing here is encrypted by this app.'}</span></div>
    {vault && <div className="segmented" role="group" aria-label="Vault views">{VAULT_VIEWS.map(item => { const Icon = item.icon; return <button key={item.id} type="button" aria-pressed={view === item.id} className={view === item.id ? 'active' : ''} onClick={() => switchView(item.id)}><Icon size={15} aria-hidden="true" />{item.label}</button> })}</div>}
    <div className="content-head">
      <div>
        {path.length > 0 && <div className="breadcrumb-list"><button className="breadcrumb-control" onClick={goUp} aria-label="Go up one folder"><ArrowLeft size={14} /></button><button className="breadcrumb-control" onClick={goToRoot}>Root</button>{path.map((item, index) => <span className="breadcrumb" key={`${item.id}-${index}`}><ChevronRight size={14} /><button className="breadcrumb-control" onClick={() => goToPath(index)}>{item.name}</button></span>)}</div>}
        <h3>{view === 'files' ? (folder ? folder.name : vault ? 'Your vault' : 'Your pCloud') : VAULT_VIEWS.find(item => item.id === view)?.label}</h3>
        <p>{view === 'trash' ? 'Items in the trash can be restored.' : `${entries.length} ${entries.length === 1 ? 'item' : 'items'}${query ? ` matching “${query}”` : ''}`}</p>
      </div>
      <div className="view-switch" aria-label="Display mode"><button className={!grid ? 'active' : ''} aria-label="List view" aria-pressed={!grid} onClick={() => setGrid(false)}><LayoutList size={16} /></button><button className={grid ? 'active' : ''} aria-label="Grid view" aria-pressed={grid} onClick={() => setGrid(true)}><Grid2X2 size={16} /></button></div>
    </div>
    {/* The slot always keeps its fixed height, so selecting never shifts the rows under the pointer (a double-click must open the row clicked). */}
    <div className="selection-slot" data-testid="selection-slot">
      {selected.length > 0 ? <div className="selection-bar"><span>{selected.length} selected</span>
        {entryActions(selectedEntry, selected.length).map(item => <button key={item.label} onClick={item.run} className={item.danger ? 'remove-action' : undefined}>{item.icon}{item.icon ? ' ' : ''}{item.label}</button>)}
        {vault && view !== 'trash' && <small className="selection-note" title={VAULT_SHARE_NOTE}><ShieldCheck size={13} aria-hidden="true" /> {VAULT_SHARE_NOTE}</small>}
        <button className="icon-button" onClick={() => setSelected([])} aria-label="Clear selection"><X size={16} /></button>
      </div> : entries.length > 0 && <p className="selection-hint">Click to select. Double-click to open.</p>}
    </div>
    {entries.length === 0 ? <EmptyState view={view} query={query} allowUpload={vault} create={() => { setFormName(''); setModal('new') }} upload={browseFiles} /> : <div className={grid ? 'files-grid' : 'files-table-wrap'}>
      {!grid && <div className="table-header"><span>Name</span><span>Last modified</span><span>Size</span><span aria-label="Actions" /></div>}
      {entries.map(entry => <FileRow key={entry.id} entry={entry} location={location} grid={grid} selected={selected.includes(entry.id)} onSelect={event => selectEntry(entry, event)} onOpen={() => openEntry(entry)} onMenu={(x, y) => openMenu(entry, x, y)} trash={view === 'trash'} />)}
    </div>}
    {vault ? <div className="drop-hint"><Upload size={15} /><span>Drop files here to add them to your vault.</span><button onClick={browseFiles} disabled={busy}>Browse files</button>{!desktop && <input ref={fileInput} type="file" multiple hidden onChange={event => void doImport(event.target.files)} />}</div>
      : <div className="drop-hint"><LockKeyhole size={15} /><span>Select a file to copy it into your Vault, encrypted. Use Upload to add new files.</span></div>}

    {menu && <ContextMenu x={menu.x} y={menu.y} label={`Actions for ${menu.entry.name}`} items={menuItems(menu.entry)} onClose={closeMenu} />}
    {modal === 'new' && <Dialog title="Create a folder" description={vault ? 'The folder name is encrypted too.' : 'This folder is created in pCloud, not encrypted.'} onClose={closeModal} onSubmit={onSubmit} submitLabel="Create folder" busy={busy}><label className="field-label">Folder name<input required maxLength={120} value={formName} onChange={event => setFormName(event.target.value)} placeholder="e.g. Home documents" /></label></Dialog>}
    {modal === 'rename' && activeEntry && <Dialog title="Rename item" description="The item will keep its current location." onClose={closeModal} onSubmit={onSubmit} submitLabel="Save name" busy={busy}><label className="field-label">New name<input required maxLength={180} value={formName} onChange={event => setFormName(event.target.value)} /></label></Dialog>}
    {modal === 'move' && targets.length > 0 && <Dialog title={targets.length === 1 ? 'Move item' : `Move ${targets.length} items`} description={targets.length === 1 ? `Choose a destination for “${targets[0].name}”.` : `Choose a destination for ${targets.length} items.`} onClose={closeModal} onSubmit={onSubmit} submitLabel="Move here" busy={busy}><label className="field-label">Destination<select value={formName} onChange={event => setFormName(event.target.value)}><option value="">Root folder</option>{allFolders.filter(item => !targets.some(target => target.id === item.id)).map(item => <option key={item.id} value={item.id}>{item.name}</option>)}</select></label></Dialog>}
    {modal === 'delete' && targets.length > 0 && <Dialog title={vault ? 'Move to trash?' : 'Move to pCloud trash?'} description={vault ? (targets.length === 1 ? `“${targets[0].name}” can be restored from the trash later.` : `These ${targets.length} items can be restored from the trash later.`) : "Moves it to pCloud's trash. Restore it from pCloud's trash on the website."} onClose={closeModal} onSubmit={onSubmit} submitLabel={vault ? 'Move to trash' : 'Move to pCloud trash'} busy={busy} danger>{targets.map(item => <p key={item.id} className="confirm-item"><File size={18} />{item.name}</p>)}</Dialog>}
    {modal === 'share' && activeEntry && <Dialog title={`Share “${activeEntry.name}”`} description="The people you invite can open this folder. It is not encrypted, so pCloud can read it too." onClose={closeModal} onSubmit={onSubmit} submitLabel="Send invitation" busy={busy}>
      <label className="field-label">Email address<input required type="email" value={shareEmail} onChange={event => setShareEmail(event.target.value)} placeholder="name@example.com" /></label>
      <label className="field-label">They can<select value={sharePermission} onChange={event => setSharePermission(event.target.value as 'view' | 'edit')}><option value="view">View and download</option><option value="edit">View, add and change files</option></select></label>
      <p className="privacy-note"><ProtectionBadge location="cloud" /> Shared folders live in pCloud Files.</p>
    </Dialog>}
    {modal === 'link' && link && <Dialog title="Link ready" description={`Anyone with this link can open “${link.name}”. It is not encrypted.`} onClose={closeModal}>
      <div className="link-box"><code>{link.url}</code><button type="button" className="button button-subtle" onClick={() => void copyText(link.url).then(ok => announce(ok ? 'Link copied' : 'Could not copy the link'))}><Copy size={14} /> Copy link</button></div>
      <p className="hint">Manage your links under Shared.</p>
    </Dialog>}
  </div>
}

function FileRow({ entry, location, grid, selected, onSelect, onOpen, onMenu, trash }: { entry: Entry; location: Location; grid: boolean; selected: boolean; onSelect: (event: React.MouseEvent) => void; onOpen: () => void; onMenu: (x: number, y: number) => void; trash: boolean }) {
  const isPhoto = isImageName(entry.name)
  const FileIcon = entry.kind === 'folder' ? Folder : isPhoto ? Image : /\.(pdf|docx?|xlsx?|md)$/i.test(entry.name) ? FileText : File
  return <div className={`${grid ? 'file-card' : 'file-row'} ${selected ? 'chosen' : ''}`} onClick={onSelect} onDoubleClick={onOpen} onContextMenu={event => { event.preventDefault(); onMenu(event.clientX, event.clientY) }} role="button" tabIndex={0} aria-pressed={selected} aria-haspopup="menu" onKeyDown={event => { if (event.key === 'ContextMenu' || (event.shiftKey && event.key === 'F10')) { event.preventDefault(); const box = event.currentTarget.getBoundingClientRect(); onMenu(box.left + 24, box.top + box.height / 2); return } if (event.key === 'Enter') onOpen(); if (event.key === ' ') { event.preventDefault(); onSelect(event as unknown as React.MouseEvent) } }}>
    <div className="file-main"><div className={`file-icon ${entry.kind === 'folder' ? 'folder-icon' : isPhoto ? 'photo-icon' : 'doc-icon'}`}><FileIcon size={18} fill={entry.kind === 'folder' ? 'currentColor' : 'none'} /></div><div className="file-title"><strong>{entry.name}</strong><small>{entry.kind === 'folder' ? 'Folder' : entry.syncState === 'synced' ? 'In sync' : entry.syncState === 'local' ? 'On this computer' : 'Syncing'}</small></div><div className="file-badges"><ProtectionBadge location={location} compact />{entry.starred && <Star size={13} fill="currentColor" aria-label="Favorite" />}{entry.pinned && <Wifi size={13} aria-label="Available offline" />}</div></div>
    <span className="modified">{timeLabel(entry.modified)}</span><span className="file-size">{entry.kind === 'folder' ? '—' : formatSize(entry.size)}</span><div className="row-actions">{selected && <Check size={15} aria-hidden="true" />}</div>
    {grid && <div className="card-meta"><span>{entry.kind === 'folder' ? 'Folder' : formatSize(entry.size)}</span><span>{timeLabel(entry.modified)}</span></div>}
    {trash && <span className="trash-label">In trash</span>}
  </div>
}

function EmptyState({ view, query, create, upload, allowUpload }: { view: View; query: string; create: () => void; upload: () => void; allowUpload: boolean }) {
  const emptyCopy = query ? ['No matches found', 'Try another search, or clear the search field to see your files.'] : view === 'favorites' ? ['A little space for the important things', 'Mark any file as a favorite and it will be easy to find here.'] : view === 'offline' ? ['Ready when you are', 'Files you mark for offline access will be available on this computer.'] : view === 'trash' ? ['All clear', 'Items you move to the trash will appear here.'] : ['Start with a folder', 'A good place for a new project, a trip, or anything you want to keep.']
  return <div className="empty-state"><img src="/art-empty.svg" alt="" /><h3>{emptyCopy[0]}</h3><p>{emptyCopy[1]}</p>{!query && view === 'files' && <div className="empty-actions"><button className="button button-primary" onClick={create}><FolderPlus size={16} /> Create a folder</button>{allowUpload && <button className="button button-subtle" onClick={upload}><Upload size={16} /> Add files</button>}</div>}</div>
}
