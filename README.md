<p align="center">
  <img src="src-tauri/icons/128x128@2x.png" alt="Mehen" width="128" />
</p>

# Mehen

A desktop app that watches every project on your machine and keeps their dependencies current and safe.

Point it at a folder like `C:\code` and Mehen finds every npm, Cargo, NuGet and GitHub Actions project inside, checks each package against its registry and the [OSV](https://osv.dev) vulnerability database, and shows you three things no single-project tool can:

- **What is out of date**, across every project at once
- **Where your projects have drifted apart**: the same package on different versions in different repos
- **Which known vulnerabilities affect you**, where, and which version fixes them

Then it updates them for you: it shows the exact file changes first, runs the install and a build check, and puts everything back if anything fails.

> Named after Mehen, the Egyptian serpent god who coils around the sun god Ra's boat every night and fights off the chaos serpent Apep. The threat returns every night, and every night it is beaten back. That is dependency maintenance.

## Status

Early and moving fast. Windows is the primary target; the engine is cross-platform.

## What it does

### Find

- Walks one or more **watched folders**, skipping `node_modules`, `target`, `bin`, `obj`, build output and linked git worktrees, so nothing is counted twice
- Reads:

  | Ecosystem | Manifests | Installed versions from |
  |---|---|---|
  | npm | `package.json` | `node_modules`, `package-lock.json`, `pnpm-lock.yaml`, `bun.lock` |
  | Cargo | `Cargo.toml` (members, workspaces, target-specific tables) | `Cargo.lock` |
  | NuGet | SDK-style `.csproj`/`.fsproj`/`.vbproj`, `packages.config` (.NET Framework), `Directory.Packages.props` | the manifest (exact versions) |
  | GitHub Actions | `.github/workflows/*.yml`, `action.yml` | the ref itself; commit-pinned actions are resolved to their tag |

- **Ignore rules** keep out what you don't care about: a whole folder or repo, a single project, or a name pattern like `temp` or `fixtures` that matches anywhere. Set them before a check (the Folders panel lists every project found, with checkboxes) or after one (Ignore project / Ignore repo on any result, with Undo).

### Check

- Latest versions from npm, the crates.io sparse index, the NuGet registration API, and `git ls-remote` for Actions (no GitHub API rate limit)
- A **safe** target alongside the latest: the newest release on the current major line (or minor line for `0.x`), where breaking changes are unlikely
- **Only versions the project can use** are offered. .NET: the version must ship for the project's target framework (`net8.0` is offered EF Core 9, with 10 marked "only supports net10.0"). Cargo: the crate's `rust-version` must fit the project's (or your installed Rust). npm: its `engines.node` must accept the Node the project runs on (`.nvmrc`, or your installed Node when it fits `engines`). A version already in use is never filtered out, so Mehen never suggests a downgrade
- Vulnerabilities from OSV, with severity, summary and fixed versions
- Versions guessed from a range (nothing installed or locked) are marked `~` so a match against them is flagged as possible, not certain

### Update

- **Per project**: tick outdated packages, choose Safe or Latest for each, review, apply
- **Across projects**: in the Packages view, bring a package to one version (the latest, or one already in use) in every project that is behind
- Edits keep the author's style:
  - npm keeps `^` and `~`
  - Cargo keeps short versions (`"0.13"`), inline tables and comments
  - NuGet handles either attribute order, `<Version>` child elements, central package versions, and `packages.config` HintPaths
  - Actions keep their pin style: `@v4` becomes `@v7`, and a commit pin becomes the new tag's commit with a `# v7.0.1` comment
- Then runs an install for the right tool (npm/pnpm/yarn/bun chosen by lockfile, `cargo update -p` for only the chosen crates, `dotnet restore`) and optionally a build check (`npm run build`, `cargo check`, `dotnet build`)
- **Rollback**: the manifest and lockfile are saved first. If any step fails they are restored byte for byte. If the files changed since you reviewed the update, nothing is written.
- **Optional git commit** of exactly the files the update touched (default message `chore(deps): update X to Y`, editable), on the current branch. Other staged work is left alone, hooks run, nothing is pushed. Not offered when those files already had uncommitted edits.

### Cache

Every network answer is kept in a local SQLite database so repeat checks only ask for what is stale:

| Answer | Kept for |
|---|---|
| Latest version, version list and what each version needs | 6 hours |
| Vulnerability matches | 12 hours |
| Advisory details | 7 days |
| Package not found (404) | 24 hours |

A warm check of ~170 projects and ~740 packages takes about 4 seconds. **Refresh all** ignores the cache. A registry that keeps answering "too many requests" is skipped for the rest of the run instead of stalling it.

The database lives at `%APPDATA%\dev.mehen.app\mehen.db` and also holds your watched folders, ignore rules and the last 30 results.

## How it's built

```
crates/mehen-core    The engine (Rust). Scan, check, cache, plan and apply updates.
                     No UI code, so other front ends can reuse it.
src-tauri            The desktop shell (Tauri 2). Thin commands over mehen-core.
src                  The UI (React 19, Tailwind 4, Vite).
```

The engine is deliberately separate: a VS Code extension (the successor to [nuget-compass](../nuget-compass)) can drive the same code later as a background process.

Key modules in `mehen-core`:

| Module | Job |
|---|---|
| `scan.rs` | Walks folders, reads manifests, applies ignore rules, `discover()` for the no-network preview |
| `lockfiles.rs` | Installed npm versions from package-lock, pnpm and bun lockfiles |
| `registry.rs` | Latest versions and version lists per ecosystem, with retry and backoff |
| `osv.rs` | OSV batch queries and advisory details |
| `check.rs` | Works out current, latest, safe and status for every dependency, through the cache |
| `store.rs` | SQLite: cache, watched folders, ignore rules, saved results |
| `ignore.rs` | Folder, project and pattern rules |
| `update.rs` | Update plans (edits + steps), apply, rollback |
| `compat.rs` | Which versions a project can use: .NET target frameworks (ported from nuget-compass), Rust `rust-version`, npm `engines.node` |
| `version.rs` | Lenient version parsing that copes with NuGet four-part versions and `v4` tags |

## Development

Requirements: Rust 1.85+ (edition 2024), Node 20+, and for the desktop app the [Tauri prerequisites](https://tauri.app/start/prerequisites/). `git` must be on PATH for Actions lookups.

```bash
npm install
npm run tauri-dev        # desktop app with hot reload
npm run dev              # UI only, in a browser (see below)
cargo test --workspace   # engine tests
npx tsc -b               # typecheck the UI
```

**Working on the UI in a browser.** Outside Tauri, `src/api.ts` falls back to a stand-in that reads `public/dev-inventory.json`. Generate it from a real scan:

```bash
cargo run -p mehen-core --example survey -- C:\code --json public/dev-inventory.json
```

**Engine harnesses** (in `crates/mehen-core/examples`, using a separate dev database):

| Example | What it does |
|---|---|
| `survey` | Full scan and check of a folder, prints a summary |
| `plan_preview` | Plans (never applies) an update for one real project per ecosystem and prints the diffs |
| `bulk_preview` | Plans moving one package to one version across every project behind it |
| `apply_smoke` | Applies a real update to a throwaway npm project in the temp folder, once passing and once failing, to prove rollback |

## Roadmap

- Optional branch per update
- Private NuGet feeds and authenticated registries
- Tray icon with nightly background checks and notifications for new advisories
- Release notes and breaking-change summaries between your version and the target
- VS Code extension on the same engine

## License

MIT
