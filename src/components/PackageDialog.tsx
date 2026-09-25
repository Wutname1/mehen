import { Check, ChevronRight, Link2, Pin, ShieldAlert } from 'lucide-react'
import { useEffect, useRef, useState } from 'react'
import * as api from '../api'
import { ECOSYSTEM_LABEL, POLICY_LABEL, compareVersions, displayVersion, reasonText, repoKey, samePath, type QueueRow, type QueueUsage } from '../derive'
import type { Hold, Move, Project, Requirement, VersionPolicy, VersionView } from '../types'
import { cx } from './bits'
import { Select } from './controls'
import { Button, Dialog } from './Dialog'
import { EcoIcon } from './EcoIcon'

/** `22` for 22.1.0, `0.15` for 0.15.1: the line inside which updates should not break anything. */
const lineOf = (version: string) => {
  const [major, minor] = version.replace(/^[^\d]+/, '').split(/[.-]/)
  return major === '0' && minor ? `0.${minor}` : major
}

const DAY = 86_400_000

function ago(iso: string): string {
  const days = Math.floor((Date.now() - Date.parse(iso)) / DAY)
  if (Number.isNaN(days)) return ''
  if (days < 1) return 'today'
  if (days < 2) return 'yesterday'
  if (days < 45) return `${days} days ago`
  if (days < 548) return `${Math.round(days / 30.4)} months ago`
  return `${Math.round(days / 365)} years ago`
}

function requirementLines(r: Requirement): string[] {
  switch (r.kind) {
    case 'peers':
      return r.peers.map(([name, range]) => `${name} ${range}`)
    case 'node':
      return [`Node ${r.range}`]
    case 'rust':
      return [`Rust ${r.version} or newer`]
    case 'frameworks':
      return [`Built for ${r.frameworks.join(', ')}`]
    case 'python':
      return [`Python ${r.range}`]
    case 'dart':
      return [`Dart SDK ${r.range}`]
    case 'php':
      return [`PHP ${r.range}`]
    case 'ruby':
      return [`Ruby ${r.range}`]
  }
}

/** True when the project's update level would not go as far as `version` on its own. */
function pastLevel(level: VersionPolicy, current: string, version: string): boolean {
  if (level === 'any') return false
  const [c, v] = [current, version].map((x) => x.replace(/^[^\d]+/, '').split(/[.-]/).map((p) => Number.parseInt(p, 10) || 0))
  if (level === 'minor') return lineOf(current) !== lineOf(version)
  return c[0] !== v[0] || c[1] !== v[1]
}

interface Pending {
  version: string
  moves?: Move[]
  error?: string
}

/**
 * Everything about one package in one project: every release, when it came
 * out, whether the project can take it and what it asks for. Any version the
 * project can use can be picked, and whatever has to move with it comes too.
 */
export function PackageDialog({
  row,
  usages,
  holds,
  levelOf,
  nameOf,
  chosenTarget,
  onChoose,
  onAdvisory,
  onClose,
}: {
  row: QueueRow
  usages: QueueUsage[]
  holds: Hold[]
  levelOf: (project: Project) => VersionPolicy
  nameOf: (folder: string) => string
  /** Where a usage is set to go if it is selected, else null. */
  chosenTarget: (usage: QueueUsage) => string | null
  onChoose: (project: Project, moves: Move[]) => void
  onAdvisory: () => void
  onClose: () => void
}) {
  const [usageKey, setUsageKey] = useState(usages[0].key)
  const usage = usages.find((u) => u.key === usageKey) ?? usages[0]
  const project = usage.project
  const current = displayVersion(usage.dep)
  const level = levelOf(project)
  const hold = holds.find((h) => h.ecosystem === row.ecosystem && h.name === row.name && (h.scope === '*' || samePath(h.scope, repoKey(project))))
  const picked = chosenTarget(usage)

  const [versions, setVersions] = useState<VersionView[] | null>(null)
  const [error, setError] = useState<string | null>(null)
  const [dates, setDates] = useState<Record<string, string>>({})
  const [showPre, setShowPre] = useState(false)
  const [showOlder, setShowOlder] = useState(false)
  const [open, setOpen] = useState<Set<string>>(new Set())
  const [needs, setNeeds] = useState<string | null>(null)
  const [pending, setPending] = useState<Pending | null>(null)
  const top = useRef<HTMLDivElement>(null)

  useEffect(() => {
    let cancelled = false
    setVersions(null)
    setError(null)
    setPending(null)
    api
      .packageVersions(project.id, row.name)
      .then((v) => !cancelled && setVersions(v))
      .catch((e) => !cancelled && setError(String(e)))
    return () => {
      cancelled = true
    }
  }, [project.id, row.name])

  useEffect(() => {
    let cancelled = false
    api
      .releaseDates(row.ecosystem, row.name)
      .then((d) => !cancelled && setDates(d))
      .catch(() => {})
    return () => {
      cancelled = true
    }
  }, [row.ecosystem, row.name])

  const choose = async (version: string) => {
    setPending({ version })
    top.current?.scrollIntoView({ block: 'nearest' })
    try {
      const moves = await api.moveWith(project.id, row.name, version)
      setPending({ version, moves })
    } catch (e) {
      setPending({ version, error: String(e) })
    }
  }

  const shown = (versions ?? []).filter((v) => (showPre || !v.prerelease || v.version === current) && (showOlder || compareVersions(v.version, current) >= 0))
  const lines: { line: string; versions: VersionView[] }[] = []
  for (const v of shown) {
    const line = lineOf(v.version)
    const last = lines.at(-1)
    if (last?.line === line) last.versions.push(v)
    else lines.push({ line, versions: [v] })
  }
  const newest = versions?.find((v) => !v.prerelease)
  const hidden = (versions ?? []).filter((v) => v.prerelease && v.version !== current && compareVersions(v.version, current) > 0).length

  const versionRow = (v: VersionView, head: { line: string; count: number } | null) => {
    const newer = compareVersions(v.version, current) > 0
    const reqs = v.requirements.flatMap(requirementLines)
    const date = dates[v.version]
    const expanded = head && open.has(head.line)
    return (
      <div key={v.version} className={cx('border-b border-line', !head && 'bg-paper-2')}>
        <div className="grid min-h-[42px] grid-cols-[22px_minmax(90px,150px)_110px_minmax(0,1fr)_auto] items-center gap-x-3 pr-2">
          {head && head.count > 1 ? (
            <button
              type="button"
              onClick={() => setOpen((prev) => (prev.has(head.line) ? new Set([...prev].filter((l) => l !== head.line)) : new Set([...prev, head.line])))}
              aria-expanded={!!expanded}
              aria-label={`${expanded ? 'Hide' : 'Show'} every ${head.line}.x release`}
              className="grid size-[22px] place-items-center rounded-[3px] text-muted hover:bg-sunken hover:text-ink"
            >
              <ChevronRight size={14} className={cx('transition-transform', expanded && 'rotate-90')} />
            </button>
          ) : (
            <span />
          )}
          <span className="flex min-w-0 flex-col">
            <code className={cx('truncate font-mono text-[13px]', head && 'font-semibold')}>{v.version}</code>
            {head && head.count > 1 && <small className="text-[11.5px] text-muted">newest of {head.count} on {head.line}.x</small>}
          </span>
          <span className="text-[12px] text-muted" title={date ? new Date(date).toLocaleString() : undefined}>
            {date ? ago(date) : ''}
          </span>
          <span className="flex min-w-0 flex-wrap items-center gap-x-2 gap-y-0.5 text-[12px]">
            {v.version === current && <Tag tone="ink">Installed</Tag>}
            {picked === v.version && <Tag tone="state">Selected</Tag>}
            {picked !== v.version && v.version === usage.target && !usage.chosen && <Tag tone="accent">Suggested</Tag>}
            {v.prerelease && <Tag tone="muted">Pre-release</Tag>}
            {newer && v.together > 0 && (
              <span className="inline-flex items-center gap-1 text-state" title="Other packages in this project have to move to matching versions; choosing it shows which">
                <Link2 size={12} />
                moves with {v.together} other{v.together === 1 ? '' : 's'}
              </span>
            )}
            {v.blocked ? (
              <span className="text-risk-review">{reasonText(v.blocked)}</span>
            ) : (
              newer && pastLevel(level, current, v.version) && <span className="text-muted">Past your update level</span>
            )}
            {reqs.length > 0 && (
              <button type="button" onClick={() => setNeeds(needs === v.version ? null : v.version)} aria-expanded={needs === v.version} className="text-muted underline decoration-dotted underline-offset-2 hover:text-ink">
                Needs {reqs.length === 1 ? reqs[0] : `${reqs.length} things`}
              </button>
            )}
          </span>
          {newer ? (
            <Button onClick={() => choose(v.version)} disabled={!!v.blocked || picked === v.version} title={v.blocked ? reasonText(v.blocked) : undefined} className="h-7 px-2.5 text-[12px]">
              {picked === v.version ? 'Selected' : 'Choose'}
            </Button>
          ) : (
            <span />
          )}
        </div>
        {needs === v.version && reqs.length > 1 && (
          <ul className="m-0 mb-2 ml-[34px] grid list-none gap-0.5 p-0 font-mono text-[12px] text-muted">
            {reqs.map((r) => (
              <li key={r}>{r}</li>
            ))}
          </ul>
        )}
      </div>
    )
  }

  return (
    <Dialog
      title={row.name}
      description={`${ECOSYSTEM_LABEL[row.ecosystem]} package. Pick any version this project can use; anything that has to move with it comes along.`}
      icon={<EcoIcon ecosystem={row.ecosystem} size={22} />}
      size="wide"
      onClose={onClose}
      footer={
        <>
          {row.vulnIds.length > 0 && (
            <Button onClick={onAdvisory} className="mr-auto text-risk-security">
              <ShieldAlert size={15} />
              {row.vulnIds.length} advisor{row.vulnIds.length === 1 ? 'y' : 'ies'}
            </Button>
          )}
          <Button onClick={onClose}>Close</Button>
        </>
      }
    >
      <div ref={top} className="grid gap-3">
        <div className="flex flex-wrap items-end gap-x-6 gap-y-2">
          {usages.length > 1 && (
            <label className="grid gap-1 text-[11px] text-muted">
              Project
              <Select value={usage.key} onChange={(e) => setUsageKey(e.target.value)} className="w-[260px]" aria-label="Project">
                {usages.map((u) => (
                  <option key={u.key} value={u.key}>
                    {nameOf(repoKey(u.project))}
                    {u.project.name !== nameOf(repoKey(u.project)) ? ` (${u.project.name})` : ''} · {displayVersion(u.dep)}
                  </option>
                ))}
              </Select>
            </label>
          )}
          <Fact label="Installed" value={current} />
          <Fact label="Update goes to" value={picked ?? usage.target} note={picked && picked !== usage.target ? 'your pick' : undefined} />
          {newest && newest.version !== (picked ?? usage.target) && <Fact label="Newest" value={newest.version} note={dates[newest.version] ? ago(dates[newest.version]) : undefined} />}
          <span className="grid gap-1 text-[11px] text-muted">
            Updates
            <span className="inline-flex items-center gap-1 text-[12.5px] text-ink">
              {hold ? (
                <>
                  <Pin size={13} className="text-state" />
                  Staying on {hold.line}.x{hold.scope === '*' ? '' : ` in ${nameOf(hold.scope)}`}
                </>
              ) : (
                POLICY_LABEL[level]
              )}
            </span>
          </span>
        </div>

        {pending && (
          <div className="rounded-[3px] border border-state bg-[color-mix(in_oklab,var(--state)_7%,transparent)] px-3 py-2.5" role="status">
            {!pending.moves && !pending.error && <p className="m-0 text-[12.5px] text-muted">Working out what else has to move for {pending.version}…</p>}
            {pending.error && (
              <div className="flex items-start gap-3">
                <p className="m-0 flex-1 text-[12.5px] text-risk-review">{pending.error}</p>
                <Button onClick={() => setPending(null)} className="h-7 px-2.5 text-[12px]">
                  OK
                </Button>
              </div>
            )}
            {pending.moves && (
              <>
                <b className="block text-[13px]">
                  {row.name} {pending.version}
                  {pending.moves.length > 1 ? ` moves ${pending.moves.length - 1} other package${pending.moves.length === 2 ? '' : 's'} with it` : ' moves on its own'}
                </b>
                {pending.moves.length > 1 && (
                  <ul className="m-0 mt-1.5 grid max-h-[140px] list-none grid-cols-[repeat(auto-fill,minmax(250px,1fr))] gap-x-4 gap-y-0.5 overflow-y-auto p-0 font-mono text-[12px]">
                    {pending.moves
                      .filter((m) => m.name !== row.name)
                      .map((m) => (
                        <li key={m.name} className="truncate" title={`${m.name} ${m.from} to ${m.to}`}>
                          {m.name} <span className="text-muted">{m.from} →</span> {m.to}
                        </li>
                      ))}
                  </ul>
                )}
                <div className="mt-2.5 flex justify-end gap-2">
                  <Button onClick={() => setPending(null)} className="h-8">
                    Cancel
                  </Button>
                  <Button
                    variant="primary"
                    className="h-8"
                    onClick={() => {
                      onChoose(project, pending.moves!)
                      setPending(null)
                    }}
                  >
                    <Check size={15} />
                    Select {pending.moves.length === 1 ? 'this update' : `these ${pending.moves.length} updates`}
                  </Button>
                </div>
              </>
            )}
          </div>
        )}

        <div className="flex items-center gap-4 text-[12px] text-muted">
          <label className="inline-flex cursor-pointer items-center gap-1.5">
            <input type="checkbox" checked={showOlder} onChange={(e) => setShowOlder(e.target.checked)} className="accent-[var(--state)]" />
            Older versions
          </label>
          <label className="inline-flex cursor-pointer items-center gap-1.5">
            <input type="checkbox" checked={showPre} onChange={(e) => setShowPre(e.target.checked)} className="accent-[var(--state)]" />
            Pre-releases{hidden > 0 && !showPre ? ` (${hidden})` : ''}
          </label>
        </div>

        {error ? (
          <p className="m-0 text-[12.5px] text-risk-review">{error}</p>
        ) : !versions ? (
          <p className="m-0 text-[12.5px] text-muted">Loading versions…</p>
        ) : (
          <div className="border-t border-line-strong" role="list" aria-label={`${row.name} versions`}>
            {lines.map(({ line, versions: list }) => (
              <div key={line} role="listitem">
                {versionRow(list[0], { line, count: list.length })}
                {open.has(line) && list.slice(1).map((v) => versionRow(v, null))}
              </div>
            ))}
          </div>
        )}
      </div>
    </Dialog>
  )
}

function Fact({ label, value, note }: { label: string; value: string; note?: string }) {
  return (
    <span className="grid gap-1 text-[11px] text-muted">
      {label}
      <span className="text-[12.5px] text-ink">
        <code className="font-mono font-semibold">{value}</code>
        {note && <span className="ml-1.5 text-muted">{note}</span>}
      </span>
    </span>
  )
}

function Tag({ tone, children }: { tone: 'ink' | 'state' | 'accent' | 'muted'; children: string }) {
  return (
    <span
      className={cx(
        'inline-flex h-[18px] items-center rounded-[2px] border px-1.5 font-mono text-[10.5px] font-semibold tracking-[0.04em] uppercase',
        tone === 'ink' ? 'text-ink' : tone === 'state' ? 'text-state' : tone === 'accent' ? 'text-accent-text' : 'text-muted',
      )}
      style={{ borderColor: 'color-mix(in oklab, currentColor 40%, transparent)' }}
    >
      {children}
    </span>
  )
}
