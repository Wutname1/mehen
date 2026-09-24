import { Code2, EyeOff, Folder, FolderOpen, FolderPlus, ListChecks, MoreHorizontal, RefreshCw, Search, Settings2, Undo2 } from 'lucide-react'
import { useEffect, useMemo, useState } from 'react'
import * as api from '../api'
import { ECOSYSTEMS, ECOSYSTEM_LABEL, folderName, isWithin, samePath, type Repo } from '../derive'
import type { DiscoveredProject, Ecosystem, IgnoreRule, Settings } from '../types'
import { cx } from './bits'
import { Button, Dialog } from './Dialog'
import { Menu, MenuItem, MenuSeparator, MenuTitle } from './Menu'
import { Checkbox } from './Queue'
import { TextInput } from './controls'
import { RepoAvatar } from './RepoAvatar'

const ECO_SHORT: Record<Ecosystem, string> = { npm: 'npm', cargo: 'Rs', nuget: 'Nu', 'github-actions': 'GA' }

/** A repository as found on disk, excluded or not. */
interface Entry {
  key: string
  name: string
  root: string | null
  ecosystems: Ecosystem[]
  manifests: number
  /** Rules hiding every manifest; empty when the project is checked. */
  rules: IgnoreRule[]
  /** Some manifests are hidden, not all. */
  partly: boolean
  checked: Repo | undefined
}

export function ManageProjects({
  settings,
  repos,
  icons,
  onOpen,
  onScan,
  onAddFolder,
  onRemoveFolder,
  onProjectSettings,
  onExclude,
  onExcludeMany,
  onInclude,
  onReveal,
  onOpenInEditor,
  onClose,
}: {
  settings: Settings
  repos: Repo[]
  icons: Record<string, string>
  onOpen: (key: string) => void
  onScan: (paths: string[]) => Promise<void>
  onAddFolder: () => void
  onRemoveFolder: (folder: string) => void
  onProjectSettings: (repo: Repo) => void
  onExclude: (repo: Repo) => void
  onExcludeMany: (keys: string[]) => void
  onInclude: (rules: IgnoreRule[], keys: string[]) => Promise<void>
  onReveal: (path: string) => void
  onOpenInEditor: (path: string) => void
  onClose: () => void
}) {
  const [found, setFound] = useState<DiscoveredProject[] | null>(null)
  const [query, setQuery] = useState('')
  const [selected, setSelected] = useState<Set<string>>(new Set())
  const [busy, setBusy] = useState<Set<string>>(new Set())
  const [menu, setMenu] = useState<{ entry: Entry; anchor: HTMLElement } | null>(null)
  const [error, setError] = useState<string | null>(null)

  const version = `${settings.folders.join('|')}#${settings.rules.map((r) => r.id).join(',')}`
  useEffect(() => {
    let cancelled = false
    api
      .discover()
      .then((list) => !cancelled && setFound(list))
      .catch((e) => !cancelled && setError(String(e)))
    return () => {
      cancelled = true
    }
  }, [version])

  const entries = useMemo(() => {
    const ruleById = new Map(settings.rules.map((r) => [r.id, r]))
    const repoByKey = new Map(repos.map((r) => [r.key.toLowerCase(), r]))
    const map = new Map<string, { key: string; projects: DiscoveredProject[] }>()
    for (const p of found ?? []) {
      const key = p.repo ?? p.dir
      const e = map.get(key.toLowerCase()) ?? { key, projects: [] }
      e.projects.push(p)
      map.set(key.toLowerCase(), e)
    }
    return [...map.values()]
      .map(({ key, projects }): Entry => {
        const hidden = projects.filter((p) => p.ignoredBy !== null)
        const all = hidden.length === projects.length
        const checked = repoByKey.get(key.toLowerCase())
        return {
          key,
          name: checked?.name ?? folderName(key),
          root: settings.folders.find((f) => isWithin(key, f)) ?? null,
          ecosystems: ECOSYSTEMS.filter((e) => projects.some((p) => p.ecosystem === e)),
          manifests: projects.length,
          rules: all ? [...new Set(hidden.map((p) => p.ignoredBy!))].map((id) => ruleById.get(id)).filter((r): r is IgnoreRule => !!r) : [],
          partly: hidden.length > 0 && !all,
          checked,
        }
      })
      .sort((a, b) => a.name.localeCompare(b.name, undefined, { sensitivity: 'base' }))
  }, [found, repos, settings.rules, settings.folders])

  const q = query.trim().toLowerCase()
  const groups = settings.folders.map((folder) => {
    const inFolder = entries.filter((e) => e.root === folder)
    const match = !q || folder.toLowerCase().includes(q)
    return { folder, all: inFolder, shown: match ? inFolder : inFolder.filter((e) => e.name.toLowerCase().includes(q) || e.key.toLowerCase().includes(q)), hidden: !!q && !match && !inFolder.some((e) => e.name.toLowerCase().includes(q) || e.key.toLowerCase().includes(q)) }
  })
  const shownKeys = groups.flatMap((g) => g.shown.map((e) => e.key))
  const picked = entries.filter((e) => selected.has(e.key))
  const own = (e: Entry) => e.rules.length > 0 && e.rules.every((r) => r.kind === 'folder' && samePath(r.value, e.key))

  const toggle = (keys: string[]) =>
    setSelected((prev) => {
      const next = new Set(prev)
      const all = keys.every((k) => next.has(k))
      for (const k of keys) {
        if (all) next.delete(k)
        else next.add(k)
      }
      return next
    })

  const scan = async (paths: string[]) => {
    setBusy((prev) => new Set([...prev, ...paths.map((p) => p.toLowerCase())]))
    try {
      await onScan(paths)
    } catch (e) {
      setError(String(e))
    } finally {
      setBusy((prev) => new Set([...prev].filter((k) => !paths.some((p) => p.toLowerCase() === k))))
    }
  }

  const status = (e: Entry): [string, string] => {
    if (busy.has(e.key.toLowerCase()) || (e.root && busy.has(e.root.toLowerCase()))) return ['Scanning…', 'text-accent-text']
    if (e.rules.length) return own(e) ? ['Excluded', 'text-muted'] : [`Hidden by ${e.rules.map((r) => (r.kind === 'pattern' ? `“${r.value}”` : folderName(r.value))).join(', ')}`, 'text-muted']
    if (!e.checked) return ['Not checked yet', 'text-muted']
    if (e.checked.vulnerable) return [`${e.checked.vulnerable} vulnerable`, 'text-risk-security']
    if (e.checked.updates) return [`${e.checked.updates} update${e.checked.updates === 1 ? '' : 's'}`, 'text-risk-review']
    return ['Up to date', 'text-ok']
  }

  const excluded = picked.filter((e) => e.rules.length > 0)
  const included = picked.filter((e) => e.rules.length === 0)
  const grid = 'grid grid-cols-[32px_minmax(0,1fr)_150px_120px_34px] items-center'

  return (
    <Dialog
      title="Manage projects"
      description="The folders Mehen watches and the projects found in them, including ones you excluded."
      icon={<ListChecks size={22} />}
      size="wide"
      onClose={onClose}
      footer={
        <div className="flex w-full items-center gap-2">
          <span className="mr-auto text-[12.5px] text-muted">
            {picked.length ? `${picked.length} selected` : `${entries.length} projects in ${settings.folders.length} folder${settings.folders.length === 1 ? '' : 's'}`}
          </span>
          {picked.length === 1 && picked[0].checked && (
            <Button onClick={() => onProjectSettings(picked[0].checked!)}>
              <Settings2 size={15} />
              Project settings…
            </Button>
          )}
          {included.length > 0 && (
            <>
              <Button onClick={() => scan(included.map((e) => e.key))} disabled={included.some((e) => busy.has(e.key.toLowerCase()))}>
                <RefreshCw size={14} />
                Scan selected
              </Button>
              <Button
                onClick={() => {
                  onExcludeMany(included.map((e) => e.key))
                  setSelected(new Set())
                }}
                className="border-[color-mix(in_oklab,var(--risk-security)_55%,transparent)] text-risk-security"
              >
                <EyeOff size={15} />
                Exclude {included.length === 1 ? '' : included.length}
              </Button>
            </>
          )}
          {excluded.some(own) && (
            <Button
              onClick={async () => {
                const list = excluded.filter(own)
                await onInclude(
                  list.flatMap((e) => e.rules),
                  list.map((e) => e.key),
                )
                setSelected(new Set())
              }}
            >
              <Undo2 size={14} />
              Include again
            </Button>
          )}
        </div>
      }
    >
      <div className="mb-2.5 flex gap-2">
        <label className="relative flex-1">
          <Search size={15} className="pointer-events-none absolute top-[9px] left-2.5 text-muted" />
          <span className="sr-only">Search folders and projects</span>
          <TextInput type="search" autoFocus value={query} onChange={(e) => setQuery(e.target.value)} placeholder="Search folders and projects" className="w-full pl-8" />
        </label>
        <Button onClick={onAddFolder} className="h-[34px]">
          <FolderPlus size={15} />
          Add a folder…
        </Button>
      </div>
      {error && <p className="mb-2 rounded-[3px] bg-vuln-row px-3 py-2 text-[12.5px] text-risk-security">{error}</p>}
      {!found ? (
        <p className="py-6 text-center text-muted">Looking through your folders…</p>
      ) : (
        <div role="table" aria-label="Folders and projects">
          <div role="row" className={cx(grid, 'min-h-8 font-mono text-[11px] tracking-[0.04em] text-muted uppercase')}>
            <span role="columnheader" className="grid place-items-center">
              <Checkbox
                checked={shownKeys.length > 0 && shownKeys.every((k) => selected.has(k))}
                partial={shownKeys.some((k) => selected.has(k))}
                onChange={() => toggle(shownKeys)}
                disabled={!shownKeys.length}
                label="Select every shown project"
              />
            </span>
            <span role="columnheader">Project</span>
            <span role="columnheader">Status</span>
            <span role="columnheader">Uses</span>
            <span role="columnheader">
              <span className="sr-only">Actions</span>
            </span>
          </div>
          {groups
            .filter((g) => !g.hidden)
            .map(({ folder, all, shown }) => {
              const keys = shown.map((e) => e.key)
              const scanning = busy.has(folder.toLowerCase())
              return (
                <div key={folder}>
                  <div role="row" className="mt-2.5 grid min-h-12 grid-cols-[32px_minmax(0,1fr)_auto] items-center rounded-t-[3px] border border-line bg-paper-2">
                    <span role="cell" className="grid place-items-center">
                      <Checkbox
                        checked={keys.length > 0 && keys.every((k) => selected.has(k))}
                        partial={keys.some((k) => selected.has(k))}
                        onChange={() => toggle(keys)}
                        disabled={!keys.length}
                        label={`Select every project in ${folder}`}
                      />
                    </span>
                    <span role="cell" className="flex min-w-0 items-center gap-2.5">
                      <Folder size={16} className="shrink-0 text-state" />
                      <span className="flex min-w-0 flex-col">
                        <b className="truncate font-mono text-[12.5px] font-semibold">{folder}</b>
                        <small className="text-[12px] text-muted">{all.length ? `${all.length} project${all.length === 1 ? '' : 's'}, including subfolders` : 'No projects found here yet'}</small>
                      </span>
                    </span>
                    <span role="cell" className="flex gap-1 pr-2">
                      <Button onClick={() => scan([folder])} disabled={scanning}>
                        <RefreshCw size={14} className={cx(scanning && 'animate-spin')} />
                        {scanning ? 'Scanning…' : 'Scan'}
                      </Button>
                      <Button variant="ghost" onClick={() => onRemoveFolder(folder)} aria-label={`Stop watching ${folder}`}>
                        Remove
                      </Button>
                    </span>
                  </div>
                  {shown.map((e) => {
                    const [label, tone] = status(e)
                    const off = e.rules.length > 0
                    return (
                      <div
                        key={e.key}
                        role="row"
                        className={cx(grid, 'min-h-[52px] border-b border-line', selected.has(e.key) && 'bg-row-selected')}
                        onContextMenu={(ev) => {
                          ev.preventDefault()
                          setMenu({ entry: e, anchor: ev.currentTarget.querySelector('[data-more]') as HTMLElement })
                        }}
                      >
                        <span role="cell" className="grid place-items-center">
                          <Checkbox checked={selected.has(e.key)} onChange={() => toggle([e.key])} label={`Select ${e.name}`} />
                        </span>
                        <span role="cell" className={cx('min-w-0', off && 'opacity-60')}>
                          <button
                            type="button"
                            onClick={() => !off && onOpen(e.key)}
                            disabled={off}
                            className="group flex min-w-0 items-center gap-2.5 py-1 pr-2 text-left disabled:cursor-default"
                          >
                            <RepoAvatar name={e.name} icon={icons[e.key.toLowerCase()]} />
                            <span className="flex min-w-0 flex-col">
                              <b className={cx('truncate text-[13px]', !off && 'group-hover:underline group-hover:underline-offset-2')}>{e.name}</b>
                              <small className="truncate font-mono text-[12px] text-muted">
                                {e.key}
                                {e.partly && ' · some manifests excluded'}
                              </small>
                            </span>
                          </button>
                        </span>
                        <span role="cell" className={cx('truncate pr-2 text-[12px] font-semibold', tone)} title={label}>
                          {label}
                        </span>
                        <span role="cell" className="flex gap-[3px]" aria-label={e.ecosystems.map((x) => ECOSYSTEM_LABEL[x]).join(', ')}>
                          {e.ecosystems.map((x) => (
                            <i key={x} title={ECOSYSTEM_LABEL[x]} className="inline-grid h-4 min-w-5 place-items-center rounded-[2px] border border-line-strong bg-paper px-[3px] font-mono text-[10.5px] leading-none font-bold not-italic text-ink">
                              {ECO_SHORT[x]}
                            </i>
                          ))}
                        </span>
                        <span role="cell">
                          <button
                            type="button"
                            data-more
                            onClick={(ev) => setMenu({ entry: e, anchor: ev.currentTarget })}
                            aria-label={`${e.name} actions`}
                            aria-haspopup="menu"
                            className="grid size-[30px] place-items-center rounded-[3px] text-muted hover:bg-sunken hover:text-ink"
                          >
                            <MoreHorizontal size={16} />
                          </button>
                        </span>
                      </div>
                    )
                  })}
                </div>
              )
            })}
        </div>
      )}
      {menu && (
        <Menu anchor={menu.anchor} label={`${menu.entry.name} actions`} placement="below-end" onClose={() => setMenu(null)} width={250}>
          <MenuTitle title={menu.entry.name} detail={menu.entry.key} />
          <MenuItem icon={<FolderOpen size={15} />} onSelect={() => (setMenu(null), onReveal(menu.entry.key))}>
            Show in File Explorer
          </MenuItem>
          <MenuItem icon={<Code2 size={15} />} onSelect={() => (setMenu(null), onOpenInEditor(menu.entry.key))}>
            Open in VS Code
          </MenuItem>
          {menu.entry.checked && (
            <MenuItem icon={<Settings2 size={15} />} onSelect={() => (setMenu(null), onProjectSettings(menu.entry.checked!))}>
              Project settings…
            </MenuItem>
          )}
          {!menu.entry.rules.length && (
            <MenuItem icon={<RefreshCw size={15} />} onSelect={() => (setMenu(null), scan([menu.entry.key]))}>
              Scan this project
            </MenuItem>
          )}
          <MenuSeparator />
          {menu.entry.rules.length ? (
            <MenuItem icon={<Undo2 size={15} />} disabled={!own(menu.entry)} onSelect={() => (setMenu(null), onInclude(menu.entry.rules, [menu.entry.key]))}>
              Include again
            </MenuItem>
          ) : (
            menu.entry.checked && (
              <MenuItem icon={<EyeOff size={15} />} danger onSelect={() => (setMenu(null), onExclude(menu.entry.checked!))}>
                Exclude…
              </MenuItem>
            )
          )}
        </Menu>
      )}
    </Dialog>
  )
}
