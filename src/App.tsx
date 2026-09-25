import { Moon, Search, Settings2, Sun, TriangleAlert, Undo2, X } from 'lucide-react'
import { useCallback, useEffect, useMemo, useRef, useState } from 'react'
import * as api from './api'
import { AdvisoryDialog } from './components/AdvisoryDialog'
import { AppUpdateButton, AppUpdateDialog, useAppUpdate } from './components/AppUpdate'
import { Logo, cx } from './components/bits'
import { ProjectRecord, Tray, type TrayGroup } from './components/Inspector'
import { HeldBackDialog } from './components/HeldBack'
import { ManageProjects } from './components/ManageProjects'
import { Queue, type QueueFilters, type ScopedRow } from './components/Queue'
import { Rail, ScanStatus } from './components/Rail'
import { SettingsDialog, type SettingsTab } from './components/SettingsDialog'
import { AddFolderDialog, ExcludeDialog } from './components/SmallDialogs'
import { UpdateFlow, type UpdateTarget } from './components/UpdateFlow'
import { RISK_ORDER, distinctVersions, folderName, heldBack, isWithin, projectTypes, queueRows, repoKey, repos as groupRepos, samePath, type QueueRow, type QueueUsage, type Repo } from './derive'
import { usePrefs } from './prefs'
import type { Change, Hold, IgnoreKind, IgnoreRule, Inventory, Progress, Settings, VersionPolicies } from './types'

const SUGGESTED_ROOT = 'C:\\code'

interface Notice {
  text: string
  undo?: () => void
}

/** Where a dialog goes back to when a nested one closes. */
type Back = 'manage' | 'settings' | null

type Dialog =
  | { kind: 'settings'; tab?: SettingsTab; repo?: Repo }
  | { kind: 'manage' }
  | { kind: 'add-folder'; back: Back }
  | { kind: 'exclude'; repo: Repo; back: Back }
  | { kind: 'update'; start: 'confirm' | 'preview'; targets: UpdateTarget[] }
  | { kind: 'advisory'; row: QueueRow }
  | { kind: 'held-back'; packageKey: string | null }
  | { kind: 'app-update' }

const plural = (n: number, word: string) => `${n} ${word}${n === 1 ? '' : 's'}`

export default function App() {
  const { prefs, update: setPrefs, theme } = usePrefs()
  const [settings, setSettings] = useState<Settings | null>(null)
  const appUpdate = useAppUpdate(settings?.appUpdateCheck ?? false)
  const [dialog, setDialog] = useState<Dialog | null>(null)
  const [notice, setNotice] = useState<Notice | null>(null)
  const [inventory, setInventory] = useState<Inventory | null>(null)
  const [loading, setLoading] = useState(true)
  const [running, setRunning] = useState(false)
  const [progress, setProgress] = useState<Progress | null>(null)
  const [error, setError] = useState<string | null>(null)

  const [repoScope, setRepoScope] = useState<string | null>(null)
  const [query, setQuery] = useState('')
  const [filters, setFilters] = useState<QueueFilters>(() => ({ types: new Set(), ecosystems: new Set(), risk: 'any', riskFirst: prefs.riskFirst }))
  const [policies, setPolicies] = useState<VersionPolicies>({})
  const [holds, setHolds] = useState<Hold[]>([])
  const [selected, setSelected] = useState<Set<string>>(new Set())
  /** Selected usages moving somewhere other than their usual target, like a smaller security fix. */
  const [chosen, setChosen] = useState<Map<string, string>>(new Map())
  const [icons, setIcons] = useState<Record<string, string>>({})
  const searchRef = useRef<HTMLInputElement>(null)
  const scannedOnOpen = useRef(false)
  /** Repositories whose holds changed during an update, checked again once it closes. */
  const keptDuringUpdate = useRef<Set<string>>(new Set())

  const run = useCallback(async (refresh: boolean, only: string[] | null = null) => {
    setRunning(true)
    setError(null)
    setProgress({ phase: only ? `Checking ${only.length === 1 ? folderName(only[0]) : `${only.length} projects`}` : 'Finding projects', done: 0, total: 0 })
    try {
      setInventory(await api.scanAndCheck(refresh, only))
    } catch (e) {
      setError(String(e))
    } finally {
      setRunning(false)
      setProgress(null)
    }
  }, [])

  useEffect(() => {
    let cancelled = false
    Promise.all([api.settings(), api.lastInventory(), api.versionPolicy(), api.holds()])
      .then(([s, inv, p, h]) => {
        if (cancelled) return
        setSettings(s)
        setInventory(inv)
        setPolicies(p)
        setHolds(h)
        if (prefs.scanOnOpen && s.folders.length && !scannedOnOpen.current) {
          scannedOnOpen.current = true
          run(false)
        }
      })
      .catch((e) => !cancelled && setError(String(e)))
      .finally(() => !cancelled && setLoading(false))
    return () => {
      cancelled = true
    }
    // Loads once; the scan-on-open choice is read at start only.
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
    const unlisten = api.onProgress((p) => running && setProgress(p))
    return () => {
      unlisten.then((fn) => fn())
    }
  }, [running])

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

  const watchFolder = async (path: string) => {
    try {
      setSettings(await api.addFolder(path))
      setNotice({ text: `Added ${path}. Checking it now.` })
      await run(false, [path])
    } catch (e) {
      setError(String(e))
    }
  }

  const removeFolder = async (folder: string) => {
    try {
      setSettings(await api.removeFolder(folder))
      setInventory((inv) => inv && { ...inv, projects: inv.projects.filter((p) => !isWithin(p.dir, folder)) })
      const count = inventory?.projects.filter((p) => isWithin(p.dir, folder)).length ?? 0
      setNotice({
        text: `Removed ${folder}.${count ? ` Its projects will no longer be checked.` : ''}`,
        undo: async () => {
          setNotice(null)
          setSettings(await api.addFolder(folder))
          await run(false, [folder])
        },
      })
    } catch (e) {
      setError(String(e))
    }
  }

  const exclude = async (kind: IgnoreKind, value: string, label: string) => {
    try {
      const result = await api.addIgnore(kind, value)
      setSettings(result.settings)
      if (result.inventory) setInventory(result.inventory)
      const rule = result.settings.rules.find((r) => r.kind === kind && (kind === 'pattern' ? r.value.toLowerCase() === value.toLowerCase() : samePath(r.value, value)))
      setNotice({
        text: `${label} excluded. Mehen will skip it from now on.`,
        undo: rule
          ? async () => {
              setNotice(null)
              setSettings(await api.removeIgnore(rule.id))
              await run(false, kind === 'pattern' ? null : [kind === 'project' ? value.replace(/[\\/][^\\/]*$/, '') : value])
            }
          : undefined,
      })
    } catch (e) {
      setError(String(e))
    }
  }

  const excludeMany = async (keys: string[]) => {
    try {
      let result: Awaited<ReturnType<typeof api.addIgnore>> | null = null
      for (const key of keys) result = await api.addIgnore('folder', key)
      if (!result) return
      setSettings(result.settings)
      if (result.inventory) setInventory(result.inventory)
      const rules = result.settings.rules.filter((r) => r.kind === 'folder' && keys.some((k) => samePath(k, r.value)))
      setNotice({
        text: `${plural(keys.length, 'project')} excluded.`,
        undo: async () => {
          setNotice(null)
          await include(rules, keys)
        },
      })
    } catch (e) {
      setError(String(e))
    }
  }

  const include = async (rules: IgnoreRule[], keys: string[]) => {
    try {
      let latest: Settings | null = null
      for (const rule of rules) latest = await api.removeIgnore(rule.id)
      if (latest) setSettings(latest)
      await run(false, keys)
    } catch (e) {
      setError(String(e))
    }
  }

  const rows = useMemo(() => (inventory ? queueRows(inventory, policies) : []), [inventory, policies])
  const repoList = useMemo(() => (inventory ? groupRepos(inventory, rows) : []), [inventory, rows])
  const repoByKey = useMemo(() => new Map(repoList.map((r) => [r.key.toLowerCase(), r])), [repoList])
  const repo = repoScope ? (repoByKey.get(repoScope.toLowerCase()) ?? null) : null
  const present = useMemo(
    () => ({ types: projectTypes(inventory?.projects ?? []), ecosystems: [...new Set((inventory?.projects ?? []).map((p) => p.ecosystem))] }),
    [inventory],
  )
  const nameOf = useCallback((key: string) => repoByKey.get(key.toLowerCase())?.name ?? folderName(key), [repoByKey])

  // Logos: remembered ones first (instant), then a one-time search of new folders.
  const repoKeys = useMemo(() => repoList.map((r) => r.key).join('|'), [repoList])
  useEffect(() => {
    if (!repoKeys) return
    const keys = repoKeys.split('|')
    let cancelled = false
    const merge = (found: { repo: string; dataUrl: string }[]) =>
      !cancelled && found.length && setIcons((prev) => ({ ...prev, ...Object.fromEntries(found.map((f) => [f.repo.toLowerCase(), f.dataUrl])) }))
    api
      .repoIcons(keys, false)
      .then((found) => {
        merge(found)
        const known = new Set(found.map((f) => f.repo.toLowerCase()))
        return api.repoIcons(
          keys.filter((k) => !known.has(k.toLowerCase())),
          true,
        )
      })
      .then(merge)
      .catch(() => {})
    return () => {
      cancelled = true
    }
  }, [repoKeys])

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
  const typesByRepo = useMemo(() => new Map(repoList.map((r) => [r.key.toLowerCase(), new Set(projectTypes(r.projects))])), [repoList])

  /** Usages the scope, project-type and dependency-type filters in `f` allow. */
  const usagesIn = useCallback(
    (row: QueueRow, f: QueueFilters) =>
      row.usages.filter((u) => {
        if (repo && !samePath(repoKey(u.project), repo.key)) return false
        if (!repo && f.types.size && ![...f.types].some((t) => typesByRepo.get(repoKey(u.project).toLowerCase())?.has(t))) return false
        return !f.ecosystems.size || f.ecosystems.has(row.ecosystem)
      }),
    [repo, typesByRepo],
  )
  const inScope = useCallback((row: QueueRow) => usagesIn(row, filters), [usagesIn, filters])

  const q = query.trim().toLowerCase()
  /** The rows `f` leaves, together with the scope and the search: the list, and each filter option's count. */
  const rowsFor = useCallback(
    (f: QueueFilters): ScopedRow[] =>
      rows
        .filter((row) => f.risk === 'any' || (f.risk === 'security' ? row.risk === 'security' : row.risk === 'security' || row.risk === 'major'))
        .map((row) => {
          const usages = usagesIn(row, f)
          return { row, usages: !q || row.name.toLowerCase().includes(q) ? usages : usages.filter((u) => u.project.name.toLowerCase().includes(q) || u.project.dir.toLowerCase().includes(q)) }
        })
        .filter(({ usages }) => usages.length > 0),
    [rows, usagesIn, q],
  )
  const countFor = useCallback((f: QueueFilters) => rowsFor(f).length, [rowsFor])

  const visible: ScopedRow[] = useMemo(() => {
    const byName = (a: ScopedRow, b: ScopedRow) => a.row.name.localeCompare(b.row.name, undefined, { sensitivity: 'base' })
    return rowsFor(filters).sort((a, b) => (filters.riskFirst ? RISK_ORDER.indexOf(a.row.risk) - RISK_ORDER.indexOf(b.row.risk) : 0) || byName(a, b))
  }, [rowsFor, filters])

  const security = useMemo(() => {
    const usages = rows.filter((r) => r.vulnIds.length > 0).flatMap((r) => inScope(r).filter((u) => u.dep.vulns.length > 0))
    if (!usages.length) return null
    const packages = new Set(usages.map((u) => `${u.dep.ecosystem}:${u.dep.name}`)).size
    const projects = new Set(usages.map((u) => repoKey(u.project).toLowerCase())).size
    return { packages, projects, usages, allSelected: usages.every((u) => selected.has(u.key)) }
  }, [rows, inScope, selected])

  const held = useMemo(() => (inventory ? heldBack(inventory.projects.filter((p) => !repo || samePath(repoKey(p), repo.key))) : []), [inventory, repo])

  /** Re-checks only where a hold on `scope` changes anything. */
  const recheck = (scope: string) => run(false, scope === '*' ? null : [scope])

  const keep = async (row: QueueRow, scope: string, line: string) => {
    try {
      const before = holds.find((h) => h.ecosystem === row.ecosystem && h.name === row.name && samePath(h.scope, scope))
      const next = await api.setHold(row.ecosystem, row.name, scope, line)
      setHolds(next)
      const added = next.find((h) => h.ecosystem === row.ecosystem && h.name === row.name && samePath(h.scope, scope))
      setNotice({
        text: `Keeping ${row.name} on ${line}.x ${scope === '*' ? 'everywhere' : `in ${nameOf(scope)}`}.`,
        undo: added
          ? async () => {
              setNotice(null)
              setHolds(before ? await api.setHold(before.ecosystem, before.name, before.scope, before.line) : await api.removeHold(added.id))
              await recheck(scope)
            }
          : undefined,
      })
      await recheck(scope)
    } catch (e) {
      setError(String(e))
    }
  }

  const release = async (hold: Hold) => {
    try {
      setHolds(await api.removeHold(hold.id))
      setNotice({
        text: `${hold.name} can move past ${hold.line}.x again.`,
        undo: async () => {
          setNotice(null)
          setHolds(await api.setHold(hold.ecosystem, hold.name, hold.scope, hold.line))
          await recheck(hold.scope)
        },
      })
      await recheck(hold.scope)
    } catch (e) {
      setError(String(e))
    }
  }

  const addAll = (usages: QueueUsage[], message: (n: number) => string) => {
    const added = usages.map((u) => u.key).filter((k) => !selected.has(k))
    if (!added.length) return
    setSelected((prev) => new Set([...prev, ...added]))
    setChosen((prev) => (added.some((k) => prev.has(k)) ? new Map([...prev].filter(([k]) => !added.includes(k))) : prev))
    const packages = new Set(usages.filter((u) => added.includes(u.key)).map((u) => `${u.dep.ecosystem}:${u.dep.name}`)).size
    setNotice({
      text: message(packages),
      undo: () => {
        setNotice(null)
        setSelected((prev) => new Set([...prev].filter((k) => !added.includes(k))))
      },
    })
  }

  /** Selects a vulnerable package's usages, moving each to the newest version or to its smallest fix. */
  const selectFix = (row: QueueRow, pick: 'newest' | 'fix') => {
    const usages = row.usages.filter((u) => u.dep.vulns.length > 0)
    const [before, beforeChosen] = [selected, chosen]
    setSelected((prev) => new Set([...prev, ...usages.map((u) => u.key)]))
    setChosen((prev) => {
      const next = new Map(prev)
      for (const u of usages) {
        if (pick === 'fix' && u.dep.fixTarget && u.dep.fixTarget !== u.target) next.set(u.key, u.dep.fixTarget)
        else next.delete(u.key)
      }
      return next
    })
    const to = distinctVersions(usages.map((u) => (pick === 'fix' && u.dep.fixTarget) || u.target))
    setNotice({
      text: `${row.name} ${to.join(', ')} selected.`,
      undo: () => (setNotice(null), setSelected(before), setChosen(beforeChosen)),
    })
  }

  const toggleUsages = (usages: QueueUsage[]) => {
    setChosen((prev) => {
      if (!usages.some((u) => prev.has(u.key))) return prev
      const next = new Map(prev)
      for (const u of usages) next.delete(u.key)
      return next
    })
    setSelected((prev) => {
      const next = new Set(prev)
      const all = usages.every((u) => next.has(u.key))
      for (const u of usages) {
        if (all) next.delete(u.key)
        else next.add(u.key)
      }
      return next
    })
  }

  const trayGroups: TrayGroup[] = useMemo(
    () =>
      rows
        .map((row) => ({ key: row.key, name: row.name, usages: row.usages.filter((u) => selected.has(u.key)).map((u) => ({ ...u, target: chosen.get(u.key) ?? u.target })) }))
        .filter((g) => g.usages.length > 0),
    [rows, selected, chosen],
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

  const startUpdate = (start: 'confirm' | 'preview') => {
    const byProject = new Map<string, UpdateTarget>()
    for (const u of trayGroups.flatMap((g) => g.usages)) {
      const entry = byProject.get(u.project.id) ?? { project: u.project, changes: [] as Change[] }
      if (!entry.changes.some((c) => c.name === u.dep.name && c.from === u.dep.requested)) entry.changes.push({ name: u.dep.name, from: u.dep.requested, to: u.target })
      byProject.set(u.project.id, entry)
    }
    setDialog({ kind: 'update', start, targets: [...byProject.values()] })
  }

  const openBack = (back: Back) => setDialog(back === 'manage' ? { kind: 'manage' } : back === 'settings' ? { kind: 'settings', tab: 'scanning' } : null)

  const excludedCount = settings?.rules.length ?? 0
  const repoCount = repoList.length
  const folderCount = settings?.folders.length ?? 0
  const warnings = inventory ? [...(inventory.checkStats?.throttled.map((t) => `${t} is rate limiting; some packages were not checked`) ?? []), ...inventory.warnings] : []

  return (
    <div className="flex h-full flex-col">
      <header className="on-rail flex h-[52px] shrink-0 items-center gap-5 border-b border-bar-line bg-bar pr-3.5 pl-4 text-rail-ink">
        <div className="flex w-[222px] items-center gap-2.5 max-[1100px]:w-[182px]">
          <Logo size={28} />
          <strong className="font-wordmark text-[17px] font-semibold tracking-[-0.025em]">Mehen</strong>
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
        <div className="flex gap-1.5">
          <AppUpdateButton update={appUpdate} onOpen={() => setDialog({ kind: 'app-update' })} />
          <button
            type="button"
            onClick={() => setPrefs({ theme: theme === 'dark' ? 'light' : 'dark' })}
            aria-label={`${theme === 'dark' ? 'Dark' : 'Light'} theme. Switch to ${theme === 'dark' ? 'light' : 'dark'}`}
            className="inline-flex h-[34px] items-center gap-2 rounded-[3px] border border-rail-border px-2.5 text-[12px] text-rail-ink hover:border-rail-border-strong hover:bg-rail-field-hover"
          >
            {theme === 'dark' ? <Moon size={15} /> : <Sun size={15} />}
            {theme === 'dark' ? 'Dark' : 'Light'}
          </button>
          <button
            type="button"
            onClick={() => setDialog({ kind: 'settings' })}
            disabled={!settings}
            aria-label="Settings"
            title="Settings"
            className="grid size-[34px] place-items-center rounded-[3px] border border-rail-border text-rail-ink hover:border-rail-border-strong hover:bg-rail-field-hover disabled:opacity-50"
          >
            <Settings2 size={16} />
          </button>
        </div>
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
          onChoose={() => setDialog({ kind: 'add-folder', back: null })}
        />
      ) : (
        <div className="grid min-h-0 flex-1 grid-cols-[256px_minmax(0,1fr)_316px] max-[1279px]:grid-cols-[232px_minmax(0,1fr)_284px] max-[1100px]:grid-cols-[216px_minmax(0,1fr)_264px]">
          <Rail
            repos={repoList}
            roots={inventory.roots}
            icons={icons}
            selected={repo?.key ?? null}
            query={query}
            sort={prefs.railSort}
            onSort={(railSort) => setPrefs({ railSort })}
            excludedCount={excludedCount}
            onSelect={setRepoScope}
            onManage={() => setDialog({ kind: 'manage' })}
            onAddFolder={() => setDialog({ kind: 'add-folder', back: null })}
            onExclusions={() => setDialog({ kind: 'settings', tab: 'scanning' })}
            onRefreshAll={() => run(true)}
            onReveal={(path) => api.reveal(path)}
            onOpenInEditor={(path) => api.openInEditor(path)}
            onExclude={(r) => setDialog({ kind: 'exclude', repo: r, back: null })}
            onProjectSettings={(r) => setDialog({ kind: 'settings', repo: r })}
            onScanRepo={(r) => run(false, [r.key])}
            footer={
              <ScanStatus
                running={running}
                phase={progress?.phase ?? null}
                checkedAt={inventory.checkedAt}
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
            onFilters={(f) => {
              setFilters(f)
              if (f.riskFirst !== prefs.riskFirst) setPrefs({ riskFirst: f.riskFirst })
            }}
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
            onAdvisory={(row) => setDialog({ kind: 'advisory', row })}
            searching={!!q}
            onClearSearch={() => setQuery('')}
            holds={holds}
            heldBack={held.length}
            nameOf={nameOf}
            onKeep={keep}
            onRelease={release}
            onWhy={(packageKey) => setDialog({ kind: 'held-back', packageKey })}
            present={present}
            countFor={countFor}
          />
          <aside className="flex min-h-0 flex-col overflow-y-auto border-l border-line bg-paper-2" aria-label="Project and selected updates">
            {repo && (
              <ProjectRecord repo={repo} icon={icons[repo.key.toLowerCase()]} onReveal={() => api.reveal(repo.key)} onOpenInEditor={() => api.openInEditor(repo.key)} onSettings={() => setDialog({ kind: 'settings', repo })} />
            )}
            <Tray
              groups={trayGroups}
              repo={repo}
              onRemove={removeGroup}
              onClear={clearSelection}
              onUpdate={() => startUpdate('confirm')}
              onPreview={() => startUpdate('preview')}
              checks={prefs.checks}
              commit={prefs.commit}
              onOptions={setPrefs}
            />
          </aside>
        </div>
      )}

      {dialog?.kind === 'app-update' && <AppUpdateDialog update={appUpdate} onClose={() => setDialog(null)} />}

      {dialog?.kind === 'settings' && settings && (
        <SettingsDialog
          appUpdate={appUpdate}
          onAppUpdate={() => setDialog({ kind: 'app-update' })}
          tab={dialog.tab}
          repo={dialog.repo}
          settings={settings}
          prefs={prefs}
          inventory={inventory}
          policies={policies}
          onPolicies={setPolicies}
          holds={holds}
          nameOf={nameOf}
          onRelease={release}
          onPrefs={(patch) => {
            setPrefs(patch)
            if (patch.riskFirst !== undefined) setFilters((f) => ({ ...f, riskFirst: patch.riskFirst! }))
          }}
          onSettings={setSettings}
          onInventory={setInventory}
          onAddFolder={() => setDialog({ kind: 'add-folder', back: 'settings' })}
          onScanFolder={(folder) => run(false, [folder])}
          onRemoveFolder={removeFolder}
          onClose={(changed) => {
            setDialog(null)
            if (changed) run(false)
          }}
        />
      )}

      {dialog?.kind === 'manage' && settings && (
        <ManageProjects
          settings={settings}
          repos={repoList}
          icons={icons}
          onOpen={(key) => {
            setRepoScope(key)
            setDialog(null)
          }}
          onScan={(paths) => run(false, paths)}
          onAddFolder={() => setDialog({ kind: 'add-folder', back: 'manage' })}
          onRemoveFolder={removeFolder}
          onProjectSettings={(r) => setDialog({ kind: 'settings', repo: r })}
          onExclude={(r) => setDialog({ kind: 'exclude', repo: r, back: 'manage' })}
          onExcludeMany={excludeMany}
          onInclude={include}
          onReveal={(path) => api.reveal(path)}
          onOpenInEditor={(path) => api.openInEditor(path)}
          onClose={() => setDialog(null)}
        />
      )}

      {dialog?.kind === 'add-folder' && (
        <AddFolderDialog
          onAdd={(path) => {
            openBack(dialog.back)
            watchFolder(path)
          }}
          onClose={() => openBack(dialog.back)}
        />
      )}

      {dialog?.kind === 'exclude' && inventory && (
        <ExcludeDialog
          repo={dialog.repo}
          roots={inventory.roots}
          onExclude={(kind, value, label) => {
            openBack(dialog.back)
            exclude(kind, value, label)
          }}
          onClose={() => openBack(dialog.back)}
        />
      )}

      {dialog?.kind === 'advisory' && inventory && (
        <AdvisoryDialog
          row={dialog.row}
          inventory={inventory}
          chosenTarget={(u) => (selected.has(u.key) ? (chosen.get(u.key) ?? u.target) : null)}
          onSelect={(pick) => {
            selectFix(dialog.row, pick)
            setDialog(null)
          }}
          onClose={() => setDialog(null)}
        />
      )}

      {dialog?.kind === 'held-back' && (
        <HeldBackDialog
          groups={dialog.packageKey ? held.filter((g) => g.key === dialog.packageKey) : held}
          holds={holds}
          icons={icons}
          nameOf={nameOf}
          scopeLabel={repo ? repo.name : 'your projects'}
          onRelease={release}
          onClose={() => setDialog(null)}
        />
      )}

      {dialog?.kind === 'update' && inventory && (
        <UpdateFlow
          targets={dialog.targets}
          start={dialog.start}
          roots={inventory.roots}
          checks={prefs.checks}
          commit={prefs.commit}
          stopOnFailure={prefs.stopOnFailure}
          onOptions={setPrefs}
          nameOf={nameOf}
          icons={icons}
          onKeep={async (k, scope) => {
            setHolds(await api.setHold(k.ecosystem, k.name, scope, k.line))
            keptDuringUpdate.current.add(scope)
          }}
          onClose={(refreshed) => {
            setDialog(null)
            if (refreshed) setInventory(refreshed)
            const scopes = [...keptDuringUpdate.current]
            keptDuringUpdate.current.clear()
            if (scopes.length) run(false, scopes)
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
      <h1 className="font-display text-[22px] font-semibold tracking-[-0.015em]">Every project, checked in one place</h1>
      <p className="max-w-lg leading-relaxed text-muted">
        Mehen finds every npm, Cargo, NuGet, Go, Python, Dart and Flutter, PHP, Ruby, and GitHub Actions project in the folders you choose, checks each package for newer versions and known
        vulnerabilities, and shows where your projects have drifted apart.
      </p>
      {folders.length > 0 ? (
        <button type="button" onClick={onRun} disabled={running} className={primary}>
          {running ? 'Checking…' : `Check ${folders.length === 1 ? folders[0] : `${folders.length} folders`}`}
        </button>
      ) : (
        <div className="flex items-center gap-2">
          <button type="button" onClick={() => onWatch(SUGGESTED_ROOT)} disabled={running} className={primary}>
            Check <span className="font-mono">{SUGGESTED_ROOT}</span>
          </button>
          <button type="button" onClick={onChoose} disabled={running} className="h-9 rounded-[3px] border border-line-strong bg-surface px-4 text-ink hover:border-muted">
            Choose a folder…
          </button>
        </div>
      )}
    </div>
  )
}
