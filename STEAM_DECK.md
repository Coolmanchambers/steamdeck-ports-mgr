# Installing and testing on a Steam Deck

This fork is for testing and validation. For everyday use, install the original
[Ports Launcher](https://github.com/Nyaldee/Ports-Launcher) by Nyaldee.

Every change that passes the Linux CI job on `main` is published as the
[rolling release](https://github.com/Coolmanchambers/steamdeck-ports-mgr/releases/latest). No GitHub
account is needed to download it.

## Install (Desktop Mode)

1. **Download the installer:**
   [steamdeck-ports-mgr-installer.desktop](https://github.com/Coolmanchambers/steamdeck-ports-mgr/releases/latest/download/steamdeck-ports-mgr-installer.desktop)
   (one small file, about 25 KB). It lands in your Downloads folder.
2. **Open it from Dolphin's Downloads folder**, not from the browser's download list. In Dolphin the
   file shows its file name with an installer icon and a warning emblem (it is not marked executable
   yet). Double-click it. It is the same kind of launcher file that Decky Loader and EmuDeck use.
3. **Answer the two questions.** Dolphin first asks "What do you wish to do with this file?" with
   Open, Execute and Cancel: choose **Execute** (Open only shows the file as text). Leave "Do not ask
   again" unticked. Then, the first time only, KDE warns "This will start the program: Install Deck
   Ports Manager": choose **Continue**. That makes the downloaded file executable, so the warning does
   not come back; the Open/Execute question does, each time you run the installer. *Exact wording not
   yet confirmed on a Deck.*
4. **Wait for the app to open.** A progress window shows each step (download, checksum, install).
   Then Deck Ports Manager opens and a message says where it was installed. The first time, the
   window lists 400+ ports within a few seconds. Offline, it can take up to about 80 seconds before
   the installer says the download failed.

That's all: no unpacking, no moving folders, no ticking "Is executable". The installer:

- downloads the newest rolling build (about 10 MB) from this repository's release and checks its
  SHA-256; if the download or the check fails, it says so and **nothing is changed**;
- installs it in `~/Applications/steamdeck-ports-mgr` (it creates `Applications` if needed);
- adds **Deck Ports Manager** to the app menu (under Games; the app itself writes the same entry,
  `steamdeck-ports-mgr.desktop`, when it starts) and a **Deck Ports Manager** icon on the desktop.
  Until the app has started once, the menu entry may show a generic icon;
- starts the app. Closing the installer's message does not close the app.

It never asks for a password, never uses `sudo`, writes nothing outside your home folder and does
not touch Steam. Its log is `~/.cache/steamdeck-ports-mgr-installer.log`. Everything it runs is plain
bash inside the downloaded file itself (open it in a text editor to read it); it does not download
and run any other script. Only one copy runs at a time: a second one says "The installer is already
running. This second copy did nothing."

**Run the installer again to update.** On an existing install it replaces only the program files
(`ports_launcher`, `7zzs`, `ports_launcher_updater.sh`, `BUILD_INFO`), keeps the old program in
`.previous/`, and never touches `Library/`, `Saves Backup/`, `state.json`, `secrets.json`,
`fork-settings.json`, `steam-queue.json`, `ports.local.json`, the catalogs or `cache/`. If the app
is open, it says the app has to close: Continue closes it (or quit it yourself first from its tray
icon), Cancel changes nothing. Afterwards it opens the app again. This is the same swap
Settings > Force update does. If the installer is stopped in the middle of the swap, it puts the
previous program back.

**The menu entry and the original Ports Launcher.** This fork's menu entry and icon have their own
names (`steamdeck-ports-mgr.desktop`, "Deck Ports Manager"), so they no longer replace the original
Ports Launcher's "Ports Launcher" entry. Builds from before 2026-10-22 wrote a "Ports Launcher" entry;
the installer and the app remove it once, but only when it points at this fork's program (an entry
for the original app is never touched). The app's window title and tray icon still say "Ports
Launcher": that text is upstream's.

### Manual install (fallback)

If the installer does not run (for example neither question offers a way to go ahead), install by
hand:

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

Another way to run the installer by hand, from Konsole:
`bash <(sed -n '/^#BEGIN_DPM_INSTALLER/,/^#END_DPM_INSTALLER/s/^#|//p' ~/Downloads/steamdeck-ports-mgr-installer.desktop)`.

The app keeps everything next to itself: `Library/` (installed ports), `Saves Backup/`,
`state.json` and a cache. Uninstalling the app means deleting that folder, which also deletes the
ports installed under `Library/`, so back up anything you want first.

Keep it on the internal drive or an ext4 SD card (the format SteamOS uses). The installer only
installs inside your home folder: if `~/Applications` is a link to an SD card, it says so and
changes nothing; remove the link or use the manual install. It can run next to the
original Ports Launcher, or next to a second copy in another folder: each install folder has its own
single-instance lock. Starting the same folder twice hands over to the open copy instead: the second
start prints "this app is already running from this folder" and quits (bringing the open window to
the front is best effort, especially on Wayland). A copy of this app from before 2026-10-21 that is
still open from the same folder is recognized too; quit old copies from the tray before updating.

## First launch: the setup wizard

The first time the app starts (and on an install from before the wizard existed) it opens **Setup wizard**
on top of the list instead of leaving you in it. It takes a few minutes. Every page asks one question with a
few big choices: A chooses (and moves on), the D-pad moves, B goes back one page. On the first page B asks
"Skip setup?" and the cursor starts on "Keep going with setup". The header shows "Step N of 5". You can skip
anything and run setup again later.

1. **Where should ports go?** With an SD card in, it is offered first ("Recommended", a `Ports` folder on it),
   then internal storage, then "Pick a folder..." (the same folder picker as Settings > Ports folder). The place
   in use says "In use now"; choosing it keeps it. A card formatted exFAT is listed as "can't be used". A folder
   that does not exist is made only after "Make it". Ports you already installed: "Move your N ports there?"
   shows the size and the room; "Move them" copies and checks each one (a progress page with a line per port and a
   Cancel), "Leave them" uses the place without moving anything. A port whose game file is missing is asked about
   one by one (Skip this port / Move anyway). The old copies are kept; the wizard asks about them at the end.
2. **Your ports** opens Import ports straight away (see below). B on its first page comes back to step 1, Skip
   goes on.
3. **Where are your game files?** Only when an installed port names a game file it needs. Pick the folder that
   holds your ROMs and disc images (it is only read); the page then says "Found game files for 3 of 4 ports" and
   what is still missing. A file already inside the port's own folder counts as found. "Connect them now" connects the
   files found in your folder to their ports, with the same rules as Settings > Game files: used in place or linked
   where the port allows, and a copy only after "Copy game files?" showed its size (your own files are never moved,
   changed or deleted). With nothing needing a file this step is not shown.
4. **Add your ports to Steam?** "Add all N" adds every port that is not in Steam yet (Desktop Mode, Steam closed:
   written now, with the old shortcut list backed up) or queues them (Steam running, and always in Game Mode: the
   shortcut file is not touched). A Steam account is asked about only when the Deck has several. "Artwork
   sources (optional)" is the free SteamGridDB key. When Steam changes are waiting (just queued, or queued by
   moving ports into the ports folder), the page becomes **Update Steam now?** with **Close Steam and update
   shortcuts** (see "Close Steam and update shortcuts" below) and Continue (they keep waiting). In Game Mode it
   says "Switch to Desktop Mode and open Deck Ports Manager to finish updating Steam."
5. **You're set up.** Where the ports go, how many moved in, game files, Steam and old copies, then Finish.
   Before it, when at least one old copy is **ready to remove** (see "Old copies" below), one page lists the old
   copies with their status and asks "Remove the N old copies?" (Keep them is the default and B). When none is
   ready, the last page says "Old copies are kept until each port has been checked." Finish (or a confirmed skip)
   is what marks setup as done; if you quit half-way the wizard opens again next time.

**Run it again any time:** Settings (Y) > **Setup wizard...**, the first row. A rerun opens on **Your setup**:
one line per step with a mark (done, still to do, or waiting), for example "Ports folder: SD card", "Your
ports: 4 moved in", "Game files: no folder set yet", "Steam: 4 waiting for Steam to close", "Old copies: 4 kept,
0 ready". **Finish what's left** (the default) visits only the steps that still need something, in order, then
the last page, which shows every step as it stands; **Change a step...** does one step again. Steps that are done
are never redone: Import ports only offers ports that are not in the ports folder yet, the Steam step only adds
missing shortcuts and applies waiting changes, and the game files step keeps the folder that is set. B on the
first visited page goes back to Your setup; "Close the wizard" leaves without changing anything you did not
choose. What the wizard remembers between runs is `setup_progress` in `fork-settings.json`.

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
- **Or run the installer again** (Install above): the same swap, from outside the app.
- **Builds from before the updater** have no `ports_launcher_updater.sh`, so they can't update
  themselves, and their Update button only shows an error. Run the installer (it adds the updater),
  or quit from the tray and copy the files from a fresh download over the old ones (this keeps
  `Library/` and `state.json`). Force update works from then on.

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
  nothing is written. Steam cannot be closed in Game Mode; in Desktop Mode, choosing Queue goes on to
  "Update your Steam shortcuts?", which can close Steam for you and write them (see "Close Steam and update shortcuts" below).
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
   was showing), the page **"Update your Steam shortcuts?"** says "N Steam changes are waiting. Steam is
   closed, so they can be made now." If a port's artwork has not been downloaded yet, the page says so:
   Not now and wait a minute to get the artwork, or update without it. **Update Steam shortcuts now**
   writes them the same way as the screen does (backup first, Steam checked again just before the write)
   and says what happened. When this page pops up by itself, the cursor is on Not now.
3. Start Steam again: the shortcuts are under Non-Steam games.

Not now keeps them queued: apply them later from Settings > Add to Steam, or the question comes back
after Steam has run and been closed again (or at the next start). It never pops up while another dialog
is open, while a game started from the app is running, or while the window is hidden to the tray, or
minimized on X11 (on Wayland the app cannot tell it is minimized, so the question can still appear).

In Desktop Mode with Steam running, the same screen offers **Queue N changes until Steam is closed...**;
those are offered as "queued while Steam was running" once Steam is closed, or straight away through
"Close Steam and update shortcuts" below.

The Add to Steam screen shows the queue too: the first row applies it now (with Steam running in Desktop
Mode it reads "N Steam changes waiting: close Steam and update them now..."; in Game Mode it says to finish
in Desktop Mode),
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

### Close Steam and update shortcuts (Desktop Mode)

In Desktop Mode Steam normally keeps running in the tray, so waiting for it to be closed by hand could
wait forever (this is why a Deck test saw "the Steam shortcuts didn't update"). Wherever changes are
waiting, the app offers **Close Steam and update shortcuts**: the Import ports finished page (outside the
wizard), the wizard's Steam step, Settings > Add to Steam (the first row, and right after Queue), and once
per run of the app by itself at start or within 30 seconds ("Update your Steam shortcuts?", with the cursor
on Not now so a stray A changes nothing).

1. The app asks Steam to shut down the normal way (`steam -shutdown`, the same as Steam menu > Exit; the
   system's own `/usr/bin/steam` first, then `steam` on PATH, then the flatpak build through
   `flatpak run com.valvesoftware.Steam`). It never kills Steam. If a game Steam started is running (seen by
   Steam's game launcher, `reaper`), it does not ask: "Close the game first", with Try again.
2. **Closing Steam...** counts the seconds. Steam counts as closed once no Steam process has been seen for
   3 seconds. **Stop waiting** (or B) leaves without writing anything: Steam still closes on its own, and the
   app asks again once Steam is gone (the same "Update your Steam shortcuts?" page).
3. Then the queue is applied the usual way (Steam checked again just before the write, `shortcuts.vdf`
   backed up first) and **Steam shortcuts updated** says how many changes were made (and how many were
   already up to date), and that Steam starts the ports from their new place when it runs again. Details has
   the full text (paths, the backup's name).
4. **Start Steam again** starts Steam detached from the app (the page you came from then says "Steam is
   starting..."); **Leave Steam closed** (also B) does not. If Steam came back by itself before the write,
   the page offers only Try again and Not now.

If Steam is still running after 90 seconds, **Steam did not close** says nothing was changed (the queue
and `shortcuts.vdf` are exactly as they were): finish whatever Steam is showing (a question, a download),
or close Steam yourself (tray icon > Exit Steam), then Try again. If the app itself was started from Steam (a Steam shortcut, even in Desktop Mode), closing
Steam would close the app too, so it says to open the app from the desktop's app menu instead. In Game
Mode nothing is closed: the page says "Switch to Desktop Mode and open Deck Ports Manager to finish
updating Steam."

The log has `steam close:` lines: the steam command found, whether the app was started by Steam, the
shutdown request, "Steam is closed", what was applied, a timeout with what was still running.

Not yet proven on a Deck: that `steam -shutdown` from Desktop Mode closes the client cleanly within the
90 seconds, and that **Start Steam again** brings it back the way the desktop's own Steam icon does.

## Choose where ports are installed (the ports folder)

By default ports go into `Library/` next to the app (`~/Applications/steamdeck-ports-mgr/Library`).
To put them on the SD card or anywhere else: Settings > **Ports folder...**. The first rows show the
folder in use, its free space and file system. (In Desktop Mode there is also **Open the ports folder
(Desktop Mode)**.) Then:

1. **Choose another folder...** opens the folder picker: your SD card and USB drives (by label, with
   free space), Home, `~/Games` and `~/Applications` at the top; below, the folders inside the current
   one. D-pad up and down, A to open or pick, B to cancel; mouse and touch work too. **New folder
   here...** asks for a name (the on-screen keyboard: Steam + X) and creates the folder at once (it stays
   even if you cancel afterwards). For the SD card, pick the card, then New folder here... `Ports`, then
   **Use this folder**. The header says: "Ports need the internal drive or a card formatted by SteamOS.
   exFAT cards can't store Linux programs."
2. The app checks the folder. It refuses system folders, the home folder itself, the top of a drive and
   the app's own folder. A drive formatted as exFAT, FAT or NTFS is marked "can't hold ports" in the
   picker and refused with: "This drive is formatted as exFAT, which can't store Linux programs (it drops
   the 'may run' permission and links). Ports need a card formatted by SteamOS: Steam > Settings > Storage >
   Format. Formatting ERASES everything on the card, so copy your game files off it first." A card bought
   pre-formatted or formatted on a PC is usually exFAT. A folder that can't keep program permissions for
   another reason gets: "This folder can't keep program permissions, so ports could not start here. Pick a
   folder on the internal drive or on a card formatted by SteamOS." A folder typed or chosen that does not
   exist yet is created only after you confirm. Cancel goes back to the Ports folder list.
3. **Switch the ports folder?** lists the new folder and offers:
   - **Move N ports here, then switch**: each installed port is copied, checked file by file and only
     then used from the new place. The old copies are kept (see below). Progress shows on screen;
     Cancel stops within a moment (it is checked every 1 MB copied), removes the copies made so far and changes nothing. If the card
     is pulled out during the move, it stops with one message and nothing changes.
   - **Switch without moving**: the ports in the old folder stay there and show as not installed here.
     Useful when the new folder already holds your ports (another card).
   - If the new folder holds an **old copy you kept** after an earlier move (moving back), the screen says
     so ("the copy at ... is the one in use") and only offers **Replace the old copy with the current one,
     then move** or Cancel. The old copy is checked first: if a game runs from it, or it has saves newer
     than the copy in use ("Copy the save files from <old save folder> into <new save folder>, or keep the
     old copy and cancel"), it is left alone and that port is not moved. Otherwise its saves are zipped to
     `Saves Backup/Update Backups/` and it is removed. A confirm says this first: the kept copy is removed
     before the move, so if the move then fails it is already gone (the copy in use is never touched). The old copy is never used in place of the current one.
   Moving and switching wait while a game runs from a port that would move.
4. Before copying, the app checks each port's game files: the ROM links and copies made in Game files,
   and links inside the port folder that point at game files. If something is missing it lists which
   file and where it was expected, with **Fix in Game files (then start the move again)**, **Move
   anyway** or **Cancel**. For a ROM that Game files copied into the port folder only that copy counts;
   its original may be gone. A file the catalog lists under "Required files" that is neither connected nor
   in the folder does not stop the move (many are only needed once, when installing); the final message
   mentions it for information. After copying it checks again at the new place; if a game file did not
   come through intact, that port is not moved (its copy is removed, the original is untouched) and you
   choose to switch without it, fix it or cancel. Links that would break at the new place are adjusted in
   the copy (a relative link to a ROM folder gets a full path, a full path into the port folder becomes
   relative).
5. Steam shortcuts follow: each shortcut that pointed into the old folder now points at the same file in
   the new one, with its app ID kept, so artwork and play time stay. With Steam closed (Desktop Mode, Steam
   menu > Exit) this happens at once, after a backup of `shortcuts.vdf`. With Steam running (always in
   Game Mode) the change is queued and the app asks to apply it next time Steam is closed in Desktop Mode.
   Until then, starting the port from Steam still runs the OLD copy, and anything you save there stays in
   the old copy; the final message says so. Launch options are not changed.
6. **Old copies.** After the move one page lists each port's old copy with one status line (see "Old
   copies" below); right after a move they all wait, so nothing is offered yet. Kept copies are listed
   under Settings > Ports folder > **Old copies kept after moves** with their status; choosing one opens the
   same page for it.

### Old copies: validated before they can go

An old copy is offered for removal only once its port is shown to work from the new place:

- **Waiting for Steam to update**: a Steam shortcut still starts the old copy, or its change is still queued.
  Use "Close Steam and update shortcuts" first.
- **Start it once from its new place**: Steam starts the new copy, but the port has not been started since.
  Start it from Steam (the app sees Steam's "last played" time on the shortcut, after the shortcut was
  switched) or from this app (a run of at least 30 seconds counts). Or choose **It starts fine...** on the
  page: one card per port, "Yes, it starts and my saves are there" or Not yet.
- **Ready to remove**.

**Remove them** removes only the Ready ones (Keep them is the default and B), each through the same checks as
before: it waits while a game runs from it, and refuses when the old copy has newer saves than the new one
("Copy the save files from <old save folder> into <new save folder>..., or keep the old copy"). Its saves are
zipped to `Saves Backup/Update Backups/` first. A shortcut Steam has not finished setting up is named: open
Steam, quit it, then try again. A port that left the catalog keeps its old copy (its saves can't be found).
Checking a big old copy takes a moment ("Checking the old copies..."). Nothing is ever removed without "Remove
them". The one exception is the explicit "Replace the old copy with the current one, then move" choice when you move
a port back onto an old copy (Ports folder, above): that replaces a copy that may not be Ready, after its own checks
and with its saves zipped first. A port still waiting for Steam has **Close Steam and update shortcuts** on its page
(Desktop Mode). A start from Steam in Game Mode may only show once Steam has restarted; "It starts fine..." always
works. The log says `old copies:` when a run or a yes is recorded.

If the app was closed in the middle of a switch (after the new folder was saved, before Steam followed),
Settings > Ports folder shows **Finish switching <port> (Steam and game files still point at the old
copy)**: choose it to finish; the old copy can be removed after that.

If the ports folder is on the SD card and the card is out when the app starts, it says "Your ports folder
... is not available. Is the SD card in?" with **Try again (the card is in now)**, Choose another folder,
Continue without it and Quit. Nothing is created where the card would be, and no port is forgotten: put the
card back and choose Try again. Until then installs, updates, adopting and save restores wait.

## Import ports you installed elsewhere

Settings > **Import ports...** (it replaces "Adopt installed ports"), and step 2 of the setup wizard, find ports
installed outside your ports folder and move them into it. You see pictures and plain sentences, not lists, and
the whole thing works with the D-pad, A and B. Nothing to edit by hand; no file is opened.

**Where it looks.** One level deep in `~/Applications`, `~/Games` and `~/Ports`, and the top of every SD card or
USB drive that is plugged in (ports kept in folders right on the card are found), each of which can be switched off,
plus any folder you add. In Details of the first page, **Change scan folders...** shows them; **Add a folder to
scan...** opens the folder picker, and **Also look one folder deeper** looks inside folders that matched nothing
(slower, limited to 400 folders). Your choices are remembered. Hidden folders, Steam's library folders,
`lost+found`, EmuDeck's `Emulation` and the ports folder itself are never listed; a folder full of games is one
line, "looks like a collection of many games".

**The pages.**
- **Found N ports** shows the covers of what was recognised (a name tile when there is no cover) and "Check them".
- **One card per port** ("Port 2 of 5"): the cover, the name, where it came from, how it matched in plain words, its
  size and "Has a Steam shortcut" when one points into it. "Yes, this is it", "Pick another port" and "Not a port".
  A weak match (only a program inside is named like the port) loses the highlight and says "check the picture".
  "Not a port" is remembered so the folder is not offered again (the folder itself is never touched); it asks first
  on a strong match, shows "You said this is not a port" if you come back, and Details has "Ask again".
- **Which port is this?** for a folder that fits several ports, or none: the likely ports as covers (left and right,
  A chooses), "Another port..." (a box to type in, the ones named like the folder first; ports already in your ports
  folder are left out), "Use a GitHub link..." and "Not a port".
- **Move N ports into your ports folder?** with their covers, the size and the room. "Each one is copied and checked
  first. Old copies stay until you say." Details lists the steps of each port.
- **N ports moved** with their covers and one short line for anything that went wrong; the long text with paths is
  behind Details.

**A port that is not in the list: Use a GitHub link.** The page says what to look for in two lines: open the
project's GitHub page and copy its address (`github.com/owner/project`), and it needs a Releases section with a
Linux download (`.AppImage`, `.tar.gz` or `.zip`). Type or paste it into the box (Game Mode: press Steam + X for the
keyboard, or "Show keyboard"; A or Down leaves the box). "Check this link" asks GitHub (at most two questions,
nothing is downloaded) and shows "Is this the project?" with the name, description, latest version and date, and
the Linux file. "Yes, use this" adds the project to your port list just before the move (the file is backed up
first, and the entry is taken off again if the port does not move). Each problem has a plain sentence: not a link,
not GitHub, a person's page, no releases, no Linux file, GitHub unreachable, too many questions (try again in about an hour), no such project. A project already in the list counts as picking that port.

**Folders named like the download.** A port unpacked into a folder named after its download file
(`SoH-Ackbar-Delta-Linux`, `2Ship-Keiichi-Charlie-Linux`, `Severed_Chains_Steam_Deck`) is recognised; the app also
uses the download names from the Deck-ready list. A port the catalog does not have but the Deck-ready list does
(Severed Chains) is marked "new to your port list": importing adds it first. Only real programs inside a folder
count (an `.AppImage`, `.sh`, `.x86_64`), never ROMs, settings files or pictures.

**What importing does.** For each port, one at a time: the game files it needs are checked in the old folder (it
asks if one is missing or broken: Skip this port / Move anyway, or B to stop there); the folder is copied into the
ports folder (a hidden staging folder first, so a failure or Cancel leaves only the old folder); the copy is
checked, game files again; the app switches over to the new folder; a Steam shortcut that pointed into the old
folder is repointed and keeps its app ID (play time and art stay), or the change is queued for Desktop Mode when
Steam is running (always in Game Mode). **The old folder is kept.** Then the old copies page lists each one with
its status; right after the move none is ready (see "Old copies: validated before they can go"). Inside the setup
wizard that page comes once, near the end, and only when one is ready. Nothing is deleted automatically. A
progress page shows each port's state and a Cancel; a cancel stops the copy in progress and removes its partial
copy, and ports already moved stay moved.

If a port does not fit (not enough free space: its size plus 64 MB), nothing is copied for it and you are offered
to **link it where it stands** instead: `Library/<port>` becomes a link and the port gets an entry in
`ports.local.json` (backed up first). A linked port is not checked for updates and the app refuses to install or
update into it. To undo a link, in this order: remove the port's entry from `ports.local.json`, then delete the
link in `Library/` (your folder stays).

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

1. **Game files folder** > choose your ROM folder in the folder picker (the same one as Settings > Ports folder). It opens in
   your EmuDeck folder when there is one, listed first as "SD card: Emulation/roms" (marked Best pick), so **Use this folder** is
   one press; other usual places follow ("SD card: ROMs", "Home: Emulation/roms"...), and the folder you are in reads as
   "SD card > Emulation > roms" (a card is never named by its ID). **Use this folder** saves it and scans. The scan shows its
   progress and has Cancel (or B). This full scan reads every game file, since its lists show everything; a big folder takes
   a while the first time, and later scans only read new or changed files.
2. **Results by system** lists, per console, which ports have their files (Found, Several matches, Wrong version, Missing...).
   "(check this)" means a file was matched by name only. Open a port to see exactly what was found and what it wants.
3. **Connect game files...** (from the port's row under Results by system, or right after it installs, see below): pick how each file reaches the port.
   **Use original** writes nothing, **Link** puts a pointer in the port's folder (no extra space), **Copy** makes a second
   copy and asks first, showing its size. Then **Connect now**.
4. **Connected ports** shows what is connected and whether it still holds. **Forget** removes only the record.

**The search for a port's files** (after an install or an import, from the setup wizard, Info > Game files...) reads only
files that could be the ones the ports need: a file whose name matches (the full name, a disc serial, or most of the title's
words), and, for a needed file no name matches, the files of its kind (for an N64 game, any `.z64`, `.v64` or `.n64`), each
confirmed by its checksum. Everything else is only listed, so a 100 GB folder takes moments, not hours, and a file read once is
not read again. The progress says "Looking at 3 of 12 files (1.2 GB of 4.0 GB)", how many others are not needed, and about
how long is left. In the setup wizard **Skip for now** stops it and keeps the folder. A port whose file is already in its own
folder, or that already made its game archive from it (Ship of Harkinian's `oot.o2r`, 2 Ship 2 Harkinian's `mm.o2r`), counts
as having its game without any search. (Settings > Game files > Results by system does not know the archive rule: it lists
such a port by its ROM, which may say Missing.) The search's result list has **Pick another folder...** to look somewhere
else; **Find their game files** searches even ports that were searched before.

**After you install a port that needs game files**, the app looks for them in your game files folder by itself (once per
port). If the file is there, the Connect step opens (with a pick list first when several files match). If not, it says so plainly:
which file the port needs (with its CRC32 when the catalog gives one) and where it looked, for example "No matching game file
found for Ship of Harkinian. It needs: oot.z64 (CRC32 ...). Looked in: /run/media/.../Emulation/roms (12,431 game files)". Then:

- **Search again** after you copied the file into that folder (only new or changed files are read again, so it is quick). A file
  replaced by one with the same size and modified time is not read again, so rename or re-copy it if you swapped a file in
  place.
- **Connect by hand...** to pick the file anywhere yourself. A file the port does not list is named with its checksum, and
  you can still **Use it anyway**; the "connected" message then says that file was not checked against the catalog's
  checksum.
- **Later** closes it. The port's **Info** (X) > **Game files...** and Settings > Game files > **Find game files for a port...**
  run the same search whenever you like.

If no game files folder is set yet, the prompt offers **Choose ROM folder...**. After Import ports moves ports in, its card page
"N ports moved" comes first, with **Find their game files** when any of them still needs a file; the search is its own page
after it. The search also runs, once, after an update of a port
that was never searched (for example one installed before this version). If the folder suddenly looks empty (a card that is
not mounted), the app says so and keeps what it knew about the folder instead of reporting the files missing.

A Link is made inside the port's folder in `Library/`, so the drive the app is installed on needs a Linux file system (ext4);
your ROM folder can be on any file system. If the app sits on an exFAT card, it says that drive does not support links and
creates nothing: open Connect again and pick Copy. To undo a Link or Copy, delete that file inside the port's folder yourself, then Forget the connection;
the app never deletes game files.

## Restore saves from a backup

The app zips a port's saves before every update, and Settings > **Backup Saves** zips every port's
saves at once. To put one of those back:

1. Quit the game first. The app refuses to restore while the port's game is running, whether it was
   started from this app or from Steam (it names the program it found).
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
5. To undo a restore, open Info > Restore saves... again and pick the newest **Before restore** row.

Files saved after the backup was made are removed from the save folder by a restore; they stay in the
Before restore zip. While a restore runs, closing the app waits until it is done. If the app was killed
or the Deck lost power during one, the restore list starts with "A restore was interrupted: your
previous saves are in ..." naming a hidden folder beside the save folder (Ctrl+H in Dolphin shows it);
the app never deletes it.

A save folder inside the game's own folder works on the Deck even when the catalog lists it only for
Windows. If the catalog only gives a real Windows location (a drive or `%APPDATA%`-style path) or an
Android one, the app says **no save folder known for Linux** and offers no restore rather than
guessing. A backup holding a file name that
points outside its folder (`../...` or `/...`) is listed as "cannot be used" and never unpacked. The
backups are plain zips, so you can also open them in Dolphin.
## Artwork sources: fill the hero, logo and wide slots (Desktop Mode)

Steam shows five pictures per shortcut: a portrait, a wide capsule, a hero banner, a logo and an icon. The catalog gives
the portrait and most icons. Settings > **Artwork sources...** adds the rest from three free sites. Each is optional;
SteamGridDB does most of the work and is the one to set up.

**Get a SteamGridDB key (about two minutes, on the Deck or any computer):**
1. Open https://www.steamgriddb.com and sign in (it offers Steam sign-in; a free account is all you need).
2. Open your profile, Preferences, and the API tab (https://www.steamgriddb.com/profile/preferences), and press the
   button that generates a key. Copy it.
3. In the app: Settings > Artwork sources... > the **SteamGridDB** row. A text box opens. Paste the key (Ctrl+V in
   Desktop Mode; in Game Mode press Steam + X for the on-screen keyboard), move down to **Save**, press A or Enter.
4. Choose **Test the SteamGridDB key**. It makes one call and says OK, Invalid or Offline.
5. Choose **Fetch artwork for installed ports now**. It looks each installed port up (a few requests per port, one at a
   time: a title search, a list for each kind of picture and a download for each picture, up to about 20 in the worst
   case and none for pictures it already has; there is a Stop row; Stop ends it within a second or two and keeps what was fetched), saves what it finds in the
   app's `cache/` folder and tells you what it found per slot. If the Deck is offline it says so; run Fetch again when it
   is back online, nothing that was fetched is lost.
6. Use **Add to Steam** as before (or apply a queued change). It places the art Fetch saved, together with the catalog's portrait
   and icon, for the shortcuts it adds or updates; it never goes online itself, so run Fetch first. A change queued with "no
   art" places none. Pictures you chose yourself in Steam
   are never replaced; art this app composed (a crop of a wider picture) may be replaced later by a better one.
7. Restart Steam to see it (Steam reads the grid folder when the library draws).

The key is stored in `secrets.json` next to the app, shown on screen as `****` plus its last four characters, and
never sent anywhere but SteamGridDB. On the Deck's own drive the file can be read only by your account. A memory card
formatted exFAT or FAT cannot restrict files at all; if the app is on one, the screen says "This drive cannot restrict
files" and the file is open to anyone who can read the card. The key is not in `state.json`, so sending `state.json` in a
bug report does not send it. The Remove choice in the key box deletes it.

**What each source does**

| Source | Needs | Gives |
|---|---|---|
| SteamGridDB | Your free key | Portrait, wide, hero, logo, icon in Steam's own sizes (static pictures only) |
| TheGamesDB | Optional key of your own (this build has none) | Box art, banners, fan art, clear logos for ports it lists |
| ScreenScraper | A build that carries the app's credentials (this one does not); your login is optional | Box art, fan art and logos of the original console game, matched by the hash of your own game file |

The screen shows each source as set, not set, or "not configured in this build", with the last problem and any waiting
time. A port is matched to SteamGridDB by title only when the title is the same; a close match ("Sonic Unleashed" for
"Sonic Unleashed Recompiled") is listed under "Look-alike matches waiting for you" and not used. Two games with
the same title and different years ("DOOM (2016)", "Doom (1993)") are never picked for you either. Open that row: each
suggestion is a row, "Use \"Doom II\" (42) for Doom", and choosing it confirms it (then Fetch again). To choose by hand,
add this to the `ports` list in `ports.local.json`, with the port's folder (the list shows it) and the number from the
game's SteamGridDB page address, and with **no name and no source**; an entry that has either replaces the port's
whole catalog entry:

    {"folder": "shipwright", "sgdb_id": 12345}

An "Another artwork task is still running" message means a Fetch or Test is still finishing in the background; wait a
moment.

Pictures that had to be cropped or padded to fit are marked as composed in `config/grid/.dpm-art-manifest.json`. Videos
and animated pictures are not used. The images belong to their authors and the games' publishers; the app downloads them
to your device, never uploads or shares them, and the credits are in the screen's last list.

## Cloud backup to Google Drive

Settings > **Cloud backup...** copies the zips in `Saves Backup` to your own Google Drive, after you sign in with
your phone (a code and a QR code; no password is typed on the Deck). It only runs while the app is open, never
deletes anything in Drive, and the app can only see the folder it makes there. What it stores and sends is in
[PRIVACY.md](PRIVACY.md). Getting a backup back never replaces a file on the Deck; you then choose what to restore in
Restore saves.

If you remove the app's access in your Google Account (Security > Third-party access), Google also removes its access
to the folders it made earlier. After signing in again the app may not see the backups it copied before; they are
still in your Drive, and the app starts a new "Deck Ports Manager" folder. The app's folder must be on a drive that
can keep a file private (the internal drive, not an exFAT SD card), or sign-in is refused.

## What to test

| Area | Pass looks like |
|---|---|
| Installer | Download, double-click in Dolphin, Execute, Continue (first time only), progress window, the app opens; "Deck Ports Manager" menu entry and desktop icon appear |
| Installer again | Only the Open/Execute question; says "updated" (or "reinstalled"), the app reopens, ports and saves are still there, `.previous/` holds the old program |
| First launch | The setup wizard opens by itself ("Where should ports go?"); B on it asks "Skip setup?"; after finishing or skipping the catalog fills and typing filters it |
| Setup wizard again | Settings > Setup wizard... opens it at the first page; A chooses and B goes back with the controller; quitting half-way brings it back at the next start |
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
| Import ports | Settings > Import ports... shows what it found as covers, one card per port; Move copies each into the ports folder, repoints its Steam shortcut and asks once about the old copies |
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
`updater.log`, `update-failed.txt` and `BUILD_INFO` if present, the installer's log
`~/.cache/steamdeck-ports-mgr-installer.log`, and the output of
`echo $XDG_SESSION_TYPE $XDG_CURRENT_DESKTOP` from Konsole.

Never attach `cloud-tokens.json` (your Google sign-in) or `secrets.json` (artwork keys). `cloud-backup.json`
(what was copied to the cloud: file names, sizes and checksums, no account) is safe to send. If you added a
GitHub or GitLab token to `state.json` by hand, remove that line before sending the file.
