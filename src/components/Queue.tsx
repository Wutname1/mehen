import { ArrowDownUp, ChevronDown, Info, Pin, Plus, ShieldAlert, X } from 'lucide-react'
import { useLayoutEffect, useRef, useState, type InputHTMLAttributes, type KeyboardEvent, type ReactNode } from 'react'
import { ECOSYSTEM_LABEL, ECOSYSTEMS, PROJECT_TYPE_LABEL, distinctVersions, displayVersion, holdLine, reasonText, relativePath, repoKey, type ProjectType, type QueueRow, type QueueUsage, type Repo, type Risk } from '../derive'
import type { Ecosystem, Hold } from '../types'
import { cx } from './bits'
import { EcoIcon, ProjectTypeIcon } from './EcoIcon'
import { Menu, MenuCheck, MenuItem, MenuSeparator, MenuTitle } from './Menu'

export type RiskFilter = 'any' | 'attention' | 'security'
export const RISK_FILTER_LABEL: Record<RiskFilter, string> = { any: 'Any risk', attention: 'Needs attention', security: 'Security only' }

const RISK_HINT: Record<Risk, string> = {
  security: 'Fixes a known vulnerability',
  major: 'Major version: expect breaking changes',
  minor: 'New features, no breaking changes expected',
  patch: 'Bug fixes only',
}

const RISK_TEXT: Record<Risk, string> = {
  security: 'text-risk-security',
  major: 'text-risk-major',
  minor: 'text-risk-minor',
  patch: 'text-risk-patch',
}

/** A row limited to the usages the current scope and filters allow. */
export interface ScopedRow {
  row: QueueRow
  usages: QueueUsage[]
}

export interface QueueFilters {
  types: Set<ProjectType>
  ecosystems: Set<Ecosystem>
  risk: RiskFilter
  riskFirst: boolean
}

export function RiskPill({ risk }: { risk: Risk }) {
  return (
    <span
      title={RISK_HINT[risk]}
      className={cx(
        'inline-flex h-[22px] items-center gap-1 justify-self-start rounded-[2px] border px-[7px] font-mono text-[11px] font-semibold tracking-[0.04em] uppercase',
        RISK_TEXT[risk],
      )}
      style={{ borderColor: 'color-mix(in oklab, currentColor 40%, transparent)', background: 'color-mix(in oklab, currentColor 8%, transparent)' }}
    >
      {risk === 'security' && <ShieldAlert size={13} />}
      {risk}
    </span>
  )
}

export function Checkbox({ checked, partial, onChange, label, ...rest }: { checked: boolean; partial?: boolean; onChange: () => void; label: string } & Omit<InputHTMLAttributes<HTMLInputElement>, 'checked' | 'onChange' | 'type'>) {
  const ref = useRef<HTMLInputElement>(null)
  useLayoutEffect(() => {
    if (ref.current) ref.current.indeterminate = !!partial && !checked
  }, [partial, checked])
  return (
    <input
      ref={ref}
      type="checkbox"
      checked={checked}
      onChange={onChange}
      aria-label={label}
      {...rest}
      className="grid size-[18px] cursor-pointer appearance-none place-items-center rounded-[3px] border-[1.5px] border-line-strong bg-surface after:content-[''] checked:border-check checked:bg-check checked:after:-mt-0.5 checked:after:h-[9px] checked:after:w-[5px] checked:after:rotate-45 checked:after:border-r-2 checked:after:border-b-2 checked:after:border-paper indeterminate:border-check indeterminate:bg-check indeterminate:after:h-0.5 indeterminate:after:w-[9px] indeterminate:after:bg-paper hover:border-muted disabled:cursor-not-allowed disabled:opacity-45"
    />
  )
}

function FilterButton({ label, value, active, menu }: { label: string; value: string; active: boolean; menu: (anchor: HTMLElement, close: () => void) => ReactNode }) {
  const [anchor, setAnchor] = useState<HTMLElement | null>(null)
  return (
    <>
      <button
        type="button"
        onClick={(e) => setAnchor(anchor ? null : e.currentTarget)}
        aria-haspopup="menu"
        aria-expanded={!!anchor}
        className={cx(
          'grid h-[42px] min-w-[128px] grid-cols-[1fr_14px] items-center gap-x-2 rounded-[3px] border bg-surface px-2.5 py-1 text-left hover:border-muted max-[1100px]:min-w-[96px]',
          active ? 'border-state shadow-[inset_0_0_0_1px_var(--state)]' : anchor ? 'border-focus' : 'border-line-strong',
        )}
      >
        <span className="truncate text-[11px] whitespace-nowrap text-muted">{label}</span>
        <ChevronDown size={14} className="row-span-2 text-muted" />
        <b className={cx('truncate text-[12.5px] font-semibold', active && 'text-state')}>{value}</b>
      </button>
      {anchor && menu(anchor, () => setAnchor(null))}
    </>
  )
}

function toggle<T>(set: Set<T>, value: T | 'all'): Set<T> {
  if (value === 'all') return new Set()
  const next = new Set(set)
  if (next.has(value)) next.delete(value)
  else next.add(value)
  return next
}

export function Queue({
  rows,
  repo,
  filters,
  onFilters,
  selected,
  onToggle,
  onSelectCompatible,
  onClearScope,
  security,
  onSelectFixes,
  onAdvisory,
  searching,
  onClearSearch,
  holds,
  heldBack,
  nameOf,
  onKeep,
  onRelease,
  onWhy,
}: {
  rows: ScopedRow[]
  repo: Repo | null
  filters: QueueFilters
  onFilters: (f: QueueFilters) => void
  selected: Set<string>
  onToggle: (usages: QueueUsage[]) => void
  onSelectCompatible: () => void
  onClearScope: () => void
  security: { packages: number; projects: number; allSelected: boolean } | null
  onSelectFixes: () => void
  onAdvisory: (row: QueueRow) => void
  searching: boolean
  onClearSearch: () => void
  holds: Hold[]
  /** Packages in scope with a newer release that does not fit. */
  heldBack: number
  nameOf: (folder: string) => string
  onKeep: (row: QueueRow, scope: string, line: string) => void
  onRelease: (hold: Hold) => void
  /** Opens the held-back explanation, for one package or all of them. */
  onWhy: (packageKey: string | null) => void
}) {
  const [keepMenu, setKeepMenu] = useState<{ row: QueueRow; usages: QueueUsage[]; anchor: HTMLElement } | null>(null)
  const holdsOf = (row: QueueRow) => holds.filter((h) => h.ecosystem === row.ecosystem && h.name === row.name)
  const shown = rows.flatMap((r) => r.usages)
  const shownSelected = shown.filter((u) => selected.has(u.key)).length
  const compatibleLeft = rows.some((r) => r.row.risk !== 'major' && r.usages.some((u) => !selected.has(u.key)))
  const filtered = filters.types.size > 0 || filters.ecosystems.size > 0 || filters.risk !== 'any' || searching

  const moveFocus = (e: KeyboardEvent<HTMLInputElement>) => {
    if (e.key !== 'ArrowDown' && e.key !== 'ArrowUp') return
    e.preventDefault()
    const all = [...document.querySelectorAll<HTMLInputElement>('[data-queue-check]')]
    const i = all.indexOf(e.currentTarget)
    all[Math.max(0, Math.min(all.length - 1, i + (e.key === 'ArrowDown' ? 1 : -1)))]?.focus()
  }

  const typeValue = filters.types.size ? [...filters.types].map((t) => PROJECT_TYPE_LABEL[t]).join(', ') : 'All'
  const ecoValue = filters.ecosystems.size ? [...filters.ecosystems].map((e) => ECOSYSTEM_LABEL[e]).join(', ') : 'All'

  return (
    <main className="min-h-0 min-w-0 overflow-y-auto bg-paper px-5 pt-4 pb-8 max-[1279px]:px-3.5" aria-label="Dependency updates">
      <div className="mb-3 flex items-center gap-2">
        {repo && (
          <div className="flex h-[42px] items-center gap-2 rounded-[3px] border border-line-strong bg-row-selected pr-1.5 pl-2.5 text-[12.5px] whitespace-nowrap">
            <span className="flex flex-col">
              <small className="text-[11px] text-muted">Showing</small>
              <b className="font-semibold">{repo.name}</b>
            </span>
            <button type="button" onClick={onClearScope} aria-label="Show all projects" title="Back to all projects" className="grid size-[26px] place-items-center rounded-[3px] text-muted hover:bg-sunken hover:text-ink">
              <X size={15} />
            </button>
          </div>
        )}
        <div className="flex min-w-0 flex-1 gap-2">
          {!repo && (
            <FilterButton
              label="Project type"
              value={typeValue}
              active={filters.types.size > 0}
              menu={(anchor, close) => (
                <Menu anchor={anchor} label="Project type" onClose={close}>
                  <MenuCheck radio checked={!filters.types.size} onSelect={() => onFilters({ ...filters, types: new Set() })}>
                    All project types
                  </MenuCheck>
                  <MenuSeparator />
                  {(Object.keys(PROJECT_TYPE_LABEL) as ProjectType[]).map((t) => (
                    <MenuCheck key={t} checked={filters.types.has(t)} onSelect={() => onFilters({ ...filters, types: toggle(filters.types, t) })} icon={<ProjectTypeIcon type={t} size={15} />}>
                      {PROJECT_TYPE_LABEL[t]}
                    </MenuCheck>
                  ))}
                </Menu>
              )}
            />
          )}
          <FilterButton
            label="Dependency type"
            value={ecoValue}
            active={filters.ecosystems.size > 0}
            menu={(anchor, close) => (
              <Menu anchor={anchor} label="Dependency type" onClose={close}>
                <MenuCheck radio checked={!filters.ecosystems.size} onSelect={() => onFilters({ ...filters, ecosystems: new Set() })}>
                  All dependency types
                </MenuCheck>
                <MenuSeparator />
                {ECOSYSTEMS.map((e) => (
                  <MenuCheck key={e} checked={filters.ecosystems.has(e)} onSelect={() => onFilters({ ...filters, ecosystems: toggle(filters.ecosystems, e) })} icon={<EcoIcon ecosystem={e} size={15} />}>
                    {ECOSYSTEM_LABEL[e]}
                  </MenuCheck>
                ))}
              </Menu>
            )}
          />
          <FilterButton
            label="Risk"
            value={RISK_FILTER_LABEL[filters.risk]}
            active={filters.risk !== 'any'}
            menu={(anchor, close) => (
              <Menu anchor={anchor} label="Risk" onClose={close}>
                {(Object.keys(RISK_FILTER_LABEL) as RiskFilter[]).map((r) => (
                  <MenuCheck
                    key={r}
                    radio
                    checked={filters.risk === r}
                    onSelect={() => {
                      onFilters({ ...filters, risk: r })
                      close()
                    }}
                  >
                    {RISK_FILTER_LABEL[r]}
                  </MenuCheck>
                ))}
              </Menu>
            )}
          />
        </div>
        <button
          type="button"
          onClick={onSelectCompatible}
          disabled={!compatibleLeft}
          title="Select every patch, minor, and security update. Major versions stay unselected."
          aria-label="Select compatible updates"
          className="inline-flex h-[42px] shrink-0 items-center gap-1.5 rounded-[3px] border border-accent-text px-3 text-[12.5px] font-semibold whitespace-nowrap text-accent-text hover:bg-[color-mix(in_oklab,var(--accent-text)_10%,transparent)] disabled:opacity-45"
        >
          <Plus size={15} />
          <span className="max-[1100px]:sr-only">Select compatible</span>
        </button>
      </div>

      {security && (
        <div className="mb-3 flex items-center gap-3 rounded-[3px] border border-[color-mix(in_oklab,var(--risk-security)_22%,var(--line))] bg-vuln-row px-3 py-2.5">
          <ShieldAlert size={20} className="shrink-0 text-risk-security" />
          <span className="flex flex-1 flex-col">
            <b className="text-[13px] font-semibold">
              {security.packages} package{security.packages === 1 ? ' has' : 's have'} known vulnerabilities
            </b>
            <small className="text-[12px] text-muted">
              Used by {security.projects} project{security.projects === 1 ? '' : 's'}. Updating to the target version fixes {security.packages === 1 ? 'it' : 'them'}.
            </small>
          </span>
          <button
            type="button"
            onClick={() => onFilters({ ...filters, risk: filters.risk === 'security' ? 'any' : 'security' })}
            className="h-8 rounded-[3px] px-3 text-[12.5px] font-semibold text-muted hover:bg-sunken hover:text-ink"
          >
            {filters.risk === 'security' ? 'Show everything' : 'Show only these'}
          </button>
          <button
            type="button"
            onClick={onSelectFixes}
            disabled={security.allSelected}
            className="h-8 rounded-[3px] border border-[color-mix(in_oklab,var(--risk-security)_55%,transparent)] px-3 text-[12.5px] font-semibold text-risk-security hover:border-risk-security hover:bg-[color-mix(in_oklab,var(--risk-security)_8%,transparent)] disabled:opacity-45"
          >
            {security.allSelected ? 'Fixes selected' : 'Select fixes'}
          </button>
        </div>
      )}

      <div className="mt-1 mb-1.5 flex items-center justify-between">
        <b className="font-display text-[15px] font-semibold" aria-live="polite">
          {rows.length} package{rows.length === 1 ? '' : 's'}
        </b>
        <button
          type="button"
          aria-pressed={filters.riskFirst}
          onClick={() => onFilters({ ...filters, riskFirst: !filters.riskFirst })}
          className={cx(
            'inline-flex h-[30px] items-center gap-1.5 rounded-[3px] border px-2.5 text-[12px]',
            filters.riskFirst ? 'border-line-strong bg-surface text-ink' : 'border-transparent text-muted hover:bg-sunken hover:text-ink',
          )}
        >
          <ArrowDownUp size={14} />
          Risk first
        </button>
      </div>

      {rows.length > 0 ? (
        <div role="table" aria-label="Dependency updates" aria-rowcount={rows.length + 1} className="border-t border-line-strong">
          <div role="rowgroup">
            <div role="row" className="grid min-h-9 grid-cols-[40px_minmax(170px,1fr)_104px_104px_minmax(96px,150px)_112px] items-center border-b border-line font-mono text-[11px] tracking-[0.04em] text-muted uppercase max-[1279px]:grid-cols-[36px_minmax(140px,1fr)_88px_88px_minmax(84px,120px)_104px] max-[1100px]:grid-cols-[32px_minmax(120px,1fr)_76px_76px_minmax(64px,96px)_96px]">
              <span role="columnheader" className="grid place-items-center">
                <Checkbox
                  checked={shown.length > 0 && shownSelected === shown.length}
                  partial={shownSelected > 0}
                  onChange={() => onToggle(shown)}
                  label={`Select all ${rows.length} shown packages`}
                />
              </span>
              <span role="columnheader">Package</span>
              <span role="columnheader">Installed</span>
              <span role="columnheader">Target</span>
              <span role="columnheader">{repo ? 'File' : 'Used by'}</span>
              <span role="columnheader">Risk</span>
            </div>
          </div>
          <div role="rowgroup">
            {rows.map(({ row, usages }, i) => {
              const picked = usages.filter((u) => selected.has(u.key)).length
              const installed = distinctVersions(usages.map((u) => displayVersion(u.dep)))
              const targets = distinctVersions(usages.map((u) => u.target))
              const projects = new Set(usages.map((u) => (u.project.repo ?? u.project.dir).toLowerCase())).size
              const vulnerable = row.vulnIds.length > 0
              return (
                <div
                  key={row.key}
                  role="row"
                  aria-rowindex={i + 2}
                  className={cx(
                    'group grid min-h-[50px] grid-cols-[40px_minmax(170px,1fr)_104px_104px_minmax(96px,150px)_112px] items-center border-b border-line max-[1279px]:grid-cols-[36px_minmax(140px,1fr)_88px_88px_minmax(84px,120px)_104px] max-[1100px]:grid-cols-[32px_minmax(120px,1fr)_76px_76px_minmax(64px,96px)_96px]',
                    vulnerable ? (picked ? 'bg-vuln-selected' : 'bg-vuln-row') : picked ? 'bg-row-selected' : 'hover:bg-row-hover',
                  )}
                >
                  <span role="cell" className="grid place-items-center self-stretch">
                    <Checkbox
                      data-queue-check
                      onKeyDown={moveFocus}
                      checked={picked === usages.length}
                      partial={picked > 0}
                      onChange={() => onToggle(usages)}
                      label={`Select ${row.name} ${installed.join(', ')} to ${targets.at(-1)}${repo ? '' : ` in ${projects} project${projects === 1 ? '' : 's'}`}`}
                    />
                  </span>
                  <span role="cell" className="flex min-w-0 flex-col gap-0.5 pr-3">
                    <b className="truncate text-[13px] font-semibold">{row.name}</b>
                    <small className="flex flex-wrap items-center gap-x-2.5 font-mono text-[12px] text-muted">
                      {ECOSYSTEM_LABEL[row.ecosystem]}
                      {holdsOf(row).map((h) => (
                        <span key={h.id} className="inline-flex items-center gap-1 font-sans text-[12px] text-state" title={`Kept on ${h.line}.x ${h.scope === '*' ? 'in every project' : `in ${nameOf(h.scope)}`}`}>
                          <Pin size={12} />
                          {h.line}.x
                        </span>
                      ))}
                      {vulnerable && (
                        <button type="button" onClick={() => onAdvisory(row)} className="inline-flex items-center gap-1 font-sans text-[12px] font-semibold whitespace-nowrap text-risk-security underline underline-offset-2">
                          <ShieldAlert size={13} />
                          {row.vulnIds.length} advisor{row.vulnIds.length === 1 ? 'y' : 'ies'}
                        </button>
                      )}
                    </small>
                  </span>
                  <code role="cell" className="font-mono text-[12.5px]" title={installed.join(', ')}>
                    {installed[0]}
                    {installed.length > 1 && <span className="ml-1 font-sans text-[11px] text-muted">+{installed.length - 1}</span>}
                  </code>
                  <span role="cell" className="flex min-w-0 items-center gap-1">
                    <code className="font-mono text-[12.5px] font-semibold" title={targets.join(', ')}>
                      {targets.length > 1 && <span className="mr-1 font-sans text-[11px] font-normal text-muted">up to</span>}
                      {targets.at(-1)}
                    </code>
                    {(() => {
                      const blocked = usages.filter((u) => u.dep.newest && u.dep.blockedReason)
                      if (!blocked.length) return null
                      const newest = distinctVersions(blocked.map((u) => u.dep.newest!)).at(-1)
                      const reasons = [...new Set(blocked.map((u) => reasonText(u.dep.blockedReason!)))]
                      return (
                        <button
                          type="button"
                          onClick={() => onWhy(row.key)}
                          aria-label={`Why not ${newest}?`}
                          title={`${newest} is out. ${reasons.slice(0, 2).join('. ')}${reasons.length > 2 ? ` (+${reasons.length - 2} more)` : ''}.`}
                          className="grid size-5 shrink-0 place-items-center rounded-[3px] text-faint hover:bg-sunken hover:text-ink"
                        >
                          <Info size={13} />
                        </button>
                      )
                    })()}
                  </span>
                  <span role="cell" className="min-w-0 truncate pr-2.5 text-[12px] text-muted">
                    {repo ? (
                      <code className="font-mono" title={usages.map((u) => u.project.manifest).join('\n')}>
                        {relativePath([repo.key], usages[0].project.manifest)}
                        {usages.length > 1 && ` +${usages.length - 1}`}
                      </code>
                    ) : (
                      `${projects} project${projects === 1 ? '' : 's'}`
                    )}
                  </span>
                  <span role="cell" className="flex items-center justify-between pr-1.5">
                    <RiskPill risk={row.risk} />
                    <button
                      type="button"
                      onClick={(e) => setKeepMenu({ row, usages, anchor: e.currentTarget })}
                      aria-label={`Keep ${row.name} on a release line`}
                      aria-haspopup="menu"
                      title="Keep on this release line"
                      className={cx(
                        'grid size-6 shrink-0 place-items-center rounded-[3px] text-muted hover:bg-sunken hover:text-ink focus-visible:opacity-100',
                        keepMenu?.row.key === row.key ? 'opacity-100' : 'opacity-0 group-hover:opacity-100 group-focus-within:opacity-100',
                      )}
                    >
                      <Pin size={14} />
                    </button>
                  </span>
                </div>
              )
            })}
          </div>
        </div>
      ) : (
        <div className="py-14 text-center text-muted">
          {filtered ? (
            <>
              <h3 className="mb-3 font-display text-[17px] font-semibold text-ink">No packages match</h3>
              <button
                type="button"
                onClick={() => {
                  onClearSearch()
                  onFilters({ types: new Set(), ecosystems: new Set(), risk: 'any', riskFirst: filters.riskFirst })
                }}
                className="h-8 rounded-[3px] border border-line-strong bg-surface px-3 text-[12.5px] font-semibold text-ink"
              >
                Clear search and filters
              </button>
            </>
          ) : (
            <>
              <h3 className="mb-2 font-display text-[17px] font-semibold text-ink">{repo ? `${repo.name} is up to date` : 'Everything is up to date'}</h3>
              <p>Mehen will tell you when a new version or advisory shows up.</p>
            </>
          )}
        </div>
      )}
      {heldBack > 0 && (
        <button type="button" onClick={() => onWhy(null)} className="mt-3 inline-flex items-center gap-1.5 text-[12px] text-muted hover:text-ink">
          <Info size={13} />
          {heldBack} newer version{heldBack === 1 ? '' : 's'} held back. Why?
        </button>
      )}
      {keepMenu &&
        (() => {
          const { row, usages, anchor } = keepMenu
          const lines = distinctVersions(usages.map((u) => holdLine(displayVersion(u.dep))))
          const line = lines[0]
          const repos = [...new Set(usages.map((u) => repoKey(u.project)))]
          const only = repo ? repo.key : repos.length === 1 ? repos[0] : null
          const close = () => setKeepMenu(null)
          return (
            <Menu anchor={anchor} label={`Keep ${row.name}`} placement="below-end" width={280} onClose={close}>
              <MenuTitle title={`Keep ${row.name} on ${line}.x`} detail="Updates stay within this release line" />
              <MenuItem icon={<Pin size={15} />} onSelect={() => (close(), onKeep(row, '*', line))}>
                In every project
              </MenuItem>
              {only && (
                <MenuItem icon={<Pin size={15} />} onSelect={() => (close(), onKeep(row, only, line))}>
                  Only in {nameOf(only)}
                </MenuItem>
              )}
              {holdsOf(row).length > 0 && <MenuSeparator />}
              {holdsOf(row).map((h) => (
                <MenuItem key={h.id} icon={<X size={15} />} onSelect={() => (close(), onRelease(h))}>
                  Stop keeping on {h.line}.x {h.scope === '*' ? 'everywhere' : `in ${nameOf(h.scope)}`}
                </MenuItem>
              ))}
            </Menu>
          )
        })()}
    </main>
  )
}

