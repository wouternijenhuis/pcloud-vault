import { useEffect, useRef, useState } from 'react'
import { Cloud, HardDriveUpload, Images, LockKeyhole, ShieldCheck } from 'lucide-react'
import { listPhotos, photoThumb, type PhotoSource } from '../api'
import { PageHeader } from '../components/Gates'
import { ProtectionBadge } from '../components/ProtectionBadge'
import { errorMessage, monthLabel } from '../format'
import type { AppStatus, Location, Photo } from '../types'
import type { PreviewItem } from '../ui'

const THUMB_LIMIT = 300
const THUMB_EDGE = 256
/** Thumbnails live only in memory, never in browser storage; locking the vault clears them. */
const thumbs = new Map<string, string | null>()
export const clearThumbnailCache = () => thumbs.clear()
const remember = (key: string, value: string | null) => {
  thumbs.delete(key)
  thumbs.set(key, value)
  while (thumbs.size > THUMB_LIMIT) thumbs.delete(thumbs.keys().next().value as string)
}

/** Shrinks a decrypted Vault image so the full-size data URL is not kept around. */
function downscale(url: string) {
  return new Promise<string>(resolve => {
    if (url.startsWith('data:image/svg')) { resolve(url); return }
    const image = new Image()
    image.onload = () => {
      const scale = Math.min(1, THUMB_EDGE / Math.max(image.naturalWidth || 1, image.naturalHeight || 1))
      const canvas = document.createElement('canvas')
      canvas.width = Math.max(1, Math.round((image.naturalWidth || 1) * scale))
      canvas.height = Math.max(1, Math.round((image.naturalHeight || 1) * scale))
      let context: CanvasRenderingContext2D | null = null
      try { context = canvas.getContext('2d') } catch { context = null }
      if (!context) { resolve(url); return }
      context.drawImage(image, 0, 0, canvas.width, canvas.height)
      try { resolve(canvas.toDataURL('image/jpeg', 0.82)) } catch { resolve(url) }
    }
    image.onerror = () => resolve(url)
    image.src = url
  })
}

async function loadThumb(location: Location, id: string) {
  const key = `${location}:${id}`
  if (thumbs.has(key)) return thumbs.get(key) ?? null
  const raw = await photoThumb(location, id)
  const value = typeof raw === 'string' && raw ? location === 'vault' && raw.startsWith('data:') ? await downscale(raw) : raw : null
  remember(key, value)
  return value
}

function Thumb({ photo, onOpen }: { photo: Photo; onOpen: () => void }) {
  const ref = useRef<HTMLButtonElement>(null)
  const [url, setUrl] = useState<string | null>(() => thumbs.get(`${photo.location}:${photo.id}`) ?? null)
  const [visible, setVisible] = useState(false)
  useEffect(() => {
    const element = ref.current
    if (!element || typeof IntersectionObserver === 'undefined') { setVisible(true); return }
    const observer = new IntersectionObserver(items => { if (items.some(item => item.isIntersecting)) { setVisible(true); observer.disconnect() } }, { rootMargin: '200px' })
    observer.observe(element)
    return () => observer.disconnect()
  }, [])
  useEffect(() => {
    if (!visible) return
    let cancelled = false
    loadThumb(photo.location, photo.id).then(value => { if (!cancelled) setUrl(value) }).catch(() => undefined)
    return () => { cancelled = true }
  }, [visible, photo.location, photo.id])
  return <button ref={ref} type="button" className="photo-tile" onClick={onOpen} aria-label={`${photo.name}, ${photo.location === 'vault' ? 'encrypted in the Vault' : 'not encrypted, in pCloud'}`} title={photo.name}>
    {url ? <img src={url} alt="" loading="lazy" /> : <span className="photo-placeholder"><Images size={22} aria-hidden="true" /></span>}
    <ProtectionBadge location={photo.location} compact className="photo-badge" />
  </button>
}

type Props = { status: AppStatus | null; dataVersion: number; openPreview: (item: PreviewItem) => void; onBackUpPictures: () => void; openUnlock: () => void }
const SOURCES: { id: PhotoSource; label: string; icon: typeof Cloud }[] = [
  { id: 'all', label: 'All', icon: Images }, { id: 'vault', label: 'Vault', icon: ShieldCheck }, { id: 'cloud', label: 'pCloud', icon: Cloud },
]

export function PhotosPage({ status, dataVersion, openPreview, onBackUpPictures, openUnlock }: Props) {
  const [source, setSource] = useState<PhotoSource>('all')
  const [photos, setPhotos] = useState<Photo[] | null>(null)
  const [problem, setProblem] = useState('')
  const unlocked = status?.unlocked ?? false
  useEffect(() => {
    let cancelled = false
    setProblem('')
    listPhotos(source).then(items => { if (!cancelled) setPhotos(items) }).catch(reason => { if (!cancelled) { setPhotos([]); setProblem(errorMessage(reason, 'Could not load photos.')) } })
    return () => { cancelled = true }
  }, [source, unlocked, dataVersion])
  const groups: { month: string; items: Photo[] }[] = []
  for (const photo of [...(photos ?? [])].sort((a, b) => b.modified - a.modified)) {
    const month = monthLabel(photo.modified)
    const group = groups.at(-1)
    if (group?.month === month) group.items.push(photo); else groups.push({ month, items: [photo] })
  }
  return <div className="page photos-page">
    <PageHeader title="Photos" description="Pictures from your Vault and from pCloud, newest first. Each one shows whether it is encrypted." />
    <div className="segmented" role="group" aria-label="Photo source">{SOURCES.map(item => { const Icon = item.icon; return <button key={item.id} type="button" aria-pressed={source === item.id} className={source === item.id ? 'active' : ''} onClick={() => setSource(item.id)}><Icon size={15} aria-hidden="true" />{item.label}{item.id !== 'all' && <ProtectionBadge location={item.id} compact />}</button> })}</div>
    {source !== 'cloud' && status?.vaultExists && !unlocked && <div className="notice"><LockKeyhole size={15} aria-hidden="true" /><span>Encrypted Vault photos appear after you unlock the Vault.</span><button type="button" className="link-button" onClick={openUnlock}>Unlock</button></div>}
    {problem && <p className="mini-empty" role="alert">{problem}</p>}
    {photos === null ? <p className="mini-empty">Loading photos…</p>
      : groups.length === 0 ? <div className="empty-state"><img src="/art-empty.svg" alt="" /><h3>No photos yet</h3><p>Back up your Pictures folder and your photos show up here, grouped by month.</p><div className="empty-actions"><button type="button" className="button button-primary" onClick={onBackUpPictures}><HardDriveUpload size={16} /> Back up your photos</button></div></div>
        : groups.map(group => <section key={group.month} className="photo-month" aria-label={group.month}>
          <h3>{group.month}</h3>
          <div className="photo-grid">{group.items.map(photo => <Thumb key={`${photo.location}:${photo.id}`} photo={photo} onOpen={() => openPreview({ location: photo.location, id: photo.id, name: photo.name, size: photo.size, modified: photo.modified })} />)}</div>
        </section>)}
  </div>
}
