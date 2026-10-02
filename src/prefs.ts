import { useEffect, useState } from 'react'

/** After a successful update: leave the files, commit them, or commit and push. */
export type CommitMode = 'off' | 'commit' | 'push'

export const COMMIT_MODES: { value: CommitMode; label: string; detail: string }[] = [
  { value: 'off', label: "Don't commit", detail: 'Leaves the changed files for you to commit.' },
  { value: 'commit', label: 'Commit', detail: 'Commits only the files Mehen changed, once that project is done. Never pushes.' },
  { value: 'push', label: 'Commit & push', detail: 'Commits only the files Mehen changed, then pushes the branch to its remote.' },
]

/** Choices that only affect this window, kept in the webview's storage. */
export interface Prefs {
  palette: 'faience' | 'parchment'
  theme: 'dark' | 'light' | 'system'
  /** Build each project after installing. */
  build: boolean
  /** Run each project's tests after installing. */
  test: boolean
  /** Commit (and maybe push) each repository once its update succeeds. */
  commit: CommitMode
  /** Check every project when Mehen opens. */
  scanOnOpen: boolean
  /** Stop a project at its first failed check instead of running the rest. */
  stopOnFailure: boolean
  /** List security fixes and bigger jumps first. */
  riskFirst: boolean
  /** Order of the project list. */
  railSort: 'az' | 'za' | 'updates'
}

const DEFAULTS: Prefs = { palette: 'faience', theme: 'dark', build: true, test: true, commit: 'off', scanOnOpen: true, stopOnFailure: true, riskFirst: true, railSort: 'az' }

/** Bumped when a default changes and older saved choices should pick it up. */
const VERSION = 2
const KEY = 'mehen-prefs'

function load(): Prefs {
  try {
    const saved = JSON.parse(localStorage.getItem(KEY) ?? '{}')
    // Version 2 turned on checking at launch for everyone.
    if ((saved.version ?? 1) < 2) delete saved.scanOnOpen
    // Build and test used to be one switch.
    if (typeof saved.checks === 'boolean') {
      saved.build ??= saved.checks
      saved.test ??= saved.checks
      delete saved.checks
    }
    // Commit used to be on or off.
    if (typeof saved.commit === 'boolean') saved.commit = saved.commit ? 'commit' : 'off'
    return { ...DEFAULTS, ...saved }
  } catch {
    return DEFAULTS
  }
}

const systemDark = () => typeof matchMedia !== 'undefined' && matchMedia('(prefers-color-scheme: dark)').matches

/** Preferences plus the theme actually shown; palette and theme land on <html>. */
export function usePrefs() {
  const [prefs, setPrefs] = useState<Prefs>(load)
  const [dark, setDark] = useState(systemDark)

  useEffect(() => {
    const media = matchMedia('(prefers-color-scheme: dark)')
    const onChange = () => setDark(media.matches)
    media.addEventListener('change', onChange)
    return () => media.removeEventListener('change', onChange)
  }, [])

  const theme = prefs.theme === 'system' ? (dark ? 'dark' : 'light') : prefs.theme

  useEffect(() => {
    document.documentElement.dataset.theme = theme
    document.documentElement.dataset.palette = prefs.palette
    try {
      localStorage.setItem(KEY, JSON.stringify({ ...prefs, version: VERSION }))
    } catch {
      // Storage unavailable: choices last for this session.
    }
  }, [prefs, theme])

  const update = (patch: Partial<Prefs>) => setPrefs((p) => ({ ...p, ...patch }))
  return { prefs, update, theme }
}
