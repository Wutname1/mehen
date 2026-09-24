import { cx } from './bits'

/** Two letters for a project without a logo: "GitWyrm" -> GW, "code/opencode" -> OP. */
export function monogram(name: string): string {
  const base = name.replace(/^.*\//, '')
  const words = base.replace(/[._-]+/g, ' ').split(/\s+|(?=[A-Z][a-z])/).filter(Boolean)
  return (words.length > 1 ? words[0][0] + words[1][0] : base.slice(0, 2)).toUpperCase()
}

/** The project's logo when one was found, otherwise its initials. */
export function RepoAvatar({ name, icon, size = 28, tone = 'paper' }: { name: string; icon?: string; size?: number; tone?: 'paper' | 'rail' }) {
  const box = { width: size, height: size }
  if (icon) return <img src={icon} alt="" style={box} className="shrink-0 rounded-[3px] object-contain" draggable={false} />
  return (
    <span
      aria-hidden
      style={{ ...box, fontSize: size <= 24 ? 10.5 : 11 }}
      className={cx(
        'grid shrink-0 place-items-center rounded-[3px] border font-mono font-bold',
        tone === 'rail' ? 'border-rail-border bg-rail-raised text-rail-ink-2' : 'border-line bg-paper-2 text-muted',
      )}
    >
      {monogram(name)}
    </span>
  )
}
