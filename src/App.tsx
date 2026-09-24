import { Moon, Search, Sun, TriangleAlert, Undo2, X } from 'lucide-react'
import { useCallback, useEffect, useMemo, useRef, useState } from 'react'
import * as api from './api'
import { AdvisoryDialog } from './components/AdvisoryDialog'
import { Logo, cx } from './components/bits'
import { BulkUpdateDialog, type BulkTarget } from './components/BulkUpdateDialog'
import { FoldersPanel } from './components/FoldersPanel'
import { ProjectRecord, Tray, type TrayGroup } from './components/Inspector'
import { Queue, type QueueFilters, type ScopedRow } from './components/Queue'
import { Rail, ScanStatus } from './components/Rail'
import { RISK_ORDER, projectTypes, queueRows, repoKey, repos as groupRepos, samePath, type QueueRow, type QueueUsage, type Repo } from './derive'
import type { Change, IgnoreKind, Inventory, Progress, Settings } from './types'

const SUGGESTED_ROOT = 'C:\\code'

interface IgnoreRequest {
  kind: Extract<IgnoreKind, 'folder' | 'project'>
  value: string
  label: string
}

interface Notice {
  text: string
  undo?: () => void
}

type Theme = 'dark' | 'light'
export type Palette = 'faience' | 'parchment'

function stored<T extends string>(key: string, allowed: T[], fallback: T): T {
  try {
    const v = localStorage.getItem(key) as T | null
    return v && allowed.includes(v) ? v : fallback
  } catch {
    return fallback
  }
}

/** Theme and palette live on <html> so every token follows them. */
function useAppearance() {
  const [theme, setTheme] = useState<Theme>(() => stored('mehen-theme', ['dark', 'light'], 'dark'))
  const [palette, setPalette] = useState<Palette>(() => stored('mehen-palette', ['faience', 'parchment'], 'faience'))
  useEffect(() => {
    document.documentElement.dataset.theme = theme
    document.documentElement.dataset.palette = palette
    try {
      localStorage.setItem('mehen-theme', theme)
      localStorage.setItem('mehen-palette', palette)
    } catch {
      // Private storage: the choice lasts for this session only.
    }
  }, [theme, palette])
  return { theme, setTheme, palette, setPalette }
}

function timeAgo(unixSeconds: number | null | undefined): string {
  if (!unixSeconds) return 'never'
  const s = Math.max(0, Math.round(Date.now() / 1000 - unixSeconds))
  if (s < 60) return 'just now'
  if (s < 3600) return `${Math.round(s / 60)} min ago`
  if (s < 86400) return `${Math.round(s / 3600)} h ago`
  return `${Math.round(s / 86400)} d ago`
}

const plural = (n: number, word: string) => `${n} ${word}${n === 1 ? '' : 's'}`

export default function App() {
  const { theme, setTheme } = useAppearance()
  const [settings, setSettings] = useState<Settings | null>(null)
  const [panelOpen, setPanelOpen] = useState(false)
  const [notice, setNotice] = useState<Notice | null>(null)
  const [inventory, setInventory] = useState<Inventory | null>(null)
  const [loading, setLoading] = useState(true)
  const [running, setRunning] = useState(false)
  const [progress, setProgress] = useState<Progress | null>(null)
  const [error, setError] = useState<string | null>(null)

  const [repoScope, setRepoScope] = useState<string | null>(null)
  const [query, setQuery] = useState('')
  const [filters, setFilters] = useState<QueueFilters>({ types: new Set(), ecosystems: new Set(), risk: 'any', riskFirst: true })
  const [selected, setSelected] = useState<Set<string>>(new Set())
  const [advisory, setAdvisory] = useState<QueueRow | null>(null)
  const [bulk, setBulk] = useState<{ title: string; targets: BulkTarget[] } | null>(null)
  const searchRef = useRef<HTMLInputElement>(null)

  useEffect(() => {
    let cancelled = false
    Promise.all([api.settings(), api.lastInventory()])
      .then(([s, inv]) => {
        if (cancelled) return
        setSettings(s)
        setInventory(inv)
      })
      .catch((e) => !cancelled && setError(String(e)))
      .finally(() => !cancelled && setLoading(false))
    return () => {
      cancelled = true
    }
  }, [])

  useEffect(() => {
    if (!notice) return
    const t = window.setTimeout(() => setNotice(null), notice.undo ? 8000 : 4500)
    return () => window.clearTimeout(t)
  }, [notice])

  useEffect(() => {
    const unlisten = api.onInventory(setInventory)
    return () => {
      unlisten.then((fn) => fn())
    }
  }, [])

  useEffect(() => {
    const unlisten = api.onProgress(setProgress)
    return () => {
      unlisten.then((fn) => fn())
    }
  }, [])

  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      if ((e.ctrlKey || e.metaKey) && e.key.toLowerCase() === 'k') {
        e.preventDefault()
        searchRef.current?.focus()
        searchRef.current?.select()
      }
    }
    window.addEventListener('keydown', onKey)
    return () => window.removeEventListener('keydown', onKey)
  }, [])

  const run = async (refresh: boolean) => {
    setRunning(true)
    setError(null)
    setProgress({ phase: 'Finding projects', done: 0, total: 0 })
    try {
      setInventory(await api.scanAndCheck(refresh))
    } catch (e) {
      setError(String(e))
    } finally {
      setRunning(false)
      setProgress(null)
    }
  }

  const watchFolder = async (path: string | null) => {
    if (!path) return
    try {
      setSettings(await api.addFolder(path))
      await run(false)
    } catch (e) {
      setError(String(e))
    }
  }

  const ignore = async ({ kind, value, label }: IgnoreRequest) => {
    try {
      const result = await api.addIgnore(kind, value)
      setSettings(result.settings)
      if (result.inventory) setInventory(result.inventory)
      const rule = result.settings.rules.find((r) => r.kind === kind && samePath(r.value, value))
      setNotice({
        text: `${label} excluded. Mehen will skip it from now on.`,
        undo: rule
          ? async () => {
              setNotice(null)
              setSettings(await api.removeIgnore(rule.id))
              await run(false)
            }
          : undefined,
      })
    } catch (e) {
      setError(String(e))
    }
  }

  const closePanel = (changed: boolean) => {
    setPanelOpen(false)
    if (changed) run(false)
  }

  const rows = useMemo(() => (inventory ? queueRows(inventory) : []), [inventory])
  const repoList = useMemo(() => (inventory ? groupRepos(inventory, rows) : []), [inventory, rows])
  const repoByKey = useMemo(() => new Map(repoList.map((r) => [r.key.toLowerCase(), r])), [repoList])
  const repo = repoScope ? (repoByKey.get(repoScope.toLowerCase()) ?? null) : null
  const repoOf = useCallback((u: QueueUsage) => repoByKey.get(repoKey(u.project).toLowerCase()), [repoByKey])

  // Drop selections and scope that no longer exist after a check.
  useEffect(() => {
    const live = new Set(rows.flatMap((r) => r.usages.map((u) => u.key)))
    setSelected((prev) => {
      const next = new Set([...prev].filter((k) => live.has(k)))
      return next.size === prev.size ? prev : next
    })
    if (repoScope && !repoByKey.has(repoScope.toLowerCase())) setRepoScope(null)
  }, [rows, repoByKey, repoScope])

  /** Usages the scope, project-type and dependency-type filters allow. */
  const inScope = useCallback(
    (row: QueueRow) =>
      row.usages.filter((u) => {
        if (repo && !samePath(repoKey(u.project), repo.key)) return false
        if (!repo && filters.types.size && !projectTypes(repoOf(u)?.ecosystems ?? []).some((t) => filters.types.has(t))) return false
        return !filters.ecosystems.size || filters.ecosystems.has(row.ecosystem)
      }),
    [repo, filters.types, filters.ecosystems, repoOf],
  )

  const q = query.trim().toLowerCase()
  const visible: ScopedRow[] = useMemo(() => {
    const list = rows
      .map((row) => ({ row, usages: inScope(row) }))
      .filter(({ usages }) => usages.length > 0)
      .filter(({ row }) => filters.risk === 'any' || (filters.risk === 'security' ? row.risk === 'security' : row.risk === 'security' || row.risk === 'major'))
      .map(({ row, usages }) => ({
        row,
        usages: !q || row.name.toLowerCase().includes(q) ? usages : usages.filter((u) => u.project.name.toLowerCase().includes(q) || u.project.dir.toLowerCase().includes(q)),
      }))
      .filter(({ usages }) => usages.length > 0)
    const byName = (a: ScopedRow, b: ScopedRow) => a.row.name.localeCompare(b.row.name, undefined, { sensitivity: 'base' })
    return list.sort((a, b) => (filters.riskFirst ? RISK_ORDER.indexOf(a.row.risk) - RISK_ORDER.indexOf(b.row.risk) : 0) || byName(a, b))
  }, [rows, inScope, filters.risk, filters.riskFirst, q])

  const security = useMemo(() => {
    const usages = rows.filter((r) => r.vulnIds.length > 0).flatMap((r) => inScope(r).filter((u) => u.dep.vulns.length > 0))
    if (!usages.length) return null
    const packages = new Set(usages.map((u) => `${u.dep.ecosystem}:${u.dep.name}`)).size
    const projects = new Set(usages.map((u) => repoKey(u.project).toLowerCase())).size
    return { packages, projects, usages, allSelected: usages.every((u) => selected.has(u.key)) }
  }, [rows, inScope, selected])

  const addAll = (usages: QueueUsage[], message: (n: number) => string) => {
    const added = usages.map((u) => u.key).filter((k) => !selected.has(k))
    if (!added.length) return
    setSelected((prev) => new Set([...prev, ...added]))
    const packages = new Set(usages.filter((u) => added.includes(u.key)).map((u) => `${u.dep.ecosystem}:${u.dep.name}`)).size
    setNotice({
      text: message(packages),
      undo: () => {
        setNotice(null)
        setSelected((prev) => new Set([...prev].filter((k) => !added.includes(k))))
      },
    })
  }

  const toggleUsages = (usages: QueueUsage[]) =>
    setSelected((prev) => {
      const next = new Set(prev)
      const all = usages.every((u) => next.has(u.key))
      for (const u of usages) {
        if (all) next.delete(u.key)
        else next.add(u.key)
      }
      return next
    })

  const trayGroups: TrayGroup[] = useMemo(
    () => rows.map((row) => ({ key: row.key, name: row.name, usages: row.usages.filter((u) => selected.has(u.key)) })).filter((g) => g.usages.length > 0),
    [rows, selected],
  )

  const removeGroup = (g: TrayGroup) => {
    const removed = g.usages.map((u) => u.key)
    setSelected((prev) => new Set([...prev].filter((k) => !removed.includes(k))))
    setNotice({ text: `${g.name} removed from the selection.`, undo: () => (setNotice(null), setSelected((prev) => new Set([...prev, ...removed]))) })
  }

  const clearSelection = () => {
    const removed = [...selected]
    setSelected(new Set())
    setNotice({ text: `${plural(trayGroups.length, 'update')} removed from the selection.`, undo: () => (setNotice(null), setSelected(new Set(removed))) })
  }

  const review = () => {
    const byProject = new Map<string, BulkTarget>()
    for (const u of trayGroups.flatMap((g) => g.usages)) {
      const entry = byProject.get(u.project.id) ?? { project: u.project, changes: [] as Change[] }
      if (!entry.changes.some((c) => c.name === u.dep.name && c.from === u.dep.requested)) entry.changes.push({ name: u.dep.name, from: u.dep.requested, to: u.target })
      byProject.set(u.project.id, entry)
    }
    setBulk({ title: plural(trayGroups.length, 'update'), targets: [...byProject.values()] })
  }

  const excludedCount = settings?.rules.length ?? 0
  const repoCount = repoList.length
  const folderCount = settings?.folders.length ?? 0
  const warnings = inventory ? [...(inventory.checkStats?.throttled.map((t) => `${t} is rate limiting; some packages were not checked`) ?? []), ...inventory.warnings] : []

  return (
    <div className="flex h-full flex-col">
      <header className="on-rail flex h-[52px] shrink-0 items-center gap-5 border-b border-bar-line bg-bar pr-3.5 pl-4 text-rail-ink">
        <div className="flex w-[222px] items-center gap-2.5 max-[1100px]:w-[182px]">
          <Logo size={28} />
          <strong className="font-display text-[16px] font-semibold tracking-[-0.015em]">Mehen</strong>
        </div>
        <label className="ml-auto flex h-[34px] w-[min(440px,40vw)] items-center gap-2 rounded-[3px] border border-rail-border bg-rail-field pr-2 pl-2.5 text-rail-muted focus-within:border-rail-focus">
          <Search size={16} className="shrink-0" />
          <span className="sr-only">Search projects and packages</span>
          <input
            ref={searchRef}
            value={query}
            onChange={(e) => setQuery(e.target.value)}
            onKeyDown={(e) => e.key === 'Escape' && setQuery('')}
            placeholder="Search projects and packages"
            spellCheck={false}
            className="h-full min-w-0 flex-1 border-0 bg-transparent text-[13px] text-rail-ink outline-none placeholder:text-rail-placeholder"
          />
          {query ? (
            <button type="button" onClick={() => setQuery('')} aria-label="Clear search" className="grid size-6 place-items-center rounded-[3px] text-rail-muted hover:text-rail-ink">
              <X size={14} />
            </button>
          ) : (
            <kbd aria-hidden className="rounded-[2px] border border-rail-border px-1.5 py-0.5 font-mono text-[11px] text-rail-muted">
              Ctrl K
            </kbd>
          )}
        </label>
        <button
          type="button"
          onClick={() => setTheme(theme === 'dark' ? 'light' : 'dark')}
          aria-label={`${theme === 'dark' ? 'Dark' : 'Light'} theme. Switch to ${theme === 'dark' ? 'light' : 'dark'}`}
          className="inline-flex h-[34px] items-center gap-2 rounded-[3px] border border-rail-border px-2.5 text-[12px] text-rail-ink hover:border-rail-border-strong hover:bg-rail-field-hover"
        >
          {theme === 'dark' ? <Moon size={15} /> : <Sun size={15} />}
          {theme === 'dark' ? 'Dark' : 'Light'}
        </button>
      </header>

      {running && progress && <ProgressBar progress={progress} />}

      {error && (
        <div className="flex items-center gap-2 border-b border-line bg-vuln-row px-4 py-2 text-[12.5px] text-risk-security">
          <TriangleAlert size={14} />
          <span className="flex-1">{error}</span>
          <button type="button" onClick={() => setError(null)} aria-label="Dismiss" className="opacity-70 hover:opacity-100">
            <X size={14} />
          </button>
        </div>
      )}

      {!inventory ? (
        <Welcome
          loading={loading}
          running={running}
          folders={settings?.folders ?? []}
          onRun={() => run(false)}
          onWatch={watchFolder}
          onChoose={async () => watchFolder(await api.pickFolder())}
        />
      ) : (
        <div className="grid min-h-0 flex-1 grid-cols-[256px_minmax(0,1fr)_316px] max-[1279px]:grid-cols-[232px_minmax(0,1fr)_284px] max-[1100px]:grid-cols-[216px_minmax(0,1fr)_264px]">
          <Rail
            repos={repoList}
            roots={inventory.roots}
            selected={repo?.key ?? null}
            query={query}
            excludedCount={excludedCount}
            onSelect={setRepoScope}
            onManage={() => setPanelOpen(true)}
            onAddFolder={async () => watchFolder(await api.pickFolder())}
            onExclusions={() => setPanelOpen(true)}
            onRefreshAll={() => run(true)}
            onReveal={(path) => api.reveal(path)}
            onOpenInEditor={(path) => api.openInEditor(path)}
            onIgnoreRepo={(r: Repo) => ignore({ kind: 'folder', value: r.key, label: r.name })}
            footer={
              <ScanStatus
                running={running}
                phase={progress?.phase ?? null}
                checkedAgo={timeAgo(inventory.checkedAt)}
                detail={warnings.length ? plural(warnings.length, 'warning') : `${plural(repoCount, 'project')} · ${plural(folderCount, 'folder')}`}
                hint={warnings.length ? warnings.join('\n') : undefined}
                onScan={() => run(false)}
              />
            }
          />
          <Queue
            rows={visible}
            repo={repo}
            filters={filters}
            onFilters={setFilters}
            selected={selected}
            onToggle={toggleUsages}
            onSelectCompatible={() =>
              addAll(
                visible.filter((r) => r.row.risk !== 'major').flatMap((r) => r.usages),
                (n) => `${plural(n, 'compatible update')} selected.`,
              )
            }
            onClearScope={() => setRepoScope(null)}
            security={security && { packages: security.packages, projects: security.projects, allSelected: security.allSelected }}
            onSelectFixes={() => security && addAll(security.usages, (n) => `${n} security fix${n === 1 ? '' : 'es'} selected.`)}
            onAdvisory={setAdvisory}
            searching={!!q}
            onClearSearch={() => setQuery('')}
          />
          <aside className="flex min-h-0 flex-col overflow-y-auto border-l border-line bg-paper-2" aria-label="Project and selected updates">
            {repo && <ProjectRecord repo={repo} onReveal={() => api.reveal(repo.key)} onOpenInEditor={() => api.openInEditor(repo.key)} />}
            <Tray groups={trayGroups} repo={repo} onRemove={removeGroup} onClear={clearSelection} onUpdate={review} />
          </aside>
        </div>
      )}

      {panelOpen && settings && <FoldersPanel settings={settings} onSettings={setSettings} onClose={closePanel} />}

      {advisory && inventory && (
        <AdvisoryDialog
          row={advisory}
          inventory={inventory}
          allSelected={advisory.usages.filter((u) => u.dep.vulns.length > 0).every((u) => selected.has(u.key))}
          onSelect={() => {
            addAll(
              advisory.usages.filter((u) => u.dep.vulns.length > 0),
              () => `${advisory.name} fix selected.`,
            )
            setAdvisory(null)
          }}
          onClose={() => setAdvisory(null)}
        />
      )}

      {bulk && inventory && (
        <BulkUpdateDialog
          title={bulk.title}
          targets={bulk.targets}
          roots={inventory.roots}
          onClose={(refreshed) => {
            setBulk(null)
            if (refreshed) setInventory(refreshed)
          }}
        />
      )}

      {notice && (
        <div
          role="status"
          className="on-rail fixed bottom-5 left-1/2 z-[70] flex max-w-[520px] -translate-x-1/2 items-center gap-2.5 rounded-[4px] border border-rail-border bg-rail-raised py-2 pr-2 pl-3 text-[12.5px] text-rail-ink shadow-[var(--shadow)]"
        >
          <span>{notice.text}</span>
          {notice.undo && (
            <button type="button" onClick={notice.undo} className="inline-flex h-7 items-center gap-1 rounded-[3px] px-2 font-semibold text-rail-link hover:bg-rail-hover">
              <Undo2 size={14} />
              Undo
            </button>
          )}
          <button type="button" onClick={() => setNotice(null)} aria-label="Dismiss" className="grid h-7 place-items-center rounded-[3px] px-1.5 text-rail-muted hover:bg-rail-hover hover:text-rail-ink">
            <X size={14} />
          </button>
        </div>
      )}
    </div>
  )
}

function ProgressBar({ progress }: { progress: Progress }) {
  const pct = progress.total > 0 ? Math.min(100, (progress.done / progress.total) * 100) : null
  return (
    <div className="h-[3px] shrink-0 overflow-hidden bg-bar" role="progressbar" aria-label={progress.phase} aria-valuenow={pct ?? undefined}>
      <div className={cx('h-full bg-accent transition-[width] duration-200', pct === null && 'w-1/3 animate-pulse')} style={pct === null ? undefined : { width: `${pct}%` }} />
    </div>
  )
}

function Welcome({
  loading,
  running,
  folders,
  onRun,
  onWatch,
  onChoose,
}: {
  loading: boolean
  running: boolean
  folders: string[]
  onRun: () => void
  onWatch: (path: string) => void
  onChoose: () => void
}) {
  if (loading) return <div className="flex flex-1 items-center justify-center text-muted">Loading…</div>
  const primary = 'h-9 rounded-[3px] bg-accent px-4 font-semibold text-accent-ink hover:bg-accent-hover disabled:opacity-60'
  return (
    <div className="flex flex-1 flex-col items-center justify-center gap-4 bg-paper px-6 text-center">
      <Logo size={72} />
      <h1 className="font-display text-[22px] font-semibold tracking-[-0.015em]">Every project, guarded every night</h1>
      <p className="max-w-lg leading-relaxed text-muted">
        Mehen finds every npm, Cargo, NuGet and GitHub Actions project in the folders you choose, checks each package for newer versions and known
        vulnerabilities, and shows where your projects have drifted apart.
      </p>
      {folders.length > 0 ? (
        <button type="button" onClick={onRun} disabled={running} className={primary}>
          {running ? 'Checking…' : `Check ${folders.length === 1 ? folders[0] : `${folders.length} folders`}`}
        </button>
      ) : (
        <div className="flex items-center gap-2">
          <button type="button" onClick={() => onWatch(SUGGESTED_ROOT)} disabled={running} className={primary}>
            Watch <span className="font-mono">{SUGGESTED_ROOT}</span>
          </button>
          <button type="button" onClick={onChoose} disabled={running} className="h-9 rounded-[3px] border border-line-strong bg-surface px-4 text-ink hover:border-muted">
            Choose a folder…
          </button>
        </div>
      )}
    </div>
  )
}
