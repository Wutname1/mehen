import { ChevronDown, Code2, FolderOpen, GitCommitHorizontal, Play, Settings2, Terminal, X } from 'lucide-react'
import { useState, type ReactNode } from 'react'
import { ECOSYSTEM_LABEL, distinctVersions, displayVersion, folderName, repoKey, samePath, type QueueUsage, type Repo } from '../derive'
import { cx } from './bits'
import { Menu, MenuCheck, MenuSeparator, MenuTitle } from './Menu'

/** Selected usages grouped by package, for the tray. */
export interface TrayGroup {
  key: string
  name: string
  usages: QueueUsage[]
}

function Fact({ label, children, warn }: { label: string; children: ReactNode; warn?: boolean }) {
  return (
    <div className="flex justify-between gap-3 border-b border-line py-[7px] text-[12.5px]">
      <dt className="text-muted">{label}</dt>
      <dd className={cx('m-0 text-right font-semibold', warn && 'text-risk-review')}>{children}</dd>
    </div>
  )
}

export function ProjectRecord({ repo, onReveal, onOpenInEditor, onSettings }: { repo: Repo; onReveal: () => void; onOpenInEditor: () => void; onSettings: () => void }) {
  return (
    <section className="shrink-0 border-b border-line px-[18px] pt-4 pb-3.5">
      <div className="flex items-start gap-2">
        <h2 className="m-0 min-w-0 flex-1 font-display text-[22px] leading-tight font-semibold tracking-[-0.015em] [overflow-wrap:anywhere]">{repo.name}</h2>
        <button type="button" onClick={onSettings} aria-label={`${repo.name} settings`} title="Settings for this project" className="grid size-[30px] shrink-0 place-items-center rounded-[3px] text-muted hover:bg-sunken hover:text-ink">
          <Settings2 size={16} />
        </button>
        <button type="button" onClick={onReveal} aria-label="Show in File Explorer" title="Show in File Explorer" className="grid size-[30px] shrink-0 place-items-center rounded-[3px] text-muted hover:bg-sunken hover:text-ink">
          <FolderOpen size={16} />
        </button>
        <button type="button" onClick={onOpenInEditor} aria-label="Open in VS Code" title="Open in VS Code" className="grid size-[30px] shrink-0 place-items-center rounded-[3px] text-muted hover:bg-sunken hover:text-ink">
          <Code2 size={16} />
        </button>
      </div>
      <code className="mt-1 mb-3 block font-mono text-[12px] text-state [overflow-wrap:anywhere]">{repo.key}</code>
      <dl className="m-0 border-t border-line">
        <Fact label="Uses">{repo.ecosystems.map((e) => ECOSYSTEM_LABEL[e]).join(', ')}</Fact>
        <Fact label="Manifests">{repo.projects.length}</Fact>
        <Fact label="Dependencies">
          {repo.dependencies}, {repo.updates ? `${repo.updates} out of date` : 'all current'}
        </Fact>
        {repo.vulnerable > 0 && (
          <Fact label="Vulnerable" warn>
            {repo.vulnerable} package{repo.vulnerable === 1 ? '' : 's'}
          </Fact>
        )}
      </dl>
    </section>
  )
}

export function Tray({
  groups,
  repo,
  onRemove,
  onClear,
  onUpdate,
  onPreview,
  checks,
  commit,
  onOptions,
}: {
  groups: TrayGroup[]
  repo: Repo | null
  onRemove: (group: TrayGroup) => void
  onClear: () => void
  onUpdate: () => void
  onPreview: () => void
  checks: boolean
  commit: boolean
  onOptions: (patch: { checks?: boolean; commit?: boolean }) => void
}) {
  const [menu, setMenu] = useState<HTMLElement | null>(null)
  const usages = groups.flatMap((g) => g.usages)
  const repos = new Set(usages.map((u) => repoKey(u.project).toLowerCase()))
  const inRepo = (g: TrayGroup) => !!repo && g.usages.some((u) => samePath(repoKey(u.project), repo.key))
  const here = repo ? groups.filter(inRepo) : groups
  const other = repo ? groups.filter((g) => !inRepo(g)) : []

  const item = (g: TrayGroup, elsewhere: boolean) => {
    const from = distinctVersions(g.usages.map((u) => displayVersion(u.dep)))
    const to = distinctVersions(g.usages.map((u) => u.target))
    const where = new Set(g.usages.map((u) => repoKey(u.project).toLowerCase())).size
    return (
      <div key={g.key} className={cx('grid grid-cols-[1fr_auto] items-center gap-2 border-b border-line py-2', elsewhere && 'opacity-75')}>
        <span className="flex min-w-0 flex-col gap-0.5">
          <b className="truncate text-[12.5px]">{g.name}</b>
          <small className="font-mono text-[12px] text-muted">
            {from[0]}
            {from.length > 1 && '+'} to {to.at(-1)} · {where === 1 ? folderName(repoKey(g.usages[0].project)) : `${where} projects`}
          </small>
        </span>
        <button type="button" onClick={() => onRemove(g)} aria-label={`Remove ${g.name} from the selection`} className="grid size-[30px] place-items-center rounded-[3px] text-muted hover:bg-sunken hover:text-ink">
          <X size={15} />
        </button>
      </div>
    )
  }

  return (
    <section className="flex flex-[1_0_auto] flex-col" aria-labelledby="tray-title">
      <div className="px-[18px] pt-4 pb-2.5">
        <div className="flex items-center gap-2">
          <h2 id="tray-title" className="m-0 flex-1 font-display text-[22px] leading-tight font-semibold tracking-[-0.015em]">
            {groups.length} selected
          </h2>
          {groups.length > 0 && (
            <button type="button" onClick={onClear} aria-label="Clear selection" title="Clear selection" className="grid size-[30px] place-items-center rounded-[3px] text-muted hover:bg-sunken hover:text-ink">
              <X size={16} />
            </button>
          )}
        </div>
        <p className="mt-1 mb-0 text-[12.5px] text-muted">
          {groups.length ? `Across ${repos.size} project${repos.size === 1 ? '' : 's'}. Files unchanged until you update.` : 'Files unchanged.'}
        </p>
      </div>
      <div className="flex-[1_0_auto] px-[18px] pb-3">
        {groups.length === 0 ? (
          <p className="py-3 text-[12.5px] leading-relaxed text-muted">Tick packages in the list to select them. Nothing on disk changes until you update.</p>
        ) : (
          <>
            {repo && here.length > 0 && <div className="mt-2.5 mb-1 font-mono text-[11px] tracking-[0.04em] text-muted uppercase">Includes {repo.name}</div>}
            {here.map((g) => item(g, false))}
            {other.length > 0 && <div className="mt-2.5 mb-1 font-mono text-[11px] tracking-[0.04em] text-muted uppercase">Other projects</div>}
            {other.map((g) => item(g, true))}
          </>
        )}
      </div>
      <div className="sticky bottom-0 grid gap-2 border-t border-line bg-paper-2 px-[18px] pt-3 pb-4">
        {groups.length > 0 && (
          <div className="grid grid-cols-3 rounded-[3px] border border-line bg-surface" aria-label="What will change">
            {[
              [groups.length, 'packages'],
              [new Set(usages.map((u) => u.project.id)).size, 'manifests'],
              [repos.size, 'projects'],
            ].map(([n, label], i) => (
              <div key={label} className={cx('flex flex-col px-2.5 py-2', i > 0 && 'border-l border-line')}>
                <b className="font-display text-[18px] font-semibold">{n}</b>
                <small className="text-[12px] text-muted">{label}</small>
              </div>
            ))}
          </div>
        )}
        {groups.length > 0 && (
          <p className="m-0 mb-0.5 flex items-center gap-1.5 text-[12px] text-muted">
            {commit ? <GitCommitHorizontal size={14} className="shrink-0" /> : <Terminal size={14} className="shrink-0" />}
            {checks && commit
              ? 'Then builds, tests, and commits each project.'
              : checks
                ? 'Then builds and tests. You commit afterwards.'
                : commit
                  ? 'Installs only, then commits each project. No build or test.'
                  : 'Installs only. No build or test, nothing committed.'}
          </p>
        )}
        <button
          type="button"
          onClick={onPreview}
          disabled={groups.length === 0}
          className="inline-flex h-[38px] items-center justify-center gap-2 rounded-[3px] border border-line-strong bg-surface px-3 text-[12.5px] font-semibold hover:border-muted disabled:opacity-45"
        >
          <Terminal size={15} />
          Review files &amp; commands
        </button>
        <div className="flex">
          <button
            type="button"
            onClick={onUpdate}
            disabled={groups.length === 0}
            className="inline-flex h-[38px] flex-1 items-center justify-center gap-2 rounded-l-[3px] bg-accent px-3 text-[12.5px] font-semibold text-accent-ink hover:bg-accent-hover disabled:opacity-45"
          >
            <Play size={15} />
            {checks ? 'Update & run checks' : 'Update & install only'}
          </button>
          <button
            type="button"
            onClick={(e) => setMenu(menu ? null : e.currentTarget)}
            aria-label="Update options"
            aria-haspopup="menu"
            aria-expanded={!!menu}
            title="Update options"
            className={cx(
              'grid h-[38px] w-[38px] place-items-center rounded-r-[3px] border-l border-[color-mix(in_oklab,var(--accent-ink)_25%,transparent)] bg-accent text-accent-ink hover:bg-accent-hover',
              !!menu && 'bg-accent-hover',
            )}
          >
            <ChevronDown size={16} />
          </button>
        </div>
        {menu && (
          <Menu anchor={menu} label="Update options" placement="above-end" width={320} onClose={() => setMenu(null)}>
            <MenuTitle title="When you update" detail="Applies to every selected project" />
            <MenuCheck radio checked={checks} onSelect={() => (onOptions({ checks: true }), setMenu(null))} detail="Install, then build and test each project. A project whose checks fail is put back.">
              Update and run checks
            </MenuCheck>
            <MenuCheck radio checked={!checks} onSelect={() => (onOptions({ checks: false }), setMenu(null))} detail="Skip build and test. Useful when CI runs your tests.">
              Update and install only
            </MenuCheck>
            <MenuSeparator />
            <MenuCheck checked={commit} onSelect={() => onOptions({ commit: !commit })} detail="Commits only the files Mehen changed, once that project is done. Never pushes.">
              Commit each repository
            </MenuCheck>
          </Menu>
        )}
      </div>
    </section>
  )
}
