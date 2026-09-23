import { Database, EyeOff, FolderSearch, Layers, Package, RefreshCw, Search, ShieldAlert, TriangleAlert, X } from 'lucide-react'
import { useCallback, useEffect, useMemo, useState, type ReactNode } from 'react'
import * as api from './api'
import { Logo, cx } from './components/bits'
import { FoldersPanel } from './components/FoldersPanel'
import { PackagesView } from './components/PackagesView'
import { ProjectsView } from './components/ProjectsView'
import { VulnsView } from './components/VulnsView'
import {
  ECOSYSTEMS,
  ECOSYSTEM_LABEL,
  SEVERITY_RANK,
  groupPackages,
  isOutdated,
  normalizeSeverity,
  summarize,
  vulnUsages,
  type PackageGroup,
} from './derive'
import type { Dependency, Ecosystem, IgnoreKind, Inventory, Progress, Settings, StoreStats } from './types'

type Tab = 'packages' | 'projects' | 'vulns'
type Focus = 'all' | 'problems' | 'drift'

const SUGGESTED_ROOT = 'C:\\code'

export interface IgnoreRequest {
  kind: Extract<IgnoreKind, 'folder' | 'project'>
  value: string
  label: string
}

interface Notice {
  text: string
  undo?: () => void
}

const STATUS_WEIGHT: Record<string, number> = { major: 3, minor: 2, patch: 1 }

function attentionOrder(a: PackageGroup, b: PackageGroup) {
  return (
    b.vulnIds.length - a.vulnIds.length ||
    (STATUS_WEIGHT[b.worst] ?? 0) - (STATUS_WEIGHT[a.worst] ?? 0) ||
    b.versions.length - a.versions.length ||
    b.usages.length - a.usages.length ||
    a.name.localeCompare(b.name)
  )
}

function timeAgo(unixSeconds: number | null | undefined): string {
  if (!unixSeconds) return 'never'
  const s = Math.max(0, Math.round(Date.now() / 1000 - unixSeconds))
  if (s < 60) return 'just now'
  if (s < 3600) return `${Math.round(s / 60)} min ago`
  if (s < 86400) return `${Math.round(s / 3600)} h ago`
  return `${Math.round(s / 86400)} d ago`
}

export default function App() {
  const [settings, setSettings] = useState<Settings | null>(null)
  const [panelOpen, setPanelOpen] = useState(false)
  const [notice, setNotice] = useState<Notice | null>(null)
  const [inventory, setInventory] = useState<Inventory | null>(null)
  const [loading, setLoading] = useState(true)
  const [running, setRunning] = useState(false)
  const [progress, setProgress] = useState<Progress | null>(null)
  const [error, setError] = useState<string | null>(null)
  const [stats, setStats] = useState<StoreStats | null>(null)

  const [tab, setTab] = useState<Tab>('packages')
  const [focus, setFocus] = useState<Focus>('all')
  const [query, setQuery] = useState('')
  const [ecosystems, setEcosystems] = useState<Set<Ecosystem>>(() => new Set(ECOSYSTEMS))

  const refreshStats = useCallback(() => {
    api.storeStats().then(setStats).catch(() => {})
  }, [])

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
    refreshStats()
    return () => {
      cancelled = true
    }
  }, [refreshStats])

  useEffect(() => {
    if (!notice) return
    const t = window.setTimeout(() => setNotice(null), 8000)
    return () => window.clearTimeout(t)
  }, [notice])

  useEffect(() => {
    const unlisten = api.onProgress(setProgress)
    return () => {
      unlisten.then((fn) => fn())
    }
  }, [])

  const run = async (refresh: boolean) => {
    setRunning(true)
    setError(null)
    setProgress({ phase: 'Finding projects', done: 0, total: 0 })
    try {
      setInventory(await api.scanAndCheck(refresh))
      refreshStats()
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
      const rule = result.settings.rules.find((r) => r.kind === kind && r.value.toLowerCase() === value.toLowerCase())
      setNotice({
        text: `Ignoring ${label}. It will be skipped from now on.`,
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

  const roots = inventory?.roots ?? settings?.folders ?? []
  const folderLabel = !settings || settings.folders.length === 0 ? 'Choose folders' : settings.folders.length === 1 ? settings.folders[0] : `${settings.folders.length} folders`
  const ruleCount = settings?.rules.length ?? 0

  const allGroups = useMemo(() => (inventory ? groupPackages(inventory) : []), [inventory])
  const summary = useMemo(() => (inventory ? summarize(inventory, allGroups) : null), [inventory, allGroups])
  const usagesByVuln = useMemo(() => (inventory ? vulnUsages(inventory) : new Map()), [inventory])

  const q = query.trim().toLowerCase()

  const groups = useMemo(() => {
    return allGroups
      .filter((g) => ecosystems.has(g.ecosystem))
      .filter((g) => focus !== 'problems' || isOutdated(g.worst) || g.vulnIds.length > 0)
      .filter((g) => focus !== 'drift' || g.versions.length > 1)
      .filter((g) => !q || g.name.toLowerCase().includes(q) || g.usages.some((u) => u.project.name.toLowerCase().includes(q)))
      .sort(attentionOrder)
  }, [allGroups, ecosystems, focus, q])

  const depVisible = useCallback(
    (d: Dependency) =>
      ecosystems.has(d.ecosystem) && (focus !== 'problems' || isOutdated(d.status) || d.vulns.length > 0),
    [ecosystems, focus],
  )

  const projects = useMemo(() => {
    if (!inventory) return []
    return inventory.projects
      .filter((p) => ecosystems.has(p.ecosystem))
      .filter((p) => focus !== 'problems' || p.dependencies.some((d) => isOutdated(d.status) || d.vulns.length > 0))
      .filter((p) => !q || p.name.toLowerCase().includes(q) || p.dir.toLowerCase().includes(q) || p.dependencies.some((d) => d.name.toLowerCase().includes(q)))
  }, [inventory, ecosystems, focus, q])

  const vulns = useMemo(() => {
    if (!inventory) return []
    return inventory.vulnerabilities
      .filter((v) => (usagesByVuln.get(v.id) ?? []).some((u: { dep: Dependency }) => ecosystems.has(u.dep.ecosystem)))
      .filter((v) => !q || v.summary.toLowerCase().includes(q) || v.id.toLowerCase().includes(q) || (usagesByVuln.get(v.id) ?? []).some((u: { dep: Dependency }) => u.dep.name.toLowerCase().includes(q)))
      .sort((a, b) => (SEVERITY_RANK[normalizeSeverity(b.severity)] ?? 0) - (SEVERITY_RANK[normalizeSeverity(a.severity)] ?? 0))
  }, [inventory, usagesByVuln, ecosystems, q])

  const toggleEcosystem = (e: Ecosystem) =>
    setEcosystems((prev) => {
      const next = new Set(prev)
      if (next.has(e) && next.size > 1) next.delete(e)
      else next.add(e)
      return next
    })

  const cs = inventory?.checkStats
  const fromCache = cs ? cs.packagesCached + cs.osvCached + cs.advisoriesCached : 0
  const fetched = cs ? cs.packagesFetched + cs.osvFetched + cs.advisoriesFetched : 0

  return (
    <div className="flex h-full flex-col">
      <header className="flex items-center gap-3 border-b border-line bg-panel px-4 py-2.5">
        <div className="flex items-center gap-2 pr-2">
          <Logo size={24} />
          <span className="text-[15px] font-semibold tracking-tight">Mehen</span>
        </div>
        <button
          type="button"
          onClick={() => setPanelOpen(true)}
          disabled={running}
          className="flex min-w-0 max-w-[420px] items-center gap-2 rounded-lg border border-line-strong bg-raised px-2.5 py-1.5 text-left transition-colors hover:border-dim disabled:opacity-60"
          title="Choose folders to watch and what to ignore"
        >
          <FolderSearch size={15} className="shrink-0 text-gold" />
          <span className="truncate font-mono text-[12px]">{folderLabel}</span>
          {ruleCount > 0 && (
            <span className="inline-flex shrink-0 items-center gap-1 text-[11px] text-dim">
              <EyeOff size={12} />
              {ruleCount}
            </span>
          )}
        </button>
        <div className="flex-1" />
        {inventory && !running && (
          <div className="hidden items-center gap-1.5 text-[12px] text-dim lg:flex" title={cs ? `${fromCache} answers from the local database, ${fetched} fetched` : undefined}>
            <Database size={13} />
            Checked {timeAgo(inventory.checkedAt)}
            {cs && <span>· {fromCache > 0 ? `${Math.round((fromCache / Math.max(1, fromCache + fetched)) * 100)}% from cache` : 'all fresh'}</span>}
          </div>
        )}
        <button
          type="button"
          onClick={() => run(true)}
          disabled={running}
          className="rounded-lg border border-line-strong px-3 py-1.5 text-[12.5px] text-muted transition-colors hover:border-dim hover:text-ink disabled:opacity-50"
          title="Ignore cached answers and ask every registry again"
        >
          Refresh all
        </button>
        <button
          type="button"
          onClick={() => run(false)}
          disabled={running}
          className="inline-flex items-center gap-1.5 rounded-lg bg-gold px-3.5 py-1.5 text-[12.5px] font-semibold text-[#1d1506] transition-[filter] hover:brightness-110 disabled:opacity-60"
        >
          <RefreshCw size={14} className={cx(running && 'animate-spin')} />
          {running ? 'Checking…' : 'Check now'}
        </button>
      </header>

      {running && progress && <ProgressBar progress={progress} />}

      {error && (
        <div className="flex items-center gap-2 border-b border-[#5c2a1d] bg-carnelian-soft px-4 py-2 text-[12.5px] text-carnelian">
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
        <>
          {summary && (
            <section className="grid grid-cols-2 gap-3 border-b border-line px-4 py-3 md:grid-cols-5">
              <Tile label="Projects" value={summary.projects} icon={<Layers size={14} />} onClick={() => { setTab('projects'); setFocus('all') }} />
              <Tile label="Packages in use" value={summary.packages} icon={<Package size={14} />} onClick={() => { setTab('packages'); setFocus('all') }} />
              <Tile
                label="Out of date"
                value={summary.outdated}
                detail={`${summary.major} a major version behind`}
                tone="amber"
                onClick={() => { setTab('packages'); setFocus('problems') }}
              />
              <Tile
                label="Version drift"
                value={summary.drift}
                detail="packages on more than one version"
                tone="gold"
                onClick={() => { setTab('packages'); setFocus('drift') }}
              />
              <Tile
                label="Vulnerabilities"
                value={inventory.vulnerabilities.length}
                detail={[
                  summary.bySeverity.CRITICAL && `${summary.bySeverity.CRITICAL} critical`,
                  summary.bySeverity.HIGH && `${summary.bySeverity.HIGH} high`,
                ]
                  .filter(Boolean)
                  .join(' · ') || 'none critical or high'}
                tone="carnelian"
                icon={<ShieldAlert size={14} />}
                onClick={() => setTab('vulns')}
              />
            </section>
          )}

          <nav className="flex flex-wrap items-center gap-3 border-b border-line px-4 py-2">
            <div className="flex rounded-lg bg-panel p-0.5" role="tablist">
              <TabButton active={tab === 'packages'} onClick={() => setTab('packages')}>
                Packages
              </TabButton>
              <TabButton active={tab === 'projects'} onClick={() => setTab('projects')}>
                Projects
              </TabButton>
              <TabButton active={tab === 'vulns'} onClick={() => setTab('vulns')}>
                Vulnerabilities <span className="ml-1 text-carnelian">{inventory.vulnerabilities.length}</span>
              </TabButton>
            </div>

            <div className="relative">
              <Search size={14} className="pointer-events-none absolute left-2.5 top-1/2 -translate-y-1/2 text-dim" />
              <input
                value={query}
                onChange={(e) => setQuery(e.target.value)}
                placeholder="Search packages or projects"
                onKeyDown={(e) => e.key === 'Escape' && setQuery('')}
                className="h-8 w-64 rounded-lg border border-line bg-panel pl-8 pr-7 text-[12.5px] placeholder:text-dim focus:border-line-strong"
              />
              {query && (
                <button
                  type="button"
                  onClick={() => setQuery('')}
                  aria-label="Clear search"
                  className="absolute right-1.5 top-1/2 inline-flex size-5 -translate-y-1/2 items-center justify-center rounded text-dim hover:text-ink"
                >
                  <X size={13} />
                </button>
              )}
            </div>

            <div className="flex items-center gap-1">
              {ECOSYSTEMS.map((e) => (
                <button
                  key={e}
                  type="button"
                  onClick={() => toggleEcosystem(e)}
                  aria-pressed={ecosystems.has(e)}
                  className={cx(
                    'h-7 rounded-md border px-2 text-[12px] transition-colors',
                    ecosystems.has(e) ? 'border-line-strong bg-raised text-ink' : 'border-transparent text-dim hover:text-muted',
                  )}
                >
                  {ECOSYSTEM_LABEL[e]}
                </button>
              ))}
            </div>

            {tab !== 'vulns' && (
              <div className="flex items-center gap-1">
                {(['all', 'problems', 'drift'] as Focus[])
                  .filter((f) => tab === 'packages' || f !== 'drift')
                  .map((f) => (
                    <button
                      key={f}
                      type="button"
                      onClick={() => setFocus(f)}
                      aria-pressed={focus === f}
                      className={cx(
                        'h-7 rounded-md px-2 text-[12px] transition-colors',
                        focus === f ? 'bg-gold-soft text-gold' : 'text-dim hover:text-muted',
                      )}
                    >
                      {f === 'all' ? 'Everything' : f === 'problems' ? 'Needs attention' : 'Version drift'}
                    </button>
                  ))}
              </div>
            )}
          </nav>

          <main className="min-h-0 flex-1 overflow-auto">
            {tab === 'packages' && <PackagesView groups={groups} roots={roots} onIgnore={ignore} />}
            {tab === 'projects' && <ProjectsView projects={projects} roots={roots} depVisible={depVisible} onIgnore={ignore} />}
            {tab === 'vulns' && <VulnsView vulns={vulns} total={inventory.vulnerabilities.length} usages={usagesByVuln} roots={roots} />}
          </main>

          <footer className="flex items-center gap-4 border-t border-line bg-panel px-4 py-1.5 text-[11.5px] text-dim">
            <span>
              Found {inventory.projects.length} projects in {(inventory.scanMs / 1000).toFixed(1)}s
              {inventory.checkMs != null && `, checked in ${(inventory.checkMs / 1000).toFixed(1)}s`}
            </span>
            {cs && cs.throttled.length > 0 && <span className="text-amber">{cs.throttled.join(', ')} is rate limiting; some packages were not checked</span>}
            {ruleCount > 0 && (
              <button type="button" onClick={() => setPanelOpen(true)} className="inline-flex items-center gap-1 hover:text-muted">
                <EyeOff size={12} />
                {ruleCount} ignore rule{ruleCount === 1 ? '' : 's'}
                {inventory.ignored.length > 0 && ` · ${inventory.ignored.length} skipped`}
              </button>
            )}
            {inventory.warnings.length > 0 && (
              <span className="text-amber" title={inventory.warnings.join('\n')}>
                {inventory.warnings.length} file{inventory.warnings.length === 1 ? '' : 's'} could not be read
              </span>
            )}
            <div className="flex-1" />
            {stats && (
              <span title="Answers cached in the local database">
                Local cache: {stats.packages} packages · {stats.advisories} advisories
              </span>
            )}
          </footer>
        </>
      )}

      {panelOpen && settings && <FoldersPanel settings={settings} onSettings={setSettings} onClose={closePanel} />}

      {notice && (
        <div role="status" className="fixed bottom-10 left-1/2 z-30 flex -translate-x-1/2 items-center gap-3 rounded-xl border border-line-strong bg-raised px-4 py-2.5 text-[12.5px] shadow-2xl">
          <span>{notice.text}</span>
          {notice.undo && (
            <button type="button" onClick={notice.undo} className="font-medium text-gold hover:underline">
              Undo
            </button>
          )}
          <button type="button" onClick={() => setNotice(null)} aria-label="Dismiss" className="text-dim hover:text-ink">
            <X size={13} />
          </button>
        </div>
      )}
    </div>
  )
}

function ProgressBar({ progress }: { progress: Progress }) {
  const pct = progress.total > 0 ? Math.min(100, (progress.done / progress.total) * 100) : null
  return (
    <div className="border-b border-line bg-panel px-4 py-1.5">
      <div className="flex items-center justify-between text-[11.5px] text-muted">
        <span>{progress.phase}</span>
        {progress.total > 1 && (
          <span className="font-mono text-dim">
            {progress.done}/{progress.total}
          </span>
        )}
      </div>
      <div className="mt-1 h-1 overflow-hidden rounded-full bg-raised">
        <div
          className={cx('h-full rounded-full bg-gold transition-[width] duration-200', pct === null && 'w-1/3 animate-pulse')}
          style={pct === null ? undefined : { width: `${pct}%` }}
        />
      </div>
    </div>
  )
}

const TONE: Record<string, string> = {
  amber: 'text-amber',
  gold: 'text-gold',
  carnelian: 'text-carnelian',
}

function Tile({ label, value, detail, icon, tone, onClick }: { label: string; value: number; detail?: string; icon?: ReactNode; tone?: string; onClick: () => void }) {
  return (
    <button type="button" onClick={onClick} className="rounded-xl border border-line bg-panel px-4 py-3 text-left transition-colors hover:border-line-strong hover:bg-raised">
      <div className="flex items-center gap-1.5 text-[11.5px] text-muted">
        {icon}
        {label}
      </div>
      <div className={cx('mt-1 text-[24px] font-semibold leading-none tracking-tight', tone && value > 0 ? TONE[tone] : 'text-ink')}>{value.toLocaleString()}</div>
      {detail && <div className="mt-1.5 truncate text-[11.5px] text-dim">{detail}</div>}
    </button>
  )
}

function TabButton({ active, onClick, children }: { active: boolean; onClick: () => void; children: ReactNode }) {
  return (
    <button
      type="button"
      role="tab"
      aria-selected={active}
      onClick={onClick}
      className={cx('rounded-md px-3 py-1 text-[12.5px] transition-colors', active ? 'bg-raised text-ink shadow-sm' : 'text-dim hover:text-muted')}
    >
      {children}
    </button>
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
  if (loading) return <div className="flex flex-1 items-center justify-center text-dim">Loading…</div>
  const primary = 'rounded-lg bg-gold px-4 py-2 font-semibold text-[#1d1506] hover:brightness-110 disabled:opacity-60'
  return (
    <div className="flex flex-1 flex-col items-center justify-center gap-4 px-6 text-center">
      <Logo size={72} />
      <h1 className="text-[22px] font-semibold tracking-tight">Every project, guarded every night</h1>
      <p className="max-w-lg text-muted">
        Mehen finds every npm, Cargo, NuGet and GitHub Actions project in the folders you choose, checks each package for newer versions and
        known vulnerabilities, and shows where your projects have drifted apart.
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
          <button type="button" onClick={onChoose} disabled={running} className="rounded-lg border border-line-strong px-4 py-2 text-muted hover:text-ink">
            Choose a folder…
          </button>
        </div>
      )}
    </div>
  )
}
