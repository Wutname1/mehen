import { ArrowRight, Check, ChevronRight, CircleAlert, Loader2, RotateCcw, TriangleAlert, X } from 'lucide-react'
import { useEffect, useRef, useState } from 'react'
import * as api from '../api'
import { commitMessageFor, relativePath } from '../derive'
import type { Change, Ecosystem, Inventory, Project, UpdateOutcome, UpdatePlan } from '../types'
import { EcoBadge, cx } from './bits'

export interface BulkTarget {
  project: Project
  changes: Change[]
}

type RowState = 'planning' | 'ready' | 'unplannable' | 'waiting' | 'running' | 'ok' | 'failed'

interface Row {
  target: BulkTarget
  state: RowState
  plan?: UpdatePlan
  error?: string
  outcome?: UpdateOutcome
  step?: string
}

type Phase = 'planning' | 'review' | 'applying' | 'refreshing' | 'done'

/**
 * Moves one package to the same version across many projects: plans each
 * project, shows them together for review, then applies them one at a time.
 * A failing project is rolled back on its own and the rest carry on.
 */
export function BulkUpdateDialog({
  packageName,
  ecosystem,
  to,
  targets,
  roots,
  onClose,
}: {
  packageName: string
  ecosystem: Ecosystem
  to: string
  targets: BulkTarget[]
  roots: string[]
  onClose: (refreshed: Inventory | null) => void
}) {
  const [rows, setRows] = useState<Row[]>(() => targets.map((target) => ({ target, state: 'planning' })))
  const [phase, setPhase] = useState<Phase>('planning')
  const [verify, setVerify] = useState(true)
  const [commit, setCommit] = useState(false)
  const [open, setOpen] = useState<string | null>(null)
  const [refreshed, setRefreshed] = useState<Inventory | null>(null)
  const [applyTotal, setApplyTotal] = useState(0)
  const current = useRef<number | null>(null)

  const patch = (i: number, update: Partial<Row>) => setRows((prev) => prev.map((r, j) => (j === i ? { ...r, ...update } : r)))

  useEffect(() => {
    let cancelled = false
    ;(async () => {
      for (const [i, t] of targets.entries()) {
        if (cancelled) return
        try {
          const plan = await api.planUpdate(t.project.id, t.changes)
          patch(i, { state: plan.edits.length > 0 || plan.steps.length > 0 ? 'ready' : 'unplannable', plan, error: plan.edits.length === 0 && plan.steps.length === 0 ? 'Nothing to change' : undefined })
        } catch (e) {
          patch(i, { state: 'unplannable', error: String(e) })
        }
      }
      if (!cancelled) setPhase('review')
    })()
    return () => {
      cancelled = true
    }
  }, [targets])

  useEffect(() => {
    const unlisten = api.onUpdateEvent((e) => {
      if (current.current !== null && e.state === 'running') patch(current.current, { step: e.label })
    })
    return () => {
      unlisten.then((fn) => fn())
    }
  }, [])

  const busy = phase === 'planning' || phase === 'applying' || phase === 'refreshing'
  const close = () => !busy && onClose(refreshed)

  const apply = async () => {
    setPhase('applying')
    const ready = rows.map((r, i) => [r, i] as const).filter(([r]) => r.state === 'ready')
    setApplyTotal(ready.length)
    setRows((prev) => prev.map((r) => (r.state === 'ready' ? { ...r, state: 'waiting' } : r)))
    let anyOk = false
    for (const [row, i] of ready) {
      current.current = i
      patch(i, { state: 'running' })
      try {
        const plan = row.plan!
        const message = commit && !plan.commitBlocked ? commitMessageFor(plan.changes) : null
        const result = await api.applyUpdate(plan, verify, false, message)
        anyOk ||= result.outcome.ok
        patch(i, { state: result.outcome.ok ? 'ok' : 'failed', outcome: result.outcome, step: undefined })
      } catch (e) {
        patch(i, { state: 'failed', outcome: { ok: false, rolledBack: false, error: String(e), steps: [], committed: null, commitError: null }, step: undefined })
      }
    }
    current.current = null
    if (anyOk) {
      setPhase('refreshing')
      try {
        setRefreshed(await api.scanAndCheck(false))
      } catch {
        // The update itself succeeded; the next manual check will catch up.
      }
    }
    setPhase('done')
  }

  const ready = rows.filter((r) => r.state === 'ready').length
  const unplannable = rows.filter((r) => r.state === 'unplannable')
  const done = rows.filter((r) => r.state === 'ok').length
  const failed = rows.filter((r) => r.state === 'failed').length
  const hasVerify = rows.some((r) => r.plan?.steps.some((s) => s.kind === 'verify'))
  const warnings = [...new Set(rows.flatMap((r) => (r.state === 'unplannable' ? [] : (r.plan?.warnings ?? []))))]

  return (
    <div className="fixed inset-0 z-50 flex items-center justify-center bg-black/60 p-6" onClick={close}>
      <div
        role="dialog"
        aria-label="Update across projects"
        className="flex max-h-full w-[min(980px,100%)] flex-col overflow-hidden rounded-2xl border border-line-strong bg-bg shadow-2xl"
        onClick={(e) => e.stopPropagation()}
      >
        <header className="flex items-center gap-3 border-b border-line px-5 py-3">
          <EcoBadge ecosystem={ecosystem} />
          <h2 className="min-w-0 flex-1 truncate text-[15px] font-semibold">
            {packageName} <ArrowRight size={14} className="mx-1 inline text-dim" />
            <span className="font-mono text-turq">{to}</span>
            <span className="font-normal text-muted">
              {' '}
              in {targets.length} project{targets.length === 1 ? '' : 's'}
            </span>
          </h2>
          <button type="button" onClick={close} disabled={busy} aria-label="Close" className="rounded-md p-1 text-dim hover:bg-hover hover:text-ink disabled:opacity-40">
            <X size={16} />
          </button>
        </header>

        <div className="min-h-0 flex-1 overflow-y-auto">
          {rows.map((row, i) => {
            const key = row.target.project.id
            const expanded = open === key
            const change = row.plan?.changes[0]
            return (
              <div key={key} className="border-b border-line/60">
                <div className="flex items-center gap-3 px-5 py-2.5">
                  <RowIcon state={row.state} />
                  <button
                    type="button"
                    onClick={() => setOpen(expanded ? null : key)}
                    disabled={!row.plan && !row.error && !row.outcome}
                    className="flex min-w-0 flex-1 items-center gap-2 text-left"
                    aria-expanded={expanded}
                  >
                    <ChevronRight size={13} className={cx('shrink-0 text-dim transition-transform', expanded && 'rotate-90')} />
                    <span className="truncate font-medium">{row.target.project.name}</span>
                    <span className="truncate text-[11.5px] text-dim">{relativePath(roots, row.target.project.dir)}</span>
                  </button>
                  <span className="shrink-0 text-right text-[12px]">
                    {row.state === 'unplannable' ? (
                      <span className="text-amber">{row.error?.split('\n')[0]}</span>
                    ) : row.state === 'running' ? (
                      <span className="text-gold">{row.step ?? 'Writing changes…'}</span>
                    ) : row.state === 'failed' ? (
                      <span className="text-carnelian">{row.outcome?.rolledBack ? 'Failed, restored' : 'Failed'}</span>
                    ) : row.state === 'ok' && (row.outcome?.committed || row.outcome?.commitError) ? (
                      row.outcome.committed ? (
                        <span className="font-mono text-turq">committed {row.outcome.committed}</span>
                      ) : (
                        <span className="text-amber">updated, commit failed</span>
                      )
                    ) : change ? (
                      <span className="font-mono text-muted">
                        {change.writtenBefore} <span className="text-dim">→</span> <span className="text-turq">{change.writtenAfter}</span>
                        {row.plan!.changes.length > 1 && <span className="font-sans text-dim"> +{row.plan!.changes.length - 1}</span>}
                      </span>
                    ) : null}
                  </span>
                </div>
                {expanded && (
                  <div className="px-5 pb-3 pl-14">
                    {row.plan?.edits.map((e) => (
                      <pre key={e.path} className="mb-2 overflow-x-auto rounded-lg border border-line bg-panel py-2 font-mono text-[11.5px] leading-[1.55]">
                        {e.diff
                          .replace(/\n$/, '')
                          .split('\n')
                          .map((line, j) => (
                            <div
                              key={j}
                              className={cx(
                                'px-3',
                                line.startsWith('+++') || line.startsWith('---')
                                  ? 'text-muted'
                                  : line.startsWith('+')
                                    ? 'bg-turq-soft text-turq'
                                    : line.startsWith('-')
                                      ? 'bg-carnelian-soft text-carnelian'
                                      : line.startsWith('@@')
                                        ? 'text-lapis'
                                        : 'text-dim',
                              )}
                            >
                              {line || ' '}
                            </div>
                          ))}
                      </pre>
                    ))}
                    {row.plan && row.plan.steps.length > 0 && (
                      <div className="text-[11.5px] text-dim">
                        Then: {row.plan.steps.filter((s) => verify || s.kind === 'install').map((s) => s.label).join(' → ')}
                      </div>
                    )}
                    {row.state === 'unplannable' && <div className="text-[12px] text-amber">{row.error}</div>}
                    {row.outcome && !row.outcome.ok && (
                      <pre className="mt-2 max-h-60 overflow-auto rounded-md bg-carnelian-soft p-2 font-mono text-[11px] text-carnelian">
                        {[row.outcome.error, ...row.outcome.steps.filter((s) => !s.ok).map((s) => s.output)].filter(Boolean).join('\n\n')}
                      </pre>
                    )}
                  </div>
                )}
              </div>
            )
          })}
        </div>

        {warnings.length > 0 && phase === 'review' && (
          <ul className="flex flex-col gap-1 border-t border-line px-5 py-2">
            {warnings.map((w) => (
              <li key={w} className="flex gap-2 text-[12px] text-amber">
                <TriangleAlert size={13} className="mt-0.5 shrink-0" />
                {w}
              </li>
            ))}
          </ul>
        )}

        <footer className="flex items-center gap-3 border-t border-line px-5 py-3 text-[12.5px]">
          <div className="flex-1 text-dim">
            {phase === 'planning' && (
              <span className="inline-flex items-center gap-2">
                <Loader2 size={13} className="animate-spin" /> Working out the changes ({rows.filter((r) => r.state !== 'planning').length}/{rows.length})…
              </span>
            )}
            {phase === 'review' && (
              <span>
                {ready} project{ready === 1 ? '' : 's'} ready
                {unplannable.length > 0 && `, ${unplannable.length} can't be changed automatically`}. Each project is applied on its own; one failing
                doesn't stop the rest.
              </span>
            )}
            {phase === 'applying' && `Updating ${Math.min(done + failed + 1, applyTotal)} of ${applyTotal}…`}
            {phase === 'refreshing' && (
              <span className="inline-flex items-center gap-2">
                <Loader2 size={13} className="animate-spin" /> Refreshing results…
              </span>
            )}
            {phase === 'done' && (
              <span className={failed ? 'text-amber' : 'text-turq'}>
                {done} updated{failed > 0 && `, ${failed} failed and restored`}.
              </span>
            )}
          </div>
          {phase === 'review' && rows.some((r) => r.plan?.repo) && (
            <label
              className="flex items-center gap-2 text-muted"
              title="Commits each project's manifest and lockfile on its current branch. Projects with uncommitted changes in those files are left uncommitted."
            >
              <input type="checkbox" checked={commit} onChange={(e) => setCommit(e.target.checked)} className="size-3.5 accent-[var(--color-gold)]" />
              Commit each
              {commit && rows.some((r) => r.state === 'ready' && r.plan?.commitBlocked) && (
                <span className="text-amber">({rows.filter((r) => r.state === 'ready' && r.plan?.commitBlocked).length} can't)</span>
              )}
            </label>
          )}
          {phase === 'review' && hasVerify && (
            <label className="flex items-center gap-2 text-muted">
              <input type="checkbox" checked={verify} onChange={(e) => setVerify(e.target.checked)} className="size-3.5 accent-[var(--color-gold)]" />
              Check builds
            </label>
          )}
          {phase === 'review' ? (
            <>
              <button type="button" onClick={close} className="rounded-lg border border-line-strong px-3.5 py-1.5 text-muted hover:text-ink">
                Cancel
              </button>
              <button
                type="button"
                onClick={apply}
                disabled={ready === 0}
                className="rounded-lg bg-gold px-4 py-1.5 font-semibold text-[#1d1506] hover:brightness-110 disabled:opacity-40"
              >
                Update {ready} project{ready === 1 ? '' : 's'}
              </button>
            </>
          ) : (
            <button type="button" onClick={close} disabled={busy} className="rounded-lg border border-line-strong px-3.5 py-1.5 text-muted hover:text-ink disabled:opacity-40">
              Close
            </button>
          )}
        </footer>
      </div>
    </div>
  )
}

function RowIcon({ state }: { state: RowState }) {
  if (state === 'planning' || state === 'running') return <Loader2 size={14} className="shrink-0 animate-spin text-gold" />
  if (state === 'ok') return <Check size={14} className="shrink-0 text-turq" />
  if (state === 'failed') return <RotateCcw size={14} className="shrink-0 text-carnelian" />
  if (state === 'unplannable') return <CircleAlert size={14} className="shrink-0 text-amber" />
  return <span className="size-3.5 shrink-0 rounded-full border border-line-strong" />
}
