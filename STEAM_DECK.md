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

## Updating

- **Right away:** Settings > **Force update**. The app closes, the updater downloads the newest
  rolling release, checks its SHA-256, swaps the program files and starts the app again.
- **Automatically:** about once a day the app checks for a newer release and the footer's
  **GitHub** button turns into **Update**. It compares dates only, so a second build on the same
  day shows up the next day; use Force update to get it sooner.
- **What changes:** only `ports_launcher`, `7zzs` and the updater itself. `Library/`, saves,
  `state.json`, `ports.local.json` and the catalogs are never touched. The previous program files
  are kept in `.previous/`; the updater writes what it did to `updater.log`.
- Builds from before the updater existed have no `ports_launcher_updater.sh`: install a fresh
  build once by hand, and Force update works from then on.

## Play from Game Mode

1. Quit the app from its tray icon (closing the window only hides it).
2. In Desktop Mode, open Steam > Games > Add a Non-Steam Game > Browse, set the file type to
   All Files, and pick `~/Applications/steamdeck-ports-mgr/ports_launcher`.
3. Optional, for a log you can read later: in the shortcut's Properties, set Launch Options to
   `%command% >/home/deck/ports_launcher.log 2>&1`
4. Switch to Game Mode and launch it from the Library. Press View to switch to the fullscreen grid.

## What to test

| Area | Pass looks like |
|---|---|
| First launch | The catalog fills; typing filters it |
| Install a port | Progress, then installed, with a Linux build under `Library/` |
| Keyboard and mouse | Arrows move, Enter installs or launches, Alt+Enter toggles fullscreen |
| Controller | D-pad moves, A activates, B closes, X opens Info, Y opens Settings |
| Game Mode | Starts from Steam, the controller works, a port launches and the app returns after |
| Force update | App closes and reopens on the newest build; `updater.log` ends with "done" |

## Known issues being tested

| Issue | What you'd see | Try this |
|---|---|---|
| Controller ignored under Wayland | Touch and mouse work, buttons don't | Launch Options `env -u WAYLAND_DISPLAY %command%` |
| No tray icon in Game Mode | The app may close at start, or vanish when you press its X | Use Steam menu > Exit Game; send the log |
| Desktop Mode copy still open | Game Mode launch returns to Steam at once | Quit from the tray in Desktop Mode first |
| Update not offered | Footer stays "GitHub" after a push the same day | Expected: the check compares dates; use Force update |

## Reporting a result

Note the step, what you expected and what happened. Include the exact text of any dialog, a
screenshot, `/home/deck/ports_launcher.log` if you set it up, the app's `state.json` and
`updater.log`, and the output of `echo $XDG_SESSION_TYPE` from Konsole.
