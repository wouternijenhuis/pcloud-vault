import { Cloud, ShieldCheck, SlidersHorizontal } from 'lucide-react'
import { PROTECTION_LABELS } from '../format'
import type { Protection } from '../types'

const OPTIONS: { id: Protection; icon: typeof Cloud; tag?: string; who: string; detail: string }[] = [
  { id: 'everything', icon: ShieldCheck, tag: 'Recommended', who: 'Only you can read your files.', detail: 'Uploads, backups and syncs all go to the Vault and are encrypted on this computer before they leave it.' },
  { id: 'choose', icon: SlidersHorizontal, who: 'You decide per upload, backup or sync.', detail: 'Each time you pick Vault (encrypted, only you can read it) or pCloud (not encrypted, pCloud can read it).' },
  { id: 'vaultOnly', icon: Cloud, who: 'pCloud can read everything outside the Vault.', detail: 'Uploads, backups and syncs go to pCloud unencrypted, like the official app. Only what you put in the Vault yourself is encrypted.' },
]

type Props = { value: Protection | null; onChange: (value: Protection) => void; disabled?: boolean; name?: string }

/** The three protection modes, explained in plain words. */
export function ProtectionChoice({ value, onChange, disabled = false, name = 'How your files are protected' }: Props) {
  return <div className="protection-choice" role="radiogroup" aria-label={name}>
    {OPTIONS.map(option => {
      const Icon = option.icon
      return <button key={option.id} type="button" role="radio" aria-checked={value === option.id} disabled={disabled}
        className={`protection-option ${option.id} ${value === option.id ? 'selected' : ''}`} onClick={() => onChange(option.id)}>
        <span className="protection-option-icon"><Icon size={19} aria-hidden="true" /></span>
        <span className="protection-option-text">
          <strong>{PROTECTION_LABELS[option.id]}{option.tag && <em>{option.tag}</em>}</strong>
          <span className="protection-who">{option.who}</span>
          <small>{option.detail}</small>
        </span>
      </button>
    })}
    <p className="protection-footnote">Even for Vault files, pCloud still sees roughly how much you store, how many pieces it is split into, and when it changes. It never sees their names or contents.</p>
  </div>
}
