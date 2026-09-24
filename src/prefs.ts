import { useEffect, useState } from 'react'

/** Choices that only affect this window, kept in the webview's storage. */
export interface Prefs {
  palette: 'faience' | 'parchment'
  theme: 'dark' | 'light' | 'system'
  /** Run build and test steps when updating. */
  checks: boolean
  /** Commit each repository once its update succeeds. */
  commit: boolean
  /** Check every project when Mehen opens. */
  scanOnOpen: boolean
  /** Stop a project at its first failed check instead of running the rest. */
  stopOnFailure: boolean
  /** List security fixes and bigger jumps first. */
  riskFirst: boolean
  /** Order of the project list. */
  railSort: 'az' | 'za' | 'updates'
}

const DEFAULTS: Prefs = { palette: 'faience', theme: 'dark', checks: true, commit: false, scanOnOpen: true, stopOnFailure: true, riskFirst: true, railSort: 'az' }

/** Bumped when a default changes and older saved choices should pick it up. */
const VERSION = 2
const KEY = 'mehen-prefs'

function load(): Prefs {
  try {
    const saved = JSON.parse(localStorage.getItem(KEY) ?? '{}')
    // Version 2 turned on checking at launch for everyone.
    if ((saved.version ?? 1) < 2) delete saved.scanOnOpen
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
