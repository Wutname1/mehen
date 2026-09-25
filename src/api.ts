import { getVersion } from '@tauri-apps/api/app'
import { invoke } from '@tauri-apps/api/core'
import { listen, type UnlistenFn } from '@tauri-apps/api/event'
import { open } from '@tauri-apps/plugin-dialog'
import { openUrl, revealItemInDir } from '@tauri-apps/plugin-opener'
import { compareVersions, isWithin, samePath } from './derive'
import type { OpenRepoRequest, AppUpdateProgress, BatchEvent, BatchResult, Change, Ecosystem, Hold, CheckCommands, VersionPolicies, VersionPolicy, CommitOutcome, DiscoveredProject, IgnoreKind, IgnoreRule, Inventory, JobOutcome, Progress, Project, Settings, StepResult, ReleaseNotes, StoreStats, UpdatePlan, VersionView, Move } from './types'

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

/** Checks every watched folder, or with `only` just those folders and repositories. */
export async function scanAndCheck(refresh: boolean, only: string[] | null = null): Promise<Inventory> {
  if (!inTauri) {
    await new Promise((r) => setTimeout(r, only ? 900 : 1500))
    const inv = await mock.inventory()
    if (!inv) throw new Error('No dev inventory available outside the desktop app')
    return inv
  }
  return invoke<Inventory>('scan_and_check', { refresh, only })
}

export async function checkCommands(): Promise<CheckCommands> {
  if (!inTauri) return { ...mockCheckCommands }
  return invoke<CheckCommands>('check_commands')
}

/** Sets the build and test commands for a scope; `null` goes back to Mehen's own. */
export async function setCheckCommands(scope: string, commands: string[] | null, cwd: string | null = null): Promise<CheckCommands> {
  if (!inTauri) {
    for (const k of Object.keys(mockCheckCommands)) if (k.toLowerCase() === scope.toLowerCase()) delete mockCheckCommands[k]
    if (commands) mockCheckCommands[scope] = { commands: commands.map((c) => c.trim()).filter(Boolean), cwd: cwd?.trim() || null }
    return { ...mockCheckCommands }
  }
  return invoke<CheckCommands>('set_check_commands', { scope, commands, cwd })
}

export async function versionPolicy(): Promise<VersionPolicies> {
  if (!inTauri) return { ...mockPolicies }
  return invoke<VersionPolicies>('version_policy')
}

/** Sets how far updates may go for a scope (`*` for every project); `null` removes it. */
export async function setVersionPolicy(scope: string, policy: VersionPolicy | null): Promise<VersionPolicies> {
  if (!inTauri) {
    for (const k of Object.keys(mockPolicies)) if (k.toLowerCase() === scope.toLowerCase()) delete mockPolicies[k]
    if (policy) mockPolicies[scope] = policy
    return { ...mockPolicies }
  }
  return invoke<VersionPolicies>('set_version_policy', { scope, policy })
}

const mockPolicies: VersionPolicies = {}

/** Every published version of a package, newest first, as one project sees it. */
export async function packageVersions(projectId: string, name: string): Promise<VersionView[]> {
  if (!inTauri) return mock.packageVersions(projectId, name)
  return invoke<VersionView[]>('package_versions', { projectId, name })
}

/** What else has to move for a package to go to `version` in one project. */
export async function moveWith(projectId: string, name: string, version: string): Promise<Move[]> {
  if (!inTauri) return mock.moveWith(projectId, name, version)
  return invoke<Move[]>('move_with', { projectId, name, version })
}

const releaseDateCache = new Map<string, Promise<Record<string, string>>>()

/** When each version came out, fetched once per package per session. */
export function releaseDates(ecosystem: Ecosystem, name: string): Promise<Record<string, string>> {
  const key = `${ecosystem}:${name}`
  let found = releaseDateCache.get(key)
  if (!found) {
    found = inTauri ? invoke<Record<string, string>>('release_dates', { ecosystem, name }) : mock.releaseDates(ecosystem, name)
    found.catch(() => releaseDateCache.delete(key))
    releaseDateCache.set(key, found)
  }
  return found
}

/** Logos for project folders; `discover` searches folders not looked at before. */
export async function repoIcons(repos: string[], discover: boolean): Promise<{ repo: string; dataUrl: string }[]> {
  if (!inTauri || !repos.length) return []
  return invoke<{ repo: string; dataUrl: string }[]>('repo_icons', { repos, discover })
}

export async function holds(): Promise<Hold[]> {
  if (!inTauri) return [...mockHolds]
  return invoke<Hold[]>('holds')
}

/** Keeps a package on `line` (`5` for 5.x) in `scope`, a project folder or `*`. Applies at the next check. */
export async function setHold(ecosystem: Ecosystem, name: string, scope: string, line: string): Promise<Hold[]> {
  if (!inTauri) {
    mockHolds = mockHolds.filter((h) => !(h.ecosystem === ecosystem && h.name === name && h.scope.toLowerCase() === scope.toLowerCase()))
    mockHolds.push({ id: ++mockHoldId, ecosystem, name, scope, line })
    return [...mockHolds]
  }
  return invoke<Hold[]>('set_hold', { ecosystem, name, scope, line })
}

export async function removeHold(id: number): Promise<Hold[]> {
  if (!inTauri) {
    mockHolds = mockHolds.filter((h) => h.id !== id)
    return [...mockHolds]
  }
  return invoke<Hold[]>('remove_hold', { id })
}

let mockHolds: Hold[] = []
let mockHoldId = 0

export async function setNotify(notify: boolean): Promise<Settings> {
  if (!inTauri) return mock.setNotify(notify)
  return invoke<Settings>('set_notify', { notify })
}

export async function setAppUpdateCheck(check: boolean): Promise<Settings> {
  if (!inTauri) return mock.setAppUpdateCheck(check)
  return invoke<Settings>('set_app_update_check', { check })
}

/** Turns crash and error reports on or off, for this window and the app behind it. */
export async function setErrorReports(enabled: boolean): Promise<Settings> {
  if (!inTauri) return mock.setErrorReports(enabled)
  return invoke<Settings>('set_error_reports', { enabled })
}

/** The running version of Mehen. */
export async function appVersion(): Promise<string> {
  if (!inTauri) return '0.1.0'
  return getVersion()
}

/** The newer Mehen version on offer, or null when up to date. */
export async function checkAppUpdate(): Promise<string | null> {
  if (!inTauri) return mock.appUpdate.version
  return invoke<string | null>('check_self_update')
}

/** Downloads the newer version without installing it. Resolves to its version, or null if there is none. */
export async function downloadAppUpdate(): Promise<string | null> {
  if (!inTauri) return mock.appUpdate.download()
  return invoke<string | null>('download_self_update')
}

/** Installs the downloaded version. Mehen closes and reopens, so this only returns on failure. */
export async function installAppUpdate(): Promise<void> {
  if (inTauri) await invoke('install_self_update')
}

/** Notes for every release after `current` up to `target`, newest first. */
export async function appReleaseNotes(current: string, target: string): Promise<ReleaseNotes[]> {
  if (!inTauri) return mock.appUpdate.notes
  return invoke<ReleaseNotes[]>('self_update_notes', { current, target })
}

export async function onAppUpdateProgress(handler: (p: AppUpdateProgress) => void): Promise<UnlistenFn> {
  if (!inTauri) {
    mock.appUpdate.listeners.add(handler)
    return () => mock.appUpdate.listeners.delete(handler)
  }
  return listen<AppUpdateProgress>('self-update://progress', (e) => handler(e.payload))
}

const mockCheckCommands: CheckCommands = {}

/** Commits updates that were applied without committing, one commit per repository. */
export async function commitUpdate(plans: UpdatePlan[]): Promise<CommitOutcome[]> {
  if (!inTauri) {
    await new Promise((r) => setTimeout(r, 500))
    const repos = [...new Set(plans.map((p) => p.repo ?? p.projectId))]
    return repos.map((job, i) => ({ job, name: job.split(/[\\/]/).pop() ?? job, committed: plans.find((p) => (p.repo ?? p.projectId) === job)?.repo ? `${(0xa04d173 + i * 977).toString(16)}` : null, error: null }))
  }
  return invoke<CommitOutcome[]>('commit_update', { plans })
}

export async function planUpdate(projectId: string, changes: Change[]): Promise<UpdatePlan> {
  if (!inTauri) return mock.planUpdate(projectId, changes)
  return invoke<UpdatePlan>('plan_update', { projectId, changes })
}


/**
 * Updates many projects at once: one job per repository, side by side except
 * where two need the same tool. `run` picks the builds and tests; `commit`
 * commits each repository that succeeds.
 */
export async function applyBatch(plans: UpdatePlan[], run: { build: boolean; test: boolean }, commit: boolean, stopOnFailure = true): Promise<BatchResult> {
  if (!inTauri) return mock.applyBatch(plans, run, commit)
  return invoke<BatchResult>('apply_batch', { plans, build: run.build, test: run.test, commit, stopOnFailure })
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

/** Jobs asked to stop in the browser preview; `*` is all of them. */
const mockCancelled = new Set<string>()

/** Stops one repository's running update, or every one when `job` is null. Its files are put back. */
export async function cancelUpdate(job: string | null): Promise<void> {
  if (!inTauri) {
    mockCancelled.add(job?.toLowerCase() ?? '*')
    return
  }
  return invoke<void>('cancel_update', { job })
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
  if (!inTauri) return window.prompt('Folder with your projects', 'C:\\code')
  const picked = await open({ directory: true, title: 'Choose a folder with your projects' })
  return typeof picked === 'string' ? picked : null
}

export async function reveal(path: string) {
  if (inTauri) await revealItemInDir(path)
}

export async function openInEditor(path: string) {
  if (inTauri) await invoke('open_in_editor', { path })
}

/** Whether GitWyrm, the git app, is installed to hand repositories to. */
export async function gitwyrmInstalled(): Promise<boolean> {
  if (!inTauri) return true
  return invoke<boolean>('gitwyrm_installed')
}

export async function openInGitWyrm(path: string) {
  if (inTauri) await invoke('open_in_gitwyrm', { path })
}

/** Project folders set up in GitWyrm that Mehen does not check yet. */
export async function gitwyrmFolders(): Promise<string[]> {
  if (!inTauri) return []
  return invoke<string[]>('gitwyrm_folders')
}

/** The repository Mehen was started to show, if any. Returns it once. */
export async function launchRepo(): Promise<OpenRepoRequest | null> {
  if (!inTauri) return null
  return invoke<OpenRepoRequest | null>('launch_repo')
}

/** A repository passed to Mehen while it was already running. */
export async function onOpenRepo(handler: (request: OpenRepoRequest) => void): Promise<UnlistenFn> {
  if (!inTauri) return () => {}
  return listen<OpenRepoRequest>('mehen://open-repo', (e) => handler(e.payload))
}

/** Creates or removes the Windows task that checks daily with Mehen closed. */
export async function setScheduledCheck(enabled: boolean): Promise<Settings> {
  if (!inTauri) return mock.setScheduledCheck(enabled)
  return invoke<Settings>('set_scheduled_check', { enabled })
}

export async function openLink(url: string) {
  if (inTauri) await openUrl(url)
  else window.open(url, '_blank', 'noopener')
}

// Browser-only stand-in for the Rust side, so the UI can be worked on in a
// plain browser. Uses a saved real scan (survey example with --json) and a
// rough copy of the ignore matching.
const mock = (() => {
  let state: Settings = { folders: ['C:\\code'], rules: [], backgroundHours: 0, updateParallel: 0, updateParallelAuto: 2, notify: true, appUpdateCheck: true, scheduledCheck: false, scheduledCheckSupported: true, errorReports: true }
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

  /** Roughly what the engine does with a hold: the newest release off the line is held back. */
  function withHolds(project: Project): Project {
    const scope = project.repo ?? project.dir
    const dependencies = project.dependencies.map((dep) => {
      const hold = mockHolds.find((h) => h.ecosystem === dep.ecosystem && h.name === dep.name && (h.scope === '*' || samePath(h.scope, scope)))
      const best = dep.newest ?? dep.latest
      if (!hold || !best || !dep.current) return dep
      const onLine = (v: string) => v.replace(/^v/i, '').startsWith(`${hold.line}.`)
      if (onLine(best)) return dep
      const within = { safeLatest: dep.safeLatest && onLine(dep.safeLatest) ? dep.safeLatest : null, patchLatest: dep.patchLatest && onLine(dep.patchLatest) ? dep.patchLatest : null }
      if (!onLine(dep.latest ?? '')) return { ...dep, ...within, latest: within.safeLatest ?? within.patchLatest ?? dep.current, newest: best, blockedReason: `kept on ${hold.line}.x`, status: within.safeLatest || within.patchLatest ? ('minor' as const) : ('up-to-date' as const) }
      return { ...dep, latest: onLine(dep.latest ?? '') ? dep.latest : dep.current, newest: best, blockedReason: `kept on ${hold.line}.x`, status: onLine(dep.latest ?? '') ? dep.status : ('up-to-date' as const) }
    })
    return { ...project, dependencies }
  }

  async function filtered(): Promise<Inventory | null> {
    const inv = await load()
    if (!inv) return null
    const projects = inv.projects.filter((p) => ruleFor(p.manifest, p.dir) === null).map(withHolds)
    const used = new Set(projects.flatMap((p) => p.dependencies.flatMap((d) => d.vulns)))
    return { ...inv, projects, vulnerabilities: inv.vulnerabilities.filter((v) => used.has(v.id)) }
  }

  return {
    settings: async () => state,
    setBackgroundHours: async (hours: number) => (state = { ...state, backgroundHours: hours }),
    setUpdateParallel: async (parallel: number) => (state = { ...state, updateParallel: Math.min(8, Math.max(0, parallel)) }),
    setNotify: async (notify: boolean) => (state = { ...state, notify }),
    setAppUpdateCheck: async (appUpdateCheck: boolean) => (state = { ...state, appUpdateCheck }),
    setScheduledCheck: async (scheduledCheck: boolean) => (state = { ...state, scheduledCheck }),
    setErrorReports: async (errorReports: boolean) => (state = { ...state, errorReports }),
    appUpdate: {
      version: '0.2.0',
      listeners: new Set<(p: AppUpdateProgress) => void>(),
      async download() {
        const total = 6_400_000
        for (let downloaded = 0; downloaded <= total; downloaded += 800_000) {
          this.listeners.forEach((l) => l({ downloaded, total }))
          await new Promise((r) => setTimeout(r, 180))
        }
        return this.version
      },
      notes: [
        {
          version: '0.2.0',
          releasedAt: '2026-09-24T12:00:00Z',
          items: [
            { section: 'feature', text: 'Mehen updates itself, and shows what changed before you restart', tags: [] },
            { section: 'fix', text: 'Checks no longer stop when a project folder is renamed', tags: [] },
            { section: 'change', text: 'The project list opens faster with many folders', tags: [] },
          ],
        },
      ] as ReleaseNotes[],
    },
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
    // Made-up release history from the current version up to the newest.
    packageVersions: async (projectId: string, name: string): Promise<VersionView[]> => {
      const dep = (await load())?.projects.find((p) => p.id === projectId)?.dependencies.find((d) => d.name === name)
      if (!dep?.current) throw new Error(`No version list for ${name}`)
      const partners = dep.group ? (await load())!.projects.find((p) => p.id === projectId)!.dependencies.filter((d) => d.group === dep.group).length - 1 : 0
      return mockVersions(dep.current, dep.newest ?? dep.latest ?? dep.current).map((version) => {
        const past = !partners && !!dep.newest && dep.latest && compareVersions(version, dep.latest) > 0
        return {
          version,
          prerelease: version.includes('-'),
          blocked: past ? (dep.blockedReason ?? 'needs a newer runtime') : null,
          together: partners && compareVersions(version, dep.current!) > 0 && !version.includes('-') ? partners : 0,
          requirements: version.includes('-') ? [] : [{ kind: 'node', range: `>=${18 + Math.max(0, Number.parseInt(version, 10) - Number.parseInt(dep.current!, 10))}` }],
        }
      })
    },
    moveWith: async (projectId: string, name: string, version: string): Promise<Move[]> => {
      const project = (await load())?.projects.find((p) => p.id === projectId)
      const dep = project?.dependencies.find((d) => d.name === name)
      if (!project || !dep) throw new Error(`${name} is not in that project`)
      const partners = dep.group ? project.dependencies.filter((d) => d.group === dep.group && d.name !== name) : []
      return [dep, ...partners].map((d) => ({ name: d.name, from: d.current ?? '', to: d === dep || d.current?.split('.')[0] === dep.current?.split('.')[0] ? version : (d.groupTarget ?? version) }))
    },
    releaseDates: async (_ecosystem: Ecosystem, name: string): Promise<Record<string, string>> => {
      const dep = (await load())?.projects.flatMap((p) => p.dependencies).find((d) => d.name === name)
      if (!dep?.current) return {}
      const versions = mockVersions(dep.current, dep.newest ?? dep.latest ?? dep.current)
      const day = 86_400_000
      return Object.fromEntries(versions.map((v, i) => [v, new Date(Date.now() - (i === 0 ? day : i * 23 * day)).toISOString()]))
    },
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
        // Like the real planner: workflow edits have nothing to run.
        steps:
          project.ecosystem === 'github-actions'
            ? []
            : [
                { kind: 'install', label: 'npm install', program: 'npm', args: ['install'], cwd: project.dir },
                { kind: 'verify', label: 'npm run build', program: 'npm', args: ['run', 'build'], cwd: project.dir },
                { kind: 'test', label: 'npm run test', program: 'npm', args: ['run', 'test'], cwd: project.dir },
              ],
        snapshots: [],
        warnings: [],
        repo: project.repo,
        commitBlocked: project.repo ? null : 'not inside a git repository',
        branch: project.repo ? 'main' : null,
      }
    },
    // Mirrors the Rust runner: one job per repository, one step per tool at a time.
    applyBatch: async (plans: UpdatePlan[], run: { build: boolean; test: boolean }, commit: boolean): Promise<BatchResult> => {
      const emit = (e: BatchEvent) => mockBatchListeners.forEach((l) => l(e))
      const jobs = new Map<string, UpdatePlan[]>()
      for (const p of plans) {
        const key = p.repo ?? p.projectId.replace(/[\\/][^\\/]*$/, '')
        jobs.set(key, [...(jobs.get(key) ?? []), p])
      }
      mockCancelled.clear()
      const lanes = new Map<string, Promise<void>>()
      const runJob = async ([job, list]: [string, UpdatePlan[]]): Promise<JobOutcome> => {
        const projects = list.map((p) => p.projectId)
        const results: StepResult[] = []
        const stopped = () => mockCancelled.has('*') || mockCancelled.has(job.toLowerCase())
        const cancelled = (): JobOutcome => {
          emit({ job, projects, state: 'cancelled', label: 'Cancelled', lane: null })
          return { job, name: job.split(/[\\/]/).pop() ?? job, repo: list[0].repo, projects, ok: false, rolledBack: true, cancelled: true, error: 'Cancelled', steps: results, committed: null, commitError: null, commitSkipped: null, conflicts: [] }
        }
        const major = (a: string, b: string) => a.replace(/^\D+/, '').split('.')[0] !== b.replace(/^\D+/, '').split('.')[0]
        const clash = list.flatMap((p) => p.changes).find((c) => /eslint/.test(c.name) && major(c.from, c.to))
        if (clash) {
          const line = clash.from.replace(/^\D+/, '').split('.')[0]
          const range = `^${line}.0.0`
          emit({ job, projects, state: 'running', label: 'npm install', lane: 'npm' })
          await new Promise((r) => setTimeout(r, 900))
          emit({ job, projects, state: 'rolled-back', label: 'npm install failed', lane: null })
          const output = `npm error code ERESOLVE
npm error ERESOLVE unable to resolve dependency tree
npm error
npm error Found: ${clash.name}@${clash.to}
npm error
npm error Could not resolve dependency:
npm error peer ${clash.name}@"${range}" from eslint-plugin-react-hooks@5.2.0`
          return {
            job,
            name: job.split(/[\\/]/).pop() ?? job,
            repo: list[0].repo,
            projects,
            ok: false,
            rolledBack: true,
            cancelled: false,
            error: '`npm install` failed',
            steps: [{ label: 'npm install', kind: 'install', ok: false, output, ms: 900 }],
            committed: null,
            commitError: null,
            commitSkipped: null,
            conflicts: [
              {
                summary: `eslint-plugin-react-hooks 5.2.0 needs ${clash.name} ${range}, not ${clash.to}`,
                keep: { ecosystem: 'npm', name: clash.name, line, from: clash.from, to: clash.to },
                blocking: true,
              },
            ],
          }
        }
        for (const step of list.flatMap((p) => p.steps).filter((s) => s.kind === 'install' || (s.kind === 'verify' ? run.build : run.test))) {
          const previous = lanes.get(step.program) ?? Promise.resolve()
          let release = () => {}
          lanes.set(step.program, previous.then(() => new Promise<void>((r) => (release = r))))
          emit({ job, projects, state: 'waiting', label: `Waiting for ${step.program}`, lane: step.program })
          await previous
          if (stopped()) {
            release()
            return cancelled()
          }
          emit({ job, projects, state: 'running', label: step.label, lane: step.program })
          for (let t = 0; t < 14 && !stopped(); t++) await new Promise((r) => setTimeout(r, 200))
          release()
          if (stopped()) return cancelled()
          results.push({ label: step.label, kind: step.kind, ok: true, output: 'done', ms: 2800 })
        }
        emit({ job, projects, state: 'done', label: null, lane: null })
        const repo = list[0].repo
        return { job, name: job.split(/[\\/]/).pop() ?? job, repo, projects, ok: true, rolledBack: false, cancelled: false, error: null, steps: results, committed: commit && repo ? 'abc1234' : null, commitError: null, commitSkipped: commit && !repo ? 'not inside a git repository' : null, conflicts: [] }
      }
      for (const [job, list] of jobs) emit({ job, projects: list.map((p) => p.projectId), state: 'queued', label: null, lane: null })
      const outcomes = await Promise.all([...jobs].map(runJob))
      return { outcomes, inventory: await filtered() }
    },
  }
})()

/** Newest first: each major from `current` to `newest`, with a few minors and patches, plus a pre-release. */
function mockVersions(current: string, newest: string): string[] {
  const [from, to] = [current, newest].map((v) => Number.parseInt(v.replace(/^\D+/, ''), 10) || 0)
  const out = [`${to + 1}.0.0-rc.1`, newest]
  for (let major = to; major >= from; major--) for (const minor of [2, 1, 0]) for (const patch of [3, 0]) out.push(`${major}.${minor}.${patch}`)
  return [...new Set(out.filter((v) => v === newest || v.includes('-') || compareVersions(v, newest) < 0))].sort((a, b) => compareVersions(b, a))
}
