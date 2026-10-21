# steamdeck-ports-mgr

A fork of [Ports Launcher](https://github.com/Nyaldee/Ports-Launcher) by Nyaldee, aimed at the Steam
Deck: installing, updating and backing up native PC ports, and adding them to Steam.

Ports Launcher is licensed GPL-3.0-or-later, and so is this fork. The original copyright notices are
kept. This file is the notice of modification that GPL-3.0 section 5(a) asks for.

## How this fork differs from upstream

New features go in new files where possible, so upstream changes keep merging cleanly. Every edit
to a file that came from upstream is listed here with its date.

| Date | Upstream file | Change |
|---|---|---|
| 2026-10-20 | `src/core/config.rs` | Added tests that pin how local entries with a GitHub or GitLab source load |
| 2026-10-20 | `src/core/version.rs` | Self-update check and project links point at this fork instead of upstream |
| 2026-10-20 | `README.md` | Note at the top crediting Nyaldee and listing what this fork adds |

Files added by this fork:

| Date | File | Purpose |
|---|---|---|
| 2026-10-20 | `.gitignore` | Keeps build output, game files, keys and firmware out of the repo |
| 2026-10-20 | `.github/workflows/ci.yml` | Builds the Linux x86_64 release and runs the tests on every push; publishes the rolling release from `main` |
| 2026-10-20 | `.github/workflows/upstream-sync.yml` | Daily merge of upstream changes (see below) |
| 2026-10-20 | `FORK.md` | This file |
| 2026-10-20 | `STEAM_DECK.md` | How to install and test this fork on a Steam Deck |
| 2026-10-20 | `tools/deck-discovery.sh` | Read-only script a user runs once on a Steam Deck to inventory installed ports, save folders, Steam shortcuts and tooling (writes only its own report file) |
| 2026-10-20 | `tools/fork-updater.sh` | The fork's updater, shipped as `ports_launcher_updater.sh` (see Builds) |

## What still comes from upstream at runtime

- **The catalog.** The app downloads `ports.json` and `themes.json` from upstream's repository about
  every 12 hours (`src/core/catalog_sync.rs`), so the catalog on a device matches upstream's, not
  this repo's copy. Personal entries belong in `ports.local.json` next to the executable.
- **Self-update** checks this fork's releases, not upstream's, and installs this fork's builds.

## Staying in step with upstream

`upstream-sync.yml` runs daily on `main` and can be started by hand from the Actions tab.

1. It merges upstream `main` into an `upstream-sync` branch.
2. It runs the full CI build and tests on that branch.
3. If upstream only touched the catalog (`ports.json`, `themes.json`, `lang/`) and the tests pass,
   the merge lands on `main` directly.
4. Anything else opens or refreshes a pull request listing the upstream commits, the files changed,
   and which of those files this fork has also changed.
5. A merge conflict, or upstream history that shares nothing with this fork's, fails the run with a
   summary.

Notes:
- Opening PRs needs Settings > Actions > General > Workflow permissions > "Allow GitHub Actions to
  create and approve pull requests".
- Pushes and PRs made by the workflow do not trigger CI on their own. The test result is in the PR
  body and the run summary.
- While a sync PR is open and already has the newest upstream commit, the workflow leaves its
  branch alone. If upstream moves again, or no PR is open, the branch is rebuilt, so put fix-ups on
  a separate branch.
- To test without touching `main`, start it by hand with `base_branch` set to a scratch branch.

## Builds

Every push builds `steamdeck-ports-mgr-linux-x86_64.tar.gz`, available as an artifact on the Actions
run. It holds the program, `7zzs`, a starter `ports.json` and `themes.json`, and the fork's updater.

Each push to `main` that passes CI (doc-only pushes included) also updates the **`rolling`**
release in place: the tag moves to the build's commit and the assets are replaced with the build and
its `.sha256`. It is a full release, so `releases/latest` points at it, and its notes name the exact
source commit; `BUILD_INFO` inside the package records the commit and build date too. A build whose
UTC date rolled over before upload is not published, because the app would offer it as an update
every day. Pushes made by workflows, such as catalog-only upstream syncs, do not trigger CI, so they
do not produce a new release.

The updater (`tools/fork-updater.sh`, shipped under upstream's file name so the app's Update and
Force update buttons run it) downloads the rolling release, refuses it unless the SHA-256 matches,
keeps the previous program files in `.previous/`, replaces only `ports_launcher`, `7zzs`, itself and
`BUILD_INFO`, and logs to `updater.log`. It only stops and restarts the copy of the program in its
own folder. On any failure it changes nothing, writes `update-failed.txt`, and reopens the version
that was running. Upstream's updater scripts stay in the repo unchanged but are not shipped,
because they install upstream's release.
