import { Cloud, ShieldCheck } from 'lucide-react'
import type { Location } from '../types'

type Props = { location: Location; compact?: boolean; className?: string }

/** The one visual answer to "can pCloud read this?", used wherever a file, folder or destination appears. */
export function ProtectionBadge({ location, compact = false, className = '' }: Props) {
  const encrypted = location === 'vault'
  const label = encrypted ? 'Encrypted' : 'Not encrypted'
  const tip = encrypted ? 'Only you can read this' : 'pCloud can read this'
  const Icon = encrypted ? ShieldCheck : Cloud
  if (compact) return <span className={`protection-badge compact ${encrypted ? 'enc' : 'plain'} ${className}`} title={`${label}: ${tip}`} aria-label={`${label}: ${tip}`} role="img"><Icon size={13} aria-hidden="true" /></span>
  return <span className={`protection-badge ${encrypted ? 'enc' : 'plain'} ${className}`} title={tip}>
    <Icon size={13} aria-hidden="true" /><span>{label}</span><span className="sr-only">: {tip}</span>
  </span>
}
