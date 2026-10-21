# Installing and testing on a Steam Deck

This fork is for testing and validation. For everyday use, install the original
[Ports Launcher](https://github.com/Nyaldee/Ports-Launcher) by Nyaldee.

Every change that passes the Linux CI job on `main` is published as the
[rolling release](https://github.com/Coolmanchambers/steamdeck-ports-mgr/releases/latest). No GitHub
account is needed to download it.

## Install (Desktop Mode)

1. **Download the build:**
   [steamdeck-ports-mgr-linux-x86_64.tar.gz](https://github.com/Coolmanchambers/steamdeck-ports-mgr/releases/latest/download/steamdeck-ports-mgr-linux-x86_64.tar.gz)
   (about 10 MB).
2. **Unpack it.** In Dolphin, right-click the file > Extract > Extract archive here. You get a
   folder `steamdeck-ports-mgr` holding `ports_launcher`, `7zzs`, `ports_launcher_updater.sh`,
   `ports.json` and `themes.json`.
3. **Move it** to `~/Applications/steamdeck-ports-mgr` (create `Applications` if it doesn't exist).
   Use a fresh folder, not an existing Ports Launcher folder.
4. **Check it can run.** Right-click `ports_launcher` > Properties > Permissions > tick
   "Is executable". Do the same for `7zzs` if needed.
5. **Launch it.** Double-click `ports_launcher` > Execute. A dark window opens and, within a few
   seconds, lists 400+ ports.

The app keeps everything next to itself: `Library/` (installed ports), `Saves Backup/`,
`state.json` and a cache. Uninstalling the app means deleting that folder, which also deletes the
ports installed under `Library/`, so back up anything you want first.

Keep it on the internal drive or an ext4 SD card (the format SteamOS uses). Don't run the original
Ports Launcher at the same time: both use the same single-instance lock, so the second one to start
closes silently.

## Updating

- **Right away:** Settings > **Force update**. The app closes, the updater downloads the newest
  rolling release, checks its SHA-256, swaps the program files and starts the app again, usually
  within 5 to 15 seconds.
- **If it fails** (offline, GitHub busy, a release being replaced): nothing is changed, the app
  reopens on the version you had, and an `update-failed.txt` next to it says why. Details are in
  `updater.log`.
- **Automatically:** only when the app starts, and only if its last check was more than 24 hours
  ago. The first check is therefore a day after first launch, and an app left running in the tray
  never checks: quit from the tray and relaunch. When a newer build exists, the footer's **GitHub**
  button turns into **Update**.
- **Dates, not versions:** a build counts as newer only if it is from a later day, in UTC. In
  Arizona that day starts at 5 pm. A second build on the same UTC day is never offered
  automatically; use Force update.
- **What changes:** only `ports_launcher`, `7zzs`, the updater itself and `BUILD_INFO` (the exact
  commit and build date). `Library/`, saves, `state.json`, `ports.local.json` and the catalogs are
  never touched. The previous program files are kept in `.previous/`.
- **Builds from before the updater** have no `ports_launcher_updater.sh`, so they can't update
  themselves, and their Update button only shows an error. Quit from the tray, then copy the files
  from a fresh download over the old ones (this keeps `Library/` and `state.json`). Force update
  works from then on.

## Play from Game Mode

1. Quit the app from its tray icon (in Desktop Mode, closing the window only hides it; in Game Mode
   there is no tray and the X button quits).
2. In Desktop Mode, open Steam > Games > Add a Non-Steam Game > Browse, set the file type to
   All Files, and pick `~/Applications/steamdeck-ports-mgr/ports_launcher`.
3. Optional, for a log you can read later: in the shortcut's Properties, set Launch Options to
   `%command% >/home/deck/ports_launcher.log 2>&1`. After an in-app update this log goes quiet
   until the next launch from Steam.
4. Switch to Game Mode and launch it from the Library. Press View to switch to the fullscreen grid.

## Add ports to Steam, one per port (Desktop Mode)

Settings > **Add to Steam...** lists every installed port with what would happen to it:
`[x] Dusklight: will add`, `[x] Ship of Harkinian: will update (program)` (a shortcut you made by
hand, matched by program or name, keeps its play time, art and controller layout), `already in Steam`,
or `skipped, <reason>` (no single program found, program not marked executable, and so on).
A shortcut that only shares the port's name but points at another program shows as
`same name as a shortcut for <program>, will repoint it` and starts switched off.
Choosing a row switches it on or off; **Review N changes...** shows the list and asks before writing.
(Backing out of the review or the account list with B returns to the main window; reopen the screen.)

- **Close Steam first.** Steam rewrites its shortcut list when it exits, so the app only writes while
  Steam is closed. If Steam is running, **Review** shows a message (naming the Steam processes still
  found) instead of the Apply button, and nothing changes. Steam cannot be closed in Game Mode: switch
  to Desktop Mode, Steam menu > Exit, wait a few seconds, then try again.
- **Backup:** every write first copies `shortcuts.vdf` to `shortcuts.vdf.bak-<date>-<time>` in the
  same folder (`userdata/<id>/config/`). The newest 10 are kept (set `steam_backups_keep` in
  `fork-settings.json`); anything from the last hour is never removed.
- **Account:** the Steam account that signed in last is used (the first row names it and lets you
  choose another). If Steam does not say which signed in last, the first row asks; the choice is
  remembered in `fork-settings.json`.
- **Artwork:** the catalog portrait and icon are placed in `userdata/<id>/config/grid/` for each
  shortcut written, under Steam's file names, and the icon is set when the shortcut has none. Art you chose
  yourself is never replaced. The images are fetched when the screen opens, so give a freshly opened
  screen a minute before applying. If the result says "no cached artwork yet" for a port, its
  shortcut was still written without art, and a later run does not add art to a shortcut that is
  already in Steam yet.
- After it says "Added ...", start Steam again. The shortcuts are under Non-Steam games.

To undo, close Steam and copy the backup from before that write (the one named for the time of the
write, listed in the result) over `shortcuts.vdf`. Each write makes its own backup, so the newest
backup is the state after the latest write, not before it.

## Adopt ports you installed elsewhere (Desktop Mode)

Settings > **Adopt installed ports...** looks one level deep in `~/Applications`, `~/Games` and
`~/Ports`, plus any folders you list under `adopt_folders` in `fork-settings.json` (the last row
creates that file and opens it). A folder is offered when its name, or a program inside it, matches a
catalog port. Adopting does not move, copy or change your folder: it adds a link `Library/<port>`
pointing at it and an entry in `ports.local.json` (backed up first). The port then shows as
installed, Play and Info work, and Uninstall is replaced by Open folder. Adopted ports are not
checked for updates, and the app refuses to install or update into a linked folder.

To undo, in this order: remove the port's entry from `ports.local.json` (and keep the file valid
JSON), then delete the link in `Library/` (your original folder stays). Deleting only the link leaves
the entry, and the port then shows an Install button that makes an empty folder.

## Deck-ready list

Press **L1** or **R1** (or Tab, or the **Deck-ready** button in the footer) to switch from All ports to
this fork's list of ports whose latest release has a Steam Deck or Linux build. It loads in the
background the first time (a few seconds) and is kept for offline use.

- **A** installs or plays, **X** opens Info, **Y** opens Filters (builds, system, community finds),
  **B** goes back to All ports. The search box works here too.
- A port the catalog already has, or one you already added, shows "Catalog" or "Yours" and behaves
  exactly as in All ports: no second copy.
- A port that is not in the catalog asks first ("Add ... ?"), then adds it as your own entry in
  `ports.local.json` (the old file is backed up next to it) and installs it. Its button says Open
  folder instead of Uninstall. Game files are never included.
- "Deck build" and "Linux build" come from release file names. Only a launch on the Deck proves a
  port runs.

To undo an added port: remove its entry from `ports.local.json` (keep the file valid JSON), then
delete its folder under `Library/` yourself if you want the files gone.

## What to test

| Area | Pass looks like |
|---|---|
| First launch | The catalog fills; typing filters it |
| Install a port | Progress, then installed, with a Linux build under `Library/` |
| Keyboard and mouse | Arrows move, Enter installs or launches, Alt+Enter toggles fullscreen |
| Controller | D-pad moves, A activates, B closes, X opens Info, Y opens Settings |
| Deck-ready list | L1/R1 switch lists; the list loads, Y filters it, A on a "Catalog" port installs it like All ports |
| Deck-ready add | A on a port without a tag asks first, then installs; it appears in All ports with Open folder |
| Deck-ready offline | With Wi-Fi off, the list still opens from the saved copy and says GitHub could not be reached |
| Game Mode | Starts from Steam, the controller works, a port launches and the app returns after |
| Controller in dialogs | D-pad and A/B work inside Info, Settings and confirm dialogs, in both modes |
| Close in Game Mode | The X button quits and Steam returns to the Library with no "running" badge |
| Close in Desktop Mode | The X button hides the window; the tray icon brings it back |
| Force update | App closes and reopens on the newest build; `updater.log` ends with `=== done` |
| Force update offline | App reopens on the old build; `update-failed.txt` says why |
| Force update in Game Mode | The app comes back in Game Mode and Steam still shows it running |
| Add to Steam, Steam open | Settings > Add to Steam > Review shows the close-Steam message (no Apply button) and changes nothing |
| Add to Steam, Steam closed | "Added N", a `shortcuts.vdf.bak-...` next to `shortcuts.vdf`, shortcuts appear after Steam restarts |
| Adopt | An outside install shows as installed with Play and Open folder; your folder is untouched |
| Update offered | After a later-dated build and a relaunch 24 h past the last check, the footer says Update |

## Known issues being tested

| Issue | What you'd see | Try this |
|---|---|---|
| Controller ignored | Touch and mouse work, buttons don't. Fixed for native Wayland and gamescope in current builds, not yet confirmed on a Deck | Send the log, then try Launch Options `env -u WAYLAND_DISPLAY %command%` |
| No tray in Game Mode | Expected: the app runs without a tray icon there, and its X button quits instead of hiding | If it closes at start, or X hides it and Steam keeps showing it running, send the log: it says whether a tray was expected |
| Desktop Mode copy still open | Game Mode launch returns to Steam at once | Quit from the tray in Desktop Mode first |
| Force update in Game Mode untested | Steam may close the updater with the app, so nothing changes and the app doesn't return | Check `updater.log`; update from Desktop Mode until this is confirmed |
| Update not offered | Footer stays "GitHub" | Same UTC day, under 24 h since the last check, or no relaunch: use Force update |

## Reporting a result

Note the step, what you expected and what happened. Include the exact text of any dialog, a
screenshot, `/home/deck/ports_launcher.log` if you set it up, the app's `state.json`,
`updater.log`, `update-failed.txt` and `BUILD_INFO` if present, and the output of
`echo $XDG_SESSION_TYPE $XDG_CURRENT_DESKTOP` from Konsole.
