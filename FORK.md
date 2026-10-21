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
| 2026-10-20 | `README.md` | Item 8 (Deck-ready list) set to "In progress" |
| 2026-10-20 | `src/main.rs` | The tray is skipped when `tray_host_available` says there is no tray host (Game Mode), and Close then quits instead of hiding the window behind a tray that isn't there (`close_action`, tested). Slint itself never reports a tray that failed to start, so the check is ours. Controller input is allowed by `gamepad_input_allowed`, asked for the main window and any open dialog. A login autostart that beats the tray host now gets no tray, and Close quits |
| 2026-10-20 | `src/app/dialogs.rs` | `DialogSlot::window` made `pub(crate)` so the controller check can see the open dialog |
| 2026-10-20 | `src/ui/linux_chrome.rs` | `gamepad_input_allowed` replaces `foreground_window_belongs_to_us`; when X11 can't say who is in front (native Wayland, gamescope, an active window with no PID) it allows input if any of our windows has focus. `tray_host_available` returns false under gamescope, else asks `busctl` (capped by `timeout 3`) for a StatusNotifier host; with no answer, only a named desktop gets a tray. Tests for each decision |
| 2026-10-20 | `src/ui/windows_chrome.rs` | Same two functions, keeping upstream's Windows behavior |
| 2026-10-20 | `src/core/gamepad.rs` | Reads every connected controller and merges their state, instead of only the first; tests |
| 2026-10-21 | `src/main.rs` | One line, `mod fork;`, to include the fork's own modules (`src/fork/`) |
| 2026-10-21 | `README.md` | Item 6 (save backups before every update) set to "Testing" |
| 2026-10-21 | `src/core/mod.rs` | One line registering the test-only module `contract_tests` |
| 2026-10-21 | `src/core/jobs.rs` | The update check passes the port's `preferred_asset` to the release lookup (`update_available`); 3 tests |
| 2026-10-21 | `src/core/github_api.rs`, `src/core/gitlab_api.rs` | New `fetch_latest_tag_and_date_preferring`; the old function now calls it with no preference. GitHub also gets the pure `latest_tag_and_date` |
| 2026-10-21 | `src/core/platform_resolve.rs` | A leading `~` in a path expands to the home folder (`HOME`, `USERPROFILE` on Windows); 5 tests |
| 2026-10-21 | `src/core/installer.rs` | After an install, older AppImage files of the same product are removed; `follow_favorite_exe`; `apply_archive` backs up saves before extracting; tests |
| 2026-10-21 | `src/core/save_backup.rs` | New `backup_before_update` (zip of the save folders under `Update Backups`), reusing the existing copy and zip helpers |
| 2026-10-21 | `.gitattributes` | `*.sh text eol=lf` and `*.py text eol=lf`, so these scripts get Unix line endings in a fresh Windows checkout too (a CRLF `#!/bin/sh` line fails to run on Linux). The repository's stored files are unchanged. An existing checkout keeps its CRLF copies until they are checked out again (delete them, then `git checkout -- .`) |
| 2026-10-21 | `src/app/events.rs` | After an install finishes, a favorite executable that was an AppImage follows the new file (8 lines); a failed install no longer leaves the Play-launch pending and launches the installed game (6 lines) |
| 2026-10-21 | `src/app/dialogs.rs` | `finish_dialog_open`, `open_confirm_dialog`, `set_picker_items` and `build_picker_dialog` made `pub(crate)` for the new screens; two Settings entries ("Add to Steam...", "Adopt installed ports...") added to the list and its menu match (English text, not in the translation tables) |
| 2026-10-21 | `src/app/mod.rs` | One line registering `steam_screens` |
| 2026-10-21 | `src/core/installer.rs` | `refuse_adopted`: `install_port` and `install_extra` stop with a message when `Library/<folder>` is a symlink (an adopted port), whatever the catalog says, so a bad edit of `ports.local.json` cannot make the installer write into the user's own folder; 1 test |
| 2026-10-21 | `README.md` | Item 4 (adopt) set to "Testing", item 5 (Steam shortcuts) to "In progress" |

Files added by this fork:

| Date | File | Purpose |
|---|---|---|
| 2026-10-20 | `.gitignore` | Keeps build output, game files, keys and firmware out of the repo |
| 2026-10-20 | `.github/workflows/ci.yml` | Builds the Linux x86_64 release and runs the tests on every push; publishes the rolling release from `main`. Since 2026-10-21 also test-builds and runs the tests on Windows (upstream's other platform), without gating the release; skipped inside upstream-sync |
| 2026-10-20 | `.github/workflows/upstream-sync.yml` | Daily merge of upstream changes (see below) |
| 2026-10-20 | `FORK.md` | This file |
| 2026-10-21 | `src/core/contract_tests.rs` | Test-only module that pins the upstream behavior our features rely on (asset choice for our ports' real release names, update decision with a recycled tag, local-entry merge, `state.json` keys) |
| 2026-10-20 | `STEAM_DECK.md` | How to install and test this fork on a Steam Deck |
| 2026-10-20 | `tools/deck-discovery.sh` | Read-only script a user runs once on a Steam Deck to inventory installed ports, save folders, Steam shortcuts and tooling (writes only its own report file) |
| 2026-10-20 | `tools/fork-updater.sh` | The fork's updater, shipped as `ports_launcher_updater.sh` (see Builds) |
| 2026-10-21 | `tools/test-fork-updater.sh` | 22 checks of the updater against fake installs and releases; CI runs it |
| 2026-10-20 | `tools/smoke-test.sh` | Starts the packaged app on a virtual display with no tray host or desktop session; checks it survives 15 s without a panic, skips the tray, maps a non-blank window, and quits on Close. CI runs it and blocks the release on failure |
| 2026-10-20 | `tools/deck_ready_index.py` | Builds the Deck-ready list (see below) |
| 2026-10-20 | `tools/test_deck_ready_index.py` | Offline tests for the builder |
| 2026-10-20 | `.github/workflows/deck-ready-index.yml` | Weekly refresh of the Deck-ready list |
| 2026-10-20 | `index/deck-ready.json` | The published Deck-ready list. Seeded by a local run of the same builder, then refreshed only by that workflow |
| 2026-10-21 | `src/fork/mod.rs` | Module root for code this fork adds (the Steam shortcuts library, the `src/fork/` files). The Settings screens call it |
| 2026-10-21 | `src/fork/vdf.rs` | Lossless codec for Steam's binary KeyValues format (`shortcuts.vdf`) |
| 2026-10-21 | `src/fork/app_id.rs` | CRC-32, non-Steam shortcut app IDs, 64-bit grid IDs, signed/unsigned forms |
| 2026-10-21 | `src/fork/shortcuts.rs` | List, add and update-in-place (app ID kept) shortcuts in a parsed file |
| 2026-10-21 | `src/fork/store.rs` | Load and save `shortcuts.vdf`: refuses while Steam runs, backup first, atomic write, read-back check |
| 2026-10-21 | `src/fork/artwork.rs` | Artwork file names and placement in `config/grid/`, plus Steam folder layout helpers |
| 2026-10-21 | `src/fork/art/fit.rs` | Fits any image into a Steam art slot: portrait 600x900 (contain over a blurred copy), wide 920x430 and hero 1920x620 (cover-crop), logo inside 1280x720 (alpha kept), icon 512x512; refuses sources needing more than 1.5x enlargement |
| 2026-10-21 | `src/fork/art/grid.rs` | Writes slot images into `config/grid/` through `place_artwork`; provenance manifest `.dpm-art-manifest.json`; never overwrites art the user chose; writes the logo position file once |
| 2026-10-21 | `src/fork/art/catalog.rs` | Turns a catalog entry's cached `image` and `icon` into the portrait and icon slots |
| 2026-10-21 | `src/fork/steam_user.rs` | Reads `config/loginusers.vdf` (text KeyValues), lists the accounts that have a `userdata` folder and picks the saved one, else `MostRecent`, else the only one |
| 2026-10-21 | `src/fork/backups.rs` | Prunes `shortcuts.vdf.bak-<stamp>` files: newest N kept (at least 1), nothing under an hour old, nothing else in the folder |
| 2026-10-21 | `src/fork/settings.rs` | `fork-settings.json` (next to `state.json`): chosen Steam account, extra adopt scan folders, backups to keep. Other keys survive a save |
| 2026-10-21 | `src/fork/adopt.rs` | Finds ports installed outside `Library/` and registers them (see "Adopting installed ports") |
| 2026-10-21 | `src/fork/steam_sync.rs` | Plans and applies the shortcuts for installed ports (add, update in place, up to date, blocked) |
| 2026-10-21 | `src/fork/screens.rs` | View models for the two screens: labels, toggling, review text, apply, adopt. All the decisions, tested without UI |
| 2026-10-21 | `src/app/steam_screens.rs` | The "Add to Steam" and "Adopt installed ports" list dialogs (thin: draws `screens.rs` text) |
| 2026-10-21 | `src/fork/rom_library/` | ROM library (library and tests only, no UI): scan a ROM folder (CRC-32, MD5, SHA-1, N64 byte-order normalization), read each port's required files from the catalog text or explicit `rom_files` in a local entry, match by checksum then name, plan and apply use-in-place / symlink / copy, record the mapping in its own `rom_library.json` |

## Add to Steam and adopting (Settings menu)

- **Add to Steam** writes one shortcut per selected installed port to the chosen account's
  `shortcuts.vdf`, from Desktop Mode with Steam closed. It goes through the library's safe save
  (refuse while Steam runs, verified backup, atomic write, read-back), then prunes old backups. The
  Steam route decision (Desktop Mode write or a Decky plugin) is still open; this is the Desktop Mode
  route and the library underneath serves either.
- **Matching:** an existing shortcut is the port's when it has the same program, else the same name
  (ignoring case). It is updated in place, so its app ID stays. Two matches, or an entry with no stored
  app ID that needs a change, are blocked and shown as skipped, never guessed.
- **A shortcut found by name only** (different program than today's) starts switched off and says what
  it points at now, so a hand-made shortcut of another program with the same name is not repointed by
  accident. A start folder differing only by a trailing slash is not a change.
- **Artwork:** for each shortcut written, the catalog portrait and icon the app has cached
  (`cache/<folder>.png`, `cache/<folder>_icon.png`, fetched in the background when the screen opens) go
  through `art::grid::write_art` into `config/grid/` before the write, and the icon path is stored on
  the shortcut when it has no icon of its own (an existing icon is kept). Art the user chose is never
  replaced (the writer's rule). Not done: art for a shortcut
  that is already in Steam, wide/hero/logo art.
- **Not done:** the collection, Game Mode queueing, repointing a moved port's shortcut on its own,
  Windows (no `/proc` check: it refuses).
- **Screen text is English only**, built from the upstream picker dialog.

## Adopting installed ports

Registering an install found outside `Library/` changes two things of ours and nothing in the install:
`Library/<catalog folder>` becomes a symlink to it (the app finds, launches and opens every port as
`Library/<folder>`), and a `user_managed` entry is added to `ports.local.json` (existing file backed
up first). The entry copies the catalog entry's name, image, tags and save paths but drops `source`
(and `extra`, `preferred_asset`): with a source the app would install and update through the link,
writing into the user's folder. Consequences: an adopted port is not checked for updates, and its state
key is its folder name instead of `owner/repo`. A failure halfway removes the link it made. Needs
Linux (symlinks) and a Library folder on a filesystem that allows them. Loose files (an AppImage with no
folder) are not adopted; only folders one level inside a scan folder.

## Steam shortcuts library (`src/fork/`)

The library underneath the Add to Steam screen. Rules it enforces so a later feature cannot break Steam:

- Saving `shortcuts.vdf` is refused while a Steam process (`steam`, `steam.sh`, `steamwebhelper`) exists, and when
  that check cannot run. Steam rewrites the file when it exits, so a write while it runs is lost.
- A timestamped backup (`shortcuts.vdf.bak-<local time stamp>`) is written and verified before any write.
- The write goes to a temp file in the same folder, is synced, then renamed over the original, and the
  result is read back. A failure leaves the original in place.
- Saving is refused if the file changed on disk since it was loaded.
- An entry's stored app ID is never recomputed. The CRC formula (the one Steam ROM Manager uses) is only
  used for new entries. Newer Steam builds may assign other IDs to shortcuts added in the Steam UI.
- Code in `src/fork/` uses English names and module docs; upstream's own tests are in French.
- Backups (`shortcuts.vdf.bak-<local time>`) are pruned only by `backups::prune_backups`, which Add to Steam calls after a write (newest 10, never anything under an hour old).

## ROM library (`src/fork/rom_library/`)

Library only; no UI and no caller yet. Rules it enforces:

- Originals are only ever opened for reading: never moved, renamed, deleted or modified.
- Nothing already in a port folder is overwritten. A different file at the destination refuses the plan;
  an identical file (or a link already pointing at the original) is accepted as connected and left alone.
  A copy is written under a temporary name, verified, and published with `hard_link` (which cannot
  replace anything), then the temporary file is removed.
- A copy is a planned action that reports its size and is refused by `apply` without `Approval::CopyAllowed`.
  A link that the file system refuses (exFAT) fails with `SymlinkUnsupported`; the fallback is a new,
  confirmed copy plan.
- Link and copy destinations are confined to the port folder (`safe_join`, plain file names only).
- Forgetting a connection removes the record and no files.
- State lives in `rom_library.json` and `rom_scan_cache.json` beside `state.json`, not in `state.json`:
  `StateManager::save` rewrites that file from a fixed field list, so a key added to it would be dropped.
- Matching: CRC-32 of the file as stored or, for N64, converted to big-endian (the form checksum lists use);
  then name (low-confidence flag); a name match whose checksum differs from the catalog's is "wrong version".
- Local entries can state requirements explicitly (`rom_files`, `rom_access`, `rom_dest`); the app's own
  parser drops unknown fields, so the library reads those from the raw `ports.local.json`.

Known limits: a ZIP is hashed as a file (a CRC of a ROM inside an archive is not matched); headered dumps
(SNES copier headers, iNES) hash differently from headerless No-Intro lists and show as "wrong version";
Switch dumps that are folders are not scanned; `rom_access` for Ship of Harkinian and 2 Ship 2 Harkinian
("use original") comes from the findings document and is not yet confirmed on a Deck.

## Fixes to upstream behavior

Four small changes fix gaps found while validating Tim's catalog entries. Each has tests that fail
when the change is reverted.

- **Update check and `preferred_asset`.** Upstream's daily check ignored a port's `preferred_asset`
  and looked at whichever asset the default rules pick, so a port like Severed Chains (the Steam Deck
  tarball has no "linux" in its name) was compared against an old release and offered an update every
  day. The check now looks at the same asset the installer picks.
- **`~` in save paths.** `~/.local/share/x` used to be joined onto the game folder. A leading `~` now
  means the home folder. `%HOME%` keeps working.
- **Old AppImages.** An AppImage is installed as a plain versioned file, so each update left the old
  one next to the new one, and a favorite executable kept launching the old file. After an install,
  AppImages in the port's top folder whose name differs from the new file only by version numbers are
  removed (a different name pattern, such as another architecture, is left alone), and a favorite that
  pointed at a removed file moves to the new one. Saves are never touched.
- **Saves backed up before every install.** Before an install writes into a port that has `save` or
  `save2` folders, those folders are zipped to
  `<Saves Backup>/Update Backups/<port folder>/<YYYY-MM-DD_HH-MM-SS>.zip` (a second one in the same
  second gets `-2`). If the backup cannot be made, the install stops and changes nothing, and a
  Play-triggered update that fails still launches the installed game (`src/app/events.rs`). Limits:
  backups are not pruned; a `save` that is the game folder itself (`"."`) or holds it is skipped,
  because it would copy the whole game; symlinked files inside a save folder and `save` paths that
  are files are not backed up; "Install extras" is not backed up.
- **Side effects to know.** The `~` fix also means the Save Backup button (global backup) now
  includes upstream's `~/...` Linux save paths (about 32 catalog entries), installed or not. AppImage
  cleanup only recognizes names that differ by version numbers, so a renamed scheme keeps the old file.

## Steam artwork pipeline, no-key half (`src/fork/art/`)

Library only; nothing calls it yet and nothing here downloads anything. Documented in `src/fork/art/mod.rs`.

- **Fit** (`fit.rs`): exact sizes, typed `FitError` (never a panic). A source that would be enlarged by more than 1.5x
  is refused (`TooSmall`). The logo is never enlarged or cropped and keeps its alpha; portrait, wide and hero come out
  opaque (transparency is flattened over black). Resizing premultiplies alpha so transparent edges do not fringe dark.
- **Icons:** catalog icons are 48, 128 or 256 px (most are 256), so a strict 512x512 rule with the 1.5x limit would
  refuse nearly all of them. `fit_icon_keep_size` keeps a square icon of 32 to 512 px at its own size; `fit(Slot::Icon)`
  is the strict 512x512 version for other sources.
- **Writer** (`grid.rs`): a slot is written only if no artwork exists for that ID and slot, or every existing file (any
  extension) is in the manifest with an unchanged content fingerprint. Anything else is the user's and is skipped, not
  overwritten or deleted. A corrupt manifest is renamed `.corrupt` and everything then counts as the user's.
- **Logo position file** `<id>.json` is created with `create_new` only when our logo is in place; an existing one is
  never touched. Unverified on a real Deck (research report: one community bug report).
- **Manifest limits:** a file is renamed into place and recorded afterwards. If the record cannot be saved the
  report carries `manifest_error` and the file counts as the user's from then on; a crash in that window does the
  same, with no repair step. A hard crash can also leave a `.dpm-art-XXXXXX` staging folder in `config/grid` that nothing sweeps (harmless). One `write_art` runs at a time inside the process; another process writing the same
  folder is not guarded against. `place_artwork` re-lists the folder before it removes other extensions of our own
  art, so a file Steam writes in that millisecond window could be removed.
- **Refused sources:** over 4096 px a side (reported as `Decode`), a portrait source wider than about 3:1, a logo
  under 64 px, anything needing more than 1.5x enlargement. EXIF rotation is not applied. The writer also fails a
  slot whose image is not the slot's size.
- **Not verified:** that Steam shows the art on a Deck (and whether it needs a restart), that Steam's `icon` field
  accepts a 256 px PNG path, and what Steam does with a slot left empty.

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

## The Deck-ready list

`index/deck-ready.json` lists ports whose latest release has a Steam Deck or Linux x86_64 build.
`deck-ready-index.yml` rebuilds it every Monday and can be started by hand.

- **Sources:** this repo's `ports.json`, maintainers' own `.recomp.json` files (GitHub code search)
  and GitHub topics such as `recomp` and `decomp`. No third-party port table is read or copied.
- **Ratings** come from release file names, not test launches. Treat them as a starting point.
- **Changes:** new projects are added and changed ones updated. A project that stops qualifying
  stays for one more cycle with a `missing_since` date, then is removed.
- The workflow commits to the branch it was started on (`main` for the weekly schedule) only when the file changed and lists what changed in the run
  summary. The builder refuses to publish a list that shrank by more than 30 percent.
- **Search limits:** GitHub's search quota for the Actions token is shared with other users, so a
  search can keep answering "try again later". When one does, that source keeps last week's
  projects and the run summary shows a warning. An optional repository secret
  `CODE_SEARCH_TOKEN` (a fine-grained token with read-only access to public repositories) gives
  the searches their own quota.
- **Reading the file:** key entries on `id` (names are not unique). `kind: game` is safe to show;
  `likely_game` came from topics alone and may include tools, so show it behind a toggle.
  `catalog_disabled` means upstream's catalog disables that entry. `archived`, `catalog_disabled`
  and `readme_mentions_deck` can be null when unknown. `generated` is the date the list last
  changed, not the date it was last checked.
- The app does not read this file yet.

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
