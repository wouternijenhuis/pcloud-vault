import { useEffect, useRef, useState, type FormEvent } from 'react'
import { Check, Cloud, Globe, HardDrive, KeyRound, LockKeyhole, MousePointerClick, ShieldCheck, Upload } from 'lucide-react'
import type { AppStatus } from '../types'

type Props = {
  status: AppStatus
  busy: boolean
  error: string
  connect: () => Promise<boolean>
  createVault: (password: string, remember: boolean) => Promise<boolean>
  unlockVault: (password: string, remember: boolean) => Promise<boolean>
  exportRecovery: () => void
  workLocally: () => void
  finish: () => void
}

const STEPS = ['Welcome', 'Connect pCloud', 'Your vault', 'All set'] as const

/** First-start guide: explains the app, connects pCloud and creates or unlocks the vault. */
export function WelcomeWizard({ status, busy, error, connect, createVault, unlockVault, exportRecovery, workLocally, finish }: Props) {
  const [step, setStep] = useState(0)
  const [password, setPassword] = useState('')
  const [confirmation, setConfirmation] = useState('')
  const [remember, setRemember] = useState(true)
  const [mismatch, setMismatch] = useState(false)
  const body = useRef<HTMLDivElement>(null)
  const unlocking = status.vaultExists

  useEffect(() => { body.current?.querySelector<HTMLElement>('input, button.button-primary')?.focus() }, [step])
  useEffect(() => { if (step === 2 && status.unlocked) setStep(3) }, [step, status.unlocked])

  const submitVault = async (event: FormEvent<HTMLFormElement>) => {
    event.preventDefault()
    if (!unlocking && password !== confirmation) { setMismatch(true); return }
    const done = unlocking ? await unlockVault(password, remember) : await createVault(password, remember)
    if (done) { setPassword(''); setConfirmation(''); setStep(3) }
  }

  return <div className="wizard-backdrop">
    <div className="wizard" role="dialog" aria-modal="true" aria-labelledby="wizard-title">
      <ol className="wizard-steps" aria-label="Setup progress">
        {STEPS.map((label, index) => <li key={label} className={index === step ? 'current' : index < step ? 'done' : ''} aria-current={index === step ? 'step' : undefined}>
          <span>{index < step ? <Check size={12} /> : index + 1}</span>{label}
        </li>)}
      </ol>
      <div className="wizard-body" ref={body}>
        {error && <p className="dialog-error" role="alert">{error}</p>}

        {step === 0 && <>
          <div className="wizard-mark"><ShieldCheck size={26} /></div>
          <h2 id="wizard-title">Welcome to pCloud Vault</h2>
          <p className="wizard-lead">A private, open-source Windows client for your pCloud storage.</p>
          <ul className="wizard-points">
            <li><LockKeyhole size={17} /><div><strong>Encrypted before it leaves your PC</strong><span>Files in your Private Vault are encrypted on this PC with your own password. pCloud only stores unreadable data.</span></div></li>
            <li><Cloud size={17} /><div><strong>Your regular pCloud files, side by side</strong><span>Browse your existing files under pCloud Files. They are clearly marked as not encrypted.</span></div></li>
            <li><HardDrive size={17} /><div><strong>On all your PCs</strong><span>Sign in on another PC, enter your vault password once, and your vault follows you.</span></div></li>
          </ul>
          <p className="wizard-origin"><span className="eu-badge">Made in Europe</span> Free and open source. No ads, no tracking.</p>
          <div className="wizard-actions"><button className="text-action" onClick={finish}>Skip setup</button><button className="button button-primary" onClick={() => setStep(1)}>Get started</button></div>
        </>}

        {step === 1 && <>
          <h2 id="wizard-title">Connect your pCloud account</h2>
          <p className="wizard-lead">Sign in on pCloud's own page. Your pCloud password and two-step verification never pass through this app.</p>
          {status.connected
            ? <div className="wizard-connected"><Check size={17} /><div><strong>Connected</strong><span>{status.accountEmail ?? 'Your pCloud account'}{status.region ? ` · ${status.region.toUpperCase()} region` : ''}</span></div></div>
            : <button className="button button-primary wizard-wide" onClick={() => void connect()} disabled={busy}><Globe size={16} /> {busy ? 'Waiting for pCloud sign-in…' : 'Sign in on pCloud website'}</button>}
          <p className="privacy-note"><ShieldCheck size={15} /> A separate private window opens pCloud's sign-in page. You can use email, Google, Apple or Facebook. The app keeps only a sign-in token, protected by Windows.</p>
          <div className="wizard-actions">
            {status.connected
              ? <><button className="text-action" onClick={() => setStep(0)}>Back</button><button className="button button-primary" onClick={() => setStep(2)}>Continue</button></>
              : <><button className="text-action" onClick={() => { workLocally(); setStep(2) }} disabled={busy}>Use only this PC for now</button><button className="button button-subtle" onClick={() => setStep(0)} disabled={busy}>Back</button></>}
          </div>
        </>}

        {step === 2 && <form onSubmit={event => void submitVault(event)}>
          <h2 id="wizard-title">{unlocking ? 'Unlock your vault' : 'Create your vault password'}</h2>
          <p className="wizard-lead">{unlocking
            ? 'We found your existing vault in pCloud. Enter its password once to use it on this PC.'
            : 'This password encrypts your Private Vault. Use at least 12 characters, and keep it safe: nobody, including pCloud and the makers of this app, can recover it for you.'}</p>
          <label className="field-label">Vault password<input required type="password" minLength={12} value={password} onChange={e => { setPassword(e.target.value); setMismatch(false) }} autoComplete={unlocking ? 'current-password' : 'new-password'} placeholder="At least 12 characters" /></label>
          {!unlocking && <label className="field-label">Repeat password<input required type="password" minLength={12} value={confirmation} onChange={e => { setConfirmation(e.target.value); setMismatch(false) }} autoComplete="new-password" aria-invalid={mismatch} /></label>}
          {mismatch && <p className="dialog-error" role="alert">The passwords don't match.</p>}
          <label className="check-label"><input type="checkbox" checked={remember} onChange={e => setRemember(e.target.checked)} />Remember this PC, so it unlocks with your Windows sign-in</label>
          <div className="wizard-actions"><button type="button" className="text-action" onClick={() => setStep(1)} disabled={busy}>Back</button><button type="submit" className="button button-primary" disabled={busy}>{busy ? 'Please wait…' : unlocking ? 'Unlock vault' : 'Create vault'}</button></div>
        </form>}

        {step === 3 && <>
          <div className="wizard-mark done"><Check size={26} /></div>
          <h2 id="wizard-title">You're all set</h2>
          <ul className="wizard-points compact">
            <li><Upload size={17} /><div><strong>Add files</strong><span>Drag files or folders into Private Vault. The originals on your PC stay where they are.</span></div></li>
            <li><MousePointerClick size={17} /><div><strong>Right-click for actions</strong><span>Preview, export, rename, keep offline or move to trash.</span></div></li>
            <li><KeyRound size={17} /><div><strong>Save a recovery key</strong><span>If you ever forget your password, a recovery key is the only way back into your vault. Store it somewhere safe, offline.</span></div></li>
          </ul>
          <div className="wizard-actions"><button className="button button-subtle" onClick={exportRecovery} disabled={busy || !status.unlocked}><KeyRound size={15} /> Save recovery key…</button><button className="button button-primary" onClick={finish}>Open my vault</button></div>
        </>}
      </div>
    </div>
  </div>
}
