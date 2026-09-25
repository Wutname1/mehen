import { RotateCcw, Send, TriangleAlert } from 'lucide-react'
import { useState } from 'react'
import { FeedbackDialog } from './FeedbackDialog'

/** Shown instead of the app when it fails to draw, with a way back and a way to tell us. */
export function CrashScreen({ error, eventId }: { error: unknown; eventId: string }) {
  const [reporting, setReporting] = useState(false)
  const message = error instanceof Error ? error.message : String(error)
  const stack = error instanceof Error ? (error.stack ?? message) : message
  return (
    <div className="grid h-full place-items-center bg-paper p-8 text-ink">
      <div className="max-w-[520px]">
        <TriangleAlert size={28} className="mb-3 text-risk-review" />
        <h1 className="m-0 mb-2 font-display text-[22px] font-semibold">Mehen hit a problem</h1>
        <p className="m-0 mb-4 text-[13px] text-muted">Nothing on disk was changed by this. Reloading usually gets you going again.</p>
        <pre className="mb-4 max-h-40 overflow-auto rounded-[3px] bg-sunken px-3 py-2 font-mono text-[12px] whitespace-pre-wrap">{message}</pre>
        <div className="flex gap-2">
          <button type="button" onClick={() => location.reload()} className="inline-flex h-9 items-center gap-2 rounded-[3px] bg-accent px-3 text-[13px] font-semibold text-accent-ink hover:bg-accent-hover">
            <RotateCcw size={15} />
            Reload
          </button>
          <button type="button" onClick={() => setReporting(true)} className="inline-flex h-9 items-center gap-2 rounded-[3px] border border-line-strong px-3 text-[13px] font-semibold hover:border-muted">
            <Send size={15} />
            Send a report
          </button>
        </div>
      </div>
      {reporting && (
        <FeedbackDialog
          preset={{
            kind: 'bug',
            title: 'Report this problem',
            message: `Mehen stopped drawing: ${message}`,
            attachments: [{ filename: 'error.txt', data: stack, contentType: 'text/plain' }],
            tags: { failed_step: 'render' },
            context: eventId ? { error_event: eventId } : undefined,
          }}
          onClose={() => setReporting(false)}
        />
      )}
    </div>
  )
}
