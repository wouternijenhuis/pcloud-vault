import { Cloud, Code, ShieldCheck } from 'lucide-react'
import { runExtra } from '../api'
import { PageHeader } from '../components/Gates'
import type { Perform } from '../ui'

export const SOURCE_URL = 'https://github.com/wouternijenhuis/pcloud-vault'

export function AboutPage({ perform }: { perform: Perform }) {
  return <div className="page narrow">
    <PageHeader title="About" description={<>pCloud Vault {__APP_VERSION__} · free and open source under GPL-3.0-only · not affiliated with pCloud AG</>} />
    <section className="boundary-grid" aria-label="What pCloud can see">
      <div className="boundary-card enc">
        <h3><ShieldCheck size={17} aria-hidden="true" /> Vault — encrypted</h3>
        <p><strong>pCloud cannot see</strong> the contents, file names or folder structure.</p>
        <p><strong>pCloud can still see</strong> roughly how much you store, how many encrypted pieces there are, and when they change.</p>
      </div>
      <div className="boundary-card plain">
        <h3><Cloud size={17} aria-hidden="true" /> pCloud Files — not encrypted</h3>
        <p><strong>pCloud can see</strong> everything: contents, names and folders, as with the official app.</p>
        <p>Links and shared folders only work for these files.</p>
      </div>
    </section>
    <section className="settings-card about-notes">
      <p>Encryption protects files stored in pCloud. It does not protect against malware on this computer, an unlocked computer someone else uses, or copies you export.</p>
      <p>The Vault format is new and has <strong>not been independently audited</strong>. Keep separate backups of anything you cannot lose.</p>
      <p className="app-origin"><span className="eu-badge">Made in Europe</span> No ads, no tracking.</p>
      <button type="button" className="button button-subtle" onClick={() => void perform(() => runExtra('open_url', { url: SOURCE_URL }))}><Code size={14} /> View the source code</button>
    </section>
  </div>
}
