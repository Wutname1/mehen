import { Code2, EyeOff, FolderMinus, FolderOpen, ShieldAlert } from 'lucide-react'
import { useEffect, useMemo, useState } from 'react'
import type { IgnoreRequest } from '../App'
import { openInEditor, reveal } from '../api'
import { displayVersion, isOutdated, relativePath, worstStatus } from '../derive'
import type { Dependency, Project } from '../types'
import { EcoBadge, Empty, IconButton, StatusPill, cx } from './bits'

const STATUS_ORDER = ['major', 'minor', 'patch', 'unknown', 'unpinned', 'pending', 'up-to-date', 'local']

export function ProjectsView({
  projects,
  roots,
  depVisible,
  onIgnore,
}: {
  projects: Project[]
  roots: string[]
  depVisible: (d: Dependency) => boolean
  onIgnore: (r: IgnoreRequest) => void
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
      {selected && <ProjectDetail project={selected} roots={roots} depVisible={depVisible} onIgnore={onIgnore} />}
    </div>
  )
}

function ProjectDetail({
  project,
  roots,
  depVisible,
  onIgnore,
}: {
  project: Project
  roots: string[]
  depVisible: (d: Dependency) => boolean
  onIgnore: (r: IgnoreRequest) => void
}) {
  const deps = useMemo(
    () =>
      project.dependencies
        .filter(depVisible)
        .sort((a, b) => b.vulns.length - a.vulns.length || STATUS_ORDER.indexOf(a.status) - STATUS_ORDER.indexOf(b.status) || a.name.localeCompare(b.name)),
    [project, depVisible],
  )
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
      <div className="flex-1 overflow-auto">
        {deps.length === 0 ? (
          <Empty title="Nothing to show">No dependencies in this project match the current filters.</Empty>
        ) : (
          <table className="w-full min-w-[760px] border-collapse text-left">
            <thead className="sticky top-0 bg-bg/95 text-[11px] uppercase tracking-wider text-dim backdrop-blur">
              <tr className="border-b border-line">
                <th className="px-6 py-2 font-medium">Package</th>
                <th className="px-3 py-2 font-medium">Written as</th>
                <th className="px-3 py-2 font-medium">In use</th>
                <th className="px-3 py-2 font-medium">Latest</th>
                <th className="px-3 py-2 font-medium">Status</th>
                <th className="px-6 py-2 text-right font-medium">Security</th>
              </tr>
            </thead>
            <tbody>
              {deps.map((d, i) => (
                <tr key={`${d.name}-${i}`} className="border-b border-line/60 hover:bg-hover/40">
                  <td className="max-w-[320px] px-6 py-1.5">
                    <div className="truncate font-medium" title={d.name}>
                      {d.name}
                    </div>
                    {(d.kind !== 'normal' && d.kind !== 'action') || d.note ? (
                      <div className="truncate text-[11px] text-dim">{[d.kind !== 'normal' && d.kind !== 'action' ? d.kind : null, d.note].filter(Boolean).join(' · ')}</div>
                    ) : null}
                  </td>
                  <td className="max-w-[200px] truncate px-3 py-1.5 font-mono text-[11.5px] text-dim" title={d.requested}>
                    {d.requested || '-'}
                  </td>
                  <td className="px-3 py-1.5 font-mono text-[12px]" title={d.approximate ? 'Guessed from the version range' : (d.installedFrom ?? '')}>
                    {d.approximate && <span className="text-dim">~</span>}
                    {d.status === 'local' ? '-' : displayVersion(d)}
                  </td>
                  <td className="px-3 py-1.5 font-mono text-[12px] text-muted">{d.latest ?? '-'}</td>
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
              ))}
            </tbody>
          </table>
        )}
      </div>
    </section>
  )
}
