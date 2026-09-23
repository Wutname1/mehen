import { ExternalLink } from 'lucide-react'
import { openLink } from '../api'
import { displayVersion, relativePath, type Usage } from '../derive'
import type { Vulnerability } from '../types'
import { EcoBadge, Empty, SeverityPill } from './bits'

export function VulnsView({ vulns, total, usages, roots }: { vulns: Vulnerability[]; total: number; usages: Map<string, Usage[]>; roots: string[] }) {
  if (vulns.length === 0) {
    return total > 0 ? (
      <Empty title="Nothing matches">None of the {total} vulnerabilities match the current search or filters.</Empty>
    ) : (
      <Empty title="No known vulnerabilities">Nothing in the scanned projects matches an advisory in the OSV database.</Empty>
    )
  }
  return (
    <div className="mx-auto flex max-w-[1100px] flex-col gap-3 px-6 py-5">
      {vulns.map((v) => {
        const hits = usages.get(v.id) ?? []
        const packages = [...new Map(hits.map((h) => [`${h.dep.ecosystem}:${h.dep.name}`, h.dep])).values()]
        const allApproximate = hits.length > 0 && hits.every((h) => h.dep.approximate)
        const cve = v.aliases.find((a) => a.startsWith('CVE-'))
        return (
          <article key={v.id} className="rounded-xl border border-line bg-panel px-5 py-4">
            <div className="flex items-start gap-3">
              <SeverityPill severity={v.severity} />
              <div className="min-w-0 flex-1">
                <h3 className="text-[14px] font-medium leading-snug">{v.summary || v.id}</h3>
                <div className="mt-1 flex flex-wrap items-center gap-x-3 gap-y-1 text-[11.5px] text-dim">
                  <button type="button" onClick={() => openLink(v.url)} className="inline-flex items-center gap-1 font-mono text-lapis hover:underline">
                    {v.id}
                    <ExternalLink size={11} />
                  </button>
                  {cve && <span className="font-mono">{cve}</span>}
                  {allApproximate && (
                    <span className="text-amber" title="The version was guessed from a range because nothing is installed or locked">
                      Possible match: version guessed from a range
                    </span>
                  )}
                </div>
              </div>
            </div>

            {packages.map((dep) => {
              const fix = v.fixed.find((f) => f.ecosystem === dep.ecosystem && f.name === dep.name)
              const where = hits.filter((h) => h.dep.name === dep.name && h.dep.ecosystem === dep.ecosystem)
              return (
                <div key={`${dep.ecosystem}:${dep.name}`} className="mt-3 rounded-lg border border-line bg-bg/50 px-3 py-2">
                  <div className="flex flex-wrap items-center gap-2">
                    <EcoBadge ecosystem={dep.ecosystem} />
                    <span className="font-medium">{dep.name}</span>
                    {fix && fix.versions.length > 0 && (
                      <span className="text-[11.5px] text-turq">
                        Fixed in <span className="font-mono">{[...new Set(fix.versions)].join(', ')}</span>
                      </span>
                    )}
                  </div>
                  <ul className="mt-1.5 grid gap-x-6 gap-y-0.5 text-[12px] sm:grid-cols-2">
                    {where.map(({ project, dep: d }) => (
                      <li key={project.id} className="flex min-w-0 items-baseline gap-2">
                        <span className="font-mono text-carnelian">
                          {d.approximate && '~'}
                          {displayVersion(d)}
                        </span>
                        <span className="truncate text-muted" title={project.dir}>
                          {project.name} <span className="text-dim">· {relativePath(roots, project.dir)}</span>
                        </span>
                      </li>
                    ))}
                  </ul>
                </div>
              )
            })}
          </article>
        )
      })}
    </div>
  )
}
