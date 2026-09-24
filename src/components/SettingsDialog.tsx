import { Asterisk, FileX, FolderMinus, FolderPlus, Pin, Plus, RefreshCw, Undo2, X } from 'lucide-react'
import { useEffect, useMemo, useRef, useState, type KeyboardEvent } from 'react'
import * as api from '../api'
import { ECOSYSTEM_LABEL, POLICY_LABEL, isWithin, relativePath, samePath, type Repo } from '../derive'
import type { Prefs } from '../prefs'
import type { CheckCommands, CheckConfig, DiscoveredProject, Ecosystem, Hold, IgnoreKind, IgnoreRule, Inventory, Settings, VersionPolicies, VersionPolicy } from '../types'
import { cx } from './bits'
import { Button, Dialog } from './Dialog'
import { SectionTitle, Select, SettingRow, Switch, TextInput } from './controls'

export type SettingsTab = 'general' | 'scanning' | 'updates' | 'security'

const TABS: Record<SettingsTab, [string, string]> = {
  general: ['General', 'How Mehen looks, starts, and checks in the background.'],
  scanning: ['Scanning', 'The folders Mehen watches and what it skips.'],
  updates: ['Updates and checks', 'How far updates go, how they run, and what proves they worked.'],
  security: ['Security', 'Vulnerability alerts and where advisories come from.'],
}

const SCHEDULES = [
  { hours: 0, label: 'Off' },
  { hours: 6, label: 'Every 6 hours' },
  { hours: 12, label: 'Every 12 hours' },
  { hours: 24, label: 'Once a day' },
]

/** Folder names worth offering as patterns when they appear in the scan. */
const SUGGESTED_PATTERNS = ['fixtures', 'examples', 'samples', 'demo', '_spikes', 'temp', 'tmp', 'archive', 'playground', 'test-data']

const KIND_LABEL: Record<IgnoreKind, string> = { folder: 'Folder and everything in it', project: 'One manifest', pattern: 'Every folder with this name' }
const KIND_ICON: Record<IgnoreKind, typeof FolderMinus> = { folder: FolderMinus, project: FileX, pattern: Asterisk }

/** What Mehen runs as checks when nobody has changed them. */
export const DEFAULT_CHECKS: Partial<Record<Ecosystem, string>> = {
  npm: 'npm run build and npm run test, when the project has them',
  cargo: 'cargo check and cargo test',
  nuget: 'dotnet build, then dotnet test on the solution',
  go: 'go build ./... and go test ./...',
  pypi: 'pytest through uv, Poetry, PDM or Pipenv, when the project has tests',
  pub: 'flutter (or dart) analyze, then test when the project has a test folder',
  packagist: 'composer test, or phpunit when the project has a phpunit.xml',
  rubygems: 'bundle exec rspec, or rake test, when the project has specs or tests',
}

export const ecosystemScope = (e: Ecosystem) => `ecosystem:${e}`

export function findCommands(all: CheckCommands, scope: string): CheckConfig | null {
  const key = Object.keys(all).find((k) => k.toLowerCase() === scope.toLowerCase())
  return key ? all[key] : null
}

function CommandEditor({ scope, initial, onSaved, placeholder }: { scope: string; initial: CheckConfig; onSaved: (all: CheckCommands) => void; placeholder: string }) {
  const [draft, setDraft] = useState<string[]>(initial.commands.length ? initial.commands : [''])
  const [cwd, setCwd] = useState(initial.cwd ?? '')
  const [saving, setSaving] = useState(false)
  const inputs = useRef<(HTMLInputElement | null)[]>([])
  const dirty = JSON.stringify(draft.map((c) => c.trim()).filter(Boolean)) !== JSON.stringify(initial.commands) || cwd.trim() !== (initial.cwd ?? '')

  const save = async () => {
    setSaving(true)
    try {
      onSaved(await api.setCheckCommands(scope, draft, cwd))
    } finally {
      setSaving(false)
    }
  }

  return (
    <div className="grid gap-1.5 py-2">
      {draft.map((command, i) => (
        <div key={i} className="flex gap-1.5">
          <TextInput
            ref={(el) => {
              inputs.current[i] = el
            }}
            mono
            value={command}
            placeholder={placeholder}
            aria-label={`Check ${i + 1}`}
            onChange={(e) => setDraft(draft.map((c, j) => (j === i ? e.target.value : c)))}
            onKeyDown={(e) => {
              if (e.key === 'Enter' && dirty) save()
            }}
            className="flex-1"
          />
          <button
            type="button"
            onClick={() => setDraft(draft.length > 1 ? draft.filter((_, j) => j !== i) : [''])}
            aria-label={`Remove check ${i + 1}`}
            className="grid size-[34px] place-items-center rounded-[3px] text-muted hover:bg-sunken hover:text-ink"
          >
            <X size={15} />
          </button>
        </div>
      ))}
      <div className="flex items-center gap-2 pt-1">
        <Button
          variant="ghost"
          onClick={() => {
            setDraft([...draft, ''])
            requestAnimationFrame(() => inputs.current[draft.length]?.focus())
          }}
        >
          <Plus size={15} />
          Add a check
        </Button>
        <span className="flex-1 text-[12px] text-muted">Separate arguments with spaces.</span>
      </div>
      <SettingRow title="Working folder" help="Relative to the project, like src/web. Leave empty to run from the project folder.">
        <TextInput mono value={cwd} onChange={(e) => setCwd(e.target.value)} placeholder="." aria-label="Working folder" className="w-[220px]" />
      </SettingRow>
      <div className="flex justify-end pt-2">
        <Button variant="primary" onClick={save} disabled={!dirty || saving}>
          Save checks
        </Button>
      </div>
    </div>
  )
}

export function SettingsDialog({
  tab: initialTab = 'general',
  repo,
  settings,
  prefs,
  inventory,
  policies,
  onPolicies,
  holds,
  nameOf,
  onRelease,
  onPrefs,
  onSettings,
  onInventory,
  onAddFolder,
  onScanFolder,
  onRemoveFolder,
  onClose,
}: {
  tab?: SettingsTab
  /** Settings for one project: only its checks. */
  repo?: Repo
  settings: Settings
  prefs: Prefs
  inventory: Inventory | null
  policies: VersionPolicies
  onPolicies: (p: VersionPolicies) => void
  holds: Hold[]
  nameOf: (folder: string) => string
  onRelease: (hold: Hold) => void
  onPrefs: (patch: Partial<Prefs>) => void
  onSettings: (s: Settings) => void
  onInventory: (inv: Inventory) => void
  onAddFolder: () => void
  onScanFolder: (folder: string) => Promise<void>
  onRemoveFolder: (folder: string) => void
  /** `rulesChanged` asks for a new check so newly included projects appear. */
  onClose: (rulesChanged: boolean) => void
}) {
  const tabs: SettingsTab[] = repo ? ['updates'] : ['general', 'scanning', 'updates', 'security']
  const [tab, setTab] = useState<SettingsTab>(repo ? 'updates' : initialTab)
  const [commands, setCommands] = useState<CheckCommands | null>(null)
  const [discovered, setDiscovered] = useState<DiscoveredProject[] | null>(null)
  const [pattern, setPattern] = useState('')
  const [scanning, setScanning] = useState<string | null>(null)
  const [error, setError] = useState<string | null>(null)
  const rulesChanged = useRef(false)
  const tabRefs = useRef<Partial<Record<SettingsTab, HTMLButtonElement | null>>>({})

  useEffect(() => {
    api.checkCommands().then(setCommands).catch((e) => setError(String(e)))
  }, [])

  useEffect(() => {
    if (tab === 'scanning' && !discovered) api.discover().then(setDiscovered).catch((e) => setError(String(e)))
  }, [tab, discovered])

  const close = () => onClose(rulesChanged.current)

  const rediscover = () => api.discover().then(setDiscovered).catch(() => {})

  const addPattern = async (value: string) => {
    const v = value.trim()
    if (!v) return
    setPattern('')
    try {
      const result = await api.addIgnore('pattern', v)
      onSettings(result.settings)
      if (result.inventory) onInventory(result.inventory)
      rediscover()
    } catch (e) {
      setError(String(e))
    }
  }

  const removeRule = async (rule: IgnoreRule) => {
    try {
      onSettings(await api.removeIgnore(rule.id))
      rulesChanged.current = true
      rediscover()
    } catch (e) {
      setError(String(e))
    }
  }

  const projectsIn = (folder: string) => new Set((inventory?.projects ?? []).filter((p) => isWithin(p.dir, folder)).map((p) => (p.repo ?? p.dir).toLowerCase())).size

  const suggestions = useMemo(() => {
    if (!discovered) return []
    const existing = new Set(settings.rules.filter((r) => r.kind === 'pattern').map((r) => r.value.toLowerCase()))
    return SUGGESTED_PATTERNS.filter((name) => !existing.has(name))
      .map((name) => ({ name, count: discovered.filter((p) => p.ignoredBy === null && p.dir.toLowerCase().split(/[\\/]/).includes(name)).length }))
      .filter((s) => s.count > 0)
  }, [discovered, settings.rules])

  const onTabKey = (e: KeyboardEvent, current: SettingsTab) => {
    if (e.key !== 'ArrowDown' && e.key !== 'ArrowUp') return
    e.preventDefault()
    const i = tabs.indexOf(current)
    const next = tabs[(i + (e.key === 'ArrowDown' ? 1 : tabs.length - 1)) % tabs.length]
    setTab(next)
    tabRefs.current[next]?.focus()
  }

  const [title, help] = TABS[tab]
  const repoCommands = repo && commands ? findCommands(commands, repo.key) : null
  const globalPolicy: VersionPolicy = policies['*'] ?? 'any'
  const repoPolicy = repo ? (Object.entries(policies).find(([k]) => k.toLowerCase() === repo.key.toLowerCase())?.[1] ?? null) : null
  const setPolicy = async (scope: string, policy: VersionPolicy | null) => {
    try {
      onPolicies(await api.setVersionPolicy(scope, policy))
    } catch (e) {
      setError(String(e))
    }
  }

  return (
    <Dialog title={repo ? `${repo.name} settings` : 'Settings'} bare size="settings" onClose={close}>
      <div className="grid h-full min-h-0 grid-cols-[200px_1fr]">
        <nav className="flex flex-col gap-0.5 border-r border-line bg-paper-2 px-3 py-[18px]" role="tablist" aria-orientation="vertical" aria-label="Settings sections">
          <h2 className="mx-2 mb-0.5 font-display text-[20px] font-semibold tracking-[-0.015em]">{repo ? repo.name : 'Settings'}</h2>
          <p className="mx-2 mb-3.5 text-[12.5px] text-muted">{repo ? 'Settings for this project' : 'Defaults for all projects'}</p>
          {tabs.map((t) => (
            <button
              key={t}
              ref={(el) => {
                tabRefs.current[t] = el
              }}
              type="button"
              role="tab"
              aria-selected={tab === t}
              aria-controls="settings-panel"
              tabIndex={tab === t ? 0 : -1}
              onClick={() => setTab(t)}
              onKeyDown={(e) => onTabKey(e, t)}
              className={cx(
                'h-[34px] rounded-[3px] px-2.5 text-left text-[13px]',
                tab === t ? 'bg-surface font-semibold text-ink shadow-[inset_3px_0_0_var(--state)]' : 'text-muted hover:bg-sunken hover:text-ink',
              )}
            >
              {TABS[t][0]}
            </button>
          ))}
        </nav>
        <div className="flex min-h-0 flex-col">
          <header className="flex items-start gap-2.5 px-[22px] pt-[18px] pb-2">
            <div className="flex-1">
              <h3 className="m-0 font-display text-[18px] font-semibold">{title}</h3>
              <p className="mt-0.5 mb-0 text-[12.5px] text-muted">{help}</p>
            </div>
            <button type="button" onClick={close} aria-label="Close" className="grid size-[30px] place-items-center rounded-[3px] text-muted hover:bg-sunken hover:text-ink">
              <X size={16} />
            </button>
          </header>
          {error && <p className="mx-[22px] my-1 rounded-[3px] bg-vuln-row px-3 py-2 text-[12.5px] text-risk-security">{error}</p>}
          <div id="settings-panel" role="tabpanel" className="min-h-0 flex-1 overflow-y-auto px-[22px] pt-1 pb-5">
            {tab === 'general' && (
              <>
                <SectionTitle>Appearance</SectionTitle>
                <SettingRow title="Color palette" help="Sun and serpent follows the logo: gold for actions, turquoise for selection.">
                  <Select value={prefs.palette} onChange={(e) => onPrefs({ palette: e.target.value as Prefs['palette'] })} className="w-[220px]" aria-label="Color palette">
                    <option value="faience">Sun and serpent</option>
                    <option value="parchment">Parchment and copper</option>
                  </Select>
                </SettingRow>
                <SettingRow title="Theme" help="Match Windows switches between light and dark with your Windows setting.">
                  <Select value={prefs.theme} onChange={(e) => onPrefs({ theme: e.target.value as Prefs['theme'] })} className="w-[220px]" aria-label="Theme">
                    <option value="dark">Dark</option>
                    <option value="light">Light</option>
                    <option value="system">Match Windows</option>
                  </Select>
                </SettingRow>
                <SectionTitle>Checking</SectionTitle>
                <SettingRow title="Check when Mehen opens" help="Look for new versions and advisories every time the window opens.">
                  <Switch checked={prefs.scanOnOpen} onChange={(v) => onPrefs({ scanOnOpen: v })} label="Check when Mehen opens" />
                </SettingRow>
                <SettingRow
                  title="Check automatically"
                  help={
                    settings.backgroundHours > 0
                      ? 'Mehen keeps running in the tray when you close the window and notifies you about new vulnerabilities.'
                      : 'Turn on to check from the tray and get a notification about new vulnerabilities.'
                  }
                >
                  <Select
                    value={settings.backgroundHours}
                    onChange={(e) =>
                      api
                        .setBackgroundHours(Number(e.target.value))
                        .then(onSettings)
                        .catch((err) => setError(String(err)))
                    }
                    className="w-[220px]"
                    aria-label="Check automatically"
                  >
                    {SCHEDULES.map((s) => (
                      <option key={s.hours} value={s.hours}>
                        {s.label}
                      </option>
                    ))}
                  </Select>
                </SettingRow>
              </>
            )}

            {tab === 'scanning' && (
              <>
                <SectionTitle>Watched folders</SectionTitle>
                {settings.folders.length === 0 && <p className="py-2 text-[12.5px] text-muted">No folders yet. Add the folder that holds your projects.</p>}
                {settings.folders.map((folder) => (
                  <SettingRow key={folder} title={<span className="font-mono text-[12.5px]">{folder}</span>} help={`${projectsIn(folder)} projects, including subfolders`}>
                    <div className="flex gap-1.5">
                      <Button
                        disabled={!!scanning}
                        onClick={async () => {
                          setScanning(folder)
                          try {
                            await onScanFolder(folder)
                          } finally {
                            setScanning(null)
                          }
                        }}
                      >
                        <RefreshCw size={14} className={cx(scanning === folder && 'animate-spin')} />
                        {scanning === folder ? 'Scanning…' : 'Scan'}
                      </Button>
                      <Button onClick={() => onRemoveFolder(folder)} aria-label={`Stop watching ${folder}`}>
                        Remove
                      </Button>
                    </div>
                  </SettingRow>
                ))}
                <div className="pt-2.5">
                  <Button onClick={onAddFolder}>
                    <FolderPlus size={15} />
                    Add a folder…
                  </Button>
                </div>

                <SectionTitle>Excluded</SectionTitle>
                {settings.rules.length === 0 && <p className="py-2 text-[12.5px] text-muted">Nothing is excluded.</p>}
                {settings.rules.map((rule) => {
                  const Icon = KIND_ICON[rule.kind]
                  const hidden = discovered?.filter((p) => p.ignoredBy === rule.id).length
                  return (
                    <SettingRow
                      key={rule.id}
                      title={
                        <span className="flex items-center gap-2 font-mono text-[12.5px]">
                          <Icon size={14} className="shrink-0 text-muted" />
                          <span className="truncate">{rule.kind === 'pattern' ? rule.value : relativePath(settings.folders, rule.value)}</span>
                        </span>
                      }
                      help={`${KIND_LABEL[rule.kind]}${hidden !== undefined ? ` · hides ${hidden} manifest${hidden === 1 ? '' : 's'}` : ''}`}
                    >
                      <Button onClick={() => removeRule(rule)}>
                        <Undo2 size={14} />
                        Include again
                      </Button>
                    </SettingRow>
                  )
                })}
                <form
                  className="mt-3 flex gap-1.5"
                  onSubmit={(e) => {
                    e.preventDefault()
                    addPattern(pattern)
                  }}
                >
                  <TextInput value={pattern} onChange={(e) => setPattern(e.target.value)} placeholder="Skip every folder named… e.g. fixtures" aria-label="Folder name to skip" className="flex-1" />
                  <Button type="submit" disabled={!pattern.trim()}>
                    <Plus size={15} />
                    Add
                  </Button>
                </form>
                <p className="mt-1.5 text-[12px] leading-relaxed text-muted">
                  A plain name skips every folder with that name. A path like <span className="font-mono">nuget-compass/fixtures</span> skips only that one.
                </p>
                {suggestions.length > 0 && (
                  <div className="mt-2 flex flex-wrap gap-1.5">
                    {suggestions.map((s) => (
                      <button
                        key={s.name}
                        type="button"
                        onClick={() => addPattern(s.name)}
                        title={`Skip ${s.count} manifest${s.count === 1 ? '' : 's'} inside folders named ${s.name}`}
                        className="rounded-[3px] border border-dashed border-line-strong px-2 py-0.5 text-[12px] text-muted hover:border-state hover:text-state"
                      >
                        + {s.name} <span className="text-faint">({s.count})</span>
                      </button>
                    ))}
                  </div>
                )}
              </>
            )}

            {tab === 'updates' && !repo && (
              <>
                <SectionTitle>How far updates go</SectionTitle>
                <SettingRow title="Allowed versions" help="What Mehen offers for every project. A project's own settings can change it.">
                  <Select value={globalPolicy} onChange={(e) => setPolicy('*', e.target.value === 'any' ? null : (e.target.value as VersionPolicy))} className="w-[220px]" aria-label="Allowed versions">
                    {(Object.keys(POLICY_LABEL) as VersionPolicy[]).map((p) => (
                      <option key={p} value={p}>
                        {POLICY_LABEL[p]}
                      </option>
                    ))}
                  </Select>
                </SettingRow>
                <KeptList holds={holds} nameOf={nameOf} onRelease={onRelease} />
                <SectionTitle>Running updates</SectionTitle>
                <SettingRow title="Run at most" help="Different repositories update side by side. Repositories that need the same tool (npm, cargo, dotnet) always take turns.">
                  <Select
                    value={settings.updateParallel}
                    onChange={(e) =>
                      api
                        .setUpdateParallel(Number(e.target.value))
                        .then(onSettings)
                        .catch((err) => setError(String(err)))
                    }
                    className="w-[220px]"
                    aria-label="Updates running at once"
                  >
                    <option value={0}>Automatic: {settings.updateParallelAuto} at once</option>
                    <option value={1}>1 at a time</option>
                    <option value={2}>2 at once</option>
                    <option value={3}>3 at once</option>
                    <option value={4}>4 at once</option>
                  </Select>
                </SettingRow>
                <SettingRow title="Build and test when updating" help="Your usual choice for the update button. A project whose checks fail is put back.">
                  <Switch checked={prefs.checks} onChange={(v) => onPrefs({ checks: v })} label="Build and test when updating" />
                </SettingRow>
                <SettingRow title="Stop at the first failed check" help="When off, the remaining checks still run so you see every failure at once. The project is put back either way.">
                  <Switch checked={prefs.stopOnFailure} onChange={(v) => onPrefs({ stopOnFailure: v })} label="Stop at the first failed check" />
                </SettingRow>
                <SettingRow title="Commit each repository" help="Commits only the files Mehen changed, as “Updated N Dependencies”. Never pushes.">
                  <Switch checked={prefs.commit} onChange={(v) => onPrefs({ commit: v })} label="Commit each repository" />
                </SettingRow>
                <SectionTitle>Checks by dependency type</SectionTitle>
                <p className="mb-1 text-[12.5px] text-muted">Change what counts as a passing update for every project of a kind. A project's own settings win over these.</p>
                {(['npm', 'pypi', 'cargo', 'nuget', 'go', 'pub', 'packagist', 'rubygems'] as Ecosystem[])
                  // Only the kinds the watched projects use, and any already customised.
                  .filter((e) => inventory?.projects.some((p) => p.ecosystem === e) || (commands && findCommands(commands, ecosystemScope(e))))
                  .map((e) => (
                  <EcosystemChecks key={e} ecosystem={e} commands={commands} onCommands={setCommands} />
                ))}
              </>
            )}

            {tab === 'updates' && repo && (
              <>
                <SettingRow title="Allowed versions" help="How far updates for this project may go.">
                  <Select value={repoPolicy ?? ''} onChange={(e) => setPolicy(repo.key, (e.target.value || null) as VersionPolicy | null)} className="w-[240px]" aria-label="Allowed versions for this project">
                    <option value="">Same as all projects ({POLICY_LABEL[globalPolicy].toLowerCase()})</option>
                    {(Object.keys(POLICY_LABEL) as VersionPolicy[]).map((p) => (
                      <option key={p} value={p}>
                        {POLICY_LABEL[p]}
                      </option>
                    ))}
                  </Select>
                </SettingRow>
                <KeptList holds={holds.filter((h) => h.scope === '*' || samePath(h.scope, repo.key))} nameOf={nameOf} onRelease={onRelease} />
                <SectionTitle>Checks</SectionTitle>
                <SettingRow
                  title={`Use custom checks for ${repo.name}`}
                  help={
                    repoCommands
                      ? 'These replace every build and test step for this project.'
                      : `Mehen runs ${repo.ecosystems
                          .map((e) => (commands && findCommands(commands, ecosystemScope(e))?.commands.join(', ')) || DEFAULT_CHECKS[e])
                          .filter(Boolean)
                          .join('; ') || 'no checks for this project'}.`
                  }
                >
                  <Switch
                    checked={!!repoCommands}
                    disabled={!commands}
                    onChange={async (on) => setCommands(await api.setCheckCommands(repo.key, on ? [] : null))}
                    label={`Use custom checks for ${repo.name}`}
                  />
                </SettingRow>
                {repoCommands && <CommandEditor key={repo.key} scope={repo.key} initial={repoCommands} onSaved={setCommands} placeholder="npm run test:ci" />}
                <p className="mt-3 text-[12px] text-muted">
                  <b className="text-ink">Which setting wins:</b> this project, then its dependency type, then Mehen's defaults.
                </p>
              </>
            )}

            {tab === 'security' && (
              <>
                <SectionTitle>Alerts</SectionTitle>
                <SettingRow
                  title="Notify about new vulnerabilities"
                  help={settings.backgroundHours > 0 ? 'A Windows notification names the affected projects when a background check finds something new.' : 'Notifications come from background checks, which are off. Turn them on under General.'}
                >
                  <Switch
                    checked={settings.notify}
                    onChange={(v) =>
                      api
                        .setNotify(v)
                        .then(onSettings)
                        .catch((err) => setError(String(err)))
                    }
                    label="Notify about new vulnerabilities"
                  />
                </SettingRow>
                <SettingRow title="Show vulnerable packages first" help="Security fixes, then major, minor, and patch updates. Turn off to list packages by name.">
                  <Switch checked={prefs.riskFirst} onChange={(v) => onPrefs({ riskFirst: v })} label="Show vulnerable packages first" />
                </SettingRow>
                <SectionTitle>Where advisories come from</SectionTitle>
                <p className="py-1 text-[12.5px] leading-relaxed text-muted">
                  Mehen asks the open OSV database (osv.dev) about every installed version. OSV gathers GitHub Security Advisories, which cover npm and
                  NuGet, and the RustSec database for Cargo. Answers are cached locally, and Check everything again skips the cache.
                </p>
              </>
            )}
          </div>
          <footer className="flex justify-end gap-2 border-t border-line px-[22px] py-3">
            <Button variant="primary" onClick={close}>
              Done
            </Button>
          </footer>
        </div>
      </div>
    </Dialog>
  )
}

function EcosystemChecks({ ecosystem, commands, onCommands }: { ecosystem: Ecosystem; commands: CheckCommands | null; onCommands: (c: CheckCommands) => void }) {
  const scope = ecosystemScope(ecosystem)
  const current = commands ? findCommands(commands, scope) : null
  const [open, setOpen] = useState(false)
  return (
    <div className="border-b border-line">
      <SettingRow title={ECOSYSTEM_LABEL[ecosystem]} help={current ? `${current.commands.join(', ') || 'No checks'}${current.cwd ? ` in ${current.cwd}` : ''}` : `Mehen's defaults: ${DEFAULT_CHECKS[ecosystem]}`}>
        <div className="flex gap-1.5">
          {current && (
            <Button variant="ghost" onClick={async () => (onCommands(await api.setCheckCommands(scope, null)), setOpen(false))}>
              Use defaults
            </Button>
          )}
          <Button onClick={() => setOpen(!open)} aria-expanded={open} disabled={!commands}>
            {open ? 'Close' : 'Customize'}
          </Button>
        </div>
      </SettingRow>
      {open && commands && <CommandEditor scope={scope} initial={current ?? { commands: [], cwd: null }} onSaved={(c) => (onCommands(c), setOpen(false))} placeholder={ecosystem === 'nuget' ? 'dotnet test' : `${ecosystem === 'npm' ? 'npm' : 'cargo'} test`} />}
    </div>
  )
}


/** Packages kept on a release line, each with a way to let it move on. */
function KeptList({ holds, nameOf, onRelease }: { holds: Hold[]; nameOf: (folder: string) => string; onRelease: (hold: Hold) => void }) {
  const sorted = [...holds].sort((a, b) => a.name.localeCompare(b.name, undefined, { sensitivity: 'base' }) || a.scope.localeCompare(b.scope))
  return (
    <>
      <SectionTitle>Kept on a release line</SectionTitle>
      {sorted.length === 0 ? (
        <p className="py-2 text-[12.5px] text-muted">
          Nothing is kept back. Use <Pin size={12} className="inline align-[-1px]" /> on a package to keep it on its current release line, such as 5.x.
        </p>
      ) : (
        sorted.map((h) => (
          <SettingRow
            key={h.id}
            title={
              <span className="flex items-center gap-2">
                <Pin size={13} className="shrink-0 text-state" />
                <span className="truncate">{h.name}</span>
                <code className="font-mono text-[12px] text-muted">{h.line}.x</code>
              </span>
            }
            help={`${ECOSYSTEM_LABEL[h.ecosystem]} · ${h.scope === '*' ? 'every project' : nameOf(h.scope)}`}
          >
            <Button onClick={() => onRelease(h)}>Stop keeping</Button>
          </SettingRow>
        ))
      )}
    </>
  )
}
