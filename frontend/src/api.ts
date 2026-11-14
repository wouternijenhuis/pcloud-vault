import { invoke } from '@tauri-apps/api/core'
import type { ActionArguments, ActionResults, Activity, AppStatus, Entry, Location, View } from './types'

declare global { interface Window { __TAURI_INTERNALS__?: unknown } }
export const isDesktop = () => typeof window !== 'undefined' && !!window.__TAURI_INTERNALS__

const now = Date.now()
const starter: Entry[] = [
  { id: 'f-work', name: 'Work', parentId: null, kind: 'folder', size: 0, modified: now - 3600000, starred: true, pinned: false, trashed: false, syncState: 'synced' },
  { id: 'f-travel', name: 'Travel', parentId: null, kind: 'folder', size: 0, modified: now - 86400000, starred: false, pinned: true, trashed: false, syncState: 'synced' },
  { id: 'f-personal', name: 'Personal', parentId: null, kind: 'folder', size: 0, modified: now - 172800000, starred: false, pinned: false, trashed: false, syncState: 'local' },
  { id: 'd-brief', name: 'Project brief.pdf', parentId: 'f-work', kind: 'file', size: 2480000, modified: now - 420000, starred: true, pinned: false, trashed: false, syncState: 'synced' },
  { id: 'd-budget', name: '2025 budget.xlsx', parentId: 'f-work', kind: 'file', size: 641000, modified: now - 5500000, starred: false, pinned: true, trashed: false, syncState: 'synced' },
  { id: 'd-mountains', name: 'Alpine morning.jpg', parentId: 'f-travel', kind: 'file', size: 5840000, modified: now - 23500000, starred: true, pinned: false, trashed: false, syncState: 'local' },
  { id: 'd-coast', name: 'Coastline.png', parentId: 'f-travel', kind: 'file', size: 3980000, modified: now - 94000000, starred: false, pinned: false, trashed: false, syncState: 'synced' },
  { id: 'd-notes', name: 'Notes.md', parentId: null, kind: 'file', size: 12400, modified: now - 1500000, starred: false, pinned: false, trashed: false, syncState: 'synced' },
  { id: 'd-old', name: 'Old draft.docx', parentId: null, kind: 'file', size: 230000, modified: now - 50000000, starred: false, pinned: false, trashed: true, syncState: 'local' },
]
let demoEntries = starter.map(item => ({ ...item }))
let demoStatus: AppStatus = { vaultExists: true, unlocked: true, connected: true, accountEmail: 'alex@example.com', region: 'eu', cacheLimitBytes: 10 * 1024 ** 3, cacheBytes: 2.4 * 1024 ** 3, pendingCount: 2, lastSync: now - 8 * 60_000, oauthClientId: '' }
let activity: Activity[] = [
  { id: 'a1', message: 'Synced Project brief.pdf', time: now - 10 * 60_000, kind: 'success' },
  { id: 'a2', message: 'Alpine morning.jpg saved for offline access', time: now - 50 * 60_000, kind: 'info' },
  { id: 'a3', message: 'Vault unlocked on this computer', time: now - 3 * 60 * 60_000, kind: 'success' },
]
export const resetDemo = () => { demoEntries = starter.map(item => ({ ...item })); demoStatus = { ...demoStatus, vaultExists: true, unlocked: true }; activity = [] }
const demo = async <T,>(native: () => Promise<T>, local: () => T): Promise<T> => isDesktop() ? native() : local()
export async function getStatus() { return demo(() => invoke<AppStatus>('status'), () => ({ ...demoStatus })) }
export async function listEntries(location: Location, parentId: string | null, query: string | null, view: View) {
  return demo(() => invoke<Entry[]>('list_entries', { location, parentId, query, view }), () => {
    let result = demoEntries.slice()
    if (view === 'trash') result = result.filter(item => item.trashed)
    else if (view === 'favorites') result = result.filter(item => item.starred && !item.trashed)
    else if (view === 'offline') result = result.filter(item => item.pinned && !item.trashed)
    else result = result.filter(item => !item.trashed)
    if (parentId !== null) result = result.filter(item => item.parentId === parentId)
    if (query) result = result.filter(item => item.name.toLowerCase().includes(query.toLowerCase()))
    return result.map(item => ({ ...item }))
  })
}
export async function getActivity() { return demo(() => invoke<Activity[]>('get_activity'), () => [...activity]) }
export async function runAction<K extends keyof ActionArguments>(command: K, args: ActionArguments[K]): Promise<ActionResults[K]> {
  return demo(() => invoke<ActionResults[K]>(command, args), () => {
    const values = args as Record<string, unknown>
    const id = String(values.id ?? '')
    const item = demoEntries.find(entry => entry.id === id)
    switch (command) {
      case 'create_vault': demoStatus = { ...demoStatus, vaultExists: true, unlocked: true }; break
      case 'unlock_vault': demoStatus = { ...demoStatus, unlocked: true }; break
      case 'lock_vault': demoStatus = { ...demoStatus, unlocked: false }; break
      case 'forget_device': demoStatus = { ...demoStatus, vaultExists: false, unlocked: false }; break
      case 'connect_pcloud': case 'connect_pcloud_web': demoStatus = { ...demoStatus, connected: true, accountEmail: 'alex@example.com', region: 'eu' }; break
      case 'save_settings': demoStatus = { ...demoStatus, cacheLimitBytes: Number(values.cacheLimitGib) * 1024 ** 3, oauthClientId: String(values.clientId) }; break
      case 'new_folder': demoEntries.unshift({ id: `folder-${Date.now()}`, name: String(values.name), parentId: (values.parentId as string | null) ?? null, kind: 'folder', size: 0, modified: Date.now(), starred: false, pinned: false, trashed: false, syncState: 'local' }); break
      case 'rename_entry': if (item) item.name = String(values.name); break
      case 'move_entry': if (item) item.parentId = (values.parentId as string | null) ?? null; break
      case 'trash_entry': if (item) item.trashed = true; break
      case 'restore_entry': if (item) item.trashed = false; break
      case 'toggle_favorite': if (item) item.starred = !item.starred; break
      case 'toggle_pin': if (item) item.pinned = !item.pinned; break
      case 'import_cloud_file': if (item && item.kind === 'file') demoEntries.unshift({ ...item, id: `import-${Date.now()}-${item.id}`, parentId: null, modified: Date.now(), starred: false, pinned: false, trashed: false, syncState: 'local' }); break
      case 'sync_now': demoStatus = { ...demoStatus, pendingCount: 0, lastSync: Date.now() }; break
      default: break
    }
    if (['new_folder', 'rename_entry', 'move_entry', 'trash_entry', 'restore_entry', 'toggle_favorite', 'toggle_pin', 'sync_now'].includes(command)) activity.unshift({ id: String(Date.now()), message: command.replaceAll('_', ' '), time: Date.now(), kind: 'success' })
    if (['create_vault', 'unlock_vault', 'lock_vault', 'forget_device', 'connect_pcloud', 'connect_pcloud_web', 'save_settings'].includes(command)) return { ...demoStatus } as ActionResults[K]
    return undefined as ActionResults[K]
  })
}
export async function importDemoFiles(files: File[], parentId: string | null) {
  if (isDesktop()) throw new Error('Use the native file picker to add files in the desktop app.')
  files.forEach(file => demoEntries.unshift({ id: `upload-${Date.now()}-${file.name}`, name: file.name, parentId, kind: 'file', size: file.size, modified: Date.now(), starred: false, pinned: false, trashed: false, syncState: 'local' }))
}
export async function importPaths(paths: string[], parentId: string | null) { return demo(() => invoke<void>('import_paths', { paths, parentId }), () => undefined) }
export async function previewEntry(location: Location, id: string) { return demo(() => invoke<string | null>('preview_entry', { location, id }), () => null) }
