import { AlertTriangle, Ban, Box, Check, ChevronRight, Copy, FileText, GitBranch, GitCommitHorizontal, Loader2, Lock, Pin, Play, RefreshCw, RotateCcw, Send, ShieldCheck, Terminal, X } from 'lucide-react'
import { useEffect, useMemo, useRef, useState, type ReactNode } from 'react'
import * as api from '../api'
import { applyPins, bumpOf, folderName, halves, leftOut, relativePath, runLabel, stagesOf, targetsOf, unitsOf, withoutCulprits, type Fallback, type UpdateUnit } from '../derive'
import type { BatchEvent, Change, CommitOutcome, Conflict, Inventory, JobOutcome, JobState, Project, UpdatePlan } from '../types'
import { GitWyrmMark, cx } from './bits'
import { Button, Dialog } from './Dialog'
import { FeedbackDialog } from './FeedbackDialog'
import { Checkbox } from './Queue'
import { Segmented } from './controls'
import { COMMIT_MODES, PIN_MODES, type CommitMode, type PinMode } from '../prefs'
import { sendReport, type Report } from '../lib/telemetry'
import { batchErrorReport, planFailureReport, updateFailureReport } from '../lib/updateReport'
import { RepoAvatar } from './RepoAvatar'

export interface UpdateTarget {
  project: Project
  changes: Change[]
}

type Stage = 'planning' | 'preview' | 'confirm' | 'running' | 'done' | 'commit'

/** How many tries Mehen makes at most, across every repository, while it works out what breaks. */
const MAX_TRIES = 16

/** What happened to one repository over every try. */
interface Tally {
  ok: JobOutcome[]
  /** Plans of the tries that went in, and of the last one that did not. */
  plans: UpdatePlan[]
  failed: { outcome: JobOutcome; plans: UpdatePlan[]; targets: UpdateTarget[] } | null
  notes: string[]
}

/** One outcome for a repository that was tried in several parts. */
function combine(t: Tally): JobOutcome | null {
  if (!t.ok.length) return t.failed ? { ...t.failed.outcome, notes: [...(t.failed.outcome.notes ?? []), ...t.notes] } : null
  const last = t.ok[t.ok.length - 1]
  const committed = t.ok.map((o) => o.committed).filter((h): h is string => !!h)
  return {
    ...last,
    projects: [...new Set(t.ok.flatMap((o) => o.projects))],
    steps: t.ok.flatMap((o) => o.steps),
    committed: committed.length ? committed.join(', ') : null,
    commitError: t.ok.map((o) => o.commitError).find(Boolean) ?? null,
    commitSkipped: t.ok.map((o) => o.commitSkipped).find(Boolean) ?? null,
    pushed: committed.length > 0 && t.ok.every((o) => !o.committed || o.pushed),
    pushError: t.ok.map((o) => o.pushError).find(Boolean) ?? null,
    conflicts: t.ok.flatMap((o) => o.conflicts),
    notes: [...t.ok.flatMap((o) => o.notes ?? []), ...t.notes],
  }
}

interface Job {
  key: string
  name: string
  plans: UpdatePlan[]
  branch: string | null
  /** Why this repository can't be committed. */
  blocked: string | null
  /** Files it touches that already had uncommitted changes. */
  dirty: string[]
}

interface Live {
  state: JobState
  label: string | null
}

/** Where to get a tool an update needed but could not find. */
function installLink(tool: string): { name: string; url: string } {
  switch (tool.toLowerCase()) {
    case 'npm':
    case 'npx':
    case 'node':
      return { name: 'Node.js', url: 'https://nodejs.org/en/download' }
    case 'pnpm':
      return { name: 'pnpm', url: 'https://pnpm.io/installation' }
    case 'yarn':
      return { name: 'Yarn', url: 'https://yarnpkg.com/getting-started/install' }
    case 'bun':
      return { name: 'Bun', url: 'https://bun.sh' }
    case 'dotnet':
      return { name: 'the .NET SDK', url: 'https://dotnet.microsoft.com/download' }
    case 'cargo':
      return { name: 'Rust', url: 'https://rustup.rs' }
    default:
      return { name: tool, url: `https://www.google.com/search?q=${encodeURIComponent(`install ${tool}`)}` }
  }
}

const jobKey = (p: UpdatePlan) => p.repo ?? p.projectId.replace(/[\\/][^\\/]*$/, '')

function jobsOf(plans: UpdatePlan[], nameOf: (key: string) => string): Job[] {
  const map = new Map<string, Job>()
  for (const plan of plans) {
    const key = jobKey(plan)
    const job = map.get(key.toLowerCase()) ?? { key, name: nameOf(key), plans: [], branch: plan.branch, blocked: null, dirty: [] }
    job.plans.push(plan)
    map.set(key.toLowerCase(), job)
  }
  for (const job of map.values()) {
    const reasons = [...new Set(job.plans.map((p) => p.commitBlocked).filter((r): r is string => !!r))]
    job.blocked = reasons.length ? reasons.join('; ') : null
    job.dirty = [...new Set(job.plans.flatMap((p) => p.uncommitted ?? []))]
  }
  return [...map.values()]
}

/** Every package once, as it will read in the commit. */
function changesOf(job: Job) {
  const seen = new Map<string, { name: string; from: string; to: string }>()
  for (const c of job.plans.flatMap((p) => p.changes)) if (!seen.has(c.name)) seen.set(c.name, { name: c.name, from: c.from, to: c.to })
  return [...seen.values()].sort((a, b) => a.name.localeCompare(b.name))
}

export const commitSubject = (n: number) => `Updated ${n} ${n === 1 ? 'Dependency' : 'Dependencies'}`

function stepsOf(job: Job, build: boolean, test: boolean) {
  const seen = new Set<string>()
  const pick = (kind: string) =>
    job.plans
      .flatMap((p) => p.steps)
      .filter((s) => s.kind === kind)
      .filter((s) => {
        const k = `${s.program}|${s.args.join(' ')}|${s.cwd}`.toLowerCase()
        return !seen.has(k) && !!seen.add(k)
      })
  const install = pick('install')
  return { install, checks: [...(build ? pick('verify') : []), ...(test ? pick('test') : [])] }
}

/** Labels once each, with a count when several folders run the same thing. */
const labels = (steps: { label: string }[]) => {
  const counts = new Map<string, number>()
  for (const s of steps) counts.set(s.label, (counts.get(s.label) ?? 0) + 1)
  return [...counts].map(([label, n]) => (n > 1 ? `${label} ×${n}` : label)).join(', ')
}

/** Logos by lowercased project folder, filled in by the app. */
let iconsByKey: Record<string, string> = {}

function Avatar({ name, repo }: { name: string; repo?: string }) {
  return <RepoAvatar name={name} icon={repo ? iconsByKey[repo.toLowerCase()] : undefined} size={24} />
}

function Diff({ diff }: { diff: string }) {
  return (
    <pre className="mb-2 overflow-x-auto rounded-[3px] border border-line bg-sunken py-2 font-mono text-[12px] leading-[1.55]">
      {diff
        .replace(/\n$/, '')
        .split('\n')
        .map((line, i) => (
          <div
            key={i}
            className={cx(
              'px-3',
              line.startsWith('+++') || line.startsWith('---')
                ? 'text-muted'
                : line.startsWith('+')
                  ? 'bg-[color-mix(in_oklab,var(--ok)_14%,transparent)] text-ok'
                  : line.startsWith('-')
                    ? 'bg-[color-mix(in_oklab,var(--risk-security)_12%,transparent)] text-risk-security'
                    : line.startsWith('@@')
                      ? 'text-risk-minor'
                      : 'text-faint',
            )}
          >
            {line || ' '}
          </div>
        ))}
    </pre>
  )
}

function StateChip({ tone, children, spin }: { tone: 'wait' | 'run' | 'ok' | 'fail'; children: ReactNode; spin?: boolean }) {
  return (
    <span
      className={cx(
        'inline-flex shrink-0 items-center gap-1.5 font-mono text-[11px] font-semibold tracking-[0.04em] whitespace-nowrap uppercase',
        tone === 'wait' && 'text-muted',
        tone === 'run' && 'text-state',
        tone === 'ok' && 'text-ok',
        tone === 'fail' && 'text-risk-security',
      )}
    >
      {spin && <Loader2 size={14} className="animate-spin" />}
      {children}
    </span>
  )
}

export function UpdateFlow({
  targets,
  start,
  roots,
  build,
  test,
  commitMode,
  pinMode,
  stopOnFailure,
  onOptions,
  nameOf,
  icons,
  onKeep,
  onReviewInGitWyrm,
  onClose,
}: {
  targets: UpdateTarget[]
  start: 'confirm' | 'preview'
  roots: string[]
  build: boolean
  test: boolean
  commitMode: CommitMode
  pinMode: PinMode
  stopOnFailure: boolean
  onOptions: (patch: { build?: boolean; test?: boolean; commit?: CommitMode; pins?: PinMode }) => void
  nameOf: (key: string) => string
  icons: Record<string, string>
  /** Keeps a package on its line in `scope` (a repository) from now on. */
  onKeep: (keep: NonNullable<Conflict['keep']>, scope: string) => Promise<void>
  /** Opens a repository in GitWyrm; absent when GitWyrm is not installed. */
  onReviewInGitWyrm?: (repo: string) => void
  onClose: (refreshed: Inventory | null) => void
}) {
  const [current, setCurrent] = useState(targets)
  const [kept, setKept] = useState<Set<string>>(new Set())
  iconsByKey = icons
  const [stage, setStage] = useState<Stage>('planning')
  const [plans, setPlans] = useState<UpdatePlan[]>([])
  const [failed, setFailed] = useState<{ project: Project; error: string }[]>([])
  const [live, setLive] = useState<Record<string, Live>>({})
  const [outcomes, setOutcomes] = useState<JobOutcome[]>([])
  const [commits, setCommits] = useState<CommitOutcome[]>([])
  const [refreshed, setRefreshed] = useState<Inventory | null>(null)
  const checks = build || test
  const commit = commitMode !== 'off'
  const push = commitMode === 'push'
  const [ran, setRan] = useState({ checks, build, test, commit, push })
  /** A failure report being written up in the full feedback form. */
  const [note, setNote] = useState<(Report & { title: string }) | null>(null)
  const [error, setError] = useState<string | null>(null)
  const [copied, setCopied] = useState(false)
  /** Commit even where files already had changes, which then go in too. Asked per update, never remembered. */
  const [forceCommit, setForceCommit] = useState(false)
  /** Jobs asked to stop (by key, lowercased); `*` when all were. */
  const [stopping, setStopping] = useState<Set<string>>(new Set())
  /** The same, read between runs: a stopped job is not run again. */
  const stopAsked = useRef<Set<string>>(new Set())
  /** The updates as last run, after any packages were held back. */
  const [tried, setTried] = useState(targets)
  /** Packages held back after a failure, by job key (lowercased). */
  const [fallbacks, setFallbacks] = useState<Record<string, Fallback[]>>({})
  /** The step to come back to when the plans are worked out again (pins changed on it). */
  const resume = useRef<Stage | null>(null)

  useEffect(() => {
    let cancelled = false
    Promise.all(
      applyPins(current, pinMode).map((t) =>
        api.planUpdate(t.project.id, t.changes).then(
          (plan) => (plan.edits.length || plan.steps.length ? { plan } : { error: 'Nothing to change', project: t.project }),
          (e) => ({ error: String(e), project: t.project }),
        ),
      ),
    ).then((results) => {
      if (cancelled) return
      setPlans(results.flatMap((r) => ('plan' in r && r.plan ? [r.plan] : [])))
      setFailed(results.flatMap((r) => ('error' in r && r.error ? [{ project: r.project, error: r.error }] : [])))
      setTried(current)
      setFallbacks({})
      setStage(resume.current ?? start)
      resume.current = null
    })
    return () => {
      cancelled = true
    }
  }, [current, start, pinMode])

  useEffect(() => {
    const unlisten = api.onBatchEvent((e: BatchEvent) => {
      setLive((prev) => {
        const next = { ...prev }
        for (const id of e.projects) next[id.toLowerCase()] = { state: e.state, label: e.label }
        return next
      })
    })
    return () => {
      unlisten.then((fn) => fn())
    }
  }, [])

  const jobs = useMemo(() => jobsOf(plans, nameOf), [plans, nameOf])
  const busy = stage === 'planning' || stage === 'running'
  const close = () => !busy && onClose(refreshed)

  const cancel = (job: Job | null) => {
    const key = job ? job.key.toLowerCase() : '*'
    stopAsked.current.add(key)
    setStopping((prev) => new Set([...prev, key]))
    api.cancelUpdate(job ? job.key : null).catch((e) => setError(String(e)))
  }

  const run = async () => {
    setRan({ checks, build, test, commit, push })
    setStopping(new Set())
    stopAsked.current = new Set()
    setFallbacks({})
    setStage('running')
    setError(null)
    try {
      const keyOf = (projectId: string) => {
        const plan = plans.find((p) => p.projectId.toLowerCase() === projectId.toLowerCase())
        return (plan ? jobKey(plan) : projectId).toLowerCase()
      }
      // Files someone else had already changed. After a part goes in uncommitted,
      // its own edits show as changed too, and must not block the next part's commit.
      const dirtyBefore = new Map<string, Set<string>>()
      for (const p of plans) dirtyBefore.set(keyOf(p.projectId), new Set([...(dirtyBefore.get(keyOf(p.projectId)) ?? []), ...(p.uncommitted ?? [])]))
      // Each repository's updates in the order they are tried. With checks to
      // run, breaking moves go one at a time after the rest; a part that fails
      // without saying why is split in half until the update that breaks is found.
      const byJob = new Map<string, UpdateTarget[]>()
      for (const t of applyPins(current, pinMode)) byJob.set(keyOf(t.project.id), [...(byJob.get(keyOf(t.project.id)) ?? []), t])
      const queue = new Map<string, UpdateUnit[][]>()
      for (const [key, targets] of byJob) {
        const units = unitsOf(targets)
        queue.set(key, checks ? stagesOf(units) : [units])
      }
      const tally = new Map<string, Tally>()
      const tallyOf = (key: string) => tally.get(key) ?? (tally.set(key, { ok: [], plans: [], failed: null, notes: [] }), tally.get(key)!)
      const left: Record<string, Fallback[]> = {}
      let inventory: Inventory | null = null

      for (let tries = 0; queue.size > 0 && tries < MAX_TRIES && !stopAsked.current.has('*'); tries++) {
        const round: { key: string; units: UpdateUnit[]; plans: UpdatePlan[] }[] = []
        for (const [key, stages] of [...queue]) {
          const units = stages.shift()
          if (!stages.length) queue.delete(key)
          if (!units || stopAsked.current.has(key)) {
            queue.delete(key)
            continue
          }
          const planned = await Promise.all(targetsOf(units).map((t) => api.planUpdate(t.project.id, t.changes).then((p) => p, (e) => `${t.project.name}: ${e}`)))
          const unplanned = planned.filter((p): p is string => typeof p === 'string')
          if (unplanned.length) {
            tallyOf(key).notes.push(...unplanned.map((e) => `Not tried: ${e}`))
            continue
          }
          const dirty = dirtyBefore.get(key) ?? new Set()
          round.push({ key, units, plans: (planned as UpdatePlan[]).map((p) => ({ ...p, uncommitted: (p.uncommitted ?? []).filter((f) => dirty.has(f)) })) })
        }
        if (!round.length) break
        const result = await api.applyBatch(
          round.flatMap((r) => r.plans),
          { build, test },
          commit,
          stopOnFailure,
          forceCommit,
          push,
        )
        inventory = result.inventory ?? inventory
        for (const o of result.outcomes) {
          const key = keyOf(o.projects[0] ?? o.job)
          const tried = round.find((r) => r.key === key)
          if (!tried) continue
          const t = tallyOf(key)
          if (o.ok) {
            t.ok.push(o)
            t.plans.push(...tried.plans)
            continue
          }
          t.failed = { outcome: o, plans: tried.plans, targets: targetsOf(tried.units) }
          // Stopped, missing a tool, already broken before the update, or its
          // files could not be put back: trying more parts would not help.
          if (o.cancelled || o.missingTool || o.failedBefore === true || !o.rolledBack) {
            const skipped = [...new Set([tried.units, ...(queue.get(key) ?? [])].flat().flatMap((u) => u.parts.map((p) => p.change.name)))]
            if (!o.cancelled && t.ok.length) {
              const why = o.failedBefore === true ? 'The project fails its checks even without these updates' : o.missingTool ? `\`${o.missingTool}\` is not installed` : (o.error ?? 'A step failed')
              t.notes.push(`${why}, so Mehen stopped. Not updated: ${skipped.join(', ')}.`)
            }
            queue.delete(key)
            continue
          }
          const culprits = o.conflicts.filter((x) => x.blocking && x.keep).map((x) => x.keep!)
          const next = culprits.length ? withoutCulprits(targetsOf(tried.units), culprits) : null
          const stages = queue.get(key) ?? []
          if (next) {
            left[key] = [...(left[key] ?? []), ...next.fallbacks]
            stages.unshift(unitsOf(next.targets))
          } else if (tried.units.length > 1) {
            stages.unshift(...halves(tried.units))
          } else {
            left[key] = [...(left[key] ?? []), ...leftOut(tried.units[0])]
          }
          if (stages.length) queue.set(key, stages)
        }
        setFallbacks({ ...left })
      }
      for (const [key, stages] of queue) {
        const names = [...new Set(stages.flat().flatMap((u) => u.parts.map((p) => p.change.name)))]
        if (names.length) tallyOf(key).notes.push(`Mehen stopped trying after ${MAX_TRIES} runs. Not tried yet: ${names.join(', ')}.`)
      }

      const all = [...tally.values()].map(combine).filter((o): o is JobOutcome => !!o)
      const shown = [...tally.values()].flatMap((t) => (t.ok.length ? t.plans : (t.failed?.plans ?? [])))
      setPlans(shown.length ? shown : plans)
      setTried([...tally.values()].flatMap((t) => t.failed?.targets ?? []))
      setOutcomes(all)
      if (inventory) setRefreshed(inventory)
    } catch (e) {
      setError(String(e))
    }
    setStage('done')
  }

  /** Plans the repository again without `names`, keeping the rest of its updates. */
  const retry = (job: Job, names: string[]) => {
    const ids = new Set(job.plans.map((p) => p.projectId.toLowerCase()))
    const next = tried
      .filter((t) => ids.has(t.project.id.toLowerCase()))
      .map((t) => ({ ...t, changes: t.changes.filter((c) => !names.includes(c.name)) }))
      .filter((t) => t.changes.length > 0)
    setOutcomes([])
    setCommits([])
    setLive({})
    setStage('planning')
    setCurrent(next)
  }

  const keep = async (k: NonNullable<Conflict['keep']>, scope: string) => {
    try {
      await onKeep(k, scope)
      setKept((prev) => new Set([...prev, `${scope.toLowerCase()}|${k.name}`]))
    } catch (e) {
      setError(String(e))
    }
  }

  const outcomeOf = (job: Job) => outcomes.find((o) => o.projects.some((id) => job.plans.some((p) => p.projectId === id)))
  const passed = jobs.filter((j) => outcomeOf(j)?.ok)
  const broke = jobs.filter((j) => outcomeOf(j) && !outcomeOf(j)!.ok && !outcomeOf(j)!.cancelled)
  const stopped = jobs.filter((j) => outcomeOf(j)?.cancelled)
  const commitOf = (job: Job) => commits.find((c) => c.job.toLowerCase() === job.key.toLowerCase())
  /** Whether Mehen will commit this repository, given the force choice. */
  const willCommit = (j: Job) => !j.blocked && (forceCommit || !j.dirty.length)
  const dirtyJobs = jobs.filter((j) => !j.blocked && j.dirty.length > 0)
  const committable = passed.filter((j) => willCommit(j) && !outcomeOf(j)?.committed && !commitOf(j)?.committed)

  const commitNow = async () => {
    setStage('running')
    try {
      setCommits(await api.commitUpdate(committable.flatMap((j) => j.plans), forceCommit, push))
    } catch (e) {
      setError(String(e))
    }
    setStage('done')
  }

  const commands = () =>
    jobs
      .map((job) => {
        const { install, checks: steps } = stepsOf(job, build, test)
        const lines = [`# ${job.name}${job.branch ? ` (${job.branch})` : ''}`, `cd ${job.key}`, ...[...install, ...steps].map((s) => [s.program, ...s.args].join(' '))]
        if (commit && willCommit(job)) lines.push(`git commit -m "${commitSubject(changesOf(job).length)}" -- <changed files>`)
        if (push && willCommit(job)) lines.push('git push')
        return lines.join('\n')
      })
      .join('\n\n')

  // Nothing to install, build or test: only workflow files change.
  const filesOnly = plans.length > 0 && plans.every((p) => p.steps.length === 0)
  const label = filesOnly ? 'Update workflow files' : runLabel(build, test)
  const total = jobs.length

  const options = (
    <div className="mr-auto flex flex-wrap gap-x-[18px] gap-y-1.5" role="group" aria-label="When updating">
      {!filesOnly && (
        <>
          <label className="inline-flex cursor-pointer items-center gap-2 text-[12.5px]">
            <Checkbox checked={build} onChange={() => onOptions({ build: !build })} label="Build" />
            Build
          </label>
          <label className="inline-flex cursor-pointer items-center gap-2 text-[12.5px]">
            <Checkbox checked={test} onChange={() => onOptions({ test: !test })} label="Run tests" />
            Run tests
          </label>
        </>
      )}
      <Segmented value={commitMode} options={COMMIT_MODES} onChange={(next) => onOptions({ commit: next })} label="After updating" />
      {commit && dirtyJobs.length > 0 && (
        <label
          className="inline-flex cursor-pointer items-center gap-2 text-[12.5px]"
          title={`${dirtyJobs.map((j) => `${j.name}: ${j.dirty.join(', ')}`).join('\n')}\n\nThose earlier changes go into the same commit as the update.`}
        >
          <Checkbox checked={forceCommit} onChange={() => setForceCommit(!forceCommit)} label="Commit anyway where files already had changes" />
          Commit anyway ({dirtyJobs.length} with other changes)
        </label>
      )}
    </div>
  )

  if (note) return <FeedbackDialog preset={note} onClose={() => setNote(null)} />

  if (stage === 'planning') {
    return (
      <Dialog title="Working out the changes" size="wide" busy onClose={close} icon={<Loader2 size={22} className="animate-spin" />}>
        <p className="py-8 text-center text-muted">
          Reading {targets.length} manifest{targets.length === 1 ? '' : 's'} to see exactly what will change…
        </p>
      </Dialog>
    )
  }

  const pinned = [...new Set(plans.flatMap((p) => p.pinned ?? []))].sort()
  const vulnerablePins = pinned.filter((name) => current.some((t) => t.project.dependencies.some((d) => d.name === name && d.pinned && d.vulns.length > 0)))
  const pins =
    pinned.length > 0 ? (
      <div className="mb-2.5 flex flex-wrap items-center gap-x-3 gap-y-2 rounded-[3px] border border-line bg-paper px-3 py-2.5 text-[12.5px]">
        <Lock size={15} className="shrink-0 text-muted" />
        <span className="min-w-[220px] flex-1" title={pinned.join('\n')}>
          {pinned.length === 1 ? `${pinned[0]} is` : `${pinned.length} packages are`} pinned to one exact version, so newer releases and their fixes only come in when the file is edited.
          {vulnerablePins.length > 0 && <span className="text-risk-security"> {vulnerablePins.length === pinned.length && pinned.length === 1 ? 'It has' : `${vulnerablePins.length} ${vulnerablePins.length === 1 ? 'has' : 'have'}`} known vulnerabilities.</span>}
        </span>
        <Segmented
          value={pinMode}
          options={PIN_MODES}
          onChange={(next) => {
            resume.current = stage
            onOptions({ pins: next })
          }}
          label="Pinned versions"
        />
      </div>
    ) : null

  if (stage === 'preview') {
    return (
      <Dialog
        title={filesOnly ? 'File changes' : 'Files and commands'}
        description={filesOnly ? 'Exactly what Mehen will change in each project. Nothing runs, and reviewing changes nothing on disk.' : 'Exactly what Mehen will change and run in each project. Reviewing changes nothing on disk.'}
        icon={filesOnly ? <FileText size={22} /> : <Terminal size={22} />}
        size="wide"
        onClose={close}
        footer={
          <>
            {!filesOnly && (
              <Button
                variant="ghost"
                className="mr-auto"
                onClick={async () => {
                  await navigator.clipboard?.writeText(commands())
                  setCopied(true)
                }}
              >
                {copied ? <Check size={15} /> : <Copy size={15} />}
                {copied ? 'Copied' : 'Copy commands'}
              </Button>
            )}
            <Button onClick={close} className={cx(filesOnly && 'ml-auto')}>
              Close
            </Button>
            <Button variant="primary" onClick={() => setStage('confirm')} disabled={!jobs.length}>
              <Play size={15} />
              Continue to update
            </Button>
          </>
        }
      >
        {pins}
        {jobs.map((job) => {
          const { install, checks: steps } = stepsOf(job, build, test)
          return (
            <section key={job.key} className="mb-5">
              <h3 className="mb-2 flex items-center gap-2 text-[13px] font-semibold">
                <Avatar name={job.name} repo={job.key} />
                {job.name}
                {job.branch && (
                  <span className="inline-flex items-center gap-1 font-mono text-[12px] font-normal text-muted">
                    <GitBranch size={14} />
                    {job.branch}
                  </span>
                )}
              </h3>
              {job.plans.flatMap((p) => p.edits).map((e) => (
                <Diff key={e.path} diff={e.diff} />
              ))}
              {install.length + steps.length > 0 && (
                <pre className="overflow-x-auto rounded-[3px] bg-bar px-3.5 py-3 font-mono text-[12.5px] leading-[1.65] text-rail-ink">
                  {[...install, ...steps].map((s) => [s.program, ...s.args].join(' ')).join('\n')}
                </pre>
              )}
            </section>
          )
        })}
        {failed.length > 0 && <Unplannable failed={failed} roots={roots} onNote={setNote} />}
      </Dialog>
    )
  }

  if (stage === 'confirm') {
    const packages = new Set(plans.flatMap((p) => p.changes.map((c) => c.name))).size
    return (
      <Dialog
        title={`Update ${total} project${total === 1 ? '' : 's'}`}
        description={`${packages} package${packages === 1 ? '' : 's'} will change. Mehen edits the files below${filesOnly ? '' : ` and refreshes lockfiles${checks ? ", then runs each project's checks" : ''}`}${push ? ', then commits and pushes' : commit ? ', then commits' : ''}.`}
        icon={<Play size={22} />}
        size="wide"
        onClose={close}
        footer={
          <>
            {options}
            <Button onClick={close}>Cancel</Button>
            <Button variant="primary" onClick={run} disabled={!jobs.length}>
              <Play size={15} />
              {label}
            </Button>
          </>
        }
      >
        {failed.length > 0 && <Unplannable failed={failed} roots={roots} onNote={setNote} />}
        {pins}
        <div className="grid gap-2.5">
          {jobs.map((job) => {
            const changes = changesOf(job)
            const edited = new Set(job.plans.flatMap((p) => p.edits.map((e) => e.path.toLowerCase())))
            const lockfiles = [...new Set(job.plans.flatMap((p) => p.snapshots).filter((f) => !edited.has(f.toLowerCase())))].map((f) => relativePath([job.key], f))
            const { install, checks: steps } = stepsOf(job, build, test)
            const warnings = [...new Set(job.plans.flatMap((p) => p.warnings))]
            return (
              <section key={job.key} className="rounded-[3px] border border-line bg-paper">
                <header className="flex items-center gap-2.5 border-b border-line px-3 py-2.5">
                  <Avatar name={job.name} repo={job.key} />
                  <span className="flex min-w-0 flex-1 flex-col">
                    <b className="text-[13px]">{job.name}</b>
                    <small className="truncate font-mono text-[12px] text-muted">{job.key}</small>
                  </span>
                  {job.branch && (
                    <span className="inline-flex items-center gap-1 font-mono text-[12px] text-muted">
                      <GitBranch size={14} />
                      {job.branch}
                    </span>
                  )}
                </header>
                <PlanFiles job={job} />
                <ul className="m-0 grid list-none gap-1 px-3 pt-1 pb-2.5 text-[12.5px]">
                  {lockfiles.length > 0 && (
                    <li className="flex items-center gap-2 text-muted">
                      <FileText size={14} className="shrink-0" />
                      <span className="truncate" title={lockfiles.join('\n')}>
                        Also refreshes {lockfiles.join(', ')}
                      </span>
                    </li>
                  )}
                  <li className="flex items-center gap-2 text-muted">
                    <Terminal size={14} className="shrink-0" />
                    {!install.length && !job.plans.some((p) => p.steps.length)
                      ? 'Only files change; nothing to install or run.'
                      : checks
                      ? steps.length
                        ? `Checks: ${labels(steps)}`
                        : `No checks for this project${install.length ? `. Install: ${labels(install)}` : ''}`
                      : install.length
                        ? `Install: ${labels(install)}. Checks skipped.`
                        : 'Files are edited only; nothing to run.'}
                  </li>
                  {commit && willCommit(job) && (
                    <li className="flex items-center gap-2 text-muted">
                      <GitCommitHorizontal size={14} className="shrink-0" />
                      Commit on {job.branch ?? 'the current branch'}: “{commitSubject(changes.length)}”{push && ', then push'}
                    </li>
                  )}
                </ul>
                {((job.blocked && commit) || (!job.blocked && job.dirty.length > 0)) && (
                  <div className="mx-3 mb-2.5 flex gap-2 rounded-[3px] bg-[color-mix(in_oklab,var(--risk-review)_10%,transparent)] px-2.5 py-2 text-[12.5px] text-risk-review">
                    <AlertTriangle size={15} className="mt-px shrink-0" />
                    <span>
                      {job.blocked
                        ? `Won't be committed: ${job.blocked}.`
                        : (() => {
                            const files = job.dirty.join(', ')
                            const have = job.dirty.length === 1 ? 'has' : 'have'
                            if (!commit) return `${files} already ${have} changes you haven't committed; they will sit alongside this update.`
                            if (forceCommit) return `${files} already ${have} changes you haven't committed. They go into this commit too.`
                            return `Won't be committed: ${files} already ${have} changes you haven't committed. Tick "Commit anyway" to include them, or commit or stash them first.`
                          })()}
                    </span>
                  </div>
                )}
                {warnings.map((w) => (
                  <p key={w} className="mx-3 mb-2 text-[12px] text-muted">
                    {w}
                  </p>
                ))}
              </section>
            )
          })}
        </div>
        <p className="mt-3 border-l-2 border-line-strong pl-3 text-[12.5px] leading-relaxed text-muted">
          {filesOnly ? 'Workflow files have nothing to install or test here; your CI runs them next time it starts.' : checks ? `Updates that stay on their release line go in together, then each bigger one on its own. If a check fails, Mehen puts the files back, works out which update caused it, and keeps the rest${stopOnFailure ? '' : ' (every check still runs first, so you see each failure)'}.` : 'Nothing is built or tested. Run your tests or let CI check before merging.'}{' '}
          {push ? 'Each commit is pushed once it is made.' : commit ? 'Commits stay local; nothing is pushed.' : 'Nothing is committed until you choose to.'}
          {!filesOnly && ' Different projects update side by side; projects that need the same tool take turns.'}
        </p>
      </Dialog>
    )
  }

  if (stage === 'running') {
    return (
      <Dialog
        title={outcomes.length ? 'Committing' : 'Updating'}
        description={
          outcomes.length
            ? `Committing${push ? ' and pushing' : ''} the files Mehen changed.`
            : filesOnly
              ? `Each project's workflow files are updated${ran.push ? ', committed and pushed' : ran.commit ? ' and committed' : ''}.`
              : `Each project is updated${ran.checks ? ' and checked' : ''}${ran.push ? ', then committed and pushed' : ran.commit ? ' and committed' : ''}. Projects that need the same tool take turns.`
        }
        icon={<RefreshCw size={22} className="animate-spin" />}
        size="wide"
        busy
        onClose={close}
        footer={
          <>
            <span className="mr-auto text-[12.5px] text-muted">{stopping.has('*') ? 'Stopping and putting files back…' : 'Working. This window closes when you choose, once everything is done.'}</span>
            {!outcomes.length && (
              <Button onClick={() => cancel(null)} disabled={stopping.has('*')} title="Stop every project that has not finished and put its files back">
                <Ban size={15} />
                {stopping.has('*') ? 'Cancelling…' : 'Cancel all'}
              </Button>
            )}
          </>
        }
      >
        {jobs.map((job) => {
          const states = job.plans.map((p) => live[p.projectId.toLowerCase()]).filter(Boolean)
          const now = states.find((s) => s.state === 'running' || s.state === 'committing' || s.state === 'pushing') ?? states.find((s) => s.state === 'waiting') ?? states[0]
          const state = now?.state ?? 'queued'
          const asked = stopping.has('*') || stopping.has(job.key.toLowerCase())
          const stoppable = !outcomes.length && (state === 'queued' || state === 'waiting' || state === 'running')
          return (
            <div key={job.key} className="grid grid-cols-[24px_1fr_auto_28px] items-center gap-2.5 border-b border-line py-2.5">
              <Avatar name={job.name} repo={job.key} />
              <span className="flex min-w-0 flex-col">
                <b className="text-[13px]">{job.name}</b>
                <small className="truncate text-[12px] text-muted">{state === 'queued' ? `${changesOf(job).length} package${changesOf(job).length === 1 ? '' : 's'}` : state === 'cancelled' ? 'Files put back' : (now?.label ?? '')}</small>
              </span>
              {state === 'done' ? (
                <StateChip tone="ok">
                  <Check size={14} />
                  Done
                </StateChip>
              ) : state === 'cancelled' ? (
                <StateChip tone="wait">
                  <Ban size={14} />
                  Cancelled
                </StateChip>
              ) : state === 'failed' || state === 'rolled-back' ? (
                <StateChip tone="fail">
                  <AlertTriangle size={14} />
                  Failed
                </StateChip>
              ) : asked ? (
                <StateChip tone="wait" spin>
                  Cancelling
                </StateChip>
              ) : state === 'running' || state === 'committing' || state === 'pushing' ? (
                <StateChip tone="run" spin>
                  {state === 'pushing' ? 'Pushing' : state === 'committing' ? 'Committing' : 'Running'}
                </StateChip>
              ) : (
                <StateChip tone="wait">Waiting</StateChip>
              )}
              {stoppable && !asked ? (
                <button
                  type="button"
                  onClick={() => cancel(job)}
                  aria-label={`Cancel ${job.name}`}
                  title={`Stop ${job.name} and put its files back`}
                  className="grid size-7 place-items-center rounded-[3px] text-muted hover:bg-sunken hover:text-ink"
                >
                  <X size={15} />
                </button>
              ) : (
                <span />
              )}
            </div>
          )
        })}
      </Dialog>
    )
  }

  if (stage === 'commit') {
    return (
      <Dialog
        title={`Commit ${committable.length} project${committable.length === 1 ? '' : 's'}`}
        description={`Only the files Mehen changed are committed, on each project's current branch. ${push ? 'Each branch is then pushed.' : 'Nothing is pushed.'}`}
        icon={<GitCommitHorizontal size={22} />}
        onClose={() => setStage('done')}
        footer={
          <>
            <Button onClick={() => setStage('done')}>Not now</Button>
            <Button variant="primary" onClick={commitNow}>
              <GitCommitHorizontal size={15} />
              {push ? 'Commit & push' : 'Commit'}
            </Button>
          </>
        }
      >
        {committable.map((job) => {
          const changes = changesOf(job)
          return (
            <div key={job.key} className="grid grid-cols-[24px_1fr_auto] items-start gap-2.5 border-b border-line py-2.5">
              <Avatar name={job.name} repo={job.key} />
              <span className="flex min-w-0 flex-col">
                <b className="text-[13px]">{job.name}</b>
                <small className="text-[12px] text-ink">“{commitSubject(changes.length)}”</small>
                <small className="text-[12px] text-muted">{changes.map((c) => `${c.name} ${c.from} to ${c.to}`).join(', ')}</small>
              </span>
              {job.branch && (
                <span className="inline-flex items-center gap-1 font-mono text-[12px] text-muted">
                  <GitBranch size={14} />
                  {job.branch}
                </span>
              )}
            </div>
          )
        })}
      </Dialog>
    )
  }

  const committedJobs = passed.filter((j) => outcomeOf(j)?.committed || commitOf(j)?.committed)
  const anyCommitted = committedJobs.length > 0
  const pushedOf = (j: Job) => !!(outcomeOf(j)?.pushed || commitOf(j)?.pushed)
  const allPushed = anyCommitted && committedJobs.every(pushedOf)
  const somePushed = committedJobs.some(pushedOf)
  const heldBack = Object.values(fallbacks).flat().filter((f) => !f.to).length
  return (
    <Dialog
      title={error ? 'Update could not run' : broke.length ? 'Update finished with a problem' : stopped.length && !passed.length ? 'Update cancelled' : 'Update finished'}
      description={
        allPushed
          ? 'Each committed project was pushed to its remote.'
          : somePushed
            ? 'Some commits were pushed; the rest stayed local.'
            : anyCommitted
              ? `Each committed project has a new local commit. ${ran.push ? 'The push did not go through.' : 'Nothing was pushed.'}`
              : passed.length ? 'Changed files are ready to commit. Nothing has been committed yet.' : 'Nothing on disk was changed.'
      }
      icon={broke.length || error ? <AlertTriangle size={22} /> : <ShieldCheck size={22} />}
      tone={broke.length || error ? 'danger' : undefined}
      size="wide"
      onClose={close}
      footer={
        <>
          <Button onClick={close} variant={committable.length ? 'default' : 'primary'}>
            Done
          </Button>
          {committable.length > 0 && (
            <Button variant="primary" onClick={() => setStage('commit')}>
              <GitCommitHorizontal size={15} />
              Commit {committable.length} project{committable.length === 1 ? '' : 's'}…
            </Button>
          )}
        </>
      }
    >
      {error ? (
        <>
          <pre className="rounded-[3px] bg-vuln-row p-3 font-mono text-[12px] whitespace-pre-wrap text-risk-security">{error}</pre>
          <div className="mt-2">
            <ReportButton make={() => batchErrorReport(error, plans, ran)} onNote={setNote} />
          </div>
        </>
      ) : (
        <div
          className={cx(
            'mb-2 flex items-center gap-3 rounded-[3px] px-3.5 py-3',
            broke.length || stopped.length || (!ran.checks && !filesOnly) ? 'bg-[color-mix(in_oklab,var(--risk-review)_12%,transparent)]' : 'bg-[color-mix(in_oklab,var(--ok)_12%,transparent)]',
          )}
        >
          {stopped.length && !broke.length ? (
            <Ban size={22} className="shrink-0 text-muted" />
          ) : broke.length || (!ran.checks && !filesOnly) ? (
            <AlertTriangle size={22} className="shrink-0 text-risk-review" />
          ) : (
            <ShieldCheck size={22} className="shrink-0 text-ok" />
          )}
          <div>
            <b className="block text-[14px]">
              {broke.length || stopped.length
                ? `${passed.length} of ${total} project${total === 1 ? '' : 's'} updated${stopped.length ? `, ${stopped.length} cancelled` : ''}`
                : `${total} project${total === 1 ? '' : 's'} updated${allPushed ? ', committed and pushed' : anyCommitted ? ' and committed' : ''}`}
            </b>
            <span className="text-[12.5px] text-muted">
              {broke.length
                ? `${broke.map((j) => j.name).join(', ')} could not be updated and ${broke.length === 1 ? 'was' : 'were'} put back as ${broke.length === 1 ? 'it was' : 'they were'}.`
                : stopped.length
                  ? `Cancelled ${stopped.length === 1 ? 'project is' : 'projects are'} back as ${stopped.length === 1 ? 'it was' : 'they were'}.`
                  : filesOnly
                  ? 'Workflow files updated. They take effect the next time your CI runs.'
                  : ran.checks
                  ? heldBack
                    ? `Every build and test passed. ${heldBack} update${heldBack === 1 ? '' : 's'} broke a check and ${heldBack === 1 ? 'was' : 'were'} left out.`
                    : 'Every build and test passed.'
                  : 'Checks were skipped. Run your tests or let CI check before merging.'}
            </span>
          </div>
        </div>
      )}
      {jobs.map((job) => {
        const o = outcomeOf(job)
        if (!o) return null
        const c = commitOf(job)
        const hash = o.committed ?? c?.committed
        const why = o.commitError ?? c?.error ?? (ran.commit ? o.commitSkipped : null)
        const pushed = !!(o.pushed || c?.pushed)
        const pushWhy = o.pushError ?? c?.pushError ?? null
        const changes = changesOf(job)
        const failing = o.steps.filter((s) => !s.ok)
        const blocking = o.conflicts.filter((x) => x.blocking)
        const warned = o.conflicts.filter((x) => !x.blocking)
        const culprits = [...new Set(blocking.flatMap((x) => (x.keep ? [x.keep.name] : [])))]
        const rest = changes.filter((ch) => !culprits.includes(ch.name)).length
        const log = [o.error, ...failing.map((s) => s.output)].filter(Boolean).join('\n\n')
        const failNotes = (o.notes ?? []).map((note) => (
          <small key={note} className="mt-0.5 block text-[12px] text-muted">
            {note}
          </small>
        ))
        const held = fallbacks[job.key.toLowerCase()] ?? []
        return (
          <div key={job.key} className="grid grid-cols-[24px_1fr_auto] items-start gap-2.5 border-b border-line py-2.5">
            <Avatar name={job.name} repo={job.key} />
            <span className="flex min-w-0 flex-col gap-0.5">
              <b className="text-[13px]">{job.name}</b>
              {held.length > 0 && !o.cancelled && (
                <ul className="m-0 mb-0.5 grid list-none gap-1.5 p-0">
                  {held.map((f) => {
                    const done = kept.has(`${job.key.toLowerCase()}|${f.keep.name}`)
                    return (
                      <li key={f.name} className="flex items-center gap-2 text-[12.5px]">
                        <RotateCcw size={14} className="shrink-0 text-risk-review" />
                        <span className="min-w-0 flex-1">
                          {f.with ? `${f.name} ${f.tried} only goes with the new ${f.with}` : `${f.name} ${f.tried} broke it`}, so Mehen {f.to ? `used ${f.to} instead` : `left it on ${f.stays}`}.
                        </span>
                        {done ? (
                          <span className="inline-flex items-center gap-1 text-[12px] whitespace-nowrap text-state">
                            <Pin size={13} />
                            Kept on {f.keep.line}.x
                          </span>
                        ) : (
                          <Button onClick={() => keep(f.keep, job.key)} title={`Stop offering ${f.name} updates past ${f.keep.line}.x in ${job.name}. You can change this in Settings.`}>
                            <Pin size={14} />
                            Keep {f.name} on {f.keep.line}.x
                          </Button>
                        )}
                      </li>
                    )
                  })}
                </ul>
              )}
              {o.cancelled ? (
                <small className="text-[12px] text-muted">{o.rolledBack ? 'Cancelled. Files put back as they were; the updates are still selected.' : o.error}</small>
              ) : o.ok ? (
                <small className="text-[12px] text-muted">
                  {changes.map((ch) => `${ch.name} ${ch.to}`).join(', ')}
                  {hash && ` · committed ${hash}${job.branch ? ` on ${job.branch}` : ''}`}
                  {pushed && ' · pushed'}
                  {why && <span className="text-risk-review"> · not committed: {why}</span>}
                  {pushWhy && <span className="text-risk-review"> · not pushed: {pushWhy}</span>}
                  {warned.length > 0 && (
                    <span className="mt-0.5 flex items-start gap-1.5 text-risk-review" title={warned.map((x) => x.summary).join('\n')}>
                      <AlertTriangle size={13} className="mt-px shrink-0" />
                      {warned[0].summary}.{warned.length > 1 && ` (+${warned.length - 1} more)`}
                    </span>
                  )}
                  {(o.notes ?? []).map((note) => (
                    <span key={note} className="mt-0.5 block text-muted">
                      {note}
                    </span>
                  ))}
                </small>
              ) : o.missingTool ? (
                <>
                  <small className="text-[12px] text-risk-security">`{o.missingTool}` is not installed on this computer, so this project could not be updated. Files restored.</small>
                  <div className="mt-1.5">
                    <Button onClick={() => void api.openLink(installLink(o.missingTool!).url)}>Install {installLink(o.missingTool).name}</Button>
                  </div>
                  <small className="mt-0.5 block text-[12px] text-muted">After installing it, restart Mehen and run the update again.</small>
                </>
              ) : blocking.length ? (
                <>
                  <small className="text-[12px] text-risk-security">
                    Files restored. {blocking.length === 1 ? 'A package the project uses' : 'Packages the project uses'} would not work with this update:
                  </small>
                  <ul className="m-0 mt-1 grid list-none gap-1.5 p-0">
                    {blocking.map((x) => {
                      const done = x.keep && kept.has(`${job.key.toLowerCase()}|${x.keep.name}`)
                      return (
                        <li key={x.summary} className="flex items-center gap-2 text-[12.5px]">
                          <AlertTriangle size={14} className="shrink-0 text-risk-review" />
                          <span className="min-w-0 flex-1">{x.summary}.</span>
                          {x.keep &&
                            (done ? (
                              <span className="inline-flex items-center gap-1 text-[12px] whitespace-nowrap text-state">
                                <Pin size={13} />
                                Kept on {x.keep.line}.x
                              </span>
                            ) : (
                              <Button onClick={() => keep(x.keep!, job.key)} title={`Stop offering ${x.keep.name} updates past ${x.keep.line}.x in ${job.name}. You can change this in Settings.`}>
                                <Pin size={14} />
                                Keep {x.keep.name} on {x.keep.line}.x
                              </Button>
                            ))}
                        </li>
                      )
                    })}
                  </ul>
                  {failNotes}
                  {culprits.length > 0 && rest > 0 && (
                    <div className="mt-1.5">
                      <Button onClick={() => retry(job, culprits)}>
                        <RotateCcw size={14} />
                        Try again without {culprits.join(', ')}
                      </Button>
                    </div>
                  )}
                  <details className="mt-1">
                    <summary className="cursor-pointer text-[12px] text-muted hover:text-ink">Show output</summary>
                    <pre className="mt-1 max-h-60 overflow-auto rounded-[3px] bg-sunken px-2.5 py-2 font-mono text-[12px] leading-relaxed whitespace-pre-wrap text-ink">{log}</pre>
                  </details>
                </>
              ) : (
                <>
                  <small className="text-[12px] text-risk-security">{o.rolledBack ? 'Files restored. The updates are still selected so you can try again.' : o.error}</small>
                  {failNotes}
                  <pre className="mt-1 max-h-60 overflow-auto rounded-[3px] bg-sunken px-2.5 py-2 font-mono text-[12px] leading-relaxed whitespace-pre-wrap text-ink">
                    {log}
                  </pre>
                </>
              )}
              {!o.ok && !o.cancelled && !o.missingTool && (
                <div className="mt-1.5">
                  <ReportButton make={() => updateFailureReport(job, o, ran)} onNote={setNote} />
                </div>
              )}
            </span>
            {o.ok ? (
              <span className="flex flex-col items-end gap-1.5">
                <StateChip tone="ok">
                  {hash ? <GitCommitHorizontal size={14} /> : <Check size={14} />}
                  {pushed ? 'Pushed' : hash ? 'Committed' : 'Updated'}
                </StateChip>
                {onReviewInGitWyrm && job.plans.some((p) => p.repo) && (
                  <Button variant="ghost" className="h-7 px-2" onClick={() => onReviewInGitWyrm(job.key)} title={hash ? 'See the new commit in GitWyrm' : 'See the changed files in GitWyrm, then commit them there'}>
                    <GitWyrmMark size={14} />
                    Review in GitWyrm
                  </Button>
                )}
              </span>
            ) : o.cancelled ? (
              <StateChip tone="wait">
                <Ban size={14} />
                Cancelled
              </StateChip>
            ) : (
              <StateChip tone="fail">
                <RotateCcw size={14} />
                {o.rolledBack ? 'Restored' : 'Failed'}
              </StateChip>
            )}
          </div>
        )
      })}
    </Dialog>
  )
}

/**
 * The packages each manifest changes, one collapsible row per file. Small
 * updates open expanded; big ones start collapsed so the dialog stays short.
 */
function PlanFiles({ job }: { job: Job }) {
  const files = job.plans
    .map((plan) => {
      const paths = plan.edits.map((e) => relativePath([job.key], e.path))
      const manifest = relativePath([job.key], plan.projectId)
      return {
        key: plan.projectId,
        label: paths.length === 1 ? paths[0] : manifest,
        // A workflow folder edits several files under one project.
        others: paths.length > 1 ? paths : [],
        changes: [...plan.changes].sort((a, b) => a.name.localeCompare(b.name, undefined, { sensitivity: 'base' })),
        pinned: new Set(plan.pinned ?? []),
      }
    })
    .sort((a, b) => a.label.localeCompare(b.label, undefined, { sensitivity: 'base' }))
  const total = files.reduce((n, f) => n + f.changes.length, 0)
  const [open, setOpen] = useState<Set<string>>(() => new Set(files.length === 1 || total <= 12 ? files.map((f) => f.key) : []))
  const allOpen = open.size === files.length
  const toggle = (key: string, isOpen: boolean) =>
    setOpen((prev) => {
      if (prev.has(key) === isOpen) return prev
      const next = new Set(prev)
      if (isOpen) next.add(key)
      else next.delete(key)
      return next
    })

  return (
    <div className="px-3 pt-1.5">
      {files.length > 2 && (
        <div className="flex items-center justify-between pb-0.5 text-[12px] text-muted">
          <span>
            {total} change{total === 1 ? '' : 's'} in {files.length} files
          </span>
          <button type="button" onClick={() => setOpen(allOpen ? new Set() : new Set(files.map((f) => f.key)))} className="rounded-[3px] px-1.5 py-0.5 hover:bg-sunken hover:text-ink">
            {allOpen ? 'Collapse all' : 'Expand all'}
          </button>
        </div>
      )}
      {files.map((file) => {
        const majors = file.changes.filter((c) => bumpOf(c.from, c.to) === 'major').length
        return (
          <details key={file.key} open={open.has(file.key)} onToggle={(e) => toggle(file.key, e.currentTarget.open)} className="group border-b border-line last:border-b-0">
            <summary className="flex cursor-pointer list-none items-center gap-2 py-1.5 text-[12.5px] hover:text-ink [&::-webkit-details-marker]:hidden">
              <ChevronRight size={14} className="shrink-0 text-muted transition-transform group-open:rotate-90" />
              <FileText size={14} className="shrink-0 text-muted" />
              <span className="min-w-0 truncate font-mono text-[12px]" title={file.label}>
                {file.label}
              </span>
              <span className="ml-auto shrink-0 pl-2 text-[12px] text-muted">
                {file.changes.length} package{file.changes.length === 1 ? '' : 's'}
                {majors > 0 && <span className="text-risk-major"> · {majors} major</span>}
              </span>
            </summary>
            <ul className="m-0 grid list-none gap-1 pb-2 pl-[22px] text-[12.5px]">
              {file.changes.map((c) => (
                <li key={`${c.name}|${c.from}`} className="flex min-w-0 items-center gap-2">
                  <Box size={14} className="shrink-0 text-muted" />
                  <b className="truncate">{c.name}</b>
                  <code className="shrink-0 font-mono text-[12px] text-muted">{c.from}</code>
                  <span className="shrink-0 text-muted">to</span>
                  <code className={cx('shrink-0 font-mono text-[12px]', bumpOf(c.from, c.to) === 'major' && 'text-risk-major')}>{c.to}</code>
                  {file.pinned.has(c.name) && (
                    <span className="inline-flex shrink-0 items-center gap-1 text-[12px] text-muted" title={`Written as ${c.writtenBefore} now; the update writes ${c.writtenAfter}`}>
                      <Lock size={12} />
                      written as <code className="font-mono">{c.writtenAfter}</code>
                    </span>
                  )}
                </li>
              ))}
              {file.others.length > 0 && <li className="truncate text-[12px] text-muted">Files: {file.others.join(', ')}</li>}
            </ul>
          </details>
        )
      })}
    </div>
  )
}

/**
 * A failed update is Mehen not doing its job, so saying so takes one click:
 * the report carries the update's output. "Add a note" opens the full form.
 */
function ReportButton({ make, onNote }: { make: () => Report & { title: string }; onNote: (report: Report & { title: string }) => void }) {
  const [state, setState] = useState<{ step: 'idle' | 'sending' | 'sent' | 'failed'; message?: string }>({ step: 'idle' })
  const send = async () => {
    setState({ step: 'sending' })
    const result = await sendReport(make())
    setState(result.ok ? { step: 'sent' } : { step: 'failed', message: result.message })
  }
  if (state.step === 'sent') {
    return (
      <span className="inline-flex items-center gap-1.5 text-[12px] text-ok">
        <Check size={13} />
        Report sent. Thanks, this is how it gets fixed.
      </span>
    )
  }
  return (
    <span className="inline-flex flex-wrap items-center gap-x-2.5 gap-y-1">
      <Button onClick={send} disabled={state.step === 'sending'} title="Sends what was being updated and the output, with your user name taken out of paths" className="h-7 px-2.5 text-[12px]">
        <Send size={13} />
        {state.step === 'sending' ? 'Sending…' : state.step === 'failed' ? 'Try again' : 'Report this failure'}
      </Button>
      <button type="button" onClick={() => onNote(make())} disabled={state.step === 'sending'} className="text-[12px] text-muted underline underline-offset-2 hover:text-ink">
        Add a note first
      </button>
      {state.step === 'failed' && <span className="text-[12px] text-risk-security">{state.message}</span>}
    </span>
  )
}

function Unplannable({ failed, roots, onNote }: { failed: { project: Project; error: string }[]; roots: string[]; onNote: (report: Report & { title: string }) => void }) {
  return (
    <div className="mb-3 rounded-[3px] border border-[color-mix(in_oklab,var(--risk-review)_35%,var(--line))] px-3 py-2.5">
      <b className="flex items-center gap-2 text-[13px] text-risk-review">
        <AlertTriangle size={15} />
        {failed.length} manifest{failed.length === 1 ? " can't" : "s can't"} be updated automatically
      </b>
      <ul className="m-0 mt-1 list-none p-0 text-[12.5px] text-muted">
        {failed.map((f) => (
          <li key={f.project.id}>
            <span className="text-ink">{folderName(f.project.repo ?? f.project.dir)}</span> · {relativePath(roots, f.project.manifest)}: {f.error.split('\n')[0]}
          </li>
        ))}
      </ul>
      <div className="mt-2">
        <ReportButton make={() => planFailureReport(failed)} onNote={onNote} />
      </div>
    </div>
  )
}
