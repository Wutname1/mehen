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
      group.latest ??= dep.latest
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

/** Path shown relative to the scanned folder. */
export function relativePath(root: string, path: string): string {
  const r = root.replace(/[\\/]+$/, '')
  return path.toLowerCase().startsWith(r.toLowerCase()) ? path.slice(r.length).replace(/^[\\/]/, '') || '.' : path
}
