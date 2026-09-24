import type { Dependency, Ecosystem, Inventory, Project, Status, Vulnerability } from './types'

export const ECOSYSTEMS: Ecosystem[] = ['npm', 'cargo', 'nuget', 'github-actions']

export const ECOSYSTEM_LABEL: Record<Ecosystem, string> = {
  npm: 'npm',
  cargo: 'Cargo',
  nuget: 'NuGet',
  'github-actions': 'Actions',
}

const STATUS_RANK: Record<Status, number> = {
  major: 7,
  minor: 6,
  patch: 5,
  unknown: 4,
  unpinned: 3,
  pending: 2,
  'up-to-date': 1,
  local: 0,
}

export const STATUS_LABEL: Record<Status, string> = {
  major: 'Major behind',
  minor: 'Minor behind',
  patch: 'Patch behind',
  unknown: 'Unknown',
  unpinned: 'Branch ref',
  pending: 'Not checked',
  'up-to-date': 'Current',
  local: 'Local',
}

export const isOutdated = (s: Status) => s === 'major' || s === 'minor' || s === 'patch'

export function worstStatus(statuses: Status[]): Status {
  return statuses.reduce<Status>((worst, s) => (STATUS_RANK[s] > STATUS_RANK[worst] ? s : worst), 'local')
}

export const SEVERITY_RANK: Record<string, number> = { CRITICAL: 4, HIGH: 3, MODERATE: 2, MEDIUM: 2, LOW: 1 }

export interface Usage {
  project: Project
  dep: Dependency
}

/** One package across every project that uses it. */
export interface PackageGroup {
  key: string
  ecosystem: Ecosystem
  name: string
  latest: string | null
  usages: Usage[]
  /** Version in use -> usages, ordered newest first. */
  versions: { version: string; status: Status; approximate: boolean; usages: Usage[] }[]
  worst: Status
  vulnIds: string[]
}

export function displayVersion(dep: Dependency): string {
  return dep.current ?? dep.requested ?? '?'
}

export function groupPackages(inventory: Inventory): PackageGroup[] {
  const groups = new Map<string, PackageGroup>()
  for (const project of inventory.projects) {
    for (const dep of project.dependencies) {
      if (dep.status === 'local') continue
      const key = `${dep.ecosystem}:${dep.name}`
      let group = groups.get(key)
      if (!group) {
        group = { key, ecosystem: dep.ecosystem, name: dep.name, latest: dep.latest, usages: [], versions: [], worst: 'local', vulnIds: [] }
        groups.set(key, group)
      }
      const best = dep.newest ?? dep.latest
      if (best && (!group.latest || compareVersions(best, group.latest) > 0)) group.latest = best
      group.usages.push({ project, dep })
      const version = displayVersion(dep)
      let bucket = group.versions.find((v) => v.version === version)
      if (!bucket) {
        bucket = { version, status: dep.status, approximate: dep.approximate, usages: [] }
        group.versions.push(bucket)
      }
      bucket.usages.push({ project, dep })
      bucket.approximate &&= dep.approximate
      for (const id of dep.vulns) if (!group.vulnIds.includes(id)) group.vulnIds.push(id)
    }
  }
  for (const group of groups.values()) {
    group.worst = worstStatus(group.usages.map((u) => u.dep.status))
    group.versions.sort((a, b) => compareVersions(b.version, a.version))
  }
  return [...groups.values()]
}

export function compareVersions(a: string, b: string): number {
  const parse = (v: string) => v.replace(/^v/i, '').split(/[.-]/).map((p) => Number.parseInt(p, 10) || 0)
  const pa = parse(a)
  const pb = parse(b)
  for (let i = 0; i < Math.max(pa.length, pb.length); i++) {
    const d = (pa[i] ?? 0) - (pb[i] ?? 0)
    if (d !== 0) return d
  }
  return 0
}

/**
 * True when `current` is older than `target`, compared only as precisely as
 * `current` is written: a floating `v7` is not behind `v7.0.1`.
 */
export function isBehind(current: string, target: string): boolean {
  const parse = (v: string) => v.replace(/^v/i, '').split('-')[0].split('.').map((p) => Number.parseInt(p, 10))
  const c = parse(current)
  const t = parse(target)
  if (c.some(Number.isNaN) || t.some(Number.isNaN)) return false
  for (let i = 0; i < c.length; i++) {
    const d = c[i] - (t[i] ?? 0)
    if (d !== 0) return d < 0
  }
  return false
}

/** Default commit message for an update, conventional-commit style. */
export function commitMessageFor(changes: { name: string; to: string }[]): string {
  if (changes.length === 1) return `chore(deps): update ${changes[0].name} to ${changes[0].to}`
  return `chore(deps): update ${changes.length} packages\n\n${changes.map((c) => `- ${c.name} to ${c.to}`).join('\n')}`
}

/** A usage that can be moved to another version by the updater. */
export function canRetarget(dep: Dependency): boolean {
  return dep.status !== 'local' && dep.status !== 'unpinned' && dep.status !== 'unknown' && !!dep.current
}

export interface Summary {
  projects: number
  packages: number
  outdated: number
  major: number
  drift: number
  vulnerable: number
  bySeverity: Record<string, number>
}

export function summarize(inventory: Inventory, groups: PackageGroup[]): Summary {
  const bySeverity: Record<string, number> = {}
  for (const v of inventory.vulnerabilities) {
    const s = normalizeSeverity(v.severity)
    bySeverity[s] = (bySeverity[s] ?? 0) + 1
  }
  return {
    projects: inventory.projects.length,
    packages: groups.length,
    outdated: groups.filter((g) => isOutdated(g.worst)).length,
    major: groups.filter((g) => g.worst === 'major').length,
    drift: groups.filter((g) => g.versions.length > 1).length,
    vulnerable: groups.filter((g) => g.vulnIds.length > 0).length,
    bySeverity,
  }
}

export function normalizeSeverity(s: string | null): string {
  if (!s) return 'UNRATED'
  return s === 'MEDIUM' ? 'MODERATE' : s
}

/** vuln id -> every place it shows up. */
export function vulnUsages(inventory: Inventory): Map<string, Usage[]> {
  const map = new Map<string, Usage[]>()
  for (const project of inventory.projects) {
    for (const dep of project.dependencies) {
      for (const id of dep.vulns) {
        const list = map.get(id) ?? []
        list.push({ project, dep })
        map.set(id, list)
      }
    }
  }
  return map
}

export function vulnById(inventory: Inventory): Map<string, Vulnerability> {
  return new Map(inventory.vulnerabilities.map((v) => [v.id, v]))
}

/** Path shown relative to whichever watched folder contains it. */
export function relativePath(roots: string[], path: string): string {
  const root = roots.map((r) => r.replace(/[\\/]+$/, '')).find((r) => isWithin(path, r))
  if (!root) return path
  const rel = path.slice(root.length).replace(/^[\\/]/, '') || '.'
  return roots.length > 1 ? `${root.split(/[\\/]/).pop()}\\${rel}` : rel
}

const normalizePath = (p: string) => p.replace(/\//g, '\\').replace(/\\+$/, '').toLowerCase()

/** True when `path` is `folder` or inside it (case-insensitive, either slash). */
export function isWithin(path: string, folder: string): boolean {
  const p = normalizePath(path)
  const f = normalizePath(folder)
  return p === f || p.startsWith(`${f}\\`)
}

export function samePath(a: string, b: string): boolean {
  return normalizePath(a) === normalizePath(b)
}

/** The repository, or outside git the project folder: one rail entry and one update job. */
export const repoKey = (p: Project) => p.repo ?? p.dir

export const folderName = (path: string) => path.replace(/[\\/]+$/, '').split(/[\\/]/).pop() || path

export type Risk = 'security' | 'major' | 'minor' | 'patch'

export const RISK_ORDER: Risk[] = ['security', 'major', 'minor', 'patch']

export interface QueueUsage {
  key: string
  project: Project
  dep: Dependency
  /** The version this project would move to: the newest it can use. */
  target: string
}

/** One package that has an update somewhere, with every usage that can take it. */
export interface QueueRow {
  key: string
  name: string
  ecosystem: Ecosystem
  usages: QueueUsage[]
  risk: Risk
  vulnIds: string[]
}

export const usageKey = (project: Project, dep: Dependency) => `${project.id}|${dep.name}|${dep.requested}`

/** Where a dependency can move to, or null when there is nothing to do. */
export function updateTarget(dep: Dependency): string | null {
  if (!canRetarget(dep) || !dep.latest || !isOutdated(dep.status)) return null
  return dep.latest
}

export function riskOf(usages: { dep: Dependency }[]): Risk {
  if (usages.some((u) => u.dep.vulns.length > 0)) return 'security'
  const worst = worstStatus(usages.map((u) => u.dep.status))
  return worst === 'major' || worst === 'minor' ? worst : 'patch'
}

export function queueRows(inventory: Inventory): QueueRow[] {
  const rows = new Map<string, QueueRow>()
  for (const project of inventory.projects) {
    for (const dep of project.dependencies) {
      const target = updateTarget(dep)
      if (!target) continue
      const key = `${dep.ecosystem}:${dep.name}`
      const row = rows.get(key) ?? { key, name: dep.name, ecosystem: dep.ecosystem, usages: [], risk: 'patch', vulnIds: [] }
      row.usages.push({ key: usageKey(project, dep), project, dep, target })
      for (const id of dep.vulns) if (!row.vulnIds.includes(id)) row.vulnIds.push(id)
      rows.set(key, row)
    }
  }
  for (const row of rows.values()) row.risk = riskOf(row.usages)
  return [...rows.values()]
}

/** Versions in a list, lowest first, without repeats. */
export function distinctVersions(versions: string[]): string[] {
  return [...new Set(versions)].sort(compareVersions)
}

export interface Repo {
  key: string
  name: string
  /** The watched folder it was found in. */
  root: string | null
  projects: Project[]
  ecosystems: Ecosystem[]
  /** Packages with an update available. */
  updates: number
  /** Packages with a known vulnerability. */
  vulnerable: number
  dependencies: number
}

export function repos(inventory: Inventory, rows: QueueRow[]): Repo[] {
  const map = new Map<string, Repo>()
  for (const project of inventory.projects) {
    const key = repoKey(project)
    const id = key.toLowerCase()
    const repo = map.get(id) ?? {
      key,
      name: folderName(key),
      root: inventory.roots.find((r) => isWithin(key, r)) ?? null,
      projects: [],
      ecosystems: [],
      updates: 0,
      vulnerable: 0,
      dependencies: 0,
    }
    repo.projects.push(project)
    if (!repo.ecosystems.includes(project.ecosystem)) repo.ecosystems.push(project.ecosystem)
    repo.dependencies += project.dependencies.length
    map.set(id, repo)
  }
  for (const row of rows) {
    const touched = new Set(row.usages.map((u) => repoKey(u.project).toLowerCase()))
    for (const id of touched) {
      const repo = map.get(id)
      if (!repo) continue
      repo.updates++
      if (row.vulnIds.length && row.usages.some((u) => u.dep.vulns.length > 0 && repoKey(u.project).toLowerCase() === id)) repo.vulnerable++
    }
  }
  for (const repo of map.values()) repo.ecosystems.sort((a, b) => ECOSYSTEMS.indexOf(a) - ECOSYSTEMS.indexOf(b))
  // Folders with the same name in different places get their parent folder too.
  const counts = new Map<string, number>()
  for (const repo of map.values()) counts.set(repo.name.toLowerCase(), (counts.get(repo.name.toLowerCase()) ?? 0) + 1)
  for (const repo of map.values()) {
    if ((counts.get(repo.name.toLowerCase()) ?? 0) > 1) repo.name = `${folderName(repo.key.replace(/[\\/][^\\/]*$/, ''))}/${repo.name}`
  }
  return [...map.values()].sort((a, b) => a.name.localeCompare(b.name, undefined, { sensitivity: 'base' }))
}

export type ProjectType = 'web' | 'rust' | 'dotnet'

export const PROJECT_TYPE_LABEL: Record<ProjectType, string> = { web: 'JavaScript / Web', rust: 'Rust', dotnet: '.NET' }

export function projectTypes(ecosystems: Ecosystem[]): ProjectType[] {
  const types: ProjectType[] = []
  if (ecosystems.includes('npm')) types.push('web')
  if (ecosystems.includes('cargo')) types.push('rust')
  if (ecosystems.includes('nuget')) types.push('dotnet')
  return types
}
