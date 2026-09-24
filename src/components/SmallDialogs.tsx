import { EyeOff, FolderPlus } from 'lucide-react'
import { useState } from 'react'
import * as api from '../api'
import { ECOSYSTEM_LABEL, folderName, relativePath, type Repo } from '../derive'
import type { IgnoreKind } from '../types'
import { cx } from './bits'
import { Button, Dialog } from './Dialog'
import { TextInput } from './controls'

/** Picks a folder to watch; the new folder is scanned straight away. */
export function AddFolderDialog({ onAdd, onClose }: { onAdd: (path: string) => void; onClose: () => void }) {
  const [path, setPath] = useState('')
  const add = () => path.trim() && onAdd(path.trim())
  return (
    <Dialog
      title="Add a folder to scan"
      description="Mehen looks for npm, Cargo, NuGet, Go, Python, Dart and Flutter, PHP, Ruby, and GitHub Actions projects anywhere under this folder. Your exclusions still apply."
      icon={<FolderPlus size={22} />}
      onClose={onClose}
      onEnter={add}
      footer={
        <>
          <Button onClick={onClose}>Cancel</Button>
          <Button variant="primary" onClick={add} disabled={!path.trim()}>
            <FolderPlus size={15} />
            Add and scan
          </Button>
        </>
      }
    >
      <label className="text-[12.5px] font-semibold" htmlFor="add-folder-path">
        Folder
      </label>
      <div className="mt-1.5 flex gap-2">
        <TextInput
          id="add-folder-path"
          mono
          autoFocus
          value={path}
          onChange={(e) => setPath(e.target.value)}
          placeholder="C:\code"
          spellCheck={false}
          className="flex-1"
        />
        <Button
          onClick={async () => {
            const picked = await api.pickFolder()
            if (picked) setPath(picked)
          }}
        >
          Browse…
        </Button>
      </div>
    </Dialog>
  )
}

interface Choice {
  id: string
  kind: IgnoreKind
  value: string
  title: string
  help: string
  shown: string
}

/** Excludes a repository, one of its manifests, or every folder with its name. */
export function ExcludeDialog({ repo, roots, onExclude, onClose }: { repo: Repo; roots: string[]; onExclude: (kind: IgnoreKind, value: string, label: string) => void; onClose: () => void }) {
  const name = folderName(repo.key)
  const choices: Choice[] = [
    {
      id: 'repo',
      kind: 'folder',
      value: repo.key,
      title: 'The whole project',
      help: 'Skips every manifest in it, including ones added later.',
      shown: relativePath(roots, repo.key),
    },
    ...(repo.projects.length > 1
      ? repo.projects.map((p) => ({
          id: p.id,
          kind: 'project' as const,
          value: p.manifest,
          title: `Only ${p.name}`,
          help: `The ${ECOSYSTEM_LABEL[p.ecosystem]} manifest. The rest of the project is still checked.`,
          shown: relativePath([repo.key], p.manifest),
        }))
      : []),
    {
      id: 'pattern',
      kind: 'pattern',
      value: name,
      title: `Every folder named ${name}`,
      help: 'Also skips folders with this name inside other projects.',
      shown: name,
    },
  ]
  const [picked, setPicked] = useState(choices[0].id)
  const choice = choices.find((c) => c.id === picked)!
  const confirm = () => onExclude(choice.kind, choice.value, choice.id === 'repo' ? repo.name : choice.title.replace(/^Only /, ''))

  return (
    <Dialog
      title={`Exclude ${repo.name}`}
      description="Excluded projects are skipped when checking and never updated. You can bring them back under Settings, Scanning."
      icon={<EyeOff size={22} />}
      onClose={onClose}
      onEnter={confirm}
      footer={
        <>
          <Button onClick={onClose}>Cancel</Button>
          <Button variant="danger" onClick={confirm}>
            Exclude
          </Button>
        </>
      }
    >
      <div role="radiogroup" aria-label="What to exclude" className="grid gap-2">
        {choices.map((c) => (
          <label
            key={c.id}
            className={cx(
              'grid cursor-pointer grid-cols-[18px_1fr] items-start gap-2.5 rounded-[3px] border bg-paper px-3 py-2.5',
              picked === c.id ? 'border-state shadow-[inset_0_0_0_1px_var(--state)]' : 'border-line',
            )}
          >
            <input type="radio" name="exclude" checked={picked === c.id} onChange={() => setPicked(c.id)} className="mt-0.5 accent-[var(--check)]" />
            <span className="min-w-0">
              <b className="block text-[13px]">{c.title}</b>
              <small className="block text-[12px] leading-snug text-muted">{c.help}</small>
              <code className="mt-0.5 block font-mono text-[12px] [overflow-wrap:anywhere] text-faint">{c.shown}</code>
            </span>
          </label>
        ))}
      </div>
    </Dialog>
  )
}
