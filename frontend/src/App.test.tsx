import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import { cleanup, fireEvent, render, screen, waitFor, within } from '@testing-library/react'
import App from './App'
import { resetDemo, runAction } from './api'
import { invoke } from '@tauri-apps/api/core'
import type { AppStatus } from './types'

vi.mock('@tauri-apps/api/core', () => ({ invoke: vi.fn() }))
vi.mock('@tauri-apps/api/webviewWindow', () => ({ getCurrentWebviewWindow: () => ({ onDragDropEvent: () => Promise.resolve(() => undefined) }) }))

const nativeStatus = {
  vaultExists: true, unlocked: true, connected: true, accountEmail: 'alex@example.com', region: 'eu' as const,
  cacheLimitBytes: 10 * 1024 ** 3, cacheBytes: 1 * 1024 ** 3,
  pendingCount: 0, lastSync: Date.now(), oauthClientId: 'client-id',
}
let nativeStatusForTest: AppStatus = { ...nativeStatus }
const nativeEntries = [
  { id: 'folder', name: 'Travel', parentId: null, kind: 'folder' as const, size: 0, modified: Date.now(), starred: false, pinned: false, trashed: false, syncState: 'synced' as const },
  { id: 'note', name: 'Notes.md', parentId: null, kind: 'file' as const, size: 1000, modified: Date.now(), starred: false, pinned: false, trashed: false, syncState: 'synced' as const },
  { id: 'photo', name: 'Photo.png', parentId: null, kind: 'file' as const, size: 1000, modified: Date.now(), starred: false, pinned: false, trashed: false, syncState: 'synced' as const },
]
function useNativeBridge() {
  window.__TAURI_INTERNALS__ = {}
  vi.mocked(invoke).mockImplementation(async (command) => {
    if (command === 'status') return { ...nativeStatusForTest } as never
    if (command === 'list_entries') return [...nativeEntries] as never
    if (command === 'lock_vault') { nativeStatusForTest = { ...nativeStatus, unlocked: false }; return { ...nativeStatusForTest } as never }
    return undefined as never
  })
}

describe('pCloud Vault interface', () => {
  beforeEach(() => { resetDemo(); nativeStatusForTest = { ...nativeStatus }; delete window.__TAURI_INTERNALS__; vi.mocked(invoke).mockReset() })
  afterEach(() => { cleanup(); vi.restoreAllMocks() })

  it('clearly labels browser mode and unencrypted cloud files', async () => {
    render(<App />)
    expect(await screen.findByText('DEMO MODE')).toBeInTheDocument()
    expect(screen.getByText('Unencrypted')).toBeInTheDocument()
    expect(await screen.findByText('Project brief.pdf')).toBeInTheDocument()
    expect(screen.getByText('Sample data only — no cloud changes are made.')).toBeInTheDocument()
    expect(screen.getByText('Cached files stay encrypted on this PC.')).toBeInTheDocument()
    expect(screen.getByRole('button', { name: 'Available offline' }).querySelector('.nav-count')).toBeNull()
  })

  it('creates a folder in the labeled demo workspace', async () => {
    render(<App />)
    fireEvent.click(await screen.findByRole('button', { name: 'New' }))
    fireEvent.change(screen.getByLabelText('Folder name'), { target: { value: 'Receipts' } })
    fireEvent.click(screen.getByRole('button', { name: 'Create folder' }))
    expect(await screen.findByText('Receipts')).toBeInTheDocument()
    expect(screen.getByRole('status')).toHaveTextContent('Folder created')
  })

  it('surfaces native backend errors instead of swallowing them', async () => {
    window.__TAURI_INTERNALS__ = {}
    vi.mocked(invoke).mockRejectedValueOnce(new Error('Sync is unavailable'))
    await expect(runAction('sync_now', {})).rejects.toThrow('Sync is unavailable')
    await waitFor(() => expect(invoke).toHaveBeenCalledWith('sync_now', {}))
  })

  it('clears visible vault names immediately when locked', async () => {
    render(<App />)
    expect(await screen.findByText('Project brief.pdf')).toBeInTheDocument()
    fireEvent.click(screen.getByTitle('Lock vault'))
    expect(await screen.findByText('Your vault is locked.')).toBeInTheDocument()
    expect(screen.queryByText('Project brief.pdf')).not.toBeInTheDocument()
  })

  it('shows vault import, not vault-only toggles, for cloud files', async () => {
    render(<App />)
    fireEvent.click(screen.getByText('pCloud Files'))
    fireEvent.click(await screen.findByText('Notes.md'))
    const selection = document.querySelector<HTMLElement>('.selection-bar')!
    expect(within(selection).getByRole('button', { name: /Import to vault/ })).toBeInTheDocument()
    expect(within(selection).queryByRole('button', { name: /Favorite|Offline/ })).not.toBeInTheDocument()
  })

  it('returns collection navigation to the private vault workspace', async () => {
    render(<App />)
    fireEvent.click(screen.getByText('pCloud Files'))
    fireEvent.click(screen.getByRole('button', { name: 'Favorites' }))
    const favorite = await screen.findByText('Project brief.pdf')
    fireEvent.click(favorite)
    const selection = document.querySelector<HTMLElement>('.selection-bar')!
    expect(within(selection).getByRole('button', { name: /Favorite/ })).toBeInTheDocument()
    expect(within(selection).queryByRole('button', { name: /Import to vault/ })).not.toBeInTheDocument()
  })

  it('supports navigating into a folder and back to root', async () => {
    render(<App />)
    const travel = await screen.findByText('Travel')
    fireEvent.doubleClick(travel.closest('[role="button"]')!)
    expect(await screen.findByText('Coastline.png')).toBeInTheDocument()
    fireEvent.click(screen.getByRole('button', { name: 'Root' }))
    expect(await screen.findByText('Notes.md')).toBeInTheDocument()
    expect(screen.queryByRole('button', { name: 'Go up one folder' })).not.toBeInTheDocument()
  })

  it('offers folder export from selection', async () => {
    render(<App />)
    fireEvent.click(await screen.findByText('Travel'))
    expect(screen.getByRole('button', { name: 'Export copy' })).toBeInTheDocument()
  })

  it('sends the Rust cache key and enforces twelve-character password input', async () => {
    useNativeBridge()
    render(<App />)
    fireEvent.click(await screen.findByRole('button', { name: 'Settings' }))
    fireEvent.change(await screen.findByLabelText('Cache limit'), { target: { value: '20' } })
    fireEvent.click(screen.getByRole('button', { name: 'Save settings' }))
    await waitFor(() => expect(invoke).toHaveBeenCalledWith('save_settings', { cacheLimitGib: 20, clientId: 'client-id' }))
    fireEvent.click(screen.getByRole('button', { name: 'Change' }))
    const newPassword = screen.getByLabelText('New password') as HTMLInputElement
    expect(newPassword.minLength).toBe(12)
    expect(screen.getByRole('dialog')).toHaveTextContent('does not re-encrypt every file')
    expect(screen.getByRole('dialog')).toHaveTextContent('Trusted PCs')
  })

  it('signs in on the pCloud website without needing an OAuth client ID', async () => {
    useNativeBridge()
    render(<App />)
    fireEvent.click(await screen.findByRole('button', { name: 'Settings' }))
    fireEvent.click(await screen.findByRole('button', { name: 'Reconnect' }))
    const dialog = screen.getByRole('dialog')
    expect(dialog).toHaveTextContent('Use a registered pCloud app instead')
    fireEvent.click(within(dialog).getByRole('button', { name: /Sign in on pCloud website/ }))
    await waitFor(() => expect(invoke).toHaveBeenCalledWith('connect_pcloud_web', {}))
    await waitFor(() => expect(screen.queryByRole('dialog')).not.toBeInTheDocument())
    expect(invoke).not.toHaveBeenCalledWith('connect_pcloud', expect.anything())
  })

  it('shows the backend message when website sign-in is cancelled', async () => {
    useNativeBridge()
    const native = vi.mocked(invoke).getMockImplementation()!
    vi.mocked(invoke).mockImplementation(async (command, args) => {
      if (command === 'connect_pcloud_web') throw 'pCloud sign-in was cancelled.'
      return native(command, args)
    })
    render(<App />)
    fireEvent.click(await screen.findByRole('button', { name: 'Settings' }))
    fireEvent.click(await screen.findByRole('button', { name: 'Reconnect' }))
    const dialog = screen.getByRole('dialog')
    fireEvent.click(within(dialog).getByRole('button', { name: /Sign in on pCloud website/ }))
    expect(await within(dialog).findByText('pCloud sign-in was cancelled.')).toBeInTheDocument()
  })

  it('opens a keyboard-accessible context menu with the same actions on right-click', async () => {
    useNativeBridge()
    render(<App />)
    fireEvent.contextMenu(await screen.findByText('Photo.png'))
    const menu = screen.getByRole('menu', { name: 'Actions for Photo.png' })
    const items = within(menu).getAllByRole('menuitem').map(item => item.textContent)
    expect(items).toEqual(['Preview', 'Export copy', 'Rename', 'Move', 'Add to favorites', 'Keep offline', 'Move to trash'])
    expect(document.activeElement).toBe(within(menu).getAllByRole('menuitem')[0])
    fireEvent.keyDown(window, { key: 'ArrowDown' })
    expect(document.activeElement).toHaveTextContent('Export copy')
    fireEvent.click(within(menu).getByRole('menuitem', { name: 'Keep offline' }))
    await waitFor(() => expect(invoke).toHaveBeenCalledWith('toggle_pin', { id: 'photo' }))
    expect(screen.queryByRole('menu')).not.toBeInTheDocument()
    const row = (await screen.findByText('Notes.md')).closest<HTMLElement>('[role="button"]')!
    row.focus()
    fireEvent.keyDown(row, { key: 'F10', shiftKey: true })
    expect(screen.getByRole('menu', { name: 'Actions for Notes.md' })).toBeInTheDocument()
    fireEvent.keyDown(window, { key: 'Escape' })
    expect(screen.queryByRole('menu')).not.toBeInTheDocument()
    expect(document.activeElement).toBe(row)
  })

  it('acts on the newly selected item after a previous rename', async () => {
    useNativeBridge()
    render(<App />)
    fireEvent.click(await screen.findByText('Notes.md'))
    fireEvent.click(screen.getByRole('button', { name: 'Rename' }))
    fireEvent.click(within(screen.getByRole('dialog')).getByRole('button', { name: 'Cancel' }))
    fireEvent.click(screen.getByText('Photo.png'))
    fireEvent.click(screen.getByRole('button', { name: 'Rename' }))
    expect(within(screen.getByRole('dialog')).getByLabelText('New name')).toHaveValue('Photo.png')
    fireEvent.change(within(screen.getByRole('dialog')).getByLabelText('New name'), { target: { value: 'Renamed.png' } })
    fireEvent.click(within(screen.getByRole('dialog')).getByRole('button', { name: 'Save name' }))
    await waitFor(() => expect(invoke).toHaveBeenCalledWith('rename_entry', { location: 'vault', id: 'photo', name: 'Renamed.png' }))
    fireEvent.click(screen.getByText('Notes.md'))
    fireEvent.click(screen.getByRole('button', { name: 'Move to trash' }))
    expect(screen.getByRole('dialog')).toHaveTextContent('Notes.md')
  })

  it('shows matching files when searching from Settings and clears search on navigation', async () => {
    useNativeBridge()
    render(<App />)
    fireEvent.click(await screen.findByRole('button', { name: 'Settings' }))
    expect(await screen.findByLabelText('Cache limit')).toBeInTheDocument()
    fireEvent.change(screen.getByLabelText('Search files'), { target: { value: 'photo' } })
    await waitFor(() => expect(invoke).toHaveBeenCalledWith('list_entries', { location: 'vault', parentId: null, query: 'photo', view: 'files' }))
    expect(screen.queryByLabelText('Cache limit')).not.toBeInTheDocument()
    fireEvent.click(screen.getByRole('button', { name: 'Settings' }))
    expect(screen.getByLabelText('Search files')).toHaveValue('')
  })

  it('guides a first start through connecting and creating a vault', async () => {
    useNativeBridge()
    localStorage.clear()
    nativeStatusForTest = { ...nativeStatus, vaultExists: false, unlocked: false, connected: false, accountEmail: null, region: null }
    const native = vi.mocked(invoke).getMockImplementation()!
    vi.mocked(invoke).mockImplementation(async (command, args) => {
      if (command === 'connect_pcloud_web') { nativeStatusForTest = { ...nativeStatusForTest, connected: true, accountEmail: 'alex@example.com', region: 'eu' }; return { ...nativeStatusForTest } as never }
      if (command === 'create_vault') { nativeStatusForTest = { ...nativeStatusForTest, vaultExists: true, unlocked: true }; return { ...nativeStatusForTest } as never }
      return native(command, args)
    })
    render(<App />)
    const wizard = await screen.findByRole('dialog', { name: 'Welcome to pCloud Vault' })
    expect(wizard).toHaveTextContent('Made in Europe')
    fireEvent.click(within(wizard).getByRole('button', { name: 'Get started' }))
    fireEvent.click(within(wizard).getByRole('button', { name: /Sign in on pCloud website/ }))
    expect(await within(wizard).findByText('Connected')).toBeInTheDocument()
    fireEvent.click(within(wizard).getByRole('button', { name: 'Continue' }))
    fireEvent.change(within(wizard).getByLabelText('Vault password'), { target: { value: 'long enough secret' } })
    fireEvent.change(within(wizard).getByLabelText('Repeat password'), { target: { value: 'long enough secreT' } })
    fireEvent.click(within(wizard).getByRole('button', { name: 'Create vault' }))
    expect(await within(wizard).findByText("The passwords don't match.")).toBeInTheDocument()
    expect(invoke).not.toHaveBeenCalledWith('create_vault', expect.anything())
    fireEvent.change(within(wizard).getByLabelText('Repeat password'), { target: { value: 'long enough secret' } })
    fireEvent.click(within(wizard).getByRole('button', { name: 'Create vault' }))
    await waitFor(() => expect(invoke).toHaveBeenCalledWith('create_vault', { password: 'long enough secret', remember: true }))
    expect(await within(wizard).findByText("You're all set")).toBeInTheDocument()
    fireEvent.click(within(wizard).getByRole('button', { name: 'Open my vault' }))
    await waitFor(() => expect(screen.queryByRole('dialog')).not.toBeInTheDocument())
    expect(localStorage.getItem('pcloud-vault.welcome-complete')).toBe('1')
  })

  it('does not show the guide when this PC already has a vault', async () => {
    useNativeBridge()
    localStorage.clear()
    render(<App />)
    expect(await screen.findByText('Notes.md')).toBeInTheDocument()
    expect(screen.queryByRole('dialog', { name: 'Welcome to pCloud Vault' })).not.toBeInTheDocument()
  })

  it('requires twelve characters when creating a vault', async () => {
    render(<App />)
    fireEvent.click(await screen.findByRole('button', { name: 'Settings' }))
    fireEvent.click(screen.getByRole('button', { name: 'Forget device' }))
    fireEvent.click(await screen.findByRole('button', { name: 'Private Vault' }))
    fireEvent.click(await screen.findByRole('button', { name: 'Create a vault password' }))
    expect((screen.getByLabelText('Vault password') as HTMLInputElement).minLength).toBe(12)
  })

  it('uses one native file picker command for Browse files', async () => {
    useNativeBridge()
    render(<App />)
    fireEvent.click(await screen.findByRole('button', { name: 'Browse files' }))
    await waitFor(() => expect(invoke).toHaveBeenCalledWith('import_files', { parentId: null }))
    expect(vi.mocked(invoke).mock.calls.filter(([command]) => command === 'import_files')).toHaveLength(1)
    expect(document.querySelector('input[type="file"]')).toBeNull()
  })

  it('uses the native export command for a selected folder', async () => {
    useNativeBridge()
    render(<App />)
    fireEvent.click(await screen.findByText('Travel'))
    fireEvent.click(screen.getByRole('button', { name: 'Export copy' }))
    await waitFor(() => expect(invoke).toHaveBeenCalledWith('download_entry', { location: 'vault', id: 'folder' }))
  })

  it('closes private previews when the native status becomes locked', async () => {
    useNativeBridge()
    let resolvePreview: (value: string | null) => void = () => undefined
    vi.mocked(invoke).mockImplementation(async (command) => {
      if (command === 'status') return { ...nativeStatusForTest } as never
      if (command === 'list_entries') return [...nativeEntries] as never
      if (command === 'preview_entry') return await new Promise<string | null>(resolve => { resolvePreview = resolve }) as never
      if (command === 'lock_vault') { nativeStatusForTest = { ...nativeStatus, unlocked: false }; return { ...nativeStatusForTest } as never }
      return undefined as never
    })
    render(<App />)
    const photo = await screen.findByText('Photo.png')
    fireEvent.doubleClick(photo.closest('[role="button"]')!)
    expect(await screen.findByRole('dialog')).toBeInTheDocument()
    fireEvent.click(screen.getByTitle('Lock vault'))
    await waitFor(() => expect(screen.queryByRole('dialog')).not.toBeInTheDocument())
    resolvePreview('data:image/png;base64,private-preview')
    await waitFor(() => expect(screen.queryByRole('img', { name: 'Photo.png' })).not.toBeInTheDocument())
  })
})
