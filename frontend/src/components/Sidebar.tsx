import {
  CircleHelp, Cloud, CloudUpload, HardDriveUpload, Images, Inbox, Info, RefreshCw, Settings, ShieldCheck, UserRound, Users,
} from 'lucide-react'
import type { AppStatus } from '../types'

export type Section = 'upload' | 'backup' | 'sync' | 'photos' | 'vault' | 'cloud' | 'shared' | 'requests' | 'about' | 'account' | 'help' | 'settings'
export const SECTION_TITLES: Record<Section, string> = {
  upload: 'Upload', backup: 'Back up', sync: 'Sync', photos: 'Photos', vault: 'Vault', cloud: 'pCloud Files',
  shared: 'Shared', requests: 'File request', about: 'About', account: 'Account', help: 'Help', settings: 'Settings',
}
const MAIN: { id: Section; icon: typeof Cloud }[] = [
  { id: 'upload', icon: CloudUpload }, { id: 'backup', icon: HardDriveUpload }, { id: 'sync', icon: RefreshCw },
  { id: 'photos', icon: Images }, { id: 'vault', icon: ShieldCheck }, { id: 'cloud', icon: Cloud },
  { id: 'shared', icon: Users }, { id: 'requests', icon: Inbox }, { id: 'about', icon: Info },
]
const BOTTOM: { id: Section; icon: typeof Cloud }[] = [{ id: 'account', icon: UserRound }, { id: 'help', icon: CircleHelp }, { id: 'settings', icon: Settings }]

type Props = { section: Section; status: AppStatus | null; onNavigate: (section: Section) => void }

export function Sidebar({ section, status, onNavigate }: Props) {
  const vaultState = !status?.vaultExists ? 'Not set up' : status.unlocked ? 'Unlocked' : 'Locked'
  const item = ({ id, icon: Icon }: { id: Section; icon: typeof Cloud }) => {
    const label = SECTION_TITLES[id]
    return <button key={id} type="button" aria-label={label} title={id === 'vault' ? `Vault: ${vaultState}` : id === 'cloud' ? 'pCloud Files: not encrypted' : label}
      aria-current={section === id ? 'page' : undefined}
      className={`nav-link ${section === id ? 'selected' : ''} ${id === 'vault' ? 'nav-vault' : ''}`} onClick={() => onNavigate(id)}>
      <Icon size={18} aria-hidden="true" />
      {id === 'cloud'
        ? <span className="nav-label nav-stack"><span>{label}</span><small className="plain-tag">Not encrypted</small></span>
        : <span className="nav-label">{label}</span>}
      {id === 'vault' && <i className={`lock-dot ${vaultState === 'Unlocked' ? 'open' : vaultState === 'Locked' ? 'closed' : 'none'}`} aria-hidden="true" />}
    </button>
  }
  return <aside className="sidebar">
    <div className="brand"><div className="brand-mark"><ShieldCheck size={19} strokeWidth={2.2} /></div><div className="brand-text"><strong>pCloud <span>Vault</span></strong><small>Your files, yours</small></div></div>
    <nav className="sidebar-nav" aria-label="Sections">{MAIN.map(item)}</nav>
    <nav className="sidebar-bottom" aria-label="Account and settings">{BOTTOM.map(item)}</nav>
  </aside>
}
