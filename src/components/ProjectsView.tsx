import { ArrowUpCircle, Code2, EyeOff, FolderMinus, FolderOpen, ShieldAlert } from 'lucide-react'
import { useEffect, useMemo, useState } from 'react'
import type { IgnoreRequest } from '../App'
import { openInEditor, reveal } from '../api'
import { displayVersion, isOutdated, relativePath, worstStatus } from '../derive'
import type { Change, Dependency, Inventory, Project } from '../types'
import { EcoBadge, Empty, IconButton, StatusPill, cx } from './bits'
import { UpdateDialog } from './UpdateDialog'

const STATUS_ORDER = ['major', 'minor', 'patch', 'unknown', 'unpinned', 'pending', 'up-to-date', 'local']

export function ProjectsView({
  projects,
  roots,
  depVisible,
  onIgnore,
  onInventory,
}: {
  projects: Project[]
  roots: string[]
  depVisible: (d: Dependency) => boolean
  onIgnore: (r: IgnoreRequest) => void
  onInventory: (inv: Inventory) => void
}) {
  const [selectedId, setSelectedId] = useState<string | null>(null)
  const selected = projects.find((p) => p.id === selectedId) ?? projects[0]

  useEffect(() => {
    if (selected && selected.id !== selectedId) setSelectedId(selected.id)
  }, [selected, selectedId])

  if (projects.length === 0) {
    return <Empty title="Nothing matches">Try clearing the search or turning off a filter.</Empty>
  }

  return (
    <div className="grid h-full grid-cols-[320px_1fr]">
      <nav className="overflow-y-auto border-r border-line" aria-label="Projects">
        {projects.map((p) => {
          const outdated = p.dependencies.filter((d) => isOutdated(d.status)).length
          const vulns = p.dependencies.reduce((n, d) => n + d.vulns.length, 0)
          const worst = worstStatus(p.dependencies.map((d) => d.status))
          return (
            <button
              key={p.id}
              type="button"
              onClick={() => setSelectedId(p.id)}
              className={cx(
                'block w-full border-b border-line/60 px-4 py-2.5 text-left transition-colors hover:bg-hover/60',
                p.id === selected?.id && 'bg-raised shadow-[inset_2px_0_0_var(--color-gold)]',
              )}
            >
              <div className="flex items-center gap-2">
                <span className="min-w-0 flex-1 truncate font-medium">{p.name}</span>
                {vulns > 0 && (
                  <span className="inline-flex items-center gap-0.5 text-[11px] text-carnelian">
                    <ShieldAlert size={12} />
                    {vulns}
                  </span>
                )}
                {outdated > 0 && (
                  <span className={cx('text-[11px]', worst === 'major' ? 'text-carnelian' : 'text-amber')}>{outdated} behind</span>
                )}
              </div>
              <div className="mt-0.5 flex items-center gap-2">
                <EcoBadge ecosystem={p.ecosystem} />
                <span className="truncate text-[11px] text-dim">{relativePath(roots, p.dir)}</span>
              </div>
            </button>
          )
        })}
      </nav>
      {selected && <ProjectDetail project={selected} roots={roots} depVisible={depVisible} onIgnore={onIgnore} onInventory={onInventory} />}
    </div>
  )
}

type Target = 'safe' | 'latest'

const updatable = (d: Dependency) => (d.status === 'major' || d.status === 'minor' || d.status === 'patch') && !!d.latest
const depKey = (d: Dependency) => `${d.name}@${d.requested}`
/** Major jumps default to the newest release on the current line when there is one. */
const defaultTarget = (d: Dependency): Target => (d.status === 'major' && d.safeLatest ? 'safe' : 'latest')
const targetVersion = (d: Dependency, t: Target) => (t === 'safe' && d.safeLatest ? d.safeLatest : d.latest!)

function ProjectDetail({
  project,
  roots,
  depVisible,
  onIgnore,
  onInventory,
}: {
  project: Project
  roots: string[]
  depVisible: (d: Dependency) => boolean
  onIgnore: (r: IgnoreRequest) => void
  onInventory: (inv: Inventory) => void
}) {
  const [selected, setSelected] = useState<Map<string, Target>>(new Map())
  const [updating, setUpdating] = useState<Change[] | null>(null)

  useEffect(() => setSelected(new Map()), [project.id])

  const deps = useMemo(
    () =>
      project.dependencies
        .filter(depVisible)
        .sort((a, b) => b.vulns.length - a.vulns.length || STATUS_ORDER.indexOf(a.status) - STATUS_ORDER.indexOf(b.status) || a.name.localeCompare(b.name)),
    [project, depVisible],
  )
  const candidates = deps.filter(updatable)

  const toggle = (d: Dependency) =>
    setSelected((prev) => {
      const next = new Map(prev)
      if (next.has(depKey(d))) next.delete(depKey(d))
      else next.set(depKey(d), defaultTarget(d))
      return next
    })

  const setTarget = (d: Dependency, t: Target) => setSelected((prev) => new Map(prev).set(depKey(d), t))

  // Everything that can move without crossing a major version.
  const selectSafe = () =>
    setSelected(new Map(candidates.filter((d) => d.status !== 'major' || d.safeLatest).map((d) => [depKey(d), d.status === 'major' ? 'safe' : 'latest'])))

  const review = () => {
    const changes = project.dependencies
      .filter((d) => selected.has(depKey(d)))
      .map((d) => ({ name: d.name, from: d.requested, to: targetVersion(d, selected.get(depKey(d))!) }))
    setUpdating(changes)
  }

  return (
    <section className="flex min-w-0 flex-col overflow-hidden">
      <header className="flex items-start gap-3 border-b border-line px-6 py-4">
        <div className="min-w-0 flex-1">
          <div className="flex items-center gap-2">
            <h2 className="truncate text-[17px] font-semibold">{project.name}</h2>
            <EcoBadge ecosystem={project.ecosystem} />
          </div>
          <div className="mt-1 truncate text-[12px] text-dim" title={project.manifest}>
            {relativePath(roots, project.manifest)}
          </div>
          {project.frameworks.length > 0 && (
            <div className="mt-2 flex flex-wrap gap-1">
              {project.frameworks.map((f) => (
                <span key={f} className="rounded border border-line-strong bg-raised px-1.5 py-px font-mono text-[11px] text-muted">
                  {f}
                </span>
              ))}
            </div>
          )}
        </div>
        <IconButton title="Show manifest in Explorer" onClick={() => reveal(project.manifest)}>
          <FolderOpen size={15} />
        </IconButton>
        <IconButton title="Open project in VS Code" onClick={() => openInEditor(project.repo ?? project.dir)}>
          <Code2 size={15} />
        </IconButton>
        <IconButton title="Ignore this project from now on" onClick={() => onIgnore({ kind: 'project', value: project.manifest, label: project.name })}>
          <EyeOff size={15} />
        </IconButton>
        {project.repo && (
          <IconButton
            title="Ignore this whole repo from now on"
            onClick={() => onIgnore({ kind: 'folder', value: project.repo!, label: relativePath(roots, project.repo!) })}
          >
            <FolderMinus size={15} />
          </IconButton>
        )}
      </header>

      {candidates.length > 0 && (
        <div className="flex items-center gap-3 border-b border-line bg-panel px-6 py-2 text-[12.5px]">
          <span className="text-muted">
            {selected.size > 0 ? `${selected.size} selected` : `${candidates.length} package${candidates.length === 1 ? '' : 's'} can be updated`}
          </span>
          <button type="button" onClick={selectSafe} className="text-gold hover:underline" title="Every update that stays within the current major version">
            Select safe updates
          </button>
          <button type="button" onClick={() => setSelected(new Map(candidates.map((d) => [depKey(d), defaultTarget(d)])))} className="text-muted hover:text-ink">
            Select all
          </button>
          {selected.size > 0 && (
            <button type="button" onClick={() => setSelected(new Map())} className="text-muted hover:text-ink">
              Clear
            </button>
          )}
          <div className="flex-1" />
          <button
            type="button"
            onClick={review}
            disabled={selected.size === 0}
            className="inline-flex items-center gap-1.5 rounded-lg bg-gold px-3 py-1 text-[12.5px] font-semibold text-[#1d1506] hover:brightness-110 disabled:opacity-40"
          >
            <ArrowUpCircle size={14} />
            Review update{selected.size === 1 ? '' : 's'}
          </button>
        </div>
      )}

      <div className="flex-1 overflow-auto">
        {deps.length === 0 ? (
          <Empty title="Nothing to show">No dependencies in this project match the current filters.</Empty>
        ) : (
          <table className="w-full min-w-[820px] border-collapse text-left">
            <thead className="sticky top-0 z-10 bg-bg/95 text-[11px] uppercase tracking-wider text-dim backdrop-blur">
              <tr className="border-b border-line">
                <th className="w-10 py-2 pl-6" aria-label="Select" />
                <th className="py-2 pr-3 font-medium">Package</th>
                <th className="px-3 py-2 font-medium">Written as</th>
                <th className="px-3 py-2 font-medium">In use</th>
                <th className="px-3 py-2 font-medium">Update to</th>
                <th className="px-3 py-2 font-medium">Status</th>
                <th className="px-6 py-2 text-right font-medium">Security</th>
              </tr>
            </thead>
            <tbody>
              {deps.map((d, i) => {
                const canUpdate = updatable(d)
                const target = selected.get(depKey(d))
                return (
                  <tr key={`${depKey(d)}-${i}`} className={cx('border-b border-line/60 hover:bg-hover/40', target && 'bg-gold-soft/30')}>
                    <td className="py-1.5 pl-6">
                      {canUpdate && (
                        <input
                          type="checkbox"
                          checked={!!target}
                          onChange={() => toggle(d)}
                          aria-label={`Update ${d.name}`}
                          className="size-3.5 accent-[var(--color-gold)]"
                        />
                      )}
                    </td>
                    <td className="max-w-[300px] py-1.5 pr-3">
                      <div className="truncate font-medium" title={d.name}>
                        {d.name}
                      </div>
                      {(d.kind !== 'normal' && d.kind !== 'action') || d.note ? (
                        <div className="truncate text-[11px] text-dim">{[d.kind !== 'normal' && d.kind !== 'action' ? d.kind : null, d.note].filter(Boolean).join(' · ')}</div>
                      ) : null}
                    </td>
                    <td className="max-w-[180px] truncate px-3 py-1.5 font-mono text-[11.5px] text-dim" title={d.requested}>
                      {d.requested || '-'}
                    </td>
                    <td className="px-3 py-1.5 font-mono text-[12px]" title={d.approximate ? 'Guessed from the version range' : (d.installedFrom ?? '')}>
                      {d.approximate && <span className="text-dim">~</span>}
                      {d.status === 'local' ? '-' : displayVersion(d)}
                    </td>
                    <td className="px-3 py-1.5 font-mono text-[12px] text-muted">
                      {target && d.safeLatest ? (
                        <select
                          value={target}
                          onChange={(e) => setTarget(d, e.target.value as Target)}
                          aria-label={`Target version for ${d.name}`}
                          className="rounded-md border border-line-strong bg-raised px-1 py-0.5 font-mono text-[12px] text-ink"
                        >
                          <option value="safe">{d.safeLatest} (safe)</option>
                          <option value="latest">{d.latest} (latest)</option>
                        </select>
                      ) : (
                        <>
                          {d.latest ?? '-'}
                          {d.safeLatest && <div className="text-[10.5px] text-dim">safe: {d.safeLatest}</div>}
                        </>
                      )}
                    </td>
                    <td className="px-3 py-1.5">
                      <StatusPill status={d.status} />
                    </td>
                    <td className="px-6 py-1.5 text-right">
                      {d.vulns.length > 0 ? (
                        <span className="inline-flex items-center gap-1 text-[12px] text-carnelian" title={d.vulns.join(', ')}>
                          <ShieldAlert size={13} />
                          {d.vulns.length}
                        </span>
                      ) : (
                        <span className="text-dim">-</span>
                      )}
                    </td>
                  </tr>
                )
              })}
            </tbody>
          </table>
        )}
      </div>

      {updating && (
        <UpdateDialog
          projectId={project.id}
          changes={updating}
          roots={roots}
          onClose={(refreshed) => {
            setUpdating(null)
            if (refreshed) {
              setSelected(new Map())
              onInventory(refreshed)
            }
          }}
        />
      )}
    </section>
  )
}
