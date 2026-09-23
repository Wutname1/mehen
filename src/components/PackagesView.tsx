import { ArrowUpCircle, ChevronRight, Code2, EyeOff, FolderOpen, ShieldAlert } from 'lucide-react'
import { useEffect, useMemo, useState } from 'react'
import type { IgnoreRequest } from '../App'
import { openInEditor, reveal } from '../api'
import { canRetarget, displayVersion, isBehind, relativePath, type PackageGroup, type Usage } from '../derive'
import type { Inventory } from '../types'
import { EcoBadge, Empty, IconButton, StatusPill, VersionChip, cx } from './bits'
import { BulkUpdateDialog, type BulkTarget } from './BulkUpdateDialog'

const MAX_CHIPS = 4

export function PackagesView({
  groups,
  roots,
  onIgnore,
  onInventory,
}: {
  groups: PackageGroup[]
  roots: string[]
  onIgnore: (r: IgnoreRequest) => void
  onInventory: (inv: Inventory) => void
}) {
  const [open, setOpen] = useState<string | null>(null)

  if (groups.length === 0) {
    return <Empty title="Nothing matches">Try clearing the search or turning off a filter.</Empty>
  }

  return (
    <div className="min-w-[860px]">
      <div className="sticky top-0 z-10 grid grid-cols-[minmax(240px,1.3fr)_minmax(260px,2fr)_120px_84px_84px] items-center gap-4 border-b border-line bg-bg/95 px-5 py-2 text-[11px] font-medium uppercase tracking-wider text-dim backdrop-blur">
        <div>Package</div>
        <div>Versions in use</div>
        <div>Latest</div>
        <div className="text-right">Projects</div>
        <div className="text-right">Security</div>
      </div>
      {groups.map((g) => {
        const expanded = open === g.key
        const hidden = g.versions.length - MAX_CHIPS
        return (
          <div key={g.key} className={cx('border-b border-line/70', expanded && 'bg-panel')}>
            <button
              type="button"
              onClick={() => setOpen(expanded ? null : g.key)}
              aria-expanded={expanded}
              className="grid w-full grid-cols-[minmax(240px,1.3fr)_minmax(260px,2fr)_120px_84px_84px] items-center gap-4 px-5 py-2.5 text-left transition-colors hover:bg-hover/60"
            >
              <div className="flex min-w-0 items-center gap-2">
                <ChevronRight size={14} className={cx('shrink-0 text-dim transition-transform', expanded && 'rotate-90')} />
                <EcoBadge ecosystem={g.ecosystem} />
                <span className="truncate font-medium" title={g.name}>
                  {g.name}
                </span>
              </div>
              <div className="flex flex-wrap items-center gap-1.5">
                {g.versions.slice(0, MAX_CHIPS).map((v) => (
                  <VersionChip key={v.version} version={v.version} status={v.status} approximate={v.approximate} count={v.usages.length} />
                ))}
                {hidden > 0 && <span className="text-[11px] text-dim">+{hidden} more</span>}
                {g.versions.length > 1 && (
                  <span className="ml-1 text-[11px] text-gold" title="Different projects use different versions">
                    {g.versions.length} versions
                  </span>
                )}
              </div>
              <div className="font-mono text-[12px] text-muted">{g.latest ?? '?'}</div>
              <div className="text-right text-muted">{new Set(g.usages.map((u) => u.project.id)).size}</div>
              <div className="flex justify-end">
                {g.vulnIds.length > 0 ? (
                  <span className="inline-flex items-center gap-1 text-[12px] font-medium text-carnelian">
                    <ShieldAlert size={13} />
                    {g.vulnIds.length}
                  </span>
                ) : (
                  <span className="text-dim">-</span>
                )}
              </div>
            </button>
            {expanded && <UsageList group={g} roots={roots} onIgnore={onIgnore} onInventory={onInventory} />}
          </div>
        )
      })}
    </div>
  )
}

function UsageList({
  group,
  roots,
  onIgnore,
  onInventory,
}: {
  group: PackageGroup
  roots: string[]
  onIgnore: (r: IgnoreRequest) => void
  onInventory: (inv: Inventory) => void
}) {
  const usages = useMemo(() => [...group.usages].sort((a, b) => a.project.dir.localeCompare(b.project.dir)), [group])
  // Targets: the latest release, or any version already in use, so drifted
  // projects can be lined up without necessarily taking something new.
  const targets = useMemo(() => {
    const exact = (v: string) => v.replace(/^v/i, '').split('.').length >= 2
    const inUse = group.versions
      .map((v) => v.version)
      .filter((v) => exact(v) && (!group.latest || isBehind(v, group.latest)))
      .filter((v) => group.usages.some((u) => canRetarget(u.dep) && isBehind(u.dep.current!, v)))
    return [...new Set([group.latest, ...inUse].filter((v): v is string => !!v))]
  }, [group])
  const [target, setTarget] = useState<string | null>(targets[0] ?? null)
  // A project can only move as far as the newest version it can use.
  const outOfReach = (u: Usage) => !!target && !!u.dep.newest && !!u.dep.latest && isBehind(u.dep.latest, target)
  const movable = (u: Usage) => !!target && canRetarget(u.dep) && isBehind(u.dep.current!, target) && !outOfReach(u)
  const usageKey = (u: Usage) => `${u.project.id}|${u.dep.requested}`
  const [picked, setPicked] = useState<Set<string>>(() => new Set(usages.filter(movable).map(usageKey)))
  const [bulk, setBulk] = useState<BulkTarget[] | null>(null)

  useEffect(() => {
    setPicked(new Set(usages.filter(movable).map(usageKey)))
  }, [target, usages])

  const togglePick = (u: Usage) =>
    setPicked((prev) => {
      const next = new Set(prev)
      if (next.has(usageKey(u))) next.delete(usageKey(u))
      else next.add(usageKey(u))
      return next
    })

  const review = () => {
    if (!target) return
    const byProject = new Map<string, BulkTarget>()
    for (const u of usages.filter((u) => movable(u) && picked.has(usageKey(u)))) {
      const entry = byProject.get(u.project.id) ?? { project: u.project, changes: [] }
      if (!entry.changes.some((c) => c.from === u.dep.requested)) entry.changes.push({ name: u.dep.name, from: u.dep.requested, to: target })
      byProject.set(u.project.id, entry)
    }
    setBulk([...byProject.values()])
  }

  const movableCount = usages.filter(movable).length
  const pickedCount = usages.filter((u) => movable(u) && picked.has(usageKey(u))).length

  return (
    <div className="pb-3 pl-[46px] pr-5">
      {targets.length > 0 && movableCount > 0 && (
        <div className="mb-2 flex flex-wrap items-center gap-2 text-[12.5px]">
          <span className="text-muted">Bring to</span>
          <select
            value={target ?? ''}
            onChange={(e) => setTarget(e.target.value)}
            aria-label={`Target version for ${group.name}`}
            className="rounded-md border border-line-strong bg-raised px-1.5 py-0.5 font-mono text-[12px]"
          >
            {targets.map((v) => (
              <option key={v} value={v}>
                {v}
                {v === group.latest ? ' (latest)' : ' (already in use)'}
              </option>
            ))}
          </select>
          <span className="text-dim">
            {pickedCount} of {movableCount} behind
          </span>
          <button
            type="button"
            onClick={review}
            disabled={pickedCount === 0}
            className="ml-auto inline-flex items-center gap-1.5 rounded-lg bg-gold px-3 py-1 font-semibold text-[#1d1506] hover:brightness-110 disabled:opacity-40"
          >
            <ArrowUpCircle size={14} />
            Review {pickedCount} update{pickedCount === 1 ? '' : 's'}
          </button>
        </div>
      )}
      <div className="overflow-hidden rounded-lg border border-line bg-bg/60">
        {usages.map((u, i) => {
          const { project, dep } = u
          const canMove = movable(u)
          return (
            <div
              key={`${project.id}-${i}`}
              className="grid grid-cols-[20px_minmax(200px,1.4fr)_minmax(110px,0.8fr)_minmax(130px,1fr)_120px_auto] items-center gap-4 border-b border-line/60 px-3 py-1.5 last:border-b-0"
            >
              <div>
                {canMove && (
                  <input
                    type="checkbox"
                    checked={picked.has(usageKey(u))}
                    onChange={() => togglePick(u)}
                    aria-label={`Update ${project.name}`}
                    className="size-3.5 accent-[var(--color-gold)]"
                  />
                )}
              </div>
              <div className="min-w-0">
                <div className="truncate text-ink">{project.name}</div>
                <div className="truncate text-[11px] text-dim" title={project.dir}>
                  {relativePath(roots, project.dir)}
                </div>
              </div>
              <div className="truncate font-mono text-[11.5px] text-dim" title="As written in the manifest">
                {dep.requested || '-'}
              </div>
              <div className="min-w-0">
                <span className="font-mono text-[12px]">
                  {dep.approximate && <span className="text-dim">~</span>}
                  {displayVersion(dep)}
                </span>
                <div className="truncate text-[10.5px] text-dim">{dep.installedFrom ?? (dep.approximate ? 'from version range' : (dep.note ?? ''))}</div>
                {outOfReach(u) && (
                  <div className="truncate text-[10.5px] text-amber" title={`${target} ${dep.blockedReason}`}>
                    can't use {target}: {dep.blockedReason}; newest usable {dep.latest}
                  </div>
                )}
              </div>
              <div className="flex items-center gap-1.5">
                <StatusPill status={dep.status} />
                {dep.vulns.length > 0 && (
                  <span className="inline-flex items-center gap-0.5 text-[11px] text-carnelian" title={dep.vulns.join(', ')}>
                    <ShieldAlert size={12} />
                    {dep.vulns.length}
                  </span>
                )}
              </div>
              <div className="flex justify-end gap-0.5">
                <IconButton title="Show manifest in Explorer" onClick={() => reveal(project.manifest)}>
                  <FolderOpen size={14} />
                </IconButton>
                <IconButton title="Open project in VS Code" onClick={() => openInEditor(project.repo ?? project.dir)}>
                  <Code2 size={14} />
                </IconButton>
                <IconButton title="Ignore this project from now on" onClick={() => onIgnore({ kind: 'project', value: project.manifest, label: project.name })}>
                  <EyeOff size={14} />
                </IconButton>
              </div>
            </div>
          )
        })}
      </div>
      {bulk && target && (
        <BulkUpdateDialog
          packageName={group.name}
          ecosystem={group.ecosystem}
          to={target}
          targets={bulk}
          roots={roots}
          onClose={(refreshed) => {
            setBulk(null)
            if (refreshed) onInventory(refreshed)
          }}
        />
      )}
    </div>
  )
}
