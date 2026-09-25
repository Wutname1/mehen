import { relativePath } from '../derive'
import type { JobOutcome, Project, UpdatePlan } from '../types'
import type { Report } from './telemetry'

interface Run {
  build: boolean
  test: boolean
  commit: boolean
}

const onOff = (on: boolean) => (on ? 'on' : 'off')

function changeLines(plans: UpdatePlan[], base: string): string[] {
  return plans.flatMap((p) => [`  ${relativePath([base], p.projectId)} (${p.ecosystem})`, ...p.changes.map((c) => `    ${c.name} ${c.from} -> ${c.to}`)])
}

/** A repository whose update failed and was put back: everything needed to see why. */
export function updateFailureReport(job: { name: string; key: string; plans: UpdatePlan[] }, o: JobOutcome, run: Run): Report & { title: string } {
  const failed = o.steps.filter((s) => !s.ok)
  const ecosystems = [...new Set(job.plans.map((p) => p.ecosystem))]
  const text = [
    `Result: ${o.error ?? 'failed'}${o.rolledBack ? ' (files put back)' : ' (files NOT put back)'}`,
    `Options: build ${onOff(run.build)}, tests ${onOff(run.test)}, commit ${onOff(run.commit)}`,
    '',
    'Changes:',
    ...changeLines(job.plans, job.key),
    '',
    'Steps:',
    ...o.steps.map((s) => `  [${s.ok ? 'ok' : 'FAILED'}] ${s.kind}: ${s.label} (${Math.round(s.ms / 100) / 10}s)`),
    ...(o.conflicts.length ? ['', 'Conflicts found:', ...o.conflicts.map((c) => `  - ${c.summary}${c.blocking ? '' : ' (warning)'}`)] : []),
    ...((o.notes ?? []).length ? ['', 'Notes:', ...(o.notes ?? []).map((n) => `  - ${n}`)] : []),
    ...failed.flatMap((s) => ['', `--- Output of ${s.label} ---`, s.output]),
  ].join('\n')
  const packages = new Set(job.plans.flatMap((p) => p.changes.map((c) => c.name))).size
  return {
    kind: 'update-failed',
    title: `Report the failed update in ${job.name}`,
    message: `Update failed in ${job.name}: ${o.error ?? 'failed'}. ${packages} package${packages === 1 ? '' : 's'} (${ecosystems.join(', ')}).`,
    attachments: [{ filename: 'update.txt', data: text, contentType: 'text/plain' }],
    tags: { ecosystem: ecosystems.join(','), failed_step: failed[0]?.kind ?? (o.conflicts.length ? 'conflict' : 'none'), rolled_back: o.rolledBack ? 'yes' : 'no' },
    context: { packages, manifests: job.plans.length, steps: o.steps.length, conflicts: o.conflicts.length },
  }
}

/** Manifests Mehen could not work out an update for. */
export function planFailureReport(failed: { project: Project; error: string }[]): Report & { title: string } {
  const text = failed.map((f) => [`${f.project.manifest} (${f.project.ecosystem})`, `  ${f.error}`].join('\n')).join('\n\n')
  return {
    kind: 'update-failed',
    title: `Report ${failed.length === 1 ? 'a manifest' : `${failed.length} manifests`} Mehen could not update`,
    message: `Could not plan an update for ${failed.length} manifest${failed.length === 1 ? '' : 's'}: ${failed[0].error.split('\n')[0]}`,
    attachments: [{ filename: 'plan.txt', data: text, contentType: 'text/plain' }],
    tags: { ecosystem: [...new Set(failed.map((f) => f.project.ecosystem))].join(','), failed_step: 'plan' },
  }
}

/** The update could not run at all. */
export function batchErrorReport(error: string, plans: UpdatePlan[], run: Run): Report & { title: string } {
  const text = [`Error: ${error}`, `Options: build ${onOff(run.build)}, tests ${onOff(run.test)}, commit ${onOff(run.commit)}`, '', 'Changes:', ...changeLines(plans, '')].join('\n')
  return {
    kind: 'update-failed',
    title: 'Report the update that could not run',
    message: `The update could not run: ${error.split('\n')[0]}`,
    attachments: [{ filename: 'update.txt', data: text, contentType: 'text/plain' }],
    tags: { ecosystem: [...new Set(plans.map((p) => p.ecosystem))].join(','), failed_step: 'batch' },
  }
}
