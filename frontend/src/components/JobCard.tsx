import { ArrowRight, Folder, Pause, Play, Recycle, RefreshCw, Trash2, TriangleAlert } from 'lucide-react'
import { ProtectionBadge } from './ProtectionBadge'
import { dateTimeLabel, plural } from '../format'
import type { FolderJob, JobState } from '../types'

const STATE_TEXT: Record<JobState, string> = {
  idle: 'Up to date',
  scanning: 'Checking for changes…',
  transferring: 'Transferring files…',
  paused: 'Paused',
  waitingForVault: 'Waiting for you to unlock the Vault',
  waitingForConnection: 'Waiting for a connection to pCloud',
  needsReview: 'Needs your review',
  error: 'Something went wrong',
}
export const jobStateText = (job: FolderJob) => job.state === 'error' && job.message ? job.message : STATE_TEXT[job.state]

type Props = {
  job: FolderJob
  busy: boolean
  onPause: (paused: boolean) => void
  onRun: () => void
  onRecycle: () => void
  onRemove: () => void
  onResolve: (apply: boolean) => void
}

export function JobCard({ job, busy, onPause, onRun, onRecycle, onRemove, onResolve }: Props) {
  const name = job.localPath.split(/[\\/]/).filter(Boolean).pop() ?? job.localPath
  return <article className={`job-card ${job.state}`} aria-label={`${job.kind === 'sync' ? 'Sync' : 'Backup'} of ${name}`}>
    <div className="job-route">
      <div className="job-end"><Folder size={18} aria-hidden="true" /><div><strong>{name}</strong><small title={job.localPath}>{job.localPath}</small></div></div>
      <ArrowRight size={16} className="job-arrow" aria-label={job.kind === 'sync' ? 'kept in sync with' : 'copied to'} />
      <div className="job-end"><div><strong>{job.target.label}</strong><ProtectionBadge location={job.target.location} /></div></div>
    </div>
    <div className="job-meta">
      <span className={`job-state ${job.state}`}>{job.state === 'needsReview' || job.state === 'error' ? <TriangleAlert size={13} aria-hidden="true" /> : null}{jobStateText(job)}</span>
      <span>{job.lastRun ? `Last run ${dateTimeLabel(job.lastRun)}` : 'Not run yet'}</span>
      {job.fileCount > 0 && <span>{plural(job.fileCount, 'file')}</span>}
      {job.kind === 'backup' && <span>{job.mirrorDeletions ? 'Mirrors deletions' : 'Keeps deleted files'}</span>}
    </div>
    {job.state === 'needsReview' && <div className="guard-box" role="alert">
      <p><strong>{plural(job.pendingDeletes, 'file')} would be deleted.</strong> {job.kind === 'sync'
        ? 'That is more than usual, so the sync stopped to ask you. If you meant to delete them, continue. Otherwise they are put back on both sides.'
        : 'That is more than usual, so the backup stopped to ask you. If you meant to delete them, continue. Otherwise the backup keeps them.'}</p>
      <div className="guard-actions"><button type="button" className="button button-danger" disabled={busy} onClick={() => onResolve(true)}>Delete them</button><button type="button" className="button button-subtle" disabled={busy} onClick={() => onResolve(false)}>Keep them</button></div>
    </div>}
    <div className="job-actions">
      <button type="button" className="button button-subtle" disabled={busy} onClick={() => onPause(!job.paused)}>{job.paused ? <><Play size={14} /> Resume</> : <><Pause size={14} /> Pause</>}</button>
      <button type="button" className="button button-subtle" disabled={busy || job.paused} onClick={onRun}><RefreshCw size={14} /> Run now</button>
      {job.kind === 'sync' && <button type="button" className="button button-subtle" disabled={busy} onClick={onRecycle}><Recycle size={14} /> Open recycle folder</button>}
      <button type="button" className="button button-ghost-danger" disabled={busy} onClick={onRemove}><Trash2 size={14} /> Remove</button>
    </div>
  </article>
}
