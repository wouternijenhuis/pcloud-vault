import { useState } from 'react'
import { ChevronRight, Cloud, Monitor, Moon, Settings as SettingsIcon, ShieldCheck, Sun } from 'lucide-react'
import { runExtra } from '../api'
import { Dialog } from '../components/Dialog'
import { ProtectionBadge } from '../components/ProtectionBadge'
import { ProtectionChoice } from '../components/ProtectionChoice'
import type { AppStatus, FolderJob, Preferences, Protection, Theme } from '../types'
import type { Perform } from '../ui'

export type SettingsForm = { cacheLimitGib: number; clientId: string }
type Props = {
  status: AppStatus | null
  preferences: Preferences | null
  protection: Protection | null
  jobs: FolderJob[]
  settings: SettingsForm
  setSettings: (value: SettingsForm) => void
  save: () => void
  setPreferences: (value: Partial<Preferences>, success?: string) => Promise<boolean>
  connect: () => void
  recovery: () => void
  vaultAction: () => void
  password: () => void
  forget: () => void
  desktop: boolean
  busy: boolean
  perform: Perform
}
const THEMES: { id: Theme; label: string; icon: typeof Sun }[] = [{ id: 'system', label: 'System', icon: Monitor }, { id: 'light', label: 'Light', icon: Sun }, { id: 'dark', label: 'Dark', icon: Moon }]

export function SettingsPage({ status, preferences, protection, jobs, settings, setSettings, save, setPreferences, connect, recovery, vaultAction, password, forget, desktop, busy, perform }: Props) {
  const [switching, setSwitching] = useState<FolderJob | null>(null)
  const plainJobs = protection === 'everything' ? jobs.filter(job => job.target.location === 'cloud') : []
  const theme = preferences?.theme ?? 'system'
  const switchJob = (job: FolderJob) => perform(async () => {
    await runExtra('add_job', { kind: job.kind, localPath: job.localPath, location: 'vault', folderId: null, mirrorDeletions: job.mirrorDeletions, replaces: job.id })
  }, `${job.localPath} now goes to the Vault, encrypted`)
  return <div className="page settings-page">
    <div className="settings-intro"><div className="settings-icon"><SettingsIcon size={20} /></div><div><h2>Settings</h2><p>Protection, appearance, your Vault and this computer.</p></div></div>
    <div className="settings-layout">
      <section className="settings-card" aria-labelledby="protection-title">
        <div className="settings-card-title"><div><h3 id="protection-title">How your files are protected</h3><p>Decides where new uploads, backups and syncs go. Files that are already stored do not change.</p></div></div>
        <ProtectionChoice value={protection} disabled={busy} onChange={value => { if (value !== protection) void setPreferences({ protection: value }, 'Protection updated') }} />
        {plainJobs.length > 0 && <div className="switch-list" role="region" aria-label="Folders that are not encrypted">
          <p><strong>These folders still go to pCloud unencrypted.</strong> Switch them to keep everything encrypted.</p>
          <ul>{plainJobs.map(job => <li key={job.id}><span>{job.kind === 'sync' ? 'Sync' : 'Backup'} of <strong>{job.localPath}</strong> → {job.target.label}</span><ProtectionBadge location="cloud" /><button type="button" className="button button-primary" disabled={busy} onClick={() => setSwitching(job)}><ShieldCheck size={14} /> Switch to encrypted</button></li>)}</ul>
        </div>}
      </section>

      <section className="settings-card" aria-labelledby="appearance-title">
        <div className="settings-card-title"><div><h3 id="appearance-title">Appearance and startup</h3><p>How the app looks and when it starts.</p></div></div>
        <div className="settings-row"><div><strong>Theme</strong><small>System follows your computer's light or dark setting.</small></div>
          <div className="segmented compact" role="radiogroup" aria-label="Theme">{THEMES.map(item => { const Icon = item.icon; return <button key={item.id} type="button" role="radio" aria-checked={theme === item.id} className={theme === item.id ? 'active' : ''} onClick={() => { if (theme !== item.id) void setPreferences({ theme: item.id }) }}><Icon size={14} aria-hidden="true" />{item.label}</button> })}</div>
        </div>
        <div className="settings-row switch-row"><div><strong>Start when I sign in</strong><small>Keeps backups and syncs running in the background.</small></div><input type="checkbox" role="switch" aria-label="Start when I sign in" checked={preferences?.startAtLogin ?? false} disabled={!preferences || busy} onChange={event => void setPreferences({ startAtLogin: event.target.checked }, event.target.checked ? 'Starts when you sign in' : 'No longer starts when you sign in')} /></div>
      </section>

      <section className="settings-card" aria-labelledby="vault-title">
        <div className="settings-card-title"><div><h3 id="vault-title">Vault security</h3><p>On-device encryption and access.</p></div><span className="secure-pill"><ShieldCheck size={13} /> {!status?.vaultExists ? 'Not set up' : status.unlocked ? 'Unlocked' : 'Locked'}</span></div>
        <div className="settings-row"><div><strong>{status?.vaultExists ? 'Vault is set up' : 'Create your vault'}</strong><small>{status?.vaultExists ? 'Your password unlocks this space on this computer.' : 'Start with one password you will remember.'}</small></div><button className="button button-subtle" onClick={vaultAction}>{status?.vaultExists ? status.unlocked ? 'Lock vault' : 'Unlock' : 'Set up vault'}</button></div>
        <div className="settings-row"><div><strong>Change password</strong><small>Choose a new password for your private files.</small></div><button className="text-action" onClick={password}>Change <ChevronRight size={15} /></button></div>
        <div className="settings-row"><div><strong>Recovery key</strong><small>Export your recovery key to a safe place.</small></div><button className="text-action" onClick={recovery}>Export recovery key <ChevronRight size={15} /></button></div>
        <div className="settings-row"><div><strong>Forget this computer</strong><small>Remove remembered access from this computer.</small></div><button className="text-action danger-text" onClick={forget}>Forget device</button></div>
      </section>

      <section className="settings-card" aria-labelledby="cache-title">
        <div className="settings-card-title"><div><h3 id="cache-title">Local cache</h3><p>Choose the limit for files kept available offline.</p></div></div>
        <div className="settings-columns"><label className="field-label">Cache limit<select value={settings.cacheLimitGib} onChange={event => setSettings({ ...settings, cacheLimitGib: Number(event.target.value) })}>{[2, 5, 10, 20, 50].map(value => <option key={value} value={value}>{value} GB</option>)}</select></label></div>
        <div className="cache-detail"><div className="storage-track"><i style={{ width: `${status ? Math.min(100, status.cacheBytes / status.cacheLimitBytes * 100) : 0}%` }} /></div><small>{status ? `${(status.cacheBytes / 1024 ** 3).toFixed(1)} GB used of ${Math.round(status.cacheLimitBytes / 1024 ** 3)} GB` : 'Cache usage unavailable'}</small><small>Cached files stay encrypted on this computer.</small></div>
      </section>

      <section className="settings-card" aria-labelledby="advanced-title">
        <div className="settings-card-title"><div><h3 id="advanced-title">Advanced</h3><p>pCloud connection and your own pCloud app registration.</p></div><span className={`connection-pill ${status?.connected ? '' : 'offline'}`}><span />{status?.connected ? 'Connected' : 'Not connected'}</span></div>
        <div className="connected-account"><div className="cloud-account-icon"><Cloud size={19} /></div><div><strong>{status?.accountEmail ?? 'No pCloud account connected'}</strong><small>{status?.region ? `${status.region.toUpperCase()} region` : 'Sign in to choose your account region'}</small></div><button className="button button-subtle" onClick={connect}>{status?.connected ? 'Reconnect' : 'Connect pCloud'}</button></div>
        <div className="settings-row"><label className="field-label compact-field">OAuth client ID<input value={settings.clientId} onChange={event => setSettings({ ...settings, clientId: event.target.value })} placeholder="Your pCloud app client ID" /></label></div>
        <div className="settings-footer"><span>{desktop ? 'Saves the cache limit and OAuth client ID on this computer.' : 'Browser preview does not change local settings.'}</span><button className="button button-primary" onClick={save}>Save settings</button></div>
      </section>
    </div>
    <p className="app-origin"><span className="eu-badge">Made in Europe</span> pCloud Vault {__APP_VERSION__} · free and open source (GPL-3.0-only) · not affiliated with pCloud AG</p>
    {switching && <Dialog title="Switch to encrypted?" description={`New and changed files from ${switching.localPath} will go to the Vault root, encrypted, replacing this ${switching.kind}.`} onClose={() => setSwitching(null)} busy={busy} submitLabel="Switch to encrypted"
      onSubmit={event => { event.preventDefault(); const job = switching; void switchJob(job).then(done => { if (done) setSwitching(null) }) }}>
      <p className="hint">The copy already in {switching.target.label} stays there, unencrypted. pCloud can still read it. Delete it in pCloud Files when you no longer need it.</p>
    </Dialog>}
  </div>
}
