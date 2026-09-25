import { ArrowDownAZ, ArrowDownWideNarrow, ArrowDownZA, Check, Code2, EyeOff, Folder, FolderOpen, FolderPlus, Layers, ListChecks, RefreshCw, Settings2, ShieldAlert, SlidersHorizontal } from 'lucide-react'
import { useEffect, useState, type ReactNode } from 'react'
import { ECOSYSTEM_LABEL, type Repo } from '../derive'
import { GitWyrmMark, cx } from './bits'
import { EcoIcon } from './EcoIcon'
import { Menu, MenuItem, MenuSeparator, MenuTitle } from './Menu'
import { RepoAvatar } from './RepoAvatar'

export type RailSort = 'az' | 'za' | 'updates'

/** One button cycles through these, in order. */
const SORTS: { id: RailSort; short: string; label: string; Icon: typeof ArrowDownAZ }[] = [
  { id: 'az', short: 'A to Z', label: 'name, A to Z', Icon: ArrowDownAZ },
  { id: 'za', short: 'Z to A', label: 'name, Z to A', Icon: ArrowDownZA },
  { id: 'updates', short: 'Updates', label: 'most updates first, vulnerable on top', Icon: ArrowDownWideNarrow },
]

const byName = (a: Repo, b: Repo) => a.name.localeCompare(b.name, undefined, { sensitivity: 'base' })

function sortRepos(list: Repo[], sort: RailSort): Repo[] {
  const sorted = [...list]
  if (sort === 'za') return sorted.sort((a, b) => byName(b, a))
  if (sort === 'updates') return sorted.sort((a, b) => b.vulnerable - a.vulnerable || b.updates - a.updates || byName(a, b))
  return sorted.sort(byName)
}

function timeAgo(unixSeconds: number, nowMs: number): string {
  const s = Math.max(0, Math.floor(nowMs / 1000 - unixSeconds))
  if (s < 60) return 'just now'
  if (s < 3600) return `${Math.floor(s / 60)} min ago`
  if (s < 86400) return `${Math.floor(s / 3600)} h ago`
  return `${Math.floor(s / 86400)} d ago`
}

/** The current time, refreshed every `everyMs` so relative labels stay true. */
function useNow(everyMs: number): number {
  const [now, setNow] = useState(Date.now)
  useEffect(() => {
    const id = window.setInterval(() => setNow(Date.now()), everyMs)
    return () => window.clearInterval(id)
  }, [everyMs])
  return now
}

export function ScanStatus({
  running,
  phase,
  checkedAt,
  detail,
  hint,
  onScan,
}: {
  running: boolean
  phase: string | null
  /** Unix seconds of the last finished check. */
  checkedAt: number | null | undefined
  detail: string
  hint?: string
  onScan: () => void
}) {
  const now = useNow(30_000)
  const checked = checkedAt ? `Checked ${timeAgo(checkedAt, now)}` : 'Not checked yet'
  return (
    <div className="flex shrink-0 items-center gap-2.5 border-t border-rail-line bg-rail-sunken px-3.5 pt-2.5 pb-3">
      <div className="flex min-w-0 flex-1 items-center gap-2.5" aria-live="polite">
        <span
          aria-hidden
          className={cx('size-2 shrink-0 rounded-full', running ? 'animate-pulse bg-rail-busy' : 'bg-rail-ok')}
          style={{ boxShadow: `0 0 0 3px color-mix(in oklab, var(${running ? '--rail-busy' : '--rail-ok'}) 20%, transparent)` }}
        />
        <span className="flex min-w-0 flex-col">
          <b className="truncate text-[12px] font-semibold" title={checkedAt ? new Date(checkedAt * 1000).toLocaleString() : undefined}>
            {running ? (phase ?? 'Checking…') : checked}
          </b>
          <small className={hint ? "truncate text-[12px] text-rail-busy" : "truncate text-[12px] text-rail-muted"} title={hint}>
            {detail}
          </small>
        </span>
      </div>
      <button
        type="button"
        onClick={onScan}
        disabled={running}
        aria-label="Check now"
        title="Look for new versions and advisories now"
        className="inline-flex h-8 shrink-0 items-center gap-1.5 rounded-[3px] border border-rail-border-strong px-2.5 text-[12px] text-rail-ink hover:border-rail-active-line hover:bg-rail-hover disabled:opacity-60"
      >
        <RefreshCw size={14} className={cx(running && 'animate-spin')} />
        Scan
      </button>
    </div>
  )
}

export function Rail({
  repos,
  roots,
  icons,
  selected,
  query,
  sort,
  onSort,
  excludedCount,
  onSelect,
  onManage,
  onAddFolder,
  onExclusions,
  onRefreshAll,
  onReveal,
  onOpenInEditor,
  onOpenInGitWyrm,
  onExclude,
  onProjectSettings,
  onScanRepo,
  footer,
}: {
  repos: Repo[]
  roots: string[]
  icons: Record<string, string>
  selected: string | null
  query: string
  sort: RailSort
  onSort: (sort: RailSort) => void
  excludedCount: number
  onSelect: (key: string | null) => void
  onManage: () => void
  onAddFolder: () => void
  onExclusions: () => void
  onRefreshAll: () => void
  onReveal: (path: string) => void
  onOpenInEditor: (path: string) => void
  /** Absent when GitWyrm is not installed. */
  onOpenInGitWyrm?: (path: string) => void
  onExclude: (repo: Repo) => void
  onProjectSettings: (repo: Repo) => void
  onScanRepo: (repo: Repo) => void
  footer: ReactNode
}) {
  const [manage, setManage] = useState<HTMLElement | null>(null)
  const [context, setContext] = useState<{ repo: Repo; at: { x: number; y: number } } | null>(null)

  const q = query.trim().toLowerCase()
  const matches = (r: Repo) =>
    !q || r.name.toLowerCase().includes(q) || r.key.toLowerCase().includes(q) || r.projects.some((p) => p.dependencies.some((d) => d.name.toLowerCase().includes(q)))
  const vulnerable = repos.filter((r) => r.vulnerable > 0).length
  const groups = [...roots, null].map((root) => ({ root, repos: repos.filter((r) => r.root === root) })).filter((g) => g.repos.length > 0)

  const openContext = (repo: Repo, x: number, y: number) => setContext({ repo, at: { x, y } })

  return (
    <aside className="on-rail flex min-h-0 flex-col border-r border-rail-line bg-rail text-rail-ink" aria-label="Projects">
      <div
        role="group"
        aria-label="All projects"
        className={cx('relative mx-3 mt-3 mb-2.5 flex rounded-[3px] border', !selected ? 'border-rail-active-line bg-rail-active' : 'border-rail-border bg-rail-field')}
      >
        <button
          type="button"
          onClick={() => onSelect(null)}
          aria-current={!selected}
          className="flex min-h-[50px] min-w-0 flex-1 items-center gap-2.5 rounded-l-[2px] px-2.5 py-1.5 text-left hover:bg-white/[0.04]"
        >
          <Layers size={18} className="shrink-0 text-rail-icon" />
          <span className="flex min-w-0 flex-col gap-0.5">
            <b className="text-[13px] font-semibold">All projects</b>
            <small className="text-[12px] leading-snug text-rail-muted">
              {repos.length} project{repos.length === 1 ? '' : 's'}
              {vulnerable > 0 && <span className="text-rail-alert"> · {vulnerable} vulnerable</span>}
            </small>
          </span>
        </button>
        <button
          type="button"
          onClick={(e) => setManage(manage ? null : e.currentTarget)}
          onKeyDown={(e) => {
            if (e.key === 'ArrowDown' && !manage) {
              e.preventDefault()
              setManage(e.currentTarget)
            }
          }}
          aria-label="Manage projects"
          aria-haspopup="menu"
          aria-expanded={!!manage}
          title="Manage projects, folders, and exclusions"
          className={cx(
            'grid w-10 shrink-0 place-items-center rounded-r-[2px] border-l text-rail-ink-2 hover:bg-white/[0.07] hover:text-rail-ink',
            !selected ? 'border-rail-active-line' : 'border-rail-border',
            manage && 'bg-white/[0.07] text-rail-ink',
          )}
        >
          <SlidersHorizontal size={16} />
        </button>
      </div>
      {manage && (
        <Menu anchor={manage} label="Manage projects" onClose={() => setManage(null)} width={272}>
          <MenuItem icon={<ListChecks size={15} />} onSelect={() => (setManage(null), onManage())}>
            Manage projects…
          </MenuItem>
          <MenuItem icon={<FolderPlus size={15} />} onSelect={() => (setManage(null), onAddFolder())}>
            Add a folder to scan…
          </MenuItem>
          <MenuItem icon={<EyeOff size={15} />} onSelect={() => (setManage(null), onExclusions())}>
            Excluded projects &amp; folders…
            {excludedCount > 0 && <em className="ml-auto font-mono text-[11px] not-italic text-faint">{excludedCount}</em>}
          </MenuItem>
          <MenuSeparator />
          <MenuItem icon={<RefreshCw size={15} />} onSelect={() => (setManage(null), onRefreshAll())}>
            Check everything again, skipping the cache
          </MenuItem>
        </Menu>
      )}

      <div className="mx-3 mb-2 flex items-center justify-between">
        <span className="font-mono text-[11px] tracking-[0.04em] text-rail-muted uppercase">
          Sort
        </span>
        {(() => {
          const i = Math.max(0, SORTS.findIndex((x) => x.id === sort))
          const { short, label, Icon } = SORTS[i]
          const next = SORTS[(i + 1) % SORTS.length]
          return (
            <button
              type="button"
              onClick={() => onSort(next.id)}
              aria-label={`Sorted by ${label}. Change to ${next.label}`}
              title={`Sorted by ${label}. Click for ${next.label}.`}
              className="inline-flex h-7 items-center gap-1.5 rounded-[3px] border border-rail-border px-2 text-[12px] text-rail-ink-2 hover:border-rail-border-strong hover:bg-rail-hover hover:text-rail-ink"
            >
              <Icon size={15} />
              {short}
            </button>
          )
        })()}
      </div>

      <div className="min-h-0 flex-1 overflow-y-auto pb-2">
        {groups.map(({ root, repos: list }) => {
          const shown = sortRepos(list.filter(matches), sort)
          return (
            <div key={root ?? 'other'}>
              <div className="sticky top-0 z-[1] flex h-[30px] items-center gap-2 border-y border-rail-line bg-rail-sunken px-3.5 font-mono text-[11px] text-rail-muted">
                <Folder size={14} className="shrink-0" />
                <span className="min-w-0 flex-1 truncate" title={root ?? undefined}>
                  {root ?? 'Other'}
                </span>
                <span aria-label={`${list.length} projects`}>{list.length}</span>
              </div>
              {shown.map((repo) => {
                const active = selected === repo.key
                const label = `${repo.name}, ${repo.ecosystems.map((e) => ECOSYSTEM_LABEL[e]).join(', ')}, ${repo.updates ? `${repo.updates} update${repo.updates === 1 ? '' : 's'}` : 'up to date'}${repo.vulnerable ? `, ${repo.vulnerable} vulnerable` : ''}`
                return (
                  <button
                    key={repo.key}
                    type="button"
                    onClick={() => onSelect(repo.key)}
                    onContextMenu={(e) => {
                      e.preventDefault()
                      openContext(repo, e.clientX, e.clientY)
                    }}
                    onKeyDown={(e) => {
                      if (e.key === 'ContextMenu' || (e.shiftKey && e.key === 'F10')) {
                        e.preventDefault()
                        const r = e.currentTarget.getBoundingClientRect()
                        openContext(repo, r.left + 40, r.top + 36)
                      }
                    }}
                    aria-current={active}
                    aria-label={label}
                    title={repo.key}
                    className={cx(
                      'grid min-h-14 w-full grid-cols-[28px_minmax(0,1fr)_auto] items-center gap-2.5 border-b border-rail-row-line py-2 pr-3 pl-3.5 text-left',
                      active ? 'bg-rail-active text-white shadow-[inset_0_0_0_1px_var(--rail-active-line)]' : 'text-rail-ink-2 hover:bg-rail-hover',
                    )}
                  >
                    <RepoAvatar name={repo.name} icon={icons[repo.key.toLowerCase()]} tone="rail" />
                    <span className="flex min-w-0 flex-col gap-1">
                      <b className="truncate text-[12.5px] font-semibold">{repo.name}</b>
                      <span className="flex items-center gap-1.5 text-rail-ink-2" aria-hidden>
                        {repo.ecosystems.map((e) => (
                          <EcoIcon key={e} ecosystem={e} size={14} />
                        ))}
                      </span>
                    </span>
                    {repo.vulnerable > 0 ? (
                      <span className="inline-flex items-center gap-1 font-mono text-[12px] text-rail-alert">
                        <ShieldAlert size={14} />
                        {repo.updates}
                      </span>
                    ) : repo.updates > 0 ? (
                      <span className="font-mono text-[12px] text-rail-ink-2">{repo.updates}</span>
                    ) : (
                      <Check size={14} className="text-rail-ok" />
                    )}
                  </button>
                )
              })}
            </div>
          )
        })}
      </div>
      {context && (
        <Menu anchor={context.at} label={`${context.repo.name} actions`} onClose={() => setContext(null)} width={260}>
          <MenuTitle title={context.repo.name} detail={context.repo.key} />
          <MenuItem icon={<FolderOpen size={15} />} onSelect={() => (setContext(null), onReveal(context.repo.key))}>
            Show in File Explorer
          </MenuItem>
          <MenuItem icon={<Code2 size={15} />} onSelect={() => (setContext(null), onOpenInEditor(context.repo.key))}>
            Open in VS Code
          </MenuItem>
          {onOpenInGitWyrm && context.repo.projects.some((p) => p.repo) && (
            <MenuItem icon={<GitWyrmMark size={15} />} onSelect={() => (setContext(null), onOpenInGitWyrm(context.repo.key))}>
              Open in GitWyrm
            </MenuItem>
          )}
          <MenuItem icon={<Settings2 size={15} />} onSelect={() => (setContext(null), onProjectSettings(context.repo))}>
            Project settings…
          </MenuItem>
          <MenuItem icon={<RefreshCw size={15} />} onSelect={() => (setContext(null), onScanRepo(context.repo))}>
            Check this project again
          </MenuItem>
          <MenuSeparator />
          <MenuItem icon={<EyeOff size={15} />} danger onSelect={() => (setContext(null), onExclude(context.repo))}>
            Exclude…
          </MenuItem>
        </Menu>
      )}
      {footer}
    </aside>
  )
}


