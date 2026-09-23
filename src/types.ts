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
  safeLatest: string | null
  /** Newest published version when this project can't use it; `latest` is then the newest it can. */
  newest: string | null
  blockedReason: string | null
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
  rustVersion: string | null
  nodeVersion: string | null
  nodeEngines: string | null
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

export interface Change {
  name: string
  from: string
  to: string
}

export interface PlannedChange {
  name: string
  from: string
  to: string
  writtenBefore: string
  writtenAfter: string
}

export interface FileEdit {
  path: string
  before: string
  after: string
  diff: string
}

export type StepKind = 'install' | 'verify'

export interface Step {
  kind: StepKind
  label: string
  program: string
  args: string[]
  cwd: string
}

export interface UpdatePlan {
  projectId: string
  projectName: string
  ecosystem: Ecosystem
  changes: PlannedChange[]
  edits: FileEdit[]
  steps: Step[]
  snapshots: string[]
  warnings: string[]
  repo: string | null
  /** Why this update can't be committed, if it can't. */
  commitBlocked: string | null
}

export interface StepResult {
  label: string
  kind: StepKind
  ok: boolean
  output: string
  ms: number
}

export interface UpdateOutcome {
  ok: boolean
  rolledBack: boolean
  error: string | null
  steps: StepResult[]
  committed: string | null
  commitError: string | null
}

export interface UpdateEvent {
  index: number
  label: string
  state: 'running' | 'ok' | 'failed' | 'rolled-back'
}
