import type { Change, Conflict, Dependency, Ecosystem, Inventory, Project, Status, VersionPolicies, VersionPolicy, Vulnerability } from './types'

export const ECOSYSTEMS: Ecosystem[] = ['npm', 'pypi', 'cargo', 'nuget', 'go', 'pub', 'packagist', 'rubygems', 'github-actions']

export const ECOSYSTEM_LABEL: Record<Ecosystem, string> = {
  npm: 'npm',
  cargo: 'Cargo',
  nuget: 'NuGet',
  'github-actions': 'Actions',
  go: 'Go',
  pypi: 'PyPI',
  pub: 'Pub',
  packagist: 'Composer',
  rubygems: 'RubyGems',
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

/** `2.0.0-rc.22` -> parts [2, 0, 0], pre ['rc', '22']; a spec like `^1.2` is read as its version. */
function parseVersion(v: string): { parts: number[]; pre: string[] | null } {
  const s = v.trim().replace(/^[^\d]+/, '')
  const dash = s.search(/[-+]/)
  const core = dash < 0 ? s : s.slice(0, dash)
  const pre = dash >= 0 && s[dash] === '-' ? s.slice(dash + 1).split('+')[0].split('.') : null
  return { parts: core.split('.').map((p) => Number.parseInt(p, 10) || 0), pre }
}

/** Semver's prerelease order: none sorts last, numbers before words, shorter first. */
function comparePre(a: string[] | null, b: string[] | null): number {
  if (!a || !b) return a ? -1 : b ? 1 : 0
  for (let i = 0; i < Math.min(a.length, b.length); i++) {
    const [x, y] = [/^\d+$/.test(a[i]), /^\d+$/.test(b[i])]
    const d = x && y ? Number(a[i]) - Number(b[i]) : x !== y ? (x ? -1 : 1) : a[i] < b[i] ? -1 : a[i] > b[i] ? 1 : 0
    if (d !== 0) return d
  }
  return a.length - b.length
}

export function compareVersions(a: string, b: string): number {
  const [pa, pb] = [parseVersion(a), parseVersion(b)]
  for (let i = 0; i < Math.max(pa.parts.length, pb.parts.length); i++) {
    const d = (pa.parts[i] ?? 0) - (pb.parts[i] ?? 0)
    if (d !== 0) return d
  }
  return comparePre(pa.pre, pb.pre)
}

/** The part a breaking release bumps: the major, or for 0.x the first part that is not zero. */
const breakingPart = (parts: number[]) => Math.min(parts.findIndex((p) => p !== 0) < 0 ? parts.length - 1 : parts.findIndex((p) => p !== 0), 2, parts.length - 1)

/**
 * True when `current` is older than `target`, compared only as precisely as
 * `current` is written: a floating `v7` is not behind `v7.0.1`.
 */
export function isBehind(current: string, target: string): boolean {
  const raw = (v: string) => v.replace(/^v/i, '').split(/[-+]/)[0].split('.').map((p) => Number.parseInt(p, 10))
  const c = raw(current)
  const t = raw(target)
  if (c.some(Number.isNaN) || t.some(Number.isNaN)) return false
  for (let i = 0; i < c.length; i++) {
    const d = c[i] - (t[i] ?? 0)
    if (d !== 0) return d < 0
  }
  return !!parseVersion(current).pre && compareVersions(current, target) < 0
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
  /** Other packages in this project that must move in the same update. */
  together: string[]
  /** The target was picked by hand rather than worked out. */
  chosen?: boolean
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

export const POLICY_LABEL: Record<VersionPolicy, string> = { any: 'Any stable version', minor: 'Minor and patch', patch: 'Bug fixes only' }

/** The policy for a project: its own, else the one for every project, else any. */
export function policyFor(policies: VersionPolicies, project: Project): VersionPolicy {
  const key = repoKey(project).toLowerCase()
  const own = Object.entries(policies).find(([scope]) => scope.toLowerCase() === key)
  return own?.[1] ?? policies['*'] ?? 'any'
}

/** Where a dependency can move to under a policy, or null when there is nothing to do. */
export function updateTarget(dep: Dependency, policy: VersionPolicy = 'any'): string | null {
  if (!canRetarget(dep) || !dep.latest || !isOutdated(dep.status)) return null
  if (policy === 'any') return dep.latest
  if (policy === 'minor') return dep.safeLatest ?? (dep.status === 'major' ? null : dep.latest)
  const patch = dep.patchLatest ?? (dep.status === 'patch' ? dep.latest : null)
  return patch && dep.current && isBehind(dep.current, patch) ? patch : null
}

/**
 * The update level's target, except that a known vulnerability is never
 * hidden by it: when the level stops short of the fix, the smallest fix is
 * offered instead.
 */
function levelTarget(dep: Dependency, policy: VersionPolicy): string | null {
  const target = updateTarget(dep, policy)
  if (policy === 'any' || !dep.vulns.length || !canRetarget(dep)) return target
  if (dep.fixTarget) return target && compareVersions(target, dep.fixTarget) >= 0 ? target : dep.fixTarget
  return target ?? updateTarget(dep, 'any')
}

/** How big the jump from `current` to `target` is. */
export function bumpOf(current: string, target: string): Exclude<Risk, 'security'> {
  // Tolerates a written spec like `^5.1.2` or `>=5` as well as a version.
  // Levels follow `holdLine`: 0.12 to 0.13 is major, and so is leaving a prerelease.
  const [c, t] = [parseVersion(current), parseVersion(target)]
  if (c.pre && compareVersions(current, target) !== 0) return 'major'
  const shift = breakingPart(c.parts)
  const at = Array.from({ length: Math.max(c.parts.length, t.parts.length) }, (_, i) => i).find((i) => (c.parts[i] ?? 0) !== (t.parts[i] ?? 0))
  if (at === undefined) return t.pre ? 'major' : 'patch'
  return at <= shift ? 'major' : at === shift + 1 ? 'minor' : 'patch'
}

export function riskOf(usages: { dep: Dependency; target?: string }[]): Risk {
  if (usages.some((u) => u.dep.vulns.length > 0)) return 'security'
  const bumps = usages.map((u) => (u.target && u.dep.current ? bumpOf(u.dep.current, u.target) : u.dep.status === 'major' || u.dep.status === 'minor' ? u.dep.status : 'patch'))
  return bumps.includes('major') ? 'major' : bumps.includes('minor') ? 'minor' : 'patch'
}

/**
 * Every package with somewhere to go, per project. `chosen` holds versions
 * picked by hand (by usage key); those count even where the policy would
 * leave the package alone.
 */
export function queueRows(inventory: Inventory, policies: VersionPolicies = {}, chosen: Map<string, string> = new Map()): QueueRow[] {
  const rows = new Map<string, QueueRow>()
  for (const project of inventory.projects) {
    const policy = policyFor(policies, project)
    // A framework's parts move to their group's versions, together, when
    // the project allows major updates at all.
    const grouped = (dep: Dependency) => (policy === 'any' && dep.group && dep.groupTarget && canRetarget(dep) ? dep.groupTarget : null)
    const members = new Map<string, string[]>()
    for (const dep of project.dependencies) if (grouped(dep)) members.set(dep.group!, [...(members.get(dep.group!) ?? []), dep.name])
    for (const dep of project.dependencies) {
      const picked = chosen.get(usageKey(project, dep))
      const custom = picked && picked !== dep.current ? picked : null
      const target = custom ?? grouped(dep) ?? levelTarget(dep, policy)
      if (!target) continue
      const key = `${dep.ecosystem}:${dep.name}`
      const row = rows.get(key) ?? { key, name: dep.name, ecosystem: dep.ecosystem, usages: [], risk: 'patch', vulnIds: [] }
      const together = grouped(dep) ? [...new Set(members.get(dep.group!) ?? [])].filter((n) => n !== dep.name) : []
      row.usages.push({ key: usageKey(project, dep), project, dep, target, together, chosen: !!custom })
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

export type ProjectType = 'web' | 'python' | 'rust' | 'dotnet' | 'dotnet-framework' | 'go' | 'dart' | 'php' | 'ruby'

export const PROJECT_TYPE_LABEL: Record<ProjectType, string> = { web: 'JavaScript / Web', python: 'Python', rust: 'Rust', dotnet: '.NET', 'dotnet-framework': '.NET Framework', go: 'Go', dart: 'Dart / Flutter', php: 'PHP', ruby: 'Ruby' }

/** `net48`, `net472`, `v4.7.2`: the Windows-only .NET Framework, not modern .NET (`net8.0`, `netstandard2.0`). */
const isFrameworkTarget = (tfm: string) => /^net\d{2,3}$/i.test(tfm) || /^v[1-4](\.|$)/i.test(tfm)

/** Which kinds of project these are. A NuGet project targeting both lines counts as both. */
export function projectTypes(projects: Project[]): ProjectType[] {
  const types = new Set<ProjectType>()
  for (const p of projects) {
    if (p.ecosystem === 'npm') types.add('web')
    if (p.ecosystem === 'cargo') types.add('rust')
    if (p.ecosystem === 'go') types.add('go')
    if (p.ecosystem === 'pypi') types.add('python')
    if (p.ecosystem === 'pub') types.add('dart')
    if (p.ecosystem === 'packagist') types.add('php')
    if (p.ecosystem === 'rubygems') types.add('ruby')
    if (p.ecosystem !== 'nuget') continue
    // packages.config belongs to .NET Framework projects.
    if (p.frameworks.length === 0) types.add(/packages\.config$/i.test(p.manifest) ? 'dotnet-framework' : 'dotnet')
    for (const tfm of p.frameworks) types.add(isFrameworkTarget(tfm) ? 'dotnet-framework' : 'dotnet')
  }
  return (Object.keys(PROJECT_TYPE_LABEL) as ProjectType[]).filter((t) => types.has(t))
}

/** The release line a version sits on: `5` for 5.1.2, `0.13` for 0.13.4, `0.0.9` for 0.0.9 (where every release can break). */
export function holdLine(version: string): string {
  const { parts } = parseVersion(version)
  return parts.slice(0, breakingPart(parts) + 1).join('.')
}

/** A package an update left behind after it broke the project, and what went in instead. */
export interface Fallback {
  name: string
  /** The version that was tried. */
  tried: string
  /** The newest release on the line it was on, or null when it stayed as it was. */
  to: string | null
  /** The version it stays on when `to` is null. */
  stays: string
  /** Set when this package only went with `with`, which is the one that broke it. */
  with: string | null
  keep: NonNullable<Conflict['keep']>
}

/**
 * The same update without the packages a failure points at. Each one moves to
 * the newest release on the line it was already on, when that is newer than
 * what the project has, and otherwise stays out. A package that only moves
 * together with a culprit (a framework's parts) is held back too when its own
 * update also leaves its line; one that stays on its line goes ahead. `null`
 * when nothing would change or nothing would be left to update.
 */
export function withoutCulprits(targets: { project: Project; changes: Change[] }[], culprits: NonNullable<Conflict['keep']>[]): { targets: { project: Project; changes: Change[] }[]; fallbacks: Fallback[] } | null {
  const lower = (s: string) => s.toLowerCase()
  const bare = (v: string) => v.replace(/^[^\d]*/, '')
  const deps = targets.flatMap((t) => t.project.dependencies)
  const leaderOf = (name: string) => deps.find((d) => lower(d.name) === lower(name))?.group ?? name
  const culpritOf = (name: string) => culprits.find((k) => lower(k.name) === lower(name))
  const partner = (name: string) => culprits.find((k) => lower(leaderOf(k.name)) === lower(leaderOf(name)))
  const fallbacks: Fallback[] = []
  let changed = false
  const next = targets
    .map((t) => ({
      ...t,
      changes: t.changes.flatMap((c) => {
        const dep = t.project.dependencies.find((d) => lower(d.name) === lower(c.name))
        const from = dep?.current ?? bare(c.from)
        const leaves = bumpOf(from, bare(c.to)) === 'major'
        const own = culpritOf(c.name)
        const mate = own ? null : partner(c.name)
        // Only an update that leaves its line is held back.
        if (!leaves || !(own || mate)) return [c]
        changed = true
        const keep = own ?? { ecosystem: t.project.ecosystem, name: c.name, line: holdLine(from), from, to: c.to }
        const safe = dep?.safeLatest
        const to = safe && dep?.current && holdLine(safe) === keep.line && isBehind(dep.current, safe) && safe !== c.to ? safe : null
        if (!fallbacks.some((f) => lower(f.name) === lower(c.name))) fallbacks.push({ name: c.name, tried: c.to, to, stays: from, with: mate?.name ?? null, keep })
        return to ? [{ ...c, to }] : []
      }),
    }))
    .filter((t) => t.changes.length > 0)
  return changed && next.length ? { targets: next, fallbacks } : null
}

/** One package's update across a repository's projects, with the packages that only move with it. */
export interface UpdateUnit {
  key: string
  /** A breaking move (see `bumpOf`), tried on its own so it can only hold back itself. */
  major: boolean
  parts: { project: Project; change: Change }[]
}

const depFor = (project: Project, change: Change) => {
  const named = project.dependencies.filter((d) => d.name.toLowerCase() === change.name.toLowerCase())
  return named.find((d) => d.requested === change.from) ?? named[0]
}

export function unitsOf(targets: { project: Project; changes: Change[] }[]): UpdateUnit[] {
  const units = new Map<string, UpdateUnit>()
  for (const t of targets) {
    for (const change of t.changes) {
      const dep = depFor(t.project, change)
      const key = `${t.project.ecosystem}:${dep?.group ?? change.name}`.toLowerCase()
      const unit = units.get(key) ?? { key, major: false, parts: [] }
      unit.major ||= bumpOf(dep?.current ?? change.from, change.to) === 'major'
      unit.parts.push({ project: t.project, change })
      units.set(key, unit)
    }
  }
  return [...units.values()]
}

export function targetsOf(units: UpdateUnit[]): { project: Project; changes: Change[] }[] {
  const byProject = new Map<string, { project: Project; changes: Change[] }>()
  for (const { project, change } of units.flatMap((u) => u.parts)) {
    const target = byProject.get(project.id) ?? { project, changes: [] }
    target.changes.push(change)
    byProject.set(project.id, target)
  }
  return [...byProject.values()]
}

/**
 * The order a repository's updates are tried in: everything that stays on
 * its release line together first, then each breaking move on its own, so
 * one that breaks the project holds back only itself.
 */
export function stagesOf(units: UpdateUnit[]): UpdateUnit[][] {
  const small = units.filter((u) => !u.major)
  return [...(small.length ? [small] : []), ...units.filter((u) => u.major).map((u) => [u])]
}

/** A failed try split in two, to find the part that breaks. */
export function halves<T>(list: T[]): T[][] {
  const mid = Math.ceil(list.length / 2)
  return [list.slice(0, mid), list.slice(mid)]
}

/** The packages of a unit that broke the project on its own, as left out. */
export function leftOut(unit: UpdateUnit): Fallback[] {
  const lead = unit.key.slice(unit.key.indexOf(':') + 1)
  return unit.parts
    .filter((p, i, all) => all.findIndex((q) => q.change.name === p.change.name) === i)
    .map(({ project, change }) => {
      const stays = depFor(project, change)?.current ?? change.from.replace(/^[^\d]*/, '')
      return {
        name: change.name,
        tried: change.to,
        to: null,
        stays,
        with: change.name.toLowerCase() === lead ? null : (unit.parts.find((p) => p.change.name.toLowerCase() === lead)?.change.name ?? null),
        keep: { ecosystem: project.ecosystem, name: change.name, line: holdLine(stays), from: stays, to: change.to },
      }
    })
}

/** The engine's reason, as a sentence: "kept on 5.x" -> "Kept on 5.x". */
export const reasonText = (reason: string) => reason.charAt(0).toUpperCase() + reason.slice(1)

/** A package with a newer release that does not fit some projects. */
export interface HeldBack {
  key: string
  name: string
  ecosystem: Ecosystem
  newest: string
  entries: { project: Project; current: string; reason: string }[]
}

export function heldBack(projects: Project[]): HeldBack[] {
  const map = new Map<string, HeldBack>()
  for (const project of projects) {
    for (const dep of project.dependencies) {
      // A group member is offered in the list, moving with the others.
      if (!dep.newest || !dep.blockedReason || !dep.current || dep.groupTarget === dep.newest) continue
      const key = `${dep.ecosystem}:${dep.name}`
      const group = map.get(key) ?? { key, name: dep.name, ecosystem: dep.ecosystem, newest: dep.newest, entries: [] }
      if (compareVersions(dep.newest, group.newest) > 0) group.newest = dep.newest
      const same = group.entries.some((e) => samePath(repoKey(e.project), repoKey(project)) && e.current === dep.current && e.reason === dep.blockedReason)
      if (!same) group.entries.push({ project, current: dep.current, reason: dep.blockedReason })
      map.set(key, group)
    }
  }
  return [...map.values()].sort((a, b) => a.name.localeCompare(b.name, undefined, { sensitivity: 'base' }))
}

/** The update button's words for what runs after the install. */
export function runLabel(build: boolean, test: boolean): string {
  if (build && test) return 'Update, build & test'
  if (build) return 'Update & build'
  if (test) return 'Update & test'
  return 'Update & install only'
}
