import { Check, Code2, EyeOff, Folder, FolderOpen, FolderPlus, Layers, ListChecks, RefreshCw, ShieldAlert, SlidersHorizontal } from 'lucide-react'
import { useState, type ReactNode } from 'react'
import { ECOSYSTEM_LABEL, type Repo } from '../derive'
import type { Ecosystem } from '../types'
import { cx } from './bits'
import { Menu, MenuItem, MenuSeparator, MenuTitle } from './Menu'

const ECO_SHORT: Record<Ecosystem, string> = { npm: 'npm', cargo: 'Rs', nuget: 'Nu', 'github-actions': 'GA' }

export function ScanStatus({ running, phase, checkedAgo, detail, hint, onScan }: { running: boolean; phase: string | null; checkedAgo: string; detail: string; hint?: string; onScan: () => void }) {
  return (
    <div className="flex shrink-0 items-center gap-2.5 border-t border-rail-line bg-rail-sunken px-3.5 pt-2.5 pb-3">
      <div className="flex min-w-0 flex-1 items-center gap-2.5" aria-live="polite">
        <span
          aria-hidden
          className={cx('size-2 shrink-0 rounded-full', running ? 'animate-pulse bg-rail-busy' : 'bg-rail-ok')}
          style={{ boxShadow: `0 0 0 3px color-mix(in oklab, var(${running ? '--rail-busy' : '--rail-ok'}) 20%, transparent)` }}
        />
        <span className="flex min-w-0 flex-col">
          <b className="truncate text-[12px] font-semibold">{running ? (phase ?? 'Checking…') : `Checked ${checkedAgo}`}</b>
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
  selected,
  query,
  excludedCount,
  onSelect,
  onManage,
  onAddFolder,
  onExclusions,
  onRefreshAll,
  onReveal,
  onOpenInEditor,
  onIgnoreRepo,
  footer,
}: {
  repos: Repo[]
  roots: string[]
  selected: string | null
  query: string
  excludedCount: number
  onSelect: (key: string | null) => void
  onManage: () => void
  onAddFolder: () => void
  onExclusions: () => void
  onRefreshAll: () => void
  onReveal: (path: string) => void
  onOpenInEditor: (path: string) => void
  onIgnoreRepo: (repo: Repo) => void
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

      <div className="min-h-0 flex-1 overflow-y-auto pb-2">
        {groups.map(({ root, repos: list }) => {
          const shown = list.filter(matches)
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
                    <span aria-hidden className="grid size-7 place-items-center rounded-[3px] border border-rail-border bg-rail-raised font-mono text-[11px] font-bold text-rail-ink-2">
                      {monogram(repo.name)}
                    </span>
                    <span className="flex min-w-0 flex-col gap-1">
                      <b className="truncate text-[12.5px] font-semibold">{repo.name}</b>
                      <span className="flex gap-[3px]" aria-hidden>
                        {repo.ecosystems.map((e) => (
                          <i key={e} title={ECOSYSTEM_LABEL[e]} className="inline-grid h-4 min-w-5 place-items-center rounded-[2px] border border-rail-border bg-chip-bg px-[3px] font-mono text-[10.5px] leading-none font-bold not-italic text-rail-ink-2">
                            {ECO_SHORT[e]}
                          </i>
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
          <MenuSeparator />
          <MenuItem icon={<EyeOff size={15} />} danger onSelect={() => (setContext(null), onIgnoreRepo(context.repo))}>
            Exclude {context.repo.name}
          </MenuItem>
        </Menu>
      )}
      {footer}
    </aside>
  )
}

function monogram(name: string): string {
  const words = name.replace(/[._-]+/g, ' ').split(/\s+|(?=[A-Z][a-z])/).filter(Boolean)
  return (words.length > 1 ? words[0][0] + words[1][0] : name.slice(0, 2)).toUpperCase()
}

