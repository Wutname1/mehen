import { invoke } from '@tauri-apps/api/core'
import { listen, type UnlistenFn } from '@tauri-apps/api/event'
import { open } from '@tauri-apps/plugin-dialog'
import { openUrl, revealItemInDir } from '@tauri-apps/plugin-opener'
import { isWithin, samePath } from './derive'
import type { DiscoveredProject, IgnoreKind, IgnoreRule, Inventory, Progress, Settings, StoreStats } from './types'

/** False when the UI runs in a plain browser (vite dev without Tauri). */
export const inTauri = typeof window !== 'undefined' && '__TAURI_INTERNALS__' in window

export interface IgnoreResult {
  settings: Settings
  inventory: Inventory | null
}

export async function settings(): Promise<Settings> {
  if (!inTauri) return mock.settings()
  return invoke<Settings>('settings')
}

export async function addFolder(path: string): Promise<Settings> {
  if (!inTauri) return mock.addFolder(path)
  return invoke<Settings>('add_folder', { path })
}

export async function removeFolder(path: string): Promise<Settings> {
  if (!inTauri) return mock.removeFolder(path)
  return invoke<Settings>('remove_folder', { path })
}

export async function addIgnore(kind: IgnoreKind, value: string, note?: string): Promise<IgnoreResult> {
  if (!inTauri) return mock.addIgnore(kind, value)
  return invoke<IgnoreResult>('add_ignore', { kind, value, note: note ?? null })
}

export async function removeIgnore(id: number): Promise<Settings> {
  if (!inTauri) return mock.removeIgnore(id)
  return invoke<Settings>('remove_ignore', { id })
}

export async function discover(): Promise<DiscoveredProject[]> {
  if (!inTauri) return mock.discover()
  return invoke<DiscoveredProject[]>('discover')
}

export async function lastInventory(): Promise<Inventory | null> {
  if (!inTauri) return mock.inventory()
  return invoke<Inventory | null>('last_inventory')
}

export async function scanAndCheck(refresh: boolean): Promise<Inventory> {
  if (!inTauri) {
    const inv = await mock.inventory()
    if (!inv) throw new Error('No dev inventory available outside the desktop app')
    return inv
  }
  return invoke<Inventory>('scan_and_check', { refresh })
}

export async function storeStats(): Promise<StoreStats | null> {
  if (!inTauri) return null
  return invoke<StoreStats>('store_stats')
}

export async function onProgress(handler: (p: Progress) => void): Promise<UnlistenFn> {
  if (!inTauri) return () => {}
  return listen<Progress>('mehen://progress', (e) => handler(e.payload))
}

export async function pickFolder(): Promise<string | null> {
  if (!inTauri) return window.prompt('Folder to watch', 'C:\\code')
  const picked = await open({ directory: true, title: 'Choose a folder to watch' })
  return typeof picked === 'string' ? picked : null
}

export async function reveal(path: string) {
  if (inTauri) await revealItemInDir(path)
}

export async function openInEditor(path: string) {
  if (inTauri) await invoke('open_in_editor', { path })
}

export async function openLink(url: string) {
  if (inTauri) await openUrl(url)
  else window.open(url, '_blank', 'noopener')
}

// Browser-only stand-in for the Rust side, so the UI can be worked on in a
// plain browser. Uses a saved real scan (survey example with --json) and a
// rough copy of the ignore matching.
const mock = (() => {
  let state: Settings = { folders: ['C:\\code'], rules: [] }
  let nextId = 1
  let cached: Inventory | null = null

  const matches = (rule: IgnoreRule, path: string) => {
    if (rule.kind === 'project') return samePath(rule.value, path)
    if (rule.kind === 'folder') return isWithin(path, rule.value)
    const segments = path.toLowerCase().split(/[\\/]/)
    const pattern = rule.value.toLowerCase().replace(/\\/g, '/')
    return pattern.includes('/') ? path.toLowerCase().replace(/\\/g, '/').includes(`/${pattern}`) : segments.includes(pattern)
  }
  const ruleFor = (manifest: string, dir: string) => state.rules.find((r) => matches(r, manifest) || matches(r, dir))?.id ?? null

  async function load(): Promise<Inventory | null> {
    if (cached) return cached
    try {
      const res = await fetch('/dev-inventory.json')
      if (!res.ok) return null
      const inv = (await res.json()) as Inventory & { root?: string }
      cached = { ...inv, roots: inv.roots ?? [inv.root ?? 'C:\\code'], ignored: inv.ignored ?? [] }
      return cached
    } catch {
      return null
    }
  }

  async function filtered(): Promise<Inventory | null> {
    const inv = await load()
    if (!inv) return null
    const projects = inv.projects.filter((p) => ruleFor(p.manifest, p.dir) === null)
    const used = new Set(projects.flatMap((p) => p.dependencies.flatMap((d) => d.vulns)))
    return { ...inv, projects, vulnerabilities: inv.vulnerabilities.filter((v) => used.has(v.id)) }
  }

  return {
    settings: async () => state,
    addFolder: async (path: string) => (state = { ...state, folders: [...new Set([...state.folders, path])] }),
    removeFolder: async (path: string) => (state = { ...state, folders: state.folders.filter((f) => f !== path) }),
    addIgnore: async (kind: IgnoreKind, value: string): Promise<IgnoreResult> => {
      if (!state.rules.some((r) => r.kind === kind && r.value === value)) {
        state = { ...state, rules: [...state.rules, { id: nextId++, kind, value, note: null }] }
      }
      return { settings: state, inventory: await filtered() }
    },
    removeIgnore: async (id: number) => (state = { ...state, rules: state.rules.filter((r) => r.id !== id) }),
    discover: async (): Promise<DiscoveredProject[]> => {
      const inv = await load()
      return (inv?.projects ?? []).map((p) => ({
        id: p.id,
        name: p.name,
        ecosystem: p.ecosystem,
        dir: p.dir,
        manifest: p.manifest,
        repo: p.repo,
        dependencyCount: p.dependencies.length,
        ignoredBy: ruleFor(p.manifest, p.dir),
      }))
    },
    inventory: filtered,
  }
})()
