import { Bug, Check, ImagePlus, Lightbulb, Send, X } from 'lucide-react'
import { useEffect, useRef, useState, type ClipboardEvent, type DragEvent } from 'react'
import { appDetails, sendReport, type Attachment, type Report, type ReportKind } from '../lib/telemetry'
import { cx } from './bits'
import { TextInput, fieldClass } from './controls'
import { Button, Dialog } from './Dialog'
import { Checkbox } from './Queue'

const COPY: Record<'bug' | 'feedback', { label: string; prompt: string; placeholder: string }> = {
  bug: {
    label: 'Something went wrong',
    prompt: 'What went wrong?',
    placeholder: 'What were you doing, and what happened instead of what you expected?',
  },
  feedback: {
    label: 'Idea or feedback',
    prompt: "What's on your mind?",
    placeholder: 'An idea, something that felt awkward, or anything you wish Mehen did.',
  },
}

const MAX_SCREENSHOT_BYTES = 8 * 1024 * 1024

interface Screenshot {
  bytes: Uint8Array
  contentType: string
  preview: string
}

async function readImage(file: File): Promise<Screenshot> {
  if (file.size > MAX_SCREENSHOT_BYTES) throw new Error('That image is too large. Choose one under 8 MB.')
  const bytes = new Uint8Array(await file.arrayBuffer())
  return { bytes, contentType: file.type || 'image/png', preview: URL.createObjectURL(file) }
}

/**
 * Tell us something: a problem or an idea. What gets attached is shown before
 * sending, and app details can be left off. `preset` fills it from a failed
 * update, whose output rides along as a file.
 */
export function FeedbackDialog({ preset, onClose }: { preset?: Report & { title?: string }; onClose: () => void }) {
  const [kind, setKind] = useState<ReportKind>(preset?.kind ?? 'bug')
  const [message, setMessage] = useState('')
  const [email, setEmail] = useState('')
  const [includeDetails, setIncludeDetails] = useState(true)
  const [screenshot, setScreenshot] = useState<Screenshot | null>(null)
  const [details, setDetails] = useState<Record<string, string> | null>(null)
  const [state, setState] = useState<{ sending?: boolean; sent?: boolean; error?: string }>({})
  const fileInput = useRef<HTMLInputElement>(null)
  const fromUpdate = preset?.kind === 'update-failed'
  const copy = COPY[kind === 'feedback' ? 'feedback' : 'bug']

  useEffect(() => {
    appDetails().then(setDetails)
  }, [])
  useEffect(() => () => void (screenshot && URL.revokeObjectURL(screenshot.preview)), [screenshot])

  const attach = async (file: File | undefined) => {
    if (!file?.type.startsWith('image/')) return
    try {
      setScreenshot(await readImage(file))
    } catch (e) {
      setState({ error: e instanceof Error ? e.message : String(e) })
    }
  }
  const onPaste = (e: ClipboardEvent) => {
    const file = [...e.clipboardData.files].find((f) => f.type.startsWith('image/'))
    if (file) {
      e.preventDefault()
      attach(file)
    }
  }
  const onDrop = (e: DragEvent) => {
    const file = [...e.dataTransfer.files].find((f) => f.type.startsWith('image/'))
    if (file) {
      e.preventDefault()
      attach(file)
    }
  }

  const presetFiles = includeDetails ? (preset?.attachments ?? []) : []
  const needsText = !preset && !message.trim()
  const send = async () => {
    if (needsText || state.sending) return
    setState({ sending: true })
    const files: Attachment[] = [...presetFiles]
    if (screenshot) files.push({ filename: `screenshot.${screenshot.contentType.split('/')[1]?.replace('jpeg', 'jpg') ?? 'png'}`, data: screenshot.bytes, contentType: screenshot.contentType })
    const text = [message.trim(), preset?.message ?? ''].filter(Boolean).join('\n\n')
    const result = await sendReport({
      kind,
      message: text,
      email,
      attachments: files.length ? files : undefined,
      tags: { ...(includeDetails ? preset?.tags : {}), has_screenshot: screenshot ? 'yes' : 'no' },
      context: includeDetails ? preset?.context : undefined,
      includeDetails,
    })
    setState(result.ok ? { sent: true } : { error: result.message })
  }

  if (state.sent) {
    return (
      <Dialog title="Thanks, it's on its way" icon={<Check size={22} />} onClose={onClose} footer={<Button variant="primary" onClick={onClose}>Done</Button>}>
        <p className="m-0 text-[13px] text-muted">{kind === 'feedback' ? 'Every note gets read.' : 'Reports like this are how problems get fixed.'}{email.trim() ? ' You may hear back by email.' : ''}</p>
      </Dialog>
    )
  }

  return (
    <Dialog
      title={preset?.title ?? 'Send feedback'}
      description={fromUpdate ? 'The update output is attached. Add anything that helps, or just send it.' : preset ? 'The error is attached. Add anything that helps, or just send it.' : 'Goes straight to the people who make Mehen.'}
      icon={fromUpdate || kind === 'bug' ? <Bug size={22} /> : <Lightbulb size={22} />}
      busy={state.sending}
      onClose={onClose}
      footer={
        <>
          {state.error && <span className="mr-auto text-[12.5px] text-risk-security">{state.error}</span>}
          <Button onClick={onClose} disabled={state.sending}>
            Cancel
          </Button>
          <Button variant="primary" onClick={send} disabled={needsText || state.sending} title={needsText ? 'Write a few words first' : undefined}>
            <Send size={15} />
            {state.sending ? 'Sending…' : 'Send'}
          </Button>
        </>
      }
    >
      <div className="grid gap-3" onPaste={onPaste} onDrop={onDrop} onDragOver={(e) => e.preventDefault()}>
        {!preset && (
          <div className="inline-flex w-fit rounded-[3px] border border-line-strong p-0.5" role="radiogroup" aria-label="Kind of report">
            {(['bug', 'feedback'] as const).map((k) => (
              <button
                key={k}
                type="button"
                role="radio"
                aria-checked={kind === k}
                onClick={() => setKind(k)}
                className={cx('inline-flex h-7 items-center gap-1.5 rounded-[2px] px-2.5 text-[12.5px]', kind === k ? 'bg-row-selected font-semibold text-ink' : 'text-muted hover:text-ink')}
              >
                {k === 'bug' ? <Bug size={14} /> : <Lightbulb size={14} />}
                {COPY[k].label}
              </button>
            ))}
          </div>
        )}
        <label className="grid gap-1 text-[12px] text-muted">
          {preset ? 'Anything to add? (optional)' : copy.prompt}
          <textarea
            autoFocus
            value={message}
            onChange={(e) => setMessage(e.target.value)}
            onKeyDown={(e) => (e.ctrlKey || e.metaKey) && e.key === 'Enter' && send()}
            placeholder={fromUpdate ? 'What you expected, or anything unusual about this project.' : copy.placeholder}
            rows={5}
            className={cx(fieldClass, 'h-auto resize-y py-2 leading-normal')}
          />
        </label>
        <label className="grid gap-1 text-[12px] text-muted">
          Email, if you'd like a reply (optional)
          <TextInput type="email" value={email} onChange={(e) => setEmail(e.target.value)} placeholder="you@example.com" autoComplete="email" />
        </label>
        <div className="flex flex-wrap items-center gap-3">
          {screenshot ? (
            <span className="relative inline-block">
              <img src={screenshot.preview} alt="Screenshot to send" className="h-16 rounded-[3px] border border-line-strong" />
              <button type="button" onClick={() => setScreenshot(null)} aria-label="Remove screenshot" className="absolute -top-2 -right-2 grid size-5 place-items-center rounded-full bg-ink text-paper">
                <X size={12} />
              </button>
            </span>
          ) : (
            <Button onClick={() => fileInput.current?.click()} className="h-8">
              <ImagePlus size={15} />
              Add a screenshot
            </Button>
          )}
          <span className="text-[12px] text-muted">{screenshot ? 'Attached.' : 'Or paste one (Win+Shift+S, then Ctrl+V).'}</span>
          <input ref={fileInput} type="file" accept="image/*" hidden onChange={(e) => attach(e.target.files?.[0])} />
        </div>
        <label className="inline-flex cursor-pointer items-center gap-2 text-[12.5px]">
          <Checkbox checked={includeDetails} onChange={() => setIncludeDetails(!includeDetails)} label={fromUpdate ? 'Include the update output and app details' : 'Include app details'} />
          {fromUpdate ? 'Include the update output and app details' : 'Include app details'}
        </label>
        <details className="text-[12px] text-muted">
          <summary className="cursor-pointer hover:text-ink">See what's sent</summary>
          <pre className="mt-1.5 max-h-56 overflow-auto rounded-[3px] bg-sunken px-2.5 py-2 font-mono text-[11.5px] leading-relaxed whitespace-pre-wrap text-ink">
            {[
              includeDetails && details && `Mehen ${details.version} on ${details.platform}`,
              preset && `--- Message ---\n${preset.message}`,
              ...presetFiles.map((f) => `--- ${f.filename} ---\n${typeof f.data === 'string' ? f.data : `(${f.data.length} bytes)`}`),
              'Paths are cleaned of your user name, and tokens and email addresses are removed, before anything leaves.',
            ]
              .filter(Boolean)
              .join('\n\n')}
          </pre>
        </details>
      </div>
    </Dialog>
  )
}
