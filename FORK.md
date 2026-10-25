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
| 2026-10-22 | `src/app/dialogs.rs`, `src/app/mod.rs` | Import ports: the Settings entry "Adopt installed ports..." renamed "Import ports..." (`SettingsEntry::ImportPorts`, opens `import_screen::open`); one module line |
| 2026-10-22 | `tools/smoke-test.sh` | Opens Settings > Import ports... and closes it again |
| 2026-10-23 | `src/fork/port_mover/{steam,old_copies,mod}.rs` | Import review fixes: `moved_path` also matches through links (and so the removal check's shortcut test); `OldCopy.import_folder` (new optional JSON key) relaxes only the name-equality rule of `removal_check` / `finish_check`, only when the new copy is named exactly that folder; `prepare` refuses (`MoveError::Protected`) a folder that holds the home folder, the app's folder or the ports folder; 8 tests in `fork::import::real_engine` |
| 2026-10-23 | `src/fork/port_mover/mod.rs`, `src/app/ports_folder_screen.rs`, `src/app/rom_screens.rs`, `src/app/rom_screens/find_flow.rs` | Import re-check fixes: `Prepared.import_folder` (set by Import after `prepare`) is written into both the pending and the finished old-copy record, so "Finish switching" and the removal check keep working for an import; "Why it is kept" returns to the same question; a busy old-copy check hands its text to the waiting hook; B on the file search's and the file check's progress lists goes where their Cancel does (`ABANDON_HOOK`, cleared when a job does not start; none for the automatic search after an install, where B just closes); the title bar's X on "Why it is kept" and on the import's confirm dialogs goes where B goes; the X on a problem's full text in the game file question brings the question back (the X on the question itself is Cancel) |
| 2026-10-23 | `src/app/ports_folder_screen.rs` | B on "Remove the old copy now?" keeps the rest and ends the run with its summary; the closing message never shows the hook token; the hook gets `None` when the run is abandoned (another dialog took the screen) |
| 2026-10-23 | `src/app/rom_screens.rs`, `src/app/rom_screens/find_flow.rs` | `after_ports_added` became `after_ports_added_then(.., then)`: `then` runs on Done, B on its list, Continue on its notices and B on the search's progress list (`ABANDON_HOOK`) |
| 2026-10-22 | `src/app/ports_folder_screen.rs` | `prompt_old_copies_then`: the old-copy questions followed by a caller's step instead of the closing message (a one-shot hook keyed by a token at the start of the summary, so another screen's run never fires it; 3 tests); `prompt_old_copies` itself unchanged for other callers |
| 2026-10-22 | `src/main.rs` | Import ports: window Close (when it quits) and tray Quit also ask `import_screen::quit_waits_for_import`, so a running import is cancelled first |
| 2026-10-21 | `src/core/installer.rs` | `refuse_adopted`: `install_port` and `install_extra` stop with a message when `Library/<folder>` is a symlink (an adopted port), whatever the catalog says, so a bad edit of `ports.local.json` cannot make the installer write into the user's own folder; 1 test |
| 2026-10-21 | `README.md` | Item 4 (adopt) set to "Testing", item 5 (Steam shortcuts) to "In progress" |
| 2026-10-22 | `README.md` | Item 4 reads "Import ports already on the Deck into your ports folder" (still Testing) |
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
| 2026-10-24 | `src/app/mod.rs` | One line registering `steam_close_screen` ("Close Steam and update shortcuts") |
| 2026-10-24 | `src/app/playtime.rs` | One line at the top of `record_playtime`: `ports_folder_screen::note_launch_end`, so a Play of 30 s or more from a moved port's new folder counts as "started from its new place" for its old copy |
| 2026-10-22 | `src/main.rs` | One line starting the Game Mode queue watch (`steam_screens::start_queue_watch`, a timer held for the app's life) |
| 2026-10-22 | `ui/dialogs/info.slint` | Restore saves: one `DialogButton` "Restore saves..." above the shortcut row (button index 10), shown only when `restore-saves-enabled`; the instructions box shrinks by its height. Property, callback, import of `DialogButton` |
| 2026-10-22 | `src/app/gamepad_target.rs` | `INFO_BUTTON_COUNT` 10 to 11; the new button in `info_buttons_enabled` and `invoke_info_button` |
| 2026-10-22 | `src/app/dialogs.rs` | Info dialog: `restore-saves-enabled` set when the port names `save` or `save2`, wired to `save_restore_screen::open_from_info` (one import, two lines). One Settings entry, "Restore saves..." (list, label and menu match; English text); Backup Saves, Restore saves and Force update wait while a restore runs |
| 2026-10-22 | `src/app/mod.rs` | One line registering `save_restore_screen` |
| 2026-10-22 | `src/app/install_launch.rs` | `launch_flow` refuses while the port's install slot is held (install, update or save restore), so a tray recent-games click cannot start a game mid-restore (upstream had the same hole for installs) |
| 2026-10-22 | `src/main.rs` | Window Close (when it quits) and tray Quit wait for a running save restore (`save_restore_screen::quit_waits_for_restore`); the footer Update is refused during one |
| 2026-10-22 | `README.md` | Item 6 now reads "Save backups before every update, and restoring a backup" (still "Testing") |
| 2026-10-21 | `src/app/dialogs.rs` | One Settings entry, "Artwork sources..." (list, labels and the menu match; English text), and `build_text_entry_dialog`: the existing search dialog used as a one-line text box with rows under it (Save, Remove, Cancel), for pasting a key. About 35 lines |
| 2026-10-21 | `ui/dialogs/picker.slint` | `SearchListDialog` gets `in property <bool> password` and sets the text box's `input-type` from it, so a ScreenScraper password shows as dots. Two lines; no change while the property is false |
| 2026-10-21 | `src/app/mod.rs` | One line registering `art_screens` |
| 2026-10-21 | `.gitignore` | `secrets.json` added to the Secrets block (it holds the user's API keys; it is never committed) |
| 2026-10-22 | `README.md` | The install pointer in the top note names the one-file installer |
| 2026-10-22 | `src/ui/linux_chrome.rs` | `register_desktop_entry` writes this fork's own menu entry, `steamdeck-ports-mgr.desktop` ("Deck Ports Manager", icon `steamdeck-ports-mgr`, same Exec and `StartupWMClass`), instead of upstream's `ports_launcher.desktop` / `ports_launcher.png`, so it no longer overwrites the original Ports Launcher's entry. One-time migration: an old `ports_launcher.desktop` is removed only when its Exec is this program, and the old icon only when no remaining entry uses it. `desktop_entry_text`, `upstream_entry_is_ours`, `migrate_upstream_entry`; 5 tests. Window title and tray text unchanged |
| 2026-10-22 | `src/main.rs` | Ports folder setting: `AppPaths.library` is built from `fork::ports_folder::at_startup` instead of `<app>/Library` (the default stays `<app>/Library` and is created as before; a chosen folder that is missing is never created and nothing is marked installed from it); one line calling `ports_folder_screen::startup_check` |
| 2026-10-22 | `src/app/state.rs` | `AppPaths.library_dir: PathBuf` becomes `library: RefCell<PathBuf>` with a `library_dir()` accessor, so the ports folder can change while the app runs |
| 2026-10-22 | `src/app/dialogs.rs`, `events.rs`, `install_launch.rs`, `save_restore_screen.rs`, `rom_screens.rs`, `steam_screens.rs`, `deck_ready_screen.rs` | `app.paths.library_dir` read through `library_dir()` (mechanical, one token per use) |
| 2026-10-22 | `src/app/install_launch.rs` | `start_install` and `try_claim_op_slot` (install, update, version picker, extras) refuse with a message while the ports folder is missing or being moved (`ports_folder_screen::refuse_when_blocked`); 2 lines |
| 2026-10-22 | `src/app/dialogs.rs` | One Settings entry, "Ports folder..." (after Library; English text); `DialogSlot::FolderPicker` (two match arms) and `build_folder_picker_dialog`, sized like Info |
| 2026-10-22 | `src/app/gamepad_target.rs` | The folder picker in the three dialog matches (B cancels, A picks, Up/Down skip headings) |
| 2026-10-22 | `src/app/mod.rs` | Two lines registering `folder_picker_screen` and `ports_folder_screen` |
| 2026-10-22 | `ui/app-window.slint` | One line exporting `FolderPickerDialog` and `FolderRow` from `ui/fork/folder-picker.slint` |
| 2026-10-22 | `src/ui/slint_layout_lint.rs` | The new Slint file is linted too |
| 2026-10-22 | `src/main.rs` | Window Close (when it quits) and tray Quit also wait for a running ports move (`ports_folder_screen::quit_waits_for_move`: the move is cancelled, its copies removed, then the app quits) |
| 2026-10-22 | `src/app/events.rs` | ROM search (branch `rom-search`): the after-install call is now `rom_screens::after_install` (looks for the port's game files in the ROM folder once per port) instead of `offer_after_install`; same place, same condition |
| 2026-10-22 | `ui/dialogs/info.slint` | ROM search: a second `DialogButton`, "Game files..." (button index 11), on the same row as "Restore saves..." (the two are centered side by side; the row and its height appear when either is shown). Property `game-files-enabled`, callback `game-files-requested` |
| 2026-10-22 | `src/app/gamepad_target.rs` | ROM search: `INFO_BUTTON_COUNT` 11 to 12; the new button in `info_buttons_enabled` and `invoke_info_button` |
| 2026-10-22 | `src/app/dialogs.rs` | ROM search: Info sets `game-files-enabled` for an installed port that names game files and wires it to `rom_screens::open_from_info` (one import, two lines) |
| 2026-10-23 | `src/app/mod.rs` | Simple setup, stages 1-2: two lines registering `import_cards_screen` and `page_dialog` |
| 2026-10-23 | `ui/app-window.slint` | Simple setup: `WizardCard` and `WizardTile` imported from `ui/fork/setup-wizard.slint` and exported with the wizard's other types (one line each place) |
| 2026-10-23 | `tools/smoke-test.sh` | Simple setup: the Import ports window is found by the `Import ports: ` title prefix (each card page has its own title: found, port N of M, move, old copies) instead of the exact title of the old list |
| 2026-10-23 | `tools/smoke-test.sh` | Simple setup stage 5: the wizard part of the first-start run follows the new pages (no welcome page: the first page is the ports folder question; keeping the folder opens Import ports at once; B on its first page goes back; Skip is the second or third choice; B on the first page asks to skip; Down+Return on the question skips; walking to Finish skips each question and B on the last page goes back one page); the fresh runs use an empty home folder so Import ports finds nothing |
| 2026-10-24 | `README.md` | Simple setup: item 10 added to the list of what this fork adds (the setup wizard and card-based Import ports), "Testing"; the note says 10 items |
| 2026-10-24 | `src/fork/page.rs`, `src/fork/setup_wizard/{flow,text,places,steam,game_files,ports_pages,progress}.rs`, `src/fork/import/{folders,screen,cards,link}.rs`, `src/fork/folder_picker/mounts.rs`, `src/app/{setup_wizard_screen,page_dialog,import_screen,import_cards_screen,rom_screens,ports_folder_screen}.rs` | Simple setup stage 6 (fork-owned, listed for the record): the wizard's pages for the ports folder (move question, progress, game file problems), game files results, Add all to Steam, progress pages for the scan and the copy, scan of plugged-in drives, `DPM_TEST_MOUNTS` test seam |
| 2026-10-23 | `src/fork/page.rs`, `src/fork/art/thumbs.rs`, `src/fork/custom_repo/parse.rs`, `src/fork/import/{cards,old_batch}.rs`, `src/app/{page_dialog,import_cards_screen}.rs`, `src/app/ports_folder_screen/batch.rs`, `ui/fork/setup-wizard.slint` | Simple setup, stages 1-2 (fork-owned files, listed for the record): Import ports shows cards instead of a list (found page with cover tiles, one card per port, a move page), "Not a port" is saved in `fork-settings.json` (`import_ignored`), and one page asks about all the old copies of an import (`prompt_old_copies_batch_then`; Settings > Ports folder keeps the per-copy questions). The wizard's page type became the shared generic `Page<A>`; its window draws through `page_dialog::show_page`. `prompt_old_copies_then` and its end-hook (used only by the old import list) are gone |
| 2026-10-23 | `src/core/image_cache.rs` | Simple setup: `cache_image` reads at most 20 MB of a cover (a cover is a few hundred KB), instead of whatever the server sends |
| 2026-10-23 | `src/fork/import/{scan,screen,plan,engine,fallback}.rs`, `src/app/import_screen.rs` | Simple setup stages 3-4 on top of import-match's files (fork-owned, listed for the record): `Choices::remove` / `clear_not_a_port`; the old list screens (`ImportScreen` lines, the chooser lists, `confirm_text`, the plan list) are gone, `ImportScreen` now holds the cards; `short_problems` / `short_steam` for the closing page; `fallback::link_instead` links a Deck-ready only port from its own entry and reports a port that is in no list; the import's closing text is a card page (`Outcome`), and `Ctx.extra` keeps projects added from a GitHub link while the screen is open |
| 2026-10-23 | `src/fork/custom_repo/`, `src/fork/import/{link,cards,old_batch}.rs`, `src/app/import_cards_screen.rs` | Simple setup stages 3-4: cards for folders that fit several ports or none (tiles you choose, a filterable list of other ports), "Not a port" kept in `fork-settings.json` (`import_ignored`, listed in Details, Ask again clears it), Use a GitHub link (`custom_repo::check` asks GitHub's API for the project and its newest release through `art::net::Fetch`, two calls; `entry` builds the user's own port list entry, written by the import just before the move and taken off if the port does not move; test switch `DPM_GITHUB_REPLAY=<folder>` answers from saved files), the old-copies page Details |
| 2026-10-24 | `src/core/installer.rs` | Versioned program files (branch `versioned-files`): superseded AppImages are no longer deleted after an install and `follow_favorite_exe` is gone; `extract` returns the release's top-level names; `apply_archive` (now `pub(crate)`) records the folder's names and launch target before unpacking and hands both to `fork::versioned::record_installed` after a successful unpack. Nothing is moved inside the installer; tests changed to match, 2 new (pending record, failed unpack leaves nothing to finish) |
| 2026-10-24 | `src/core/executable_detect.rs` | A folder named `.previous` (superseded releases kept by `fork::versioned`) is never searched for launch candidates; 1 test |
| 2026-10-24 | `src/app/events.rs` | After an install, `versioned_update::after_install` replaces the favorite-follow block; its summary is shown when no launch is pending, before the game file search; when Play started the update it is told so (`launching`) and keeps the note for later |
| 2026-10-24 | `src/app/mod.rs` | One module line, `versioned_update` |
| 2026-10-24 | `src/core/http.rs` | `cloud_agent(https_only)`: the agent the cloud services use (no redirects, error statuses returned as replies, long body timeouts); https-only except for tests and debug builds talking to a mock server on this machine |
| 2026-10-24 | `src/fork/secrets.rs` | `file_is_private`, `tighten_permissions` and `write_private` made `pub(crate)` so `cloud-tokens.json` follows the same rules as `secrets.json` |
| 2026-10-24 | `.github/workflows/ci.yml` | The release build step gets `DPM_GOOGLE_CLIENT_ID` and `DPM_GOOGLE_CLIENT_SECRET` from repository secrets, only for a push to main (never branches, pull requests or the upstream-sync build); a step after it fails the job if the test-only cloud switch names (`DPM_CLOUD_TEST_BASE`, `DPM_CLOUD_DEMO_DIR`) are found in the release program |
| 2026-10-24 | `src/fork/mod.rs` | One line registering `cloud` |
| 2026-10-24 | `Cargo.toml`, `Cargo.lock` | Added `sha2` and `qrcode` (default features off) for cloud backup; the lock gained eight crates, all MIT or Apache-2.0 |
| 2026-10-24 | `src/app/dialogs.rs`, `src/app/mod.rs` | Settings gets "Cloud backup..." (`SettingsEntry::CloudBackup`, opens `cloud_screen::open_from_settings`); one module line |
| 2026-10-24 | `src/app/events.rs` | `cloud_screen::trigger` after Backup Saves finishes and after an install finishes or fails (the update backup), so a backup is copied to the cloud when signed in |
| 2026-10-24 | `src/app/save_restore_screen.rs`, `src/app/save_finder_screen.rs` | `show_port` made `pub(crate)` (the cloud screens hand a copied backup to Restore saves); `cloud_screen::trigger` after a restore's safety backup and after the save finder's Back up now |
| 2026-10-24 | `src/fork/save_restore/mod.rs` | The stale-scratch sweep also removes `_cloud_download_*` folders a crash left in `Saves Backup` (a day old) |
| 2026-10-24 | `src/fork/page.rs`, `src/fork/import/cards.rs`, `src/fork/import/link.rs`, `src/app/page_dialog.rs` | `Card.qr`: a QR code drawn where the cover goes (the cloud sign-in page); the two other `Card` constructors pass `None`; `PageHandle::is_open` |
| 2026-10-24 | `src/main.rs` | One line: `cloud_screen::at_start` (the catch-up pass and the ten-minute check; nothing runs unless signed in) |
| 2026-10-24 | `src/fork/art/resolve.rs`, `src/fork/art/screen.rs` | The art sources' redaction lists also hold the cloud sign-in tokens and email (`cloud::tokens::values_beside`) |
| 2026-10-24 | `STEAM_DECK.md` | The bug-report instructions say never to attach `cloud-tokens.json` or `secrets.json`, and that `cloud-backup.json` is safe |
| 2026-10-24 | `ui/fork/setup-wizard.slint` | `WizardCard.square`: a card's picture can be square (the cloud sign-in QR code) instead of a portrait cover |
| 2026-10-24 | `tools/smoke-test.sh` | Opens Settings > Cloud backup... (by keyboard) and closes it again |

Files added by this fork:

| Date | File | Purpose |
|---|---|---|
| 2026-10-20 | `.gitignore` | Keeps build output, game files, keys and firmware out of the repo |
| 2026-10-20 | `.github/workflows/ci.yml` | Builds the Linux x86_64 release and runs the tests on every push; publishes the rolling release from `main` (since 2026-10-22 with the one-file installer). Since 2026-10-21 also test-builds and runs the tests on Windows (upstream's other platform), without gating the release; skipped inside upstream-sync |
| 2026-10-20 | `.github/workflows/upstream-sync.yml` | Daily merge of upstream changes (see below) |
| 2026-10-20 | `FORK.md` | This file |
| 2026-10-21 | `src/core/contract_tests.rs` | Test-only module that pins the upstream behavior our features rely on (asset choice for our ports' real release names, update decision with a recycled tag, local-entry merge, `state.json` keys) |
| 2026-10-20 | `STEAM_DECK.md` | How to install and test this fork on a Steam Deck |
| 2026-10-24 | `PRIVACY.md` | Privacy policy: what the app stores and sends, and what the optional Google Drive backup touches |
| 2026-10-20 | `tools/deck-discovery.sh` | Read-only script a user runs once on a Steam Deck to inventory installed ports, save folders, Steam shortcuts and tooling (writes only its own report file) |
| 2026-10-20 | `tools/fork-updater.sh` | The fork's updater, shipped as `ports_launcher_updater.sh` (see Builds) |
| 2026-10-21 | `tools/test-fork-updater.sh` | 22 checks of the updater against fake installs and releases; CI runs it. A failed check prints the copy count, the processes under the test folder and the log tail (since 2026-10-21) |
| 2026-10-20 | `tools/smoke-test.sh` | Starts the packaged app on a virtual display with no tray host or desktop session; checks it survives 15 s without a panic, skips the tray, maps a non-blank window, opens the Deck-ready list and comes back (since 2026-10-21), opens Settings > Artwork sources and its Back row, Game files, and Settings > Ports folder with its folder picker, then moves two seeded ports end to end into a new folder (New folder here, Use this folder, Move, Keep for now twice) and checks the setting, the copy's program bits, the kept old copies, their records and a relaunch (since 2026-10-22), and quits on Close. CI runs it and blocks the release on failure |
| 2026-10-21 | `tools/test-app-update.sh` | Offline in-app update test: the packaged app, started from an old copy (marker appended to the program, old `BUILD_INFO`), runs Settings > Force update from the UI against a fake release on `file://`; checks the swap, `.previous/`, the restart on the new file and that saves, catalogs and state survive, then a tampered-checksum case (nothing changes, `update-failed.txt`, app reopens). CI runs it after the smoke test and blocks the release on failure |
| 2026-10-22 | `tools/installer.sh` | The one-file installer's script: downloads the rolling release, checks its SHA-256, installs into `~/Applications/steamdeck-ports-mgr` or updates an existing install exactly like the updater, writes the app-menu entry (same bytes as the app's own) and a desktop shortcut, starts the app. Not shipped on its own (see Builds) |
| 2026-10-22 | `tools/build-installer.sh` | Builds `steamdeck-ports-mgr-installer.desktop`: a desktop-entry launcher with `tools/installer.sh` embedded as comment lines, which its Exec line extracts from its own file (`%k`) and runs |
| 2026-10-22 | `tools/test-installer.sh` | Offline installer test in fake home folders, run through the .desktop file's own Exec line: fresh install, update with user files byte-identical, tampered checksum, no network, running copy, detached launch, path escaping, kdialog/zenity calls, GLib's launcher, and the real app keeping the menu entry. CI runs it and blocks the release on failure |
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
| 2026-10-21 | `src/fork/secrets.rs` | `secrets.json` next to the app: the user's SteamGridDB key, optional TheGamesDB key and ScreenScraper login. Mode 0600, atomic, masked, never logged; plus `BuildCreds`, the developer credentials a build may carry (`option_env!`) |
| 2026-10-21 | `src/fork/art/net.rs` | `Fetch`: the one door for art HTTP (the app's https-only helper), response limits, URL encoding, host allow-lists, secret redaction, and the scripted fake the tests use |
| 2026-10-21 | `src/fork/art/sgdb.rs`, `tgdb.rs`, `screenscraper.rs` | The three source clients: SteamGridDB v2, TheGamesDB v1, ScreenScraper v2 (hash lookup, name fallback, error codes and back-off) |
| 2026-10-21 | `src/fork/art/source.rs`, `title.rs`, `cache.rs` | Source ids and errors, the saved per-source state (last problem, back-off), same-title matching, and the on-disk id table and image cache under `cache/art/` |
| 2026-10-21 | `src/fork/art/resolve.rs` | The resolver: per-slot priority across catalog, SteamGridDB, TheGamesDB and ScreenScraper, id overrides, composed art flagged `generated`, `art_for_port` for the screens |
| 2026-10-21 | `src/fork/art/screen.rs`, `src/app/art_screens.rs` | The "Artwork sources" screen: rows, status, Test key, Fetch artwork, credits (logic tested without UI; the app file draws it) |
| 2026-10-21 | `src/fork/art/fixtures/` | Saved example answers from the three sources, made up to match their documented shapes (no real keys, ids or images), used by the tests |
| 2026-10-21 | `src/fork/steam_user.rs` | Reads `config/loginusers.vdf` (text KeyValues), lists the accounts that have a `userdata` folder and picks the saved one, else `MostRecent`, else the only one |
| 2026-10-21 | `src/fork/backups.rs` | Prunes `shortcuts.vdf.bak-<stamp>` files: newest N kept (at least 1), nothing under an hour old, nothing else in the folder |
| 2026-10-21 | `src/fork/settings.rs` | `fork-settings.json` (next to `state.json`): chosen Steam account, extra adopt scan folders, backups to keep, the self-update offer (`get`/`set`). Other keys survive a save |
| 2026-10-21 | `src/fork/save_notes.rs` | Text for ports with no known Linux save folder (Info line, Save Backup summary) |
| 2026-10-21 | `src/fork/self_update.rs` | Self-update by commit: reads `BUILD_INFO` and the release notes' source commit, decides, and keeps what raised the flag |
| 2026-10-21 | `src/fork/adopt.rs` | Finds ports installed outside `Library/` and links them in (see "Linking an install in place"); 2026-10-22: scan folders with switchable defaults (`scan_roots_with`), optional one-level-deeper look (`scan_with`, cancel flag), helpers shared with `fork::import`; 2026-10-23: release-name and Deck-ready download matching (`scan_with_hints`, `Hints`), programs by extension only, skip list, collections, folders to choose, `why` per find (see "Importing ports") |
| 2026-10-21 | `src/fork/steam_sync.rs` | Plans and applies the shortcuts for installed ports (add, update in place, up to date, blocked) |
| 2026-10-21 | `src/fork/screens.rs` | View model for the Add to Steam screen: labels, toggling, review text, apply. All the decisions, tested without UI (2026-10-22: the adopt screen model moved to `fork::import`) |
| 2026-10-21 | `src/app/steam_screens.rs` | The "Add to Steam" list dialogs (thin: draws `screens.rs` text); the adopt screen left it for `import_screen.rs` on 2026-10-22 |
| 2026-10-22 | `src/fork/import/` | Import ports: scan folders, finds with confidence, size and Steam shortcuts, per-port plan, the move adapter and engine over the port mover, the summary, the link fallback, the list models; tested without UI (see "Importing ports") |
| 2026-10-22 | `src/app/import_screen.rs` | The Import ports lists, the worker-thread job with a Cancel row, and the entry point the setup wizard calls |
| 2026-10-22 | `src/fork/steam_queue.rs` | Steam changes queued from Game Mode: `steam-queue.json` (versioned, atomic), applying it through the screen's safe write, the startup/30-second offer decision, the mode wording; tested without UI |
| 2026-10-21 | `src/fork/rom_library/` | ROM library (library and tests only, no UI): scan a ROM folder (CRC-32, MD5, SHA-1, N64 byte-order normalization), read each port's required files from the catalog text or explicit `rom_files` in a local entry, match by checksum then name, plan and apply use-in-place / symlink / copy, record the mapping in its own `rom_library.json` |
| 2026-10-22 | `src/fork/rom_screens/` | The logic of the game-files screens (R2): folder browser, background job runner, scan and report, results by system, Connect step, connected ports, the after-install search decision (`offer.rs`); tested without UI |
| 2026-10-22 | `src/app/rom_screens.rs` | The screens themselves: list dialogs, a timer that polls the worker thread |
| 2026-10-22 | `src/fork/rom_screens/find.rs` | "Find the game files for this port": `find_game_files_for_port` / `_for_ports` (saved scan reused, only new or changed files read), outcomes (found, several, not found, no folder, folder unreadable, no files needed), prompt text, connect by hand; tested without UI |
| 2026-10-22 | `src/fork/rom_screens/file_pick.rs` | A file chooser list for "Connect by hand..." (folders, then game files first) |
| 2026-10-22 | `src/app/rom_screens/find_flow.rs` | The screens around `find`: after an install or adopt, from Info and Settings; entry points for other screens |
| 2026-10-22 | `src/fork/save_restore/` | Restore saves: list a port's backups (before update, Backup Saves, before restore), resolve its save folders for this system without guessing, and restore one (safety zip first, zip-slip check, swap by rename); `zip_index.rs` reads a zip's table of contents; `view.rs` holds the screen text; tested without UI |
| 2026-10-22 | `src/app/save_restore_screen.rs` | The Restore saves screens (Info button and Settings entry): list dialogs, confirm, a progress dialog polling the worker thread |
| 2026-10-22 | `src/fork/setup_wizard/` | The first-run setup wizard (see "Setup wizard" below): state machine and its adapters (`flow.rs`), the text of each view (`text.rs`), the ports-folder quick choices (`places.rs`), `setup_done` in `fork-settings.json` (`mod.rs`); tested without UI |
| 2026-10-22 | `src/app/setup_wizard_screen.rs` | The wizard page dialog (filled and wired here), the hand-offs to the existing screens and the timer that brings the wizard back when one of them is closed |
| 2026-10-22 | `ui/fork/setup-wizard.slint` | The wizard page (`SetupWizardDialog`): header with the app name, "Step N of 6" and a progress bar, a title, wrapped body text, a summary card, real options (radio choices or actions) and a footer of buttons (Back left, Skip, primary Next or Finish right); theme colors and fonts |
| 2026-10-22 | `ui/app-window.slint`, `src/ui/slint_layout_lint.rs` | Import and export `SetupWizardDialog` and `WizardOption`; the new Slint file is linted |
| 2026-10-22 | `src/app/gamepad_target.rs` | `DialogSlot::SetupWizard`: B (close-requested), A (activate-requested) and the D-pad (move-requested) reach the wizard page |
| 2026-10-22 | `src/main.rs` | One call after `start_queue_watch`: `app::setup_wizard_screen::start_if_first_run(&app, &router)` |
| 2026-10-22 | `src/app/dialogs.rs` | `DialogSlot::SetupWizard` and `build_setup_wizard_dialog` (sized from the work area, 88% high, kept on screen); Settings entry "Setup wizard..." (first row, `SettingsEntry::SetupWizard`); `open_settings_dialog` hands back to the wizard when an existing screen it opened leads back to Settings |
| 2026-10-22 | `src/app/rom_screens.rs` | Setup wizard: `open_folder_chooser`, the folder choice of Game files without the menu in front (4 lines, wraps `choose_folder`); `open_game_files` first asks `setup_wizard_screen::resume_instead_of_menu` (3 lines), so backing out of a screen the wizard opened returns to the wizard |
| 2026-10-22 | `src/app/ports_folder_screen.rs` | Setup wizard: `open_menu` first asks `setup_wizard_screen::resume_instead_of_menu` (3 lines), so cancelling the ports-folder flow the wizard opened returns to the wizard |
| 2026-10-22 | `src/app/mod.rs`, `src/fork/mod.rs` | One module line each (`setup_wizard_screen`, `setup_wizard`) |
| 2026-10-22 | `tools/smoke-test.sh` | The main run starts with `fork-settings.json` holding `setup_done`; a new first part starts a fresh folder and checks by keyboard that the wizard opens, a resting mouse pointer does not move the highlight, the ports step opens the shared picker and cancelling it returns to the wizard, Skip setup asks, quitting half-way leaves setup unfinished, the wizard returns, a confirmed skip is remembered, and walking every page to Finish writes setup_done; click offsets for Settings rows moved down one row |
| 2026-10-22 | `tools/test-app-update.sh` | The old install it builds has `setup_done` set, so the wizard does not cover the main window |
| 2026-10-21 | `src/fork/deck_ready/index.rs` | Reads `index/deck-ready.json` (schema 1); unusable entries are skipped, an unusable file is refused |
| 2026-10-21 | `src/fork/deck_ready/sync.rs` | Fetches the list from this repo with an ETag, at most every 12 hours, and keeps the saved copy in `cache/` |
| 2026-10-21 | `src/fork/deck_ready/view.rs` | Rows, filters, sorting, matching to ports the app already has, and the Info and confirm text; tested without UI |
| 2026-10-21 | `src/fork/deck_ready/entry.rs` | Turns an entry the app doesn't have into the user's own `ports.local.json` entry (backup first) |
| 2026-10-21 | `src/app/deck_ready_screen.rs` | The Deck-ready list screen: switching lists, Filters, Info, install and play |
| 2026-10-21 | `ui/fork/deck-ready.slint` | The Deck-ready list and its Info dialog |
| 2026-10-22 | `src/fork/folder_picker/` | The in-app folder picker's logic: quick places (mounted SD card and USB drives from `/proc/self/mounts`, Home, `~/Games`, `~/Applications`, extra places from the caller), listing, rows, new-folder name rules, header text; tested without UI. Replaces the Game files screen's own folder walker (`rom_screens/browse.rs` keeps only its ROM start points) |
| 2026-10-22 | `src/app/folder_picker_screen.rs`, `ui/fork/folder-picker.slint` | The folder picker screen (`open_folder_picker`), used by Settings > Ports folder and Settings > Game files |
| 2026-10-22 | `src/fork/ports_folder.rs` | The ports folder setting (`ports_folder` in `fork-settings.json`), the startup availability check, the rules for a new ports folder (`check`, with an exFAT/permission/link probe) and `plan_move` |
| 2026-10-22 | `src/fork/port_mover/` | The move engine: copy with permission bits, links and times, verify (list, sizes, bits, links, hashes), game file check before and after, Steam repoint (written or queued, app IDs kept), ROM records follow, old copies recorded in `port-moves.json` and removed only on request |
| 2026-10-22 | `src/app/ports_folder_screen.rs` | Settings > Ports folder, the "not available" message at startup, the move with progress and Cancel, and the "Remove the old copy now?" prompts |
| 2026-10-22 | `src/fork/ports_folder_view.rs` | The lists of the ports folder screens (labels, row actions, preselected row), tested without UI |
| 2026-10-24 | `src/fork/versioned/` | Versioned program files: family and version from a name (reuses `adopt::release_tokens`), the plan of what a new release supersedes, the finish step (launch target, favorite, Steam repoint or queue, move to `.previous/<version>/`, launcher scripts, size of the older versions) and `after_queued_repoint` |
| 2026-10-24 | `src/app/versioned_update.rs` | Runs the finish step on the UI thread after an install and sets the favorite |
| 2026-10-24 | `src/fork/save_finder/`, `src/app/save_finder_screen.rs` | Save finder (branch `save-finder`): per-port save override in `fork-settings.json`, state of a port's saves, read-only search of likely places, the wizard pages and the screen that draws them |
| 2026-10-24 | `ui/dialogs/info.slint`, `src/app/gamepad_target.rs`, `src/app/dialogs.rs` | Info: 13th button "Find saves..." (index 12; the three buttons of the extra row are centred together), shown when the saves are not found or an answer is stored; the save line comes from the save finder first, the old O1 note second |
| 2026-10-24 | `ui/app-window.slint`, `ui/card-grid.slint`, `src/app/cards.rs`, `src/app/state.rs` | `saves-warning` on `PortItem` and `CardItem`: a small "Saves not found" badge on an installed port's list row and on its card (touch or mouse opens the finder; a controller uses Info). `build_card_rows` takes the check as a parameter |
| 2026-10-24 | `src/main.rs`, `src/app/sync.rs`, `src/app/deck_ready_screen.rs`, `src/app/import_screen.rs`, `src/app/mod.rs` | The stored save overrides are applied where the catalog is loaded (start-up, catalog update) and where a local entry is added (`save_finder_screen::reapply`); two callbacks for the badge; one module line |
| 2026-10-24 | `src/fork/save_restore/mod.rs`, `src/fork/save_restore/broad.rs` | Save finder review fixes: the guards moved into one `folder_problem` that `resolve_target`, the finder and the symlink re-check share; new refusals for folders that hold many apps' files (`broad::container_reason`: another port's folder in the library, `~/.local/share`, `~/.config`, Steam and Proton prefix levels, Documents, mount roots); a save folder that is a link is checked again by where it leads before the swap; `tree_size_capped`, `free_bytes` made `pub(crate)` |
| 2026-10-24 | `src/core/save_backup.rs` | `backup_before_update`: a save folder outside the game folder is checked against the free room before it is copied (fork: save finder) |
| 2026-10-24 | `src/fork/folder_picker/mod.rs`, `src/app/folder_picker_screen.rs` | `PickerRequest.cursor_off_use_this`: the cursor starts on the first sub-folder (or Cancel), never "Use this folder" (used by the save finder) |
| 2026-10-24 | `src/fork/save_restore/mod.rs`, `src/fork/save_restore/broad.rs`, `src/core/save_backup.rs` | Save guards (branch `save-guards`): Restore checks where a linked save folder leads (`link_problem`) before the safety backup copies anything; `backup_before_update` refuses a save folder that holds many apps' files and a link that leads to one (no copy, no staging); `userdata` and Flatpak roots (`~/.var/app/<id>`, its `data`/`config`/`cache`) join the refused folders; the ports library is passed to `folder_problem`/`container_reason` explicitly |
| 2026-10-24 | `README.md` | One line in the note at the top linking `PRIVACY.md` (new file, the fork's own) |
| 2026-10-24 | `src/fork/cloud/` | Cloud backup of the save backups to Google Drive (branch `cloud-backup`): `tokens` (`cloud-tokens.json`, mode 0600), `record` (`cloud-backup.json`), `local`/`plan`/`sync` (what to upload and one pass), `service` (the trait and failure kinds; no delete), `net`/`oauth` (HTTPS client, device-code sign-in and refresh), `build` (the Google client compiled in from CI secrets), `sha256`, `log` |
| 2026-10-24 | `src/app/cloud_screen.rs` | Settings > Cloud backup: the pages, the sign-in, the status card, the restore list and the passes the hooks start |
| 2026-10-25 | `src/app/dialogs.rs` | Polish (branch `polish`): `close_current_dialog` hands the guided page window to `page_dialog::park` (4 lines) instead of hiding it at once, so the next page can be drawn in the same window; anything else closes as before |
| 2026-10-25 | `tools/smoke-test.sh` | Polish: two checks that the first Import ports page and the page B returns to are the wizard's own window (same X window id); one that the wizard's next page after Import ports closes and hands back (a close-first hand-off) is that same window |
| 2026-10-25 | `src/ui/gamepad_router.rs` | Polish: `dispatch` asks `fork::press_echo::pad` before acting on A or B (an echo of a key Steam typed for the same press is dropped) and runs the target inside `press_echo::while_pad`, so a dialog callback the pad reaches is not taken for a key |
| 2026-10-25 | `src/app/dialogs.rs` | Polish: `wire_keyboard_nav` (Return) and `wire_dialog_close` (Escape) ask `press_echo::key` first (2 lines) |
| 2026-10-25 | `ui/app-window.slint`, `src/main.rs` | Polish: Escape on the main list calls a new `escape-pressed` callback; in big mode it leaves big mode (unless it echoes a pad B), in the windowed view Close waits 300 ms and a pad B in that time drops it (`press_echo::hold_main_escape`); Return on the main list asks `press_echo::key` |
| 2026-10-25 | `src/app/dialogs.rs` | Polish: `build_setup_wizard_dialog` fills the work area and sets `full` in big mode and in Game Mode (4 lines) |
| 2026-10-25 | `ui/dialogs/chrome.slint` | Polish: `DialogChrome` draws its title bar only when `bar-height` is above zero (the guided pages full screen have none; every other dialog is unchanged) |
| 2026-10-25 | `ui/app-window.slint` | Polish: `WizardFact` imported and exported next to the other wizard types |
| 2026-10-25 | `src/app/gamepad_target.rs` | Polish review: `dialog_activate` runs inside `press_echo::not_a_key` (the close callback it invokes for A on a message is not a second key) |
| 2026-10-25 | `src/app/dialogs.rs`, `src/app/events.rs` | Polish review: the theme picker's and the after-Play version picker's own close callbacks ask `press_echo::key` first |
| 2026-10-25 | `ui/app-window.slint`, `src/main.rs` | Polish review: Escape in the Deck-ready list also goes through `escape-pressed` (filtered, acts at once); big mode Escape waits 300 ms like Close; the held Escape does nothing when a dialog opened meanwhile |
| 2026-10-25 | `ui/app-window.slint` | Home screen: imports `ui/fork/home.slint`; `home-*` properties and callbacks; `HomeView` shown instead of the port list or card grid while `home-mode` is on (both of those now also check `!home-mode`); the title says "› Home" and the big-mode hint "L1 / R1  Lists"; the footer list button's text comes from `next-list-text`; `HomeTile` exported |
| 2026-10-25 | `src/app/state.rs` | Home screen: `home` field; `current_selected_port`, `move_selection` and `refresh_current_view` hand over to Home while it is shown (3 guards); `apply_live_mode` redraws Home for the new size; `ensure_card_images_cached` made `pub(crate)` |
| 2026-10-25 | `src/app/cards.rs`, `src/app/install_launch.rs`, `src/main.rs`, `src/app/mod.rs` | Home screen: `load_cached_card_image` made `pub(crate)`; `activate_selection` hands over to Home; `home` field initialised; one module line |
| 2026-10-25 | `src/app/gamepad_target.rs` | Home screen: L1/R1 go round the lists (Home, All ports, Deck-ready); B in the Deck-ready list goes back to Home in both modes (`home_screen::back`) |
| 2026-10-25 | `src/core/jobs.rs` | Home review: `run_update_check_result` tells a failed check (rate limit, no network) from "no update"; `run_update_check` and the private `update_available` now go through it (same behaviour for Play); 1 test |
| 2026-10-25 | `src/app/install_launch.rs` | Home review: Play's "is the update check due" test is `fork::home::update_check_due`, the one the home screen uses too (same rule) |
| 2026-10-25 | `src/ui/slint_layout_lint.rs`, `tools/smoke-test.sh` | Home screen: the new Slint file is linted; the smoke test checks the app opens on Home, All ports is one Tab away, Deck-ready the next, and Home after that |

## Guided pages: one window (`src/app/page_dialog.rs`)

Every guided page (setup wizard, Import ports cards and progress, old copies, cloud backup, save finder, Close Steam
and update shortcuts, ports-folder pages) is drawn by `page_dialog::show_page`. The first page opens the window;
each later page is drawn in that same window: its texts are refilled and the page's handlers are swapped in behind
callbacks that were wired once (`HOST`, a type-erased `Binding`), so a page of any owner can take the window over.

- **Hand-offs that close first.** Page owners close the dialog before handing over (`close_current_dialog`). For the
  page window that call parks it (`park`): the slot empties and the controller target is popped at once, but the
  window stays up for 40 ms. A page shown in that time takes it back (no new window); otherwise it is hidden and the
  main window gets the focus as a normal close gives it (not when another dialog has opened meanwhile). A key that
  reaches a parked window does nothing.
- **The hand-off watcher** (`setup_wizard_screen::tick`) sees the slot exactly as before: a page swap happens
  inside one callback, so the slot is never empty between two pages.
- **Progress pages.** A `PageHandle` is tied to a generation that every page shown and every park bumps, so a
  handle whose page was replaced (by another page in the same window, or by any dialog) reports `is_open() == false`
  and `update` does nothing, as when its window used to close.
- **Window close.** The window system's close button runs the page's `on_exit`; when that shows another page in the
  same window, the window stays.
- **The cursor comes back to where you were.** The cursor position on each page is remembered by page (window
  title, header, counter such as "Port 2 of 5", and question; last 24 pages). A page shown again (B, a footer Back
  button, the wizard drawing its page again after a hand-off) starts where the cursor was, if that place still
  exists; a page seen for the first time starts where the page says. The memory survives hand-offs to other screens
  and is cleared when the guided pages end (the page window goes and nothing else is open).
- **Old copies and import progress are pages too.** "Checking the old copies" and "Removing the old copies"
  (`ports_folder_screen/batch.rs`) used upstream's progress dialog between two pages; they are pages now
  (`old_batch::working_page`, nothing to choose, B does nothing, as before), so the old copies page takes the same
  window. The import's progress list outside the setup wizard (Settings > Import ports: "Looking for ports",
  "Importing ports") is the same progress page the wizard uses, with the Import ports header and no step bar; B
  closes it and cancels, as B closed the list.
- **A new page** scrolls the body to the top, re-arms the "resting pointer" guard and gives the keyboard to the page
  (or to its text box when the cursor starts there) (`page-shown()` in `ui/fork/setup-wizard.slint`).

## Guided pages: focus look, hint bar, one press acts once

- **Focus look** (`ui/fork/setup-wizard.slint`, `FocusLook` / `FocusRing`): the focused option, tile, button or text
  box gets a 4 px ring (`selection-border`, white on the default theme; dark on light themes) and a slightly lighter
  fill; borders never change width, so nothing moves when focus moves. "Recommended" is a small tag at the right of
  the option's title plus an accent edge on its left, never a fill. The primary button and the tag use the accent
  darkened (`info.darker(0.6)`) so white text reads at about 5:1.
- **Hint bar**: a strip along the bottom of every page, from `fork::page::hints`: "A Choose · B Back · D-pad Move".
  "D-pad Move" only when more than one thing can take focus, "A Choose" only when something can; pages with a text
  box add "Steam + X Keyboard". What B does comes from `Page.back_hint`: `Page::b_does(&action)` takes the label of
  the button or option that does what B does (the wizard: its `back_action`; Close Steam: its `back_action`; the old
  copies page: Keep them; notices: their safe answer), `Page::b_means("Cancel")` sets it by hand (progress and moving
  pages, the import's game-file question: "Stop the import").
- **One press acts once** (`src/fork/press_echo.rs`): Steam Input can type Return or Escape on top of the A or B our
  gamepad reader (gilrs, reading the pad whatever window has focus) sees, and both reach the same action. A press
  from the other source (pad or keyboard) of the same kind within 300 ms of one acted on is dropped, once per press;
  presses from one source always count. Asked by the pad dispatch, the page window's Return/Escape, the upstream
  dialogs' keyboard wiring and the main list. Escape on the main list in the windowed view (Close) waits 300 ms; a
  pad B in that time drops it and counts as the one B (coordinator ruling 2026-10-25), so B on the main list never
  closes the app even when Steam's Escape arrives first. Escape in big mode (leave big mode) waits the same way;
  Escape in the Deck-ready list (leave it) acts at once through the filter. Dialogs that wire their own close
  callback (13 of them: art sources, theme picker, the version picker after a Play, the folder picker and its New
  folder box, the import's confirm/game-file/detail dialogs, the ports-folder dialogs, the game files lists) ask the
  filter too. Callbacks the app invokes itself after a press already let through (A on a message dialog closes it
  through its close callback) run inside `press_echo::not_a_key`, so they leave no phantom key press. Return in a
  page's text box goes through the same check. Not covered: D-pad arrows (Steam may also type arrow keys; watch for
  double moves on the Deck).

## Guided pages: full screen on the Deck, status marks, progress bars

- **Full screen** (big mode or Game Mode): the page window takes the whole work area (1280x800 on a Deck) with no
  title bar and no X (`full` in `ui/fork/setup-wizard.slint`; `DialogChrome` skips a zero-height bar). The
  "Deck Ports Manager" header line goes; the step counter ("Step 2 of 5", or the page's own header outside the
  wizard) sits at the right of the title, under the step bar. Margins are 30 px instead of 14. Windowed desktop
  keeps the floating window with its title bar. With no X in full screen, "Skip setup?" also offers "Not now" (leave;
  setup stays unfinished and comes back at the next start), as the X does windowed. Big mode cannot be switched while
  a page is open (the View button reaches only the main window), so a page never changes mode under the user. The
  body reaches into the right margin, where its scrollbar runs, so its content ends where the step bar ends.
- **Status marks**: a line of the summary box can carry a `fork::page::Mark` (done, to do, waiting, skipped,
  problem), drawn as a small picture in a coloured disc (`StatusIcon`: a check, an arrow, a clock, a dash, an
  exclamation mark), never as a text character that a font could lack. Used by "Your setup" and the rerun's last
  page (`rerun::StepLine.mark`), the first run's last page, the import finished page and the old copies status
  lines. Each mark is set where the line is made, from the typed outcome (`StepLine.mark`, `steam_line` /
  `game_files_line` returning their mark, `ImportSummary::short_problem_lines` / `short_steam_line`, the import
  `Outcome.lines`), never read back from the words. A line with no mark keeps the indent of marked lines on the same
  page. The old text glyphs (check mark, arrow) are gone.
- **Progress bars**: `Page.progress` draws a bar under the summary box, filled when the work knows how far it is,
  a sliding block otherwise. Background jobs say how far with `Ctl::set_fraction`. Filled: moving ports (bytes of
  each port over the batch), the import's copy (`engine::move_selected_with`, `MoveContext.fraction`), the game
  files scan while it reads (bytes; sliding while it walks the folders), copying back from the cloud (files and
  bytes) and the upload pass (`PassInput.fraction`, files and bytes, shown on the Cloud backup status while a pass
  runs; it ends full when the pass finishes). Sliding: looking for ports, checking and removing old copies. The sliding
  block is redrawn every frame while shown (`animation-tick`); on the Deck that is a small, constant cost only while a
  long scan walks folders.

## Home screen (`src/fork/home.rs`, `src/app/home_screen.rs`, `ui/fork/home.slint`)

The first thing the app shows (Tim, 2026-10-25: "want a better first impression screen").

- **Your ports**: a grid of cover tiles for the installed ports (covers from the card image cache, the port's name on a
  plain tile while there is none), each with one short status badge drawn with the guided pages' status marks, in
  this order of priority: "No game files" (red; the game files records), "Update ready" (blue arrow), "Steam waiting"
  (orange; a change for the port in the Steam queue), "Find saves" (blue arrow; the save finder), else "Ready".
- **Update check**: Play's own check (`jobs::run_update_check_result`), for exactly the ports Play would check
  (`home::update_check_due`, shared with `install_launch`: "check for updates" on, the port's own switch on, a GitHub
  or GitLab release, an install older than Play's stale time). At most one round every 12 hours (`home_update_check_at`
  in `fork-settings.json`, stamped when a round starts), one port after another in the background, stopping at the first
  rate-limit answer; each request has the API agent's 30 s limit and the poll stops at a deadline. Results are kept per
  port with their time (`home_update_checks`), so a restart shows the last known badge without asking; a result older
  than the port's last install is not shown. One switch turns the round off: `CHECK_UPDATES_AT_START` in
  `src/app/home_screen.rs`.
- **Continue**: the port played last (the later of the app's play record and Steam's `LastPlayTime` for a shortcut
  whose program is inside the port's folder) is the first tile, tagged Continue, and not repeated below; the rest are
  by name. While setup was never finished, a "Finish setting up" tile comes first.
- **Nothing installed**: one card, "Set up Deck Ports Manager" (the setup wizard) and "Browse ports" (All ports).
- **Top line**: "Your ports", a small cloud backup status (cloud outline, a status mark and "Backed up", "Backing up",
  "Backup problem" or "Not backed up yet"; nothing when not signed in) and the Settings gear.
- **Lists**: L1/R1, Tab and the footer button go Home, All ports, Deck-ready, then Home. In big mode, where All ports
  is upstream's card grid of installed ports only, the order is Home, Deck-ready, All ports, so the next list after Home
  is one to install from; "Browse ports" goes to All ports windowed and to the Deck-ready list in big mode. All ports
  itself is unchanged; B (and Escape) in the Deck-ready list goes back to Home in both modes (`List::back`). The search bar stays on Home: typing
  searches All ports (the app switches to it with what was typed).
- **Buttons on Home**: A plays (or sets up on the Setup tile and the empty card), X opens Info, Y opens Settings, B does
  nothing; the hint bar says so ("A Play · X Info · D-pad Move · Y Settings · L1/R1 Lists").
- **Sizes**: about 210 px a tile in big mode (6 on a Deck), 160 px windowed, never taller than the room left; redrawn
  when the mode changes.

## Add to Steam (Settings menu)

- **Add to Steam** writes one shortcut per selected installed port to the chosen account's
  `shortcuts.vdf`, from Desktop Mode with Steam closed. It goes through the library's safe save
  (refuse while Steam runs, verified backup, atomic write, read-back), then prunes old backups. This is
  the Desktop Mode route (chosen 2026-10-21; no Decky plugin).
- **Queue from Game Mode** (`steam_queue.rs`): Steam always runs in Game Mode, so there the review row
  is always "Queue N changes for Desktop Mode..." and nothing is ever offered or written, whatever the
  `/proc` check says (Game Mode = `XDG_CURRENT_DESKTOP` containing "gamescope" or
  `GAMESCOPE_WAYLAND_DISPLAY` set; unverified on a Deck; logged at startup as `game mode: yes|no (...)`).
  In Desktop Mode with Steam running the row is "Queue N changes until Steam is closed...". The choice
  (port key, Steam account id and label, whether a name-only match may be repointed, catalog art or
  none, queued in Game Mode or not) is saved to `steam-queue.json` next to the app (`"version": 1`,
  temp file + fsync + rename + folder fsync; an empty queue removes the file). Re-queuing a port for the
  same account replaces its entry.
- **Offer:** shortly after startup and then every 30 s, outside Game Mode, when no dialog is open, no
  game started from the app runs and the window is not hidden to the tray, or minimized on X11 (winit
  reports "minimized" as unknown on Wayland, so a hand-minimized window there can still get it), a non-empty
  queue with Steam closed brings up "N Steam changes queued from Game Mode: apply now?" ("queued while
  Steam was running" unless every entry came from Game Mode), with Cancel preselected. It names ports
  whose catalog art is not downloaded yet (the prefetch runs when the offer appears or the screen
  opens) and says Cancel keeps them queued. A declined offer comes back only after Steam was seen running
  and closed again (or on the next start); a skipped look leaves that memory alone. Log lines:
  `steam queue: offering N`, `steam queue: <problem>`. The screen also has an "apply now" row and a
  "View or remove queued changes..." list (remove one, after which the list reopens on Back; clear all,
  after which the screen reopens). Accounts are named when the device or the queue has several.
- **Applying** plans again from the current `shortcuts.vdf` and writes through
  `screens::apply_selected_with` (same Steam re-checks, backup, atomic write, read-back,
  `art::grid::write_art`), one write per account: an entry already in Steam counts as done; a port that
  left the catalog, was uninstalled or is Android only, an unknown account (when others exist), a blocked
  plan or a name-only match that was not approved when queuing is reported and dropped. If Steam comes
  back before the rename, only the entries that were going to be written for that account (and every
  entry of later accounts) stay queued. With no Steam account folder at all the whole queue stays; with
  an unreadable `shortcuts.vdf` that account's entries (and later accounts') stay, and nothing of theirs
  is dropped. An unchanged queue file is not rewritten.
- **Queue file problems:** one that can be read but not parsed (bad JSON, not text, unknown version, a
  bad entry) is renamed to `steam-queue.json.corrupt-<time>` and the user is told; the queue starts
  empty. One that cannot be opened (permissions, a folder in its place) or cannot be moved aside is left
  alone, and queuing and applying refuse until it is fixed, so it is never silently replaced. Not
  verified on a Deck: the whole flow, the Game Mode signals.
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

## Importing ports (`src/fork/import/`, `src/app/import_screen.rs`)

Settings > Import ports... replaces "Adopt installed ports" (2026-10-22: Tim, "it should just open to a
file select menu ... it needs to move them into the folder that was created during setup and update the
steam links"). It is also step c of the setup wizard, which opens it through
`import_screen::open(app, router, from_wizard)` (or `open_with(.., finish)` to be told when to go on).

- **Scan folders** (`folders.rs`): `~/Applications`, `~/Games`, `~/Ports` (each switchable) plus folders the
  user adds with the folder picker (`picker.rs`; no JSON is shown or opened), one level deep, optionally
  one more level into folders that matched nothing (hidden folders skipped, 400 folders at most). Saved in
  `fork-settings.json`: `adopt_folders` (the old key, now written by the picker), `import_defaults_off`,
  `import_look_deeper`.
- **Matching** (`adopt.rs`, 2026-10-23 after Tim's SD card showed only dead "fits N ports" rows): in order
  (1) folder name = a port's folder, name, name without its note, or project (last part of the GitHub/GitLab
  source); (2) release name: the folder's words (split on `-`, `_`, space, `.`; platform words like `linux`,
  `x86_64`, `appimage`, `steam` before `deck`, and versions dropped) are a Deck-ready download's words, or
  the first k words joined (longest first) are a port name (`Severed_Chains_Steam_Deck`, `Dusklight-Linux`)
  = `ReleaseName`; (3) first word only (`ReleaseWord`): a port name of 4+ letters
  (`Dusklight-twilight-princess`), a built-in alias (`soh`, `2ship`, `2s2h`: the list cannot cover program
  names or a never-fetched list), or the start of a Deck-ready download (`SoH-Dewey-Bravo.appimage`; one
  shared word counts only when no other project's downloads start with it); (4) a program inside, two
  levels deep: only files with a program extension (`.AppImage`, `.x86_64`, `.x86`, `.sh`, `.run`, `.bin`,
  `.elf`, `.exe`) or no extension and executable (on Windows, which has no such bit, any file without an
  extension except `README`, `LICENSE`, `COPYING`, `CHANGELOG`, `Makefile`, `AUTHORS`, `NOTICE`); hidden
  files never. ROMs, `.o2r`, JSON, images and saves never count, even with the executable bit (Tim's ROMs
  have it).
  **Ticking (Fable review 2026-10-23):** a match that rests on one word of the folder name (every
  `ReleaseWord`, and a one-word `ReleaseName` such as `Melee` or `Dusklight-Linux`; `Candidate::word`) is
  ticked only when a program right in the folder is named with that word (`soh.appimage`,
  `Dusklight-v2.0.3-linux-x86_64.AppImage`) or is the one the catalog names; otherwise "check this".
  Two or more words matched (`Severed_Chains_Steam_Deck`) are ticked. A program match is ticked only for a
  program right in the folder, so `Games` holding `Zelda/soh.appimage` is never ticked; when two folders fit
  one port the better match is offered (by name, then a program right inside, then a program further
  down) and the other is "Also found". Folders never looked at: hidden ones, `steamapps`,
  `SteamLibrary`, `lost+found`, `Emulation`, `compatdata`, `shadercache`. A folder that fits more than 3
  ports, or holds `roms`, `bios` or `steamapps` or a `libraryfolder.vdf` (unless named like a port), is a
  collection: one info row. Each find carries `why` in plain words ("folder name", "folder named like its
  download", "program soh.appimage", "you chose") for the card UI planned next.
- **Deck-ready list** (`targets.rs`): the saved list (or one fetch when it was never saved) adds download
  names as hints, and projects the catalog lacks (Severed Chains: upstream's catalog disables it) as extra
  entries built with `deck_ready::entry::local_entry`, the Deck-ready screen's "Add and install" entry
  (Deck build: `preferred_asset` = the Deck marker of the main release's file). Such a find is marked "new to
  your port list" (an info row explains it); Import writes the entry (`add_to_local_file`,
  `ports.local.json` backed up) just before the move, then moves the folder in like any other. If that port
  does not move (cancelled, refused, failed), the entry is taken off again (`targets::remove_new_entry`:
  only the entry exactly as written, file backed up first) and the summary says so. The fetch on the scan
  waits at most 10 s and Cancel stops waiting at once. When the list cannot be loaded, an info row says
  ports only on that list (like Severed Chains) are not recognised.
- **Test data**: unit tests use a pinned, trimmed copy of the catalog and the Deck-ready list in
  `testdata/import/`; `live_lists_still_match_tims_card_and_tick_no_ordinary_folder` (ignored in normal
  runs) checks the live `ports.json` and `index/deck-ready.json`, and `deck-ready-index.yml` runs it before
  committing a refreshed list (that push does not trigger CI).
- **Choosing** (`screen.rs` `ChooserScreen`, `PortListScreen`; `scan.rs` `Choices`): a folder that fits 2 or 3
  ports is a row "Choose which port this is: <folder> (could be A or B)...", and a top-level folder that
  fits nothing but holds a program (in it or one folder in) is "Not recognised: <folder>. Choose the
  port...". The chooser lists the matches, "Another port..." (every port, those sharing a word with the
  folder first, most shared words first) and "Not a port (leave it)"; a port already found in another folder
  says "(already found in <folder>)". A chosen folder is measured on the worker and becomes a ticked
  find ("you chose") that goes through the same plan and checks as any other; choices last while the screen
  is open (a new scan keeps them). Two folders for one port in one import: the second is refused
  (`Block::SameDestination`, naming the folder left out and the one that goes). The main list comes back
  on the row last chosen.
- **Finds** (`scan.rs`, on a worker thread with a Cancel row): the matcher of `adopt.rs` (never the library
  or a folder that holds it), plus a confidence (exact match = folder named like the catalog folder;
  likely; check this = a one-word or program match without a program named so right in the folder, starts
  unticked; you chose),
  the folder's size (links not followed, capped at 400,000 entries, "at least" when cut) and the Steam
  shortcuts of the active account whose program or start folder lies inside it (the shortcuts file is only
  read; a missing or broken one is a note).
- **Plan** (`plan.rs`): for each port, in words: check the game files, copy, verify, switch over, repoint
  the Steam shortcut (app ID kept, program and start folder re-rooted under the new folder) or queue it
  (Steam running, always in Game Mode, or not provably closed), keep the old folder. Refused before
  anything happens when the destination exists, the source is gone, the folder name is not one plain name,
  the two folders contain each other, or free space is under the port's size plus 64 MB, the mover's own rule (a batch counts the
  space earlier ports use).
- **Adapter and engine** (`engine.rs`, `real_engine.rs`): `move_selected(engine, plans, cancel, publish, prompts)`
  runs the plans one at a time, publishes a status line per port, checks the cancel flag between ports and
  returns an `ImportSummary` (moved, old copies on disk with sizes, Steam repointed/queued/failed, notes,
  problems). `MoveEngine` is what moves a port; `Prompter` is how it asks. `RealEngine` (the one the screen
  uses, `engine_for_import`) runs the port mover per port on the worker thread: `prepare` (checks, copy to a
  hidden staging folder, verify, game files checked before and after, rename into place), then `switch_over`
  (Steam shortcuts repointed with app IDs kept, or a queued `Repoint` while Steam runs and always in Game
  Mode; Game files records follow; the old folder recorded in `port-moves.json`). The mover's rules apply
  as written in "Ports folder, folder picker and moving ports" (space = size + 64 MB, nothing runs from the
  folder, old copy never deleted). A game file problem found before the copy is asked on the UI thread while
  the worker waits (the screen shows it between progress lists): Fix in Game files (skips that port; the
  summary then offers Open Game files), Move anyway, Cancel (stops the ports after it). Problems found
  after the copy always stop that port. After the batch the screen marks what moved as installed and runs
  `ports_folder_screen::prompt_old_copies` (Keep for now preselected; the summary ends that sequence).
  Scan folders go through `folder_picker_screen::open_folder_picker` (`Purpose::ScanFolder`) and
  `ForkSettings::set_adopt_folders`; imports go to `AppPaths::library_dir()`; Import refuses while
  `ports_folder_screen::blocked_reason` is set (ports folder missing, a move running) and while an install,
  update or restore runs; moved ports count as busy (`installing`) meanwhile, and Close or Quit cancels the
  copy in progress first (`import_screen::quit_waits_for_import`, two lines in `main.rs`). `DryRunEngine`
  (plan and report, write nothing) is kept for tests.
- **Fallback** (`fallback.rs`): a port refused for lack of space can be linked where it stands instead
  (`Library/<folder>` symlink plus a sourceless `ports.local.json` entry, backed up first; `adopt.rs` does
  the work and its rules stand: nothing in the install is touched, no updates through the link). It is
  offered only after the summary shows why, and asks first.
- Nothing is ever deleted by Import. The old folder stays after a verified move.
- After the batch: the old-copy questions (`ports_folder_screen::prompt_old_copies_then`: the stage 1
  sequence plus a hook that runs when it ends; B on a question keeps that copy and every one still to be
  asked, and the run ends normally), then the game file search for the ports that came in
  (`rom_screens::after_ports_added_then`, with the summary at the top of its list when it has something to
  look for; a continuation runs when its Done row, B, notices or progress list B are used), else the summary
  alone, or "Continue setup" in the wizard. The wizard's `finish` (`open_with(.., Some(finish))`) runs
  exactly once on every way out (Continue, the end of an import, B on the main list or the scan's progress
  list, any notice, a dialog that cannot be built; quitting excepted; `fork::import::Once` is the once-only
  guard). The game file question closed with the title bar's X counts as Cancel, and a question queued after
  the list was closed is answered Cancel, not asked. Cancel or B on a confirm, and B on the scan folders or plan lists,
  return to the previous list.
- What is refused before any copy (plan and mover both): a folder that is a link ("this is a link to X;
  import the real folder instead"), one that holds the home folder, the app's folder, the ports folder or a
  scan folder (`MoveError::Protected`, `Block::Protected`), an existing destination, not enough room.
  A match by program name only is never preselected directly under the home folder or below the first level.
- Shortcut matching is one rule for the scan, the plan, the repoint and the removal check:
  `port_mover::steam::moved_path` tries the path as written, then through links (the real path against an
  aliased scan folder or the other way round).
- Old copy removal for imports: the record carries `import_folder` (the catalog folder the port was imported
  as; written by `RealEngine` after `switch_over`, JSON key `import_folder`, absent means false). It relaxes
  only the folder-name equality in `removal_check` and in `finish_check`, and only when the new copy's
  name is exactly that folder; every other removal rule is unchanged (switched, new copy present and in use,
  not the same place, old folder holds no home/app/ports folder, no port installed there, top-level
  containment, links into it, queued repoints and shortcuts, running programs, newer saves).
- Limits: the old-copy questions are skipped (they stay under Old copies) when another dialog takes the
  screen while an old copy is being checked; a game file search closed with B on its progress list ends the
  flow. Not verified: a Deck (SD card paths, exFAT, Game Mode wording, controller feel).

## Linking an install in place (`src/fork/adopt.rs`)

The old "Adopt" code, kept for matching and for the link fallback of Import ports.
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

## Setup wizard (`src/fork/setup_wizard/`, `src/app/setup_wizard_screen.rs`)

The first start opens a short wizard instead of the plain list (Tim, 2026-10-22: "this one is really hard to
understand to start with"). Settings > **Setup wizard...** (first row) runs it again at any time, starting at
Welcome and showing the current values. English only, plain wording.

- **Page layout:** each step is one page (`ui/fork/setup-wizard.slint`): a header with the app name, "Step N of 6" and a
  six-segment progress bar; a title; wrapped body text; a summary card (what is set or found) and/or a short list of
  real options (radio choices such as the ports folder places, or actions such as "Find ports I already have...");
  a footer with Back on the left, Skip this step next to it where allowed, "Close the wizard" in a rerun (every page
  after the first), and the primary Next or Finish on the right. Focus moves only between enabled options and
  buttons (D-pad or arrow keys; text and disabled options never take it): Up and Down walk the options and drop to
  the primary button, Left and Right move between buttons; A or Enter activates; the primary button has the
  initial focus (on the "Skip setup?" question it is "Keep going with setup"; on the ports folder step with an SD
  card in and the default folder in use it is the card); B or Escape goes back; the title bar's X leaves the
  wizard. The mouse moves the highlight only when the pointer really moves: a pointer resting where a page opens
  (the first pointer event only records where it is, the highlight follows after a few pixels of movement) cannot
  take the highlight from the controller. If the text is long the body scrolls, never the buttons. The window is
  sized from the work area (about 62% wide windowed, 80% fullscreen, 88% high), not from the small windowed main
  window, and uses the app's theme colors and font sizes.
- **Steps:** Welcome, Ports folder, Ports you already have (import), Game files folder, Steam, All done (the header
  says "Step 1 of 6" to "Step 6 of 6" and the Welcome list uses the same numbers). Each page has Next and Back; the
  import, game files and Steam steps also have Skip this step; the ports folder always has an answer (the current
  folder, and its button says "Keep this folder" until a different one is chosen). B goes back one step; on Welcome
  B asks "Skip setup?". In a rerun, Welcome's exit button is "Close the wizard" and needs no question.
- **When it opens:** at start when `fork-settings.json` has no `"setup_done": true` (no file, or an install that
  never ran it, so existing installs get it once). A settings file that is not a JSON object never opens it by
  itself (it could never be saved as done and would return at every start); Settings still opens it. A start-up
  wait that outlives a run of the wizard from Settings (finished meanwhile) opens nothing.
- **`setup_done` is written only** by Finish on the last page, or by Skip setup after its question. Quitting
  half-way (closing the window, quitting the app) leaves it unset, so the wizard returns at the next start. If the
  save fails, the page says so and offers "Close anyway" (leaves without marking setup done) next to OK.
- **Ports folder step:** shows the folder in use ("Right now") and quick choices as radio options: usable removable
  drives first (`<mount>/Ports` from `fork::folder_picker::mounts::removable_drives`, SD cards before USB drives,
  marked "new folder" when missing, free space shown), then internal storage, then the current folder if it is
  neither, then a folder an earlier build of the wizard recorded as `setup_ports_folder` (offered once in the first
  run after this build, then the key is removed), then drives that cannot hold ports (exFAT, FAT, NTFS: listed with
  "(can't be used)", dimmed, not selectable), and "Choose another folder...". The folder in use is shown dimmed as
  "in use now" and cannot be chosen again. With no SD card (a USB drive does not count) the card says "No SD card found. Put one in and come
  back to this step (Settings > Setup wizard), or keep internal storage." Choosing an option hands over to the app's
  own ports-folder flow (`ports_folder_screen::consider` for a quick choice, `choose_folder` for the shared folder
  picker): the folder is checked (exFAT, permissions, system folders and the rest of `ports_folder::check`), created
  only after a confirm, installed ports can be moved there, and the old copies are kept until the user removes them.
  The page says so. The wizard comes back on the same step when those screens are closed.
- **Hand-offs:** the ports folder, import, game files, Add to Steam and Artwork sources steps open the app's own screens.
  Those do not know about the wizard, so while one is open the wizard waits, and once no dialog has been open for
  half a second it comes back at the same step. Their Back and Cancel rows that lead to a menu (Settings, Settings >
  Ports folder, Settings > Game files) come back to the wizard at once: `open_settings_dialog`,
  `ports_folder_screen::open_menu` and `rom_screens::open_game_files` ask `resume_instead_of_menu` first, which only
  acts while the wizard waits for the hand-off that leads to that menu (`flow::menu_returns_to_wizard`: the ports
  menu for the ports-folder step, Game files for the game-files step, Settings for any). The ports-folder flow
  opening Game files itself ("Fix in Game files") is therefore left alone. "Ports you already had: N moved" is the change in the number of
  installed ports across the import screen (the ports it moved into the ports folder). The import screen calls the
  wizard back through its `finish` (once, on every exit except quitting the app); the watcher also resumes the
  wizard if no dialog is open for a moment (it also waits while the import screen's worker is still running, `import_screen::import_running()`), and a second resume changes nothing; the finisher resumes only while the import is still the pending hand-off (`flow::resume_import`).
- **Adapters (the integration points):** `flow::Adapters` is everything the wizard needs from the rest of the app and
  the only thing the tests fake. In `setup_wizard_screen.rs`: `ports_folder_step` (calls
  `ports_folder_screen::{consider, choose_folder}`; the wizard keeps no ports-folder checks of its own) and
  `import_step` (the import screen: `import_screen::open_with(app, router, true, Some(finish))`; `finish` runs once when the user leaves it, whichever way except quitting, and hands control back to the wizard).
  `start_if_first_run` waits while `ports_folder_screen::startup_dialog_pending()` is true (the missing ports
  folder message is due or showing) or any dialog is open.
- **Not done by the wizard:** it never moves, deletes or overwrites anything itself. What the screens it opens do
  (including moving ports) is governed by their own rules.
- **Known limits:** if an unrelated dialog replaces the wizard's page (a Steam queue offer, say) the wizard shows the
  same page again (a skip question or failed-save page as it was) when that dialog is closed; only the title bar's X, the window's own close button, Finish or a
  confirmed skip end it (it returns at the next start unless setup is done). The shared folder picker opens on "Use
  this folder", which the ports-folder check refuses in the home folder (not changed here: the end-to-end move test
  in the smoke test depends on that starting row). Fullscreen layout on a real Deck is not verified (Xvfb has no
  window manager).

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
- **After an install (ROM search, 2026-10-22):** once per port, when the port names files it cannot run without and is not
  connected yet, the app looks for them in the game files folder on its own (`find.rs`; `rom_prompted_ports` in
  `fork-settings.json` remembers it, the same key the earlier offer used, so there is never a second prompt; a port is marked only
  once a result is on screen, so a search cancelled or abandoned with B is tried again after the next install or update). The folder is walked
  on the worker thread with progress and Cancel; only new or changed files are read (a folder that did not change costs one walk
  and no reads, and the saved scan is not rewritten). Found: the Connect step. Several files: a pick list per slot, then Connect.
  Not found: a list naming the port, each missing file with its CRC32 (or "checksum listed", or "only the game is named"), the
  folder and its game-file count, notes for a wrong version or the files that were found, then **Search again**, **Connect by
  hand...** and **Later**. No folder set: the same prompt with **Choose ROM folder...**. Folder unreadable (card out): **Search
  again** and **Choose another folder...**. Folder that holds no game files although the saved scan had some (an unmounted card
  leaves an empty mount point): "The folder looks empty. Is the card or drive plugged in?" with the same buttons, and the saved scan
  is not overwritten. Nothing when a launch follows the install (Play), when the port needs no files, or when
  a dialog or another game-files task is already up. Adopting ports does the same with one walk for all of them and a list of
  results under the adopt summary (`after_ports_added`).
- **Finding a port's files later:** the port's Info dialog has **Game files...** (installed ports that name files; beside
  Restore saves) and Settings > Game files has **Find game files for a port...** (installed ports that name files, not yet
  connected first). Both run the same search. **Connect by hand...** walks folders (starting at the game files folder), lists
  game files first, reads and checksums the chosen file (a cancellable "Checking the file" list): a listed file opens Connect; any other file says so with its CRC32 and
  offers **Use it anyway** (marked "matched by name, check this" in Connect; the "Game files connected" message adds "<file> was
  not checked against the catalog's checksum", as it does for any name-only match), Choose another file or Cancel. The saved
  connection does not record that it was not checksum-matched (Connected ports shows it like any other).
- **API for other screens:** `fork::rom_screens::find::find_game_files_for_port` (pure, any thread) and
  `app::rom_screens::{find_for_port, after_ports_added}` (the screens). See the module docs of `find.rs` and `find_flow.rs`.
- **Known limits:** B closes the whole flow from any list (each deeper list has a Back or Cancel row; the copy confirm's Cancel
  returns to the main window); an abandoned scan (B) stops within a second and a new job is refused until it has; a disc whose
  `.cue` lists a track that is missing is still reported Found (the cue's `FILE` lines are not read); the automatic search
  runs once per port (Later keeps the Info and Settings ways in); a disc picked by hand connects only the chosen file; an install that finishes while a scan list is up closes that list and so cancels the scan (upstream's
  `finish_operation`, same for the Steam screens); the browser cursor returns to the first row after each step.
- **Not done:** the BIOS folder
  setting and N64 byte-order conversion of R1's backlog; a startup health check of recorded connections (the list shows health
  when opened); "Use original" does not pass the path to the port, it only records it and says where the file is.
- **Not verified on a Deck:** fullscreen layout, controller input through Steam Input, SD card start points
  (`/run/media/...`), exFAT link refusal, copy speed on the SD card.

## Restore saves (`src/fork/save_restore/`, `src/app/save_restore_screen.rs`)

Upstream makes save backups (Settings > Backup Saves, and this fork adds one before every update) but
has no way to put one back. Info > **Restore saves...** and Settings > **Restore saves...** do that.

- **What is listed:** `Saves Backup/Update Backups/<folder>/*.zip`, the port's part of every
  `Saves Backup/Global Backups/*.zip`, and `Saves Backup/Restore Backups/<folder>/*.zip` (made by a
  restore, so a restore can be undone). Newest first by file time, with size and the save folders
  (`save`, `save2`) each holds. The zip's table of contents is read directly (no unpacking); a zip
  that cannot be read, holds a name with `..`, a leading `/` or `\`, a drive letter, or a link entry is
  listed as "cannot be used".
- **Which save folder:** backup and restore share one resolver, `platform_resolve::save_location`
  (O1), so a restore puts files back in the folder the backup read them from. Its rules: a plain
  relative folder inside the game counts on every system, also when the catalog lists it only under
  `windows`, and on Linux its `\` separators become `/`; a value naming only another system's real
  path (a Windows drive or `%VARIABLE%` path, an Android path) or a `%VARIABLE%` not set here is
  "unknown on Linux", shown as "no save folder known for Linux", and no restore is offered; an empty
  value "does not say where it keeps its saves". On top of it the restore refuses only folders that
  are far too broad: the game folder itself or anything holding it, a relative path leading out of
  the game folder, the home folder or anything above it, anything overlapping `Saves Backup`, and a
  path with fewer than two folder levels. A contract test runs `backup_before_update`, the list and a
  restore for `{"windows": "saves\\slots"}` and `"save\\card0"`.
- **Order of a restore:** refuse while the port's game runs: the app's own record of a game it
  started, and on Linux any process whose program file or working folder (`/proc/<pid>/exe`, `cwd`)
  is inside the game folder or a save folder (the refusal names it). Also refused while the port
  installs or another restore runs, and when a drive is known to be too full (about twice the
  current saves plus the backup in `Saves Backup`, the backup's size beside each save folder;
  unknown free space does not refuse). Read the zip's names again from disk and refuse on any unsafe
  one; zip every current save folder of the port to `Restore Backups/<folder>/<stamp>.zip` and check
  it reads back with every file (refuse to go on if that fails); unpack with 7-Zip (`--` before the
  paths) into a scratch folder in `Saves Backup`, refusing on any link and on any file count or size
  that differs from the zip's table; copy each folder to a hidden sibling
  (`.<name>.restoring-<pid>`), then rename the old folder aside (`.<name>.before-restore-<pid>`)
  and the new one in, for every folder; if a rename fails, every folder already swapped is put back.
  Only after every swap worked are the old folders removed. A save folder that is a link keeps the
  link (its destination is replaced). A save folder that does not exist yet is created (its parent
  folders too). A whole-library backup is unpacked only for the port's folder, unless the folder
  name holds `*` or `?` (then the whole zip is unpacked; every name in it was checked first).
- **While it runs:** the port's install slot is held, so Play, the tray's recent games, Install and
  Update of that port wait; window Close (when it would quit) and tray Quit wait for the restore and
  then quit; Backup Saves, Restore saves, Force update and the footer Update say to wait.
- **Interrupted restore:** if the app is killed or the Deck loses power mid-restore, the hidden
  siblings stay. The restore list shows each one at the top ("A restore was interrupted: your
  previous saves are in ...", or "An unfinished restore copy was left at ...") with what to do; the
  app never deletes them. Scratch folders inside `Saves Backup` (`_restore_*`) are removed when a
  list is opened once they are a day old (they only hold copies).
- **Known limits:** a link to a folder inside a save folder is not in the safety zip, so that save
  folder's old copy is kept beside it (named in the result) instead of being removed. An interrupted
  restore is only noticed when that port's restore list is opened (not at startup). The
  running-game check beyond the app's own record is Linux only. The "Pending Restore" copies kept
  across an uninstall are not listed (they are put back by the next install). Backups are never
  pruned. Text is English only. Not yet tried on a Deck.

## Ports folder, folder picker and moving ports (`src/fork/ports_folder.rs`, `folder_picker/`, `port_mover/`)

- **Setting.** Where installed ports live (upstream's `Library/` next to the app) is `ports_folder` in
  `fork-settings.json`. Not set means `<app folder>/Library`, as before, so existing installs behave the
  same. `main.rs` builds `AppPaths` from it; every user of the folder reads `AppPaths::library_dir()`.
- **Missing at startup.** A chosen folder that is not there (SD card out, drive not mounted) is never
  created and nothing is marked installed or uninstalled: `state.json` is left alone. The app first shows
  "Your ports folder ... is not available" with "Try again (the card is in now)", Choose another folder...,
  Continue without it and Quit; the Steam queue offer (and any other startup dialog that asks
  `startup_dialog_pending`) waits while it is due or showing. While the folder is missing (or a move runs), installs, updates, adopting and
  save restores are refused with a message (`refuse_when_blocked` / `blocked_reason`), so nothing is ever
  created at an unmounted ports folder. Ports there show as not installed until it is back (they are not
  forgotten). The default folder is re-created when missing, as upstream did. The startup log line names
  the folder and the drive folders looked at.
- **Drives.** Removable drives are the mounts directly in `/run/media/<user>` (from `USER`, and from the
  home folder's name when Steam starts the app without `USER`), `/run/media` or `/media`, and any mount one
  folder deeper (`/run/media/<x>/<label>`) whose middle folder is not a mount itself. Labels with escaped
  characters (spaces, accents) are decoded; several SD cards are listed.
- **Choosing a folder** (Settings > Ports folder... > Choose another folder...) goes through the folder
  picker, then `ports_folder::check`: a full path; not a system folder, the home folder itself, the top of
  a drive, inside the app folder (except the default `Library`), a folder holding the app folder, or
  inside the current ports folder; not exFAT, FAT or NTFS ("This drive is formatted as exFAT, which can't
  store Linux programs ... Ports need a card formatted by SteamOS: Steam > Settings > Storage > Format.
  Formatting ERASES everything on the card, so copy your game files off it first."); writable, and
  executable bits and symlinks stick (tried in a temporary sub-folder, removed again; a failure on an
  unknown file system says "This folder can't keep program permissions, so ports could not start here.
  Pick a folder on the internal drive or on a card formatted by SteamOS."). A missing folder is created
  only after the user confirms, and never where a drive is not mounted. Cancel on these questions goes
  back to the Ports folder list.
- **Folder picker.** In-app, controller first (no native dialog; those misbehave in Game Mode). Quick
  places: each mounted removable drive by label with free space (an exFAT/FAT/NTFS drive says "exfat:
  can't hold ports" when choosing a ports folder), Home, `~/Games`, `~/Applications`, plus places the
  caller adds (Game files adds ROM folders). Rows: Use this folder, New folder here... (creates the folder
  at once; name rules: no `/`, no leading dot, not `.`/`..`, at most 255 bytes, no leading or trailing
  spaces, not already there), Up, sub-folders (sorted, hidden ones on request, at most 500 listed), Cancel.
  Header: path, free space, file system, drive, and a hint or the reason the folder can't be used.
  Whether a folder is writable is tried once per folder. `open_folder_picker(app, router, PickerRequest,
  on_chosen, on_cancel)`.
- **Moving the installed ports** when the folder changes (`port_mover`). Never deletes: "move" is copy,
  verify, switch over, keep the old folder. Refused while a program runs from a port being moved (save
  restore's `/proc` check) or a game started from the app runs (also for Switch without moving), while
  any install, update or restore runs, or when space is short (size + 64 MB). The copy goes to a hidden
  staging folder next to the target (leftover staging folders of runs that are gone are swept first),
  keeps permission bits, links (as links) and times, syncs each file, then is verified (file list, kinds,
  sizes, permission bits, link targets, modification times; every file read back and compared by SHA-1
  when the port is up to 4 GB, else programs and game files). Only then is it renamed into place. Any
  failure or Cancel removes only the staging folder. An adopted port (a link in the ports folder) moves as
  a new link to the same folder. Cancel is checked between files and after every 1 MB copied; ports already copied in that run are
  removed again (their originals were never touched) and the setting does not change. If the target or
  source drive goes away during the batch (card pulled out), it stops at once with one message and the
  copies made so far are removed. When some ports fail, the user picks Switch anyway (the others stay in
  the old folder and show as not installed), Fix in Game files or Cancel. Close and Quit during a move
  stop it first (copies removed), then quit; during an old-copy removal they wait for it.
- **Moving onto a kept old copy.** When the target already holds a folder that `port-moves.json` lists as
  an old copy the user kept, the switch screen says "<port>: an old copy you kept is already there; the
  copy at <current> is the one in use" and offers only "Replace the old copy with the current one, then
  move" or Cancel; Switch without moving is not offered, so the stale copy is never used silently. Before
  the old copy is removed (`old_copies::replace_check`): it must be what was recorded (folder or link),
  not a protected folder (the ports folder in use, the app folder, home, or a folder holding the copy in
  use), nothing may run from it, and it must not have saves newer than the copy in use ("The old copy at
  <path> has newer saves (last played <date>) than the copy in use. Copy the save files from <old save
  folder> into <new save folder>, or keep the old copy and cancel."). A refusal stops only that port.
  Then its saves are zipped to `Saves Backup/Update Backups/`, it is removed and its record dropped. A
  confirm says so before anything starts: the kept copy is removed first, so if the move then fails it is
  already gone (the copy in use is never touched).
- **Game file check** (`port_mover::game_files`). Before copying: every connection recorded by Game files
  for the port must be healthy (for a copied game file only the copy in the port folder counts; a link or
  "use original" also needs its original), and every link in the port folder that points outside it must
  resolve. Files the port's entry names (`Required files:`, `rom_files`) that are neither connected nor in
  the folder never stop a move (many are only needed once at install time, like an archive or a ROM turned
  into an `.o2r`); the move summary mentions them as information. Problems are listed with the file and
  where it was expected, with "Fix in Game files (then start the move again)", Move anyway or Cancel.
  After copying: links that leave the folder must reach the same file from the new place, no link may
  still point into the old folder, links recorded by Game files must reach the original, and copied game
  files must have the same size and, where a CRC is known, the same CRC (they are always read back by hash
  too). A mismatch after the copy always stops that port's move (the copy is removed); there is no
  "anyway" for it. A relative link that points outside the port folder (`../../roms/x.z64`) gets the
  absolute path of what it pointed at, and an absolute link into the port folder itself, or a relative one
  that leaves the folder on its way back in (`../../SoH/saves` from `SoH/sub/`), becomes a plain relative one,
  so both keep working at the new place; other links stay as written.
- **Switch over.** The old-copy records are written first (marked "switch not finished"; if the app closes
  before Steam follows, Settings > Ports folder shows "Finish switching <port> (Steam and game files still
  point at the old copy)", which runs the rest of the switch for that record after checking it: the new
  copy exists and is in use, the two places differ, share their folder name, are not inside each other,
  and the old one holds neither home, the app folder nor the ports folder; otherwise nothing changes),
  then the setting
  is saved, then for each moved port: every non-Steam shortcut in every Steam account whose program, start
  folder or icon is inside the old folder gets the same path under the new one, app ID kept (artwork and
  play time stay attached), through the safe write (refused while Steam runs, verified backup, atomic
  write, backups pruned). While Steam runs (always in Game Mode) a `repoint` entry goes into
  `steam-queue.json` and is applied with the rest of the queue, before adds and updates; a second move of
  a queued port chains (A to B, then B to C, is A to C) and a move back cancels it. The summary then says:
  "Until it is applied (next time this app runs in Desktop Mode with Steam closed), starting <port> from
  Steam still runs the OLD copy, and anything you save there stays in the old copy." Launch options are
  not rewritten. A shortcut with no stored app ID is left alone and named (Steam would derive a new ID from
  the new path). Game files connections (`rom_library.json`) follow, and the record is marked finished.
  `state.json` (install records, favorite program, play time) and `ports.local.json` need nothing: they
  are keyed by port and hold paths relative to the port folder.
- **Old copies** (`port_mover::old_copies`, `port-moves.json`). The app asks right after a move, per
  port, "Remove the old copy now?" with the old path, its size and where the port runs from now: Remove
  old copy (its saves are zipped to `Saves Backup/Update Backups/` first) or Keep for now, which is the row
  selected when the question opens. Kept ones are listed under Settings > Ports folder > Old copies.
  Removal is refused unless: the switch finished; the new copy exists, has the same folder name, is the
  one in use, and the old copy still holds everything at the top of the new one; no link in the new copy
  reaches into the old one; no Steam shortcut points into it (program, start folder, icon or launch
  options) and no repoint for the port is still queued; no program runs from it; it is not the app
  folder, the home folder or a folder holding a ports folder in use; the port is still in the catalog (so
  its saves can be found); and the old copy has no saves newer than the new copy's (otherwise: "The old
  copy has newer saves (last played <date>). Copy the save files from <old save folder> into <new save
  folder> (or play once from the new copy to check), then try again; or keep the old copy."). A shortcut
  Steam has not finished setting up (no stored app ID) is named in the refusal ("open Steam, quit it, then
  try again"). The zip comes first; if it fails nothing is removed. An adopted port's old link is removed
  as a link; its target folder is never touched. The check walks the old copy, so it runs on the worker
  thread behind a "Checking the old copy..." note; only that note is closed when it ends, and one check
  runs at a time (a second request says the old copies stay listed under Old copies).
- **API for other screens** (import, and later screens): `folder_picker_screen::open_folder_picker`,
  `fork::ports_folder::{current, configured, save_choice, check, create, at_startup, plan_move}`,
  `port_mover::{prepare, record_pending, switch_over, finish_switch, discard, run_batch, sweep_staging}` with `MoveEnv` /
  `MoveOptions` / `game_files::GameFiles::for_port`, `port_mover::old_copies::{load, removal_check, replace_check, remove,
  forget}`, `ports_folder_screen::{choose_folder, consider, prompt_old_copies, open_old_copies,
  refuse_when_blocked, blocked_reason, startup_dialog_pending, quit_waits_for_move}`, the list models in
  `fork::ports_folder_view`, `ForkSettings::set_adopt_folders`.
- **Known limits.** The move copies on a worker thread; the Steam repoint, records and old-copy prompts
  run on the UI thread after it (fast). Removing a large old copy runs on a worker thread with a progress
  dialog that cannot be cancelled. Hard links inside a port are copied as separate files. Folder times
  are best effort. Ownership is not copied (same user). Launch options naming the old folder keep it
  (the old copy is not removable) until edited in Steam. English only.


## Save finder (`src/fork/save_finder/`, `src/app/save_finder_screen.rs`)

Update backup, Save Backup, Restore saves and the uninstall preserve only work where the app knows a port's save
folder. Some recomps do not follow the catalog (Severed Chains added from the Deck-ready list has no save field;
Dusklight's names only a Windows path). Tim, 2026-10-24: show a discreet warning on the game, and let a short wizard
find the saves.

- **State per installed port** (`state_of`): saves found (a known folder that holds a file), the known folder is empty,
  the known folder is missing, no folder is known on Linux (no save field, or only a Windows or Android path), or the
  user said the game has no saves. The game folder itself or a folder holding it never counts as a save folder.
- **Badge:** installed ports whose saves are unknown always show a small amber "Saves not found" badge (list row and
  card); an empty or missing known folder shows it only once the game was played (a game that never ran has nothing to
  save; in a window too narrow for the words the badge is a small amber "!"). No popups. A mouse or touch click on the
  badge opens the wizard; a controller uses Info (X in Game Mode). A port's state is
  worked out at most every 8 seconds and forgotten when an answer is stored or cleared.
- **Info:** a line ("Saves not found: unknown on Linux", "Saves not found: folder is empty",
  "Saves: using the folder you chose", "Saves: you said this game has none") and a button **Find saves...**, shown when
  the saves are not found or an answer is stored.
- **Wizard** (the page framework `fork::page`, same window as the setup wizard): (1) "Where does X keep its saves?":
  Look for them for me (recommended), Pick the folder..., This game has no saves; with an answer already stored the
  page shows it and offers Look again, Pick another folder and Use what the catalog says. (2) The search page with
  Cancel, then "Is one of these X's saves?" with a card per folder (folder name, file count, last changed, where) and
  None of these (goes to the shared folder picker), or "I could not find X's saves" with Pick and No saves. (3) "Saved"
  with Back up now (recommended; the same Update Backup that Restore saves lists, zipped to
  `Saves Backup/Update Backups/<folder>/`), Not now and Undo (puts back what was stored when the wizard opened).
- **Search** (`search::find`, read only, never writes or follows a link into another folder): the port's own folder
  three levels down for folders named like "Save"; `~/.local/share`, `~/.config`, `~/Documents` and hidden folders of
  the home folder (`~/.dusklight`) one and two levels down (`Company/Game`); every Proton prefix under Steam's
  `compatdata` (the Deck's Steam folders and `steamapps` on a card under `/run/media`) at
  `drive_c/users/steamuser/{AppData/Roaming,Local,LocalLow,Documents,My Documents,Saved Games}`. A folder is a
  candidate when its name fits the port (`adopt::port_names`, `adopt::shared_words`, the import scan's words) and it
  holds a save-like file (`.sav .srm .sra .eep .mpk .fla .save .sl2 .sgd .dat`, a file with "save", "slot" or
  "profile" in its name, or any path with "save" in it). A folder that is another catalog port's own name is never
  offered (Ship of Harkinian does not get 2 Ship 2 Harkinian's saves), nor anything in Saves Backup or in another
  port's folder in the library. When every save file sits in one "Save" sub-folder, that sub-folder is offered. At most
  6 cards, newest first. Bounded: 25 seconds, 60,000 directory entries, 4,000 per candidate; Cancel stops at the next
  folder.
- **The stored answer:** `save_overrides` in `fork-settings.json`, keyed by the port's catalog folder:
  `{"path": "<form>"}` or `{"no_saves": true}`. The path is stored portably: relative to the game folder when inside it
  (it follows the game when the ports folder moves), `%HOME%/...` when inside the home folder, else absolute (the same
  forms `platform_resolve::save_location` reads). A name with `%` in it is refused. A chosen folder goes through the
  Restore saves guards first (not the game folder, the home folder, the top of a drive or Saves Backup). The override is
  applied when the catalog loads (start-up, a catalog update, a local entry added): a folder replaces both `save` and
  `save2`, "no saves" empties both, so the update backup, Save Backup, Restore saves, the uninstall preserve and the
  Info save buttons all read it. Clearing it re-reads the catalog entry from disk, so the catalog's own answer comes
  back. Nothing is moved, copied into or written in a save folder.
- **What a chosen folder may not be** (`fork::save_restore::folder_problem`, shared with Restore saves, so a hand-edited
  override gets the same refusal): the game folder or a folder holding it, anything in or around Saves Backup, the home
  folder, a folder that holds many apps' files (another port's folder in the library or the library itself,
  `~/.local`, `~/.local/share`, `~/.config`, `~/.steam`, `~/.var`, `~/.cache`, Documents, Desktop, Downloads, Steam and
  `steamapps` folders, `compatdata`, `compatdata/<n>`, `pfx`, `drive_c`, `users`, `users/<x>`, `AppData` and its
  `Roaming`/`Local`/`LocalLow`, Saved Games, My Games, and mount roots `/run/media/*/*`, `/media/*`, `/mnt/*`) and
  anything near the top of a drive. The finder says "That folder holds many apps' files, not this game's. Open the
  game's own folder inside it." A link is judged by where it leads (the finder when it is chosen, Restore saves again
  before the swap), and a folder over 2 GB is refused as "very large for saves". The folder picker the finder opens
  never starts on "Use this folder". "Back up now" and the update backup check the free room first for a save folder
  outside the game folder, and the Saved page shows the size.
- **Links and the update backup:** Restore checks a linked save folder's destination before the safety backup copies
  anything; the update backup refuses (no copy) a save folder that holds many apps' files and a link that leads to one,
  and "Back up now" B/X are held for 2 minutes while it runs, then leave it running with a message.
- **Known limits:** a catalog entry adopted into `ports.local.json` while an override is active carries the
  overridden `save` value into the local entry (harmless; it is the same folder). The uninstall preserve only copies
  saves that are inside the game folder (as upstream does); a folder outside it is simply left in place. Not yet tried
  on a Deck; Severed Chains and Dusklight are on the Deck checklist.

## Versioned program files (`src/fork/versioned/`, `src/app/versioned_update.rs`)

Some releases put the version in the file name (`Dusklight-v2.0.3-linux-x86_64.AppImage`). An update
lands the new file beside the old one. Upstream left both; until 2026-10-24 this fork deleted older
AppImages and left the Steam shortcut pointing at a missing file. Now, for any port:

- **When:** after the update is unpacked and the saves are back (`apply_archive` records the folder's
  names and launch target from before), the app finishes it on the UI thread. A failed unpack records
  nothing, so nothing moves.
- **Same family:** the same words once version and platform words are dropped (`adopt::release_tokens`),
  both names with a real version (`v1`, `1.0`, `v2.0.3`; a bare `2` or a `(1)` copy is not one), and
  the same kind (file or folder). Of several new files of one family, the closest name wins (fewest
  differing parts, so `-aarch64` follows `-aarch64`); a tie leaves the old file with a reason.
- **What moves:** program files (`.AppImage`, `.x86_64`, `.x86`, `.sh`, `.run`, or an ELF or `#!` script
  by content; never `.bin`). A versioned release folder moves only when (a) every save field of the
  port resolves to a known folder on this system (none listed, or one unknown on Linux such as
  `%APPDATA%`, keeps every folder in place, since the saves may be inside), (b) it holds at least one
  program file, and (c) it holds no link, no `saves*` or `mods` folder and no file of the keep list
  (settings `ini cfg conf json toml yaml yml xml`; saves and keys `sav srm mcr mcd sol db keys dat`; game
  data, archives and disc images `o2r otr bin img iso cue chd gcm rvz wbfs pbp pkg wad cia 3ds zip 7z rar`;
  ROMs `z64 n64 v64 nds gba gbc gb sfc smc nes xci nsp`). Never moved: a save folder (`save`/`save2` as
  resolved here), anything holding one, hidden names, names the new release also has. When the saves
  are the game folder itself, program files still move but folders stay.
- **Which updates:** when the old launch target (favorite, else what the app found before the update)
  is itself superseded, its counterpart in the new release must exist or nothing moves. When the launch
  target is not superseded (a launcher script as favorite, or several programs so the app had to ask),
  superseded files still move, because that is exactly the case where the old file would otherwise stay
  and keep being started (by the script, or by a Steam shortcut to it). But when the launch target is a
  script (`.sh`, or `#!` at the start; any depth) that would still contain an old name after the rewrite
  (a script in a subfolder or in the release is never rewritten, and a match the rewrite refuses stays),
  that entry stays in place: "The old version (v1.0) was left where it is: the launcher bin/run.sh still
  names it." Identical lines are shown once.
- **Order:** for each superseded entry, queued repoints of the port that still end at it are
  re-aimed at its successor (`steam_queue::retarget_repoints`, matched by path whichever port queued it;
  a change that cancels out is dropped), so
  a change queued by an earlier update never points Steam into `.previous`. Then Steam shortcuts are
  repointed (`port_mover::steam::repoint_everywhere`: every account, app ID kept, backup first, never
  while Steam runs; queued in `steam-queue.json` while it does or in Game Mode; a release folder is
  repointed only when it holds the launch target). An entry any shortcut still points at stays where it
  is: when its change is really in the queue it waits, and `after_queued_repoint` moves it (and updates
  launcher scripts) when the change is applied; otherwise the summary names the shortcut. Then each
  entry goes to `.previous/<old version>/` (`-2`, `-3` when the name is taken; moved, never deleted).
- **After:** top-level `.sh` launchers that are not part of the release get the new name wherever the
  old one stands alone as a name in the port folder (at the start, after a separator or quote, `./`,
  `$VAR/`, `${VAR}/`, `"$(...)"/` or a path whose last folder is the port folder, such as
  `/home/deck/.../dusklight/` or `"$HOME/.../dusklight/`, and with no name character after it), so
  `~/.local/share/Game-v1.0`, `Game-v1.0-saves` and `Game-v1.0.cfg` are left alone; the original is kept
  as `.previous/<version>/<script>.before-update`. Nothing is ever deleted: every superseded version
  stays in `.previous/`. The favorite follows the new file; one is set when the app would not find the
  new program on its own.
- **What Tim reads:** "Dusklight is now v2.0.4. The version you had (v2.0.3) is kept in a hidden folder
  (.previous) inside the dusklight folder; to go back, use Info > Select version." While Steam runs:
  "Steam is running, so the shortcut change waits. Close Steam (or come back to Desktop Mode) and this
  app will ask to apply it; until then Steam starts the old version, which stays in place." When older
  versions are there: "Older versions in .previous use N MB (in Desktop Mode's file manager, Ctrl+H shows
  hidden folders); remove them from the port's folder if you need the room." (GB above 1000 MB). Kept
  entries are named by version, not file name. When Play started the update the game launches at once,
  so a summary that needs attention (something waits, was kept, or Steam skipped a shortcut) is saved in
  `update-notes.json` and shown once no game runs and the window is in view (after the game exits, or at
  the next start), before the "Steam changes queued" offer. No delete button.
- **Going back:** Info > Select version installs an older release; the same steps keep the newer one in
  `.previous/`. Offline, by hand: in Desktop Mode move the file out of `.previous/<version>/` into the
  port folder and pick it in Info > Favorite executable (`.previous` is never searched otherwise).
- **Known limits:** a catalog `executable` naming a versioned file is not rewritten (the favorite
  covers it); launcher scripts in subfolders are not rewritten; a release folder holding a settings
  file, or of a port whose save folder is unknown here, stays beside the new one (named in the summary).

## Cloud backup (`src/fork/cloud/`, `src/app/cloud_screen.rs`)

Optional copy of the save backups in `Saves Backup` to the user's own Google Drive (design and decisions in the
project notes). Off until the user signs in under Settings > Cloud backup; nothing runs while the app is
closed. Privacy: [PRIVACY.md](PRIVACY.md).

- **Files.** `cloud-tokens.json` holds the sign-in tokens and the account email, mode 0600, and nothing else the
  app writes holds either. `cloud-backup.json` records what was uploaded (path, size, modified time, SHA-256,
  cloud path, times, last failure as a code word); it holds no secret and is safe in a bug report.
- **What goes up.** Only zips: `Global Backups/<date>.zip`, `Update Backups/<folder>/<stamp>.zip`, `Restore
  Backups/<folder>/<stamp>.zip`. Never `Pending Restore`, scratch folders (`_backup_*`, `_update_backup_*`,
  `_restore_*`, `_cloud_download_*`), links, hidden or empty files.
- **Readiness rule.** A zip is taken only when it reads back as a zip (its central directory is at the end, so a
  half-written file does not) and has not changed for a few seconds: 3 s for a pass started by a backup the app
  has just reported finished or by Back up now, 60 s for a pass nobody asked for after a specific backup (app
  start, retry, the ten-minute check). A zip that is not ready is looked at again about 70 s later; one that
  still does not read back after 10 minutes is called broken and never uploaded.
- **Never replaces, never deletes.** An upload goes to a name that is not taken. A zip rewritten the same day
  (a second Backup Saves) goes up beside the old copy as `<name>-<first 8 of SHA-256>.zip`. A cloud file with the
  same name, size and SHA-256 is recognised, not uploaded again; one that reports no checksum is never taken for
  the same file (two deflated save zips of one size are realistic), so the new zip goes up beside it. The service
  trait offers list, upload and download only; there is no delete or trash call.
- **Sign-in.** The OAuth device flow (code and QR, finished on a phone), scopes `openid email drive.file`
  only. Signing out, or signing in again, forgets the upload record of that service (it may have been another
  account); the next pass recognises what the cloud already holds. Signing out deletes `cloud-tokens.json`
  when no service is left. Only `invalid_grant` means signed out: a 401 or `invalid_client` from the token
  endpoint is this build's own credentials being wrong and never touches the stored sign-in. The account email
  comes from the id token when there is one, else from Google's userinfo address. Sign-in is refused, before a
  code is shown, when the app's folder is on a drive that cannot keep the token file private (exFAT or FAT);
  the token file is made private before it is read.
- **Google Drive** (`google.rs`). Layout `Deck Ports Manager/<Deck name>/Saves Backup/<same folders as on the Deck>`,
  found by name before anything is made (so a second run never makes a second copy). A file up to 5 MB goes up in
  one multipart request, a bigger one by a resumable session in 8 MiB pieces; after a dropped connection, a 5xx or a
  lost answer the app asks the session where it stands and carries on from there (five tries in a row without
  progress, then it gives up with the failure's kind). The session address is a credential: it is never logged; the
  pieces and the status questions carry the bearer token too, and a 401 on one goes through the same one-refresh rule. Listing always asks for `sha256Checksum` and follows pages. A 401 makes one forced
  token refresh and one more try; a 401 after that is signed out. Drive's code words (`storageQuotaExceeded`,
  `rateLimitExceeded`, `userRateLimitExceeded`, `dailyLimitExceeded`) and `Retry-After` pick the failure kind. The
  code never deletes, trashes or changes a file (a test reads the source for `DELETE`, `PATCH`, `trashed`, `/trash`).
- **Who may write the record.** Every sign-in has a name (`signin` in `cloud-tokens.json`, not a secret and not an
  account id; a token refresh keeps it). A pass or a copy back takes the name when it starts, and the store refuses
  its writes once the name is no longer the current sign-in (signed out, or signed in again as someone else). The
  pass merges its entries with what is in the record (a copy back may have added some meanwhile) and a copy back
  only adds entries, so neither undoes the other.
- **What reaches logs and pages.** Every cloud log line is cleaned of web addresses (an upload session address
  acts like a key), control characters and the tokens and email this process has held; the "Details:" line and
  panic texts shown to the person go through the same `cloud::log::scrub`. A download is cut off before it writes
  more bytes than the size Drive reported.
- **Build credentials.** `DPM_GOOGLE_CLIENT_ID` and `DPM_GOOGLE_CLIENT_SECRET` are compiled in from repository
  secrets for a push to `main` only (never branches, pull requests or the upstream-sync build). A build without
  them says "Cloud backup isn't set up in this build". The mock-server and demo-folder switches
  (`DPM_CLOUD_TEST_BASE`, `DPM_CLOUD_DEMO_DIR`) exist only in test and debug builds.
- **New crates.** `sha2` (MIT or Apache-2.0) for the checksum and `qrcode` with default features off (MIT or
  Apache-2.0) for the sign-in QR code, plus the RustCrypto pieces `sha2` needs (all MIT or Apache-2.0).

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

## Online art sources and credentials (`src/fork/art/`, `src/fork/secrets.rs`)

Settings > **Artwork sources...** fills Steam's hero, logo and wide slots (and, when the catalog has no usable
image, the portrait and icon) from SteamGridDB, TheGamesDB and ScreenScraper. Source credits are in that screen
("Credits and what each source does") and in `STEAM_DECK.md`.

**Credentials, and the rule behind them.** Keys are never committed and never compiled into a public build unless a
build injects them from CI secrets later.

- **User keys** live in `secrets.json` beside `state.json`, not in it (bug reports ask users to send `state.json`):
  `steamgriddb_api_key`, `thegamesdb_api_key` (optional), `screenscraper_ssid` and `screenscraper_sspassword`
  (optional). Created with mode 0600, written through a temp file that already has that mode and renamed (atomic), an
  existing file with wider permissions is tightened on load, a file that is not a JSON object, or that is a symbolic
  link, is never read or overwritten. Some drives cannot restrict a file at all (exFAT or FAT on an SD card report every
  file as open to everyone): that is checked after each save and the screen says "This drive cannot restrict files". The
  screen shows only the last four characters; `Debug` output hides every value; no error text, note or saved file
  carries a key (tests plant keys and search every saved file and message). Nothing exports or reports the file.
  `secrets.json` is in `.gitignore`, and the updater never touches it.
- **Developer credentials** (ScreenScraper's `devid`, `devpassword`, `softname`; an app TheGamesDB key) come only from
  `option_env!` values read at compile time: `DPM_SS_DEVID`, `DPM_SS_DEVPASSWORD`, `DPM_SS_SOFTNAME`,
  `DPM_TGDB_APIKEY`. No build sets them today, so ScreenScraper says "not configured in this build" and TheGamesDB
  works only with the user's own key. **To add them later:** register the app with each service, store the values as
  GitHub Actions secrets, and set them as environment variables on the compile step only (`DPM_SS_DEVID: ${{ secrets.X }}`
  under that step's `env:`). Do not echo them, do not put them in a step summary, and keep logs of that step quiet;
  anything in a shipped binary can be extracted, so treat them as low value. Set them on the release compile step
  only: a test run that also gets them would not break anything, but it has no reason to carry them.
- **Never add a logger without filtering `ureq` below `debug`.** TheGamesDB and ScreenScraper carry their keys in the request
  address, and `ureq`'s debug logging prints addresses. The art code itself never logs (its error text is fixed phrases, cut
  to 200 characters, with addresses and known secrets removed), so today nothing reaches a log.

**What each source does.**

- **SteamGridDB** (user key, Bearer header): title search, then grids, heroes, logos and icons by game id with the slot's
  `dimensions`, `types=static` (no animated art), `nsfw`, `humor` and `epilepsy` all off. 401 and 403 mean a bad key,
  404 no such game, 429 "too many requests" (the source is left alone for 5 minutes, not until midnight). No published rate limit; the resolver makes a handful of calls per port, one at a time.
- **TheGamesDB** (`apikey` in the query): `ByGameID` (or an exact-title `ByGameName` on the PC platform), `Images`. Takes
  the `original` file when its `resolution` is big enough (the `large` size is upscaled from small originals), skips images
  that would need more than 1.5x enlargement, reads `remaining_monthly_allowance` and `extra_allowance` from every answer
  and stops at zero. 403 means a bad key or a spent allowance. Never asked without an id or a key.
- **ScreenScraper** (`jeuInfos.php` by ROM hash + size + system id from the ROM library's scan, `jeuRecherche.php` by the
  ROM's file name as a fallback, same-title only): honors 401 (closed or saturated, 10 min), 403 (developer credentials
  refused), 423 (closed, 1 h), 426 (app blocked, 1 day), 429 (thread limit: waits once, retries, then 60 s), 430 (daily quota,
  until 00:00 UTC), 431 (too many unknown ROMs, until 00:00 UTC). One request at a time with a 1.2 s pause, which is within
  every account's thread limit (`maxthreads` is read for display only). Used only for a port with a hash-confirmed ROM in the
  ROM library. System ids are from memory of ScreenScraper's list (N64 is 14): unverified until a credentialed call.

**Per-slot priority** (`resolve.rs` module docs has the table): portrait and icon from the catalog first; wide, hero and
logo from SteamGridDB, then TheGamesDB, then ScreenScraper; an image that already has the slot's shape beats one that has to
be cropped or padded whatever its source; what remains is composed by `fit` and flagged `generated` in
`.dpm-art-manifest.json`. A wide capsule with no source of its own is cropped from the hero. A logo needs real
transparency (a `wheel` with none is refused) and is never composed. ScreenScraper's `steamgrid` type is taken for the wide
capsule only when the downloaded image already has 920x430 proportions (its shape is not documented).

**Game ids.** A port's SteamGridDB id can be set per port as `sgdb_id` (and `tgdb_id`) in its `ports.local.json` entry,
number or numeric string. Otherwise the id table `cache/art/ids.json` remembers what a title search found. A search result is
used on its own only when it is the same title (case, punctuation, "the", and trailing "(...)" ignored). A look-alike
("Sonic Unleashed" for "Sonic Unleashed Recompiled") is saved as a candidate, mentioned in a note, listed under "Look-alike
matches waiting" and never used until the user picks it: the "Use this match" rows on that screen (`PendingChoice::confirm`),
or `{"folder": "<folder>", "sgdb_id": 12345}` added to the ports list of `ports.local.json`. That entry must have no name and
no source: with a name the app replaces the port's catalog entry (source and all) by the local one, without one the app skips
it and the art code reads the id from the raw file (pinned by two tests in `src/core/config.rs`). Choosing by hand also
clears the waiting row. Two same-title games that differ only in their bracket text ("DOOM (2016)", "Doom (1993)") are a tie
and are never picked for a query without brackets. A search that found
nothing usable is not repeated for 3 days.

**Cache** (`cache/art/`, in the app's cache folder, never the repo): `ids.json`, `state.json` (last problem and back-off per
source, no keys, no addresses), `images/<source>/<game id>-<slot>.img` plus a `.json` (downloaded as is, reused for 90 days,
oldest trimmed past 300 MB) and `.none` markers (a slot a source had nothing for, believed for 3 days). Source images are
never committed, bundled or released.

**Wiring.** Add to Steam (and the Game Mode queue's apply, which shares it: `screens::apply_selected_with`) places art through
`art::resolve` in `place_catalog_art`, in `Mode::CacheOnly`: one `Session` for the whole batch, no network, so it is safe on the
UI thread. It places the catalog's cached portrait and icon exactly as before, and, when a source is set up, what the screen's
"Fetch artwork for installed ports now" saved (hero, logo, wide, a native portrait or icon in place of a composed one). With no
source set up the result is byte for byte the old catalog-only art (a test compares the files). Slots holding the user's own art
are not resolved at all, so a hero the user chose also means no wide capsule is cropped from a fetched hero; our own composed
pictures are replaced when a real one is cached (`refresh_own`). A queued change that asks for no art (`none`) places none.


**Limits and not verified.** The three clients are written to the documented API shapes and tested only against saved
example answers; nothing here has run against a live key (no key was available to the builders). Not verified: ScreenScraper's
JSON field names and system ids, TheGamesDB's `API/Limit` endpoint, that `types=static` leaves no animated art, that Steam
shows the art, and how the Steam on-screen keyboard behaves with the key box in Game Mode. Animated art is off. Videos are
never fetched. Fetching uses the calling thread's time: a few requests per port (one title search, a list per picture type, a download per picture), at most about 20 with SteamGridDB alone when nothing is cached.

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

### The one-file installer

CI also builds `steamdeck-ports-mgr-installer.desktop` (`tools/build-installer.sh`) and the release
job publishes it, with its `.sha256`, next to the package, so its link is
`releases/latest/download/steamdeck-ports-mgr-installer.desktop`. It is the same pattern Decky Loader
(`decky_installer.desktop`) and EmuDeck (`EmuDeck.desktop`) use on the Deck: a desktop-entry file the
user downloads and double-clicks in Dolphin. Dolphin asks Open / Execute / Cancel (each run), and the
first time KDE also warns that it will start the program, with Continue; Continue marks the file
executable (wording unconfirmed until a Deck test). Unlike
those two, it does not download and run a script (theirs `curl` one and pipe or run it): the whole
installer is `tools/installer.sh`, embedded as comment lines between `#BEGIN_DPM_INSTALLER` and
`#END_DPM_INSTALLER`, each prefixed with `#|`. The Exec line is

    Exec=bash -c "bash <(sed -n '/^#BEGIN_DPM_INSTALLER/,/^#END_DPM_INSTALLER/s/^#|//p' -- \\"\\$1\\") \\"\\$1\\"" steamdeck-ports-mgr-installer %k

`%k` is the desktop file's own path (KIO expands it to the entry's path; it stands alone, unquoted, as
the spec requires). After the spec's string unescape and quoting rules this runs
`bash -c 'bash <(sed ... -- "$1") "$1"' steamdeck-ports-mgr-installer <file>`: the installer lines are
pulled out of the very file the user clicked and run by bash. A POSIX shell reads the same value to the
same arguments, so a launcher that hands Exec to `sh` behaves the same. Readers of the desktop entry
ignore comment lines, so the script cannot change how the launcher is read.

`Terminal=false`, with dialogs instead: `kdialog` (KDE's own; progress bar driven over D-Bus with
`dbus-send`), else `zenity` (which Decky's installer relies on, so SteamOS has it), else the same text
steps in a Konsole or xterm window, else only the log (`~/.cache/steamdeck-ports-mgr-installer.log`).
A terminal-first launcher (what Decky and EmuDeck do) shows a console full of text and closes when
the script ends; dialogs give a progress bar and a result box that stays until it is read.

What it changes: on a fresh install, the whole package moved into place in one rename (temporary
files sit in a hidden folder beside the install folder, inside `$HOME`; ones left by a killed run are
swept at the next start); on an existing install, only the four files the updater replaces (only
those are unpacked), staged as `.new`, `sync`ed, then renamed, with `.previous/` and the same
running-copy handling (asks first, then waits, then stops only a copy whose program is in that
folder). A failure or an INT/TERM/HUP during the renames puts the previous program back. The package
must hold only plain files and folders inside `steamdeck-ports-mgr/`. One installer runs at a time
(`flock` on `~/.cache/steamdeck-ports-mgr-installer.lock`; programs it starts do not inherit the
lock). Then `~/.local/share/applications/steamdeck-ports-mgr.desktop` with exactly the text
`register_desktop_entry` writes (so the app's own write at startup is a no-op), the removal of an old
upstream-named `ports_launcher.desktop` only when its Exec is this program, and on a fresh install
`~/Desktop/steamdeck-ports-mgr.desktop` (same text, executable so Plasma trusts it), unless another
launcher on the desktop already points at the program; an update only refreshes that shortcut if it
is still there. The app is started with `setsid`, so it outlives the installer. It refuses an install
folder outside `$HOME` (naming the link when `~/Applications` points elsewhere) and never uses `sudo`.

Test switches: `DPM_RELEASE_BASE` (https:// or file:// only), `DPM_INSTALL_DIR` (inside `$HOME`),
`DPM_NO_LAUNCH=1`, `DPM_UI=kdialog|zenity|text`.

Known limits: both KDE dialogs' wording, kdialog on SteamOS (assumed present with Plasma; zenity is
the fallback) and the desktop shortcut's trust are unverified until a Deck test; a launcher that does
not expand `%k` (GLib's `gio launch` up to at least 2.72) runs nothing; an `~/Applications` that is a
link to an SD card (outside `$HOME`) is refused, so such a Deck needs the manual install or the link
removed; without `flock` on the system there is no single-instance lock (SteamOS has util-linux).
