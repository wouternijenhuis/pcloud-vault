import { useState } from 'react'
import { FolderOpen, Plus } from 'lucide-react'
import { runExtra } from '../api'
import { Dialog } from '../components/Dialog'
import { PageHeader } from '../components/Gates'
import { JobCard } from '../components/JobCard'
import { knownFolderList } from '../components/FolderJobDialog'
import type { FolderJob, JobKind, KnownFolders } from '../types'
import type { Perform } from '../ui'

type Props = {
  kind: JobKind
  jobs: FolderJob[]
  knownFolders: KnownFolders | null
  busy: boolean
  perform: Perform
  onAdd: (initialPath?: string) => void
}

const COPY: Record<JobKind, { title: string; lead: string; points: string[]; add: string; empty: string }> = {
  backup: {
    title: 'Back up',
    lead: 'A backup is a one-way copy of a folder from this computer to the cloud.',
    points: [
      'New and changed files on this computer are copied to the backup.',
      'Deleting a file on this computer does not delete it from the backup, unless you turn on "Mirror deletions".',
      'Changes made in the cloud never touch this computer.',
    ],
    add: 'Add backup',
    empty: 'No backups yet. Pick a folder to keep a safe copy of it in the cloud.',
  },
  sync: {
    title: 'Sync',
    lead: 'A sync keeps the same files on this computer and in the cloud, in both directions.',
    points: [
      'Changes on either side appear on the other side.',
      'Deletions flow both ways too: delete a file here and it is removed in the cloud, and the other way around.',
      'Files the sync removes from this computer go to the app\'s recycle folder for 30 days first.',
    ],
    add: 'Add sync',
    empty: 'No synced folders yet. Pick a folder to have it on this computer and in the cloud.',
  },
}

/** Shared page for Back up and Sync: explanation, suggestions and one card per folder. */
export function JobsPage({ kind, jobs, knownFolders, busy, perform, onAdd }: Props) {
  const [removing, setRemoving] = useState<FolderJob | null>(null)
  const copy = COPY[kind]
  const mine = jobs.filter(job => job.kind === kind)
  const used = new Set(mine.map(job => job.localPath))
  const suggestions = knownFolderList(knownFolders).filter(item => !used.has(item.path))
  return <div className="page">
    <PageHeader title={copy.title} description={copy.lead} actions={<button type="button" className="button button-primary" onClick={() => onAdd()}><Plus size={16} /> {copy.add}</button>} />
    <section className="explain-card" aria-label={`How ${kind === 'sync' ? 'sync' : 'backup'} works`}>
      <ul>{copy.points.map(point => <li key={point}>{point}</li>)}</ul>
      <p className="explain-other">{kind === 'backup' ? 'Want the same files in both places instead? Use Sync.' : 'Only want a safety copy that you never change in the cloud? Use Back up.'}</p>
    </section>
    {suggestions.length > 0 && <section className="suggestions" aria-label="Suggested folders">
      <h3>Suggested folders</h3>
      <div className="chip-row">{suggestions.map(item => <button key={item.label} type="button" className="chip" onClick={() => onAdd(item.path)}><FolderOpen size={14} /> {kind === 'sync' ? 'Sync' : 'Back up'} {item.label}</button>)}</div>
    </section>}
    <section className="job-list" aria-label={kind === 'sync' ? 'Synced folders' : 'Backed up folders'}>
      {mine.length === 0 ? <p className="mini-empty">{copy.empty}</p> : mine.map(job => <JobCard key={job.id} job={job} busy={busy}
        onPause={paused => void perform(() => runExtra('update_job', { id: job.id, paused }), paused ? 'Paused' : 'Resumed')}
        onRun={() => void perform(() => runExtra('run_job', { id: job.id }), 'Started')}
        onRecycle={() => void perform(() => runExtra('open_recycle', { id: job.id }))}
        onRemove={() => setRemoving(job)}
        onResolve={apply => void perform(() => runExtra('resolve_guard', { id: job.id, apply }), apply ? 'Deletions applied' : 'Files kept')} />)}
    </section>
    {removing && <Dialog title={kind === 'sync' ? 'Stop syncing this folder?' : 'Stop backing up this folder?'}
      description={`Nothing is deleted: the files stay on this computer and in ${removing.target.label}.`}
      onClose={() => setRemoving(null)} busy={busy} danger submitLabel="Remove"
      onSubmit={event => { event.preventDefault(); const job = removing; void perform(() => runExtra('remove_job', { id: job.id }), 'Removed').then(done => { if (done) setRemoving(null) }) }}>
      <p className="confirm-item"><FolderOpen size={18} />{removing.localPath}</p>
    </Dialog>}
  </div>
}

export const BackupPage = (props: Omit<Props, 'kind'>) => <JobsPage kind="backup" {...props} />
export const SyncPage = (props: Omit<Props, 'kind'>) => <JobsPage kind="sync" {...props} />
