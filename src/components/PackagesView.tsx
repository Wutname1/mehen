import { ChevronRight, Code2, FolderOpen, ShieldAlert } from 'lucide-react'
import { useState } from 'react'
import { openInEditor, reveal } from '../api'
import { displayVersion, relativePath, type PackageGroup } from '../derive'
import { EcoBadge, Empty, IconButton, StatusPill, VersionChip, cx } from './bits'

const MAX_CHIPS = 4

export function PackagesView({ groups, root }: { groups: PackageGroup[]; root: string }) {
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
            {expanded && <UsageList group={g} root={root} />}
          </div>
        )
      })}
    </div>
  )
}

function UsageList({ group, root }: { group: PackageGroup; root: string }) {
  const usages = [...group.usages].sort((a, b) => a.project.dir.localeCompare(b.project.dir))
  return (
    <div className="pb-3 pl-[46px] pr-5">
      <div className="overflow-hidden rounded-lg border border-line bg-bg/60">
        {usages.map(({ project, dep }, i) => (
          <div
            key={`${project.id}-${i}`}
            className="grid grid-cols-[minmax(200px,1.4fr)_minmax(110px,0.8fr)_minmax(130px,1fr)_120px_auto] items-center gap-4 border-b border-line/60 px-3 py-1.5 last:border-b-0"
          >
            <div className="min-w-0">
              <div className="truncate text-ink">{project.name}</div>
              <div className="truncate text-[11px] text-dim" title={project.dir}>
                {relativePath(root, project.dir)}
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
            </div>
          </div>
        ))}
      </div>
    </div>
  )
}
