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

Keep it on the internal drive or an ext4 SD card (the format SteamOS uses). It can run next to the
original Ports Launcher, or next to a second copy in another folder: each install folder has its own
single-instance lock. Starting the same folder twice hands over to the open copy instead: the second
start prints "this app is already running from this folder" and quits (bringing the open window to
the front is best effort, especially on Wayland). A copy of this app from before 2026-10-21 that is
still open from the same folder is recognized too; quit old copies from the tray before updating.

## Updating

- **Right away:** Settings > **Force update**. The app closes, the updater downloads the newest
  rolling release, checks its SHA-256, swaps the program files and starts the app again, usually
  within 5 to 15 seconds.
- **If it fails** (offline, GitHub busy, a release being replaced): nothing is changed, the app
  reopens on the version you had, and an `update-failed.txt` next to it says why. Details are in
  `updater.log`.
- **Automatically:** every 12 hours, at start or while the app stays open (in the tray). The first
  check comes 12 hours after the first launch. When a newer build exists, the footer's **GitHub**
  button turns into **Update**. If that build is already running (for example you copied the files
  over by hand), the next start clears the Update button by itself and checks again.
- **Commits, then dates:** the app compares the commit in its `BUILD_INFO` with the commit the
  rolling release was built from, so a second build on the same day is offered and the build you
  run is never offered again. Without `BUILD_INFO` (a copy you built yourself) it falls back to the
  build date: only a build from a later UTC day counts.
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
Applying, queuing, or applying the queue from this screen also ends on the main window once you close
the result message; reopen the screen to see the new state.

- **Close Steam first.** Steam rewrites its shortcut list when it exits, so the app only writes while
  Steam is closed. If Steam is running, the last row reads **Queue N changes until Steam is closed...**
  (in Game Mode: **Queue N changes for Desktop Mode...**) instead of Review (see the next section), and
  nothing is written. Steam cannot be closed in Game Mode; in Desktop Mode, Steam menu > Exit, wait a
  few seconds, then reopen the screen to write now.
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

### Queue from Game Mode, apply in Desktop Mode

In Game Mode Steam is always running, so Add to Steam never writes there (even if the Steam check
somehow finds no Steam). Instead:

1. In Game Mode, open Settings > **Add to Steam...**. The status row reads "Game Mode: Steam can't be
   closed here, so changes are queued for Desktop Mode". Pick the ports as usual and choose
   **Queue N changes for Desktop Mode...**. The confirmation says Steam can't be closed in Game Mode and
   the change will apply next time you open this app in Desktop Mode with Steam closed. Choose
   **Queue**. Nothing in Steam changes yet; the choice is saved in `steam-queue.json` next to the app
   (port, Steam account and its name, whether a same-name shortcut may be repointed, artwork).
2. In Desktop Mode, close Steam (Steam menu > Exit) and open the app. Within a couple of seconds of
   starting, or within 30 seconds if the app was already open when Steam closed (or if another message
   was showing), it asks **"N Steam changes queued from Game Mode: apply now?"**. If a port's artwork
   has not been downloaded yet, the question says so: Cancel and wait a minute to get the artwork, or
   Apply without it. **Apply** writes them the same way as the screen does (backup first, Steam
   checked again just before the write) and lists what happened. When this question pops up by itself,
   Cancel is preselected.
3. Start Steam again: the shortcuts are under Non-Steam games.

Cancel keeps them queued: apply them later from Settings > Add to Steam, or the question comes back
after Steam has run and been closed again (or at the next start). It never pops up while another dialog
is open, while a game started from the app is running, or while the window is hidden to the tray, or
minimized on X11 (on Wayland the app cannot tell it is minimized, so the question can still appear).

In Desktop Mode with Steam running, the same screen offers **Queue N changes until Steam is closed...**;
those are offered as "queued while Steam was running" once Steam is closed.

The Add to Steam screen shows the queue too: the first row applies it now (or says why it has to wait),
**View or remove queued changes...** lists each one (choose a row to remove it, or clear them all), and
ports already queued are marked `(queued)`. After a removal the list reopens with Back selected; after
clearing you are back on the Add to Steam screen. Queuing the same port again replaces its entry. When
the Deck has more than one Steam account, or the queue covers more than one, each line names the account.

When applying, each port is planned again against the shortcuts as they are then: one already in Steam
is left alone, one that is in Steam with old details is updated in place (never a second shortcut), and
one that left the catalog or was uninstalled since queuing is listed under "Dropped from the queue" and
skipped. A port that now only matches a different program's shortcut by name is skipped unless you had
switched that repoint on when queuing. If Steam starts again before the write, nothing is written and
the changes that were going to be written stay queued. If no Steam account folder is found at all (not
signed in yet, say), nothing is written and the whole queue stays. If `steam-queue.json` cannot be
parsed (damaged or from another version), the app keeps it as `steam-queue.json.corrupt-<date>-<time>`,
says so, and starts with an empty queue. If it cannot be opened at all (permissions, or a folder in its
place), it is left alone and nothing can be queued or applied until you fix or remove it.

The app's log has a line `game mode: yes` or `game mode: no` at startup with the two values it went by,
and `steam queue: offering N` whenever it asks.

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
  `ports.local.json` (the old file is backed up next to it as `ports.local.json.bak-<date>-<time>`)
  and installs it. Its button says Open folder instead of Uninstall. Game files are never included.
- The app doesn't know where an added port keeps its saves, so its updates are not backed up first.
  Copy its save folder yourself before updating it.
- "Deck build" and "Linux build" come from release file names. Only a launch on the Deck proves a
  port runs.

To undo an added port: remove its entry from `ports.local.json` (keep the file valid JSON), or copy
the `ports.local.json.bak-...` from just before the add back over it. Its files stay in `Library/`.
Before deleting that folder, look inside it for a save folder and copy that out: many ports keep their
saves there.

## Connect ports to your game files

Settings > **Game files...** is where the app finds your ROMs and connects them to the ports that need them. It only reads
your game folder: nothing in it is ever moved, renamed, changed or deleted.

1. **Game files folder** > choose your ROM folder. The list offers the usual places (SD card, `~/Emulation/roms`...); "Browse"
   walks down folder by folder and **Use this folder** saves it and scans. The scan shows its progress and has Cancel (or B).
   A big folder takes a while the first time; later scans only read new or changed files.
2. **Results by system** lists, per console, which ports have their files (Found, Several matches, Wrong version, Missing...).
   "(check this)" means a file was matched by name only. Open a port to see exactly what was found and what it wants.
3. **Connect game files...** (from the port's row under Results by system, or offered once right after it installs): pick how each file reaches the port.
   **Use original** writes nothing, **Link** puts a pointer in the port's folder (no extra space), **Copy** makes a second
   copy and asks first, showing its size. Then **Connect now**.
4. **Connected ports** shows what is connected and whether it still holds. **Forget** removes only the record.

A Link is made inside the port's folder in `Library/`, so the drive the app is installed on needs a Linux file system (ext4);
your ROM folder can be on any file system. If the app sits on an exFAT card, it says that drive does not support links and
creates nothing: open Connect again and pick Copy. To undo a Link or Copy, delete that file inside the port's folder yourself, then Forget the connection;
the app never deletes game files.

## Restore saves from a backup

The app zips a port's saves before every update, and Settings > **Backup Saves** zips every port's
saves at once. To put one of those back:

1. Quit the game first. The app refuses to restore while the port's game is running.
2. Open the port's **Info** (X) > **Restore saves...** (the button above Game folder / Save folder;
   only ports that name a save folder have it), or Settings > **Restore saves...**, which lists every
   port that has a backup.
3. The list shows the port's save folders, then its backups, newest first: the date, the kind
   (**Before update**, **Backup Saves (all ports)** or **Before restore**), the size and which save
   folders it holds. Pick one.
4. The confirm says which folders are replaced. **Restore** first zips your current saves to
   `Saves Backup/Restore Backups/<port folder>/`; if that fails, nothing is changed. Then each save
   folder is rebuilt beside the old one and swapped in, and the old one is removed only after every
   swap worked.
5. To undo a restore, pick the newest **Before restore** backup in the same list.

A save folder inside the game's own folder works on the Deck even when the catalog lists it only for
Windows. If the catalog only gives a real Windows location (a drive or `%APPDATA%`-style path) or an
Android one, the app says **no save folder known for Linux** and offers no restore rather than
guessing. A backup holding a file name that
points outside its folder (`../...` or `/...`) is listed as "cannot be used" and never unpacked. The
backups are plain zips, so you can also open them in Dolphin.

## What to test

| Area | Pass looks like |
|---|---|
| First launch | The catalog fills; typing filters it |
| Install a port | Progress, then installed, with a Linux build under `Library/` |
| Keyboard and mouse | Arrows move, Enter installs or launches, Alt+Enter toggles fullscreen |
| Controller | D-pad moves, A activates, B closes, X opens Info, Y opens Settings |
| Deck-ready list | L1/R1 switch lists; the list loads, Y filters it, A on a "Catalog" port installs it like All ports |
| Deck-ready add | A on a row with Install and no Catalog or Yours tag (Info says "In the catalog: Not yet") asks first, then installs; it appears in All ports with Open folder |
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
| Game files scan | Settings > Game files > pick the ROM folder: progress with Cancel, then results by system; a second scan is quick |
| Game files connect | Connect a port: Link adds a pointer in its folder, Copy asks first with the size; your ROM folder is unchanged |
| Update offered | After a newer build is published and 12 h past the last check (relaunch, or leave it open), the footer says Update |
| Save folder unknown | Info of an installed port whose catalog entry has only a Windows save folder says "Save folder: unknown on Linux, no backup"; Save Backup names it |
| Side by side | The original Ports Launcher and this fork, open at the same time, both stay open |
| Restore saves | Info > Restore saves... lists the backups; Restore makes a "Before restore" zip first and the saves match the backup; refused while the game runs |

## Known issues being tested

| Issue | What you'd see | Try this |
|---|---|---|
| Controller ignored | Touch and mouse work, buttons don't. Fixed for native Wayland and gamescope in current builds, not yet confirmed on a Deck | Send the log, then try Launch Options `env -u WAYLAND_DISPLAY %command%` |
| No tray in Game Mode | Expected: the app runs without a tray icon there, and its X button quits instead of hiding | If it closes at start, or X hides it and Steam keeps showing it running, send the log: it says whether a tray was expected |
| Desktop Mode copy still open | Game Mode launch of the same folder returns to Steam at once (it brought the open copy forward, which Game Mode may not show) | Quit from the tray in Desktop Mode first |
| Force update in Game Mode untested | Steam may close the updater with the app, so nothing changes and the app doesn't return | Check `updater.log`; update from Desktop Mode until this is confirmed |
| Update not offered | Footer stays "GitHub" | Under 12 h since the last check, or no `BUILD_INFO` and the newer build is from the same UTC day: use Force update. Offline, the log says `self-update check failed` |

## Reporting a result

Note the step, what you expected and what happened. Include the exact text of any dialog, a
screenshot, `/home/deck/ports_launcher.log` if you set it up, the app's `state.json`,
`updater.log`, `update-failed.txt` and `BUILD_INFO` if present, and the output of
`echo $XDG_SESSION_TYPE $XDG_CURRENT_DESKTOP` from Konsole.
