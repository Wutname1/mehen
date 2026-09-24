import { Check, ChevronRight, ShieldAlert } from 'lucide-react'
import * as api from '../api'
import { distinctVersions, displayVersion, normalizeSeverity, repoKey, vulnById, type QueueRow } from '../derive'
import type { Inventory } from '../types'
import { Button, Dialog } from './Dialog'

/** The advisories behind a vulnerable package and the version that fixes them. */
export function AdvisoryDialog({ row, inventory, allSelected, onSelect, onClose }: { row: QueueRow; inventory: Inventory; allSelected: boolean; onSelect: () => void; onClose: () => void }) {
  const byId = vulnById(inventory)
  const vulns = row.vulnIds.map((id) => byId.get(id)).filter((v) => !!v)
  const affected = row.usages.filter((u) => u.dep.vulns.length > 0)
  const installed = distinctVersions(affected.map((u) => displayVersion(u.dep)))
  const targets = distinctVersions(affected.map((u) => u.target))
  const projects = new Set(affected.map((u) => repoKey(u.project).toLowerCase())).size

  return (
    <Dialog
      title={`${row.name} advisories`}
      description={`You have ${installed.join(', ')} in ${projects} project${projects === 1 ? '' : 's'}. Updating to ${targets.at(-1)} fixes ${vulns.length === 1 ? 'it' : 'them'}.`}
      icon={<ShieldAlert size={22} />}
      tone="danger"
      onClose={onClose}
      footer={
        <>
          <Button onClick={onClose}>Close</Button>
          <Button variant="primary" onClick={onSelect} disabled={allSelected}>
            {allSelected ? (
              <>
                <Check size={15} />
                Fix selected
              </>
            ) : (
              'Select fix'
            )}
          </Button>
        </>
      }
    >
      <div className="grid gap-2">
        {vulns.map((v) => (
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
            </span>
            <span className="inline-flex h-[22px] items-center rounded-[2px] border border-[color-mix(in_oklab,var(--risk-security)_40%,transparent)] px-[7px] font-mono text-[11px] font-semibold tracking-[0.04em] text-risk-security uppercase">
              {normalizeSeverity(v.severity)}
            </span>
            <ChevronRight size={16} className="text-muted" />
          </a>
        ))}
      </div>
    </Dialog>
  )
}
