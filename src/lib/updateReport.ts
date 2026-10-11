import { folderName, relativePath } from '../derive'
import type { JobOutcome, Project, StepResult, UpdatePlan } from '../types'
import type { Report } from './telemetry'

interface Run {
  build: boolean
  test: boolean
  commit: boolean
}

/** How much of a failed command's output travels with a report. */
const OUTPUT_LINES = 100

const onOff = (on: boolean) => (on ? 'on' : 'off')

/** The last lines of a command's output. Scrubbed on the way out (see `sendReport`). */
export function outputTail(output: string, lines = OUTPUT_LINES): string {
  const all = output.replace(/\r\n/g, '\n').replace(/\s+$/, '').split('\n')
  return all.length > lines ? [`... (${all.length - lines} earlier lines left out)`, ...all.slice(-lines)].join('\n') : all.join('\n')
}

function changeLines(plans: UpdatePlan[], base: string): string[] {
  return plans.flatMap((p) => [`  ${relativePath([base], p.projectId)} (${p.ecosystem})`, ...p.changes.map((c) => `    ${c.name} ${c.from} -> ${c.to}`)])
}

/** Every package an update moved, with where it is written, as structured rows. */
export function packageRows(plans: UpdatePlan[], base: string) {
  return plans.flatMap((p) => p.changes.map((c) => ({ ecosystem: p.ecosystem, manifest: relativePath([base], p.projectId), name: c.name, from: c.from, to: c.to })))
}

/** The output of each failed step, keyed so each shows as its own field on the report. */
function failedOutputs(steps: StepResult[]): Record<string, string> {
  const out: Record<string, string> = {}
  steps.forEach((s, i) => {
    if (!s.ok) out[`output ${i + 1}: ${s.label}`] = outputTail(s.output)
  })
  return out
}

/** A repository whose update failed and was put back: everything needed to see why. */
export function updateFailureReport(job: { name: string; key: string; plans: UpdatePlan[] }, o: JobOutcome, run: Run): Report & { title: string } {
  const failed = o.steps.filter((s) => !s.ok)
  const ecosystems = [...new Set(job.plans.map((p) => p.ecosystem))]
  const text = [
    `Repository: ${job.name}`,
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
    ...failed.flatMap((s) => ['', `--- Output of ${s.label} (last ${OUTPUT_LINES} lines) ---`, outputTail(s.output)]),
  ].join('\n')
  const rows = packageRows(job.plans, job.key)
  const packages = new Set(rows.map((r) => r.name)).size
  return {
    kind: 'update-failed',
    title: `Report the failed update in ${job.name}`,
    message: `Update failed in ${job.name}: ${o.error ?? 'failed'}. ${packages} package${packages === 1 ? '' : 's'} (${ecosystems.join(', ')}).`,
    attachments: [{ filename: 'update.txt', data: text, contentType: 'text/plain' }],
    tags: {
      ecosystem: ecosystems.join(','),
      failed_step: failed[0]?.kind ?? (o.conflicts.length ? 'conflict' : 'none'),
      rolled_back: o.rolledBack ? 'yes' : 'no',
      failed_before: o.failedBefore == null ? 'unknown' : o.failedBefore ? 'yes' : 'no',
    },
    context: {
      repository: job.name,
      error: o.error,
      packages,
      manifests: job.plans.length,
      steps: o.steps.length,
      failed_steps: failed.map((s) => `${s.kind}: ${s.label}`),
      conflicts: o.conflicts.map((c) => `${c.summary}${c.blocking ? '' : ' (warning)'}`),
      notes: o.notes ?? [],
      options: `build ${onOff(run.build)}, tests ${onOff(run.test)}, commit ${onOff(run.commit)}`,
    },
    extra: { packages: rows, ...failedOutputs(o.steps) },
  }
}

/** Manifests Mehen could not work out an update for. */
export function planFailureReport(failed: { project: Project; error: string }[], roots: string[] = []): Report & { title: string } {
  const repoOf = (p: Project) => folderName(p.repo ?? p.dir)
  const repos = [...new Set(failed.map((f) => repoOf(f.project)))]
  const text = failed.map((f) => [`${repoOf(f.project)}: ${relativePath(roots, f.project.manifest)} (${f.project.ecosystem})`, `  ${f.error}`].join('\n')).join('\n\n')
  return {
    kind: 'update-failed',
    title: `Report ${failed.length === 1 ? 'a manifest' : `${failed.length} manifests`} Mehen could not update`,
    message: `Could not plan an update for ${failed.length} manifest${failed.length === 1 ? '' : 's'} in ${repos.join(', ')}: ${failed[0].error.split('\n')[0]}`,
    attachments: [{ filename: 'plan.txt', data: text, contentType: 'text/plain' }],
    tags: { ecosystem: [...new Set(failed.map((f) => f.project.ecosystem))].join(','), failed_step: 'plan' },
    context: { repositories: repos, manifests: failed.length },
    extra: { failures: failed.map((f) => ({ repository: repoOf(f.project), manifest: relativePath(roots, f.project.manifest), ecosystem: f.project.ecosystem, error: f.error })) },
  }
}

/** The update could not run at all. */
export function batchErrorReport(error: string, plans: UpdatePlan[], run: Run): Report & { title: string } {
  const repos = [...new Set(plans.map((p) => folderName(p.repo ?? p.projectId.replace(/[\\/][^\\/]*$/, ''))))]
  const text = [`Repositories: ${repos.join(', ')}`, `Error: ${error}`, `Options: build ${onOff(run.build)}, tests ${onOff(run.test)}, commit ${onOff(run.commit)}`, '', 'Changes:', ...changeLines(plans, '')].join('\n')
  return {
    kind: 'update-failed',
    title: 'Report the update that could not run',
    message: `The update could not run in ${repos.join(', ')}: ${error.split('\n')[0]}`,
    attachments: [{ filename: 'update.txt', data: text, contentType: 'text/plain' }],
    tags: { ecosystem: [...new Set(plans.map((p) => p.ecosystem))].join(','), failed_step: 'batch' },
    context: { repositories: repos, error },
    extra: { packages: plans.flatMap((p) => packageRows([p], p.repo ?? '')) },
  }
}
