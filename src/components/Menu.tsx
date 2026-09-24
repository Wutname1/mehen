import { Check } from 'lucide-react'
import { useEffect, useLayoutEffect, useRef, useState, type ReactNode } from 'react'
import { createPortal } from 'react-dom'
import { cx } from './bits'

type Anchor = HTMLElement | { x: number; y: number }

/**
 * A popup menu placed next to `anchor` (an element, or a point for right-click
 * menus). Arrow keys, Home and End move between items; Escape, Tab or a click
 * outside closes it and returns focus to the element that opened it.
 */
export function Menu({
  anchor,
  label,
  onClose,
  placement = 'below-start',
  width,
  children,
}: {
  anchor: Anchor
  label: string
  onClose: () => void
  placement?: 'below-start' | 'below-end' | 'above-end'
  width?: number
  children: ReactNode
}) {
  const ref = useRef<HTMLDivElement>(null)
  const [pos, setPos] = useState<{ left: number; top: number } | null>(null)
  const returnTo = useRef<Element | null>(anchor instanceof HTMLElement ? anchor : document.activeElement)

  useLayoutEffect(() => {
    const menu = ref.current
    if (!menu) return
    const r = anchor instanceof HTMLElement ? anchor.getBoundingClientRect() : new DOMRect(anchor.x, anchor.y, 0, 0)
    const w = menu.offsetWidth
    const h = menu.offsetHeight
    let left = placement === 'below-start' ? r.left : r.right - w
    let top = placement === 'above-end' ? r.top - h - 6 : r.bottom + 4
    left = Math.max(8, Math.min(left, innerWidth - w - 8))
    top = Math.max(8, Math.min(top, innerHeight - h - 8))
    setPos({ left, top })
    menu.querySelector<HTMLElement>('[role^="menuitem"]:not(:disabled)')?.focus()
  }, [anchor, placement])

  useEffect(() => {
    const close = (restore: boolean) => {
      onClose()
      if (restore && returnTo.current instanceof HTMLElement && document.contains(returnTo.current)) returnTo.current.focus()
    }
    const onDown = (e: PointerEvent) => {
      if (ref.current?.contains(e.target as Node)) return
      if (anchor instanceof HTMLElement && anchor.contains(e.target as Node)) return
      close(false)
    }
    const onKey = (e: KeyboardEvent) => {
      const items = [...(ref.current?.querySelectorAll<HTMLElement>('[role^="menuitem"]:not(:disabled)') ?? [])]
      const i = items.indexOf(document.activeElement as HTMLElement)
      if (e.key === 'ArrowDown') items[(i + 1) % items.length]?.focus()
      else if (e.key === 'ArrowUp') items[(i - 1 + items.length) % items.length]?.focus()
      else if (e.key === 'Home') items[0]?.focus()
      else if (e.key === 'End') items.at(-1)?.focus()
      else if (e.key === 'Escape') {
        e.stopPropagation()
        close(true)
      }
      else if (e.key === 'Tab') close(false)
      else return
      if (e.key !== 'Tab') e.preventDefault()
    }
    document.addEventListener('pointerdown', onDown, true)
    document.addEventListener('keydown', onKey, true)
    return () => {
      document.removeEventListener('pointerdown', onDown, true)
      document.removeEventListener('keydown', onKey, true)
    }
  }, [anchor, onClose])

  return createPortal(
    <div
      ref={ref}
      role="menu"
      aria-label={label}
      style={{ left: pos?.left ?? -9999, top: pos?.top ?? -9999, width }}
      className="fixed z-[60] flex min-w-[220px] flex-col rounded-[4px] border border-line-strong bg-surface p-[5px] text-ink shadow-[var(--shadow)]"
    >
      {children}
    </div>,
    document.body,
  )
}

const itemClass =
  'flex min-h-8 items-center gap-2.5 rounded-[3px] px-2.5 text-left text-[12.5px] hover:bg-sunken focus-visible:bg-sunken focus-visible:outline-none focus-visible:shadow-[inset_0_0_0_2px_var(--focus)] disabled:opacity-45'

export function MenuItem({ icon, onSelect, danger, disabled, children }: { icon?: ReactNode; onSelect: () => void; danger?: boolean; disabled?: boolean; children: ReactNode }) {
  return (
    <button type="button" role="menuitem" disabled={disabled} onClick={onSelect} className={cx(itemClass, danger && 'text-risk-security')}>
      {icon && <span className={cx('flex size-4 shrink-0 items-center justify-center', danger ? 'text-risk-security' : 'text-muted')}>{icon}</span>}
      <span className="flex min-w-0 flex-1 items-center gap-2">{children}</span>
    </button>
  )
}

/** A checkbox or radio item; `detail` adds a second line of explanation. */
export function MenuCheck({
  checked,
  onSelect,
  radio,
  detail,
  icon,
  children,
}: {
  checked: boolean
  onSelect: () => void
  radio?: boolean
  detail?: ReactNode
  /** Shown between the check and the label. */
  icon?: ReactNode
  children: ReactNode
}) {
  return (
    <button
      type="button"
      role={radio ? 'menuitemradio' : 'menuitemcheckbox'}
      aria-checked={checked}
      onClick={onSelect}
      className={cx(itemClass, !!detail && 'items-start py-2')}
    >
      <span
        className={cx(
          'mt-px flex size-4 shrink-0 items-center justify-center border-[1.5px]',
          radio ? 'rounded-full' : 'rounded-[3px]',
          checked ? 'border-check bg-check text-paper' : 'border-line-strong',
        )}
      >
        {checked && <Check size={11} strokeWidth={3} />}
      </span>
      {icon && <span className="mt-px flex shrink-0 items-center">{icon}</span>}
      <span className="flex min-w-0 flex-col gap-0.5">
        <span className={cx(!!detail && 'font-semibold')}>{children}</span>
        {detail && <span className="text-[12px] leading-snug text-muted">{detail}</span>}
      </span>
    </button>
  )
}

export function MenuSeparator() {
  return <div role="separator" className="mx-0.5 my-1 h-px bg-line" />
}

export function MenuTitle({ title, detail }: { title: string; detail?: string }) {
  return (
    <div className="mb-1 border-b border-line px-2.5 pt-1.5 pb-2 text-[12px] text-muted">
      <b className="block text-[12.5px] text-ink">{title}</b>
      {detail}
    </div>
  )
}
