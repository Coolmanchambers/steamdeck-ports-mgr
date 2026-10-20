# Installing and testing on a Steam Deck

This fork is for testing and validation. For everyday use, install the original
[Ports Launcher](https://github.com/Nyaldee/Ports-Launcher) by Nyaldee.

There are no published releases yet. Builds come from this repo's GitHub Actions, and GitHub only
lets signed-in users download those, so sign in to GitHub in the Deck's browser first.

## Install (Desktop Mode)

1. **Download the build.** Open the [CI runs on `main`](https://github.com/Coolmanchambers/steamdeck-ports-mgr/actions/workflows/ci.yml?query=branch%3Amain),
   open the newest run with a green check, scroll to **Artifacts**, and click
   `steamdeck-ports-mgr-linux-x86_64`. You get a zip of about 10 MB. Artifacts expire 30 days
   after their run.
2. **Unpack it twice.** In Dolphin, right-click the zip > Extract > Extract archive here. Do the
   same with the `.tar.gz` inside. You end up with a folder `steamdeck-ports-mgr` holding
   `ports_launcher`, `7zzs`, `ports.json` and `themes.json`.
3. **Move it** to `~/Applications/steamdeck-ports-mgr` (create `Applications` if it doesn't exist).
   Use a fresh folder, not an existing Ports Launcher folder.
4. **Check it can run.** Right-click `ports_launcher` > Properties > Permissions > tick
   "Is executable". Do the same for `7zzs` if needed.
5. **Launch it.** Double-click `ports_launcher` > Execute. A dark window opens and, within a few
   seconds, lists 400+ ports.

The app keeps everything next to itself: `Library/` (installed ports), `Saves Backup/`,
`state.json` and a cache. Uninstalling the app means deleting that folder, which also deletes the
ports installed under `Library/`, so back up anything you want first.

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
| Update button | Footer says "GitHub" and opens this repo; "Force update" shows an error and changes nothing |

## Known issues being tested

| Issue | What you'd see | Try this |
|---|---|---|
| Controller ignored under Wayland | Touch and mouse work, buttons don't | Launch Options `env -u WAYLAND_DISPLAY %command%` |
| No tray icon in Game Mode | The app may close at start, or vanish when you press its X | Use Steam menu > Exit Game; send the log |
| Desktop Mode copy still open | Game Mode launch returns to Steam at once | Quit from the tray in Desktop Mode first |
| Self-update | "Force update" shows an error | Expected: this fork ships no updater yet |

## Reporting a result

Note the step, what you expected and what happened. Include the exact text of any dialog, a
screenshot, `/home/deck/ports_launcher.log` if you set it up, the app's `state.json`, and the output
of `echo $XDG_SESSION_TYPE` from Konsole.
