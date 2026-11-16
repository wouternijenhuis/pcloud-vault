import { Bug, ClipboardCopy } from 'lucide-react'
import { runExtra } from '../api'
import { PageHeader } from '../components/Gates'
import { copyText, PROTECTION_LABELS } from '../format'
import type { AppStatus, FolderJob, Protection, Transfer } from '../types'
import type { Perform } from '../ui'

export const ISSUES_URL = 'https://github.com/wouternijenhuis/pcloud-vault/issues'

const GUIDES: { title: string; body: string[] }[] = [
  { title: 'What does encrypted mean here?', body: [
    'Files in the Vault are encrypted on this computer with a key that only your Vault password (or recovery key) unlocks. pCloud stores only scrambled data: it cannot read the contents or the names of Vault files.',
    'Files in pCloud Files are not encrypted by this app. pCloud can read them, just like files uploaded with the official app or website.',
    'Every file, folder, destination and link in this app carries a badge: a green shield means "Encrypted — only you can read this", a grey cloud means "Not encrypted — pCloud can read this".',
    'Even for Vault files, pCloud still sees roughly how much you store, how many pieces it is split into, and when it changes.',
  ] },
  { title: 'Back up or sync?', body: [
    'A backup is a one-way copy from this computer to the cloud. Deleting a file on this computer does not delete it from the backup, unless you turn on "Mirror deletions".',
    'A sync keeps the same files on both sides. Changes and deletions flow both ways.',
  ] },
  { title: 'What happens with conflicts?', body: [
    'If the same file changes in two places before they could sync, nothing is overwritten. You get both versions, and the second one is saved as a conflict copy next to the original.',
    'When a sync or backup would delete many files at once, it stops and asks you first.',
  ] },
  { title: 'The recycle folder', body: [
    'When a sync removes or replaces a file on this computer, the previous version first goes to the app\'s recycle folder and stays there for 30 days. Open it from the folder\'s card on the Sync page.',
  ] },
  { title: 'Your recovery key', body: [
    'The recovery key unlocks your Vault without the password. Export it in Settings and keep it somewhere safe and offline, such as a printed copy.',
    'Anyone with the recovery key can read your Vault, so treat it like the password itself.',
  ] },
  { title: 'I forgot my Vault password', body: [
    'Nobody can reset it for you, not pCloud and not the makers of this app. That is what keeps your files private.',
    'If this computer remembers the Vault, it still unlocks here: export a recovery key right away. Otherwise use your recovery key with the recovery tool. Without either, the encrypted files cannot be recovered.',
  ] },
]

type Props = { status: AppStatus | null; protection: Protection | null; jobs: FolderJob[]; transfers: Transfer[]; perform: Perform; announce: (message: string) => void }

/** Diagnostic info deliberately leaves out the account email, file names and folder paths. */
export function diagnosticInfo(status: AppStatus | null, protection: Protection | null, jobs: FolderJob[], transfers: Transfer[]) {
  return [
    `pCloud Vault ${__APP_VERSION__}`,
    `Platform: ${navigator.platform || 'unknown'}`,
    `Browser engine: ${navigator.userAgent}`,
    `Protection: ${protection ? PROTECTION_LABELS[protection] : 'not chosen'}`,
    `Vault: ${!status?.vaultExists ? 'not set up' : status.unlocked ? 'unlocked' : 'locked'}`,
    `pCloud: ${status?.connected ? `connected (${status.region?.toUpperCase() ?? 'region unknown'})` : 'not connected'}`,
    `Backups: ${jobs.filter(job => job.kind === 'backup').length}, syncs: ${jobs.filter(job => job.kind === 'sync').length}, needing review: ${jobs.filter(job => job.state === 'needsReview').length}, with errors: ${jobs.filter(job => job.state === 'error').length}`,
    `Transfers: ${transfers.filter(item => item.state === 'active' || item.state === 'queued').length} running, ${transfers.filter(item => item.state === 'failed').length} failed`,
    `Changes waiting to upload: ${status?.pendingCount ?? 0}`,
  ].join('\n')
}

export function HelpPage({ status, protection, jobs, transfers, perform, announce }: Props) {
  return <div className="page narrow">
    <PageHeader title="Help" description="Short guides to how this app protects your files." />
    <div className="guide-list">{GUIDES.map(guide => <details key={guide.title} className="guide">
      <summary>{guide.title}</summary>
      {guide.body.map(paragraph => <p key={paragraph}>{paragraph}</p>)}
    </details>)}</div>
    <section className="settings-card help-actions">
      <div className="settings-row"><div><strong>Report a problem</strong><small>Opens the public issue tracker. Don't include passwords, recovery keys or private file names. Report security problems privately as described in SECURITY.md.</small></div><button type="button" className="button button-subtle" onClick={() => void perform(() => runExtra('open_url', { url: ISSUES_URL }))}><Bug size={14} /> Report a problem</button></div>
      <div className="settings-row"><div><strong>Diagnostic info</strong><small>App version, system, protection mode and counts. No email address, file names or folder paths.</small></div><button type="button" className="button button-subtle" onClick={() => void copyText(diagnosticInfo(status, protection, jobs, transfers)).then(ok => announce(ok ? 'Diagnostic info copied' : 'Could not copy diagnostic info'))}><ClipboardCopy size={14} /> Copy diagnostic info</button></div>
    </section>
  </div>
}
