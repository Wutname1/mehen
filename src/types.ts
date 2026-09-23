// Mirrors crates/mehen-core/src/model.rs (serde camelCase / kebab-case).

export type Ecosystem = 'npm' | 'cargo' | 'nuget' | 'github-actions'
export type DepKind = 'normal' | 'dev' | 'build' | 'peer' | 'action'
export type Status = 'pending' | 'local' | 'unpinned' | 'unknown' | 'up-to-date' | 'patch' | 'minor' | 'major'

export interface Dependency {
  name: string
  ecosystem: Ecosystem
  kind: DepKind
  requested: string
  installed: string | null
  installedFrom: string | null
  pinnedComment: string | null
  current: string | null
  approximate: boolean
  latest: string | null
  status: Status
  vulns: string[]
  note: string | null
}

export interface Project {
  id: string
  name: string
  ecosystem: Ecosystem
  dir: string
  manifest: string
  repo: string | null
  frameworks: string[]
  dependencies: Dependency[]
}

export interface FixedIn {
  ecosystem: Ecosystem
  name: string
  versions: string[]
}

export interface Vulnerability {
  id: string
  aliases: string[]
  summary: string
  severity: string | null
  url: string
  fixed: FixedIn[]
}

export interface CheckStats {
  packagesCached: number
  packagesFetched: number
  packagesFailed: number
  osvCached: number
  osvFetched: number
  advisoriesCached: number
  advisoriesFetched: number
  throttled: string[]
}

export interface Inventory {
  roots: string[]
  projects: Project[]
  vulnerabilities: Vulnerability[]
  skippedWorktrees: string[]
  ignored: string[]
  warnings: string[]
  scanMs: number
  checkMs: number | null
  checkedAt: number | null
  checkStats: CheckStats | null
}

export interface Progress {
  phase: string
  done: number
  total: number
}

export interface StoreStats {
  packages: number
  osvQueries: number
  advisories: number
  scans: number
}

export type IgnoreKind = 'folder' | 'project' | 'pattern'

export interface IgnoreRule {
  id: number
  kind: IgnoreKind
  value: string
  note: string | null
}

export interface Settings {
  folders: string[]
  rules: IgnoreRule[]
}

export interface DiscoveredProject {
  id: string
  name: string
  ecosystem: Ecosystem
  dir: string
  manifest: string
  repo: string | null
  dependencyCount: number
  ignoredBy: number | null
}
