import { Download, RotateCw } from 'lucide-react'
import { useCallback, useEffect, useRef, useState } from 'react'
import * as api from '../api'
import type { AppUpdateProgress, ReleaseNotes } from '../types'
import { cx } from './bits'
import { Button, Dialog } from './Dialog'

const CHECK_EVERY_MS = 2 * 60 * 60 * 1000
/** Waits a little after opening, so the first check never competes with loading the projects. */
const FIRST_CHECK_DELAY_MS = 8000
const RELEASES_URL = 'https://github.com/Wutname1/mehen/releases'

export type AppUpdateStatus = 'idle' | 'checking' | 'current' | 'available' | 'downloading' | 'ready' | 'installing' | 'error'

export interface AppUpdate {
  status: AppUpdateStatus
  current: string | null
  version: string | null
  progress: AppUpdateProgress | null
  error: string | null
  check: () => Promise<void>
  download: () => Promise<void>
  install: () => Promise<void>
}

/**
 * Mehen's own updates. Checks shortly after opening and every two hours while
 * `auto` is on. A dev build never checks by itself, since installing a release
 * over it would replace the build being worked on.
 */
export function useAppUpdate(auto: boolean): AppUpdate {
  const [status, setStatus] = useState<AppUpdateStatus>('idle')
  const [current, setCurrent] = useState<string | null>(null)
  const [version, setVersion] = useState<string | null>(null)
  const [progress, setProgress] = useState<AppUpdateProgress | null>(null)
  const [error, setError] = useState<string | null>(null)
  const statusRef = useRef(status)
  statusRef.current = status

  useEffect(() => {
    api.appVersion().then(setCurrent, () => {})
  }, [])

  const check = useCallback(async () => {
    if (['checking', 'downloading', 'ready', 'installing'].includes(statusRef.current)) return
    setStatus('checking')
    setError(null)
    try {
      const found = await api.checkAppUpdate()
      setVersion(found)
      setStatus(found ? 'available' : 'current')
    } catch (e) {
      setError(String(e))
      setStatus('error')
    }
  }, [])

  const download = useCallback(async () => {
    setStatus('downloading')
    setError(null)
    setProgress(null)
    const unlisten = api.onAppUpdateProgress(setProgress)
    try {
      const got = await api.downloadAppUpdate()
      if (got) {
        setVersion(got)
        setStatus('ready')
      } else {
        setStatus('current')
      }
    } catch (e) {
      setError(String(e))
      setStatus('error')
    } finally {
      unlisten.then((fn) => fn())
    }
  }, [])

  const install = useCallback(async () => {
    setStatus('installing')
    setError(null)
    try {
      await api.installAppUpdate()
    } catch (e) {
      setError(String(e))
      setStatus('error')
    }
  }, [])

  useEffect(() => {
    if (!auto || import.meta.env.DEV) return
    const first = window.setTimeout(check, FIRST_CHECK_DELAY_MS)
    const every = window.setInterval(check, CHECK_EVERY_MS)
    return () => {
      window.clearTimeout(first)
      window.clearInterval(every)
    }
  }, [auto, check])

  return { status, current, version, progress, error, check, download, install }
}

/** Header button that appears only once there is a new Mehen to get. */
export function AppUpdateButton({ update, onOpen }: { update: AppUpdate; onOpen: () => void }) {
  const { status, version, progress } = update
  const showing = status === 'available' || status === 'downloading' || status === 'ready' || (status === 'error' && version)
  if (!showing) return null
  const label = status === 'ready' ? 'Restart to update' : status === 'downloading' ? `Downloading ${percent(progress) ?? ''}`.trim() : `Mehen ${version} is out`
  return (
    <button
      type="button"
      onClick={onOpen}
      className="inline-flex h-[34px] items-center gap-2 rounded-[3px] border border-accent bg-accent px-2.5 text-[12px] font-semibold text-accent-ink hover:border-accent-hover hover:bg-accent-hover"
    >
      {status === 'ready' ? <RotateCw size={15} /> : <Download size={15} />}
      {label}
    </button>
  )
}

const SECTIONS: [string, string][] = [
  ['breaking', 'Changes to how things work'],
  ['feature', 'New'],
  ['fix', 'Fixed'],
  ['change', 'Improved'],
  ['docs', 'Help and guides'],
]

/** What changed since the running version, and the steps to get it. */
export function AppUpdateDialog({ update, onClose }: { update: AppUpdate; onClose: () => void }) {
  const { status, current, version, progress, error } = update
  const [notes, setNotes] = useState<ReleaseNotes[] | null>(null)
  const [notesFailed, setNotesFailed] = useState(false)

  useEffect(() => {
    if (!current || !version) return
    api.appReleaseNotes(current, version).then(setNotes, () => setNotesFailed(true))
  }, [current, version])

  const busy = status === 'installing'
  const pct = percent(progress)

  return (
    <Dialog
      title={status === 'ready' ? `Mehen ${version} is ready to install` : `Mehen ${version} is available`}
      description={
        status === 'ready'
          ? 'Mehen closes, installs the new version, and opens again. This takes under a minute.'
          : `You have ${current ?? 'an older version'}. Here is what changed since then.`
      }
      busy={busy}
      onClose={onClose}
      footer={
        <>
          <Button variant="ghost" className="mr-auto" onClick={() => api.openLink(RELEASES_URL)}>
            All releases
          </Button>
          <Button onClick={onClose} disabled={busy}>
            Later
          </Button>
          {status === 'ready' || status === 'installing' ? (
            <Button variant="primary" onClick={update.install} disabled={busy}>
              <RotateCw size={14} />
              {busy ? 'Restarting...' : 'Restart to update'}
            </Button>
          ) : (
            <Button variant="primary" onClick={update.download} disabled={status === 'downloading'}>
              <Download size={14} />
              {status === 'downloading' ? `Downloading${pct ? ` ${pct}` : '...'}` : status === 'error' ? 'Try again' : 'Download'}
            </Button>
          )}
        </>
      }
    >
      {status === 'downloading' && (
        <div className="mb-3 h-1.5 overflow-hidden rounded-full bg-sunken" role="progressbar" aria-label="Download progress" aria-valuenow={fraction(progress) * 100} aria-valuemin={0} aria-valuemax={100}>
          <div className={cx('h-full bg-accent transition-[width]', !progress?.total && 'w-1/3 animate-pulse')} style={progress?.total ? { width: `${fraction(progress) * 100}%` } : undefined} />
        </div>
      )}
      {error && <p className="mb-3 rounded-[3px] bg-vuln-row px-3 py-2 text-[12.5px] text-risk-security">{error}</p>}
      {notesFailed ? (
        <p className="text-[12.5px] text-muted">The release notes could not be loaded. The update is still safe to install.</p>
      ) : !notes ? (
        <p className="text-[12.5px] text-muted">Loading what changed...</p>
      ) : notes.length === 0 ? (
        <p className="text-[12.5px] text-muted">No notes were published for this version.</p>
      ) : (
        <div className="grid gap-5">
          {notes.map((release) => (
            <Release key={release.version} release={release} />
          ))}
        </div>
      )}
    </Dialog>
  )
}

function Release({ release }: { release: ReleaseNotes }) {
  const date = release.releasedAt ? new Date(release.releasedAt).toLocaleDateString(undefined, { year: 'numeric', month: 'long', day: 'numeric' }) : null
  return (
    <section>
      <h3 className="m-0 flex items-baseline gap-2 font-display text-[15px] font-semibold">
        Mehen {release.version}
        {date && <span className="font-sans text-[12px] font-normal text-muted">{date}</span>}
      </h3>
      {SECTIONS.map(([section, label]) => {
        const items = release.items.filter((i) => i.section === section)
        if (!items.length) return null
        return (
          <div key={section} className="mt-2">
            <h4 className="m-0 font-mono text-[11px] font-normal tracking-[0.05em] text-muted uppercase">{label}</h4>
            <ul className="mt-1 mb-0 grid list-disc gap-1 pl-4 text-[13px] leading-snug marker:text-faint">
              {items.map((item, i) => (
                <li key={i}>{item.text}</li>
              ))}
            </ul>
          </div>
        )
      })}
    </section>
  )
}

function fraction(p: AppUpdateProgress | null): number {
  return p?.total ? Math.min(1, p.downloaded / p.total) : 0
}

function percent(p: AppUpdateProgress | null): string | null {
  return p?.total ? `${Math.round(fraction(p) * 100)}%` : null
}
