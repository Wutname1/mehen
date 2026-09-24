import { X } from 'lucide-react'
import { useEffect, useId, useRef, type ButtonHTMLAttributes, type ReactNode } from 'react'
import { createPortal } from 'react-dom'
import { cx } from './bits'

/**
 * A modal dialog. The app behind it is inert, Tab stays inside, Escape and
 * the backdrop close it (unless `busy`), and focus returns to where it was.
 */
export function Dialog({
  title,
  description,
  icon,
  tone,
  size = 'normal',
  busy,
  onClose,
  footer,
  children,
}: {
  title: ReactNode
  description?: ReactNode
  icon?: ReactNode
  tone?: 'danger'
  size?: 'normal' | 'wide'
  busy?: boolean
  onClose: () => void
  footer?: ReactNode
  children: ReactNode
}) {
  const ref = useRef<HTMLElement>(null)
  const titleId = useId()
  const descId = useId()
  const close = useRef(onClose)
  close.current = onClose
  const busyRef = useRef(busy)
  busyRef.current = busy

  useEffect(() => {
    const returnTo = document.activeElement as HTMLElement | null
    const root = document.getElementById('root')
    root?.setAttribute('inert', '')
    const dialog = ref.current
    const first = dialog?.querySelector<HTMLElement>('[autofocus], [data-autofocus]') ?? dialog?.querySelector<HTMLElement>('footer [data-primary]:not(:disabled)') ?? dialog?.querySelector<HTMLElement>('input, select, textarea, button:not([data-close])')
    first?.focus()

    const onKey = (e: KeyboardEvent) => {
      if (e.key === 'Escape' && !busyRef.current) {
        e.stopPropagation()
        close.current()
      }
      if (e.key !== 'Tab' || !dialog) return
      const nodes = [...dialog.querySelectorAll<HTMLElement>('button:not(:disabled), input:not(:disabled), select:not(:disabled), textarea:not(:disabled), a[href], [tabindex="0"]')].filter((n) => n.offsetParent !== null)
      if (!nodes.length) return
      if (e.shiftKey && document.activeElement === nodes[0]) {
        e.preventDefault()
        nodes.at(-1)!.focus()
      } else if (!e.shiftKey && document.activeElement === nodes.at(-1)) {
        e.preventDefault()
        nodes[0].focus()
      }
    }
    document.addEventListener('keydown', onKey)
    return () => {
      document.removeEventListener('keydown', onKey)
      root?.removeAttribute('inert')
      if (returnTo && document.contains(returnTo)) returnTo.focus()
    }
  }, [])

  return createPortal(
    <div className="fixed inset-0 z-50 grid place-items-center bg-[rgb(8_11_9/62%)] p-7" onMouseDown={(e) => e.target === e.currentTarget && !busy && onClose()}>
      <section
        ref={ref}
        role="dialog"
        aria-modal="true"
        aria-labelledby={titleId}
        aria-describedby={description ? descId : undefined}
        className={cx(
          'flex max-h-[calc(100vh-56px)] w-full flex-col rounded-[4px] border border-line-strong bg-surface text-ink shadow-[var(--shadow)]',
          size === 'wide' ? 'h-[min(620px,calc(100vh-56px))] max-w-[880px]' : 'max-w-[560px]',
        )}
      >
        <header className="flex items-start gap-3 px-5 pt-[18px] pb-3">
          {icon && <span className={cx('mt-[3px] shrink-0', tone === 'danger' ? 'text-risk-security' : 'text-accent-text')}>{icon}</span>}
          <div className="min-w-0 flex-1">
            <h2 id={titleId} className="m-0 font-display text-[20px] leading-tight font-semibold tracking-[-0.015em]">
              {title}
            </h2>
            {description && (
              <p id={descId} className="mt-1.5 mb-0 max-w-[62ch] text-[13px] leading-normal text-muted">
                {description}
              </p>
            )}
          </div>
          <button type="button" data-close onClick={onClose} disabled={busy} aria-label="Close" className="grid size-[30px] shrink-0 place-items-center rounded-[3px] text-muted hover:bg-sunken hover:text-ink disabled:opacity-40">
            <X size={16} />
          </button>
        </header>
        <div className="min-h-0 flex-1 overflow-y-auto px-5 pt-1 pb-4">{children}</div>
        {footer && <footer className="flex items-center justify-end gap-2 border-t border-line px-5 py-3">{footer}</footer>}
      </section>
    </div>,
    document.body,
  )
}

export function Button({
  variant = 'default',
  className,
  ...props
}: ButtonHTMLAttributes<HTMLButtonElement> & { variant?: 'default' | 'primary' | 'ghost' | 'danger' }) {
  return (
    <button
      type="button"
      data-primary={variant === 'primary' ? '' : undefined}
      {...props}
      className={cx(
        'inline-flex h-8 items-center justify-center gap-1.5 rounded-[3px] border px-3 text-[12.5px] font-semibold whitespace-nowrap disabled:opacity-45',
        variant === 'primary' && 'border-accent bg-accent text-accent-ink hover:border-accent-hover hover:bg-accent-hover',
        variant === 'default' && 'border-line-strong bg-surface text-ink hover:border-muted',
        variant === 'ghost' && 'border-transparent bg-transparent text-muted hover:bg-sunken hover:text-ink',
        variant === 'danger' && 'border-danger bg-danger text-white hover:bg-danger-hover',
        className,
      )}
    />
  )
}
