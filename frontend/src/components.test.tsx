import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import { cleanup, fireEvent, render, screen } from '@testing-library/react'
import { DestinationPicker, rootTarget } from './components/DestinationPicker'
import { ProtectionBadge } from './components/ProtectionBadge'
import { FileRequestDialog, FILE_REQUEST_NOTE } from './pages/FileRequestPage'
import { resetDemo, runAction } from './api'

vi.mock('@tauri-apps/api/core', () => ({ invoke: vi.fn() }))

beforeEach(() => { resetDemo(); delete window.__TAURI_INTERNALS__ })
afterEach(cleanup)

describe('ProtectionBadge', () => {
  it('says who can read a file', () => {
    render(<><ProtectionBadge location="vault" /><ProtectionBadge location="cloud" /></>)
    expect(screen.getByText('Encrypted').closest('.protection-badge')).toHaveAttribute('title', 'Only you can read this')
    expect(screen.getByText('Not encrypted').closest('.protection-badge')).toHaveAttribute('title', 'pCloud can read this')
  })
})

describe('DestinationPicker', () => {
  it('is locked to the Vault when everything is encrypted', () => {
    const onChange = vi.fn()
    const openSettings = vi.fn()
    render(<DestinationPicker protection="everything" value={rootTarget('vault')} onChange={onChange} onOpenSettings={openSettings} />)
    expect(screen.getByRole('radio', { name: /Vault — encrypted/ })).toHaveAttribute('aria-checked', 'true')
    expect(screen.getByRole('radio', { name: /pCloud — not encrypted/ })).toBeDisabled()
    expect(screen.getByText(/Everything is encrypted, so new files always go to the Vault/)).toBeInTheDocument()
    fireEvent.click(screen.getByRole('button', { name: 'Change in Settings' }))
    expect(openSettings).toHaveBeenCalled()
    expect(onChange).not.toHaveBeenCalled()
  })

  it('moves the destination to pCloud and locks it there for Only the Vault', () => {
    const onChange = vi.fn()
    render(<DestinationPicker protection="vaultOnly" value={rootTarget('vault')} onChange={onChange} />)
    expect(onChange).toHaveBeenCalledWith({ location: 'cloud', folderId: null, label: 'pCloud' })
    expect(screen.getByRole('radio', { name: /Vault — encrypted/ })).toBeDisabled()
    expect(screen.getByRole('radio', { name: /pCloud — not encrypted/ })).toBeEnabled()
    expect(screen.getByText(/go to pCloud unencrypted/)).toBeInTheDocument()
  })

  it('offers a free choice when the user chooses each time', () => {
    const onChange = vi.fn()
    render(<DestinationPicker protection="choose" value={rootTarget('vault')} onChange={onChange} />)
    expect(screen.getByRole('radio', { name: /Vault — encrypted/ })).toBeEnabled()
    fireEvent.click(screen.getByRole('radio', { name: /pCloud — not encrypted/ }))
    expect(onChange).toHaveBeenCalledWith({ location: 'cloud', folderId: null, label: 'pCloud' })
    expect(screen.queryByRole('button', { name: 'Change in Settings' })).not.toBeInTheDocument()
  })

  it('chooses a folder inside the selected side', async () => {
    const onChange = vi.fn()
    render(<DestinationPicker protection="choose" value={rootTarget('vault')} onChange={onChange} folders />)
    fireEvent.click(await screen.findByRole('button', { name: 'Open Work' }))
    expect(onChange).toHaveBeenLastCalledWith({ location: 'vault', folderId: 'f-work', label: 'Vault / Work' })
  })
})

describe('FolderChooser refresh', () => {
  it('lists folders again when the refresh key changes, for example after an upload finishes', async () => {
    const props = { protection: 'choose' as const, value: rootTarget('cloud'), onChange: vi.fn(), folders: true }
    const { rerender } = render(<DestinationPicker {...props} refreshKey="0:" />)
    expect(await screen.findByRole('button', { name: 'Open Camera roll' })).toBeInTheDocument()
    await runAction('new_folder', { location: 'cloud', parentId: null, name: 'Uploaded folder' })
    rerender(<DestinationPicker {...props} refreshKey="0:" />)
    expect(screen.queryByRole('button', { name: 'Open Uploaded folder' })).not.toBeInTheDocument()
    rerender(<DestinationPicker {...props} refreshKey="0:t-upload" />)
    expect(await screen.findByRole('button', { name: 'Open Uploaded folder' })).toBeInTheDocument()
  })
})

describe('FileRequestDialog', () => {
  it('keeps "Move received files into the Vault" on and locked when everything is encrypted', async () => {
    render(<FileRequestDialog protection="everything" busy={false} onClose={vi.fn()} perform={vi.fn()} />)
    const toggle = screen.getByRole('switch', { name: /Move received files into the Vault/ })
    expect(toggle).toBeChecked()
    expect(toggle).toBeDisabled()
    expect(screen.getByText(FILE_REQUEST_NOTE)).toBeInTheDocument()
    expect(await screen.findByRole('button', { name: 'Open Camera roll' })).toBeInTheDocument()
  })

  it('lets the user decide when choosing each time', async () => {
    const perform = vi.fn(async (task: () => Promise<unknown>) => { await task(); return true })
    render(<FileRequestDialog protection="choose" busy={false} onClose={vi.fn()} perform={perform} />)
    const toggle = screen.getByRole('switch', { name: /Move received files into the Vault/ })
    expect(toggle).toBeEnabled()
    fireEvent.click(toggle)
    expect(toggle).not.toBeChecked()
    fireEvent.click(await screen.findByRole('button', { name: 'Open Client uploads' }))
    fireEvent.click(screen.getByRole('button', { name: 'Create file request' }))
    await vi.waitFor(() => expect(perform).toHaveBeenCalled())
  })
})
