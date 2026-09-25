import * as Sentry from '@sentry/react'
import { scrubDeep, scrubText } from './scrub'

/**
 * Two destinations, on purpose. Crashes and errors go to GlitchTip, and only
 * from release builds of someone who has not turned error reports off.
 * Feedback goes to Sentry, and only when someone presses Send: it is their
 * message, so the error-report switch does not apply to it.
 */
const ERRORS_DSN = 'https://a637017bd857436a9bb9840d939e3351@errors.nyxservices.com/12'
const FEEDBACK_DSN = 'https://8c96fd54b5896ff373736dfbc4b61650@o4511760230907904.ingest.us.sentry.io/4512148479082496'

const environment = import.meta.env.DEV ? 'development' : 'production'

/** No user, cookies, headers, bodies or query strings from the SDK itself. */
const PRIVATE: Sentry.BrowserOptions['dataCollection'] = { userInfo: false, cookies: false, httpHeaders: false, httpBodies: [], urlQueryParams: false }

/**
 * Error reports wait for the saved choice, which arrives with the settings a
 * moment after start. Until then, and whenever it is off, events and
 * breadcrumbs are dropped before they are recorded, so nothing is kept to
 * send later.
 */
let reporting = false

export function setErrorReporting(on: boolean) {
  reporting = on
}

/** Call once, before the app renders. Development builds report nothing. */
export function initErrorReporting() {
  if (import.meta.env.DEV) return
  Sentry.init({
    dsn: ERRORS_DSN,
    release: `mehen@${__APP_VERSION__}`,
    environment,
    // Paths, project names and package names travel in error messages. Keep
    // the extra identifiers off and scrub what does go.
    dataCollection: PRIVATE,
    maxBreadcrumbs: 100,
    beforeSend: (event) => (reporting ? scrubDeep(event) : null),
    beforeBreadcrumb: (breadcrumb) => (reporting ? scrubDeep(breadcrumb) : null),
  })
}

let feedback: { client: Sentry.BrowserClient; scope: Sentry.Scope } | null = null
/** Reports waiting to hear how their upload went, by event id. */
const waiting = new Map<string, (status: number | undefined) => void>()

/** A client of its own, so feedback never mixes with the error reports. */
function feedbackScope() {
  if (!feedback) {
    const client = new Sentry.BrowserClient({
      dsn: FEEDBACK_DSN,
      release: `mehen@${__APP_VERSION__}`,
      environment,
      dataCollection: PRIVATE,
      transport: Sentry.makeFetchTransport,
      stackParser: Sentry.defaultStackParser,
      integrations: [],
    })
    const scope = new Sentry.Scope()
    scope.setClient(client)
    client.init()
    client.on('afterSendEvent', (event, response) => {
      if (event.event_id) waiting.get(event.event_id)?.(response?.statusCode)
    })
    feedback = { client, scope }
  }
  return feedback
}

export type ReportKind = 'bug' | 'feedback' | 'update-failed'

export interface Attachment {
  filename: string
  data: string | Uint8Array
  contentType: string
}

export interface Report {
  kind: ReportKind
  message: string
  email?: string
  /** Files sent with it, like an update's output. Scrubbed here before sending. */
  attachments?: Attachment[]
  /** Searchable labels, like the dependency type of a failed update. */
  tags?: Record<string, string>
  /** Small structured facts shown on the report itself. */
  context?: Record<string, unknown>
  /** Send the app version and platform along. On unless turned off. */
  includeDetails?: boolean
}

export type SendResult = { ok: true; eventId: string } | { ok: false; message: string }

/** The app and machine a report came from. Never throws. */
export async function appDetails(): Promise<Record<string, string>> {
  let version = __APP_VERSION__
  try {
    const { getVersion } = await import('@tauri-apps/api/app')
    version = await getVersion()
  } catch {
    // A plain browser (the dev preview) has no app version to ask for.
  }
  return { version, platform: navigator.platform, userAgent: navigator.userAgent }
}

/**
 * Sends a report and waits until it has left, since people often close the
 * app straight after. Returns what happened instead of throwing, so the
 * caller can say so.
 */
export async function sendReport(report: Report): Promise<SendResult> {
  try {
    const { scope } = feedbackScope()
    const details = report.includeDetails === false ? null : await appDetails()
    // Attachments skip every hook on the way out: scrub them now.
    const attachments = report.attachments?.map((a) => (typeof a.data === 'string' ? { ...a, data: scrubText(a.data) } : a))
    const eventScope = scope.clone()
    // Our own id, so the upload's outcome can be matched to this report.
    const eventId = crypto.randomUUID().replace(/-/g, '')
    const uploaded = new Promise<number | undefined>((resolve) => {
      waiting.set(eventId, resolve)
      setTimeout(() => resolve(undefined), 15000)
    })
    if (details) eventScope.setContext('mehen', details)
    if (report.context) eventScope.setContext('report', scrubDeep(report.context))
    eventScope.setTag('report_kind', report.kind)
    for (const [k, v] of Object.entries(report.tags ?? {})) eventScope.setTag(k, scrubText(v))
    Sentry.captureFeedback(
      {
        name: 'Mehen user',
        email: report.email?.trim() || undefined,
        message: scrubText(report.message.trim() || '(no description given)'),
        tags: { report_kind: report.kind },
      },
      { attachments, event_id: eventId },
      eventScope,
    )
    const status = await uploaded
    waiting.delete(eventId)
    if (!status || status >= 300) {
      return { ok: false, message: status ? `The report service answered ${status}. Try again in a moment.` : 'The report could not be sent. Check your connection and try again.' }
    }
    return { ok: true, eventId }
  } catch (e) {
    return { ok: false, message: e instanceof Error ? e.message : String(e) }
  }
}

export { Sentry }
