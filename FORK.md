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
| 2026-10-21 | `ui/app-window.slint` | The Deck-ready list: imports `ui/fork/deck-ready.slint`, `deck-ready-*` properties and callbacks, the list shown instead of the port list or card grid while `deck-ready-mode` is on, a "Deck-ready" / "All ports" footer button, the title and search placeholder for that list, an "L1 / R1  Deck-ready list" hint in the fullscreen title, Tab switches lists and Escape leaves it |
| 2026-10-21 | `src/app/state.rs` | `deck_ready` field; `refresh_current_view`, `move_selection` and `current_selected_port` hand over to the Deck-ready list while it is shown (3 guards) |
| 2026-10-21 | `src/app/install_launch.rs` | `activate_port` made `pub(crate)`; `activate_selection` and `show_info_for_current_selection` hand over to the Deck-ready list while it is shown |
| 2026-10-21 | `src/app/events.rs` | `AppEvent::DeckReadyFetched` and its one-line handler |
| 2026-10-21 | `src/app/dialogs.rs` | `DialogSlot::DeckReadyInfo` (two match arms) and `build_deck_ready_info_dialog`, sized and wired like Info |
| 2026-10-21 | `src/app/gamepad_target.rs` | The Deck-ready Info dialog in the three dialog matches; in the main window B leaves the Deck-ready list, Y opens its Filters while it is shown, L1/R1 switch lists |
| 2026-10-21 | `src/app/mod.rs` | One line registering `deck_ready_screen` |
| 2026-10-21 | `src/main.rs` | `deck_ready` field initialised; one line wiring the Deck-ready callbacks |
| 2026-10-21 | `src/core/gamepad.rs` | Reads L1 and R1 (`button_lb`, `button_rb`), merged across controllers like the other buttons |
| 2026-10-21 | `src/ui/gamepad_router.rs` | L1 and R1 map to a new `SwitchList` action (`GamepadTarget::switch_list`, no-op by default); tests |
| 2026-10-21 | `src/core/catalog_sync.rs` | A 304 now counts as "not modified": `fetch_if_changed` reads the status in its `Ok` arm through a new `classify(status)` (ureq 3 returns a 304 as `Ok`, only 4xx and 5xx are errors; the old `Err(StatusCode(304))` arm never fired). 8 tests (6 offline, see the next row), one of them an `#[ignore]` live ETag round trip |
| 2026-10-21 | `src/app/sync.rs` | Catalog and themes checks send the stored ETag only while the saved `ports.json` / `themes.json` exists and passes the same validation as a download (`etag_for_saved_copy`), and record the new ETag only when the file was written (`etag_to_record`); `catalog_sync.rs` gains the pure `interpret(status, ...)`, `validate_ports` / `validate_themes` and 6 offline tests |
| 2026-10-21 | `src/ui/slint_layout_lint.rs` | The new Slint file is linted too |
| 2026-10-21 | `src/core/installer.rs` | `.tar.xz`, `.txz`, `.tar.zst` and `.tzst` count as archives and unpack in two steps like `.tar.gz` (7-Zip reads xz and zstd); 1 test |
| 2026-10-21 | `src/core/asset_select.rs` | With `preferred_asset`, checksum and update files (`.sha256`, `.zsync`...) beside the match are dropped before deciding it is ambiguous; `.zsync` joins the non-archive list; 1 test |
| 2026-10-21 | `src/ui/font_metrics.rs` | Linux `wrapped_text_height` counts each line break as a new line (as Windows' `DrawTextW` does). It split the whole text on whitespace, so every multi-line message was measured as one paragraph and its dialog cut off the last lines; 1 test |
| 2026-10-21 | `README.md` | Item 8 (Deck-ready list) set to "Testing" |
| 2026-10-22 | `src/app/dialogs.rs` | One Settings entry, "Game files..." (list, labels and the menu match; English text) |
| 2026-10-22 | `src/app/events.rs` | After an install that is not followed by a launch, one call to `rom_screens::offer_after_install` (the once-per-port offer to connect game files) |
| 2026-10-22 | `src/app/mod.rs` | One line registering `rom_screens` |
| 2026-10-22 | `README.md` | Item 7 (game files folder) set to "Testing" |
| 2026-10-21 | `src/core/platform_resolve.rs` | O1: `save_location` / `SaveLocation`; on Linux a save value never uses a Windows OS path (drive, `%VAR%`, home, absolute) or any Android value, and a path with an unset `%VAR%` is `UnknownOnLinux`; a Windows value that is a plain folder inside the game (`is_in_game_relative`) counts on Linux; `resolve_save_folder` returns no folder for an unknown one; `expand_env_path` is now test-only (`expand_env_path_checked` reports unset variables); Windows unchanged; 6 tests |
| 2026-10-21 | `src/core/save_backup.rs` | O1: `unknown_on_linux_fields`, `save_folder_unknown_on_linux`; `GlobalBackupSummary::no_linux_folder` (installed ports concerned); `backup_before_update` logs it; `restore_after_install` keeps the Pending Restore copy when there is no folder to restore into (upstream deleted it); 5 tests |
| 2026-10-21 | `src/app/dialogs.rs`, `ui/dialogs/info.slint`, `src/app/events.rs` | O1: for an installed port, Info shows a `save-note-text` line ("Save folder: unknown on Linux, no backup", or "Save folder 2" / "Save folders"); the uninstall confirmation says saves are not kept; the Save Backup summary appends the names (`SaveBackupDone` carries them). O2: closing the file picker of a Play-triggered update launches the installed game |
| 2026-10-21 | `src/core/asset_select.rs` | O2/O3: `choose_release` (`ReleaseChoice::AskForNewest` when the newest release has several files for this system, instead of walking down), `prefer_stable` (drafts dropped, prereleases skipped unless none is stable), `is_draft`, `needs_latest_stable`, `newest_release`; `latest_installable_release` wraps it; 9 tests |
| 2026-10-21 | `src/core/github_api.rs`, `src/core/gitlab_api.rs` | O2/O3: `candidate_releases` (5 newest without drafts, plus `releases/latest` when all 5 are prereleases); `most_recent_release` returns the release `choose_release` asked about; the update check compares against the newest release even when it needs the picker, dated by its newest candidate file (`ask_for_newest_date`); `fetch_latest_tag_and_date` removed from both (no caller left); 2 tests |
| 2026-10-21 | `src/core/jobs.rs`, `src/core/contract_tests.rs` | O2: test renamed and re-asserted (`update_check_without_preferred_asset_compares_the_newest_release_not_an_older_one`); contract test `severed_chains_without_preferred_asset_walks_down_to_the_2023_zip` replaced by `..._asks_for_a_file_of_the_newest_release`, plus a main-release and a devbuild-prerelease test |
| 2026-10-21 | `src/app/sync.rs`, `src/app/events.rs`, `src/main.rs` | O4/O5: `settle_self_update_flag` at startup (clears a flag that no longer applies, then checks at once); the check compares commits through `fork::self_update`; `SelfUpdateAvailable` carries the offer, saved in `fork-settings.json`. O6: catalog and themes failures are logged and deferred (`PortsCheckFailed`, `ThemesCheckFailed`). O7: a 15-minute timer runs whichever check is due |
| 2026-10-21 | `src/core/state/throttles.rs` | O6: `defer_catalog_check`, `defer_themes_check` (back-date the last check so the next one is due after the backoff); O7: self-update interval 24 h -> 12 h; 1 test |
| 2026-10-21 | `src/core/catalog_sync.rs` | O6: `SyncError` with `Retry-After` (seconds or HTTP date), `retry_delay` (1 h default, 1 min to 1 day); 4xx/5xx read as responses (`http_status_as_error(false)`) so the header can be read; `fetch_with` takes the agent so 2 tests talk plain HTTP to a local fake (429 with `Retry-After`, 404); 4 tests |
| 2026-10-21 | `src/ui/linux_chrome.rs` | O8: the single-instance socket is named per install folder (`ports_launcher-<FNV-1a of the folder>.sock`); the old shared `ports_launcher.sock` gets the hand-over only when `/proc` shows its listener runs from this folder (a copy from before the fix); a hand-over prints a line; 5 tests |
| 2026-10-22 | `src/main.rs` | One line starting the Game Mode queue watch (`steam_screens::start_queue_watch`, a timer held for the app's life) |

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
| 2026-10-21 | `tools/test-fork-updater.sh` | 22 checks of the updater against fake installs and releases; CI runs it. A failed check prints the copy count, the processes under the test folder and the log tail (since 2026-10-21) |
| 2026-10-20 | `tools/smoke-test.sh` | Starts the packaged app on a virtual display with no tray host or desktop session; checks it survives 15 s without a panic, skips the tray, maps a non-blank window, opens the Deck-ready list and comes back (since 2026-10-21), and quits on Close. CI runs it and blocks the release on failure |
| 2026-10-21 | `tools/test-app-update.sh` | Offline in-app update test: the packaged app, started from an old copy (marker appended to the program, old `BUILD_INFO`), runs Settings > Force update from the UI against a fake release on `file://`; checks the swap, `.previous/`, the restart on the new file and that saves, catalogs and state survive, then a tampered-checksum case (nothing changes, `update-failed.txt`, app reopens). CI runs it after the smoke test and blocks the release on failure |
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
| 2026-10-21 | `src/fork/settings.rs` | `fork-settings.json` (next to `state.json`): chosen Steam account, extra adopt scan folders, backups to keep, the self-update offer (`get`/`set`). Other keys survive a save |
| 2026-10-21 | `src/fork/save_notes.rs` | Text for ports with no known Linux save folder (Info line, Save Backup summary) |
| 2026-10-21 | `src/fork/self_update.rs` | Self-update by commit: reads `BUILD_INFO` and the release notes' source commit, decides, and keeps what raised the flag |
| 2026-10-21 | `src/fork/adopt.rs` | Finds ports installed outside `Library/` and registers them (see "Adopting installed ports") |
| 2026-10-21 | `src/fork/steam_sync.rs` | Plans and applies the shortcuts for installed ports (add, update in place, up to date, blocked) |
| 2026-10-21 | `src/fork/screens.rs` | View models for the two screens: labels, toggling, review text, apply, adopt. All the decisions, tested without UI |
| 2026-10-21 | `src/app/steam_screens.rs` | The "Add to Steam" and "Adopt installed ports" list dialogs (thin: draws `screens.rs` text) |
| 2026-10-22 | `src/fork/steam_queue.rs` | Steam changes queued from Game Mode: `steam-queue.json` (versioned, atomic), applying it through the screen's safe write, the startup/30-second offer decision, the mode wording; tested without UI |
| 2026-10-21 | `src/fork/rom_library/` | ROM library (library and tests only, no UI): scan a ROM folder (CRC-32, MD5, SHA-1, N64 byte-order normalization), read each port's required files from the catalog text or explicit `rom_files` in a local entry, match by checksum then name, plan and apply use-in-place / symlink / copy, record the mapping in its own `rom_library.json` |
| 2026-10-22 | `src/fork/rom_screens/` | The logic of the game-files screens (R2): folder browser, background job runner, scan and report, results by system, Connect step, connected ports, the after-install offer; tested without UI |
| 2026-10-22 | `src/app/rom_screens.rs` | The screens themselves: list dialogs, a timer that polls the worker thread |
| 2026-10-21 | `src/fork/deck_ready/index.rs` | Reads `index/deck-ready.json` (schema 1); unusable entries are skipped, an unusable file is refused |
| 2026-10-21 | `src/fork/deck_ready/sync.rs` | Fetches the list from this repo with an ETag, at most every 12 hours, and keeps the saved copy in `cache/` |
| 2026-10-21 | `src/fork/deck_ready/view.rs` | Rows, filters, sorting, matching to ports the app already has, and the Info and confirm text; tested without UI |
| 2026-10-21 | `src/fork/deck_ready/entry.rs` | Turns an entry the app doesn't have into the user's own `ports.local.json` entry (backup first) |
| 2026-10-21 | `src/app/deck_ready_screen.rs` | The Deck-ready list screen: switching lists, Filters, Info, install and play |
| 2026-10-21 | `ui/fork/deck-ready.slint` | The Deck-ready list and its Info dialog |

## Add to Steam and adopting (Settings menu)

- **Add to Steam** writes one shortcut per selected installed port to the chosen account's
  `shortcuts.vdf`, from Desktop Mode with Steam closed. It goes through the library's safe save
  (refuse while Steam runs, verified backup, atomic write, read-back), then prunes old backups. This is
  the Desktop Mode route (chosen 2026-10-21; no Decky plugin).
- **Queue from Game Mode** (`steam_queue.rs`): Steam always runs in Game Mode, so with Steam up the
  review row becomes "Queue N changes for Desktop Mode...". The choice (port key, Steam account, whether
  a name-only match may be repointed, catalog art or none) is saved to `steam-queue.json` next to the app
  (`"version": 1`, temp file + fsync + rename; an empty queue removes the file). Re-queuing a port for the
  same account replaces its entry. Shortly after startup and then every 30 s, when no dialog is open and
  no game started from the app runs, a non-empty queue with Steam closed brings up "N Steam changes
  queued from Game Mode: apply now?"; a declined offer comes back only after Steam was seen running and
  closed again (or on the next start). The screen also has an "apply now" row and a "View or remove
  queued changes..." list (remove one, clear all, Back). Applying plans again from the current
  `shortcuts.vdf` and writes through `screens::apply_selected_with` (same Steam re-checks, backup,
  atomic write, read-back, `art::grid::write_art`), one write per account: an entry already in Steam
  counts as done, a port that left the catalog or was uninstalled, an unknown account, a blocked plan or
  a name-only match that was not approved when queuing is reported and dropped; if Steam comes back
  before the rename (or `shortcuts.vdf` cannot be read) that account's entries stay queued and the file
  is not rewritten. A queue file that cannot be read (bad JSON, not text, unknown version, a bad entry)
  is renamed to `steam-queue.json.corrupt-<time>` and the user is told; the queue starts empty. Game Mode
  is detected from `XDG_CURRENT_DESKTOP` containing "gamescope" or `GAMESCOPE_WAYLAND_DISPLAY` (unverified
  on a Deck) and only changes the wording. Not verified on a Deck: the whole flow; whether a Game Mode
  launch sees Steam in `/proc` (expected yes).
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
- **Not done:** the collection, repointing a moved port's shortcut on its own,
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

The library (R1). The screens that use it are described in "Game files screens" below. Rules it enforces:

- Originals are only ever opened for reading: never moved, renamed, deleted or modified.
- Nothing already in a port folder is overwritten. A different file at the destination refuses the plan;
  an identical file (or a link already pointing at the original) is accepted as connected and left alone.
  A copy is written under a temporary name, verified, and published with `hard_link` (which cannot
  replace anything; where hard links are unavailable the content is written with `create_new`, which fails the
  same way), then the temporary file is removed.
- Destinations are checked after resolving symlinks, so a subfolder that is a link out of the port folder is refused.
- A disc is connected as a set (`plan_connect_set` / `apply_set`): planned whole, one approval, and if a later step
  fails the links and copies that call already made are removed again (only those; an existing file is never ours
  to remove, and an original is never touched).
- A copy is a planned action that reports its size and is refused by `apply` without `Approval::CopyAllowed`.
  A link that the file system refuses (exFAT) fails with `SymlinkUnsupported`; the fallback is a new,
  confirmed copy plan.
- Link and copy destinations are confined to the port folder (`safe_join`, plain file names only).
- Forgetting a connection removes the record and no files.
- State lives in `rom_library.json` and `rom_scan_cache.json` beside `state.json`, not in `state.json`:
  `StateManager::save` rewrites that file from a fixed field list, so a key added to it would be dropped.
- Matching: a file fills at most one slot of a port; CRC-32 or SHA-1 of the file as stored or, for N64, converted to big-endian (the form checksum lists use);
  then name (low-confidence flag); a name match whose checksum differs from the catalog's is "wrong version", and a
  title-only match is not offered when the region tags disagree (USA vs Europe). A `.cue` with its `.bin` tracks
  (any number) is one disc led by the `.cue`.
- Local entries can state requirements explicitly (`rom_files`, `rom_access`, `rom_dest`); the app's own
  parser drops unknown fields, so the library reads those from the raw `ports.local.json`.

Known limits: a catalog entry that lists several games as a bulleted list (Pokemon Gen 1) is read as "any one of these"; on an exFAT card (no hard links) a copy briefly needs twice the file's size; matching clones the scan once per extra slot (fine for one port, to be tightened before bulk use); a multi-bin disc whose tracks are not named like the cue plus "(Track N)" reads as several candidates instead of one disc; no N64 conversion (a `.v64` is reported with a warning, not converted); PS1 ports that list only a
disc serial can be found only when a file name contains it; BIOS files kept outside the ROM folder show those ports
as partial; the BIOS and serial checks are by name only. A ZIP is hashed as a file (a CRC of a ROM inside an archive is not matched); headered dumps
(SNES copier headers, iNES) hash differently from headerless No-Intro lists and show as "wrong version";
Switch dumps that are folders are not scanned; `rom_access` for Ship of Harkinian and 2 Ship 2 Harkinian
("use original") comes from the findings document and is not yet confirmed on a Deck.

## Game files screens (`src/fork/rom_screens/`, `src/app/rom_screens.rs`)

Settings > **Game files...** (stage R2). Every screen is a list in the picker dialog the other fork screens use, so the
controller (D-pad, A, B), keyboard and mouse work in a window and fullscreen; B closes the whole flow, like the Steam
screens. The text is English only.

- **Folder:** one setting. The first list offers places that exist (an SD card's or home's `ROMs`, `Roms`, `roms`,
  `Emulation/roms`, `Games`...), "Browse from your home folder" and "Browse from the top"; the browser walks down folder by
  folder (hidden folders skipped, 400 shown at most) and "Use this folder" saves it and starts a scan.
- **Scan:** on a worker thread. A list shows files checked, data read and the current file, with Cancel; B cancels too. A
  cancelled scan changes nothing and saves nothing. The result is saved (`rom_scan_cache.json`) and reused: a rescan hashes only
  new or changed files, and "Results by system" opens from the saved scan after a restart without touching the folder.
- **Results:** grouped by system. Per system: files and how many ports are connected / found / to pick / wrong or partial /
  missing. Per port: Found, Several matches, Partly found, Wrong version, Missing, Cannot be checked, Connected, with
  "(check this)" for name-only matches and the expected versus found file in the detail. A port shows when something real was
  found for it or when it is installed and missing its files, so uninstalled catalog entries do not bury the result. The system
  of a port with no match comes from its catalog tags (n64, ps1, gc...).
- **Connect game files:** per found file the method (Use original, Link, Copy) limited by the port's access level
  (`rom_access`), the default being the cheapest allowed; an ambiguous slot needs a file picked first; "(+size)" and
  an "Extra disk space" line for copies. **Connect now** plans on the worker thread (nothing is written), then:
  links and use-in-place apply at once; a copy first shows a confirm with the files, the size and the free space of the drive and
  is refused if less than size + 1 GB would be left (`df`; unknown space does not block). The copy runs on the worker thread and
  cannot be cancelled once started (an app exit mid-copy can leave a `.<name>.dpm-copy-...` temporary file in the port folder; it is never used). A disc is connected as one set, all or nothing. Originals are never touched; the recorded
  connection is saved in `rom_library.json`.
- **Connected ports:** each recorded connection with its health (OK, game file missing, game file changed, link or copy gone).
  Choosing one offers "Forget": only the record goes, never a file.
- **After an install:** once per port, when the port names files it cannot run without, one confirm offers to connect them
  (the answer is final; `rom_prompted_ports` in `fork-settings.json`). Not offered when the install is followed by Play.
  It sets the folder first if none is set, rescans, then opens the Connect step.
- **Known limits:** B closes the whole flow from any list (each deeper list has a Back or Cancel row; the copy confirm's Cancel
  returns to the main window); an abandoned scan (B) stops within a second and a new job is refused until it has; a disc whose
  `.cue` lists a track that is missing is still reported Found (the cue's `FILE` lines are not read); declining the after-install
  offer is final for that port; an install that finishes while a scan list is up closes that list and so cancels the scan (upstream's
  `finish_operation`, same for the Steam screens); the browser cursor returns to the first row after each step.
- **Not done:** no button in the Info dialog (its 5+5 button grid is fixed; the offer and Settings cover it); the BIOS folder
  setting and N64 byte-order conversion of R1's backlog; a startup health check of recorded connections (the list shows health
  when opened); "Use original" does not pass the path to the port, it only records it and says where the file is.
- **Not verified on a Deck:** fullscreen layout, controller input through Steam Input, SD card start points
  (`/run/media/...`), exFAT link refusal, copy speed on the SD card.

## Fixes to upstream behavior

Changes that fix upstream defects. The first ones fixed gaps found while validating Tim's catalog
entries; the rest (O1 to O8) are from the upstream bugs register. Each has tests that fail when the
change is reverted.

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
- **Catalog and themes ETag check (304).** With the pinned ureq 3.4.2 a `304 Not Modified` comes back
  as an `Ok` response, not an error, so upstream's check read the empty body, failed the catalog
  validation, and treated the check as failed. The check time was never saved, so once the 12 hours
  had passed the app repeated the conditional request for `ports.json` and `themes.json` at every
  start (headers only, no catalog bytes while upstream was unchanged). The
  status is now read first: 304 means unchanged, any other non-2xx is an error. Test that fails on
  the old code: `cargo test -- --ignored live_etag` (needs the network; also covers the Deck-ready
  list).
  Two guards come with it, both copied from the Deck-ready sync: the stored ETag is sent only while
  the saved file exists and still passes validation (otherwise a 304 would leave a deleted or broken
  `themes.json` missing until upstream changes it), and the new ETag is stored only when the file was
  written (otherwise a read-only install folder would be told "unchanged" about a file never saved).
  A 304 to a request that carried no ETag is treated as an error.
- **No Windows or Android save folder on Linux (O1).** Upstream used a port's Windows save path on
  Linux when the catalog had no `linux` one (and the first value, sometimes Android's, when there was
  no Windows one either), and kept an unset `%APPDATA%` literally, so the folder could not exist and
  backups quietly did nothing (about 45 catalog ports). On Linux these now count as "no Linux save
  folder known". A Windows value that is a plain folder inside the game (`"windows": "saves"`: no
  `%VAR%`, `~`, drive letter, leading slash or backslash, or `..`) still counts on Linux, so Ikemen-GO,
  luanti, Open-Nectar, zelda3-PL and WarioWareTwistedRecomp are backed up and preserved as before. On
  Linux the backslashes of such a folder are separators (`saves\slots` is `<game>/saves/slots`), for
  backup, uninstall and restore alike. One catalog entry is a file, not a folder (DBZ Burst Limit,
  `.\burstlimit.toml`): it now resolves on Linux, so as on Windows the uninstall refuses to run because
  a file save cannot be copied first.
  What a user sees for the others: Info of an installed port says "Save folder: unknown on Linux, no
  backup" (or "Save folder 2" / "Save folders"), the uninstall confirmation says saves are not kept,
  and Save Backup names the installed ports concerned. The update backup only logs it (its progress
  line is replaced at once). Uninstall never touched folders outside the game folder and still does
  not. A Linux path in `ports.local.json` fixes a port. Windows keeps upstream's fallbacks.
- **A kept save copy is no longer deleted (O1 review).** Upstream's restore after a reinstall deleted
  the `Pending Restore` copy when the port had no save folder to restore into, which lost the only copy
  of those saves. The copy now stays.
- **Newest release, not an older one (O2).** When the newest release offers several files for this
  system (Severed Chains without `preferred_asset`: two Linux tarballs), Install now shows the file
  picker for that release. Upstream walked down to the first older release with one clear file and
  installed it without a word (the 2023 zip for Severed Chains). A newest release with no file for
  this system still walks down. The update check compares against that newest release too (dated by
  its newest candidate file), so a new release, or a file replaced in place, is offered and Install
  asks for the file. Closing that picker when Play started the update launches the installed game.
- **Prereleases (O3).** Drafts are dropped and GitHub prereleases are skipped unless none of the 5
  listed releases is stable; then `releases/latest` (the newest stable release) is added, so an older
  stable release still wins over betas. When the stable releases have no file for this system, the
  install fails rather than taking a beta. "Select version" in Info still lists every release,
  prereleases included. A recycled tag such as the `devbuild` of Severed Chains is not a prerelease and
  is listed below `release-3.0.0` (GitHub orders by creation date), so `preferred_asset` plus this
  order keeps the main release.
- **Self-update by commit (O5) and a stale flag (O4).** See Builds. The flag's cause is kept in
  `fork-settings.json` (`self_update_offer`); at start, a flag that no longer applies is cleared and
  a check runs at once.
- **Catalog sync failures (O6) and checks while open (O7).** A failed catalog or themes check is
  logged (`catalog check failed: ...`) and retried after 1 hour, or the server's `Retry-After`,
  instead of at every start. An app left open runs the catalog, themes and self-update checks when
  they fall due (every 12 hours).
- **One instance per install folder (O8).** The single-instance socket is named after the install
  folder, so this fork and the original, or two copies in different folders, run side by side. The
  same folder started twice still hands over to the open copy. A copy from before this fix (old shared
  socket name) gets the hand-over too when it runs from the same folder; that is read from `/proc`
  without connecting, because a copy on the old name comes to the front on any connection.
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
  Since 2026-10-21 it compares commits (see Builds).

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
- **In the app** (since 2026-10-21): see "The Deck-ready list in the app" below.

## The Deck-ready list in the app

A second list beside All ports. L1/R1, Tab or the footer button switch lists; in the Deck-ready list
A installs or plays, X opens Info, Y opens its Filters and B (or Escape) goes back to All ports.

- **Fetching:** `index/deck-ready.json` from this repository's `main`, with `If-None-Match`, when the
  list is opened and the saved copy is missing or older than 12 hours. The copy is saved as
  `cache/deck-ready.json` (ETag and check time in `cache/deck-ready.meta.json`) and shown at once;
  offline, the saved copy stays on screen with a note. A download that doesn't parse never replaces it,
  and an ETag is only sent when a usable copy exists.
- **What is shown:** `kind: game` by default; community finds (`likely_game`) behind a Filters toggle.
  Filters: builds (all, Deck build, Linux build), system, community finds. Deck builds first, then the
  newest release. The search box searches name, system and repository.
- **No duplicates:** an entry is matched to a port the app already has by the index's
  `catalog_folder`, then by repository. A match installs or plays that port exactly as All ports does
  ("Catalog", "Yours" or no tag when installed).
- **Adding a new one:** an entry with no match (including ones the catalog turned off) is added after
  a confirm dialog as the user's own entry in `ports.local.json` (backed up first, like adopting):
  `source` is the repository, so the normal installer downloads it and the normal update check follows
  it (but a Linux build whose release has several Linux files needs the file picker at install and is then
  not updated on its own; Deck builds avoid this through `preferred_asset`, unless the release has two Deck
  files, which also go through the picker); Uninstall is replaced by Open folder. For a Deck build, `preferred_asset` is the Deck marker of
  the file name (`steamdeck`, `steamos`...), so next version's file still matches. The folder is the
  catalog's folder name or the repository name, with the owner added when that is taken.
- **Not installable from here:** an entry missing from the latest refresh (`missing_since`), and one
  that is not on GitHub or GitLab (unless the catalog has it).
- **Saves:** an added entry has no `save` folders (the list doesn't know them), so the save backup
  before an update does not cover it. The confirm dialog says so.
- **Entries are found by `id`**, not by position, so a refresh that lands while Info is open can't
  point Install at another project.
- **Limits:** English only. `ports.local.json` backups are never pruned. The ratings are file-name
  ratings; nothing here records whether a port ran on the Deck. If the catalog later adds a port the
  user already added under another folder name, both stay in All ports and the Deck-ready row follows
  the catalog's. A failed check is not remembered, so offline every opening of the list tries again
  (30 s timeout, in the background). Switching lists clears the search box.
- **Upstream fixes made for this list:** `.tar.xz` and `.tar.zst` releases now unpack (several Deck
  builds ship only those), and a checksum or `.zsync` file beside the wanted one no longer makes
  `preferred_asset` ask which file to use.

## Builds

Every push builds `steamdeck-ports-mgr-linux-x86_64.tar.gz`, available as an artifact on the Actions
run. It holds the program, `7zzs`, a starter `ports.json` and `themes.json`, and the fork's updater.

Each push to `main` that passes CI (doc-only pushes included) also updates the **`rolling`**
release in place: the tag moves to the build's commit and the assets are replaced with the build and
its `.sha256`. It is a full release, so `releases/latest` points at it, and its notes name the exact
source commit; `BUILD_INFO` inside the package records the commit and build date too. The app's
update check compares the commit in its own `BUILD_INFO` with the commit in the notes' "Source for
this build: .../tree/<sha>" line (`src/fork/self_update.rs`; keep that line's form in `ci.yml`), so a
second build on the same day is offered and an installed build is never offered again. Without either
commit it falls back to upstream's day-only date check. A build whose UTC date rolled over before
upload is still not published, for that fallback. Pushes made by workflows, such as catalog-only upstream syncs, do not trigger CI, so they
do not produce a new release.

The updater (`tools/fork-updater.sh`, shipped under upstream's file name so the app's Update and
Force update buttons run it) downloads the rolling release, refuses it unless the SHA-256 matches,
keeps the previous program files in `.previous/`, replaces only `ports_launcher`, `7zzs`, itself and
`BUILD_INFO`, and logs to `updater.log`. It only stops and restarts the copy of the program in its
own folder. On any failure it changes nothing, writes `update-failed.txt`, and reopens the version
that was running. Upstream's updater scripts stay in the repo unchanged but are not shipped,
because they install upstream's release.
