import { Info, Pin } from 'lucide-react'
import { reasonText, type HeldBack } from '../derive'
import type { Hold } from '../types'
import { Button, Dialog } from './Dialog'
import { RepoAvatar } from './RepoAvatar'

/**
 * Why newer versions are not offered: a package the project already has
 * would not work with them, or the user is keeping the package on a line.
 */
export function HeldBackDialog({
  groups,
  holds,
  icons,
  nameOf,
  scopeLabel,
  onRelease,
  onClose,
}: {
  groups: HeldBack[]
  holds: Hold[]
  icons: Record<string, string>
  nameOf: (folder: string) => string
  /** "every project" or a project name, for the description. */
  scopeLabel: string
  onRelease: (hold: Hold) => void
  onClose: () => void
}) {
  const single = groups.length === 1 ? groups[0] : null
  return (
    <Dialog
      title={single ? `Why not ${single.name} ${single.newest}?` : `${groups.length} newer version${groups.length === 1 ? '' : 's'} held back`}
      description={
        single
          ? `${single.newest} is out, but Mehen offers the newest version that fits each project.`
          : `Newer releases exist for these packages in ${scopeLabel}, but they would not fit. Mehen offers the newest version that does.`
      }
      icon={<Info size={22} />}
      size={single ? 'normal' : 'wide'}
      onClose={onClose}
      footer={<Button variant="primary" onClick={onClose}>Done</Button>}
    >
      {groups.length === 0 ? (
        <p className="py-4 text-muted">Nothing is held back.</p>
      ) : (
        <div className="grid gap-2.5">
          {groups.map((g) => {
            const kept = holds.filter((h) => h.ecosystem === g.ecosystem && h.name === g.name)
            return (
              <section key={g.key} className="rounded-[3px] border border-line bg-paper">
                {!single && (
                  <header className="flex items-center gap-2 border-b border-line px-3 py-2">
                    <b className="flex-1 truncate text-[13px]">{g.name}</b>
                    <span className="font-mono text-[12px] text-muted">newest {g.newest}</span>
                  </header>
                )}
                <ul className="m-0 grid list-none gap-1.5 p-3">
                  {g.entries.map((e) => (
                    <li key={e.project.id} className="grid grid-cols-[24px_1fr] items-start gap-2.5 text-[12.5px]">
                      <RepoAvatar name={nameOf(e.project.repo ?? e.project.dir)} icon={icons[(e.project.repo ?? e.project.dir).toLowerCase()]} size={24} />
                      <span className="min-w-0">
                        <b className="block truncate">{nameOf(e.project.repo ?? e.project.dir)}</b>
                        <span className="text-muted">
                          On <code className="font-mono text-[12px] text-ink">{e.current}</code>. {reasonText(e.reason)}.
                        </span>
                      </span>
                    </li>
                  ))}
                </ul>
                {kept.length > 0 && (
                  <footer className="flex flex-wrap items-center gap-2 border-t border-line px-3 py-2">
                    {kept.map((h) => (
                      <span key={h.id} className="inline-flex items-center gap-2 text-[12.5px] text-muted">
                        <Pin size={13} className="text-state" />
                        Kept on {h.line}.x {h.scope === '*' ? 'in every project' : `in ${nameOf(h.scope)}`}
                        <Button onClick={() => onRelease(h)}>Stop keeping</Button>
                      </span>
                    ))}
                  </footer>
                )}
              </section>
            )
          })}
        </div>
      )}
    </Dialog>
  )
}
