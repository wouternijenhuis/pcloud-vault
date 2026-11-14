import { useCallback, useEffect, useRef, useState, type FormEvent } from 'react'
import {
  Activity as ActivityIcon, ArrowDownToLine, ArrowLeft, ArrowRight, Check, ChevronRight, Cloud,
  File, FileImage, FileText, Folder, FolderPlus, Globe, Grid2X2, HardDrive, Home, Image,
  LayoutList, LockKeyhole, Moon, Plus, RefreshCw, Search, Settings as SettingsIcon,
  ShieldCheck, Star, Sun, Trash2, Upload, Wifi, X,
} from 'lucide-react'
import { getCurrentWebviewWindow } from '@tauri-apps/api/webviewWindow'
import { ContextMenu, type MenuItem } from './components/ContextMenu'
import { Dialog } from './components/Dialog'
import { getActivity, getStatus, importDemoFiles, importPaths, isDesktop, listEntries, previewEntry, runAction } from './api'
import type { ActionArguments, Activity, AppStatus, Entry, Location, View } from './types'

type Modal = 'new' | 'rename' | 'move' | 'password' | 'connect' | 'preview' | 'delete' | 'unlock' | 'create' | null
const nav: { id: View; label: string; icon: typeof Home }[] = [
  { id: 'files', label: 'My files', icon: Home }, { id: 'favorites', label: 'Favorites', icon: Star },
  { id: 'offline', label: 'Available offline', icon: HardDrive }, { id: 'activity', label: 'Activity', icon: ActivityIcon },
  { id: 'trash', label: 'Trash', icon: Trash2 },
]
const formatSize = (bytes: number) => bytes < 1024 ? `${bytes} B` : bytes < 1024 ** 2 ? `${(bytes / 1024).toFixed(0)} KB` : `${(bytes / 1024 ** 2).toFixed(1)} MB`
const timeLabel = (time: number) => new Intl.DateTimeFormat(undefined, { month: 'short', day: 'numeric' }).format(new Date(time))

/** Tauri commands reject with the backend's message as a plain string, not an Error. */
function errorMessage(reason: unknown, fallback: string) {
  if (reason instanceof Error && reason.message) return reason.message
  if (typeof reason === 'string' && reason.trim()) return reason
  return fallback
}

export default function App() {
  const desktop = isDesktop()
  const [status, setStatus] = useState<AppStatus | null>(null)
  const [view, setView] = useState<View>('files')
  const [location, setLocation] = useState<Location>('vault')
  const [parentId, setParentId] = useState<string | null>(null)
  const [path, setPath] = useState<Entry[]>([])
  const [query, setQuery] = useState('')
  const [entries, setEntries] = useState<Entry[]>([])
  const [activity, setActivity] = useState<Activity[]>([])
  const activityRequest = useRef(0)
  const [modal, setModal] = useState<Modal>(null)
  const [selected, setSelected] = useState<string[]>([])
  const [current, setCurrent] = useState<Entry | null>(null)
  const [menu, setMenu] = useState<{ x: number; y: number; entry: Entry } | null>(null)
  const [busy, setBusy] = useState(false)
  const [error, setError] = useState('')
  const [toast, setToast] = useState('')
  const [grid, setGrid] = useState(false)
  const [dark, setDark] = useState(false)
  const [localOnly, setLocalOnly] = useState(false)
  const [preview, setPreview] = useState<string | null>(null)
  const [settings, setSettings] = useState({ cacheLimitGib: 10, clientId: '' })
  const [settingsInitialized, setSettingsInitialized] = useState(false)
  const statusRequest = useRef(0)
  const entriesRequest = useRef(0)
  const previewRequest = useRef(0)
  const statusRef = useRef<AppStatus | null>(null)
  const fileInput = useRef<HTMLInputElement>(null)
  const searchInput = useRef<HTMLInputElement>(null)
  const [formName, setFormName] = useState('')
  const [password, setPassword] = useState('')
  const [remember, setRemember] = useState(true)

  const applyStatus = useCallback((value: AppStatus) => {
    if (statusRef.current?.unlocked && !value.unlocked) {
      previewRequest.current++
      entriesRequest.current++
      setEntries([]); setSelected([]); setCurrent(null); setModal(null); setPreview(null)
      setFormName(''); setPassword('')
    }
    statusRef.current = value
    setStatus(value)
  }, [])
  const refreshStatus = useCallback(async () => {
    const request = ++statusRequest.current
    try { const value = await getStatus(); if (request === statusRequest.current) applyStatus(value) }
    catch (reason) { if (request === statusRequest.current) setError(errorMessage(reason, 'Could not read vault status.')) }
  }, [applyStatus])
  const refreshEntries = useCallback(async () => {
    if (view === 'settings' || view === 'activity') { entriesRequest.current++; setEntries([]); return }
    const available = status && (location === 'vault' ? status.vaultExists && status.unlocked : status.connected)
    if (!available) { entriesRequest.current++; setEntries([]); return }
    const request = ++entriesRequest.current
    try { const result = await listEntries(location, parentId, query || null, view); if (request === entriesRequest.current) setEntries(result) }
    catch (reason) { if (request === entriesRequest.current) setError(errorMessage(reason, 'Could not load files.')) }
  }, [location, parentId, query, status, view])
  useEffect(() => { void refreshStatus(); const id = window.setInterval(() => { void refreshStatus() }, 5000); return () => { window.clearInterval(id); statusRequest.current++ } }, [refreshStatus])
  useEffect(() => () => { previewRequest.current++ }, [])
  useEffect(() => { void refreshEntries(); const id = window.setInterval(() => { void refreshEntries() }, 5000); return () => { window.clearInterval(id); entriesRequest.current++ } }, [refreshEntries])
  useEffect(() => {
    if (view !== 'activity') { activityRequest.current++; return }
    const request = ++activityRequest.current
    getActivity().then(value => { if (request === activityRequest.current) setActivity(value) }).catch(e => { if (request === activityRequest.current) setError(String(e)) })
    return () => { activityRequest.current++ }
  }, [view])
  useEffect(() => { if (status && !settingsInitialized) { setSettings({ cacheLimitGib: Math.round(status.cacheLimitBytes / 1024 ** 3), clientId: status.oauthClientId }); setSettingsInitialized(true) } }, [status, settingsInitialized])
  useEffect(() => {
    const shortcut = (event: KeyboardEvent) => { if ((event.ctrlKey || event.metaKey) && event.key.toLowerCase() === 'k') { event.preventDefault(); searchInput.current?.focus() } }
    window.addEventListener('keydown', shortcut)
    return () => window.removeEventListener('keydown', shortcut)
  }, [])
  useEffect(() => {
    if (!desktop) return
    let unlisten: (() => void) | undefined
    let cancelled = false
    getCurrentWebviewWindow().onDragDropEvent(event => {
      if (event.payload.type === 'drop' && location === 'vault' && status?.unlocked) void importPaths(event.payload.paths, parentId).then(() => { announce('Files added to import queue'); void refreshEntries() }).catch(e => setError(String(e)))
    }).then(fn => { if (cancelled) fn(); else unlisten = fn }).catch(() => undefined)
    return () => { cancelled = true; unlisten?.() }
  }, [desktop, location, parentId, refreshEntries, status?.unlocked])

  const announce = (message: string) => { setToast(message); window.setTimeout(() => setToast(''), 2800) }
  const perform = async (command: keyof ActionArguments, args: ActionArguments[keyof ActionArguments] = {} as ActionArguments[keyof ActionArguments], success?: string) => {
    if (busy) return false
    setBusy(true); setError('')
    try {
      const result = await runAction(command, args)
      if (result && typeof result === 'object' && 'unlocked' in result) applyStatus(result as AppStatus)
      if (command === 'lock_vault' || command === 'forget_device') { entriesRequest.current++; setEntries([]); setParentId(null); setPath([]); setSelected([]) }
      await refreshStatus()
      if (command !== 'lock_vault' && command !== 'forget_device') await refreshEntries()
      if (success) announce(success)
      setSelected([])
      return true
    }
    catch (reason) { setError(errorMessage(reason, 'That action could not be completed.')); return false }
    finally { setBusy(false) }
  }
  const folder = path.at(-1) ?? null
  const filteredEntries = entries
  const allFolders = entries.filter(item => item.kind === 'folder')
  // The selection drives the selection bar; `current` only pins the item an open dialog acts on.
  const selectedEntry = entries.find(item => item.id === selected[0]) ?? null
  const activeEntry = current ?? selectedEntry
  const openFolder = (entry: Entry) => { if (entry.kind === 'folder') { entriesRequest.current++; setPath(items => [...items, entry]); setParentId(entry.id); setEntries([]); setSelected([]) } else void showPreview(entry) }
  const showPreview = async (entry: Entry) => {
    const request = ++previewRequest.current
    setCurrent(entry); setPreview(null); setModal('preview')
    if (/\.(png|jpe?g|gif|webp)$/i.test(entry.name)) {
      try { const url = await previewEntry(location, entry.id); if (request === previewRequest.current) setPreview(url) }
      catch { if (request === previewRequest.current) setPreview(null) }
    }
  }
  const selectEntry = (entry: Entry, event: React.MouseEvent) => { setCurrent(null); if (location === 'vault' && (event.ctrlKey || event.metaKey)) setSelected(value => value.includes(entry.id) ? value.filter(id => id !== entry.id) : [...value, entry.id]); else setSelected(value => value.length === 1 && value[0] === entry.id ? [] : [entry.id]) }
  /** One action list for the selection bar and the right-click menu, so both always match. */
  const entryActions = (entry: Entry | null, count: number): MenuItem[] => {
    if (!entry) return []
    const exportCopy: MenuItem[] = count === 1 ? [{ label: 'Export copy', icon: <ArrowDownToLine size={15} />, run: () => void exportEntry(entry) }] : []
    if (location === 'cloud') return [...exportCopy, ...(count === 1 && entry.kind === 'file' ? [{ label: 'Import to vault', icon: <LockKeyhole size={15} />, run: () => void perform('import_cloud_file', { id: entry.id, parentId: null }, 'File imported to Private Vault') }] : [])]
    if (view === 'trash') return [{ label: 'Restore', run: () => void perform('restore_entry', { location, id: entry.id }, 'Item restored') }]
    return [
      ...exportCopy,
      { label: 'Rename', run: () => { setCurrent(entry); setFormName(entry.name); setModal('rename') } },
      { label: 'Move', run: () => { setCurrent(entry); setFormName(''); setModal('move') } },
      { label: 'Favorite', icon: <Star size={15} />, run: () => void perform('toggle_favorite', { id: entry.id }, 'Favorite updated') },
      { label: 'Offline', icon: <HardDrive size={15} />, run: () => void perform('toggle_pin', { id: entry.id }, 'Offline access updated') },
      { label: 'Move to trash', icon: <Trash2 size={15} />, danger: true, run: () => { setCurrent(entry); setModal('delete') } },
    ]
  }
  const menuItems = (entry: Entry): MenuItem[] => {
    const open: MenuItem = { label: entry.kind === 'folder' ? 'Open' : 'Preview', icon: entry.kind === 'folder' ? <Folder size={15} /> : <Image size={15} />, run: () => openFolder(entry) }
    const labels: Record<string, string> = {
      Favorite: entry.starred ? 'Remove from favorites' : 'Add to favorites',
      Offline: entry.pinned ? 'Remove offline copy' : 'Keep offline',
    }
    return [...(view === 'trash' ? [] : [open]), ...entryActions(entry, 1).map(item => ({ ...item, label: labels[item.label] ?? item.label }))]
  }
  const openMenu = (entry: Entry, x: number, y: number) => { setCurrent(null); setSelected([entry.id]); setMenu({ x, y, entry }) }
  const closeMenu = useCallback(() => setMenu(null), [])
  useEffect(() => {
    // The WebView's own menu (Back, Refresh, Print…) does not belong in a desktop app; keep it for text fields.
    const suppress = (event: MouseEvent) => { if (!(event.target as HTMLElement).closest('input, textarea, [contenteditable="true"]')) event.preventDefault() }
    document.addEventListener('contextmenu', suppress)
    return () => document.removeEventListener('contextmenu', suppress)
  }, [])
  const doImport = async (files: FileList | null) => { if (!files?.length || location !== 'vault') return; setBusy(true); try { await importDemoFiles(Array.from(files), parentId); announce(`${files.length} file${files.length > 1 ? 's' : ''} added to demo`); await refreshEntries() } catch (e) { setError(String(e)) } finally { setBusy(false); if (fileInput.current) fileInput.current.value = '' } }
  const browseFiles = async () => {
    if (location !== 'vault' || busy) return
    if (!desktop) { fileInput.current?.click(); return }
    setBusy(true); setError('')
    try { await runAction('import_files', { parentId }); announce('Import started'); await refreshEntries() }
    catch (reason) { setError(errorMessage(reason, 'Could not open the file picker.')) }
    finally { setBusy(false) }
  }
  const exportEntry = async (entry: Entry) => {
    if (!desktop) { announce('Export copies are available in the Windows app'); return }
    await perform('download_entry', { location, id: entry.id }, 'Export complete')
  }
  const exportRecovery = () => { if (desktop) void perform('export_recovery', {}, 'Recovery key exported'); else announce('Recovery key export is available in the Windows app') }
  const onSubmit = async (event: FormEvent<HTMLFormElement>) => {
    event.preventDefault()
    const target = activeEntry
    if (modal === 'new') { await perform('new_folder', { location, parentId, name: formName.trim() }, 'Folder created') }
    if (modal === 'rename' && target) await perform('rename_entry', { location, id: target.id, name: formName.trim() }, 'Name updated')
    if (modal === 'move' && target) await perform('move_entry', { location, id: target.id, parentId: formName || null }, 'Item moved')
    if (modal === 'create') await perform('create_vault', { password, remember }, 'Your private vault is ready')
    if (modal === 'unlock') await perform('unlock_vault', { password, remember }, 'Vault unlocked')
    if (modal === 'password') await perform('change_password', { oldPassword: formName, newPassword: password }, 'Password changed')
    if (modal === 'connect') await perform('connect_pcloud', { clientId: settings.clientId }, 'pCloud connected')
    setPassword(''); setFormName(''); setModal(null); setCurrent(null)
  }
  const connectWeb = async () => {
    if (!desktop) { announce('Website sign-in is available in the Windows app'); return }
    if (await perform('connect_pcloud_web', {}, 'pCloud connected')) setModal(null)
  }
  const modalClose = useCallback(() => { if (!busy) { setModal(null); setCurrent(null) } }, [busy])
  const moveNavigation = (item: View) => { entriesRequest.current++; setView(item); if (item === 'favorites' || item === 'offline' || item === 'trash') setLocation('vault'); setParentId(null); setPath([]); setEntries([]); setSelected([]); if (item !== 'settings') setQuery('') }
  const switchWorkspace = (next: Location) => { entriesRequest.current++; setLocation(next); setView('files'); setParentId(null); setPath([]); setEntries([]); setSelected([]); setQuery('') }
  const goToRoot = () => { entriesRequest.current++; setParentId(null); setPath([]); setEntries([]); setSelected([]) }
  const goUp = () => { entriesRequest.current++; const nextPath = path.slice(0, -1); setPath(nextPath); setParentId(nextPath.at(-1)?.id ?? null); setEntries([]); setSelected([]) }
  const goToPath = (index: number) => { entriesRequest.current++; const nextPath = path.slice(0, index + 1); setPath(nextPath); setParentId(nextPath.at(-1)?.id ?? null); setEntries([]); setSelected([]) }
  const title = view === 'files' ? (location === 'vault' ? 'Private Vault' : 'pCloud Files') : nav.find(item => item.id === view)?.label ?? 'Settings'
  const greeting = location === 'cloud' && view === 'files' ? 'Your cloud files, ready when you are.' : 'Your files, quietly in their place.'
  const unlocked = status?.unlocked ?? false
  const vaultLibraryView = view === 'files' || view === 'favorites' || view === 'offline' || view === 'trash'

  return <div className={`app-shell ${dark ? 'dark' : ''}`}>
    <aside className="sidebar">
      <div className="brand"><div className="brand-mark"><ShieldCheck size={19} strokeWidth={2.2} /></div><div><strong>pCloud <span>Vault</span></strong><small>YOUR FILES, YOURS</small></div><button className="icon-button theme-toggle" aria-label={dark ? 'Use light appearance' : 'Use dark appearance'} onClick={() => setDark(!dark)}>{dark ? <Sun size={17} /> : <Moon size={17} />}</button></div>
      <div className="sidebar-label">WORKSPACES</div>
      <button className={`workspace-link ${location === 'vault' && view === 'files' ? 'selected' : ''}`} onClick={() => switchWorkspace('vault')}><LockKeyhole size={17} /><span>Private Vault</span><i className="secure-dot" /></button>
      <button className={`workspace-link ${location === 'cloud' && view === 'files' ? 'selected' : ''}`} onClick={() => switchWorkspace('cloud')}><Cloud size={17} /><span>pCloud Files</span><span className="unencrypted">Unencrypted</span></button>
      <div className="sidebar-rule" />
      <div className="sidebar-label">LIBRARY</div>
      <nav aria-label="Library">{nav.map(item => { const Icon = item.icon; return <button key={item.id} className={`nav-link ${view === item.id ? 'selected' : ''}`} onClick={() => moveNavigation(item.id)}><Icon size={17} /><span>{item.label}</span></button> })}</nav>
      <div className="sidebar-bottom"><div className="storage-card"><div className="storage-title"><span>Local cache</span><span>{status ? `${(status.cacheBytes / 1024 ** 3).toFixed(1)} / ${Math.round(status.cacheLimitBytes / 1024 ** 3)} GB` : '—'}</span></div><div className="storage-track"><i style={{ width: `${status ? Math.min(100, status.cacheBytes / status.cacheLimitBytes * 100) : 0}%` }} /></div><small>Cached files stay encrypted on this PC.</small></div><button className={`nav-link ${view === 'settings' ? 'selected' : ''}`} onClick={() => moveNavigation('settings')}><SettingsIcon size={17} /><span>Settings</span></button><div className="account"><div className="avatar">{status?.accountEmail?.slice(0, 1).toUpperCase() ?? 'A'}</div><div className="account-info"><strong>{status?.accountEmail?.split('@')[0] ?? 'On this PC'}</strong><small>{status?.connected ? 'pCloud connected' : 'Not connected'}</small></div><button className="icon-button" title="Lock vault" onClick={() => void perform('lock_vault', {}, 'Vault locked')}><LockKeyhole size={15} /></button></div></div>
    </aside>

    <main className="main-area">
      {!desktop && <div className="demo-banner"><span className="demo-dot" /> Browser preview <span>DEMO MODE</span><small>Sample data only — no cloud changes are made.</small></div>}
      <header className="topbar"><div className="crumb-row"><span className="top-context">{location === 'vault' && view === 'files' ? 'PRIVATE SPACE' : location === 'cloud' && view === 'files' ? 'CLOUD STORAGE' : 'YOUR LIBRARY'}</span><span className="top-divider">/</span><h1>{title}</h1>{view === 'files' && path.length > 0 && <div className="breadcrumb-list"><button className="breadcrumb-control" onClick={goUp} aria-label="Go up one folder"><ArrowLeft size={14} /></button><button className="breadcrumb-control" onClick={goToRoot}>Root</button>{path.map((item, index) => <span className="breadcrumb" key={`${item.id}-${index}`}><ChevronRight size={14} /><button className="breadcrumb-control" onClick={() => goToPath(index)}>{item.name}</button></span>)}</div>}</div><div className="top-actions"><label className="search-box"><Search size={17} /><input ref={searchInput} aria-label="Search files" placeholder="Search your files" value={query} onChange={e => setQuery(e.target.value)} /><kbd>Ctrl K</kbd></label><button className="icon-button sync-button" title="Sync now" onClick={() => void perform('sync_now', {}, 'Everything is up to date')}><RefreshCw size={17} /></button><button className="button button-primary top-new" onClick={() => { setFormName(''); setModal('new') }}><Plus size={16} /> New</button></div></header>
      <div className="page-scroll" onDragOver={event => { if (!desktop && location === 'vault') event.preventDefault() }} onDrop={event => { if (!desktop && location === 'vault') { event.preventDefault(); void doImport(event.dataTransfer.files) } }}>
        {error && <div className="alert error-alert" role="alert"><span>{error}</span><button onClick={() => setError('')} aria-label="Dismiss error"><X size={16} /></button></div>}
        {view === 'settings' ? <SettingsPage status={status} settings={settings} setSettings={setSettings} save={() => void perform('save_settings', settings, 'Settings saved')} connect={() => { setModal('connect') }} recovery={exportRecovery} create={() => { if (status?.vaultExists && status.unlocked) void perform('lock_vault', {}, 'Vault locked'); else setModal(status?.vaultExists ? 'unlock' : 'create') }} password={() => setModal('password')} forget={() => void perform('forget_device', {}, 'This PC was forgotten')} desktop={desktop} /> : view === 'activity' ? <ActivityPage activities={activity} /> : !status ? <LoadingGate /> : <>
          {view === 'files' && location === 'cloud' && !status.connected ? <CloudGate connect={() => setModal('connect')} /> : vaultLibraryView && location === 'vault' && status.vaultExists && !unlocked ? <UnlockGate unlock={() => setModal('unlock')} /> : vaultLibraryView && location === 'vault' && !status.vaultExists && !localOnly ? <SetupGate connected={status.connected} connect={() => setModal('connect')} create={() => { setModal('create') }} workLocally={() => setLocalOnly(true)} /> : <>
          {view === 'files' && <div className="welcome-row"><div><p className="eyebrow">{location === 'vault' ? <><LockKeyhole size={13} /> ENCRYPTED ON THIS PC</> : <><Cloud size={13} /> CONNECTED STORAGE</>}</p><h2>{greeting}</h2><p>{location === 'vault' ? 'Anything you place here is protected with your vault password.' : 'Browse and manage your pCloud files from one calm place.'}</p></div><button className="text-action" onClick={() => setGrid(!grid)}>{grid ? <><LayoutList size={16} /> List view</> : <><Grid2X2 size={16} /> Grid view</>}</button></div>}
          <div className="content-head"><div><h2>{view === 'files' ? (parentId ? folder?.name ?? 'Folder' : location === 'vault' ? 'Your vault' : 'Your pCloud') : title}</h2><p>{view === 'trash' ? 'Items in the trash can be restored or removed forever.' : `${filteredEntries.length} ${filteredEntries.length === 1 ? 'item' : 'items'}${query ? ` matching “${query}”` : ''}`}</p></div><div className="view-switch" aria-label="Display mode"><button className={!grid ? 'active' : ''} aria-label="List view" onClick={() => setGrid(false)}><LayoutList size={16} /></button><button className={grid ? 'active' : ''} aria-label="Grid view" onClick={() => setGrid(true)}><Grid2X2 size={16} /></button></div></div>
          {selected.length > 0 && <div className="selection-bar"><span>{selected.length} selected</span>{entryActions(selectedEntry, selected.length).map(item => <button key={item.label} onClick={item.run} className={item.danger ? 'remove-action' : undefined}>{item.icon}{item.icon ? ' ' : ''}{item.label}</button>)}{location === 'cloud' && selectedEntry && selectedEntry.kind !== 'file' && <small>Open a file to import it into your vault.</small>}<button className="icon-button" onClick={() => setSelected([])} aria-label="Clear selection"><X size={16} /></button></div>}
          {filteredEntries.length === 0 ? <EmptyState view={view} query={query} allowUpload={location === 'vault'} create={() => { setFormName(''); setModal('new') }} upload={() => void browseFiles()} /> : <div className={grid ? 'files-grid' : 'files-table-wrap'}>
            {!grid && <div className="table-header"><span>Name</span><span>Last modified</span><span>Size</span><span aria-label="Actions" /></div>}
            {filteredEntries.map(entry => <FileRow key={entry.id} entry={entry} grid={grid} selected={selected.includes(entry.id)} onSelect={e => selectEntry(entry, e)} onOpen={() => openFolder(entry)} onMenu={(x, y) => openMenu(entry, x, y)} view={view} />)}
          </div>}
          {location === 'vault' ? <div className="drop-hint"><Upload size={15} /><span>Drop files here to add them to your vault.</span><button onClick={() => void browseFiles()} disabled={busy}>Browse files</button>{!desktop && <input ref={fileInput} type="file" multiple hidden onChange={e => void doImport(e.target.files)} />}</div> : <div className="drop-hint"><LockKeyhole size={15} /><span>Select a cloud file to import a copy into your private vault.</span></div>}
          </>}
        </>}
      </div>
      <footer className="statusbar"><div><span className="status-online" />{status?.connected ? 'pCloud connected' : 'Local only'}</div><div>{status?.connected && status.unlocked ? status.pendingCount ? `${status.pendingCount} changes waiting` : 'All changes synced' : 'Sync status unavailable'}<span className="status-sep">·</span>Last sync {status?.connected && status.unlocked && status.lastSync ? timeLabel(status.lastSync) : '—'}</div></footer>
    </main>

    {menu && <ContextMenu x={menu.x} y={menu.y} label={`Actions for ${menu.entry.name}`} items={menuItems(menu.entry)} onClose={closeMenu} />}
    {modal === 'new' && <Dialog title="Create a folder" description="Give this folder a name. You can move files into it anytime." onClose={modalClose} onSubmit={onSubmit} submitLabel="Create folder" busy={busy}><label className="field-label">Folder name<input required maxLength={120} value={formName} onChange={e => setFormName(e.target.value)} placeholder="e.g. Home documents" /></label></Dialog>}
    {modal === 'rename' && activeEntry && <Dialog title="Rename item" description="The item will keep its current location." onClose={modalClose} onSubmit={onSubmit} submitLabel="Save name" busy={busy}><label className="field-label">New name<input required maxLength={180} value={formName} onChange={e => setFormName(e.target.value)} /></label></Dialog>}
    {modal === 'move' && activeEntry && <Dialog title="Move item" description={`Choose a destination for “${activeEntry.name}”.`} onClose={modalClose} onSubmit={onSubmit} submitLabel="Move here" busy={busy}><label className="field-label">Destination<select value={formName} onChange={e => setFormName(e.target.value)}><option value="">Root folder</option>{allFolders.filter(item => item.id !== activeEntry.id).map(item => <option key={item.id} value={item.id}>{item.name}</option>)}</select></label></Dialog>}
    {modal === 'delete' && activeEntry && <Dialog title="Move to trash?" description={`“${activeEntry.name}” can be restored from the trash later.`} onClose={modalClose} onSubmit={e => { e.preventDefault(); void perform('trash_entry', { location, id: activeEntry.id }, 'Moved to trash'); setModal(null) }} submitLabel="Move to trash" busy={busy} danger><p className="confirm-item"><File size={18} />{activeEntry.name}</p></Dialog>}
    {modal === 'create' && <Dialog title="Create your private vault" description="Choose a strong password with at least 12 characters. Files in your vault stay encrypted on this PC." onClose={modalClose} onSubmit={onSubmit} submitLabel="Create vault" busy={busy}><label className="field-label">Vault password<input required type="password" minLength={12} value={password} onChange={e => setPassword(e.target.value)} autoComplete="new-password" placeholder="At least 12 characters" /></label><label className="check-label"><input type="checkbox" checked={remember} onChange={e => setRemember(e.target.checked)} />Remember this PC so you can unlock faster</label></Dialog>}
    {modal === 'unlock' && <Dialog title="Unlock your vault" description="Enter your vault password to continue." onClose={modalClose} onSubmit={onSubmit} submitLabel="Unlock vault" busy={busy}><label className="field-label">Vault password<input required type="password" minLength={12} value={password} onChange={e => setPassword(e.target.value)} autoComplete="current-password" /></label><label className="check-label"><input type="checkbox" checked={remember} onChange={e => setRemember(e.target.checked)} />Remember this PC</label></Dialog>}
    {modal === 'password' && <Dialog title="Change vault password" description="This wraps your existing vault key with the new password; it does not re-encrypt every file. Trusted PCs that already remember this vault keep their key." onClose={modalClose} onSubmit={onSubmit} submitLabel="Update password" busy={busy}><label className="field-label">Current password<input required type="password" minLength={12} value={formName} onChange={e => setFormName(e.target.value)} /></label><label className="field-label">New password<input required type="password" minLength={12} value={password} onChange={e => setPassword(e.target.value)} placeholder="At least 12 characters" /></label></Dialog>}
    {modal === 'connect' && <Dialog title="Connect to pCloud" description="Sign in on pCloud's own website. Your password and two-step verification stay on pCloud's page." onClose={modalClose} onSubmit={onSubmit} busy={busy}>{error && <p className="dialog-error" role="alert">{error}</p>}<button type="button" className="button button-primary connect-web" onClick={() => void connectWeb()} disabled={busy}><Globe size={15} /> {busy ? 'Waiting for pCloud sign-in…' : 'Sign in on pCloud website'}</button><p className="privacy-note"><ShieldCheck size={15} /> A separate private window opens pCloud's sign-in page. This app keeps only the resulting session token, protected by Windows, and ends that session when you forget this PC.</p><details className="connect-alternative"><summary>Use a registered pCloud app instead</summary><label className="field-label">OAuth client ID<input required value={settings.clientId} onChange={e => setSettings({ ...settings, clientId: e.target.value })} placeholder="Paste your pCloud app client ID" /></label><p className="privacy-note"><ShieldCheck size={15} /> Register a pCloud app with full account access and callback URL <code>http://127.0.0.1:43187/callback</code>. Your browser opens pCloud's sign-in; this app only receives an access token.</p><button type="submit" className="button button-subtle" disabled={busy}>Continue with OAuth app</button></details></Dialog>}
    {modal === 'preview' && activeEntry && <Dialog title={activeEntry.name} description={`${formatSize(activeEntry.size)} · ${timeLabel(activeEntry.modified)}`} onClose={modalClose}><div className="preview-area">{preview ? <img src={preview} alt={activeEntry.name} /> : <div className="preview-placeholder"><FileImage size={38} /><span>Preview is available in the desktop app.</span></div>}</div><p className="export-warning">Exported copies are decrypted and saved outside your managed vault.</p><div className="preview-actions"><button className="button button-subtle" onClick={() => void exportEntry(activeEntry)}><ArrowDownToLine size={16} /> Export copy</button><button className="button button-primary" onClick={() => desktop ? void perform('open_entry', { location, id: activeEntry.id }, 'Exported copy opened') : announce('Export and open is available in the Windows app')}><ArrowRight size={16} /> Export and open</button></div></Dialog>}
    {toast && <div className="toast" role="status"><Check size={16} />{toast}</div>}
  </div>
}

function FileRow({ entry, grid, selected, onSelect, onOpen, onMenu, view }: { entry: Entry; grid: boolean; selected: boolean; onSelect: (event: React.MouseEvent) => void; onOpen: () => void; onMenu: (x: number, y: number) => void; view: View }) {
  const isPhoto = /\.(png|jpe?g|gif|webp)$/i.test(entry.name)
  const FileIcon = entry.kind === 'folder' ? Folder : isPhoto ? Image : /\.(pdf|docx?|xlsx?|md)$/i.test(entry.name) ? FileText : File
  return <div className={`${grid ? 'file-card' : 'file-row'} ${selected ? 'chosen' : ''}`} onClick={onSelect} onDoubleClick={onOpen} onContextMenu={event => { event.preventDefault(); onMenu(event.clientX, event.clientY) }} role="button" tabIndex={0} aria-pressed={selected} aria-haspopup="menu" onKeyDown={event => { if (event.key === 'ContextMenu' || (event.shiftKey && event.key === 'F10')) { event.preventDefault(); const box = event.currentTarget.getBoundingClientRect(); onMenu(box.left + 24, box.top + box.height / 2); return } if (event.key === 'Enter') onOpen(); if (event.key === ' ') { event.preventDefault(); onSelect(event as unknown as React.MouseEvent) } }}>
    <div className="file-main"><div className={`file-icon ${entry.kind === 'folder' ? 'folder-icon' : isPhoto ? 'photo-icon' : 'doc-icon'}`}><FileIcon size={18} fill={entry.kind === 'folder' ? 'currentColor' : 'none'} /></div><div className="file-title"><strong>{entry.name}</strong><small>{entry.kind === 'folder' ? 'Folder' : entry.syncState === 'synced' ? 'In sync' : entry.syncState === 'local' ? 'On this PC' : 'Syncing'}</small></div><div className="file-badges">{entry.starred && <Star size={13} fill="currentColor" aria-label="Favorite" />}{entry.pinned && <Wifi size={13} aria-label="Available offline" />}</div></div>
    <span className="modified">{timeLabel(entry.modified)}</span><span className="file-size">{entry.kind === 'folder' ? '—' : formatSize(entry.size)}</span><div className="row-actions">{selected && <Check size={15} aria-hidden="true" />}</div>
    {grid && <div className="card-meta"><span>{entry.kind === 'folder' ? 'Folder' : formatSize(entry.size)}</span><span>{timeLabel(entry.modified)}</span></div>}
    {view === 'trash' && <span className="trash-label">In trash</span>}
  </div>
}

function EmptyState({ view, query, create, upload, allowUpload }: { view: View; query: string; create: () => void; upload: () => void; allowUpload: boolean }) {
  const emptyCopy = query ? ['No matches found', 'Try another search, or clear the search field to see your files.'] : view === 'favorites' ? ['A little space for the important things', 'Mark any file as a favorite and it will be easy to find here.'] : view === 'offline' ? ['Ready when you are', 'Files you mark for offline access will be available on this PC.'] : view === 'trash' ? ['All clear', 'Items you move to the trash will appear here.'] : ['Start with a folder', 'A good place for a new project, a trip, or anything you want to keep.']
  return <div className="empty-state"><img src="/art-empty.svg" alt="" /><h3>{emptyCopy[0]}</h3><p>{emptyCopy[1]}</p>{!query && view === 'files' && <div className="empty-actions"><button className="button button-primary" onClick={create}><FolderPlus size={16} /> Create a folder</button>{allowUpload && <button className="button button-subtle" onClick={upload}><Upload size={16} /> Add files</button>}</div>}</div>
}

function ActivityPage({ activities }: { activities: Activity[] }) {
  return <div className="settings-page activity-page"><div className="settings-intro"><div className="settings-icon"><ActivityIcon size={20} /></div><div><h2>Recent activity</h2><p>A clear view of what happened in your vault and pCloud.</p></div></div><section className="settings-card activity-card"><div className="settings-card-title"><div><h3>Latest updates</h3><p>Sync, security, and file changes.</p></div></div>{activities.length ? activities.map(item => <div className="activity-item" key={item.id}><span className={`activity-dot ${item.kind}`} /><div><strong>{item.message}</strong><small>{new Intl.DateTimeFormat(undefined, { dateStyle: 'medium', timeStyle: 'short' }).format(new Date(item.time))}</small></div></div>) : <div className="mini-empty">Nothing here yet. File and sync updates will show up here.</div>}</section></div>
}

function SetupGate({ connected, connect, create, workLocally }: { connected: boolean; connect: () => void; create: () => void; workLocally: () => void }) {
  return <div className="onboarding-wrap"><div className="onboarding-card"><div className="onboarding-art"><img src="/art-empty.svg" alt="" /></div><p className="eyebrow"><ShieldCheck size={13} /> PRIVATE BY DESIGN</p><h2>{connected ? 'Your private space starts here.' : 'A private place for your files.'}</h2><p className="onboarding-copy">{connected ? 'Create a password to protect files you choose to keep in your vault.' : 'Connect pCloud first to discover an existing vault. Your account password is never shared with this app.'}</p>{connected ? <button className="button button-primary" onClick={create}><LockKeyhole size={15} /> Create a vault password</button> : <><button className="button button-primary" onClick={connect}><Cloud size={15} /> Connect pCloud</button><p className="onboarding-foot">Already have a vault? Connect your account to unlock it.</p><button className="text-action" onClick={workLocally}>Work locally on this PC <ChevronRight size={14} /></button></>}</div></div>
}

function LoadingGate() {
  return <div className="onboarding-wrap"><div className="onboarding-card"><div className="gate-lock"><ShieldCheck size={22} /></div><h2>Checking your private space.</h2><p className="onboarding-copy">Reading vault and connection status from this PC…</p></div></div>
}

function UnlockGate({ unlock }: { unlock: () => void }) {
  return <div className="onboarding-wrap"><div className="onboarding-card"><div className="gate-lock"><LockKeyhole size={22} /></div><p className="eyebrow"><ShieldCheck size={13} /> YOUR PRIVATE SPACE</p><h2>Your vault is locked.</h2><p className="onboarding-copy">Unlock your vault to see your protected files on this PC.</p><button className="button button-primary" onClick={unlock}><LockKeyhole size={15} /> Unlock vault</button></div></div>
}

function CloudGate({ connect }: { connect: () => void }) {
  return <div className="onboarding-wrap"><div className="onboarding-card"><div className="gate-lock"><Cloud size={22} /></div><p className="eyebrow"><Cloud size={13} /> PCLOUD STORAGE</p><h2>Connect your pCloud account.</h2><p className="onboarding-copy">Sign in with pCloud to browse your cloud files. They remain visibly separate from your encrypted vault.</p><button className="button button-primary" onClick={connect}><Cloud size={15} /> Connect pCloud</button><p className="onboarding-foot">Files in pCloud are stored unencrypted by this app.</p></div></div>
}

function SettingsPage({ status, settings, setSettings, save, connect, recovery, create, password, forget, desktop }: { status: AppStatus | null; settings: { cacheLimitGib: number; clientId: string }; setSettings: (value: { cacheLimitGib: number; clientId: string }) => void; save: () => void; connect: () => void; recovery: () => void; create: () => void; password: () => void; forget: () => void; desktop: boolean }) {
  return <div className="settings-page"><div className="settings-intro"><div className="settings-icon"><SettingsIcon size={20} /></div><div><h2>Make it yours</h2><p>Your vault stays private. Configure pCloud and local cache preferences.</p></div></div><div className="settings-layout"><section className="settings-card"><div className="settings-card-title"><div><h3>Private Vault</h3><p>On-device encryption and access.</p></div><span className="secure-pill"><ShieldCheck size={13} /> {status?.unlocked ? 'Unlocked' : 'Locked'}</span></div><div className="settings-row"><div><strong>{status?.vaultExists ? 'Vault is set up' : 'Create your vault'}</strong><small>{status?.vaultExists ? 'Your password unlocks this space on this PC.' : 'Start with one password you will remember.'}</small></div><button className="button button-subtle" onClick={create}>{status?.vaultExists ? status.unlocked ? 'Lock vault' : 'Unlock' : 'Set up vault'}</button></div><div className="settings-row"><div><strong>Change password</strong><small>Choose a new password for your private files.</small></div><button className="text-action" onClick={password}>Change <ChevronRight size={15} /></button></div><div className="settings-row"><div><strong>Recovery key</strong><small>Export your recovery key to a safe place.</small></div><button className="text-action" onClick={recovery}>Export recovery key <ChevronRight size={15} /></button></div><div className="settings-row"><div><strong>Forget this PC</strong><small>Remove remembered access from this computer.</small></div><button className="text-action danger-text" onClick={forget}>Forget device</button></div></section>
    <section className="settings-card"><div className="settings-card-title"><div><h3>pCloud account</h3><p>Connect your cloud storage to browse and sync files.</p></div><span className={`connection-pill ${status?.connected ? '' : 'offline'}`}><span />{status?.connected ? 'Connected' : 'Not connected'}</span></div><div className="connected-account"><div className="cloud-account-icon"><Cloud size={19} /></div><div><strong>{status?.accountEmail ?? 'No pCloud account connected'}</strong><small>{status?.region ? `${status.region.toUpperCase()} region` : 'Sign in to choose your account region'}</small></div><button className="button button-subtle" onClick={connect}>{status?.connected ? 'Reconnect' : 'Connect pCloud'}</button></div><div className="settings-row"><label className="field-label compact-field">OAuth client ID<input value={settings.clientId} onChange={e => setSettings({ ...settings, clientId: e.target.value })} placeholder="Your pCloud app client ID" /></label></div></section>
    <section className="settings-card"><div className="settings-card-title"><div><h3>Local cache</h3><p>Choose the limit for files kept available offline.</p></div></div><div className="settings-columns"><label className="field-label">Cache limit<select value={settings.cacheLimitGib} onChange={e => setSettings({ ...settings, cacheLimitGib: Number(e.target.value) })}>{[2, 5, 10, 20, 50].map(value => <option key={value} value={value}>{value} GB</option>)}</select></label></div><div className="cache-detail"><div className="storage-track"><i style={{ width: `${status ? Math.min(100, status.cacheBytes / status.cacheLimitBytes * 100) : 0}%` }} /></div><small>{status ? `${(status.cacheBytes / 1024 ** 3).toFixed(1)} GB used of ${Math.round(status.cacheLimitBytes / 1024 ** 3)} GB` : 'Cache usage unavailable'}</small></div><div className="settings-footer"><span>{desktop ? 'Cache preferences apply on this PC.' : 'Browser preview does not change local settings.'}</span><button className="button button-primary" onClick={save}>Save settings</button></div></section></div></div>
}
