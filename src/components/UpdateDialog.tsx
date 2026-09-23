import { ArrowRight, Check, ChevronRight, CircleAlert, Loader2, RotateCcw, TriangleAlert, X } from 'lucide-react'
import { useEffect, useState } from 'react'
import * as api from '../api'
import { relativePath } from '../derive'
import type { Change, Inventory, StepResult, UpdateOutcome, UpdatePlan } from '../types'
import { cx } from './bits'

type Phase = { name: 'planning' } | { name: 'plan-failed'; error: string } | { name: 'review' } | { name: 'applying' } | { name: 'done'; outcome: UpdateOutcome }

type StepState = 'waiting' | 'running' | 'ok' | 'failed' | 'skipped'

export function UpdateDialog({
  projectId,
  changes,
  roots,
  onClose,
}: {
  projectId: string
  changes: Change[]
  roots: string[]
  /** Called with a refreshed result when an update succeeded. */
  onClose: (refreshed: Inventory | null) => void
}) {
  const [phase, setPhase] = useState<Phase>({ name: 'planning' })
  const [plan, setPlan] = useState<UpdatePlan | null>(null)
  const [verify, setVerify] = useState(true)
  const [stepStates, setStepStates] = useState<StepState[]>([])
  const [refreshed, setRefreshed] = useState<Inventory | null>(null)

  useEffect(() => {
    let cancelled = false
    api
      .planUpdate(projectId, changes)
      .then((p) => {
        if (cancelled) return
        setPlan(p)
        setPhase({ name: 'review' })
      })
      .catch((e) => !cancelled && setPhase({ name: 'plan-failed', error: String(e) }))
    return () => {
      cancelled = true
    }
  }, [projectId, changes])

  useEffect(() => {
    const unlisten = api.onUpdateEvent((e) => {
      if (e.state === 'rolled-back') return
      setStepStates((prev) => {
        const next = [...prev]
        next[e.index] = e.state === 'running' ? 'running' : e.state === 'ok' ? 'ok' : 'failed'
        return next
      })
    })
    return () => {
      unlisten.then((fn) => fn())
    }
  }, [])

  const busy = phase.name === 'planning' || phase.name === 'applying'
  const close = () => !busy && onClose(refreshed)

  useEffect(() => {
    const onKey = (e: KeyboardEvent) => e.key === 'Escape' && close()
    window.addEventListener('keydown', onKey)
    return () => window.removeEventListener('keydown', onKey)
  })

  const apply = async () => {
    if (!plan) return
    setStepStates(plan.steps.map((s) => (s.kind === 'verify' && !verify ? 'skipped' : 'waiting')))
    setPhase({ name: 'applying' })
    try {
      const result = await api.applyUpdate(plan, verify)
      setRefreshed(result.inventory)
      setPhase({ name: 'done', outcome: result.outcome })
    } catch (e) {
      setPhase({ name: 'done', outcome: { ok: false, rolledBack: false, error: String(e), steps: [] } })
    }
  }

  const hasVerify = plan?.steps.some((s) => s.kind === 'verify') ?? false
  const restoredFiles = plan ? plan.edits.length + plan.snapshots.length : 0

  return (
    <div className="fixed inset-0 z-50 flex items-center justify-center bg-black/60 p-6" onClick={close}>
      <div
        role="dialog"
        aria-label="Update packages"
        className="flex max-h-full w-[min(920px,100%)] flex-col overflow-hidden rounded-2xl border border-line-strong bg-bg shadow-2xl"
        onClick={(e) => e.stopPropagation()}
      >
        <header className="flex items-center gap-3 border-b border-line px-5 py-3">
          <div className="min-w-0 flex-1">
            <h2 className="text-[15px] font-semibold">
              Update {changes.length} package{changes.length === 1 ? '' : 's'}
              {plan && <span className="font-normal text-muted"> in {plan.projectName}</span>}
            </h2>
          </div>
          <button type="button" onClick={close} disabled={busy} aria-label="Close" className="rounded-md p-1 text-dim hover:bg-hover hover:text-ink disabled:opacity-40">
            <X size={16} />
          </button>
        </header>

        <div className="min-h-0 flex-1 overflow-y-auto px-5 py-4">
          {phase.name === 'planning' && (
            <div className="flex items-center gap-2 py-10 text-muted">
              <Loader2 size={16} className="animate-spin" /> Working out the changes…
            </div>
          )}

          {phase.name === 'plan-failed' && (
            <div className="flex gap-2 rounded-lg border border-[#5c2a1d] bg-carnelian-soft px-4 py-3 text-carnelian">
              <CircleAlert size={16} className="mt-0.5 shrink-0" />
              <div>
                <div className="font-medium">Mehen can't make this change automatically</div>
                <div className="mt-1 text-[12.5px]">{phase.error}</div>
              </div>
            </div>
          )}

          {plan && phase.name !== 'plan-failed' && (
            <div className="flex flex-col gap-5">
              <section>
                <h3 className="mb-2 text-[11.5px] font-medium uppercase tracking-wider text-dim">Changes</h3>
                <ul className="flex flex-col gap-1">
                  {plan.changes.map((c) => (
                    <li key={`${c.name}-${c.writtenBefore}`} className="flex flex-wrap items-center gap-2 rounded-lg border border-line bg-panel px-3 py-2">
                      <span className="font-medium">{c.name}</span>
                      <span className="font-mono text-[12px] text-muted">{c.from}</span>
                      <ArrowRight size={13} className="text-dim" />
                      <span className="font-mono text-[12px] text-turq">{c.to}</span>
                      {c.writtenBefore === c.writtenAfter && (
                        <span className="text-[11.5px] text-dim">already allowed by {c.writtenBefore}; only the lockfile moves</span>
                      )}
                    </li>
                  ))}
                </ul>
              </section>

              {plan.edits.length > 0 && (
                <section>
                  <h3 className="mb-2 text-[11.5px] font-medium uppercase tracking-wider text-dim">File changes</h3>
                  <div className="flex flex-col gap-2">
                    {plan.edits.map((e) => (
                      <Diff key={e.path} diff={e.diff} />
                    ))}
                  </div>
                </section>
              )}

              {plan.steps.length > 0 && (
                <section>
                  <h3 className="mb-2 text-[11.5px] font-medium uppercase tracking-wider text-dim">Then Mehen runs</h3>
                  <ol className="flex flex-col gap-1">
                    {plan.steps.map((s, i) => {
                      const state = stepStates[i] ?? (s.kind === 'verify' && !verify ? 'skipped' : 'waiting')
                      return (
                        <li key={i} className={cx('flex items-center gap-2.5 rounded-lg border border-line px-3 py-2', state === 'skipped' && 'opacity-45')}>
                          <StepIcon state={phase.name === 'review' ? 'waiting' : state} />
                          <span className="font-mono text-[12px]">
                            {s.program} {s.args.join(' ')}
                          </span>
                          <span className="ml-auto shrink-0 text-[11px] text-dim">
                            {s.kind === 'install' ? 'updates the lockfile' : 'checks the build'} · {relativePath(roots, s.cwd)}
                          </span>
                        </li>
                      )
                    })}
                  </ol>
                  {hasVerify && phase.name === 'review' && (
                    <label className="mt-2 flex items-center gap-2 text-[12.5px] text-muted">
                      <input type="checkbox" checked={verify} onChange={(e) => setVerify(e.target.checked)} className="size-3.5 accent-[var(--color-gold)]" />
                      Check the build after installing (recommended; slower)
                    </label>
                  )}
                </section>
              )}

              {plan.warnings.length > 0 && (
                <ul className="flex flex-col gap-1">
                  {plan.warnings.map((w) => (
                    <li key={w} className="flex gap-2 text-[12.5px] text-amber">
                      <TriangleAlert size={14} className="mt-0.5 shrink-0" />
                      {w}
                    </li>
                  ))}
                </ul>
              )}

              {phase.name === 'done' && <Result outcome={phase.outcome} restoredFiles={restoredFiles} />}
            </div>
          )}
        </div>

        <footer className="flex items-center gap-3 border-t border-line px-5 py-3">
          <span className="flex-1 text-[12px] text-dim">
            {phase.name === 'review' &&
              plan &&
              plan.steps.length > 0 &&
              `If any step fails, the ${restoredFiles} file${restoredFiles === 1 ? '' : 's'} involved (manifest and lockfile) are put back exactly as they were.`}
            {phase.name === 'applying' && 'Running… this can take a minute for large installs.'}
          </span>
          {phase.name === 'review' ? (
            <>
              <button type="button" onClick={close} className="rounded-lg border border-line-strong px-3.5 py-1.5 text-[12.5px] text-muted hover:text-ink">
                Cancel
              </button>
              <button type="button" onClick={apply} className="rounded-lg bg-gold px-4 py-1.5 text-[12.5px] font-semibold text-[#1d1506] hover:brightness-110">
                Apply update
              </button>
            </>
          ) : (
            <button
              type="button"
              onClick={close}
              disabled={busy}
              className="rounded-lg border border-line-strong px-3.5 py-1.5 text-[12.5px] text-muted hover:text-ink disabled:opacity-40"
            >
              Close
            </button>
          )}
        </footer>
      </div>
    </div>
  )
}

function StepIcon({ state }: { state: StepState }) {
  if (state === 'running') return <Loader2 size={14} className="shrink-0 animate-spin text-gold" />
  if (state === 'ok') return <Check size={14} className="shrink-0 text-turq" />
  if (state === 'failed') return <X size={14} className="shrink-0 text-carnelian" />
  return <span className="size-3.5 shrink-0 rounded-full border border-line-strong" />
}

function Diff({ diff }: { diff: string }) {
  const lines = diff.replace(/\n$/, '').split('\n')
  return (
    <pre className="overflow-x-auto rounded-lg border border-line bg-panel py-2 font-mono text-[11.5px] leading-[1.55]">
      {lines.map((line, i) => (
        <div
          key={i}
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
  )
}

function Result({ outcome, restoredFiles }: { outcome: UpdateOutcome; restoredFiles: number }) {
  const failed = outcome.steps.find((s) => !s.ok)
  if (outcome.ok) {
    return (
      <div className="flex items-center gap-2 rounded-lg border border-[#1f4c42] bg-turq-soft px-4 py-3 text-turq">
        <Check size={16} />
        <span>Updated. The results have been refreshed with the new versions.</span>
      </div>
    )
  }
  return (
    <div className="rounded-lg border border-[#5c2a1d] bg-carnelian-soft px-4 py-3 text-carnelian">
      <div className="flex items-center gap-2 font-medium">
        {outcome.rolledBack ? <RotateCcw size={15} /> : <CircleAlert size={15} />}
        {outcome.error ?? 'The update failed'}
      </div>
      {outcome.rolledBack && (
        <div className="mt-1 text-[12.5px]">
          Nothing was kept: the {restoredFiles} file{restoredFiles === 1 ? ' was' : 's were'} restored. If an install ran, run it again to reset node_modules.
        </div>
      )}
      {failed && <Output step={failed} />}
    </div>
  )
}

function Output({ step }: { step: StepResult }) {
  const [open, setOpen] = useState(true)
  return (
    <div className="mt-2">
      <button type="button" onClick={() => setOpen(!open)} className="inline-flex items-center gap-1 text-[12px] text-muted hover:text-ink">
        <ChevronRight size={13} className={cx('transition-transform', open && 'rotate-90')} />
        Output of {step.label}
      </button>
      {open && <pre className="mt-1 max-h-72 overflow-auto rounded-md bg-bg/80 p-2 font-mono text-[11px] leading-relaxed text-muted">{step.output || '(no output)'}</pre>}
    </div>
  )
}
