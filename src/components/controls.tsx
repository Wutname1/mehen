import type { ComponentProps, ReactNode } from 'react'
import { cx } from './bits'

export function Switch({ checked, onChange, label, disabled }: { checked: boolean; onChange: (next: boolean) => void; label: string; disabled?: boolean }) {
  return (
    <button
      type="button"
      role="switch"
      aria-checked={checked}
      aria-label={label}
      disabled={disabled}
      onClick={() => onChange(!checked)}
      className={cx(
        'relative h-[22px] w-[38px] shrink-0 rounded-full transition-colors disabled:opacity-45',
        checked ? 'bg-check' : 'bg-line-strong',
        "after:absolute after:top-[3px] after:left-[3px] after:size-4 after:rounded-full after:bg-surface after:transition-transform after:content-['']",
        checked && 'after:translate-x-4',
      )}
    />
  )
}

/** A labelled setting: title and help on the left, the control on the right. */
export function SettingRow({ title, help, children }: { title: ReactNode; help?: ReactNode; children?: ReactNode }) {
  return (
    <div className="flex min-h-14 items-center gap-4 border-b border-line py-2">
      <span className="flex min-w-0 flex-1 flex-col gap-0.5">
        <b className="text-[13px] font-semibold">{title}</b>
        {help && <small className="text-[12px] leading-snug text-muted">{help}</small>}
      </span>
      {children}
    </div>
  )
}

export function SectionTitle({ children }: { children: ReactNode }) {
  return <h4 className="mt-4 mb-1 font-mono text-[11px] font-normal tracking-[0.05em] text-muted uppercase">{children}</h4>
}

export const fieldClass =
  'h-[34px] rounded-[3px] border border-line-strong bg-paper px-2.5 text-[13px] text-ink placeholder:text-faint focus:border-focus focus:shadow-[0_0_0_1px_var(--focus)] focus:outline-none disabled:opacity-55'

export function TextInput({ className, mono, ...props }: ComponentProps<'input'> & { mono?: boolean }) {
  return <input {...props} className={cx(fieldClass, mono && 'font-mono text-[12.5px]', className)} />
}

export function Select({ className, ...props }: ComponentProps<'select'>) {
  return <select {...props} className={cx(fieldClass, 'pr-7', className)} />
}
