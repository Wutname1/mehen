// Mirrors crates/mehen-core/src/model.rs (serde camelCase / kebab-case).

export type Ecosystem = 'npm' | 'cargo' | 'nuget' | 'github-actions' | 'go' | 'pypi' | 'pub' | 'packagist' | 'rubygems'
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
  /** Newest bug-fix release on the same minor line. */
  patchLatest?: string | null
  /** Newest published version when this project can't use it; `latest` is then the newest it can. */
  newest: string | null
  blockedReason: string | null
  /** For a vulnerable package: the smallest safe update, on the lowest line no advisory covers. */
  fixTarget?: string | null
  /** Can only move together with others (a framework's parts): the package leading the group, and where this one goes. */
  group?: string | null
  groupTarget?: string | null
  status: Status
  vulns: string[]
  note: string | null
  /** Written as one exact version, so nothing newer comes in until the manifest is edited. */
  pinned?: boolean
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
  /** The lowest Python the project supports. */
  pythonVersion?: string | null
  /** Composer's platform PHP, when the project sets one. */
  phpVersion?: string | null
  dependencies: Dependency[]
}

export interface AffectedRange {
  /** Null for "every version before". */
  introduced: string | null
  fixed: string | null
  lastAffected: string | null
}

export interface FixedIn {
  ecosystem: Ecosystem
  name: string
  versions: string[]
  /** Which versions the advisory covers; missing on advisories saved before ranges were kept. */
  ranges?: AffectedRange[]
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
  /** Hours between background checks; 0 means off. */
  backgroundHours: number
  /** Update steps that may run at once across repositories; 1 runs one at a time, 0 is automatic. */
  updateParallel: number
  /** What automatic means on this computer. */
  updateParallelAuto: number
  /** Notify about new vulnerabilities found by background checks. */
  notify: boolean
  /** Check for new versions of Mehen itself. */
  appUpdateCheck: boolean
  /** Windows runs a quick check at sign-in and daily, with Mehen closed. */
  scheduledCheck: boolean
  /** Whether this computer can run that check (Windows only for now). */
  scheduledCheckSupported: boolean
  /** Send crash and error reports (release builds only). */
  errorReports: boolean
}

/** A repository someone asked Mehen to show, from GitWyrm or the command line. */
export interface OpenRepoRequest {
  path: string
  /** Select its security fixes too, so updating is one click away. */
  fix: boolean
}

/** One line of a Mehen release's notes. `section` is feature, fix, change, docs, or breaking. */
export interface ReleaseNoteItem {
  section: string
  text: string
  tags: string[]
}

export interface ReleaseNotes {
  version: string
  releasedAt: string | null
  items: ReleaseNoteItem[]
}

export interface AppUpdateProgress {
  downloaded: number
  total: number | null
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
  /** Write an exact pin as a range from `to`. */
  loosen?: boolean
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

export type StepKind = 'install' | 'verify' | 'test'

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
  /** Files it touches that already had uncommitted changes; committing them takes forcing. */
  uncommitted?: string[]
  /** The checked-out branch, where a commit would land. */
  branch: string | null
  /** Packages whose entry was an exact pin before this update. */
  pinned?: string[]
  /** Failures a check already had before any update; while it fails with only these, it counts as passing. */
  acceptedFailures?: KnownFailure[]
}

export interface StepResult {
  label: string
  kind: StepKind
  ok: boolean
  output: string
  ms: number
}


export type JobState = 'queued' | 'waiting' | 'running' | 'committing' | 'pushing' | 'done' | 'failed' | 'rolled-back' | 'cancelled'

/** Progress for one repository's share of a batch update. */
export interface BatchEvent {
  job: string
  /** Project ids (manifest paths) in this job. */
  projects: string[]
  state: JobState
  label: string | null
  /** The tool the step uses: npm, cargo, dotnet... */
  lane: string | null
}

/** An AI coding tool Mehen can hand a broken build to, as found on this computer. */
export type AiToolId = 'claude' | 'codex'

export interface AiTool {
  tool: AiToolId
  /** "Claude Code", "Codex". */
  name: string
  version: string | null
  ready: boolean
  /** Why it cannot be used yet: not installed, too old, not answering. */
  problem: string | null
  installUrl: string
}

/** What happens while a tool fixes an update, as it happens. */
export type FixEvent =
  | { kind: 'step'; label: string; state: 'running' | 'ok' | 'failed' }
  | { kind: 'asking'; round: number; of: number }
  | { kind: 'text'; text: string }
  | { kind: 'tool'; title: string; status: string }
  | { kind: 'denied'; title: string; reason: string }

export interface FixOutcome {
  ok: boolean
  /** Times the tool was asked; 0 when the update needed no fix. */
  rounds: number
  /** Files outside the update's own that changed, relative to the repository. */
  changed: string[]
  /** What the tool said it did. */
  summary: string
  error: string | null
  steps: StepResult[]
  rolledBack: boolean
  committed: string | null
  commitError: string | null
  commitSkipped: string | null
  pushed: boolean
  pushError: string | null
}

/** What a check reported when it failed the same way without the update. */
export interface KnownFailure {
  step: string
  label: string
  failures: string[]
}

export interface JobOutcome {
  job: string
  name: string
  repo: string | null
  projects: string[]
  ok: boolean
  rolledBack: boolean
  /** Stopped on request rather than failed; its files were put back. */
  cancelled: boolean
  error: string | null
  steps: StepResult[]
  committed: string | null
  commitError: string | null
  /** Why no commit was attempted although one was asked for. */
  commitSkipped: string | null
  /** The commit reached the remote. */
  pushed?: boolean
  /** The commit stayed local because the push failed. */
  pushError?: string | null
  /** Dependency conflicts the package managers reported along the way. */
  conflicts: Conflict[]
  /** Anything worth knowing about how it went, like a clean install. */
  notes?: string[]
  /** After a failure, whether what failed also fails without the update (the project was already broken). */
  failedBefore?: boolean | null
  /** Checks that failed with no failures the update added; rerun with these accepted, the update goes through. */
  knownFailures?: KnownFailure[]
  /** Projects whose own step fails without the update, with nothing to accept. */
  brokenProjects?: string[]
  /** The tool a step needed that is not installed: npm, dotnet... */
  missingTool?: string | null
}

export interface Conflict {
  /** One sentence: "eslint-plugin-react-hooks 5.2.0 needs eslint ^8.57.0 || ^9.0.0, not 10.0.1". */
  summary: string
  /** The updated package to keep back, when the conflict points at one. */
  keep: { ecosystem: Ecosystem; name: string; line: string; from: string; to: string } | null
  /** The step failed over it; otherwise the install only warned. */
  blocking: boolean
}

export interface BatchResult {
  outcomes: JobOutcome[]
  inventory: Inventory | null
}

export interface CommitOutcome {
  job: string
  name: string
  committed: string | null
  error: string | null
  pushed?: boolean
  pushError?: string | null
}

export interface CheckConfig {
  commands: string[]
  /** Folder to run them in, relative to the project. */
  cwd: string | null
}

/** Build and test commands by scope: a repository path, or `ecosystem:<name>`. */
export type CheckCommands = Record<string, CheckConfig>

/** How far updates may go: any stable version, same major (minor), or same minor (patch). */
export type VersionPolicy = 'any' | 'minor' | 'patch'

/** By scope: a repository path, or `*` for every project. */
export type VersionPolicies = Record<string, VersionPolicy>

/** A package kept on one release line, everywhere or in one project. */
export interface Hold {
  id: number
  ecosystem: Ecosystem
  name: string
  /** A project folder, or `*` for every project. */
  scope: string
  /** `5` for 5.x, `0.13` for 0.13.x. */
  line: string
}

/** What a published version asks of the project using it. */
export type Requirement =
  | { kind: 'frameworks'; frameworks: string[] }
  | { kind: 'rust'; version: string }
  | { kind: 'node'; range: string }
  | { kind: 'peers'; peers: [string, string][] }
  | { kind: 'crates'; deps: [string, string][] }
  | { kind: 'python' | 'dart' | 'php' | 'ruby'; range: string }

/** One published version, as one project sees it. */
export interface VersionView {
  version: string
  prerelease: boolean
  /** Why the project cannot use it; null when it can. */
  blocked: string | null
  requirements: Requirement[]
  /** How many other packages have to move with it for it to fit. */
  together: number
}

/** A package moving as part of a chosen update. */
export interface Move {
  name: string
  from: string
  to: string
}
