import { Asterisk, ChevronRight, FileX, Folder, FolderMinus, FolderPlus, Loader2, Plus, Search, X } from 'lucide-react'
import { useCallback, useEffect, useMemo, useRef, useState } from 'react'
import * as api from '../api'
import { isWithin, relativePath, samePath } from '../derive'
import type { DiscoveredProject, IgnoreKind, IgnoreRule, Settings } from '../types'
import { EcoBadge, cx } from './bits'

/** Folder names worth suggesting as patterns when they appear in the scan. */
const SUGGESTED_PATTERNS = ['fixtures', 'examples', 'samples', 'demo', '_spikes', 'temp', 'tmp', 'archive', 'playground', 'test-data']

interface Group {
  key: string
  path: string
  projects: DiscoveredProject[]
  ignoredCount: number
}

const KIND_ICON: Record<IgnoreKind, typeof Folder> = { folder: FolderMinus, project: FileX, pattern: Asterisk }
const KIND_LABEL: Record<IgnoreKind, string> = { folder: 'Folder', project: 'Project', pattern: 'Name pattern' }

export function FoldersPanel({
  settings,
  onSettings,
  onClose,
}: {
  settings: Settings
  onSettings: (s: Settings) => void
  /** `changed` is true when folders or rules were edited while open. */
  onClose: (changed: boolean) => void
}) {
  const [projects, setProjects] = useState<DiscoveredProject[] | null>(null)
  const [busy, setBusy] = useState(false)
  const [error, setError] = useState<string | null>(null)
  const [pattern, setPattern] = useState('')
  const [filter, setFilter] = useState('')
  const [expanded, setExpanded] = useState<Set<string>>(new Set())
  const changed = useRef(false)

  const rediscover = useCallback(async () => {
    setBusy(true)
    try {
      setProjects(await api.discover())
    } catch (e) {
      setError(String(e))
    } finally {
      setBusy(false)
    }
  }, [])

  useEffect(() => {
    rediscover()
  }, [rediscover])

  useEffect(() => {
    const onKey = (e: KeyboardEvent) => e.key === 'Escape' && onClose(changed.current)
    window.addEventListener('keydown', onKey)
    return () => window.removeEventListener('keydown', onKey)
  }, [onClose])

  const ruleById = useMemo(() => new Map(settings.rules.map((r) => [r.id, r])), [settings.rules])
  const roots = settings.folders

  const apply = async (work: () => Promise<Settings>, thenRediscover: boolean) => {
    setError(null)
    try {
      onSettings(await work())
      changed.current = true
      if (thenRediscover) await rediscover()
    } catch (e) {
      setError(String(e))
    }
  }

  // Adding a folder or project rule only hides things, so the preview can be
  // updated in place instead of walking the disk again.
  const ignore = async (kind: 'folder' | 'project', value: string) => {
    setError(null)
    try {
      const result = await api.addIgnore(kind, value)
      onSettings(result.settings)
      changed.current = true
      const rule = result.settings.rules.find((r) => r.kind === kind && samePath(r.value, value))
      if (rule && projects) {
        setProjects(
          projects.map((p) =>
            p.ignoredBy === null && (kind === 'project' ? samePath(p.manifest, value) : isWithin(p.dir, value)) ? { ...p, ignoredBy: rule.id } : p,
          ),
        )
      }
    } catch (e) {
      setError(String(e))
    }
  }

  const unignore = (id: number) => apply(() => api.removeIgnore(id), true)

  const addPattern = async (value: string) => {
    const v = value.trim()
    if (!v) return
    setPattern('')
    await apply(async () => (await api.addIgnore('pattern', v)).settings, true)
  }

  const addFolder = async () => {
    const picked = await api.pickFolder()
    if (picked) await apply(() => api.addFolder(picked), true)
  }

  const groups = useMemo<Group[]>(() => {
    const map = new Map<string, Group>()
    for (const p of projects ?? []) {
      const path = p.repo ?? p.dir
      const key = path.toLowerCase()
      const group = map.get(key) ?? { key, path, projects: [], ignoredCount: 0 }
      group.projects.push(p)
      if (p.ignoredBy !== null) group.ignoredCount++
      map.set(key, group)
    }
    return [...map.values()].sort((a, b) => a.path.localeCompare(b.path))
  }, [projects])

  const f = filter.trim().toLowerCase()
  const visibleGroups = f ? groups.filter((g) => g.path.toLowerCase().includes(f) || g.projects.some((p) => p.name.toLowerCase().includes(f))) : groups

  const suggestions = useMemo(() => {
    if (!projects) return []
    const existing = new Set(settings.rules.filter((r) => r.kind === 'pattern').map((r) => r.value.toLowerCase()))
    return SUGGESTED_PATTERNS.filter((name) => !existing.has(name))
      .map((name) => ({ name, count: projects.filter((p) => p.dir.toLowerCase().split(/[\\/]/).includes(name)).length }))
      .filter((s) => s.count > 0)
  }, [projects, settings.rules])

  const total = projects?.length ?? 0
  const included = projects?.filter((p) => p.ignoredBy === null).length ?? 0

  const describeRule = (rule: IgnoreRule | undefined) =>
    !rule ? 'a rule' : rule.kind === 'pattern' ? `pattern “${rule.value}”` : rule.kind === 'folder' ? `folder ${relativePath(roots, rule.value)}` : 'its own rule'

  const toggleGroup = (g: Group) => {
    if (g.ignoredCount < g.projects.length) {
      ignore('folder', g.path)
      return
    }
    // Bring the repo back by removing rules that only concern it; wider rules
    // (patterns, parent folders) are left for the user to remove on purpose.
    const own = [...new Set(g.projects.map((p) => p.ignoredBy))]
      .map((id) => (id === null ? undefined : ruleById.get(id)))
      .filter((r): r is IgnoreRule => !!r && r.kind !== 'pattern' && isWithin(r.value, g.path))
    if (own.length > 0) apply(async () => {
      let s = settings
      for (const r of own) s = await api.removeIgnore(r.id)
      return s
    }, true)
  }

  return (
    <div className="fixed inset-0 z-40 flex justify-end bg-black/55" onClick={() => onClose(changed.current)}>
      <aside
        className="flex h-full w-[min(1040px,100%)] flex-col border-l border-line-strong bg-bg shadow-2xl"
        onClick={(e) => e.stopPropagation()}
        role="dialog"
        aria-label="Folders and ignore rules"
      >
        <header className="flex items-center gap-3 border-b border-line px-5 py-3">
          <h2 className="flex-1 text-[15px] font-semibold">What Mehen watches</h2>
          <button type="button" onClick={() => onClose(changed.current)} aria-label="Close" className="rounded-md p-1 text-dim hover:bg-hover hover:text-ink">
            <X size={16} />
          </button>
        </header>

        {error && <div className="border-b border-[#5c2a1d] bg-carnelian-soft px-5 py-2 text-[12.5px] text-carnelian">{error}</div>}

        <div className="grid min-h-0 flex-1 grid-cols-[340px_1fr]">
          <div className="flex min-h-0 flex-col gap-6 overflow-y-auto border-r border-line px-5 py-4">
            <section>
              <div className="mb-2 flex items-center justify-between">
                <h3 className="text-[11.5px] font-medium uppercase tracking-wider text-dim">Watched folders</h3>
                <button type="button" onClick={addFolder} className="inline-flex items-center gap-1 text-[12px] text-gold hover:underline">
                  <FolderPlus size={13} /> Add folder
                </button>
              </div>
              {settings.folders.length === 0 && <p className="text-muted">No folders yet. Add the folder that holds your projects.</p>}
              <ul className="flex flex-col gap-1">
                {settings.folders.map((folder) => (
                  <li key={folder} className="group flex items-center gap-2 rounded-lg border border-line bg-panel px-2.5 py-1.5">
                    <Folder size={14} className="shrink-0 text-gold" />
                    <span className="min-w-0 flex-1 truncate font-mono text-[12px]" title={folder}>
                      {folder}
                    </span>
                    <button
                      type="button"
                      onClick={() => apply(() => api.removeFolder(folder), true)}
                      aria-label={`Stop watching ${folder}`}
                      className="rounded p-0.5 text-dim opacity-0 transition-opacity hover:text-carnelian group-hover:opacity-100 focus-visible:opacity-100"
                    >
                      <X size={13} />
                    </button>
                  </li>
                ))}
              </ul>
            </section>

            <section>
              <h3 className="mb-2 text-[11.5px] font-medium uppercase tracking-wider text-dim">Ignore rules</h3>
              <form
                className="flex gap-1.5"
                onSubmit={(e) => {
                  e.preventDefault()
                  addPattern(pattern)
                }}
              >
                <input
                  value={pattern}
                  onChange={(e) => setPattern(e.target.value)}
                  placeholder="Name or path pattern, e.g. fixtures"
                  className="h-8 min-w-0 flex-1 rounded-lg border border-line bg-panel px-2.5 text-[12.5px] placeholder:text-dim focus:border-line-strong"
                />
                <button type="submit" className="inline-flex h-8 items-center gap-1 rounded-lg border border-line-strong px-2.5 text-[12px] text-muted hover:text-ink">
                  <Plus size={13} /> Add
                </button>
              </form>
              <p className="mt-1.5 text-[11.5px] leading-relaxed text-dim">
                A plain name hides every folder with that name. A path like <span className="font-mono">nuget-compass/fixtures</span> hides only that one.
              </p>
              {suggestions.length > 0 && (
                <div className="mt-2 flex flex-wrap gap-1.5">
                  {suggestions.map((s) => (
                    <button
                      key={s.name}
                      type="button"
                      onClick={() => addPattern(s.name)}
                      className="rounded-md border border-dashed border-line-strong px-2 py-0.5 text-[11.5px] text-muted hover:border-gold hover:text-gold"
                      title={`Hide ${s.count} project${s.count === 1 ? '' : 's'} inside folders named ${s.name}`}
                    >
                      + {s.name} <span className="text-dim">({s.count})</span>
                    </button>
                  ))}
                </div>
              )}
              <ul className="mt-3 flex flex-col gap-1">
                {settings.rules.length === 0 && <li className="text-muted">Nothing ignored yet.</li>}
                {settings.rules.map((rule) => {
                  const Icon = KIND_ICON[rule.kind]
                  const hidden = projects?.filter((p) => p.ignoredBy === rule.id).length
                  return (
                    <li key={rule.id} className="group flex items-center gap-2 rounded-lg border border-line bg-panel px-2.5 py-1.5">
                      <Icon size={14} className="shrink-0 text-dim" aria-label={KIND_LABEL[rule.kind]} />
                      <div className="min-w-0 flex-1">
                        <div className="truncate font-mono text-[12px]" title={rule.value}>
                          {rule.kind === 'pattern' ? rule.value : relativePath(roots, rule.value)}
                        </div>
                        <div className="text-[11px] text-dim">
                          {KIND_LABEL[rule.kind]}
                          {hidden !== undefined && ` · hides ${hidden}`}
                        </div>
                      </div>
                      <button
                        type="button"
                        onClick={() => unignore(rule.id)}
                        aria-label="Remove rule"
                        className="rounded p-0.5 text-dim opacity-0 transition-opacity hover:text-ink group-hover:opacity-100 focus-visible:opacity-100"
                      >
                        <X size={13} />
                      </button>
                    </li>
                  )
                })}
              </ul>
            </section>
          </div>

          <div className="flex min-h-0 flex-col">
            <div className="flex items-center gap-3 border-b border-line px-5 py-2.5">
              <div className="flex-1">
                <div className="font-medium">Projects found</div>
                <div className="text-[11.5px] text-dim">
                  {projects ? `${included} of ${total} will be checked · no network used to find them` : 'Looking through your folders…'}
                </div>
              </div>
              {busy && <Loader2 size={15} className="animate-spin text-dim" />}
              <div className="relative">
                <Search size={13} className="pointer-events-none absolute left-2 top-1/2 -translate-y-1/2 text-dim" />
                <input
                  value={filter}
                  onChange={(e) => setFilter(e.target.value)}
                  placeholder="Filter"
                  className="h-7 w-44 rounded-md border border-line bg-panel pl-7 pr-2 text-[12px] placeholder:text-dim"
                />
              </div>
            </div>
            <div className="min-h-0 flex-1 overflow-y-auto">
              {visibleGroups.map((g) => {
                const open = expanded.has(g.key) || !!f
                const allIgnored = g.ignoredCount === g.projects.length
                const someIgnored = g.ignoredCount > 0 && !allIgnored
                const blocker = allIgnored ? ruleById.get(g.projects[0].ignoredBy!) : undefined
                const lockedByWiderRule = allIgnored && blocker && (blocker.kind === 'pattern' || !isWithin(blocker.value, g.path))
                return (
                  <div key={g.key} className="border-b border-line/60">
                    <div className={cx('flex items-center gap-2 px-5 py-2', allIgnored && 'opacity-55')}>
                      <button
                        type="button"
                        onClick={() => setExpanded((prev) => {
                          const next = new Set(prev)
                          if (next.has(g.key)) next.delete(g.key)
                          else next.add(g.key)
                          return next
                        })}
                        aria-expanded={open}
                        aria-label="Show projects"
                        className="rounded p-0.5 text-dim hover:text-ink"
                      >
                        <ChevronRight size={14} className={cx('transition-transform', open && 'rotate-90')} />
                      </button>
                      <input
                        type="checkbox"
                        checked={!allIgnored}
                        ref={(el) => {
                          if (el) el.indeterminate = someIgnored
                        }}
                        disabled={!!lockedByWiderRule}
                        onChange={() => toggleGroup(g)}
                        aria-label={`Check ${relativePath(roots, g.path)}`}
                        className="size-3.5 accent-[var(--color-gold)]"
                      />
                      <span className="min-w-0 flex-1 truncate font-mono text-[12.5px]" title={g.path}>
                        {relativePath(roots, g.path)}
                      </span>
                      {lockedByWiderRule && <span className="text-[11px] text-dim">hidden by {describeRule(blocker)}</span>}
                      <span className="text-[11.5px] text-dim">
                        {g.projects.length - g.ignoredCount}/{g.projects.length}
                      </span>
                    </div>
                    {open && (
                      <ul className="pb-2 pl-[70px] pr-5">
                        {g.projects.map((p) => {
                          const rule = p.ignoredBy === null ? undefined : ruleById.get(p.ignoredBy)
                          const ownRule = rule?.kind === 'project'
                          return (
                            <li key={p.id} className={cx('flex items-center gap-2 py-0.5', p.ignoredBy !== null && 'opacity-55')}>
                              <input
                                type="checkbox"
                                checked={p.ignoredBy === null}
                                disabled={p.ignoredBy !== null && !ownRule}
                                onChange={() => (p.ignoredBy === null ? ignore('project', p.manifest) : rule && unignore(rule.id))}
                                aria-label={`Check ${p.name}`}
                                className="size-3.5 accent-[var(--color-gold)]"
                              />
                              <EcoBadge ecosystem={p.ecosystem} />
                              <span className="min-w-0 truncate">{p.name}</span>
                              <span className="truncate text-[11px] text-dim">{relativePath([g.path], p.dir)}</span>
                              <span className="ml-auto shrink-0 text-[11px] text-dim">
                                {p.ignoredBy !== null && !ownRule ? `hidden by ${describeRule(rule)}` : `${p.dependencyCount} deps`}
                              </span>
                            </li>
                          )
                        })}
                      </ul>
                    )}
                  </div>
                )
              })}
            </div>
          </div>
        </div>

        <footer className="flex items-center gap-3 border-t border-line px-5 py-3">
          <span className="flex-1 text-[12px] text-dim">Changes apply to the next check. Hidden projects are skipped without being read.</span>
          <button
            type="button"
            onClick={() => onClose(changed.current)}
            className="rounded-lg bg-gold px-4 py-1.5 text-[12.5px] font-semibold text-[#1d1506] hover:brightness-110"
          >
            Done
          </button>
        </footer>
      </aside>
    </div>
  )
}
