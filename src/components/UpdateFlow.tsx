import { AlertTriangle, Box, Check, Copy, FileText, GitBranch, GitCommitHorizontal, Loader2, Play, RefreshCw, RotateCcw, ShieldCheck, Terminal } from 'lucide-react'
import { useEffect, useMemo, useState, type ReactNode } from 'react'
import * as api from '../api'
import { folderName, relativePath } from '../derive'
import type { BatchEvent, Change, CommitOutcome, Inventory, JobOutcome, JobState, Project, UpdatePlan } from '../types'
import { cx } from './bits'
import { Button, Dialog } from './Dialog'
import { Checkbox } from './Queue'

export interface UpdateTarget {
  project: Project
  changes: Change[]
}

type Stage = 'planning' | 'preview' | 'confirm' | 'running' | 'done' | 'commit'

interface Job {
  key: string
  name: string
  plans: UpdatePlan[]
  branch: string | null
  /** Why this repository can't be committed. */
  blocked: string | null
}

interface Live {
  state: JobState
  label: string | null
}

const jobKey = (p: UpdatePlan) => p.repo ?? p.projectId.replace(/[\\/][^\\/]*$/, '')

function jobsOf(plans: UpdatePlan[], nameOf: (key: string) => string): Job[] {
  const map = new Map<string, Job>()
  for (const plan of plans) {
    const key = jobKey(plan)
    const job = map.get(key.toLowerCase()) ?? { key, name: nameOf(key), plans: [], branch: plan.branch, blocked: null }
    job.plans.push(plan)
    map.set(key.toLowerCase(), job)
  }
  for (const job of map.values()) {
    const reasons = [...new Set(job.plans.map((p) => p.commitBlocked).filter((r): r is string => !!r))]
    job.blocked = reasons.length ? reasons.join('; ') : null
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

function stepsOf(job: Job, checks: boolean) {
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
  return { install, checks: checks ? [...pick('verify'), ...pick('test')] : [] }
}

/** Labels once each, with a count when several folders run the same thing. */
const labels = (steps: { label: string }[]) => {
  const counts = new Map<string, number>()
  for (const s of steps) counts.set(s.label, (counts.get(s.label) ?? 0) + 1)
  return [...counts].map(([label, n]) => (n > 1 ? `${label} ×${n}` : label)).join(', ')
}

const monogram = (name: string) => name.replace(/^.*\//, '').slice(0, 2).toUpperCase()

function Avatar({ name }: { name: string }) {
  return (
    <span aria-hidden className="grid size-6 shrink-0 place-items-center rounded-[3px] border border-line bg-paper-2 font-mono text-[10.5px] font-bold text-muted">
      {monogram(name)}
    </span>
  )
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
  checks,
  commit,
  onOptions,
  nameOf,
  onClose,
}: {
  targets: UpdateTarget[]
  start: 'confirm' | 'preview'
  roots: string[]
  checks: boolean
  commit: boolean
  onOptions: (patch: { checks?: boolean; commit?: boolean }) => void
  nameOf: (key: string) => string
  onClose: (refreshed: Inventory | null) => void
}) {
  const [stage, setStage] = useState<Stage>('planning')
  const [plans, setPlans] = useState<UpdatePlan[]>([])
  const [failed, setFailed] = useState<{ project: Project; error: string }[]>([])
  const [live, setLive] = useState<Record<string, Live>>({})
  const [outcomes, setOutcomes] = useState<JobOutcome[]>([])
  const [commits, setCommits] = useState<CommitOutcome[]>([])
  const [refreshed, setRefreshed] = useState<Inventory | null>(null)
  const [ran, setRan] = useState({ checks, commit })
  const [error, setError] = useState<string | null>(null)
  const [copied, setCopied] = useState(false)

  useEffect(() => {
    let cancelled = false
    Promise.all(
      targets.map((t) =>
        api.planUpdate(t.project.id, t.changes).then(
          (plan) => (plan.edits.length || plan.steps.length ? { plan } : { error: 'Nothing to change', project: t.project }),
          (e) => ({ error: String(e), project: t.project }),
        ),
      ),
    ).then((results) => {
      if (cancelled) return
      setPlans(results.flatMap((r) => ('plan' in r && r.plan ? [r.plan] : [])))
      setFailed(results.flatMap((r) => ('error' in r && r.error ? [{ project: r.project, error: r.error }] : [])))
      setStage(start)
    })
    return () => {
      cancelled = true
    }
  }, [targets, start])

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

  const run = async () => {
    setRan({ checks, commit })
    setStage('running')
    setError(null)
    try {
      const result = await api.applyBatch(plans, checks, commit)
      setOutcomes(result.outcomes)
      if (result.inventory) setRefreshed(result.inventory)
    } catch (e) {
      setError(String(e))
    }
    setStage('done')
  }

  const outcomeOf = (job: Job) => outcomes.find((o) => o.projects.some((id) => job.plans.some((p) => p.projectId === id)))
  const passed = jobs.filter((j) => outcomeOf(j)?.ok)
  const broke = jobs.filter((j) => outcomeOf(j) && !outcomeOf(j)!.ok)
  const commitOf = (job: Job) => commits.find((c) => c.job.toLowerCase() === job.key.toLowerCase())
  const committable = passed.filter((j) => !j.blocked && !outcomeOf(j)?.committed && !commitOf(j)?.committed)

  const commitNow = async () => {
    setStage('running')
    try {
      setCommits(await api.commitUpdate(committable.flatMap((j) => j.plans)))
    } catch (e) {
      setError(String(e))
    }
    setStage('done')
  }

  const commands = () =>
    jobs
      .map((job) => {
        const { install, checks: steps } = stepsOf(job, checks)
        const lines = [`# ${job.name}${job.branch ? ` (${job.branch})` : ''}`, `cd ${job.key}`, ...[...install, ...steps].map((s) => [s.program, ...s.args].join(' '))]
        if (commit && !job.blocked) lines.push(`git commit -m "${commitSubject(changesOf(job).length)}" -- <changed files>`)
        return lines.join('\n')
      })
      .join('\n\n')

  const label = checks ? 'Update & run checks' : 'Update & install only'
  const total = jobs.length

  const options = (
    <div className="mr-auto flex flex-wrap gap-x-[18px] gap-y-1.5" role="group" aria-label="When updating">
      <label className="inline-flex cursor-pointer items-center gap-2 text-[12.5px]">
        <Checkbox checked={checks} onChange={() => onOptions({ checks: !checks })} label="Run build and test checks" />
        Run build and test checks
      </label>
      <label className="inline-flex cursor-pointer items-center gap-2 text-[12.5px]">
        <Checkbox checked={commit} onChange={() => onOptions({ commit: !commit })} label="Commit each repository" />
        Commit each repository
      </label>
    </div>
  )

  if (stage === 'planning') {
    return (
      <Dialog title="Working out the changes" size="wide" busy onClose={close} icon={<Loader2 size={22} className="animate-spin" />}>
        <p className="py-8 text-center text-muted">
          Reading {targets.length} manifest{targets.length === 1 ? '' : 's'} to see exactly what will change…
        </p>
      </Dialog>
    )
  }

  if (stage === 'preview') {
    return (
      <Dialog
        title="Files and commands"
        description="Exactly what Mehen will change and run in each project. Reviewing changes nothing on disk."
        icon={<Terminal size={22} />}
        size="wide"
        onClose={close}
        footer={
          <>
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
            <Button onClick={close}>Close</Button>
            <Button variant="primary" onClick={() => setStage('confirm')} disabled={!jobs.length}>
              <Play size={15} />
              Continue to update
            </Button>
          </>
        }
      >
        {jobs.map((job) => {
          const { install, checks: steps } = stepsOf(job, checks)
          return (
            <section key={job.key} className="mb-5">
              <h3 className="mb-2 flex items-center gap-2 text-[13px] font-semibold">
                <Avatar name={job.name} />
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
              <pre className="overflow-x-auto rounded-[3px] bg-bar px-3.5 py-3 font-mono text-[12.5px] leading-[1.65] text-rail-ink">
                {[...install, ...steps].map((s) => [s.program, ...s.args].join(' ')).join('\n') || '# Files are edited only; nothing to run.'}
              </pre>
            </section>
          )
        })}
        {failed.length > 0 && <Unplannable failed={failed} roots={roots} />}
      </Dialog>
    )
  }

  if (stage === 'confirm') {
    const packages = new Set(plans.flatMap((p) => p.changes.map((c) => c.name))).size
    return (
      <Dialog
        title={`Update ${total} project${total === 1 ? '' : 's'}`}
        description={`${packages} package${packages === 1 ? '' : 's'} will change. Mehen edits the files below and refreshes lockfiles${checks ? ", then runs each project's checks" : ''}${commit ? ', then commits' : ''}.`}
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
        {failed.length > 0 && <Unplannable failed={failed} roots={roots} />}
        <div className="grid gap-2.5">
          {jobs.map((job) => {
            const changes = changesOf(job)
            const files = [...new Set(job.plans.flatMap((p) => [...p.edits.map((e) => e.path), ...p.snapshots]))].map((f) => relativePath([job.key], f))
            const { install, checks: steps } = stepsOf(job, checks)
            const warnings = [...new Set(job.plans.flatMap((p) => p.warnings))]
            return (
              <section key={job.key} className="rounded-[3px] border border-line bg-paper">
                <header className="flex items-center gap-2.5 border-b border-line px-3 py-2.5">
                  <Avatar name={job.name} />
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
                <ul className="m-0 grid list-none gap-1 px-3 pt-2 pb-2.5 text-[12.5px]">
                  {changes.map((c) => (
                    <li key={c.name} className="flex items-center gap-2">
                      <Box size={14} className="shrink-0 text-muted" />
                      <b>{c.name}</b>
                      <code className="font-mono text-[12px] text-muted">{c.from}</code> to <code className="font-mono text-[12px]">{c.to}</code>
                    </li>
                  ))}
                  <li className="flex items-center gap-2 text-muted">
                    <FileText size={14} className="shrink-0" />
                    <span className="truncate" title={files.join('\n')}>
                      {files.join(', ')}
                    </span>
                  </li>
                  <li className="flex items-center gap-2 text-muted">
                    <Terminal size={14} className="shrink-0" />
                    {checks
                      ? steps.length
                        ? `Checks: ${labels(steps)}`
                        : `No checks for this project${install.length ? `. Install: ${labels(install)}` : ''}`
                      : install.length
                        ? `Install: ${labels(install)}. Checks skipped.`
                        : 'Files are edited only; nothing to run.'}
                  </li>
                  {commit && !job.blocked && (
                    <li className="flex items-center gap-2 text-muted">
                      <GitCommitHorizontal size={14} className="shrink-0" />
                      Commit on {job.branch ?? 'the current branch'}: “{commitSubject(changes.length)}”
                    </li>
                  )}
                </ul>
                {job.blocked && (commit || /uncommitted/.test(job.blocked)) && (
                  <div className="mx-3 mb-2.5 flex gap-2 rounded-[3px] bg-[color-mix(in_oklab,var(--risk-review)_10%,transparent)] px-2.5 py-2 text-[12.5px] text-risk-review">
                    <AlertTriangle size={15} className="mt-px shrink-0" />
                    <span>
                      {/uncommitted/.test(job.blocked)
                        ? commit
                          ? `Won't be committed: ${job.blocked.replace(/^uncommitted changes in /, '')} already ${job.blocked.includes(',') ? 'have' : 'has'} changes you haven't committed. Commit or stash them first to include this project.`
                          : `${job.blocked.replace(/^uncommitted changes in /, '')} already ${job.blocked.includes(',') ? 'have' : 'has'} changes you haven't committed; they will sit alongside this update.`
                        : `Won't be committed: ${job.blocked}.`}
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
          {checks ? "If a check fails, Mehen puts that project's files back and keeps its updates selected." : 'Nothing is built or tested. Run your tests or let CI check before merging.'}{' '}
          {commit ? 'Commits stay local; nothing is pushed.' : 'Nothing is committed until you choose to.'} Different projects update side by side; projects that need the same tool take turns.
        </p>
      </Dialog>
    )
  }

  if (stage === 'running') {
    return (
      <Dialog
        title={outcomes.length ? 'Committing' : 'Updating'}
        description={outcomes.length ? 'Committing the files Mehen changed.' : `Each project is updated${ran.checks ? ' and checked' : ''}${ran.commit ? ' and committed' : ''}. Projects that need the same tool take turns.`}
        icon={<RefreshCw size={22} className="animate-spin" />}
        size="wide"
        busy
        onClose={close}
        footer={<span className="mr-auto text-[12.5px] text-muted">Working. This window closes when you choose, once everything is done.</span>}
      >
        {jobs.map((job) => {
          const states = job.plans.map((p) => live[p.projectId.toLowerCase()]).filter(Boolean)
          const now = states.find((s) => s.state === 'running' || s.state === 'committing') ?? states.find((s) => s.state === 'waiting') ?? states[0]
          const state = now?.state ?? 'queued'
          return (
            <div key={job.key} className="grid grid-cols-[24px_1fr_auto] items-center gap-2.5 border-b border-line py-2.5">
              <Avatar name={job.name} />
              <span className="flex min-w-0 flex-col">
                <b className="text-[13px]">{job.name}</b>
                <small className="truncate text-[12px] text-muted">{state === 'queued' ? `${changesOf(job).length} package${changesOf(job).length === 1 ? '' : 's'}` : (now?.label ?? '')}</small>
              </span>
              {state === 'done' ? (
                <StateChip tone="ok">
                  <Check size={14} />
                  Done
                </StateChip>
              ) : state === 'failed' || state === 'rolled-back' ? (
                <StateChip tone="fail">
                  <AlertTriangle size={14} />
                  Failed
                </StateChip>
              ) : state === 'running' || state === 'committing' ? (
                <StateChip tone="run" spin>
                  {state === 'committing' ? 'Committing' : 'Running'}
                </StateChip>
              ) : (
                <StateChip tone="wait">Waiting</StateChip>
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
        description="Only the files Mehen changed are committed, on each project's current branch. Nothing is pushed."
        icon={<GitCommitHorizontal size={22} />}
        onClose={() => setStage('done')}
        footer={
          <>
            <Button onClick={() => setStage('done')}>Not now</Button>
            <Button variant="primary" onClick={commitNow}>
              <GitCommitHorizontal size={15} />
              Commit
            </Button>
          </>
        }
      >
        {committable.map((job) => {
          const changes = changesOf(job)
          return (
            <div key={job.key} className="grid grid-cols-[24px_1fr_auto] items-start gap-2.5 border-b border-line py-2.5">
              <Avatar name={job.name} />
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

  const anyCommitted = passed.some((j) => outcomeOf(j)?.committed || commitOf(j)?.committed)
  return (
    <Dialog
      title={error ? 'Update could not run' : broke.length ? 'Update finished with a problem' : 'Update finished'}
      description={anyCommitted ? 'Each committed project has a new local commit. Nothing was pushed.' : 'Changed files are ready to commit. Nothing has been committed yet.'}
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
        <pre className="rounded-[3px] bg-vuln-row p-3 font-mono text-[12px] whitespace-pre-wrap text-risk-security">{error}</pre>
      ) : (
        <div
          className={cx(
            'mb-2 flex items-center gap-3 rounded-[3px] px-3.5 py-3',
            broke.length || !ran.checks ? 'bg-[color-mix(in_oklab,var(--risk-review)_12%,transparent)]' : 'bg-[color-mix(in_oklab,var(--ok)_12%,transparent)]',
          )}
        >
          {broke.length || !ran.checks ? <AlertTriangle size={22} className="shrink-0 text-risk-review" /> : <ShieldCheck size={22} className="shrink-0 text-ok" />}
          <div>
            <b className="block text-[14px]">
              {broke.length ? `${passed.length} of ${total} projects updated` : `${total} project${total === 1 ? '' : 's'} updated${anyCommitted ? ' and committed' : ''}`}
            </b>
            <span className="text-[12.5px] text-muted">
              {broke.length
                ? `${broke.map((j) => j.name).join(', ')} failed ${broke.length === 1 ? 'its' : 'their'} checks and ${broke.length === 1 ? 'was' : 'were'} restored.`
                : ran.checks
                  ? 'Every build and test passed.'
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
        const changes = changesOf(job)
        const failing = o.steps.filter((s) => !s.ok)
        return (
          <div key={job.key} className="grid grid-cols-[24px_1fr_auto] items-start gap-2.5 border-b border-line py-2.5">
            <Avatar name={job.name} />
            <span className="flex min-w-0 flex-col gap-0.5">
              <b className="text-[13px]">{job.name}</b>
              {o.ok ? (
                <small className="text-[12px] text-muted">
                  {changes.map((ch) => `${ch.name} ${ch.to}`).join(', ')}
                  {hash && ` · committed ${hash}${job.branch ? ` on ${job.branch}` : ''}`}
                  {why && <span className="text-risk-review"> · not committed: {why}</span>}
                </small>
              ) : (
                <>
                  <small className="text-[12px] text-risk-security">{o.rolledBack ? 'Files restored. The updates are still selected so you can try again.' : o.error}</small>
                  <pre className="mt-1 max-h-60 overflow-auto rounded-[3px] bg-sunken px-2.5 py-2 font-mono text-[12px] leading-relaxed whitespace-pre-wrap text-ink">
                    {[o.error, ...failing.map((s) => s.output)].filter(Boolean).join('\n\n')}
                  </pre>
                </>
              )}
            </span>
            {o.ok ? (
              <StateChip tone="ok">
                {hash ? <GitCommitHorizontal size={14} /> : <Check size={14} />}
                {hash ? 'Committed' : 'Updated'}
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

function Unplannable({ failed, roots }: { failed: { project: Project; error: string }[]; roots: string[] }) {
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
    </div>
  )
}
