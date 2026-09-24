'use strict'

/* ---------- Data ---------- */

const projects = [
  { id: 'hearth', name: 'HearthShelf Web', path: 'C:\\code\\HearthShelf-WebApp', framework: 'React 19', stacks: ['ts', 'react', 'actions'], logo: 'assets/hearthshelf.png', branch: 'main', root: 'code', deps: 142 },
  { id: 'gitwyrm', name: 'GitWyrm', path: 'C:\\code\\GitWyrm', framework: 'Tauri 2', stacks: ['ts', 'tauri', 'react', 'actions'], logo: 'assets/gitwyrm.png', branch: 'develop', root: 'code', deps: 168, tauri: true },
  { id: 'mehen', name: 'Mehen', path: 'C:\\code\\Mehen', framework: 'Tauri 2', stacks: ['ts', 'tauri', 'react', 'actions'], logo: 'logo.svg', branch: 'main', root: 'code', deps: 121, tauri: true },
  { id: 'spartan', name: 'Spartan UI', path: 'C:\\code\\SpartanUI-LibAT-Website', framework: 'Astro', stacks: ['ts', 'astro', 'actions'], logo: 'assets/spartan.png', branch: 'main', root: 'code', deps: 96 },
  { id: 'libs', name: 'Libs Addon Tools', path: 'C:\\code\\LibsAddonTools', framework: 'React, Rust', stacks: ['ts', 'rust', 'react', 'actions'], branch: 'main', root: 'code', deps: 154 },
  { id: 'billing', name: 'Billing Gateway', path: 'C:\\code\\work\\Billing.Gateway', framework: '.NET 8', stacks: ['csharp', 'dotnet8', 'nuget', 'actions'], branch: 'release/4.2', root: 'work', deps: 188, solution: 'Billing.slnx' },
  { id: 'claims', name: 'Claims Desktop', path: 'C:\\code\\work\\Claims.Desktop', framework: '.NET Framework 4.8', stacks: ['csharp', 'netfx48', 'nuget', 'actions'], branch: 'main', root: 'work', deps: 203, solution: 'Claims.sln', uncommitted: 2, failsTests: true },
]

const dependencies = [
  { name: 'react', ecosystem: 'npm', current: '19.1.0', target: '19.3.0', projects: ['hearth', 'spartan', 'libs'], risk: 'minor' },
  { name: '@tauri-apps/api', ecosystem: 'npm', current: '2.8.0', target: '2.11.1', projects: ['gitwyrm', 'mehen'], risk: 'minor' },
  { name: 'serde', ecosystem: 'cargo', current: '1.0.219', target: '1.0.228', projects: ['gitwyrm', 'mehen', 'libs'], risk: 'patch' },
  { name: 'tokio', ecosystem: 'cargo', current: '1.44.1', target: '1.47.1', projects: ['gitwyrm', 'mehen', 'libs'], risk: 'review' },
  {
    name: 'Newtonsoft.Json', ecosystem: 'nuget', current: '12.0.3', target: '13.0.4', projects: ['billing', 'claims'], risk: 'security',
    advisories: [
      { id: 'CVE-2024-21907', severity: 'High', title: 'Denial of service from deeply nested input', url: 'https://nvd.nist.gov/vuln/detail/CVE-2024-21907' },
      { id: 'GHSA-5crp-9r3c-p9vr', severity: 'High', title: 'Stack overflow when deserializing crafted data', url: 'https://github.com/advisories/GHSA-5crp-9r3c-p9vr' },
    ],
  },
  { name: 'Microsoft.Extensions.Hosting', ecosystem: 'nuget', current: '8.0.0', target: '9.0.9', projects: ['billing'], risk: 'major' },
  {
    name: 'lodash', ecosystem: 'npm', current: '4.17.20', target: '4.17.21', projects: ['hearth', 'spartan', 'libs'], risk: 'security',
    advisories: [
      { id: 'CVE-2021-23337', severity: 'High', title: 'Command injection through template variables', url: 'https://nvd.nist.gov/vuln/detail/CVE-2021-23337' },
      { id: 'CVE-2020-28500', severity: 'High', title: 'Regular expression denial of service', url: 'https://nvd.nist.gov/vuln/detail/CVE-2020-28500' },
    ],
  },
  { name: 'actions/checkout', ecosystem: 'actions', current: 'v4.1.7', target: 'v4.2.2', projects: ['hearth', 'gitwyrm', 'mehen', 'spartan', 'libs', 'billing', 'claims'], risk: 'patch' },
  { name: 'actions/setup-node', ecosystem: 'actions', current: 'v4.0.3', target: 'v4.4.0', projects: ['hearth', 'spartan', 'libs'], risk: 'minor' },
]

const STACKS = {
  ts: ['TS', 'TypeScript'], react: ['Re', 'React'], tauri: ['Ta', 'Tauri'], rust: ['Rs', 'Rust'], actions: ['GA', 'GitHub Actions'],
  astro: ['As', 'Astro'], csharp: ['C#', 'C#'], dotnet8: ['N8', '.NET 8'], netfx48: ['NF', '.NET Framework 4.8'], nuget: ['Nu', 'NuGet'],
}
const ECOSYSTEM_LABEL = { npm: 'npm', cargo: 'Cargo', nuget: 'NuGet', actions: 'GitHub Actions' }
const PROJECT_TYPE_LABEL = { web: 'React / Web', rust: 'Rust / Tauri', dotnet: '.NET' }
const RISK_ORDER = ['security', 'major', 'review', 'minor', 'patch']
const RISK_HINT = {
  security: 'Fixes a known vulnerability',
  major: 'Major version: expect breaking changes',
  review: 'Minor version with notes worth reading first',
  minor: 'New features, no breaking changes expected',
  patch: 'Bug fixes only',
}
const RISK_FILTERS = { any: 'Any risk', attention: 'Needs attention', security: 'Security only' }

/* ---------- State ---------- */

const state = {
  selected: null,
  staged: new Set(['Newtonsoft.Json|billing', 'Newtonsoft.Json|claims']),
  updated: new Set(),
  failed: new Set(),
  excluded: new Set(),
  exclusions: [{ id: 'vendor-sdk', kind: 'folder', path: 'C:\\code\\vendor\\partner-sdk' }],
  roots: [{ id: 'code', path: 'C:\\code' }, { id: 'work', path: 'C:\\code\\work' }],
  ecosystems: new Set(),
  projectTypes: new Set(),
  risk: 'any',
  riskFirst: false,
  scanning: false,
  run: (() => { try { return { checks: true, commit: false, ...JSON.parse(localStorage.getItem('mehen-mockup-run') ?? '{}') } } catch { return { checks: true, commit: false } } })(),
}

/* ---------- Helpers ---------- */

const $ = (s, root = document) => root.querySelector(s)
const $$ = (s, root = document) => [...root.querySelectorAll(s)]
const icon = (id) => `<svg aria-hidden="true"><use href="#${id}"/></svg>`
const esc = (v) => String(v).replace(/[&<>"']/g, (c) => ({ '&': '&amp;', '<': '&lt;', '>': '&gt;', '"': '&quot;', "'": '&#39;' })[c])
const plural = (n, word, many = `${word}s`) => `${n} ${n === 1 ? word : many}`
const key = (dep, pid) => `${dep}|${pid}`
const project = (id) => projects.find((p) => p.id === id)
const dependency = (name) => dependencies.find((d) => d.name === name)
const isWatched = (pid) => state.roots.some((r) => r.id === project(pid).root)

function projectTypes(p) {
  const types = []
  if (p.stacks.some((s) => ['react', 'astro', 'ts'].includes(s))) types.push('web')
  if (p.stacks.some((s) => ['rust', 'tauri'].includes(s))) types.push('rust')
  if (p.stacks.some((s) => ['dotnet8', 'netfx48'].includes(s))) types.push('dotnet')
  return types
}

/** Projects that still use the old version of a dependency. */
function remaining(d) {
  return d.projects.filter((pid) => !state.updated.has(key(d.name, pid)) && !state.excluded.has(pid) && isWatched(pid))
}

/** Projects a row acts on, given the selected project and project type filter. */
function scopeIds(d) {
  const ids = remaining(d)
  if (state.selected) return ids.includes(state.selected) ? [state.selected] : []
  if (!state.projectTypes.size) return ids
  return ids.filter((pid) => projectTypes(project(pid)).some((t) => state.projectTypes.has(t)))
}

function projectHealth(p) {
  const open = dependencies.filter((d) => remaining(d).includes(p.id))
  return { updates: open.length, vulnerable: open.filter((d) => d.advisories).length }
}

function stagedPairs() {
  return [...state.staged].map((k) => {
    const [name, pid] = k.split('|')
    return { dep: dependency(name), project: project(pid), key: k }
  })
}

function manifestFiles(p, d) {
  if (d.ecosystem === 'npm') return ['package.json', 'package-lock.json']
  if (d.ecosystem === 'cargo') return p.tauri ? ['src-tauri/Cargo.toml', 'src-tauri/Cargo.lock'] : ['Cargo.toml', 'Cargo.lock']
  if (d.ecosystem === 'nuget') return p.id === 'billing' ? ['Directory.Packages.props'] : ['src/Claims.Desktop.csproj', 'tests/Claims.Tests.csproj']
  return ['.github/workflows/ci.yml']
}

function checksFor(p) {
  if (p.solution) return [`dotnet build ${p.solution}`, `dotnet test ${p.solution}`]
  if (p.stacks.includes('rust') && !p.stacks.includes('react')) return ['cargo build --locked', 'cargo test --locked']
  return ['npm run build', 'npm test']
}

function updateCommand(d) {
  if (d.ecosystem === 'npm') return `npm install ${d.name}@${d.target}`
  if (d.ecosystem === 'cargo') return `cargo update -p ${d.name} --precise ${d.target}`
  if (d.ecosystem === 'nuget') return `dotnet add package ${d.name} --version ${d.target}`
  return `set ${d.name}@${d.target} in workflow files`
}

function saveRun() {
  try { localStorage.setItem('mehen-mockup-run', JSON.stringify(state.run)) } catch { /* private window */ }
}

const runLabel = () => (state.run.checks ? 'Update &amp; run checks' : 'Update &amp; install only')

function runSummary() {
  const { checks, commit } = state.run
  if (checks && commit) return 'Then builds, tests, and commits each project.'
  if (checks) return 'Then builds and tests. You commit afterwards.'
  if (commit) return 'Installs only, then commits each project. No build or test.'
  return 'Installs only. No build or test, nothing committed.'
}

function installFor(p) {
  if (p.solution) return `dotnet restore ${p.solution}`
  if (p.stacks.includes('rust') && !p.stacks.includes('react')) return 'cargo fetch'
  return 'npm install'
}

const commitMessage = (list) => `Updated ${list.length} ${list.length === 1 ? 'Dependency' : 'Dependencies'}`
const commitBody = (list) => list.map((x) => `${x.dep.name} ${x.dep.current} to ${x.dep.target}`)
const fakeHash = (s) => [...s].reduce((acc, ch) => (acc * 31 + ch.charCodeAt(0)) >>> 0, 7).toString(16).padStart(7, '0').slice(0, 7)

function avatar(p, cls = 'avatar') {
  if (p.logo) return `<img class="${cls}" src="${p.logo}" alt="">`
  const letters = p.name.split(/\s+/).map((w) => w[0]).join('').slice(0, 2)
  return `<span class="${cls} monogram" aria-hidden="true">${letters}</span>`
}

function chips(p) {
  return `<span class="chips" aria-hidden="true">${p.stacks.map((s) => `<i class="chip stack-${s}" data-tooltip="${STACKS[s][1]}">${STACKS[s][0]}</i>`).join('')}</span>`
}

const stackNames = (p) => p.stacks.map((s) => STACKS[s][1]).join(', ')

/* ---------- Rendering ---------- */

function visibleRows() {
  const q = $('#search').value.trim().toLowerCase()
  let rows = dependencies.filter((d) => {
    if (!scopeIds(d).length) return false
    if (state.ecosystems.size && !state.ecosystems.has(d.ecosystem)) return false
    if (state.risk === 'attention' && !['security', 'major', 'review'].includes(d.risk)) return false
    if (state.risk === 'security' && d.risk !== 'security') return false
    if (!q) return true
    const inPackage = `${d.name} ${d.ecosystem} ${d.risk}`.toLowerCase().includes(q)
    const inProject = scopeIds(d).some((pid) => { const p = project(pid); return `${p.name} ${p.path}`.toLowerCase().includes(q) })
    return inPackage || inProject
  })
  if (state.riskFirst) rows = [...rows].sort((a, b) => RISK_ORDER.indexOf(a.risk) - RISK_ORDER.indexOf(b.risk))
  return rows
}

function renderRail() {
  const q = $('#search').value.trim().toLowerCase()
  const matches = (p) => !q || `${p.name} ${p.path} ${stackNames(p)}`.toLowerCase().includes(q) || dependencies.some((d) => d.projects.includes(p.id) && d.name.toLowerCase().includes(q))
  $('#rail-list').innerHTML = state.roots.map((root) => {
    const list = projects.filter((p) => p.root === root.id)
    const items = list.filter(matches).map((p) => {
      const h = projectHealth(p)
      const excluded = state.excluded.has(p.id)
      const label = `${p.name}, ${stackNames(p)}, ${excluded ? 'excluded' : h.updates ? plural(h.updates, 'update') : 'up to date'}${h.vulnerable && !excluded ? `, ${h.vulnerable} vulnerable` : ''}`
      const count = excluded
        ? `<span class="rail-count">${icon('i-eyeoff')}</span>`
        : h.vulnerable
          ? `<span class="rail-count vuln">${icon('i-shield')}${h.updates}</span>`
          : h.updates
            ? `<span class="rail-count">${h.updates}</span>`
            : `<span class="rail-count clear">${icon('i-check')}</span>`
      return `<button class="project-item ${excluded ? 'excluded' : ''}" type="button" data-project="${p.id}" aria-current="${state.selected === p.id}" aria-label="${esc(label)}">${avatar(p)}<span class="who"><b>${p.name}</b>${chips(p)}</span>${count}</button>`
    }).join('')
    return `<div class="root-label">${icon('i-folder')}<span title="${esc(root.path)}">${esc(root.path)}</span><span aria-label="${plural(list.length, 'project')}" style="flex:none">${list.length}</span></div>${items}`
  }).join('')

  const active = projects.filter((p) => !state.excluded.has(p.id) && isWatched(p.id))
  const vulnerable = active.filter((p) => projectHealth(p).vulnerable).length
  $('#all-summary').innerHTML = `${plural(active.length, 'project')}${vulnerable ? ` · <span class="vuln">${vulnerable} vulnerable</span>` : ''}`
  $('#all-projects').classList.toggle('active', !state.selected)
  $('#all-projects').setAttribute('aria-current', String(!state.selected))
  $('#exclusion-count').textContent = state.exclusions.length || ''
  if (!state.scanning) $('#scan-detail').textContent = `${plural(active.length, 'project')} · ${plural(state.roots.length, 'folder')}`
}

function renderCommandBar() {
  const p = project(state.selected)
  const chip = $('#scope-chip')
  chip.hidden = !p
  if (p) chip.innerHTML = `<span><small>Showing</small><b>${p.name}</b></span><button type="button" data-action="all-projects" aria-label="Show all projects" data-tooltip="Back to all projects">${icon('i-x')}</button>`

  const typeValue = state.projectTypes.size ? [...state.projectTypes].map((t) => PROJECT_TYPE_LABEL[t]).join(', ') : 'All'
  const ecoValue = state.ecosystems.size ? [...state.ecosystems].map((e) => ECOSYSTEM_LABEL[e]).join(', ') : 'All'
  const trigger = (id, label, value, active) =>
    `<button class="filter-trigger ${active ? 'active' : ''}" type="button" data-filter="${id}" aria-haspopup="menu" aria-expanded="false"><span>${label}</span><b>${value}</b>${icon('i-chevron-down')}</button>`
  $('#filters').innerHTML =
    (p ? '' : trigger('type', 'Project type', typeValue, state.projectTypes.size > 0)) +
    trigger('ecosystem', 'Dependency type', ecoValue, state.ecosystems.size > 0) +
    trigger('risk', 'Risk', RISK_FILTERS[state.risk], state.risk !== 'any')
}

function renderSecurity() {
  const vulnerable = dependencies.filter((d) => d.advisories && scopeIds(d).length && (!state.ecosystems.size || state.ecosystems.has(d.ecosystem)))
  const banner = $('#security-banner')
  banner.hidden = !vulnerable.length
  if (!vulnerable.length) return
  const affected = new Set(vulnerable.flatMap(scopeIds))
  const allStaged = vulnerable.every((d) => scopeIds(d).every((pid) => state.staged.has(key(d.name, pid))))
  const only = state.risk === 'security'
  banner.innerHTML = `${icon('i-shield')}<span><b>${plural(vulnerable.length, 'package has', 'packages have')} known vulnerabilities</b><small>Used by ${plural(affected.size, 'project')}. Updating to the target version fixes ${vulnerable.length === 1 ? 'it' : 'all of them'}.</small></span>
    <button class="button ghost" type="button" data-action="toggle-security">${only ? 'Show everything' : 'Show only these'}</button>
    <button class="button fix" type="button" data-action="prepare-security" ${allStaged ? 'disabled' : ''}>${allStaged ? `${icon('i-check')}Fixes selected` : 'Select fixes'}</button>`
}

function renderTable() {
  const rows = visibleRows()
  const p = project(state.selected)
  $('#usage-head').textContent = p ? 'File' : 'Used by'
  $('#dep-table').classList.toggle('project-view', !!p)
  $('#dep-rows').innerHTML = rows.map((d, i) => {
    const ids = scopeIds(d)
    const stagedCount = ids.filter((pid) => state.staged.has(key(d.name, pid))).length
    const all = stagedCount === ids.length
    const usage = p ? `<code title="${manifestFiles(p, d)[0]}">${manifestFiles(p, d)[0]}</code>` : plural(ids.length, 'project')
    const advisory = d.advisories
      ? `<button class="advisory-link" type="button" data-advisory="${d.name}">${icon('i-shield')}${plural(d.advisories.length, 'advisory', 'advisories')}</button>`
      : ''
    return `<div class="dep-row ${d.advisories ? 'vulnerable' : ''} ${stagedCount ? 'selected' : ''}" role="row" aria-rowindex="${i + 2}">
      <span class="cell-check" role="cell"><input class="check" type="checkbox" data-stage="${d.name}" ${all ? 'checked' : ''} data-partial="${stagedCount > 0 && !all}" aria-label="Select ${d.name} ${d.current} to ${d.target}${p ? '' : ` in ${plural(ids.length, 'project')}`}"></span>
      <span class="cell-name" role="cell"><b>${d.name}</b><small>${ECOSYSTEM_LABEL[d.ecosystem]}${advisory}</small></span>
      <code role="cell">${d.current}</code>
      <code role="cell" class="target">${d.target}</code>
      <span role="cell" class="usage">${usage}</span>
      <span role="cell"><span class="risk ${d.risk}" data-tooltip="${RISK_HINT[d.risk]}">${d.risk === 'security' ? icon('i-shield') : ''}${d.risk}</span></span>
    </div>`
  }).join('')
  $$('#dep-rows [data-partial="true"]').forEach((c) => { c.indeterminate = true })
  $('#dep-table').setAttribute('aria-rowcount', rows.length + 1)

  const shownPairs = rows.flatMap((d) => scopeIds(d).map((pid) => key(d.name, pid)))
  const shownStaged = shownPairs.filter((k) => state.staged.has(k)).length
  const all = $('#select-visible')
  all.checked = shownPairs.length > 0 && shownStaged === shownPairs.length
  all.indeterminate = shownStaged > 0 && shownStaged < shownPairs.length
  all.disabled = !rows.length
  all.setAttribute('aria-label', `Select all ${plural(rows.length, 'shown package')}`)

  $('#result-count').textContent = plural(rows.length, 'package')
  const empty = $('#empty-state')
  empty.hidden = rows.length > 0
  $('#dep-table').hidden = !rows.length
  if (!rows.length) {
    const filtered = $('#search').value.trim() || state.ecosystems.size || state.projectTypes.size || state.risk !== 'any'
    empty.innerHTML = filtered
      ? `<h3>No packages match</h3><button class="button" type="button" data-action="clear-filters">Clear search and filters</button>`
      : `<h3>${p ? `${p.name} is up to date` : 'Everything is up to date'}</h3><p>Mehen will tell you when a new version or advisory shows up.</p>`
  }
  const compatible = rows.filter((d) => d.risk !== 'major').flatMap((d) => scopeIds(d).map((pid) => key(d.name, pid)))
  $('#prepare-compatible').disabled = compatible.every((k) => state.staged.has(k))
}

function renderInspector() {
  const p = project(state.selected)
  const record = $('#project-record')
  record.hidden = !p
  if (p) {
    const h = projectHealth(p)
    record.innerHTML = `<div class="record-top"><h2>${p.name}</h2>
        <button class="icon-button" type="button" data-action="project-settings" aria-label="${p.name} settings" data-tooltip="Settings for this project">${icon('i-settings')}</button>
        <button class="icon-button" type="button" data-action="project-menu" aria-label="${p.name} actions" aria-haspopup="menu">${icon('i-more')}</button></div>
      <code>${p.path}</code>
      <dl class="facts">
        <div><dt>Stack</dt><dd>${stackNames(p)}</dd></div>
        <div><dt>Branch</dt><dd>${p.branch}</dd></div>
        <div><dt>Dependencies</dt><dd>${p.deps}, ${h.updates ? `${h.updates} out of date` : 'all current'}</dd></div>
        ${p.uncommitted ? `<div><dt>Working tree</dt><dd class="warn">${plural(p.uncommitted, 'uncommitted file')}</dd></div>` : ''}
      </dl>`
  }

  const pairs = stagedPairs()
  const byDep = new Map()
  pairs.forEach((x) => byDep.set(x.dep.name, [...(byDep.get(x.dep.name) ?? []), x]))
  const affected = new Set(pairs.map((x) => x.project.id))
  const files = new Set(pairs.flatMap((x) => manifestFiles(x.project, x.dep).map((f) => `${x.project.id}/${f}`)))

  const item = ([name, list]) => {
    const d = dependency(name)
    const failed = list.some((x) => state.failed.has(x.key))
    const where = list.length === 1 ? list[0].project.name : plural(list.length, 'project')
    return `<div class="staged-item ${p && !list.some((x) => x.project.id === p.id) ? 'elsewhere' : ''}">
      <span><b>${name}</b><small>${d.current} to ${d.target} · ${where}</small>${failed ? '<small class="failed">Checks failed last time</small>' : ''}</span>
      <button class="icon-button" type="button" data-unstage="${name}" aria-label="Remove ${name} from selected updates">${icon('i-x')}</button></div>`
  }
  let list
  if (!byDep.size) {
    list = `<p class="tray-empty">Tick packages in the list to select them. Nothing on disk changes until you update.</p>`
  } else if (p) {
    const here = [...byDep].filter(([, l]) => l.some((x) => x.project.id === p.id))
    const other = [...byDep].filter(([, l]) => !l.some((x) => x.project.id === p.id))
    list = (here.length ? `<div class="tray-group-label">Includes ${p.name}</div>${here.map(item).join('')}` : '') +
      (other.length ? `<div class="tray-group-label">Other projects</div>${other.map(item).join('')}` : '')
  } else {
    list = [...byDep].map(item).join('')
  }

  $('#tray').innerHTML = `<div class="tray-head"><div><h2 id="tray-title">${byDep.size} selected</h2>
      ${byDep.size ? `<button class="icon-button" type="button" data-action="clear-staged" aria-label="Clear selection" data-tooltip="Clear selection">${icon('i-x')}</button>` : ''}</div>
      <p>${byDep.size ? `Across ${plural(affected.size, 'project')}. Files unchanged until you update.` : 'Files unchanged.'}</p></div>
    <div class="tray-list">${list}</div>
    <div class="tray-foot">
      ${byDep.size ? `<div class="impact" aria-label="What will change"><div><b>${files.size}</b><small>files</small></div><div><b>${affected.size}</b><small>projects</small></div><div><b>${state.run.checks ? affected.size * 2 : 'Off'}</b><small>checks</small></div></div><p class="run-summary">${icon(state.run.commit ? 'i-git' : 'i-terminal')}${runSummary()}</p>` : ''}
      <button class="button wide" type="button" data-action="preview" ${byDep.size ? '' : 'disabled'}>${icon('i-terminal')}Review files &amp; commands</button>
      <div class="split-primary">
        <button class="button primary" type="button" data-action="update" ${byDep.size ? '' : 'disabled'}>${icon('i-play')}${runLabel()}</button>
        <button class="button primary split-toggle" type="button" data-action="run-menu" aria-haspopup="menu" aria-expanded="false" aria-controls="run-menu" aria-label="Update options" data-tooltip="Update options">${icon('i-chevron-down')}</button>
      </div>
    </div>`
}

function render() {
  renderRail()
  renderCommandBar()
  renderSecurity()
  renderTable()
  renderInspector()
}

/* ---------- Actions ---------- */

function selectProject(id) {
  state.selected = id || null
  render()
}

function toggleRow(name) {
  const d = dependency(name)
  const ids = scopeIds(d)
  const all = ids.every((pid) => state.staged.has(key(name, pid)))
  ids.forEach((pid) => (all ? state.staged.delete(key(name, pid)) : state.staged.add(key(name, pid))))
  render()
}

function stageMany(pairKeys, message) {
  const added = pairKeys.filter((k) => !state.staged.has(k))
  added.forEach((k) => state.staged.add(k))
  render()
  if (added.length) toast(message(added), { undo: () => { added.forEach((k) => state.staged.delete(k)); render() } })
}

function packagesIn(pairKeys) {
  return new Set(pairKeys.map((k) => k.split('|')[0])).size
}

function prepareCompatible(projectId) {
  const rows = projectId ? dependencies.filter((d) => remaining(d).includes(projectId)) : visibleRows()
  const keys = rows.filter((d) => d.risk !== 'major').flatMap((d) => (projectId ? [projectId] : scopeIds(d)).map((pid) => key(d.name, pid)))
  stageMany(keys, (added) => `${plural(packagesIn(added), 'compatible update')} selected.`)
}

function prepareSecurity(names) {
  const list = names ? names.map(dependency) : dependencies.filter((d) => d.advisories)
  const keys = list.flatMap((d) => scopeIds(d).map((pid) => key(d.name, pid)))
  stageMany(keys, (added) => `${plural(packagesIn(added), 'security fix', 'security fixes')} selected.`)
}

function excludeProject(pid, kind) {
  const p = project(pid)
  const path = kind === 'manifest' ? `${p.path}\\${p.solution ?? 'package.json'}` : p.path
  const rule = { id: `${pid}-${kind}-${Date.now()}`, kind, path }
  const removed = [...state.staged].filter((k) => k.endsWith(`|${pid}`))
  state.exclusions.push(rule)
  state.excluded.add(pid)
  removed.forEach((k) => state.staged.delete(k))
  if (state.selected === pid) state.selected = null
  render()
  toast(`${p.name} excluded. Mehen will skip it from now on.`, {
    undo: () => {
      state.exclusions = state.exclusions.filter((r) => r !== rule)
      state.excluded.delete(pid)
      removed.forEach((k) => state.staged.add(k))
      render()
    },
  })
}

function runScan(ids = projects.map((p) => p.id)) {
  if (state.scanning) return
  state.scanning = true
  const status = $('#scan-status')
  const button = $('#scan')
  status.classList.add('scanning')
  button.disabled = true
  let done = 0
  const tick = () => {
    $('#scan-label').textContent = `Scanning ${done + 1} of ${ids.length}`
    $('#scan-detail').textContent = project(ids[done]).name
    done++
    if (done < ids.length) return setTimeout(tick, 260)
    setTimeout(() => {
      state.scanning = false
      status.classList.remove('scanning')
      button.disabled = false
      $('#scan-label').textContent = 'Checked just now'
      $('#scan-detail').textContent = `${plural(projects.length, 'project')} · ${plural(state.roots.length, 'folder')}`
      toast(`Scan finished. No new advisories since the last check.`, { tone: 'success' })
    }, 260)
  }
  tick()
}

/* ---------- Toast and tooltip ---------- */

let toastTimer
let toastUndo = null
function toast(message, { undo, tone } = {}) {
  $('#toast-message').textContent = message
  $('#toast-icon').innerHTML = tone === 'success' ? icon('i-check') : tone === 'warn' ? icon('i-alert') : ''
  $('#toast-icon').className = `toast-icon ${tone === 'warn' ? 'warn' : ''}`
  toastUndo = undo ?? null
  $('#toast-action').hidden = !undo
  $('#toast').hidden = false
  clearTimeout(toastTimer)
  toastTimer = setTimeout(() => { $('#toast').hidden = true }, undo ? 8000 : 4500)
}

const tooltip = $('#tooltip')
let tooltipTimer
function showTooltip(target) {
  clearTimeout(tooltipTimer)
  tooltipTimer = setTimeout(() => {
    const r = target.getBoundingClientRect()
    tooltip.textContent = target.dataset.tooltip
    tooltip.hidden = false
    const half = tooltip.offsetWidth / 2
    tooltip.style.left = `${Math.max(half + 8, Math.min(innerWidth - half - 8, r.left + r.width / 2))}px`
    tooltip.style.top = `${Math.max(tooltip.offsetHeight + 8, r.top - 6)}px`
  }, 350)
}
function hideTooltip() {
  clearTimeout(tooltipTimer)
  tooltip.hidden = true
}

/* ---------- Menus ---------- */

let openMenuState = null

function openMenu(menu, trigger, { x, y, focusFirst = true } = {}) {
  closeMenu(false)
  menu.hidden = false
  if (x !== undefined) {
    menu.style.left = `${Math.min(x, innerWidth - menu.offsetWidth - 8)}px`
    menu.style.top = `${Math.min(y, innerHeight - menu.offsetHeight - 8)}px`
  }
  trigger?.setAttribute('aria-expanded', 'true')
  openMenuState = { menu, trigger }
  if (focusFirst) menu.querySelector('[role^="menuitem"]')?.focus()
}

function closeMenu(restoreFocus = true) {
  if (!openMenuState) return
  const { menu, trigger } = openMenuState
  menu.hidden = true
  trigger?.setAttribute('aria-expanded', 'false')
  openMenuState = null
  if (restoreFocus && trigger && document.contains(trigger)) trigger.focus()
}

function menuKeys(e) {
  if (!openMenuState) return false
  const items = $$('[role^="menuitem"]:not(:disabled)', openMenuState.menu)
  const i = items.indexOf(document.activeElement)
  if (e.key === 'ArrowDown') items[(i + 1) % items.length].focus()
  else if (e.key === 'ArrowUp') items[(i - 1 + items.length) % items.length].focus()
  else if (e.key === 'Home') items[0].focus()
  else if (e.key === 'End') items.at(-1).focus()
  else if (e.key === 'Escape' || e.key === 'Tab') closeMenu(e.key === 'Escape')
  else return false
  if (e.key !== 'Tab') e.preventDefault()
  return true
}

function openFilterMenu(trigger) {
  const kind = trigger.dataset.filter
  const menu = $('#filter-menu')
  const box = `<span class="box">${icon('i-check')}</span>`
  const item = (attrs, label, checked, role = 'menuitemcheckbox') => `<button type="button" role="${role}" aria-checked="${checked}" ${attrs}>${box}${label}</button>`
  if (kind === 'type') {
    menu.innerHTML = item('data-type="all"', 'All project types', !state.projectTypes.size, 'menuitemradio') + '<hr>' +
      Object.entries(PROJECT_TYPE_LABEL).map(([k, v]) => item(`data-type="${k}"`, v, state.projectTypes.has(k))).join('')
  } else if (kind === 'ecosystem') {
    menu.innerHTML = item('data-eco="all"', 'All dependency types', !state.ecosystems.size, 'menuitemradio') + '<hr>' +
      Object.entries(ECOSYSTEM_LABEL).map(([k, v]) => item(`data-eco="${k}"`, v, state.ecosystems.has(k))).join('')
  } else {
    menu.innerHTML = Object.entries(RISK_FILTERS).map(([k, v]) => item(`data-risk="${k}"`, v, state.risk === k, 'menuitemradio')).join('')
  }
  menu.setAttribute('aria-label', trigger.querySelector('span').textContent)
  const r = trigger.getBoundingClientRect()
  openMenu(menu, trigger, { x: r.left, y: r.bottom + 4 })
}

function openRunMenu(trigger) {
  const menu = $('#run-menu')
  const box = `<span class="box">${icon('i-check')}</span>`
  menu.innerHTML = `<div class="menu-title"><b>When you update</b>Applies to every selected project</div>
    <button type="button" class="rich" role="menuitemradio" aria-checked="${state.run.checks}" data-run="checks">${box}<span><b>Update and run checks</b><small>Install, then build and test each project. A project whose checks fail is put back.</small></span></button>
    <button type="button" class="rich" role="menuitemradio" aria-checked="${!state.run.checks}" data-run="install">${box}<span><b>Update and install only</b><small>Skip build and test. Useful when CI runs your tests.</small></span></button>
    <hr>
    <button type="button" class="rich" role="menuitemcheckbox" aria-checked="${state.run.commit}" data-run="commit">${box}<span><b>Commit each repository</b><small>Commits only the files Mehen changed, once that project is done. Never pushes.</small></span></button>`
  const r = trigger.getBoundingClientRect()
  openMenu(menu, trigger, { x: r.right - 320, y: r.top })
  menu.style.top = `${Math.max(8, r.top - menu.offsetHeight - 6)}px`
  menu.style.left = `${Math.max(8, r.right - menu.offsetWidth)}px`
}

function setRun(option) {
  if (option === 'checks') state.run.checks = true
  if (option === 'install') state.run.checks = false
  if (option === 'commit') state.run.commit = !state.run.commit
  saveRun()
}

function showContextMenu(pid, x, y, trigger) {
  const p = project(pid)
  const menu = $('#context-menu')
  menu.dataset.project = pid
  menu.setAttribute('aria-label', `${p.name} actions`)
  const excluded = state.excluded.has(pid)
  menu.innerHTML = `<div class="menu-title"><b>${p.name}</b>${p.path}</div>
    <button type="button" role="menuitem" data-context="open">${icon('i-folder')}Open project folder</button>
    <button type="button" role="menuitem" data-context="settings">${icon('i-settings')}Project settings…</button>
    <button type="button" role="menuitem" data-context="prepare" ${excluded ? 'disabled' : ''}>${icon('i-plus')}Select compatible updates for ${p.name}</button>
    <div class="separator"></div>
    ${excluded
      ? `<button type="button" role="menuitem" data-context="include">${icon('i-undo')}Include ${p.name} again</button>`
      : `<button type="button" role="menuitem" class="danger" data-context="exclude">${icon('i-eyeoff')}Exclude from Mehen…</button>`}`
  openMenu(menu, trigger, { x, y })
}

/* ---------- Dialog ---------- */

let dialogReturn = null
let dialogOnClose = null

function openDialog({ title, description = '', iconId = 'i-folder', tone = '', size = '', body = '', foot = '', raw = null, onClose = null }) {
  const backdrop = $('#dialog-backdrop')
  const dialog = $('#dialog')
  if (backdrop.hidden) dialogReturn = document.activeElement
  dialogOnClose = onClose
  dialog.className = `dialog ${size}`
  delete dialog.dataset.mode
  dialog.innerHTML = raw ?? `<header class="dialog-head ${tone}">${icon(iconId)}<div><h2 id="dialog-title">${title}</h2>${description ? `<p id="dialog-desc">${description}</p>` : ''}</div>
      <button class="icon-button" type="button" data-action="close-dialog" aria-label="Close">${icon('i-x')}</button></header>
    <div class="dialog-body">${body}</div>${foot ? `<footer class="dialog-foot">${foot}</footer>` : ''}`
  dialog.setAttribute('aria-describedby', description ? 'dialog-desc' : '')
  backdrop.hidden = false
  $('#app-shell').inert = true
  hideTooltip()
  const target = dialog.querySelector('[autofocus]') ?? dialog.querySelector('.dialog-foot .primary:not(:disabled)') ?? dialog.querySelector('input, select, button:not([data-action="close-dialog"])')
  target?.focus()
}

function closeDialog() {
  const backdrop = $('#dialog-backdrop')
  if (backdrop.hidden) return
  backdrop.hidden = true
  $('#app-shell').inert = false
  const cb = dialogOnClose
  dialogOnClose = null
  cb?.()
  if (dialogReturn && document.contains(dialogReturn)) dialogReturn.focus()
  dialogReturn = null
}

function trapFocus(e) {
  const nodes = $$('button:not(:disabled), input:not(:disabled), select:not(:disabled), a[href], [tabindex="0"]', $('#dialog')).filter((n) => n.offsetParent !== null)
  if (!nodes.length) return
  const first = nodes[0]
  const last = nodes.at(-1)
  if (e.shiftKey && document.activeElement === first) { e.preventDefault(); last.focus() }
  else if (!e.shiftKey && document.activeElement === last) { e.preventDefault(); first.focus() }
}

/* ---------- Dialog content ---------- */

function groupByProject(pairs) {
  const map = new Map()
  pairs.forEach((x) => map.set(x.project.id, [...(map.get(x.project.id) ?? []), x]))
  return [...map].map(([pid, list]) => ({ project: project(pid), list }))
}

function openPreview() {
  const groups = groupByProject(stagedPairs())
  const text = groups.map(({ project: p, list }) =>
    [`<span class="comment"># ${esc(p.name)} (${esc(p.branch)})</span>`, `cd ${esc(p.path)}`, ...list.map((x) => esc(updateCommand(x.dep))), ...(state.run.checks ? checksFor(p) : [installFor(p)]).map(esc), ...(state.run.commit ? [esc(`git commit -m "${commitMessage(list)}"${commitBody(list).map((line) => ` -m "${line}"`).join('')} -- <changed files>`)] : [])].join('\n'),
  ).join('\n\n')
  openDialog({
    title: 'Files and commands',
    description: 'What Mehen will run in each project. Reviewing changes nothing on disk.',
    iconId: 'i-terminal',
    size: 'wide',
    body: `<pre class="commands">${text}</pre>`,
    foot: `<button class="button ghost" type="button" data-action="copy-commands">Copy commands</button>
      <button class="button" type="button" data-action="close-dialog">Close</button>
      <button class="button primary" type="button" data-action="update">${icon('i-play')}Continue to update</button>`,
  })
}

function openUpdate() {
  const groups = groupByProject(stagedPairs())
  if (!groups.length) return
  const totalPackages = new Set(stagedPairs().map((x) => x.dep.name)).size
  const body = `<div class="plan-list">${groups.map(({ project: p, list }) => `
    <section class="plan-project">
      <header>${avatar(p)}<span class="who"><b>${p.name}</b><small>${p.path}</small></span><span class="branch">${icon('i-branch')}${p.branch}</span></header>
      <ul>
        ${list.map((x) => `<li>${icon('i-box')}<b>${x.dep.name}</b><code class="dim">${x.dep.current}</code> to <code>${x.dep.target}</code></li>`).join('')}
        <li>${icon('i-file')}<span class="dim">${[...new Set(list.flatMap((x) => manifestFiles(p, x.dep)))].join(', ')}</span></li>
        <li>${icon('i-terminal')}<span class="dim">${state.run.checks ? `Checks: ${checksFor(p).join(', ')}` : `Install: ${installFor(p)}. Checks skipped.`}</span></li>
        ${state.run.commit ? `<li>${icon('i-git')}<span class="dim">Commit on ${p.branch}: “${esc(commitMessage(list))}”</span></li>` : ''}
      </ul>
      ${p.uncommitted ? `<div class="plan-warning">${icon('i-alert')}<span>${plural(p.uncommitted, 'file has', 'files have')} uncommitted changes on <b>${p.branch}</b>. ${state.run.commit ? 'Mehen commits only the files it changed and leaves these alone.' : 'Mehen leaves them alone, but commit or stash them first if you want a clean update commit.'}</span></div>` : ''}
    </section>`).join('')}</div>
    <p class="plan-note">${state.run.checks ? "If a check fails, Mehen puts that project's files back and keeps its updates selected." : 'Nothing is built or tested. Run your tests or let CI check before merging.'} ${state.run.commit ? 'Commits stay local; nothing is pushed.' : 'Nothing is committed until you choose to.'}</p>`
  openDialog({
    title: `Update ${plural(groups.length, 'project')}`,
    description: `${plural(totalPackages, 'package')} will change. Mehen edits the files below and refreshes lockfiles${state.run.checks ? ", then runs each project's checks" : ''}${state.run.commit ? ', then commits' : ''}.`,
    iconId: 'i-play',
    size: 'wide',
    body,
    foot: `<div class="foot-options" role="group" aria-label="When updating">
        <label class="inline-option"><input class="check" type="checkbox" data-run-option="checks" ${state.run.checks ? 'checked' : ''}>Run build and test checks</label>
        <label class="inline-option"><input class="check" type="checkbox" data-run-option="commit" ${state.run.commit ? 'checked' : ''}>Commit each repository</label>
      </div>
      <button class="button" type="button" data-action="close-dialog">Cancel</button>
      <button class="button primary" type="button" data-action="run-update">${icon('i-play')}${runLabel()}</button>`,
  })
}

function runUpdate() {
  const groups = groupByProject(stagedPairs())
  const { checks, commit } = state.run
  const steps = ['Updating files', 'Installing', ...(checks ? ['Building', 'Testing'] : []), ...(commit ? ['Committing'] : [])]
  const status = groups.map(() => ({ step: -1, result: null }))
  const dialog = $('#dialog')

  const row = ({ project: p, list }, i) => {
    const s = status[i]
    let state = `<span class="step-state">Waiting</span>`
    let detail = `${plural(list.length, 'package')}`
    if (s.result === 'pass') { state = `<span class="step-state pass">${icon('i-check')}Done</span>`; detail = 'Finished' }
    else if (s.result === 'fail') { state = `<span class="step-state fail">${icon('i-alert')}Failed</span>`; detail = '<span class="fail">Tests failed. Files restored.</span>' }
    else if (s.step >= 0) {
      const name = steps[s.step]
      state = `<span class="step-state running">${icon('i-refresh')}${name}</span>`
      detail = name === 'Installing' ? installFor(p) : name === 'Building' ? checksFor(p)[0] : name === 'Testing' ? checksFor(p)[1] : name === 'Committing' ? `git commit on ${p.branch}` : detail
    }
    return `<div class="progress-project">${avatar(p)}<span class="who"><b>${p.name}</b><small class="${s.result === 'fail' ? 'fail' : ''}">${detail}</small></span>${state}</div>`
  }
  const paint = (done) => {
    dialog.querySelector('.dialog-body').innerHTML = groups.map(row).join('')
    if (done) return
    dialog.querySelector('.dialog-foot').innerHTML = `<span class="note">Updating. You can keep this open or come back later.</span>`
  }
  dialog.querySelector('#dialog-title').textContent = 'Updating'
  dialog.querySelector('#dialog-desc').textContent = `Each project is updated${checks ? ' and checked' : ''}${commit ? ' and committed' : ''} one at a time.`
  paint()

  let gi = 0
  const advance = () => {
    const s = status[gi]
    const p = groups[gi].project
    s.step++
    if (p.failsTests && steps[s.step] === 'Testing') {
      s.result = 'fail'
    } else if (s.step >= steps.length) {
      s.result = 'pass'
    }
    paint()
    if (s.result) {
      gi++
      if (gi >= groups.length) return setTimeout(() => finishUpdate(groups, status, { checks, commit }), 400)
    }
    setTimeout(advance, 420)
  }
  setTimeout(advance, 300)
}

function finishUpdate(groups, status, { checks, commit }) {
  const passed = groups.filter((_, i) => status[i].result === 'pass')
  const failed = groups.filter((_, i) => status[i].result === 'fail')
  passed.forEach(({ list }) => list.forEach((x) => { state.staged.delete(x.key); state.updated.add(x.key); state.failed.delete(x.key) }))
  failed.forEach(({ list }) => list.forEach((x) => state.failed.add(x.key)))
  render()

  const summary = failed.length
    ? `<div class="result-summary mixed">${icon('i-alert')}<div><b>${passed.length} of ${plural(groups.length, 'project')} updated</b><span>${failed.map((g) => g.project.name).join(', ')} failed ${failed.length === 1 ? 'its' : 'their'} checks and ${failed.length === 1 ? 'was' : 'were'} restored.${commit && passed.length ? ` ${passed.length === 1 ? 'The other was' : 'The others were'} committed.` : ''}</span></div></div>`
    : `<div class="result-summary ${checks ? '' : 'mixed'}">${icon(checks ? 'i-shield-check' : 'i-alert')}<div><b>${plural(groups.length, 'project')} updated${commit ? ' and committed' : ''}</b><span>${checks ? 'Every build and test passed.' : 'Checks were skipped. Run your tests or let CI check before merging.'}</span></div></div>`
  const rows = groups.map((g, i) => {
    const ok = status[i].result === 'pass'
    return `<div class="progress-project">${avatar(g.project)}<span class="who"><b>${g.project.name}</b>
      <small class="${ok ? '' : 'fail'}">${ok ? `${g.list.map((x) => `${x.dep.name} ${x.dep.target}`).join(', ')}${commit ? ` · committed ${fakeHash(g.project.id + g.list.length)} on ${g.project.branch}` : ''}` : 'Files restored. The updates are still selected so you can try again.'}</small>
      ${ok ? '' : `<pre class="log">${esc(checksFor(g.project)[1])}\nFailed: 2, Passed: 146, Skipped: 0 (Claims.Tests)\n  JsonSettingsTests.ReadsLegacyDates\n  JsonSettingsTests.RoundTripsNullableEnums</pre>`}</span>
      <span class="step-state ${ok ? 'pass' : 'fail'}">${icon(ok ? (commit ? 'i-git' : 'i-check') : 'i-alert')}${ok ? (commit ? 'Committed' : 'Updated') : 'Restored'}</span></div>`
  }).join('')

  const dialog = $('#dialog')
  dialog.querySelector('#dialog-title').textContent = failed.length ? 'Update finished with a problem' : 'Update finished'
  dialog.querySelector('#dialog-desc').textContent = commit ? 'Each updated repository has a new local commit. Nothing was pushed.' : 'Changed files are ready to commit. Nothing has been committed yet.'
  dialog.querySelector('.dialog-body').innerHTML = summary + rows
  dialog.querySelector('.dialog-foot').innerHTML = `<button class="button" type="button" data-action="close-dialog">Done</button>
    ${passed.length && !commit ? `<button class="button primary" type="button" data-action="commit" data-projects="${passed.map((g) => g.project.id).join(',')}">${icon('i-git')}Commit ${plural(passed.length, 'project')}…</button>` : ''}`
  dialog.querySelector('.dialog-foot .primary, .dialog-foot .button')?.focus()
}

function openCommit(ids) {
  const list = ids.map(project)
  const changesIn = (p) => dependencies.filter((d) => state.updated.has(key(d.name, p.id))).map((d) => ({ dep: d }))
  openDialog({
    title: `Commit ${plural(list.length, 'project')}`,
    description: 'Only the files Mehen changed are committed. Other changes in the working tree are left alone. Nothing is pushed.',
    iconId: 'i-git',
    body: `<div class="plan-list">${list.map((p) => `<div class="progress-project">${avatar(p)}<span class="who"><b>${p.name}</b><small>“${esc(commitMessage(changesIn(p)))}” · ${commitBody(changesIn(p)).map(esc).join(', ')}</small></span><span class="branch">${icon('i-branch')}${p.branch}</span></div>`).join('')}</div>`,
    foot: `<button class="button" type="button" data-action="close-dialog">Not now</button>
      <button class="button primary" type="button" data-action="confirm-commit" data-count="${list.length}">${icon('i-git')}Commit</button>`,
  })
}

function openAdvisory(name) {
  const d = dependency(name)
  const ids = scopeIds(d)
  const staged = ids.every((pid) => state.staged.has(key(name, pid)))
  openDialog({
    title: `${name} advisories`,
    description: `You have ${d.current} installed in ${plural(remaining(d).length, 'project')}. Version ${d.target} fixes both.`,
    iconId: 'i-shield',
    tone: 'danger',
    body: `<div class="advisory-list">${d.advisories.map((a) => `<a href="${a.url}" target="_blank" rel="noopener noreferrer"><span><b>${a.id}</b><small>${a.title}</small></span><span class="risk security">${a.severity}</span>${icon('i-chevron')}</a>`).join('')}</div>`,
    foot: `<button class="button" type="button" data-action="close-dialog">Close</button>
      <button class="button primary" type="button" data-action="prepare-advisory" data-name="${name}" ${staged ? 'disabled' : ''}>${staged ? `${icon('i-check')}Fix selected` : 'Select fix'}</button>`,
  })
}

function openExclude(pid) {
  const p = project(pid)
  const repoRoot = p.path
  const choices = [
    ['repository', 'The whole repository', 'Skips every project inside it, including ones added later.', repoRoot],
    ['folder', 'This folder', 'Skips the folder and everything under it.', repoRoot],
    ['project', 'Just this project', 'Other projects in the same repository are still checked.', p.name],
    ['manifest', 'One manifest file', 'Only this file is ignored. Useful for samples and fixtures.', `${repoRoot}\\${p.solution ?? 'package.json'}`],
  ]
  openDialog({
    title: `Exclude ${p.name}`,
    description: 'Excluded projects are skipped during scans and never updated. You can bring them back from Excluded projects & folders.',
    iconId: 'i-eyeoff',
    body: `<div class="choice-list" role="radiogroup" aria-label="What to exclude">${choices.map(([k, label, help, path], i) => `
      <label class="choice"><input type="radio" name="exclude-kind" value="${k}" ${i === 0 ? 'checked' : ''}><span><b>${label}</b><small>${help}</small><br><code>${esc(path)}</code></span></label>`).join('')}</div>`,
    foot: `<button class="button" type="button" data-action="close-dialog">Cancel</button>
      <button class="button danger" type="button" data-action="confirm-exclude" data-project="${pid}">Exclude</button>`,
  })
  $('#dialog input[name="exclude-kind"]').focus()
}

let addRootReturn = null

function returnFromAddRoot() {
  const back = addRootReturn
  addRootReturn = null
  if (back === 'manager') openManager({ keepQuery: true })
  else if (back === 'settings') openSettings(null, 'scanning')
  else closeDialog()
}

function openAddRoot(returnTo = null) {
  addRootReturn = returnTo === 'manager' || returnTo === 'settings' ? returnTo : null
  openDialog({
    title: 'Add a folder to scan',
    description: 'Mehen looks for npm, Cargo, NuGet, and GitHub Actions projects anywhere under this folder. Your exclusions still apply.',
    iconId: 'i-folder-plus',
    body: `<label class="field" for="root-path">Folder</label>
      <div class="field-row" style="margin-top:5px"><input class="input mono" id="root-path" value="D:\\projects" spellcheck="false" autofocus>
      <button class="button" type="button" data-action="browse-root">Browse…</button></div>`,
    foot: `<button class="button" type="button" data-action="root-back">Cancel</button>
      <button class="button primary" type="button" data-action="confirm-root">${icon('i-folder-plus')}Add and scan</button>`,
  })
}

/* ---------- Project manager ---------- */

const manager = { query: '', selected: new Set(), scanning: new Set(), scanningRoots: new Set() }

function managerStatus(p) {
  if (state.excluded.has(p.id)) return ['Excluded', 'excluded']
  if (manager.scanning.has(p.id)) return ['Scanning…', 'review']
  const h = projectHealth(p)
  if (h.vulnerable) return [`${h.vulnerable} vulnerable`, 'urgent']
  if (h.updates) return [plural(h.updates, 'update'), 'review']
  return ['Up to date', 'clear']
}

/** Groups for the manager: each watched folder with the projects that match the search. */
function managerGroups() {
  const q = manager.query.trim().toLowerCase()
  return state.roots.map((root) => {
    const all = projects.filter((p) => p.root === root.id)
    const folderMatch = !q || root.path.toLowerCase().includes(q)
    const shown = folderMatch ? all : all.filter((p) => `${p.name} ${p.path} ${stackNames(p)}`.toLowerCase().includes(q))
    return { root, all, shown, hidden: !!q && !folderMatch && !shown.length }
  }).filter((g) => !g.hidden)
}

function checkState(ids) {
  const n = ids.filter((id) => manager.selected.has(id)).length
  return { checked: ids.length > 0 && n === ids.length, partial: n > 0 && n < ids.length }
}

function renderManager() {
  const body = $('#dialog .dialog-body')
  if (!body || $('#dialog').dataset.mode !== 'manager') return
  const groups = managerGroups()
  const shownIds = groups.flatMap((g) => g.shown.map((p) => p.id))
  const all = checkState(shownIds)
  const row = (p) => {
    const [label, tone] = managerStatus(p)
    return `<div class="manager-row ${manager.selected.has(p.id) ? 'selected' : ''} ${state.excluded.has(p.id) ? 'excluded' : ''}" role="row" data-manager-project="${p.id}">
      <span role="cell"><input class="check" type="checkbox" data-manager-check="${p.id}" ${manager.selected.has(p.id) ? 'checked' : ''} aria-label="Select ${p.name}"></span>
      <span role="cell"><button class="manager-project" type="button" data-manager-open="${p.id}">${avatar(p)}<span class="who"><b>${p.name}</b><small>${p.path}</small></span></button></span>
      <span role="cell" class="status ${tone}">${label}</span>
      <span role="cell">${chips(p)}</span>
      <span role="cell"><button class="icon-button" type="button" data-manager-more="${p.id}" aria-label="${p.name} actions" aria-haspopup="menu">${icon('i-more')}</button></span>
    </div>`
  }
  const folder = ({ root, all: inFolder, shown }) => {
    const ids = shown.map((p) => p.id)
    const s = checkState(ids)
    const scanning = manager.scanningRoots.has(root.id)
    return `<div class="manager-folder" role="row">
      <span role="cell"><input class="check" type="checkbox" data-folder-check="${root.id}" ${s.checked ? 'checked' : ''} data-partial="${s.partial}" ${ids.length ? '' : 'disabled'} aria-label="Select every project in ${esc(root.path)}"></span>
      <span role="cell" class="folder-name">${icon('i-folder')}<span><b>${esc(root.path)}</b><small>${inFolder.length ? `${plural(inFolder.length, 'project')}, including subfolders` : 'No projects found here yet'}</small></span></span>
      <span role="cell" class="folder-actions">
        <button class="button" type="button" data-root-scan="${root.id}" ${scanning ? 'disabled' : ''}>${icon('i-refresh')}${scanning ? 'Scanning…' : 'Scan'}</button>
        <button class="button ghost" type="button" data-root-remove="${root.id}" aria-label="Stop watching ${esc(root.path)}">Remove</button>
      </span>
    </div>${shown.map(row).join('')}`
  }
  $('#manager-list').innerHTML = `<div class="manager-row head" role="row"><span role="columnheader"><input class="check" type="checkbox" data-manager-all ${all.checked ? 'checked' : ''} data-partial="${all.partial}" ${shownIds.length ? '' : 'disabled'} aria-label="Select all shown projects"></span><span role="columnheader">Project</span><span role="columnheader">Status</span><span role="columnheader">Stack</span><span role="columnheader"><span class="sr-only">Actions</span></span></div>` +
    (groups.map(folder).join('') || `<p class="tray-empty">${state.roots.length ? 'No folders or projects match.' : 'Mehen is not watching any folders. Add one to find your projects.'}</p>`)
  $$('#manager-list [data-partial="true"]').forEach((c) => { c.indeterminate = true })

  const selected = projects.filter((p) => manager.selected.has(p.id))
  const watchedCount = projects.filter((p) => isWatched(p.id)).length
  $('#dialog .dialog-foot').innerHTML = `<div class="selection-bar" style="width:100%"><span>${selected.length ? `${selected.length} selected` : `${plural(watchedCount, 'project')} in ${plural(state.roots.length, 'folder')}`}</span>
    ${selected.length === 1 ? '<button class="button" type="button" data-manager-action="settings">Project settings…</button>' : ''}
    ${selected.length ? `<button class="button" type="button" data-manager-action="scan">${icon('i-refresh')}Scan selected</button><button class="button danger-outline" type="button" data-manager-action="exclude">${icon('i-eyeoff')}Exclude selected</button>` : ''}</div>`
}

function openManager({ keepQuery = false } = {}) {
  if (!keepQuery) { manager.query = ''; manager.selected.clear() }
  openDialog({
    title: 'Manage projects',
    description: 'The folders Mehen watches and the projects found in them. Select projects to scan or exclude several at once.',
    iconId: 'i-box',
    size: 'wide',
    body: `<div class="manager-toolbar"><label class="search-field">${icon('i-search')}<span class="sr-only">Search folders and projects</span><input class="input" type="search" data-manager-query placeholder="Search folders and projects" value="${esc(manager.query)}" autofocus></label>
      <button class="button" type="button" data-action="add-root">${icon('i-folder-plus')}Add a folder…</button></div><div id="manager-list" role="table" aria-label="Folders and projects"></div>`,
    foot: ' ',
  })
  $('#dialog').dataset.mode = 'manager'
  renderManager()
}

/* ---------- Watched folders ---------- */

function scanFolder(rootId) {
  const ids = projects.filter((p) => p.root === rootId).map((p) => p.id)
  const root = state.roots.find((r) => r.id === rootId)
  manager.scanningRoots.add(rootId)
  ids.forEach((id) => manager.scanning.add(id))
  renderManager()
  setTimeout(() => {
    manager.scanningRoots.delete(rootId)
    ids.forEach((id) => manager.scanning.delete(id))
    renderManager()
    toast(ids.length ? `Scanned ${root.path}. Nothing new.` : `Scanned ${root.path}. No projects found.`, { tone: 'success' })
  }, 900)
}

function removeFolder(rootId) {
  const index = state.roots.findIndex((r) => r.id === rootId)
  const root = state.roots[index]
  const ids = projects.filter((p) => p.root === rootId).map((p) => p.id)
  const dropped = [...state.staged].filter((k) => ids.includes(k.split('|')[1]))
  const wasSelected = ids.includes(state.selected) ? state.selected : null
  state.roots.splice(index, 1)
  dropped.forEach((k) => state.staged.delete(k))
  ids.forEach((id) => manager.selected.delete(id))
  if (wasSelected) state.selected = null
  refreshAll()
  toast(`Stopped watching ${root.path}.${ids.length ? ` Its ${plural(ids.length, 'project')} will no longer be checked.` : ''}`, {
    undo: () => {
      state.roots.splice(index, 0, root)
      dropped.forEach((k) => state.staged.add(k))
      if (wasSelected) state.selected = wasSelected
      refreshAll()
    },
  })
}

function refreshAll() {
  render()
  if ($('#dialog-backdrop').hidden) return
  if ($('#dialog').dataset.mode === 'manager') renderManager()
  if ($('#dialog').dataset.mode === 'settings') renderSettings()
}

/* ---------- Settings ---------- */

const settings = { projectId: null, tab: 'general', policyTarget: 'global', policyType: 'npm', configs: {}, toggles: { 'system-theme': false, compact: true, 'startup-scan': true, 'auto-stage': false, verification: true, 'pin-security': true, 'security-notify': true } }

function policyTargets() {
  const p = project(settings.projectId)
  if (!p) return [{ value: 'global', label: 'All projects', kind: 'auto', group: 'Scope' }]
  const items = [{ value: 'repo', label: `Repository: ${p.name}`, kind: 'auto', group: 'Repository' }]
  if (p.stacks.some((s) => ['ts', 'react', 'astro'].includes(s))) items.push({ value: 'package.json', label: 'package.json', kind: 'npm', group: 'Manifests and projects' })
  if (p.stacks.some((s) => ['rust', 'tauri'].includes(s))) items.push({ value: 'Cargo.toml', label: p.tauri ? 'src-tauri/Cargo.toml' : 'Cargo.toml', kind: 'cargo', group: 'Manifests and projects' })
  if (p.stacks.includes('actions')) items.push({ value: 'workflows', label: '.github/workflows/*.yml', kind: 'actions', group: 'Manifests and projects' })
  if (p.solution) {
    const app = p.id === 'claims' ? 'Claims.Desktop' : 'Billing.Api'
    const tests = p.id === 'claims' ? 'Claims.Tests' : 'Billing.Tests'
    items.push({ value: 'solution', label: p.solution, kind: 'nuget', group: 'Manifests and projects' })
    items.push({ value: 'props', label: 'Directory.Packages.props', kind: 'nuget', group: 'Manifests and projects' })
    items.push({ value: 'app-csproj', label: `src/${app}.csproj`, kind: 'nuget', group: 'Manifests and projects', coveredBy: p.solution })
    items.push({ value: 'test-csproj', label: `tests/${tests}.csproj`, kind: 'nuget', group: 'Manifests and projects', coveredBy: p.solution })
  }
  dependencies.filter((d) => d.projects.includes(p.id)).forEach((d) => items.push({ value: `dep:${d.name}`, label: d.name, kind: d.ecosystem, group: 'Dependencies', dependency: d }))
  return items
}

function defaultPolicy(kind, target) {
  const p = project(settings.projectId)
  const working = p ? p.path : 'Repository root'
  const dep = target.dependency
  if (kind === 'npm') return { update: dep ? `npm install ${dep.name}@${dep.target}` : 'npm install', validation: ['npm run build', 'npm test --if-present'], working }
  if (kind === 'cargo') return { update: dep ? `cargo update -p ${dep.name} --precise ${dep.target}` : 'cargo update', validation: ['cargo build --locked', 'cargo test --locked'], working }
  if (kind === 'nuget') {
    const buildTarget = target.value === 'solution' ? target.label : target.coveredBy ?? ''
    const suffix = buildTarget ? ` ${buildTarget}` : ''
    return { update: dep ? `dotnet add package ${dep.name} --version ${dep.target}` : `dotnet restore${suffix}`, validation: target.coveredBy ? [] : [`dotnet build${suffix} --no-restore`, `dotnet test${suffix} --no-build`], working }
  }
  return { update: dep ? `Set ${dep.name} to ${dep.target}` : 'Update action versions in workflow files', validation: ['git diff --check'], working }
}

function currentPolicy() {
  const items = policyTargets()
  const target = items.find((x) => x.value === settings.policyTarget) ?? items[0]
  const kind = target.kind === 'auto' ? settings.policyType : target.kind
  const k = `${settings.projectId ?? 'global'}|${target.value}|${kind}`
  if (!settings.configs[k]) settings.configs[k] = { ...defaultPolicy(kind, target), override: !settings.projectId, stopOnFailure: true }
  return { target, kind, config: settings.configs[k] }
}

const sw = (id, label) => `<button class="switch" type="button" role="switch" aria-checked="${settings.toggles[id]}" data-toggle="${id}" aria-label="${label}"></button>`
const settingRow = (title, help, control) => `<div class="setting"><span><b>${title}</b><small>${help}</small></span>${control}</div>`

function settingsPanel() {
  const tab = settings.tab
  if (tab === 'general') {
    return `<div class="group"><h4>Appearance</h4>
        ${settingRow('Color palette', 'Sun and serpent follows the logo: gold for actions, turquoise for selection.', `<select class="select" id="palette-select">${[['faience', 'Sun and serpent'], ['parchment', 'Parchment and copper']].map(([v, l]) => `<option value="${v}" ${palette === v ? 'selected' : ''}>${l}</option>`).join('')}</select>`)}
        ${settingRow('Follow the Windows theme', 'Switch between light and dark with Windows.', sw('system-theme', 'Follow the Windows theme'))}
        ${settingRow('Compact rows', 'Fit more packages on screen.', sw('compact', 'Compact rows'))}</div>
      <div class="group"><h4>Startup</h4>
        ${settingRow('Scan when Mehen opens', 'Check every project for new versions and advisories on launch.', sw('startup-scan', 'Scan when Mehen opens'))}</div>`
  }
  if (tab === 'scanning') {
    return `<div class="group"><h4>Folders</h4>
        ${state.roots.map((r) => settingRow(`<span class="path">${esc(r.path)}</span>`, `${plural(projects.filter((p) => p.root === r.id).length, 'project')}, including subfolders`, `<div class="actions"><button class="button" type="button" data-root-scan="${r.id}">Scan</button><button class="button" type="button" data-root-remove="${r.id}" aria-label="Stop watching ${esc(r.path)}">Remove</button></div>`)).join('')}
        <div style="padding-top:10px"><button class="button" type="button" data-action="add-root">${icon('i-folder-plus')}Add a folder…</button></div></div>
      <div class="group"><h4>Excluded</h4>
        ${state.exclusions.map((r) => settingRow(`<span class="path">${esc(r.path)}</span>`, { repository: 'Whole repository', folder: 'Folder and everything under it', project: 'Single project', manifest: 'Single manifest file' }[r.kind], `<div class="actions"><button class="button" type="button" data-rule-remove="${r.id}">${icon('i-undo')}Include again</button></div>`)).join('') || '<p class="tray-empty">Nothing is excluded.</p>'}</div>`
  }
  if (tab === 'security') {
    return `<div class="group"><h4>Vulnerabilities</h4>
        ${settingRow('Keep vulnerable packages at the top', 'Security fixes stay above routine updates.', sw('pin-security', 'Keep vulnerable packages at the top'))}
        ${settingRow('Advisory sources', 'Where Mehen looks for known vulnerabilities.', '<select class="select"><option>All available sources</option><option>GitHub only</option><option>Registry sources only</option></select>')}
        ${settingRow('Notify about new vulnerabilities', 'Show a Windows notification naming the affected projects.', sw('security-notify', 'Notify about new vulnerabilities'))}</div>`
  }
  const items = policyTargets()
  if (!items.some((x) => x.value === settings.policyTarget)) settings.policyTarget = items[0].value
  const { target, kind, config } = currentPolicy()
  const inherited = !!settings.projectId && !config.override
  const dis = inherited ? 'disabled' : ''
  const groups = [...new Set(items.map((x) => x.group))]
  return `<div class="policy-scope">
      <label class="field">Configure<select class="select" id="policy-target">${groups.map((g) => `<optgroup label="${g}">${items.filter((x) => x.group === g).map((x) => `<option value="${esc(x.value)}" ${x.value === settings.policyTarget ? 'selected' : ''}>${esc(x.label)}</option>`).join('')}</optgroup>`).join('')}</select></label>
      <label class="field">Dependency type<select class="select" id="policy-type" ${target.kind === 'auto' ? '' : 'disabled'}>${Object.entries(ECOSYSTEM_LABEL).map(([k, v]) => `<option value="${k}" ${kind === k ? 'selected' : ''}>${v}</option>`).join('')}</select></label>
    </div>
    ${settings.projectId ? `<div class="inherit-line"><span><b>${inherited ? 'Using the defaults' : 'Custom for this target'}</b> from all projects, ${ECOSYSTEM_LABEL[kind]}</span><button class="switch" type="button" role="switch" aria-checked="${config.override}" data-policy-override aria-label="Customize for this target"></button></div>` : ''}
    ${target.coveredBy ? `<div class="coverage-note"><b>Built as part of ${target.coveredBy}</b><span>No separate build runs for this project unless you customize it.</span></div>` : ''}
    <div class="group"><h4>Update</h4>
      ${settingRow('Command', 'Runs from the working folder below.', `<input class="input mono" data-policy-field="update" value="${esc(config.update)}" ${dis}>`)}
      ${settingRow('Allowed versions', 'Security fixes can go further after you review them.', `<select class="select" ${dis}><option>Patch and minor</option><option>Patch only</option><option>Any stable version</option><option>Ask every time</option></select>`)}
    </div>
    <div class="group"><h4>Checks</h4>
      <div class="command-list">${config.validation.map((c, i) => `<div><input class="input mono" data-policy-command="${i}" value="${esc(c)}" aria-label="Check ${i + 1}" ${dis}><button class="icon-button" type="button" data-policy-remove="${i}" aria-label="Remove check ${i + 1}" ${dis}>${icon('i-x')}</button></div>`).join('') || '<p>No checks of its own. The solution build covers it.</p>'}</div>
      <button class="button ghost" type="button" data-policy-add ${dis}>${icon('i-plus')}Add a check</button>
      ${settingRow('Working folder', 'Paths relative to the repository work too.', `<input class="input mono" data-policy-field="working" value="${esc(config.working)}" ${dis}>`)}
      ${settingRow('Stop at the first failed check', 'Skip the remaining checks once one fails.', `<button class="switch" type="button" role="switch" aria-checked="${config.stopOnFailure}" data-policy-stop aria-label="Stop at the first failed check" ${dis}></button>`)}
    </div>
    <p class="resolution"><b>Which setting wins:</b> manifest or project, then repository, then dependency type, then all projects.</p>`
}

const SETTINGS_TABS = {
  general: ['General', 'How Mehen looks and starts.'],
  scanning: ['Scanning', 'Folders Mehen watches and what it skips.'],
  updates: ['Updates and checks', 'Commands and version limits for this scope.'],
  security: ['Security', 'Advisory sources and alerts.'],
}

function renderSettings() {
  const p = project(settings.projectId)
  const tabs = p ? ['updates'] : Object.keys(SETTINGS_TABS)
  const [title, help] = SETTINGS_TABS[settings.tab]
  $('#dialog').innerHTML = `<div class="settings-shell">
      <nav class="settings-nav" role="tablist" aria-orientation="vertical" aria-label="Settings sections"><h2 id="dialog-title">${p ? p.name : 'Settings'}</h2><p>${p ? 'Settings for this project' : 'Defaults for all projects'}</p>
        ${tabs.map((t) => `<button type="button" role="tab" aria-selected="${t === settings.tab}" aria-controls="settings-body" data-settings-tab="${t}" ${t === settings.tab ? '' : 'tabindex="-1"'}>${SETTINGS_TABS[t][0]}</button>`).join('')}</nav>
      <div class="settings-main"><header><div><h3 class="title">${title}</h3><p>${help}</p></div><button class="icon-button" type="button" data-action="close-dialog" aria-label="Close">${icon('i-x')}</button></header>
        <div class="settings-body" id="settings-body" role="tabpanel">${settingsPanel()}</div>
        <footer class="settings-foot"><button class="button ghost" type="button" data-settings-restore>Restore defaults</button><button class="button primary" type="button" data-settings-save>Save settings</button></footer></div></div>`
}

function openSettings(projectId = null, tab) {
  settings.projectId = projectId
  const p = project(projectId)
  settings.tab = tab ?? (p ? 'updates' : 'general')
  settings.policyTarget = p ? (p.solution ? 'solution' : 'repo') : 'global'
  settings.policyType = p?.solution ? 'nuget' : 'npm'
  openDialog({ raw: '<div></div>', size: 'settings' })
  $('#dialog').dataset.mode = 'settings'
  renderSettings()
  $(`#dialog [data-settings-tab="${settings.tab}"]`).focus()
}

/* ---------- Events ---------- */

document.addEventListener('click', (e) => {
  const t = e.target
  if (openMenuState && !t.closest('.menu') && !t.closest('[aria-haspopup="menu"]')) closeMenu(false)

  const filter = t.closest('[data-filter]')
  if (filter) {
    if (openMenuState?.trigger === filter) closeMenu()
    else openFilterMenu(filter)
    return
  }
  const typeItem = t.closest('[data-type]')
  const ecoItem = t.closest('[data-eco]')
  const riskItem = t.closest('[data-risk]')
  if (typeItem || ecoItem || riskItem) {
    const trigger = openMenuState?.trigger
    const kind = trigger?.dataset.filter
    if (typeItem) toggleSetValue(state.projectTypes, typeItem.dataset.type)
    if (ecoItem) toggleSetValue(state.ecosystems, ecoItem.dataset.eco)
    if (riskItem) state.risk = riskItem.dataset.risk
    closeMenu(false)
    render()
    const again = $(`[data-filter="${kind}"]`)
    if (riskItem) again?.focus()
    else if (again) { openFilterMenu(again); const sel = typeItem ? `[data-type="${typeItem.dataset.type}"]` : `[data-eco="${ecoItem.dataset.eco}"]`; $(`#filter-menu ${sel}`)?.focus() }
    return
  }

  const proj = t.closest('[data-project]')
  if (proj && proj.classList.contains('project-item')) return selectProject(proj.dataset.project)

  const stage = t.closest('[data-stage]')
  if (stage) return toggleRow(stage.dataset.stage)
  const unstage = t.closest('[data-unstage]')
  if (unstage) {
    const removed = [...state.staged].filter((k) => k.startsWith(`${unstage.dataset.unstage}|`))
    removed.forEach((k) => state.staged.delete(k))
    render()
    return toast(`${unstage.dataset.unstage} removed from the selection.`, { undo: () => { removed.forEach((k) => state.staged.add(k)); render() } })
  }
  const advisory = t.closest('[data-advisory]')
  if (advisory) return openAdvisory(advisory.dataset.advisory)

  const runItem = t.closest('[data-run]')
  if (runItem) {
    const option = runItem.dataset.run
    setRun(option)
    closeMenu(false)
    render()
    const trigger = $('[data-action="run-menu"]')
    if (option === 'commit') { openRunMenu(trigger); $('#run-menu [data-run="commit"]').focus() } else trigger.focus()
    return
  }

  const ctx = t.closest('[data-context]')
  if (ctx) {
    const pid = $('#context-menu').dataset.project
    const p = project(pid)
    closeMenu(false)
    const a = ctx.dataset.context
    if (a === 'open') toast(`Opened ${p.path} in File Explorer.`)
    if (a === 'settings') openSettings(pid)
    if (a === 'prepare') prepareCompatible(pid)
    if (a === 'exclude') openExclude(pid)
    if (a === 'include') {
      state.excluded.delete(pid)
      state.exclusions = state.exclusions.filter((r) => !r.id.startsWith(`${pid}-`))
      render()
      toast(`${p.name} will be checked again.`, { tone: 'success' })
    }
    return
  }

  const action = t.closest('[data-action]')?.dataset.action
  if (!action) return
  const el = t.closest('[data-action]')
  switch (action) {
    case 'all-projects': selectProject(null); $('#all-projects').focus(); break
    case 'open-manager': closeMenu(false); openManager(); break
    case 'add-root': closeMenu(false); openAddRoot($('#dialog-backdrop').hidden ? null : $('#dialog').dataset.mode); break
    case 'root-back': returnFromAddRoot(); break
    case 'open-exclusions': closeMenu(false); openSettings(null, 'scanning'); break
    case 'toggle-security': state.risk = state.risk === 'security' ? 'any' : 'security'; render(); break
    case 'prepare-security': prepareSecurity(); break
    case 'clear-filters': $('#search').value = ''; state.ecosystems.clear(); state.projectTypes.clear(); state.risk = 'any'; render(); break
    case 'clear-staged': {
      const removed = [...state.staged]
      state.staged.clear()
      render()
      toast(`${plural(packagesIn(removed), 'update')} removed from the selection.`, { undo: () => { removed.forEach((k) => state.staged.add(k)); render() } })
      break
    }
    case 'preview': openPreview(); break
    case 'update': openUpdate(); break
    case 'run-menu': if (openMenuState?.trigger === el) closeMenu(); else openRunMenu(el); break
    case 'run-update': runUpdate(); break
    case 'commit': openCommit(el.dataset.projects.split(',')); break
    case 'confirm-commit': closeDialog(); toast(`Committed to ${plural(Number(el.dataset.count), 'repository', 'repositories')}. Nothing was pushed.`, { tone: 'success' }); break
    case 'copy-commands': toast('Commands copied.', { tone: 'success' }); break
    case 'prepare-advisory': closeDialog(); prepareSecurity([el.dataset.name]); break
    case 'confirm-exclude': {
      const kind = $('#dialog input[name="exclude-kind"]:checked').value
      closeDialog()
      excludeProject(el.dataset.project, kind)
      break
    }
    case 'browse-root': toast('The folder picker opens here in the app.'); break
    case 'confirm-root': {
      const path = $('#root-path').value.trim()
      if (!path) return $('#root-path').focus()
      state.roots.push({ id: `root-${Date.now()}`, path })
      returnFromAddRoot()
      render()
      toast(`Watching ${path}. No projects found there yet.`)
      break
    }
    case 'project-settings': openSettings(state.selected); break
    case 'project-menu': { const r = el.getBoundingClientRect(); showContextMenu(state.selected, r.right - 240, r.bottom + 4, el); break }
    case 'close-dialog': closeDialog(); break
  }
})

function toggleSetValue(set, value) {
  if (value === 'all') return set.clear()
  set.has(value) ? set.delete(value) : set.add(value)
}

$('#all-projects').addEventListener('click', () => selectProject(null))
$('#manage-trigger').addEventListener('click', (e) => {
  const trigger = e.currentTarget
  if (openMenuState?.trigger === trigger) closeMenu()
  else openMenu($('#manage-menu'), trigger)
})
$('#manage-trigger').addEventListener('keydown', (e) => {
  if (e.key === 'ArrowDown' && !openMenuState) { e.preventDefault(); e.stopPropagation(); openMenu($('#manage-menu'), e.currentTarget) }
})
$('#prepare-compatible').addEventListener('click', () => prepareCompatible())
$('#sort-risk').addEventListener('click', (e) => {
  state.riskFirst = !state.riskFirst
  e.currentTarget.setAttribute('aria-pressed', String(state.riskFirst))
  render()
})
$('#select-visible').addEventListener('change', () => {
  const pairs = visibleRows().flatMap((d) => scopeIds(d).map((pid) => key(d.name, pid)))
  const all = pairs.every((k) => state.staged.has(k))
  pairs.forEach((k) => (all ? state.staged.delete(k) : state.staged.add(k)))
  render()
})
$('#search').addEventListener('input', render)
$('#scan').addEventListener('click', () => runScan())
$('#open-settings').addEventListener('click', () => openSettings(null))
$('#toast-close').addEventListener('click', () => { $('#toast').hidden = true })
$('#toast-action').addEventListener('click', () => {
  toastUndo?.()
  toastUndo = null
  toast('Undone.')
})
$('#dialog-backdrop').addEventListener('mousedown', (e) => {
  if (e.target.id === 'dialog-backdrop' && $('#dialog .dialog-foot .note') === null) closeDialog()
})

document.addEventListener('contextmenu', (e) => {
  const item = e.target.closest('[data-project].project-item, [data-manager-project]')
  if (!item) return
  e.preventDefault()
  showContextMenu(item.dataset.project ?? item.dataset.managerProject, e.clientX, e.clientY, item.querySelector('button') ?? item)
})

document.addEventListener('keydown', (e) => {
  if (menuKeys(e)) return
  const dialogOpen = !$('#dialog-backdrop').hidden
  if (dialogOpen) {
    if (e.key === 'Escape' && $('#dialog .dialog-foot .note') === null) closeDialog()
    if (e.key === 'Tab') trapFocus(e)
    if (e.key === 'Enter' && e.target.id === 'root-path') $('[data-action="confirm-root"]').click()
    return
  }
  if ((e.ctrlKey || e.metaKey) && e.key.toLowerCase() === 'k') { e.preventDefault(); $('#search').focus(); $('#search').select() }
  if ((e.ctrlKey || e.metaKey) && e.key === 'Enter' && state.staged.size) { e.preventDefault(); openUpdate() }
  if (e.key === 'Escape' && e.target.id === 'search' && e.target.value) { e.target.value = ''; render() }
  if (e.key === 'ContextMenu' || (e.shiftKey && e.key === 'F10')) {
    const item = document.activeElement.closest?.('[data-project].project-item')
    if (item) { e.preventDefault(); const r = item.getBoundingClientRect(); showContextMenu(item.dataset.project, r.left + 40, r.top + 36, item) }
  }
  const check = e.target.closest?.('#dep-rows [data-stage]')
  if (check && (e.key === 'ArrowDown' || e.key === 'ArrowUp')) {
    e.preventDefault()
    const all = $$('#dep-rows [data-stage]')
    all[Math.max(0, Math.min(all.length - 1, all.indexOf(check) + (e.key === 'ArrowDown' ? 1 : -1)))].focus()
  }
})

/* Dialog-scoped events */
const dialogEl = $('#dialog')
dialogEl.addEventListener('click', (e) => {
  const t = e.target
  const mode = dialogEl.dataset.mode
  if (mode === 'manager') {
    const check = t.closest('[data-manager-check]')
    const all = t.closest('[data-manager-all]')
    const open = t.closest('[data-manager-open]')
    const more = t.closest('[data-manager-more]')
    const act = t.closest('[data-manager-action]')
    if (check) { toggleSetValue(manager.selected, check.dataset.managerCheck); renderManager(); $(`[data-manager-check="${check.dataset.managerCheck}"]`)?.focus(); return }
    const folderCheck = t.closest('[data-folder-check]')
    if (all || folderCheck) {
      const groups = managerGroups().filter((g) => !folderCheck || g.root.id === folderCheck.dataset.folderCheck)
      const ids = groups.flatMap((g) => g.shown.map((p) => p.id))
      const every = ids.every((id) => manager.selected.has(id))
      ids.forEach((id) => (every ? manager.selected.delete(id) : manager.selected.add(id)))
      renderManager()
      $(folderCheck ? `[data-folder-check="${folderCheck.dataset.folderCheck}"]` : '[data-manager-all]')?.focus()
      return
    }
    const rootScan = t.closest('[data-root-scan]')
    if (rootScan) return scanFolder(rootScan.dataset.rootScan)
    const rootRemove = t.closest('[data-root-remove]')
    if (rootRemove) { removeFolder(rootRemove.dataset.rootRemove); return $('[data-manager-query]')?.focus() }
    if (open) { closeDialog(); selectProject(open.dataset.managerOpen); $(`.project-item[data-project="${open.dataset.managerOpen}"]`)?.focus(); return }
    if (more) { const r = more.getBoundingClientRect(); showContextMenu(more.dataset.managerMore, r.right - 240, r.bottom + 4, more); return }
    if (act) {
      const selected = projects.filter((p) => manager.selected.has(p.id))
      if (act.dataset.managerAction === 'settings') return openSettings(selected[0].id)
      if (act.dataset.managerAction === 'scan') {
        selected.forEach((p) => manager.scanning.add(p.id))
        renderManager()
        setTimeout(() => { selected.forEach((p) => manager.scanning.delete(p.id)); renderManager(); toast(`Scanned ${plural(selected.length, 'project')}. Nothing new.`, { tone: 'success' }) }, 900)
      }
      if (act.dataset.managerAction === 'exclude') {
        const ids = selected.map((p) => p.id).filter((id) => !state.excluded.has(id))
        const rules = ids.map((id) => ({ id: `${id}-repository-bulk`, kind: 'repository', path: project(id).path }))
        const removed = [...state.staged].filter((k) => ids.includes(k.split('|')[1]))
        ids.forEach((id) => state.excluded.add(id))
        state.exclusions.push(...rules)
        removed.forEach((k) => state.staged.delete(k))
        manager.selected.clear()
        render()
        renderManager()
        toast(`${plural(ids.length, 'project')} excluded.`, { undo: () => { ids.forEach((id) => state.excluded.delete(id)); state.exclusions = state.exclusions.filter((r) => !rules.includes(r)); removed.forEach((k) => state.staged.add(k)); render(); renderManager() } })
      }
    }
    return
  }
  if (mode === 'settings') {
    const tab = t.closest('[data-settings-tab]')
    if (tab) { settings.tab = tab.dataset.settingsTab; renderSettings(); $(`[data-settings-tab="${settings.tab}"]`).focus(); return }
    const toggle = t.closest('[data-toggle]')
    if (toggle) { settings.toggles[toggle.dataset.toggle] = !settings.toggles[toggle.dataset.toggle]; toggle.setAttribute('aria-checked', String(settings.toggles[toggle.dataset.toggle])); return }
    const { config } = currentPolicy()
    const refocus = (sel) => { renderSettings(); $(sel)?.focus() }
    if (t.closest('[data-policy-override]')) { config.override = !config.override; return refocus('[data-policy-override]') }
    if (t.closest('[data-policy-stop]')) { config.stopOnFailure = !config.stopOnFailure; return refocus('[data-policy-stop]') }
    if (t.closest('[data-policy-add]')) { config.override = true; config.validation.push(''); renderSettings(); return $$('[data-policy-command]').at(-1)?.focus() }
    const remove = t.closest('[data-policy-remove]')
    if (remove) { config.validation.splice(Number(remove.dataset.policyRemove), 1); return refocus('[data-policy-add]') }
    const rootScan = t.closest('[data-root-scan]')
    if (rootScan) { closeDialog(); return runScan(projects.filter((p) => p.root === rootScan.dataset.rootScan).map((p) => p.id)) }
    const rootRemove = t.closest('[data-root-remove]')
    if (rootRemove) return removeFolder(rootRemove.dataset.rootRemove)
    const ruleRemove = t.closest('[data-rule-remove]')
    if (ruleRemove) {
      const rule = state.exclusions.find((r) => r.id === ruleRemove.dataset.ruleRemove)
      state.exclusions = state.exclusions.filter((r) => r !== rule)
      const pid = projects.find((p) => rule.id.startsWith(`${p.id}-`))?.id
      if (pid) state.excluded.delete(pid)
      renderSettings()
      render()
      return toast(`${rule.path} will be checked again.`, { undo: () => { state.exclusions.push(rule); if (pid) state.excluded.add(pid); render(); if (dialogEl.dataset.mode === 'settings') renderSettings() } })
    }
    if (t.closest('[data-settings-save]')) { closeDialog(); return toast('Settings saved.', { tone: 'success' }) }
    if (t.closest('[data-settings-restore]')) {
      Object.assign(settings.toggles, { 'system-theme': false, compact: true, 'startup-scan': true, verification: true, 'pin-security': true, 'security-notify': true })
      renderSettings()
      return toast('Defaults restored. Save to keep them.')
    }
  }
})

dialogEl.addEventListener('keydown', (e) => {
  const tab = e.target.closest('[data-settings-tab]')
  if (tab && (e.key === 'ArrowDown' || e.key === 'ArrowUp')) {
    e.preventDefault()
    const tabs = $$('[data-settings-tab]')
    const next = tabs[(tabs.indexOf(tab) + (e.key === 'ArrowDown' ? 1 : tabs.length - 1)) % tabs.length]
    settings.tab = next.dataset.settingsTab
    renderSettings()
    $(`[data-settings-tab="${settings.tab}"]`).focus()
  }
})

dialogEl.addEventListener('input', (e) => {
  if (e.target.matches('[data-manager-query]')) {
    manager.query = e.target.value
    renderManager()
    return
  }
  if (dialogEl.dataset.mode !== 'settings') return
  const { config } = currentPolicy()
  const field = e.target.closest('[data-policy-field]')
  const command = e.target.closest('[data-policy-command]')
  if (field) config[field.dataset.policyField] = field.value
  if (command) config.validation[Number(command.dataset.policyCommand)] = command.value
})

dialogEl.addEventListener('change', (e) => {
  const runOption = e.target.closest('[data-run-option]')
  if (runOption) {
    const option = runOption.dataset.runOption
    if (option === 'checks') state.run.checks = runOption.checked
    else state.run.commit = runOption.checked
    saveRun()
    const scroll = $('#dialog .dialog-body').scrollTop
    openUpdate()
    $('#dialog .dialog-body').scrollTop = scroll
    $(`[data-run-option="${option}"]`).focus()
    render()
    return
  }
  if (e.target.id === 'palette-select') return applyPalette(e.target.value)
  if (e.target.id === 'policy-target') {
    settings.policyTarget = e.target.value
    const target = policyTargets().find((x) => x.value === settings.policyTarget)
    if (target?.kind !== 'auto') settings.policyType = target.kind
    renderSettings()
    $('#policy-target').focus()
  }
  if (e.target.id === 'policy-type') {
    settings.policyType = e.target.value
    renderSettings()
    $('#policy-type').focus()
  }
})

document.addEventListener('pointerover', (e) => { const t = e.target.closest('[data-tooltip]'); if (t) showTooltip(t) })
document.addEventListener('pointerout', (e) => { if (e.target.closest('[data-tooltip]')) hideTooltip() })
document.addEventListener('focusin', (e) => { const t = e.target.closest('[data-tooltip]'); if (t && t.matches(':focus-visible')) showTooltip(t); else hideTooltip() })

/* ---------- Theme and boot ---------- */

const params = new URLSearchParams(location.search)
let theme = params.get('theme') ?? (() => { try { return localStorage.getItem('mehen-mockup-theme') } catch { return null } })() ?? 'dark'

function applyTheme(next) {
  theme = next
  document.documentElement.dataset.theme = theme
  try { localStorage.setItem('mehen-mockup-theme', theme) } catch { /* private window */ }
  const button = $('#theme-toggle')
  const dark = theme === 'dark'
  button.querySelector('use').setAttribute('href', dark ? '#i-moon' : '#i-sun')
  button.querySelector('span').textContent = dark ? 'Dark' : 'Light'
  button.setAttribute('aria-label', `${dark ? 'Dark' : 'Light'} theme. Switch to ${dark ? 'light' : 'dark'}`)
}
$('#theme-toggle').addEventListener('click', () => applyTheme(theme === 'dark' ? 'light' : 'dark'))

let palette = params.get('palette') ?? (() => { try { return localStorage.getItem('mehen-mockup-palette') } catch { return null } })() ?? 'faience'
function applyPalette(next) {
  palette = ['faience', 'parchment'].includes(next) ? next : 'faience'
  document.documentElement.dataset.palette = palette
  try { localStorage.setItem('mehen-mockup-palette', palette) } catch { /* private window */ }
}
applyPalette(palette)

applyTheme(theme)
if (params.get('project') && project(params.get('project'))) state.selected = params.get('project')
if (params.get('search')) $('#search').value = params.get('search')
render()
if (params.get('advisory')) openAdvisory(params.get('advisory'))
if (params.get('manage') === '1') openManager()
if (params.get('update') === '1') openUpdate()
if (params.get('settings')) openSettings(params.get('project'), params.get('settings') === '1' ? undefined : params.get('settings'))
