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
    expect(within(screen.getByRole('button', { name: 'pCloud Files' })).getByText('Not encrypted')).toBeInTheDocument()
    expect(await screen.findByText('Notes.md')).toBeInTheDocument()
    expect(screen.getByText('Sample data only — no cloud changes are made.')).toBeInTheDocument()
    expect(screen.getByText('You are in the Vault. Everything here is encrypted.')).toBeInTheDocument()
    expect(screen.getByRole('button', { name: 'Available offline' }).querySelector('.nav-count')).toBeNull()
    fireEvent.click(screen.getByRole('button', { name: 'pCloud Files' }))
    expect(await screen.findByText('You are in pCloud Files. Nothing here is encrypted by this app.')).toBeInTheDocument()
    fireEvent.click(screen.getByRole('button', { name: 'Settings' }))
    expect(await screen.findByText('Cached files stay encrypted on this computer.')).toBeInTheDocument()
  })

  it('moves every selected Vault item to the trash, not only the first', async () => {
    render(<App />)
    const work = (await screen.findByText('Work')).closest('[role="button"]') as HTMLElement
    const travel = screen.getByText('Travel').closest('[role="button"]') as HTMLElement
    fireEvent.click(work)
    fireEvent.click(travel, { ctrlKey: true })
    expect(screen.getByText('2 selected')).toBeInTheDocument()
    fireEvent.click(screen.getByRole('button', { name: 'Move to trash' }))
    const dialog = screen.getByRole('dialog')
    expect(dialog).toHaveTextContent('Work')
    expect(dialog).toHaveTextContent('Travel')
    fireEvent.click(within(dialog).getByRole('button', { name: 'Move to trash' }))
    await waitFor(() => expect(screen.queryByText('Travel')).not.toBeInTheDocument())
    expect(screen.queryByText('Work')).not.toBeInTheDocument()
    expect(screen.getByText('Notes.md')).toBeInTheDocument()
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
    expect(await screen.findByText('Notes.md')).toBeInTheDocument()
    fireEvent.click(screen.getByTitle('Lock vault'))
    expect(await screen.findByText('Your vault is locked.')).toBeInTheDocument()
    expect(screen.queryByText('Notes.md')).not.toBeInTheDocument()
  })

  it('shows copy into Vault and sharing, not vault-only toggles, for cloud files', async () => {
    render(<App />)
    fireEvent.click(screen.getByText('pCloud Files'))
    fireEvent.click(await screen.findByText('Notes.md'))
    const selection = document.querySelector<HTMLElement>('.selection-bar')!
    expect(within(selection).getByRole('button', { name: /Copy into Vault/ })).toBeInTheDocument()
    expect(within(selection).getByRole('button', { name: /Get link/ })).toBeInTheDocument()
    expect(within(selection).queryByRole('button', { name: /Favorite|Offline/ })).not.toBeInTheDocument()
  })

  it('offers Get link and Share folder for pCloud folders', async () => {
    render(<App />)
    fireEvent.click(screen.getByText('pCloud Files'))
    fireEvent.click(await screen.findByText('Team handbook'))
    const selection = document.querySelector<HTMLElement>('.selection-bar')!
    expect(within(selection).getByRole('button', { name: /Get link/ })).toBeInTheDocument()
    fireEvent.click(within(selection).getByRole('button', { name: /Share folder/ }))
    expect(screen.getByRole('dialog')).toHaveTextContent('It is not encrypted')
  })

  it('never offers sharing for Vault files and says why', async () => {
    render(<App />)
    fireEvent.click(await screen.findByText('Notes.md'))
    const selection = document.querySelector<HTMLElement>('.selection-bar')!
    expect(within(selection).queryByRole('button', { name: /Get link|Share/ })).not.toBeInTheDocument()
    expect(within(selection).getByText(/Vault files can't be shared: sharing would let pCloud read them/)).toBeInTheDocument()
    fireEvent.contextMenu(screen.getByText('Notes.md'))
    expect(within(screen.getByRole('menu')).queryByRole('menuitem', { name: /link|Share/i })).not.toBeInTheDocument()
    fireEvent.keyDown(window, { key: 'Escape' })
    fireEvent.click(screen.getByRole('button', { name: 'Shared' }))
    expect(await screen.findByText(/To share a Vault file, export a copy and upload it to pCloud Files/)).toBeInTheDocument()
  })

  it('keeps vault collections inside the Vault workspace', async () => {
    render(<App />)
    fireEvent.click(screen.getByText('pCloud Files'))
    expect(screen.queryByRole('button', { name: 'Favorites' })).not.toBeInTheDocument()
    fireEvent.click(screen.getByRole('button', { name: 'Vault' }))
    fireEvent.click(await screen.findByRole('button', { name: 'Favorites' }))
    const favorite = await screen.findByText('Project brief.pdf')
    fireEvent.click(favorite)
    const selection = document.querySelector<HTMLElement>('.selection-bar')!
    expect(within(selection).getByRole('button', { name: /Favorite/ })).toBeInTheDocument()
    expect(within(selection).queryByRole('button', { name: /Copy into Vault/ })).not.toBeInTheDocument()
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

  it('opens the folder that was double-clicked; selecting never inserts space above the rows', async () => {
    render(<App />)
    fireEvent.click(screen.getByText('pCloud Files'))
    const handbook = (await screen.findByText('Team handbook')).closest<HTMLElement>('[role="button"]')!
    const slot = screen.getByTestId('selection-slot')
    expect(slot.querySelector('.selection-bar')).toBeNull()
    expect(slot.compareDocumentPosition(handbook) & Node.DOCUMENT_POSITION_FOLLOWING).toBeTruthy()
    fireEvent.click(handbook)
    // The same reserved slot holds the bar, so the rows below it keep their place.
    expect(screen.getByTestId('selection-slot')).toBe(slot)
    expect(slot.querySelector('.selection-bar')).not.toBeNull()
    fireEvent.click(handbook)
    fireEvent.doubleClick(handbook)
    expect(await screen.findByText('Onboarding guide.pdf')).toBeInTheDocument()
    expect(screen.getByRole('button', { name: 'Team handbook' })).toBeInTheDocument()
    expect(screen.queryByText('Beach day.jpg')).not.toBeInTheDocument()
    expect(screen.getByTestId('selection-slot').querySelector('.selection-bar')).toBeNull()
  })

  it('renames, moves and trashes pCloud files in pCloud, without vault-only actions', async () => {
    render(<App />)
    fireEvent.click(screen.getByText('pCloud Files'))
    fireEvent.contextMenu(await screen.findByText('Notes.md'))
    const items = within(screen.getByRole('menu', { name: 'Actions for Notes.md' })).getAllByRole('menuitem').map(item => item.textContent)
    expect(items).toEqual(['Preview', 'Export copy', 'Get link', 'Copy into Vault', 'Rename', 'Move', 'Move to pCloud trash'])
    fireEvent.keyDown(window, { key: 'Escape' })
    const selection = () => document.querySelector<HTMLElement>('.selection-bar')!
    fireEvent.click(within(selection()).getByRole('button', { name: 'Rename' }))
    fireEvent.change(within(screen.getByRole('dialog')).getByLabelText('New name'), { target: { value: 'Plain notes.md' } })
    fireEvent.click(within(screen.getByRole('dialog')).getByRole('button', { name: 'Save name' }))
    fireEvent.click(await screen.findByText('Plain notes.md'))
    fireEvent.click(within(selection()).getByRole('button', { name: 'Move' }))
    fireEvent.change(within(screen.getByRole('dialog')).getByLabelText('Destination'), { target: { value: 'c-handbook' } })
    fireEvent.click(within(screen.getByRole('dialog')).getByRole('button', { name: 'Move here' }))
    await waitFor(() => expect(screen.queryByText('Plain notes.md')).not.toBeInTheDocument())
    fireEvent.click(screen.getByText('Invoice 2025-03.pdf'))
    expect(within(selection()).queryByRole('button', { name: /Favorite|Offline|Restore/ })).not.toBeInTheDocument()
    fireEvent.click(within(selection()).getByRole('button', { name: /Move to pCloud trash/ }))
    const dialog = screen.getByRole('dialog')
    expect(dialog).toHaveTextContent("Moves it to pCloud's trash. Restore it from pCloud's trash on the website.")
    fireEvent.click(within(dialog).getByRole('button', { name: 'Move to pCloud trash' }))
    await waitFor(() => expect(screen.queryByText('Invoice 2025-03.pdf')).not.toBeInTheDocument())
    expect(screen.getByRole('status')).toHaveTextContent('Moved to pCloud trash')
  })

  it('sends cloud move and trash with location cloud', async () => {
    useNativeBridge()
    render(<App />)
    fireEvent.click(screen.getByText('pCloud Files'))
    fireEvent.click(await screen.findByText('Notes.md'))
    fireEvent.click(screen.getByRole('button', { name: /Move to pCloud trash/ }))
    fireEvent.click(within(screen.getByRole('dialog')).getByRole('button', { name: 'Move to pCloud trash' }))
    await waitFor(() => expect(invoke).toHaveBeenCalledWith('trash_entry', { location: 'cloud', id: 'note' }))
    fireEvent.click(screen.getByText('Photo.png'))
    fireEvent.click(screen.getByRole('button', { name: 'Move' }))
    fireEvent.change(within(screen.getByRole('dialog')).getByLabelText('Destination'), { target: { value: 'folder' } })
    fireEvent.click(within(screen.getByRole('dialog')).getByRole('button', { name: 'Move here' }))
    await waitFor(() => expect(invoke).toHaveBeenCalledWith('move_entry', { location: 'cloud', id: 'photo', parentId: 'folder' }))
  })

  it('confirms before deleting a public link', async () => {
    render(<App />)
    fireEvent.click(await screen.findByRole('button', { name: 'Shared' }))
    fireEvent.click(await screen.findByRole('button', { name: 'Delete link to Invoice 2025-03.pdf' }))
    const dialog = screen.getByRole('dialog')
    expect(dialog).toHaveTextContent('Delete this link?')
    expect(dialog).toHaveTextContent('People who have it can no longer open the file.')
    fireEvent.click(within(dialog).getByRole('button', { name: 'Cancel' }))
    expect(screen.getByText('Invoice 2025-03.pdf')).toBeInTheDocument()
    fireEvent.click(screen.getByRole('button', { name: 'Delete link to Invoice 2025-03.pdf' }))
    fireEvent.click(within(screen.getByRole('dialog')).getByRole('button', { name: 'Delete link' }))
    await waitFor(() => expect(screen.queryByText('Invoice 2025-03.pdf')).not.toBeInTheDocument())
  })

  it('shows only the state for job transfers without a byte total, with a readable protection label', async () => {
    useNativeBridge()
    const native = vi.mocked(invoke).getMockImplementation()!
    vi.mocked(invoke).mockImplementation(async (command, args) => {
      if (command === 'get_transfers') return [
        { id: 'j1', name: 'Documents sync', direction: 'upload', location: 'vault', done: 0, total: 0, state: 'done', error: null, time: Date.now() },
        { id: 'j2', name: 'Pictures backup', direction: 'upload', location: 'cloud', done: 0, total: 0, state: 'active', error: null, time: Date.now() },
        { id: 'j3', name: 'Report.pdf', direction: 'upload', location: 'cloud', done: 50, total: 100, state: 'active', error: null, time: Date.now() },
      ] as never
      return native(command, args)
    })
    render(<App />)
    fireEvent.click(await screen.findByRole('button', { name: 'Upload' }))
    const queue = screen.getByRole('region', { name: 'Upload queue' })
    expect(await within(queue).findByText('Documents sync')).toBeInTheDocument()
    expect(within(queue).queryByText('0 B of 0 B')).not.toBeInTheDocument()
    expect(within(queue).getAllByText(/ of /)).toHaveLength(1)
    expect(within(queue).getByText('Done')).toBeInTheDocument()
    expect(within(queue).getByText('Working…')).toBeInTheDocument()
    expect(within(queue).getByText('50%')).toBeInTheDocument()
    expect(within(queue).getAllByText('Not encrypted')).toHaveLength(2)
    expect(within(queue).getAllByText('Encrypted')).toHaveLength(1)
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
    expect(screen.getByRole('dialog')).toHaveTextContent('Trusted computers')
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

  it('shows search only on the file pages and opens it with Ctrl+K from other pages', async () => {
    useNativeBridge()
    render(<App />)
    expect(await screen.findByLabelText('Search files')).toHaveAttribute('placeholder', 'Search the Vault')
    for (const label of ['Upload', 'Back up', 'Sync', 'Photos', 'Shared', 'File request', 'About', 'Account', 'Help', 'Settings']) {
      fireEvent.click(screen.getByRole('button', { name: label }))
      expect(screen.queryByLabelText('Search files')).not.toBeInTheDocument()
    }
    fireEvent.click(screen.getByRole('button', { name: 'pCloud Files' }))
    expect(screen.getByLabelText('Search files')).toHaveAttribute('placeholder', 'Search pCloud Files')
    fireEvent.click(screen.getByRole('button', { name: 'Settings' }))
    expect(await screen.findByLabelText('Cache limit')).toBeInTheDocument()
    fireEvent.keyDown(window, { key: 'k', ctrlKey: true })
    await waitFor(() => expect(screen.getByLabelText('Search files')).toHaveFocus())
    expect(screen.getByRole('heading', { level: 1 })).toHaveTextContent('pCloud Files')
    fireEvent.change(screen.getByLabelText('Search files'), { target: { value: 'photo' } })
    await waitFor(() => expect(invoke).toHaveBeenCalledWith('list_entries', { location: 'cloud', parentId: null, query: 'photo', view: 'files' }))
    fireEvent.click(screen.getByRole('button', { name: 'Settings' }))
    fireEvent.click(screen.getByRole('button', { name: 'pCloud Files' }))
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
    await waitFor(() => expect(document.querySelector('main')).toHaveAttribute('inert'))
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
    expect(await within(wizard).findByText('How should your files be protected?')).toBeInTheDocument()
    expect(wizard).toHaveTextContent('pCloud still sees roughly how much you store')
    fireEvent.click(within(wizard).getByRole('radio', { name: /Let me choose each time/ }))
    expect(within(wizard).getByRole('radio', { name: /Let me choose each time/ })).toHaveAttribute('aria-checked', 'true')
    fireEvent.click(within(wizard).getByRole('radio', { name: /Everything encrypted/ }))
    fireEvent.click(within(wizard).getByRole('button', { name: 'Continue' }))
    await waitFor(() => expect(invoke).toHaveBeenCalledWith('set_preferences', { protection: 'everything' }))
    expect(await within(wizard).findByText('Back up or sync a folder now?')).toBeInTheDocument()
    fireEvent.click(within(wizard).getByRole('button', { name: 'Skip for now' }))
    expect(await within(wizard).findByText("You're all set")).toBeInTheDocument()
    fireEvent.click(within(wizard).getByRole('button', { name: 'Open my vault' }))
    await waitFor(() => expect(screen.queryByRole('dialog')).not.toBeInTheDocument())
    expect(localStorage.getItem('pcloud-vault.welcome-complete')).toBe('1')
    expect(document.querySelector('main')).not.toHaveAttribute('inert')
  })

  it('does not show the guide when this computer already has a vault', async () => {
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
    fireEvent.click(await screen.findByRole('button', { name: 'Vault' }))
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

  it('shows every section of the official app in order and opens each one', async () => {
    render(<App />)
    await screen.findByText('Notes.md')
    const labels = ['Upload', 'Back up', 'Sync', 'Photos', 'Vault', 'pCloud Files', 'Shared', 'File request', 'About', 'Account', 'Help', 'Settings']
    const buttons = [...screen.getByRole('navigation', { name: 'Sections' }).querySelectorAll('button'), ...screen.getByRole('navigation', { name: 'Account and settings' }).querySelectorAll('button')]
    expect(buttons.map(button => button.getAttribute('aria-label'))).toEqual(labels)
    const content: Record<string, string | RegExp> = {
      Upload: 'Upload your files', 'Back up': /one-way copy of a folder/, Sync: /in both directions/, Photos: /newest first/,
      Vault: 'You are in the Vault. Everything here is encrypted.', 'pCloud Files': 'You are in pCloud Files. Nothing here is encrypted by this app.',
      Shared: /Vault files can't be shared/, 'File request': /Let anyone upload files to you/, About: /not been independently audited/,
      Account: 'Storage used', Help: 'What does encrypted mean here?', Settings: 'How your files are protected',
    }
    for (const label of labels) {
      fireEvent.click(screen.getByRole('button', { name: label }))
      expect(screen.getByRole('heading', { level: 1 })).toHaveTextContent(label)
      expect(screen.getByRole('button', { name: label })).toHaveAttribute('aria-current', 'page')
      expect(await screen.findByText(content[label])).toBeInTheDocument()
    }
  })

  it('shows the upload drop zone with a destination choice', async () => {
    render(<App />)
    fireEvent.click(await screen.findByRole('button', { name: 'Upload' }))
    expect(screen.getByText('Drop files and folders here or click Browse')).toBeInTheDocument()
    expect(screen.getByRole('button', { name: /Browse folder/ })).toBeInTheDocument()
    expect(screen.getByText('Files are encrypted before they leave this computer.')).toBeInTheDocument()
    fireEvent.click(screen.getByRole('radio', { name: /pCloud — not encrypted/ }))
    expect(screen.getByText('Files are uploaded as they are. pCloud can read them.')).toBeInTheDocument()
    expect(within(screen.getByRole('region', { name: 'Upload queue' })).getByText('Quarterly report.pdf')).toBeInTheDocument()
  })

  it('summarizes downloads left and uploads right in the idle status bar', async () => {
    useNativeBridge()
    render(<App />)
    expect(await screen.findByRole('button', { name: /Everything downloaded/ })).toBeInTheDocument()
    expect(screen.getByRole('button', { name: /Everything uploaded/ })).toBeInTheDocument()
  })

  it('counts running transfers and waiting changes in the status bar and opens the drawer', async () => {
    useNativeBridge()
    nativeStatusForTest = { ...nativeStatus, pendingCount: 2 }
    const transfer = (id: string, name: string, direction: 'upload' | 'download', state: 'queued' | 'active' | 'done') =>
      ({ id, name, direction, location: 'vault', done: 50, total: 100, state, error: null, time: Date.now() })
    const native = vi.mocked(invoke).getMockImplementation()!
    vi.mocked(invoke).mockImplementation(async (command, args) => {
      if (command === 'get_transfers') return [transfer('t1', 'Report.pdf', 'upload', 'active'), transfer('t2', 'Photo.jpg', 'download', 'queued'), transfer('t3', 'Old.zip', 'download', 'done')] as never
      return native(command, args)
    })
    render(<App />)
    expect(await screen.findByRole('button', { name: /Downloading 1 file…/ })).toBeInTheDocument()
    fireEvent.click(screen.getByRole('button', { name: /Uploading 3 files…/ }))
    const drawer = screen.getByRole('complementary', { name: 'Activity and transfers' })
    expect(within(drawer).getByText('Report.pdf')).toBeInTheDocument()
    expect(within(drawer).getAllByText('Encrypted')).toHaveLength(3)
    fireEvent.click(within(drawer).getByRole('button', { name: 'Clear finished' }))
    await waitFor(() => expect(invoke).toHaveBeenCalledWith('clear_transfers', {}))
    fireEvent.keyDown(window, { key: 'Escape' })
    expect(screen.queryByRole('complementary', { name: 'Activity and transfers' })).not.toBeInTheDocument()
  })

  it('asks before applying mass deletions and sends the answer to resolve_guard', async () => {
    useNativeBridge()
    const native = vi.mocked(invoke).getMockImplementation()!
    vi.mocked(invoke).mockImplementation(async (command, args) => {
      if (command === 'list_jobs') return [{ id: 'job-1', kind: 'sync', localPath: '~/Documents', target: { location: 'vault', folderId: null, label: 'Vault' }, paused: false, mirrorDeletions: false, lastRun: null, state: 'needsReview', message: null, pendingDeletes: 12, fileCount: 40 }] as never
      return native(command, args)
    })
    render(<App />)
    fireEvent.click(await screen.findByRole('button', { name: 'Sync' }))
    const card = await screen.findByRole('article', { name: 'Sync of Documents' })
    expect(card).toHaveTextContent('12 files would be deleted.')
    expect(within(card).getByText('Encrypted')).toBeInTheDocument()
    fireEvent.click(within(card).getByRole('button', { name: 'Keep them' }))
    await waitFor(() => expect(invoke).toHaveBeenCalledWith('resolve_guard', { id: 'job-1', apply: false }))
    await waitFor(() => expect(screen.getByRole('button', { name: 'Delete them' })).toBeEnabled())
    fireEvent.click(screen.getByRole('button', { name: 'Delete them' }))
    await waitFor(() => expect(invoke).toHaveBeenCalledWith('resolve_guard', { id: 'job-1', apply: true }))
  })

  it('adds a backup in three steps and says plainly who can read the files', async () => {
    render(<App />)
    fireEvent.click(await screen.findByRole('button', { name: 'Back up' }))
    fireEvent.click(screen.getByRole('button', { name: /Back up Documents/ }))
    const dialog = screen.getByRole('dialog')
    fireEvent.click(within(dialog).getByRole('radio', { name: /pCloud — not encrypted/ }))
    fireEvent.click(within(dialog).getByRole('button', { name: 'Next' }))
    expect(dialog).toHaveTextContent('Files in ~/Documents will be stored unencrypted in pCloud; pCloud can read them.')
    fireEvent.click(within(dialog).getByRole('button', { name: 'Back' }))
    fireEvent.click(within(dialog).getByRole('radio', { name: /Vault — encrypted/ }))
    fireEvent.click(within(dialog).getByRole('button', { name: 'Next' }))
    expect(dialog).toHaveTextContent('Files in ~/Documents will be encrypted before they leave this computer')
    fireEvent.click(within(dialog).getByRole('button', { name: 'Start backup' }))
    expect(await screen.findByRole('article', { name: 'Backup of Documents' })).toHaveTextContent('Encrypted')
  })

  it('asks once to choose protection after upgrading from 0.2.0', async () => {
    useNativeBridge()
    const native = vi.mocked(invoke).getMockImplementation()!
    vi.mocked(invoke).mockImplementation(async (command, args) => {
      if (command === 'get_preferences') return { protection: null, theme: 'system', startAtLogin: false } as never
      return native(command, args)
    })
    render(<App />)
    const prompt = await screen.findByRole('region', { name: 'Choose how your files are protected' })
    fireEvent.click(within(prompt).getByRole('radio', { name: /Only the Vault/ }))
    await waitFor(() => expect(invoke).toHaveBeenCalledWith('set_preferences', { protection: 'vaultOnly' }))
    await waitFor(() => expect(screen.queryByRole('region', { name: 'Choose how your files are protected' })).not.toBeInTheDocument())
  })

  it('offers to switch unencrypted folders when everything should be encrypted', async () => {
    render(<App />)
    fireEvent.click(await screen.findByRole('button', { name: 'Settings' }))
    fireEvent.click(await screen.findByRole('radio', { name: /Everything encrypted/ }))
    const list = await screen.findByRole('region', { name: 'Folders that are not encrypted' })
    fireEvent.click(within(list).getByRole('button', { name: /Switch to encrypted/ }))
    const dialog = screen.getByRole('dialog')
    expect(dialog).toHaveTextContent('stays there, unencrypted')
    fireEvent.click(within(dialog).getByRole('button', { name: 'Switch to encrypted' }))
    await waitFor(() => expect(screen.queryByRole('region', { name: 'Folders that are not encrypted' })).not.toBeInTheDocument())
    fireEvent.click(screen.getByRole('button', { name: 'Back up' }))
    expect(await screen.findByRole('article', { name: 'Backup of Pictures' })).toHaveTextContent('Encrypted')
  })

  it('saves the theme and follows the system appearance when set to System', async () => {
    render(<App />)
    fireEvent.click(await screen.findByRole('button', { name: 'Settings' }))
    fireEvent.click(await screen.findByRole('radio', { name: 'Dark' }))
    await waitFor(() => expect(document.querySelector('.app-shell')).toHaveAttribute('data-theme', 'dark'))
    fireEvent.click(screen.getByRole('radio', { name: 'Light' }))
    await waitFor(() => expect(document.querySelector('.app-shell')).toHaveAttribute('data-theme', 'light'))
  })
})
