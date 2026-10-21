# Installing and testing on a Steam Deck

This fork is for testing and validation. For everyday use, install the original
[Ports Launcher](https://github.com/Nyaldee/Ports-Launcher) by Nyaldee.

Every change that passes CI on `main` is published as the
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

1. Quit the app from its tray icon (closing the window only hides it).
2. In Desktop Mode, open Steam > Games > Add a Non-Steam Game > Browse, set the file type to
   All Files, and pick `~/Applications/steamdeck-ports-mgr/ports_launcher`.
3. Optional, for a log you can read later: in the shortcut's Properties, set Launch Options to
   `%command% >/home/deck/ports_launcher.log 2>&1`. After an in-app update this log goes quiet
   until the next launch from Steam.
4. Switch to Game Mode and launch it from the Library. Press View to switch to the fullscreen grid.

## What to test

| Area | Pass looks like |
|---|---|
| First launch | The catalog fills; typing filters it |
| Install a port | Progress, then installed, with a Linux build under `Library/` |
| Keyboard and mouse | Arrows move, Enter installs or launches, Alt+Enter toggles fullscreen |
| Controller | D-pad moves, A activates, B closes, X opens Info, Y opens Settings |
| Game Mode | Starts from Steam, the controller works, a port launches and the app returns after |
| Force update | App closes and reopens on the newest build; `updater.log` ends with `=== done` |
| Force update offline | App reopens on the old build; `update-failed.txt` says why |
| Force update in Game Mode | The app comes back in Game Mode and Steam still shows it running |
| Update offered | After a later-dated build and a relaunch 24 h past the last check, the footer says Update |

## Known issues being tested

| Issue | What you'd see | Try this |
|---|---|---|
| Controller ignored | Touch and mouse work, buttons don't. Fixed for native Wayland and gamescope in current builds, not yet confirmed on a Deck | Send the log, then try Launch Options `env -u WAYLAND_DISPLAY %command%` |
| No tray in Game Mode | Expected: the app runs without a tray icon there, and its X button quits instead of hiding | If it closes at start instead, send the log |
| Desktop Mode copy still open | Game Mode launch returns to Steam at once | Quit from the tray in Desktop Mode first |
| Force update in Game Mode untested | Steam may close the updater with the app, so nothing changes and the app doesn't return | Check `updater.log`; update from Desktop Mode until this is confirmed |
| Update not offered | Footer stays "GitHub" | Same UTC day, under 24 h since the last check, or no relaunch: use Force update |

## Reporting a result

Note the step, what you expected and what happened. Include the exact text of any dialog, a
screenshot, `/home/deck/ports_launcher.log` if you set it up, the app's `state.json`,
`updater.log`, `update-failed.txt` and `BUILD_INFO` if present, and the output of
`echo $XDG_SESSION_TYPE` from Konsole.
