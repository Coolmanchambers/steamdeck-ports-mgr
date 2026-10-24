# Privacy policy

Applies to **Deck Ports Manager** (the `steamdeck-ports-mgr` build of Ports Launcher), published at
https://github.com/Coolmanchambers/steamdeck-ports-mgr. Last updated 2026-10-24.

The short version: the app runs on your own device, keeps its data on your own device, and has no
server of its own. Nothing is sent to the developer. Backing up your saves to Google Drive is optional
and off until you sign in.

## What the app reads and writes on your device

- **Its own settings and records:** the list of ports you installed, your preferences, and, if you add
  them, API keys for the game-art websites. These live in the app's own config and cache folders.
- **Your ports folder:** the ports the app installs, updates, imports, or moves for you.
- **Your save files, only to protect them:** before an update or a restore, and when you press Save
  Backup, the app copies save folders into zip files in a `Saves Backup` folder. It never deletes or
  overwrites your saves on its own.
- **Steam shortcuts and artwork:** when you ask it to add a port to Steam, it writes a shortcut entry and
  artwork files into your Steam folder, after making a backup of the shortcut file.
- **Your game files folder:** it looks inside the folder you pick to find the files a port needs. It
  does not copy, upload, or send game files anywhere.

## What the app sends over the internet

Only to the sites needed for the thing you asked for, and never any personal profile or identifier:

- **GitHub and GitLab:** to look up and download port releases, and to check for updates to this app.
  Those sites see an ordinary web request from your IP address.
- **SteamGridDB, TheGamesDB and ScreenScraper:** to fetch cover art, when you use game-art lookup. The
  request carries the game name or file hash and the key for that site.
- **Discord (off by default):** if you switch on Rich Presence, the name of the game you are playing is
  shown on your Discord profile through the Discord app running on the same device.

## Google Drive backup (optional)

If you choose **Settings > Cloud backup** and sign in, the app copies your save-backup zip files to your
own Google Drive. It does nothing with Google until you do this.

**Which Google data it touches**

- **Files the app creates itself,** in a folder called "Deck Ports Manager" in your Google Drive. The app
  asks for the `drive.file` permission, which lets it see only files and folders it created. It cannot
  read, list, change, or delete anything else in your Drive.
- **Your Google account email address,** read once at sign-in, so the app can show which account is
  connected ("Signed in as ..."). It stays on your device and is not sent anywhere else.

**Where sign-in data is kept.** The sign-in tokens are stored only on your device, in a private file
(readable only by your user) in the app's config folder. They are never sent to the developer or to any
third party, and are never written to logs or bug reports. Signing out in the app deletes that file.

**What the app does with the files.** It uploads your save backups, lists them so you can pick one to
restore, and downloads the one you pick. It does not read the contents of your saves for any other
purpose, show ads, sell or share data, or use it to train anything.

The app's use of information received from Google APIs follows the
[Google API Services User Data Policy](https://developers.google.com/terms/api-services-user-data-policy),
including the Limited Use requirements.

## What the developer receives

Nothing. There is no account, no analytics, no crash reporting, and no server run by the developer. If
you report a problem on GitHub, only what you choose to paste is shared. The files the app asks you to
attach to a bug report never include your sign-in tokens or API keys.

## Stopping and deleting

- **Stop copying:** in the app, Settings > Cloud backup > Sign out. This deletes the tokens on your
  device. Backups already in Google Drive stay there.
- **Revoke the app's access from Google:** Google Account > Security > Third-party apps & services (the
  "Third-party access" list), choose Deck Ports Manager, then Remove access.
- **Delete the cloud copies:** open Google Drive and delete the "Deck Ports Manager" folder.
- **Delete everything local:** uninstall the app and delete its config folder, cache folder, ports
  folder, and `Saves Backup` folder. Your original save files are not touched by any of this.

## Children

The app is not directed at children and collects no personal information from anyone.

## Changes and contact

If this policy changes, the new text will be published in this file with a new date; its history is in
the repository. Questions or requests: open an issue at
https://github.com/Coolmanchambers/steamdeck-ports-mgr/issues.
