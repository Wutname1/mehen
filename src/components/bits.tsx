import type { ReactNode } from 'react'
import { STATUS_LABEL, normalizeSeverity } from '../derive'
import type { Status } from '../types'

const cx = (...parts: (string | false | null | undefined)[]) => parts.filter(Boolean).join(' ')
export { cx }

export function Logo({ size = 22 }: { size?: number }) {
  return (
    <svg width={size} height={size} viewBox="0 0 1024 1024" aria-hidden>
      <defs>
        <radialGradient id="mehen-sun" cx="46%" cy="42%" r="60%">
          <stop offset="0" stopColor="#ffd98a" />
          <stop offset="1" stopColor="#d9951f" />
        </radialGradient>
      </defs>
      <circle cx="512" cy="530" r="205" fill="url(#mehen-sun)" />
      <path d="M 676 250 A 322 322 0 1 1 398 227" fill="none" stroke="#45c2a6" strokeWidth="86" strokeLinecap="round" />
      <ellipse cx="700" cy="232" rx="84" ry="62" transform="rotate(-28 700 232)" fill="#45c2a6" />
    </svg>
  )
}

export const STATUS_STYLE: Record<Status, string> = {
  major: 'text-carnelian bg-carnelian-soft border-[#5c2a1d]',
  minor: 'text-amber bg-amber-soft border-[#54421b]',
  patch: 'text-sand bg-sand-soft border-[#423b29]',
  'up-to-date': 'text-turq bg-turq-soft border-[#1f4c42]',
  unpinned: 'text-lapis bg-lapis-soft border-[#2a3b5e]',
  unknown: 'text-muted bg-raised border-line-strong',
  pending: 'text-dim bg-raised border-line',
  local: 'text-dim bg-raised border-line',
}

export function StatusPill({ status }: { status: Status }) {
  return (
    <span className={cx('inline-flex h-5 items-center whitespace-nowrap rounded-full border px-2 text-[11px] font-medium', STATUS_STYLE[status])}>
      {STATUS_LABEL[status]}
    </span>
  )
}

const SEVERITY_STYLE: Record<string, string> = {
  CRITICAL: 'text-critical bg-critical-soft border-[#6b1f28]',
  HIGH: 'text-carnelian bg-carnelian-soft border-[#5c2a1d]',
  MODERATE: 'text-amber bg-amber-soft border-[#54421b]',
  LOW: 'text-sand bg-sand-soft border-[#423b29]',
  UNRATED: 'text-muted bg-raised border-line-strong',
}

export function SeverityPill({ severity }: { severity: string | null }) {
  const s = normalizeSeverity(severity)
  return (
    <span className={cx('inline-flex h-5 items-center rounded border px-1.5 text-[10.5px] font-semibold tracking-wider', SEVERITY_STYLE[s] ?? SEVERITY_STYLE.UNRATED)}>
      {s === 'UNRATED' ? 'UNRATED' : s}
    </span>
  )
}

export function VersionChip({ version, status, approximate, count }: { version: string; status: Status; approximate?: boolean; count?: number }) {
  return (
    <span
      className={cx('inline-flex h-[22px] items-center gap-1 rounded-md border px-1.5 font-mono text-[11.5px]', STATUS_STYLE[status])}
      title={approximate ? 'Guessed from the version range: nothing is installed or locked for this project' : STATUS_LABEL[status]}
    >
      {approximate && <span className="opacity-60">~</span>}
      {version}
      {count !== undefined && count > 1 && <span className="font-sans text-[10.5px] opacity-70">×{count}</span>}
    </span>
  )
}

export function IconButton({ title, onClick, children }: { title: string; onClick: () => void; children: ReactNode }) {
  return (
    <button
      type="button"
      title={title}
      aria-label={title}
      onClick={(e) => {
        e.stopPropagation()
        onClick()
      }}
      className="inline-flex size-7 items-center justify-center rounded-md text-dim transition-colors hover:bg-hover hover:text-ink"
    >
      {children}
    </button>
  )
}

export function Empty({ title, children }: { title: string; children?: ReactNode }) {
  return (
    <div className="flex flex-col items-center justify-center gap-2 py-24 text-center">
      <div className="text-[15px] font-medium text-ink">{title}</div>
      {children && <div className="max-w-md text-muted">{children}</div>}
    </div>
  )
}
