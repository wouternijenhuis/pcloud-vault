import { invoke } from '@tauri-apps/api/core'
import type {
  AccountInfo, ActionArguments, ActionResults, Activity, AppStatus, Entry, ExtraArguments, ExtraResults, FileRequest,
  FolderJob, KnownFolders, Location, Photo, Preferences, PublicLink, Share, Transfer, View,
} from './types'

declare global { interface Window { __TAURI_INTERNALS__?: unknown } }
export const isDesktop = () => typeof window !== 'undefined' && !!window.__TAURI_INTERNALS__
export type PhotoSource = 'all' | 'vault' | 'cloud'

// ---- In-memory demo used by the browser preview and the tests. All data is synthetic. ----
const now = Date.now()
const minute = 60_000
const day = 24 * 60 * minute
const file = (id: string, name: string, parentId: string | null, size: number, modified: number, extra: Partial<Entry> = {}): Entry =>
  ({ id, name, parentId, kind: 'file', size, modified, starred: false, pinned: false, trashed: false, syncState: 'synced', ...extra })
const folder = (id: string, name: string, parentId: string | null, modified: number, extra: Partial<Entry> = {}): Entry =>
  ({ ...file(id, name, parentId, 0, modified, extra), kind: 'folder' })
const vaultStarter: Entry[] = [
  folder('f-work', 'Work', null, now - 3600000, { starred: true }),
  folder('f-travel', 'Travel', null, now - 86400000, { pinned: true }),
  folder('f-personal', 'Personal', null, now - 172800000, { syncState: 'local' }),
  file('d-brief', 'Project brief.pdf', 'f-work', 2480000, now - 420000, { starred: true }),
  file('d-budget', '2025 budget.xlsx', 'f-work', 641000, now - 5500000, { pinned: true }),
  file('d-mountains', 'Alpine morning.jpg', 'f-travel', 5840000, now - 23500000, { starred: true, syncState: 'local' }),
  file('d-coast', 'Coastline.png', 'f-travel', 3980000, now - 94000000),
  file('d-notes', 'Notes.md', null, 12400, now - 1500000),
  file('d-old', 'Old draft.docx', null, 230000, now - 50000000, { trashed: true, syncState: 'local' }),
]
const cloudStarter: Entry[] = [
  folder('c-camera', 'Camera roll', null, now - 2 * day),
  folder('c-handbook', 'Team handbook', null, now - 9 * day),
  folder('c-uploads', 'Client uploads', null, now - 4 * day),
  file('c-notes', 'Notes.md', null, 8200, now - 3 * 3600000),
  file('c-invoice', 'Invoice 2025-03.pdf', null, 184000, now - 6 * day),
  file('c-beach', 'Beach day.jpg', 'c-camera', 4210000, now - 41 * day),
  file('c-picnic', 'Picnic.jpg', 'c-camera', 3650000, now - 72 * day),
  file('c-guide', 'Onboarding guide.pdf', 'c-handbook', 920000, now - 9 * day),
]
const preferencesStarter: Preferences = { protection: 'choose', theme: 'system', startAtLogin: false }
const jobsStarter: FolderJob[] = [
  { id: 'job-documents', kind: 'sync', localPath: '~/Documents', target: { location: 'vault', folderId: 'f-work', label: 'Vault / Work' }, paused: false, mirrorDeletions: false, lastRun: now - 12 * minute, state: 'idle', message: null, pendingDeletes: 0, fileCount: 1284 },
  { id: 'job-pictures', kind: 'backup', localPath: '~/Pictures', target: { location: 'cloud', folderId: 'c-camera', label: 'pCloud / Camera roll' }, paused: false, mirrorDeletions: true, lastRun: now - 3 * 60 * minute, state: 'needsReview', message: null, pendingDeletes: 14, fileCount: 3410 },
]
const transfersStarter: Transfer[] = [
  { id: 't-report', name: 'Quarterly report.pdf', direction: 'upload', location: 'vault', done: 3_200_000, total: 8_000_000, state: 'active', error: null, time: now - minute },
  { id: 't-video', name: 'Holiday video.mp4', direction: 'upload', location: 'cloud', done: 0, total: 120_000_000, state: 'queued', error: null, time: now - minute },
  { id: 't-coast', name: 'Coastline.png', direction: 'download', location: 'vault', done: 3_980_000, total: 3_980_000, state: 'done', error: null, time: now - 20 * minute },
]
const linksStarter: PublicLink[] = [
  { id: 'link-invoice', entryId: 'c-invoice', name: 'Invoice 2025-03.pdf', url: 'https://share.example.com/l/demo-invoice', created: now - 2 * day, downloads: 3, isFolder: false },
  { id: 'link-handbook', entryId: 'c-handbook', name: 'Team handbook', url: 'https://share.example.com/l/demo-handbook', created: now - 8 * day, downloads: 17, isFolder: true },
]
const sharesStarter: Share[] = [
  { id: 'share-family', direction: 'incoming', folderName: 'Family photos', email: 'sam@example.com', permissions: 'view', state: 'pending', folderId: null },
  { id: 'share-handbook', direction: 'outgoing', folderName: 'Team handbook', email: 'jordan@example.com', permissions: 'edit', state: 'active', folderId: 'c-handbook' },
]
const requestsStarter: FileRequest[] = [
  { id: 'request-client', folderId: 'c-uploads', folderName: 'Client uploads', url: 'https://share.example.com/r/demo-client', comment: 'Please upload your signed forms.', expires: now + 7 * day, maxFiles: 20, filesReceived: 4, encrypt: true, vaultFolderId: 'f-work', vaultFolderName: 'Work' },
]
const statusStarter: AppStatus = { vaultExists: true, unlocked: true, connected: true, accountEmail: 'alex@example.com', region: 'eu', cacheLimitBytes: 10 * 1024 ** 3, cacheBytes: 2.4 * 1024 ** 3, pendingCount: 2, lastSync: now - 8 * minute, oauthClientId: '' }
const activityStarter: Activity[] = [
  { id: 'a1', message: 'Synced Project brief.pdf', time: now - 10 * minute, kind: 'success' },
  { id: 'a2', message: 'Alpine morning.jpg saved for offline access', time: now - 50 * minute, kind: 'info' },
  { id: 'a3', message: 'Vault unlocked on this computer', time: now - 3 * 60 * minute, kind: 'success' },
]
const copy = <T,>(items: T[]) => items.map(item => ({ ...item }))
let vaultEntries = copy(vaultStarter)
let cloudEntries = copy(cloudStarter)
let demoStatus: AppStatus = { ...statusStarter }
let activity = copy(activityStarter)
let demoPreferences: Preferences = { ...preferencesStarter }
let demoJobs = jobsStarter.map(job => ({ ...job, target: { ...job.target } }))
let demoTransfers = copy(transfersStarter)
let demoLinks = copy(linksStarter)
let demoShares = copy(sharesStarter)
let demoRequests = copy(requestsStarter)
let demoCounter = 0
const demoId = (prefix: string) => `${prefix}-${Date.now()}-${++demoCounter}`

export const resetDemo = () => {
  vaultEntries = copy(vaultStarter); cloudEntries = copy(cloudStarter)
  demoStatus = { ...statusStarter }; activity = []
  demoPreferences = { ...preferencesStarter }
  demoJobs = jobsStarter.map(job => ({ ...job, target: { ...job.target } }))
  demoTransfers = copy(transfersStarter); demoLinks = copy(linksStarter); demoShares = copy(sharesStarter); demoRequests = copy(requestsStarter)
}
const entriesFor = (location: Location) => location === 'vault' ? vaultEntries : cloudEntries
const folderName = (location: Location, id: string | null) => id ? entriesFor(location).find(entry => entry.id === id)?.name ?? null : null
const targetLabel = (location: Location, folderId: string | null) => {
  const root = location === 'vault' ? 'Vault' : 'pCloud'
  const name = folderName(location, folderId)
  return name ? `${root} / ${name}` : root
}
const log = (message: string, kind: Activity['kind'] = 'success') => { activity.unshift({ id: demoId('activity'), message, time: Date.now(), kind }) }
const queueUpload = (name: string, location: Location, size: number) => {
  demoTransfers.push({ id: demoId('transfer'), name, direction: 'upload', location, done: 0, total: Math.max(size, 1), state: 'queued', error: null, time: Date.now() })
}
/** Each poll moves the demo queue forward, so the browser preview shows progress. */
const advanceDemoTransfers = () => {
  for (const direction of ['upload', 'download'] as const) {
    const active = demoTransfers.find(item => item.direction === direction && item.state === 'active')
    if (active) {
      active.done = Math.min(active.total, active.done + Math.ceil(active.total / 4))
      if (active.done >= active.total) { active.state = 'done'; active.time = Date.now() }
    } else {
      const next = demoTransfers.find(item => item.direction === direction && item.state === 'queued')
      if (next) next.state = 'active'
    }
  }
}
const demoThumb = (id: string) => {
  let hash = 0
  for (const char of id) hash = (hash * 31 + char.charCodeAt(0)) % 360
  const svg = `<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 64 48"><defs><linearGradient id="g" x1="0" y1="0" x2="1" y2="1"><stop offset="0" stop-color="hsl(${hash},55%,70%)"/><stop offset="1" stop-color="hsl(${(hash + 60) % 360},45%,45%)"/></linearGradient></defs><rect width="64" height="48" fill="url(#g)"/><path d="M0 40 L20 24 L32 34 L44 22 L64 38 L64 48 L0 48 Z" fill="rgba(255,255,255,.35)"/></svg>`
  return `data:image/svg+xml;charset=utf-8,${encodeURIComponent(svg)}`
}

const demo = async <T,>(native: () => Promise<T>, local: () => T): Promise<T> => isDesktop() ? native() : local()
/** Older backends may not know a query yet; treat anything that is not a list as empty. */
const list = async <T,>(native: () => Promise<T[]>, local: () => T[]): Promise<T[]> => {
  const value = await demo(native, local)
  return Array.isArray(value) ? value : []
}

export async function getStatus() { return demo(() => invoke<AppStatus>('status'), () => ({ ...demoStatus })) }
export async function listEntries(location: Location, parentId: string | null, query: string | null, view: View) {
  return demo(() => invoke<Entry[]>('list_entries', { location, parentId, query, view }), () => {
    let result = entriesFor(location).slice()
    if (view === 'trash') result = result.filter(item => item.trashed)
    else if (view === 'favorites') result = result.filter(item => item.starred && !item.trashed)
    else if (view === 'offline') result = result.filter(item => item.pinned && !item.trashed)
    else result = result.filter(item => !item.trashed)
    if (query) result = result.filter(item => item.name.toLowerCase().includes(query.toLowerCase()))
    else if (view === 'files' || parentId !== null) result = result.filter(item => item.parentId === parentId)
    return result.map(item => ({ ...item }))
  })
}
export async function getActivity() { return demo(() => invoke<Activity[]>('get_activity'), () => [...activity]) }
export async function runAction<K extends keyof ActionArguments>(command: K, args: ActionArguments[K]): Promise<ActionResults[K]> {
  return demo(() => invoke<ActionResults[K]>(command, args), () => {
    const values = args as Record<string, unknown>
    const id = String(values.id ?? '')
    const location = (values.location as Location | undefined) ?? 'vault'
    const item = entriesFor(location).find(entry => entry.id === id)
    switch (command) {
      case 'create_vault': demoStatus = { ...demoStatus, vaultExists: true, unlocked: true }; break
      case 'unlock_vault': demoStatus = { ...demoStatus, unlocked: true }; break
      case 'lock_vault': demoStatus = { ...demoStatus, unlocked: false }; break
      case 'forget_device': demoStatus = { ...demoStatus, vaultExists: false, unlocked: false }; break
      case 'connect_pcloud': case 'connect_pcloud_web': demoStatus = { ...demoStatus, connected: true, accountEmail: 'alex@example.com', region: 'eu' }; break
      case 'save_settings': demoStatus = { ...demoStatus, cacheLimitBytes: Number(values.cacheLimitGib) * 1024 ** 3, oauthClientId: String(values.clientId) }; break
      case 'new_folder': entriesFor(location).unshift(folder(demoId('folder'), String(values.name), (values.parentId as string | null) ?? null, Date.now(), { syncState: 'local' })); break
      case 'rename_entry': if (item) item.name = String(values.name); break
      case 'move_entry': if (item) item.parentId = (values.parentId as string | null) ?? null; break
      case 'trash_entry': if (item) item.trashed = true; break
      case 'restore_entry': if (item) item.trashed = false; break
      case 'toggle_favorite': { const entry = vaultEntries.find(value => value.id === id); if (entry) entry.starred = !entry.starred; break }
      case 'toggle_pin': { const entry = vaultEntries.find(value => value.id === id); if (entry) entry.pinned = !entry.pinned; break }
      case 'import_cloud_file': {
        const source = cloudEntries.find(value => value.id === id)
        if (source?.kind === 'file') vaultEntries.unshift({ ...source, id: demoId('import'), parentId: (values.parentId as string | null) ?? null, modified: Date.now(), syncState: 'local' })
        break
      }
      case 'sync_now': demoStatus = { ...demoStatus, pendingCount: 0, lastSync: Date.now() }; break
      default: break
    }
    if (['new_folder', 'rename_entry', 'move_entry', 'trash_entry', 'restore_entry', 'toggle_favorite', 'toggle_pin', 'sync_now', 'import_cloud_file'].includes(command)) log(command.replaceAll('_', ' '))
    if (['create_vault', 'unlock_vault', 'lock_vault', 'forget_device', 'connect_pcloud', 'connect_pcloud_web', 'save_settings'].includes(command)) return { ...demoStatus } as ActionResults[K]
    return undefined as ActionResults[K]
  })
}
export async function importDemoFiles(files: File[], parentId: string | null) {
  if (isDesktop()) throw new Error('Use the native file picker to add files in the desktop app.')
  files.forEach(item => vaultEntries.unshift(file(demoId('upload'), item.name, parentId, item.size, Date.now(), { syncState: 'local' })))
}
/** Browser preview only: queues picked or dropped files as demo uploads to a destination. */
export async function uploadDemoFiles(files: File[], location: Location, folderId: string | null) {
  if (isDesktop()) throw new Error('Use the native file picker to upload files in the desktop app.')
  files.forEach(item => {
    queueUpload(item.name, location, item.size)
    entriesFor(location).unshift(file(demoId('upload'), item.name, folderId, item.size, Date.now(), { syncState: 'uploading' }))
  })
}
export async function importPaths(paths: string[], parentId: string | null) { return demo(() => invoke<void>('import_paths', { paths, parentId }), () => undefined) }
export async function previewEntry(location: Location, id: string) { return demo(() => invoke<string | null>('preview_entry', { location, id }), () => null) }

// ---- 0.3.0 commands ----
export async function runExtra<K extends keyof ExtraArguments>(command: K, args: ExtraArguments[K]): Promise<ExtraResults[K]> {
  return demo(() => invoke<ExtraResults[K]>(command, args), () => demoExtra(command, args) as ExtraResults[K])
}

function demoExtra<K extends keyof ExtraArguments>(command: K, args: ExtraArguments[K]): unknown {
  const values = args as Record<string, unknown>
  const id = String(values.id ?? '')
  const job = demoJobs.find(item => item.id === id)
  switch (command) {
    case 'set_preferences': demoPreferences = { ...demoPreferences, ...(args as Partial<Preferences>) }; return { ...demoPreferences }
    case 'add_job': {
      const input = args as ExtraArguments['add_job']
      const created: FolderJob = { id: demoId('job'), kind: input.kind, localPath: input.localPath, target: { location: input.location, folderId: input.folderId, label: targetLabel(input.location, input.folderId) }, paused: false, mirrorDeletions: input.mirrorDeletions, lastRun: Date.now(), state: 'idle', message: null, pendingDeletes: 0, fileCount: 0 }
      if (input.replaces) demoJobs = demoJobs.filter(item => item.id !== input.replaces)
      demoJobs.push(created); log(`${input.kind === 'sync' ? 'Sync' : 'Backup'} added for ${input.localPath}`)
      return { ...created, target: { ...created.target } }
    }
    case 'update_job': {
      if (!job) throw new Error('That folder is no longer set up.')
      const input = args as ExtraArguments['update_job']
      if (input.paused !== undefined) { job.paused = input.paused; job.state = input.paused ? 'paused' : 'idle' }
      if (input.mirrorDeletions !== undefined) job.mirrorDeletions = input.mirrorDeletions
      return { ...job, target: { ...job.target } }
    }
    case 'remove_job': demoJobs = demoJobs.filter(item => item.id !== id); return undefined
    case 'run_job': if (job) { job.lastRun = Date.now(); if (!job.paused && job.state !== 'needsReview') job.state = 'idle' } return undefined
    case 'resolve_guard': if (job) { job.state = job.paused ? 'paused' : 'idle'; job.pendingDeletes = 0; job.lastRun = Date.now() } return undefined
    case 'upload_paths': {
      const input = args as ExtraArguments['upload_paths']
      input.paths.forEach(path => queueUpload(path.split(/[\\/]/).pop() || path, input.location, 1_000_000))
      return undefined
    }
    case 'upload_files': case 'upload_folder': {
      const input = args as ExtraArguments['upload_files']
      const names = command === 'upload_folder' ? ['Sample folder/Plan.pdf', 'Sample folder/Photo.jpg'] : ['Sample file.pdf']
      names.forEach(name => queueUpload(name, input.location, 2_000_000))
      return undefined
    }
    case 'clear_transfers': demoTransfers = demoTransfers.filter(item => item.state === 'active' || item.state === 'queued'); return undefined
    case 'create_link': {
      const entry = cloudEntries.find(item => item.id === id)
      if (!entry) throw new Error('Only pCloud files and folders can be shared.')
      const existing = demoLinks.find(item => item.entryId === id)
      if (existing) return { ...existing }
      const created: PublicLink = { id: demoId('link'), entryId: id, name: entry.name, url: `https://share.example.com/l/${entry.id}`, created: Date.now(), downloads: 0, isFolder: entry.kind === 'folder' }
      demoLinks.unshift(created)
      return { ...created }
    }
    case 'delete_link': demoLinks = demoLinks.filter(item => item.id !== id); return undefined
    case 'share_folder': {
      const input = args as ExtraArguments['share_folder']
      demoShares.push({ id: demoId('share'), direction: 'outgoing', folderName: folderName('cloud', input.id) ?? 'Folder', email: input.email, permissions: input.permissions, state: 'pending', folderId: input.id })
      return undefined
    }
    case 'accept_share': demoShares = demoShares.map(item => item.id === id ? { ...item, state: 'active' } : item); return undefined
    case 'decline_share': case 'remove_share': demoShares = demoShares.filter(item => item.id !== id); return undefined
    case 'create_file_request': {
      const input = args as ExtraArguments['create_file_request']
      const created: FileRequest = { id: demoId('request'), folderId: input.folderId, folderName: folderName('cloud', input.folderId) ?? 'pCloud', url: `https://share.example.com/r/${demoCounter}`, comment: input.comment, expires: input.expiresDays ? Date.now() + input.expiresDays * day : null, maxFiles: input.maxFiles, filesReceived: 0, encrypt: input.encrypt, vaultFolderId: input.encrypt ? input.vaultFolderId : null, vaultFolderName: input.encrypt ? folderName('vault', input.vaultFolderId) ?? 'Vault' : null }
      demoRequests.unshift(created)
      return { ...created }
    }
    case 'delete_file_request': demoRequests = demoRequests.filter(item => item.id !== id); return undefined
    default: return undefined
  }
}

export async function getPreferences() { return demo(() => invoke<Preferences>('get_preferences'), () => ({ ...demoPreferences })) }
export async function getAccountInfo() { return demo(() => invoke<AccountInfo>('account_info'), () => ({ email: 'alex@example.com', region: 'eu' as const, usedBytes: 182 * 1024 ** 3, quotaBytes: 500 * 1024 ** 3, premium: true })) }
export async function getKnownFolders() { return demo(() => invoke<KnownFolders>('known_folders'), () => ({ documents: '~/Documents', pictures: '~/Pictures', desktop: '~/Desktop' })) }
/** Opens the system folder picker; null when the user cancels. */
export async function pickFolder() { return demo(() => invoke<string | null>('pick_folder'), () => '~/Projects') }
export async function listJobs() { return list(() => invoke<FolderJob[]>('list_jobs'), () => demoJobs.map(job => ({ ...job, target: { ...job.target } }))) }
export async function getTransfers() { return list(() => invoke<Transfer[]>('get_transfers'), () => { advanceDemoTransfers(); return copy(demoTransfers) }) }
export async function listPhotos(source: PhotoSource) {
  return list(() => invoke<Photo[]>('list_photos', { source }), () => {
    const photos = (location: Location): Photo[] => entriesFor(location)
      .filter(entry => entry.kind === 'file' && !entry.trashed && /\.(png|jpe?g|gif|webp)$/i.test(entry.name))
      .map(entry => ({ id: entry.id, name: entry.name, location, modified: entry.modified, size: entry.size }))
    const vault = demoStatus.unlocked ? photos('vault') : []
    const result = source === 'vault' ? vault : source === 'cloud' ? photos('cloud') : [...vault, ...photos('cloud')]
    return result.sort((a, b) => b.modified - a.modified)
  })
}
export async function photoThumb(location: Location, id: string) { return demo(() => invoke<string | null>('photo_thumb', { location, id }), () => demoThumb(id)) }
export async function listLinks() { return list(() => invoke<PublicLink[]>('list_links'), () => copy(demoLinks)) }
export async function listShares() { return list(() => invoke<Share[]>('list_shares'), () => copy(demoShares)) }
export async function listFileRequests() { return list(() => invoke<FileRequest[]>('list_file_requests'), () => copy(demoRequests)) }
