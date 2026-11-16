import { useEffect, useState } from 'react'
import { Cloud, LogOut, RefreshCw } from 'lucide-react'
import { getAccountInfo, runAction } from '../api'
import { Dialog } from '../components/Dialog'
import { CloudGate, PageHeader } from '../components/Gates'
import { errorMessage, formatSize } from '../format'
import type { AccountInfo, AppStatus } from '../types'
import type { Perform } from '../ui'

type Props = { status: AppStatus | null; busy: boolean; dataVersion: number; perform: Perform; openConnect: () => void }

export function AccountPage({ status, busy, dataVersion, perform, openConnect }: Props) {
  const [info, setInfo] = useState<AccountInfo | null>(null)
  const [problem, setProblem] = useState('')
  const [confirming, setConfirming] = useState(false)
  const connected = status?.connected ?? false
  useEffect(() => {
    if (!connected) { setInfo(null); return }
    let cancelled = false
    getAccountInfo().then(value => { if (!cancelled) { setInfo(value && typeof value === 'object' ? value : null); setProblem('') } }).catch(reason => { if (!cancelled) setProblem(errorMessage(reason, 'Could not read account details.')) })
    return () => { cancelled = true }
  }, [connected, dataVersion])
  const header = <PageHeader title="Account" description="The pCloud account this computer is signed in to." />
  if (!connected) return <div className="page">{header}<CloudGate connect={openConnect} what="see your account and storage" /></div>
  const email = info?.email ?? status?.accountEmail ?? 'Your pCloud account'
  const region = info?.region ?? status?.region
  const percent = info && info.quotaBytes ? Math.min(100, info.usedBytes / info.quotaBytes * 100) : 0
  return <div className="page narrow">
    {header}
    {problem && <p className="mini-empty" role="alert">{problem}</p>}
    <section className="settings-card">
      <div className="connected-account"><div className="avatar">{email.slice(0, 1).toUpperCase()}</div><div><strong>{email}</strong><small>{region ? `${region.toUpperCase()} data region` : 'Data region unknown'}{info ? ` · ${info.premium ? 'Premium' : 'Free plan'}` : ''}</small></div></div>
      <div className="storage-block">
        <div className="storage-title"><span>Storage used</span><span>{info ? `${formatSize(info.usedBytes)} of ${formatSize(info.quotaBytes)}` : '—'}</span></div>
        <div className="storage-track" role="progressbar" aria-label="Storage used" aria-valuemin={0} aria-valuemax={100} aria-valuenow={Math.round(percent)}><i style={{ width: `${percent}%` }} /></div>
        <small>Vault files count toward your pCloud storage too. They are stored encrypted.</small>
      </div>
      <div className="settings-row"><div><strong>Sign in again</strong><small>Use this if pCloud asks you to sign in, or to switch accounts.</small></div><button type="button" className="button button-subtle" onClick={openConnect}><RefreshCw size={14} /> Reconnect</button></div>
      <div className="settings-row"><div><strong>Disconnect this computer</strong><small>Signs out of pCloud and forgets the remembered Vault key on this computer.</small></div><button type="button" className="button button-ghost-danger" onClick={() => setConfirming(true)}><LogOut size={14} /> Disconnect</button></div>
    </section>
    {confirming && <Dialog title="Disconnect this computer?" description="This signs out of pCloud and removes the remembered sign-in and Vault key from this computer. Your files in pCloud and in the Vault stay where they are. You need your Vault password to use the Vault here again." onClose={() => setConfirming(false)} busy={busy} danger submitLabel="Disconnect"
      onSubmit={event => { event.preventDefault(); void perform(() => runAction('forget_device', {}), 'This computer was disconnected').then(done => { if (done) setConfirming(false) }) }}>
      <p className="confirm-item"><Cloud size={18} />{email}</p>
    </Dialog>}
  </div>
}
