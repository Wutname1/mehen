import { invoke } from '@tauri-apps/api/core'
import { listen, type UnlistenFn } from '@tauri-apps/api/event'
import { open } from '@tauri-apps/plugin-dialog'
import { openUrl, revealItemInDir } from '@tauri-apps/plugin-opener'
import { isWithin, samePath } from './derive'
import type { BatchEvent, BatchResult, Change, DiscoveredProject, IgnoreKind, IgnoreRule, Inventory, JobOutcome, Progress, Settings, StepResult, StoreStats, UpdateEvent, UpdateOutcome, UpdatePlan } from './types'

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

export async function setUpdateParallel(parallel: number): Promise<Settings> {
  if (!inTauri) return mock.setUpdateParallel(parallel)
  return invoke<Settings>('set_update_parallel', { parallel })
}

export async function setBackgroundHours(hours: number): Promise<Settings> {
  if (!inTauri) return mock.setBackgroundHours(hours)
  return invoke<Settings>('set_background_hours', { hours })
}

/** Results from checks started by the tray or the background schedule. */
export async function onInventory(handler: (inv: Inventory) => void): Promise<UnlistenFn> {
  if (!inTauri) return () => {}
  return listen<Inventory>('mehen://inventory', (e) => handler(e.payload))
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

export interface ApplyResult {
  outcome: UpdateOutcome
  inventory: Inventory | null
}

export async function planUpdate(projectId: string, changes: Change[]): Promise<UpdatePlan> {
  if (!inTauri) return mock.planUpdate(projectId, changes)
  return invoke<UpdatePlan>('plan_update', { projectId, changes })
}

export async function applyUpdate(plan: UpdatePlan, verify: boolean, rescan = true, commitMessage: string | null = null): Promise<ApplyResult> {
  if (!inTauri) return mock.applyUpdate(plan, verify, commitMessage)
  return invoke<ApplyResult>('apply_update', { plan, verify, rescan, commitMessage })
}

export async function onUpdateEvent(handler: (e: UpdateEvent) => void): Promise<UnlistenFn> {
  if (!inTauri) {
    mockUpdateListener = handler
    return () => {
      if (mockUpdateListener === handler) mockUpdateListener = null
    }
  }
  return listen<UpdateEvent>('mehen://update', (e) => handler(e.payload))
}

let mockUpdateListener: ((e: UpdateEvent) => void) | null = null

/**
 * Updates many projects at once: one job per repository, side by side except
 * where two need the same tool. `checks` runs builds and tests; `commit`
 * commits each repository that succeeds.
 */
export async function applyBatch(plans: UpdatePlan[], checks: boolean, commit: boolean): Promise<BatchResult> {
  if (!inTauri) return mock.applyBatch(plans, checks, commit)
  return invoke<BatchResult>('apply_batch', { plans, checks, commit })
}

export async function onBatchEvent(handler: (e: BatchEvent) => void): Promise<UnlistenFn> {
  if (!inTauri) {
    mockBatchListeners.add(handler)
    return () => {
      mockBatchListeners.delete(handler)
    }
  }
  return listen<BatchEvent>('mehen://batch', (e) => handler(e.payload))
}

const mockBatchListeners = new Set<(e: BatchEvent) => void>()

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
  let state: Settings = { folders: ['C:\\code'], rules: [], backgroundHours: 0, updateParallel: 2 }
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
    setBackgroundHours: async (hours: number) => (state = { ...state, backgroundHours: hours }),
    setUpdateParallel: async (parallel: number) => (state = { ...state, updateParallel: Math.min(8, Math.max(1, parallel)) }),
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
    // Fake plan and apply so the update flow can be exercised in a browser.
    planUpdate: async (projectId: string, changes: Change[]): Promise<UpdatePlan> => {
      const inv = await load()
      const project = inv?.projects.find((p) => p.id === projectId)
      if (!project) throw new Error('Project not found')
      const planned = changes.map((c) => ({ name: c.name, from: c.from, to: c.to, writtenBefore: c.from, writtenAfter: c.to }))
      const diff = [`--- ${project.manifest}`, `+++ ${project.manifest}`, '@@ -1,3 +1,3 @@', ...planned.flatMap((c) => [`-  "${c.name}": "${c.writtenBefore}",`, `+  "${c.name}": "${c.writtenAfter}",`])].join('\n')
      return {
        projectId,
        projectName: project.name,
        ecosystem: project.ecosystem,
        changes: planned,
        edits: [{ path: project.manifest, before: '', after: '', diff }],
        steps: [
          { kind: 'install', label: 'npm install', program: 'npm', args: ['install'], cwd: project.dir },
          { kind: 'verify', label: 'npm run build', program: 'npm', args: ['run', 'build'], cwd: project.dir },
          { kind: 'test', label: 'npm run test', program: 'npm', args: ['run', 'test'], cwd: project.dir },
        ],
        snapshots: [],
        warnings: [],
        repo: project.repo,
        commitBlocked: project.repo ? null : 'not inside a git repository',
      }
    },
    applyUpdate: async (plan: UpdatePlan, verify: boolean, commitMessage: string | null): Promise<ApplyResult> => {
      const steps = plan.steps.filter((s) => verify || s.kind === 'install')
      const results: StepResult[] = []
      for (const [index, step] of steps.entries()) {
        mockUpdateListener?.({ index, label: step.label, state: 'running' })
        await new Promise((r) => setTimeout(r, 700))
        mockUpdateListener?.({ index, label: step.label, state: 'ok' })
        results.push({ label: step.label, kind: step.kind, ok: true, output: 'done', ms: 700 })
      }
      if (commitMessage) {
        mockUpdateListener?.({ index: plan.steps.length, label: 'git commit', state: 'running' })
        await new Promise((r) => setTimeout(r, 300))
        mockUpdateListener?.({ index: plan.steps.length, label: 'git commit', state: 'ok' })
      }
      return {
        outcome: { ok: true, rolledBack: false, error: null, steps: results, committed: commitMessage ? 'abc1234' : null, commitError: null },
        inventory: await filtered(),
      }
    },
    // Mirrors the Rust runner: one job per repository, one step per tool at a time.
    applyBatch: async (plans: UpdatePlan[], checks: boolean, commit: boolean): Promise<BatchResult> => {
      const emit = (e: BatchEvent) => mockBatchListeners.forEach((l) => l(e))
      const jobs = new Map<string, UpdatePlan[]>()
      for (const p of plans) {
        const key = p.repo ?? p.projectId.replace(/[\\/][^\\/]*$/, '')
        jobs.set(key, [...(jobs.get(key) ?? []), p])
      }
      const lanes = new Map<string, Promise<void>>()
      const runJob = async ([job, list]: [string, UpdatePlan[]]): Promise<JobOutcome> => {
        const projects = list.map((p) => p.projectId)
        const results: StepResult[] = []
        for (const step of list.flatMap((p) => p.steps).filter((s) => checks || s.kind === 'install')) {
          const previous = lanes.get(step.program) ?? Promise.resolve()
          let release = () => {}
          lanes.set(step.program, previous.then(() => new Promise<void>((r) => (release = r))))
          emit({ job, projects, state: 'waiting', label: `Waiting for ${step.program}`, lane: step.program })
          await previous
          emit({ job, projects, state: 'running', label: step.label, lane: step.program })
          await new Promise((r) => setTimeout(r, 700))
          release()
          results.push({ label: step.label, kind: step.kind, ok: true, output: 'done', ms: 700 })
        }
        emit({ job, projects, state: 'done', label: null, lane: null })
        const repo = list[0].repo
        return { job, name: job.split(/[\\/]/).pop() ?? job, repo, projects, ok: true, rolledBack: false, error: null, steps: results, committed: commit && repo ? 'abc1234' : null, commitError: null, commitSkipped: commit && !repo ? 'not inside a git repository' : null }
      }
      for (const [job, list] of jobs) emit({ job, projects: list.map((p) => p.projectId), state: 'queued', label: null, lane: null })
      const outcomes = await Promise.all([...jobs].map(runJob))
      return { outcomes, inventory: await filtered() }
    },
  }
})()
