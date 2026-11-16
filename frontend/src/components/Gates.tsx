import { ChevronRight, Cloud, LockKeyhole, ShieldCheck } from 'lucide-react'

export function SetupGate({ connected, connect, create, workLocally }: { connected: boolean; connect: () => void; create: () => void; workLocally: () => void }) {
  return <div className="onboarding-wrap"><div className="onboarding-card"><div className="onboarding-art"><img src="/art-empty.svg" alt="" /></div><p className="eyebrow"><ShieldCheck size={13} /> PRIVATE BY DESIGN</p><h2>{connected ? 'Your private space starts here.' : 'A private place for your files.'}</h2><p className="onboarding-copy">{connected ? 'Create a password to protect files you choose to keep in your vault.' : 'Connect pCloud first to discover an existing vault. Your account password is never shared with this app.'}</p>{connected ? <button className="button button-primary" onClick={create}><LockKeyhole size={15} /> Create a vault password</button> : <><button className="button button-primary" onClick={connect}><Cloud size={15} /> Connect pCloud</button><p className="onboarding-foot">Already have a vault? Connect your account to unlock it.</p><button className="text-action" onClick={workLocally}>Work locally on this computer <ChevronRight size={14} /></button></>}</div></div>
}

export function LoadingGate() {
  return <div className="onboarding-wrap"><div className="onboarding-card"><div className="gate-lock"><ShieldCheck size={22} /></div><h2>Checking your private space.</h2><p className="onboarding-copy">Reading vault and connection status from this computer…</p></div></div>
}

export function UnlockGate({ unlock }: { unlock: () => void }) {
  return <div className="onboarding-wrap"><div className="onboarding-card"><div className="gate-lock enc"><LockKeyhole size={22} /></div><p className="eyebrow"><ShieldCheck size={13} /> YOUR PRIVATE SPACE</p><h2>Your vault is locked.</h2><p className="onboarding-copy">Unlock your vault to see your protected files on this computer.</p><button className="button button-primary" onClick={unlock}><LockKeyhole size={15} /> Unlock vault</button></div></div>
}

export function CloudGate({ connect, what = 'browse your cloud files' }: { connect: () => void; what?: string }) {
  return <div className="onboarding-wrap"><div className="onboarding-card"><div className="gate-lock"><Cloud size={22} /></div><p className="eyebrow"><Cloud size={13} /> PCLOUD STORAGE</p><h2>Connect your pCloud account.</h2><p className="onboarding-copy">Sign in with pCloud to {what}. Your encrypted Vault stays visibly separate.</p><button className="button button-primary" onClick={connect}><Cloud size={15} /> Connect pCloud</button><p className="onboarding-foot">Files in pCloud Files are not encrypted by this app.</p></div></div>
}

/** A consistent page heading with an optional badge and actions. */
export function PageHeader({ title, description, badge, actions }: { title: string; description: React.ReactNode; badge?: React.ReactNode; actions?: React.ReactNode }) {
  return <div className="page-header"><div><h2>{title}{badge}</h2><p>{description}</p></div>{actions && <div className="page-actions">{actions}</div>}</div>
}
