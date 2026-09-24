import { Check, ChevronRight, ShieldAlert } from 'lucide-react'
import { useState } from 'react'
import * as api from '../api'
import { compareVersions, distinctVersions, displayVersion, normalizeSeverity, repoKey, vulnById, type QueueRow, type QueueUsage } from '../derive'
import type { AffectedRange, Inventory } from '../types'
import { cx } from './bits'
import { Button, Dialog } from './Dialog'

type Pick = 'newest' | 'fix'

function covers(range: AffectedRange, version: string): boolean {
  const started = !range.introduced || compareVersions(version, range.introduced) >= 0
  if (range.fixed) return started && compareVersions(version, range.fixed) < 0
  if (range.lastAffected) return started && compareVersions(version, range.lastAffected) <= 0
  return started
}

function rangeText(range: AffectedRange): string {
  const from = range.introduced
  if (range.fixed) return from ? `${from} up to ${range.fixed}` : `before ${range.fixed}`
  if (range.lastAffected) return from ? `${from} to ${range.lastAffected}` : `up to ${range.lastAffected}`
  return from ? `${from} and later` : 'every version'
}

/** `15` for 15.1.3, `0.13` for 0.13.4. */
const lineOf = (version: string) => {
  const [major, minor] = version.replace(/^[^\d]+/, '').split('.')
  return major === '0' && minor ? `0.${minor}` : major
}

/**
 * The advisories behind a vulnerable package. The newest release is the
 * usual choice; when an older line is also safe, it is offered as the
 * smallest fix, with the ranges each advisory covers so the choice is clear.
 */
export function AdvisoryDialog({
  row,
  inventory,
  chosenTarget,
  onSelect,
  onClose,
}: {
  row: QueueRow
  inventory: Inventory
  /** Where a usage is set to go if it is selected, else null. */
  chosenTarget: (usage: QueueUsage) => string | null
  onSelect: (pick: Pick) => void
  onClose: () => void
}) {
  const byId = vulnById(inventory)
  const vulns = row.vulnIds.map((id) => byId.get(id)).filter((v) => !!v)
  const affected = row.usages.filter((u) => u.dep.vulns.length > 0)
  const installed = distinctVersions(affected.map((u) => displayVersion(u.dep)))
  const projects = new Set(affected.map((u) => repoKey(u.project).toLowerCase())).size
  const targetFor = (u: QueueUsage, pick: Pick) => (pick === 'fix' && u.dep.fixTarget) || u.target
  const newest = distinctVersions(affected.map((u) => targetFor(u, 'newest')))
  const smallest = distinctVersions(affected.map((u) => targetFor(u, 'fix')))
  const hasChoice = affected.some((u) => u.dep.fixTarget && u.dep.fixTarget !== u.target)
  const [pick, setPick] = useState<Pick>(() => (hasChoice && affected.every((u) => chosenTarget(u) === targetFor(u, 'fix')) ? 'fix' : 'newest'))
  const picked = pick === 'fix' ? smallest : newest
  const alreadySelected = affected.every((u) => chosenTarget(u) === targetFor(u, pick))
  const rangesOf = (v: (typeof vulns)[number]) => v.fixed.filter((f) => f.ecosystem === row.ecosystem && f.name.toLowerCase() === row.name.toLowerCase()).flatMap((f) => f.ranges ?? [])
  // Every version affected and nothing fixed yet: no update helps.
  const unfixed = new Set(vulns.filter((v) => rangesOf(v).some((r) => !r.fixed && !r.lastAffected)).map((v) => v.id))
  const fixable = vulns.length - unfixed.size
  const them = fixable === 1 ? 'it' : 'them'
  const noFix = unfixed.size ? ` ${unfixed.size === vulns.length ? (unfixed.size === 1 ? 'It has' : 'They have') : `${unfixed.size} ${unfixed.size === 1 ? 'has' : 'have'}`} no fix yet.` : ''

  const option = (value: Pick, versions: string[], detail: string) => (
    <label
      className={cx(
        'grid cursor-pointer grid-cols-[16px_1fr] items-start gap-2.5 rounded-[3px] border px-3 py-2.5',
        pick === value ? 'border-state bg-[color-mix(in_oklab,var(--state)_8%,transparent)]' : 'border-line bg-paper hover:border-line-strong',
      )}
    >
      <input type="radio" name="fix" checked={pick === value} onChange={() => setPick(value)} className="mt-0.5 accent-[var(--state)]" />
      <span className="min-w-0">
        <b className="font-mono text-[13px]">{versions.join(', ')}</b>
        <small className="block text-[12.5px] text-muted">{detail}</small>
      </span>
    </label>
  )

  return (
    <Dialog
      title={`${row.name} advisories`}
      description={`You have ${installed.join(', ')} in ${projects} project${projects === 1 ? '' : 's'}.${hasChoice || !fixable ? '' : ` Updating to ${newest.at(-1)} fixes ${unfixed.size ? `${fixable} of them` : them}.`}${noFix}`}
      icon={<ShieldAlert size={22} />}
      tone="danger"
      onClose={onClose}
      onEnter={() => !alreadySelected && onSelect(pick)}
      footer={
        <>
          <Button onClick={onClose}>Close</Button>
          <Button variant="primary" onClick={() => onSelect(pick)} disabled={alreadySelected}>
            {alreadySelected ? (
              <>
                <Check size={15} />
                Selected
              </>
            ) : (
              `Select ${picked.join(', ')}`
            )}
          </Button>
        </>
      }
    >
      {hasChoice && (
        <fieldset className="m-0 mb-3 grid gap-1.5 border-0 p-0">
          <legend className="mb-1.5 font-mono text-[11px] tracking-[0.04em] text-muted uppercase">Update to</legend>
          {option('newest', newest, `Newest release. Also fixes ${them}.`)}
          {option('fix', smallest, `Smallest version that fixes ${them}. Stays on ${smallest.map((v) => `${lineOf(v)}.x`).join(', ')}.`)}
        </fieldset>
      )}
      <div className="grid gap-2">
        {vulns.map((v) => {
          const ranges = rangesOf(v).sort((a, b) => (a.introduced ? (b.introduced ? compareVersions(a.introduced, b.introduced) : 1) : b.introduced ? -1 : 0))
          return (
            <a
              key={v.id}
              href={v.url}
              onClick={(e) => {
                e.preventDefault()
                api.openLink(v.url)
              }}
              className="grid grid-cols-[1fr_auto_16px] items-center gap-3 rounded-[3px] border border-line bg-paper px-3 py-2.5 text-inherit no-underline hover:border-line-strong"
            >
              <span className="min-w-0">
                <b className="block font-mono text-[12.5px] font-semibold">{v.aliases.find((a) => a.startsWith('CVE-')) ?? v.id}</b>
                <small className="text-[12.5px] text-muted">{v.summary}</small>
                {unfixed.has(v.id) && <small className="mt-0.5 block text-[12px] font-semibold text-risk-review">No fixed version yet</small>}
                {ranges.length > 0 && !unfixed.has(v.id) && (
                  <small className="mt-0.5 block text-[12px] text-muted">
                    Affects{' '}
                    {ranges.map((r, i) => {
                      const yours = installed.filter((version) => covers(r, version))
                      return (
                        <span key={i}>
                          {i > 0 && ' · '}
                          <span className={cx(yours.length > 0 && 'font-semibold text-ink')}>{rangeText(r)}</span>
                          {yours.length > 0 && ` (yours: ${yours.join(', ')})`}
                        </span>
                      )
                    })}
                  </small>
                )}
              </span>
              <span className="inline-flex h-[22px] items-center rounded-[2px] border border-[color-mix(in_oklab,var(--risk-security)_40%,transparent)] px-[7px] font-mono text-[11px] font-semibold tracking-[0.04em] text-risk-security uppercase">
                {normalizeSeverity(v.severity)}
              </span>
              <ChevronRight size={16} className="text-muted" />
            </a>
          )
        })}
      </div>
    </Dialog>
  )
}
