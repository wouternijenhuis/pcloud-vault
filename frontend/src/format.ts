import type { Location, Protection } from './types'

export const formatSize = (bytes: number) =>
  bytes < 1024 ? `${bytes} B`
    : bytes < 1024 ** 2 ? `${(bytes / 1024).toFixed(0)} KB`
      : bytes < 1024 ** 3 ? `${(bytes / 1024 ** 2).toFixed(1)} MB`
        : bytes < 1024 ** 4 ? `${(bytes / 1024 ** 3).toFixed(1)} GB`
          : `${(bytes / 1024 ** 4).toFixed(2)} TB`
export const timeLabel = (time: number) => new Intl.DateTimeFormat(undefined, { month: 'short', day: 'numeric' }).format(new Date(time))
export const dateTimeLabel = (time: number) => new Intl.DateTimeFormat(undefined, { dateStyle: 'medium', timeStyle: 'short' }).format(new Date(time))
export const monthLabel = (time: number) => new Intl.DateTimeFormat(undefined, { month: 'long', year: 'numeric' }).format(new Date(time))
export const isImageName = (name: string) => /\.(png|jpe?g|gif|webp)$/i.test(name)
export const plural = (count: number, one: string, many = `${one}s`) => `${count} ${count === 1 ? one : many}`
export const rootLabel = (location: Location) => location === 'vault' ? 'Vault' : 'pCloud'

/** Tauri commands reject with the backend's message as a plain string, not an Error. */
export function errorMessage(reason: unknown, fallback: string) {
  if (reason instanceof Error && reason.message) return reason.message
  if (typeof reason === 'string' && reason.trim()) return reason
  return fallback
}

/** The destination every new upload, backup or sync must use, or null when the user chooses. */
export function lockedLocation(protection: Protection | null): Location | null {
  if (protection === 'everything') return 'vault'
  if (protection === 'vaultOnly') return 'cloud'
  return null
}

export const PROTECTION_LABELS: Record<Protection, string> = {
  everything: 'Everything encrypted',
  choose: 'Let me choose each time',
  vaultOnly: 'Only the Vault',
}

export async function copyText(text: string) {
  try {
    await navigator.clipboard.writeText(text)
    return true
  } catch {
    try {
      const area = document.createElement('textarea')
      area.value = text
      area.setAttribute('readonly', '')
      area.style.position = 'fixed'
      area.style.opacity = '0'
      document.body.appendChild(area)
      area.select()
      const copied = document.execCommand('copy')
      area.remove()
      return copied
    } catch { return false }
  }
}
