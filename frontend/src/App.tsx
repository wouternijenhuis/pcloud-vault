import { useCallback, useEffect, useRef, useState, type FormEvent } from 'react'
import { ArrowDownToLine, ArrowRight, Check, FileImage, Globe, LockKeyhole, RefreshCw, Search, ShieldCheck, X } from 'lucide-react'
import { Dialog } from './components/Dialog'
import { WelcomeWizard } from './components/WelcomeWizard'
import { Sidebar, SECTION_TITLES, type Section } from './components/Sidebar'
import { ActivityDrawer, StatusBar } from './components/StatusBar'
import { FolderJobDialog } from './components/FolderJobDialog'
import { ProtectionBadge } from './components/ProtectionBadge'
import { ProtectionChoice } from './components/ProtectionChoice'
import {
  getActivity, getKnownFolders, getPreferences, getStatus, getTransfers, isDesktop, listJobs, pickFolder, previewEntry, runAction, runExtra,
} from './api'
import { errorMessage, formatSize, isImageName, timeLabel } from './format'
import { AboutPage } from './pages/AboutPage'
import { AccountPage } from './pages/AccountPage'
import { FileRequestPage } from './pages/FileRequestPage'
import { FilesPage } from './pages/FilesPage'
import { HelpPage } from './pages/HelpPage'
import { BackupPage, SyncPage } from './pages/JobsPage'
import { PhotosPage, clearThumbnailCache } from './pages/PhotosPage'
import { SettingsPage, type SettingsForm } from './pages/SettingsPage'
import { SharedPage } from './pages/SharedPage'
import { UploadPage } from './pages/UploadPage'
import type { Activity, AppStatus, ExtraArguments, FolderJob, JobKind, KnownFolders, Preferences, Transfer } from './types'
import type { Perform, PreviewItem } from './ui'

type Modal = 'password' | 'connect' | 'unlock' | 'create' | null
const WELCOME_KEY = 'pcloud-vault.welcome-complete'
function welcomeCompleted() {
  try { return localStorage.getItem(WELCOME_KEY) === '1' } catch { return false }
}
const isStatus = (value: unknown): value is AppStatus => !!value && typeof value === 'object' && 'unlocked' in value
const isPreferences = (value: unknown): value is Preferences => !!value && typeof value === 'object' && 'theme' in value
const prefersDark = () => typeof window.matchMedia === 'function' && window.matchMedia('(prefers-color-scheme: dark)').matches

export default function App() {
  const desktop = isDesktop()
  const [status, setStatus] = useState<AppStatus | null>(null)
  const [preferences, setPreferences] = useState<Preferences | null>(null)
  const [section, setSection] = useState<Section>('vault')
  const [lastFiles, setLastFiles] = useState<'vault' | 'cloud'>('vault')
  const [query, setQuery] = useState('')
  const [jobs, setJobs] = useState<FolderJob[]>([])
  const [transfers, setTransfers] = useState<Transfer[]>([])
  const [activity, setActivity] = useState<Activity[]>([])
  const [knownFolders, setKnownFolders] = useState<KnownFolders | null>(null)
  const [modal, setModal] = useState<Modal>(null)
  const [preview, setPreview] = useState<PreviewItem | null>(null)
  const [previewUrl, setPreviewUrl] = useState<string | null>(null)
  const [busy, setBusy] = useState(false)
  const [error, setError] = useState('')
  const [toast, setToast] = useState('')
  const [drawer, setDrawer] = useState(false)
  const [jobDialog, setJobDialog] = useState<{ kind: JobKind; path?: string } | null>(null)
  const [dataVersion, setDataVersion] = useState(0)
  const [systemDark, setSystemDark] = useState(prefersDark)
  const [askLater, setAskLater] = useState(false)
  const [localOnly, setLocalOnly] = useState(false)
  // Decided once, from the first status: the guide runs on a computer that has no vault yet.
  const [wizard, setWizard] = useState<boolean | null>(null)
  const [settings, setSettings] = useState<SettingsForm>({ cacheLimitGib: 10, clientId: '' })
  const [settingsInitialized, setSettingsInitialized] = useState(false)
  const [formName, setFormName] = useState('')
  const [password, setPassword] = useState('')
  const [remember, setRemember] = useState(true)
  const statusRequest = useRef(0)
  const previewRequest = useRef(0)
  const statusRef = useRef<AppStatus | null>(null)
  const busyRef = useRef(false)
  const toastTimer = useRef(0)
  const searchInput = useRef<HTMLInputElement>(null)
  const [focusSearch, setFocusSearch] = useState(false)

  const applyStatus = useCallback((value: AppStatus) => {
    if (statusRef.current?.unlocked && !value.unlocked) {
      previewRequest.current++
      clearThumbnailCache()
      setModal(null); setPreview(null); setPreviewUrl(null); setFormName(''); setPassword('')
    }
    statusRef.current = value
    setStatus(value)
  }, [])
  const refreshStatus = useCallback(async () => {
    const request = ++statusRequest.current
    try { const value = await getStatus(); if (request === statusRequest.current && isStatus(value)) applyStatus(value) }
    catch (reason) { if (request === statusRequest.current) setError(errorMessage(reason, 'Could not read vault status.')) }
  }, [applyStatus])
  const announce = useCallback((message: string) => {
    setToast(message)
    window.clearTimeout(toastTimer.current)
    toastTimer.current = window.setTimeout(() => setToast(''), 2800)
  }, [])
  const perform: Perform = useCallback(async (task, success) => {
    if (busyRef.current) return false
    busyRef.current = true; setBusy(true); setError('')
    try {
      const result = await task()
      if (isStatus(result)) applyStatus(result)
      await refreshStatus()
      setDataVersion(value => value + 1)
      if (success) announce(success)
      return true
    }
    catch (reason) { setError(errorMessage(reason, 'That action could not be completed.')); return false }
    finally { busyRef.current = false; setBusy(false) }
  }, [announce, applyStatus, refreshStatus])
  const savePreferences = useCallback(async (value: Partial<Preferences>, success?: string) => {
    const saved: { value?: Preferences } = {}
    const done = await perform(async () => { saved.value = await runExtra('set_preferences', value) }, success)
    if (done) setPreferences(current => isPreferences(saved.value) ? saved.value : { ...(current ?? { protection: null, theme: 'system', startAtLogin: false }), ...value })
    return done
  }, [perform])

  useEffect(() => { void refreshStatus(); const id = window.setInterval(() => { void refreshStatus() }, 5000); return () => { window.clearInterval(id); statusRequest.current++ } }, [refreshStatus])
  useEffect(() => () => { previewRequest.current++; window.clearTimeout(toastTimer.current) }, [])
  const unlocked = status?.unlocked ?? false
  useEffect(() => {
    let cancelled = false
    getPreferences().then(value => { if (!cancelled && isPreferences(value)) setPreferences(value) }).catch(() => undefined)
    return () => { cancelled = true }
  }, [unlocked])
  useEffect(() => {
    let cancelled = false
    getKnownFolders().then(value => { if (!cancelled && value && typeof value === 'object') setKnownFolders(value) }).catch(() => undefined)
    return () => { cancelled = true }
  }, [])
  useEffect(() => {
    let cancelled = false
    const load = () => { listJobs().then(value => { if (!cancelled) setJobs(value) }).catch(() => undefined) }
    load(); const id = window.setInterval(load, 5000)
    return () => { cancelled = true; window.clearInterval(id) }
  }, [dataVersion, unlocked])
  useEffect(() => {
    let cancelled = false
    const load = () => { getTransfers().then(value => { if (!cancelled) setTransfers(value) }).catch(() => undefined) }
    load(); const id = window.setInterval(load, 2500)
    return () => { cancelled = true; window.clearInterval(id) }
  }, [dataVersion])
  useEffect(() => {
    if (!drawer) return
    let cancelled = false
    const load = () => { getActivity().then(value => { if (!cancelled) setActivity(Array.isArray(value) ? value : []) }).catch(() => undefined) }
    load(); const id = window.setInterval(load, 5000)
    return () => { cancelled = true; window.clearInterval(id) }
  }, [drawer, dataVersion])
  useEffect(() => { if (status && !settingsInitialized) { setSettings({ cacheLimitGib: Math.round(status.cacheLimitBytes / 1024 ** 3), clientId: status.oauthClientId }); setSettingsInitialized(true) } }, [status, settingsInitialized])
  // Theme: System follows the operating system and keeps following it while the app runs.
  useEffect(() => {
    if (typeof window.matchMedia !== 'function') return
    const media = window.matchMedia('(prefers-color-scheme: dark)')
    const change = (event: MediaQueryListEvent) => setSystemDark(event.matches)
    media.addEventListener?.('change', change)
    return () => media.removeEventListener?.('change', change)
  }, [])
  const theme = preferences?.theme ?? 'system'
  const dark = theme === 'dark' || (theme === 'system' && systemDark)
  useEffect(() => { document.documentElement.dataset.theme = dark ? 'dark' : 'light' }, [dark])
  useEffect(() => {
    // Search only exists on the file pages; from any other page Ctrl+K opens the last file page and focuses its search.
    const shortcut = (event: KeyboardEvent) => {
      if (!(event.ctrlKey || event.metaKey) || event.key.toLowerCase() !== 'k') return
      // Never leave a page while the user is typing in one of its forms (or a dialog is open).
      const typing = event.target instanceof HTMLElement && event.target.closest('input, textarea, select, [contenteditable="true"], [role="dialog"]') && event.target !== searchInput.current
      if (typing) return
      event.preventDefault()
      setSection(current => current === 'vault' || current === 'cloud' ? current : lastFiles)
      setFocusSearch(true)
    }
    window.addEventListener('keydown', shortcut)
    return () => window.removeEventListener('keydown', shortcut)
  }, [lastFiles])
  const fileSection = section === 'vault' || section === 'cloud'
  useEffect(() => { if (focusSearch && fileSection) { searchInput.current?.focus(); setFocusSearch(false) } }, [focusSearch, fileSection])
  useEffect(() => {
    // The WebView's own menu (Back, Refresh, Print…) does not belong in a desktop app; keep it for text fields.
    const suppress = (event: MouseEvent) => { if (!(event.target as HTMLElement).closest('input, textarea, [contenteditable="true"]')) event.preventDefault() }
    document.addEventListener('contextmenu', suppress)
    return () => document.removeEventListener('contextmenu', suppress)
  }, [])
  useEffect(() => { if (status && wizard === null) setWizard(desktop && !status.vaultExists && !welcomeCompleted()) }, [status, wizard, desktop])
  // While the guide is open, the app behind it cannot be focused or clicked.
  useEffect(() => {
    document.querySelectorAll('.app-shell > aside, .app-shell > main').forEach(element => element.toggleAttribute('inert', !!wizard))
  }, [wizard])

  const protection = preferences?.protection ?? null
  const navigate = (next: Section) => { setSection(next); setQuery(''); if (next === 'vault' || next === 'cloud') setLastFiles(next) }
  const openConnect = () => { setError(''); setModal('connect') }
  const openVaultDialog = (kind: 'create' | 'unlock') => setModal(kind)
  const openUnlock = () => setModal(status?.vaultExists ? 'unlock' : 'create')
  const openPreview = async (item: PreviewItem) => {
    const request = ++previewRequest.current
    setPreview(item); setPreviewUrl(null)
    if (!isImageName(item.name)) return
    try { const url = await previewEntry(item.location, item.id); if (request === previewRequest.current) setPreviewUrl(url) }
    catch { if (request === previewRequest.current) setPreviewUrl(null) }
  }
  const closePreview = useCallback(() => { previewRequest.current++; setPreview(null); setPreviewUrl(null) }, [])
  const exportItem = async (item: PreviewItem, open: boolean) => {
    if (!desktop) { announce(open ? 'Export and open is available in the desktop app' : 'Export copies are available in the desktop app'); return }
    await perform(() => open ? runAction('open_entry', { location: item.location, id: item.id }) : runAction('download_entry', { location: item.location, id: item.id }), open ? 'Exported copy opened' : 'Export complete')
  }
  const exportRecovery = () => { if (desktop) void perform(() => runAction('export_recovery', {}), 'Recovery key exported'); else announce('Recovery key export is available in the desktop app') }
  const lockVault = () => void perform(() => runAction('lock_vault', {}), 'Vault locked')
  const onSubmit = async (event: FormEvent<HTMLFormElement>) => {
    event.preventDefault()
    if (modal === 'create') await perform(() => runAction('create_vault', { password, remember }), 'Your private vault is ready')
    if (modal === 'unlock') await perform(() => runAction('unlock_vault', { password, remember }), 'Vault unlocked')
    if (modal === 'password') await perform(() => runAction('change_password', { oldPassword: formName, newPassword: password }), 'Password changed')
    if (modal === 'connect') await perform(() => runAction('connect_pcloud', { clientId: settings.clientId }), 'pCloud connected')
    setPassword(''); setFormName(''); setModal(null)
  }
  const finishWizard = () => {
    try { localStorage.setItem(WELCOME_KEY, '1') } catch { /* the guide simply shows again next time */ }
    setError(''); setWizard(false)
  }
  const connectWeb = async () => {
    if (!desktop) { announce('Website sign-in is available in the desktop app'); return }
    if (await perform(() => runAction('connect_pcloud_web', {}), 'pCloud connected')) setModal(null)
  }
  const modalClose = useCallback(() => { if (!busyRef.current) setModal(null) }, [])
  const addJob = (args: ExtraArguments['add_job']) => perform(() => runExtra('add_job', args), args.kind === 'sync' ? 'Sync added' : 'Backup added')
  const showProtectionPrompt = !!preferences && preferences.protection === null && unlocked && !wizard && !askLater

  const page = (() => {
    switch (section) {
      case 'upload': return <UploadPage status={status} protection={protection} transfers={transfers} desktop={desktop} busy={busy} dataVersion={dataVersion} perform={perform} openSettings={() => navigate('settings')} openConnect={openConnect} openUnlock={openUnlock} />
      case 'backup': return <BackupPage jobs={jobs} knownFolders={knownFolders} busy={busy} perform={perform} onAdd={path => setJobDialog({ kind: 'backup', path })} />
      case 'sync': return <SyncPage jobs={jobs} knownFolders={knownFolders} busy={busy} perform={perform} onAdd={path => setJobDialog({ kind: 'sync', path })} />
      case 'photos': return <PhotosPage status={status} dataVersion={dataVersion} openPreview={item => void openPreview(item)} onBackUpPictures={() => setJobDialog({ kind: 'backup', path: knownFolders?.pictures ?? undefined })} openUnlock={openUnlock} />
      case 'vault': case 'cloud': return <FilesPage key={section} location={section} status={status} query={query} desktop={desktop} busy={busy} dataVersion={dataVersion} perform={perform} announce={announce} setError={setError} openPreview={item => void openPreview(item)} openConnect={openConnect} openVaultDialog={openVaultDialog} localOnly={localOnly} workLocally={() => setLocalOnly(true)} />
      case 'shared': return <SharedPage status={status} busy={busy} dataVersion={dataVersion} perform={perform} announce={announce} openConnect={openConnect} />
      case 'requests': return <FileRequestPage status={status} protection={protection} busy={busy} dataVersion={dataVersion} perform={perform} announce={announce} openConnect={openConnect} />
      case 'about': return <AboutPage perform={perform} />
      case 'account': return <AccountPage status={status} busy={busy} dataVersion={dataVersion} perform={perform} openConnect={openConnect} />
      case 'help': return <HelpPage status={status} protection={protection} jobs={jobs} transfers={transfers} perform={perform} announce={announce} />
      case 'settings': return <SettingsPage status={status} preferences={preferences} protection={protection} jobs={jobs} settings={settings} setSettings={setSettings}
        save={() => void perform(() => runAction('save_settings', settings), 'Settings saved')} setPreferences={savePreferences} connect={openConnect} recovery={exportRecovery}
        vaultAction={() => { if (status?.vaultExists && status.unlocked) lockVault(); else setModal(status?.vaultExists ? 'unlock' : 'create') }}
        password={() => setModal('password')} forget={() => void perform(() => runAction('forget_device', {}), 'This computer was forgotten')} desktop={desktop} busy={busy} perform={perform} />
    }
  })()

  return <div className="app-shell" data-theme={dark ? 'dark' : 'light'}>
    <Sidebar section={section} status={status} onNavigate={navigate} />
    <main className="main-area">
      {!desktop && <div className="demo-banner"><span className="demo-dot" /> Browser preview <span>DEMO MODE</span><small>Sample data only — no cloud changes are made.</small></div>}
      <header className="topbar">
        <h1>{SECTION_TITLES[section]}</h1>
        <div className="top-actions">
          {fileSection && <label className="search-box"><Search size={17} aria-hidden="true" /><input ref={searchInput} aria-label="Search files" placeholder={section === 'cloud' ? 'Search pCloud Files' : 'Search the Vault'} value={query} onChange={event => setQuery(event.target.value)} /><kbd>Ctrl K</kbd></label>}
          <button className="icon-button bordered" title="Sync now" aria-label="Sync now" onClick={() => void perform(() => runAction('sync_now', {}), 'Everything is up to date')}><RefreshCw size={17} /></button>
          {unlocked && <button className="icon-button bordered" title="Lock vault" aria-label="Lock vault" onClick={lockVault}><LockKeyhole size={17} /></button>}
        </div>
      </header>
      <div className="page-scroll">
        {error && <div className="alert error-alert" role="alert"><span>{error}</span><button onClick={() => setError('')} aria-label="Dismiss error"><X size={16} /></button></div>}
        {showProtectionPrompt && <section className="protection-prompt" aria-label="Choose how your files are protected">
          <div><h2>New: choose how your files are protected</h2><p>This version can back up, sync and upload whole folders. Decide where new files go. Nothing you already stored changes.</p></div>
          <ProtectionChoice value={null} disabled={busy} onChange={value => void savePreferences({ protection: value }, 'Protection saved')} />
          <button type="button" className="text-action" onClick={() => setAskLater(true)}>Decide later</button>
        </section>}
        {page}
      </div>
      <StatusBar transfers={transfers} pendingCount={status?.pendingCount ?? 0} open={drawer} onToggle={() => setDrawer(value => !value)} />
    </main>
    {drawer && <ActivityDrawer transfers={transfers} activity={activity} onClose={() => setDrawer(false)} onClear={() => void perform(() => runExtra('clear_transfers', {}))} />}

    {wizard && status && <WelcomeWizard status={status} busy={busy} error={error}
      connect={() => perform(() => runAction('connect_pcloud_web', {}), 'pCloud connected')}
      createVault={(secret, rememberPc) => perform(() => runAction('create_vault', { password: secret, remember: rememberPc }), 'Your private vault is ready')}
      unlockVault={(secret, rememberPc) => perform(() => runAction('unlock_vault', { password: secret, remember: rememberPc }), 'Vault unlocked')}
      exportRecovery={exportRecovery} workLocally={() => setLocalOnly(true)} finish={finishWizard}
      protection={protection} chooseProtection={value => savePreferences({ protection: value })}
      knownFolders={knownFolders} pickFolder={pickFolder} addJob={addJob} />}
    {jobDialog && <FolderJobDialog kind={jobDialog.kind} initialPath={jobDialog.path} protection={protection} knownFolders={knownFolders} busy={busy} pickFolder={pickFolder}
      onClose={() => setJobDialog(null)} onOpenSettings={() => navigate('settings')}
      onSubmit={async args => { const done = await addJob(args); if (done && section !== args.kind) navigate(args.kind); return done }} />}
    {modal === 'create' && <Dialog title="Create your private vault" description="Choose a strong password with at least 12 characters. Files in your vault stay encrypted on this computer." onClose={modalClose} onSubmit={onSubmit} submitLabel="Create vault" busy={busy}><label className="field-label">Vault password<input required type="password" minLength={12} value={password} onChange={event => setPassword(event.target.value)} autoComplete="new-password" placeholder="At least 12 characters" /></label><label className="check-label"><input type="checkbox" checked={remember} onChange={event => setRemember(event.target.checked)} />Remember this computer so you can unlock faster</label></Dialog>}
    {modal === 'unlock' && <Dialog title="Unlock your vault" description="Enter your vault password to continue." onClose={modalClose} onSubmit={onSubmit} submitLabel="Unlock vault" busy={busy}><label className="field-label">Vault password<input required type="password" minLength={12} value={password} onChange={event => setPassword(event.target.value)} autoComplete="current-password" /></label><label className="check-label"><input type="checkbox" checked={remember} onChange={event => setRemember(event.target.checked)} />Remember this computer</label></Dialog>}
    {modal === 'password' && <Dialog title="Change vault password" description="This wraps your existing vault key with the new password; it does not re-encrypt every file. Trusted computers that already remember this vault keep their key." onClose={modalClose} onSubmit={onSubmit} submitLabel="Update password" busy={busy}><label className="field-label">Current password<input required type="password" minLength={12} value={formName} onChange={event => setFormName(event.target.value)} /></label><label className="field-label">New password<input required type="password" minLength={12} value={password} onChange={event => setPassword(event.target.value)} placeholder="At least 12 characters" /></label></Dialog>}
    {modal === 'connect' && <Dialog title="Connect to pCloud" description="Sign in on pCloud's own website. Your password and two-step verification stay on pCloud's page." onClose={modalClose} onSubmit={onSubmit} busy={busy}>{error && <p className="dialog-error" role="alert">{error}</p>}<button type="button" className="button button-primary connect-web" onClick={() => void connectWeb()} disabled={busy}><Globe size={15} /> {busy ? 'Waiting for pCloud sign-in…' : 'Sign in on pCloud website'}</button><p className="privacy-note"><ShieldCheck size={15} /> A separate private window opens pCloud's sign-in page. This app keeps only the resulting session token, protected by your system's secure storage, and ends that session when you forget this computer.</p><details className="connect-alternative"><summary>Use a registered pCloud app instead</summary><label className="field-label">OAuth client ID<input required value={settings.clientId} onChange={event => setSettings({ ...settings, clientId: event.target.value })} placeholder="Paste your pCloud app client ID" /></label><p className="privacy-note"><ShieldCheck size={15} /> Register a pCloud app with full account access and callback URL <code>http://127.0.0.1:43187/callback</code>. Your browser opens pCloud's sign-in; this app only receives an access token.</p><button type="submit" className="button button-subtle" disabled={busy}>Continue with OAuth app</button></details></Dialog>}
    {preview && <Dialog title={preview.name} description={`${formatSize(preview.size)} · ${timeLabel(preview.modified)}`} onClose={closePreview}>
      <p className="preview-protection"><ProtectionBadge location={preview.location} /> {preview.location === 'vault' ? 'Stored encrypted in the Vault.' : 'Stored in pCloud, not encrypted.'}</p>
      <div className="preview-area">{previewUrl ? <img src={previewUrl} alt={preview.name} /> : <div className="preview-placeholder"><FileImage size={38} /><span>Preview is available in the desktop app.</span></div>}</div>
      <p className="export-warning">{preview.location === 'vault' ? 'Exported copies are decrypted and saved outside your managed vault.' : 'Exported copies are saved on this computer.'}</p>
      <div className="preview-actions"><button className="button button-subtle" onClick={() => void exportItem(preview, false)}><ArrowDownToLine size={16} /> Export copy</button><button className="button button-primary" onClick={() => void exportItem(preview, true)}><ArrowRight size={16} /> Export and open</button></div>
    </Dialog>}
    {toast && <div className="toast" role="status"><Check size={16} />{toast}</div>}
  </div>
}
