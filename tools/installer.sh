#!/bin/bash
# Deck Ports Manager installer, for Steam Deck Desktop Mode (or any Linux desktop).
#
# Not run from here. tools/build-installer.sh embeds this file, every line prefixed with "#|", between
# the #BEGIN_DPM_INSTALLER and #END_DPM_INSTALLER comment lines of steamdeck-ports-mgr-installer.desktop.
# That file's Exec line pulls these lines back out of its own file (%k) and runs them with bash, so the
# one .desktop file is everything the user downloads: no second script is fetched and run.
#
# What it does:
#   1. downloads the rolling release package and its .sha256 over HTTPS and checks the checksum;
#   2. no install yet: unpacks the whole package into ~/Applications/steamdeck-ports-mgr;
#      an install already there: replaces only what the in-app updater replaces (ports_launcher, 7zzs,
#      ports_launcher_updater.sh, BUILD_INFO), keeps the old program in .previous/, and never touches
#      Library/, Saves Backup/, state.json, secrets.json, fork-settings.json, steam-queue.json,
#      ports.local.json, the catalogs or cache/; a running copy from that folder is closed first;
#   3. writes the app-menu entry with the same bytes the app writes itself, and a desktop shortcut;
#   4. starts the app, detached, and says what happened.
# No sudo, nothing written outside $HOME, nothing of Steam's touched. On a failure before the swap
# nothing in the install folder changes; a failed swap puts the previous program back.
#
# Switches (tests use them): DPM_RELEASE_BASE (https:// or file:// only), DPM_INSTALL_DIR (absolute,
# inside $HOME), DPM_NO_LAUNCH=1, DPM_UI=kdialog|zenity|text.

set -u
umask 022

TITLE="Deck Ports Manager"
PKG="steamdeck-ports-mgr"
ASSET="$PKG-linux-x86_64.tar.gz"
BASE="https://github.com/Coolmanchambers/steamdeck-ports-mgr/releases/latest/download"
SELF_FILE="${1:-}"
EXTRACT_SED='/^#BEGIN_DPM_INSTALLER/,/^#END_DPM_INSTALLER/s/^#|//p'

if [ -z "${HOME:-}" ] || [ ! -d "$HOME" ]; then
    echo "HOME is not set to a folder; nothing was changed." >&2
    exit 1
fi
HOME_REAL="$(realpath -e -- "$HOME")" || exit 1
mkdir -p "$HOME/.cache" 2>/dev/null
LOG="$HOME/.cache/steamdeck-ports-mgr-installer.log"
: > "$LOG" 2>/dev/null || LOG=/dev/null

# ---------------------------------------------------------------- how the user sees it
have_display() { [ -n "${DISPLAY:-}${WAYLAND_DISPLAY:-}" ]; }
find_terminal() {
    for t in konsole xterm; do command -v "$t" >/dev/null 2>&1 && { echo "$t"; return 0; }; done
    return 1
}
UI="${DPM_UI:-}"
case "$UI" in kdialog | zenity | text) ;; *) UI="" ;; esac
if [ -z "$UI" ]; then
    if have_display && command -v kdialog >/dev/null 2>&1; then
        UI=kdialog
    elif have_display && command -v zenity >/dev/null 2>&1; then
        UI=zenity
    elif [ -t 1 ]; then
        UI=text
    elif have_display && [ -n "$SELF_FILE" ] && [ -z "${DPM_IN_TERMINAL:-}" ] && TERM_APP="$(find_terminal)"; then
        # No dialog program: show the same steps as text in a terminal window instead.
        DPM_UI=text DPM_IN_TERMINAL=1 "$TERM_APP" -e bash -c 'bash <(sed -n "$1" -- "$2") "$2"' \
            dpm-installer "$EXTRACT_SED" "$SELF_FILE" >/dev/null 2>&1 </dev/null
        exit $?
    else
        UI=text
    fi
fi

log() { printf '%s %s\n' "$(date '+%H:%M:%S')" "$*" >>"$LOG" 2>/dev/null; }
say() { log "$*"; [ "$UI" = text ] && printf '%s\n' "$*"; return 0; }

PROG_REF=""
ZEN_FD=""
kd() { # kd <method> [dbus-send args...] on the kdialog progress bar; cosmetic, so errors are ignored
    [ -n "$PROG_REF" ] || return 0
    local svc="${PROG_REF%% *}" path="${PROG_REF#* }" method="$1"
    shift
    dbus-send --session --print-reply=literal --dest="$svc" "$path" "$method" "$@" >/dev/null 2>&1 || true
}
progress_start() {
    case "$UI" in
        kdialog)
            PROG_REF="$(kdialog --title "$TITLE" --progressbar "Starting..." 100 2>/dev/null)" || PROG_REF=""
            PROG_REF="$(printf '%s' "$PROG_REF" | head -n 1)"
            case "$PROG_REF" in "org.kde.kdialog-"*" /"*) ;; *) PROG_REF="" ;; esac
            ;;
        zenity)
            exec {ZEN_FD}> >(zenity --progress --title="$TITLE" --text="Starting..." --percentage=0 \
                --auto-close --no-cancel --width=460 >/dev/null 2>&1)
            ;;
    esac
}
step() { # step <percent> <text>
    say "[$1%] $2"
    case "$UI" in
        kdialog)
            kd org.kde.kdialog.ProgressDialog.setLabelText "string:$2"
            kd org.freedesktop.DBus.Properties.Set string:org.kde.kdialog.ProgressDialog string:value "variant:int32:$1"
            ;;
        zenity) [ -n "$ZEN_FD" ] && printf '%s\n# %s\n' "$1" "$2" >&"$ZEN_FD" 2>/dev/null ;;
    esac
    return 0
}
progress_end() {
    case "$UI" in
        kdialog) kd org.kde.kdialog.ProgressDialog.close; PROG_REF="" ;;
        zenity)
            if [ -n "$ZEN_FD" ]; then
                printf '100\n' >&"$ZEN_FD" 2>/dev/null
                exec {ZEN_FD}>&-
                ZEN_FD=""
            fi
            ;;
    esac
    return 0
}
markup() { printf '%s' "$1" | sed 's/&/\&amp;/g; s/</\&lt;/g; s/>/\&gt;/g'; }
message() { # message info|warning|error <text>
    log "$1: $2"
    case "$UI" in
        kdialog)
            case "$1" in info) k=--msgbox ;; warning) k=--sorry ;; *) k=--error ;; esac
            kdialog --title "$TITLE" "$k" "$2" >/dev/null 2>&1
            ;;
        zenity) zenity "--$1" --title="$TITLE" --width=480 --text="$(markup "$2")" >/dev/null 2>&1 ;;
        *) printf '\n%s\n' "$2" ;;
    esac
    return 0
}
confirm() { # 0 = go ahead. Text mode has nobody to ask, so it goes ahead (as the in-app updater does).
    log "asked: $1"
    case "$UI" in
        kdialog) kdialog --title "$TITLE" --warningcontinuecancel "$1" >/dev/null 2>&1 ;;
        zenity) zenity --question --title="$TITLE" --width=480 --ok-label=Continue --cancel-label=Cancel \
            --text="$(markup "$1")" >/dev/null 2>&1 ;;
        *) say "$1"; say "(no one to ask in text mode: continuing)"; return 0 ;;
    esac
}
pretty() { case "$1" in "$HOME_REAL"/*) printf '~/%s' "${1#"$HOME_REAL"/}" ;; *) printf '%s' "$1" ;; esac; }

TMP=""
PARENT_MADE=""
cleanup() {
    progress_end
    [ -n "$TMP" ] && rm -rf -- "$TMP"
    [ -n "$PARENT_MADE" ] && rmdir -- "$PARENT_MADE" 2>/dev/null
    if [ -n "${DPM_IN_TERMINAL:-}" ]; then
        printf '\nPress Enter to close this window. '
        read -r _ || true
    fi
}
trap cleanup EXIT
trap 'exit 130' INT TERM HUP

finish() { # finish <exit code> <info|warning|error> <result word> <text>
    progress_end
    say "RESULT: $3"
    message "$2" "$4"
    exit "$1"
}
fail() { finish 1 error failed "$1

Details: $(pretty "$LOG")"; }

say "Deck Ports Manager installer, $(date '+%Y-%m-%d %H:%M:%S')"
progress_start
step 2 "Checking this system..."

for tool in curl tar sha256sum realpath mktemp sed awk cmp; do
    command -v "$tool" >/dev/null 2>&1 || fail "The program \"$tool\" is missing on this system. Nothing was changed."
done

case "${DPM_RELEASE_BASE:-}" in
    https://* | file://*) BASE="${DPM_RELEASE_BASE%/}" ;;
    "") ;;
    *) say "ignoring DPM_RELEASE_BASE (only https:// or file:// are used)" ;;
esac

DIR_IN="${DPM_INSTALL_DIR:-$HOME/Applications/$PKG}"
case "$DIR_IN" in /*) ;; *) fail "The install folder must be an absolute path ($DIR_IN). Nothing was changed." ;; esac
DIR="$(realpath -m -- "$DIR_IN")" || fail "Could not work out the install folder $DIR_IN. Nothing was changed."
case "$DIR/" in
    "$HOME_REAL"/?*/) ;;
    *) fail "The install folder must be inside your home folder ($DIR). Nothing was changed." ;;
esac
if [ -e "$DIR" ] && [ ! -d "$DIR" ]; then
    fail "$(pretty "$DIR") exists but is not a folder. Nothing was changed."
fi
APP="$DIR/ports_launcher"
PARENT="$(dirname -- "$DIR")"

# Temporary files go next to the install folder: inside $HOME, and on the same drive, so the fresh
# install is one rename at the end.
if [ ! -d "$PARENT" ]; then
    first_new="$PARENT"
    while [ ! -d "$(dirname -- "$first_new")" ]; do first_new="$(dirname -- "$first_new")"; done
    mkdir -p -- "$PARENT" || fail "Could not create $(pretty "$PARENT"). Nothing was changed."
    PARENT_MADE="$first_new"
fi
TMP="$(mktemp -d "$PARENT/.$PKG-installer.XXXXXX")" || fail "Could not create a temporary folder in $(pretty "$PARENT"). Nothing was changed."

fetch() { curl -fsSL --proto '=https,file' --proto-redir '=https' --connect-timeout 15 --max-time 300 \
    --retry 3 --retry-delay 5 -o "$2" "$1" 2>>"$LOG"; }
step 10 "Downloading the newest build (about 10 MB)..."
say "from $BASE/$ASSET"
fetch "$BASE/$ASSET" "$TMP/$ASSET" ||
    fail "The download failed (no internet, or GitHub is busy). Nothing was changed. Try again in a minute."
step 55 "Downloading its checksum..."
fetch "$BASE/$ASSET.sha256" "$TMP/$ASSET.sha256" ||
    fail "The checksum download failed. Nothing was changed. Try again in a minute."

step 62 "Checking the download..."
expected="$(awk 'NR == 1 { print tolower($1) }' "$TMP/$ASSET.sha256")"
actual="$(sha256sum "$TMP/$ASSET" | awk '{ print $1 }')"
say "sha256 expected $expected"
say "sha256 actual   $actual"
case "$expected" in
    *[!0-9a-f]* | "") fail "The checksum file is not readable. Nothing was changed. Try again in a minute." ;;
esac
[ "${#expected}" -eq 64 ] && [ "$expected" = "$actual" ] ||
    fail "The checksum does not match, so the download was not used. Nothing was changed. Try again in a minute (a new build may have been halfway through publishing)."

step 70 "Unpacking..."
listing="$(tar -tzf "$TMP/$ASSET" 2>>"$LOG")" || fail "The package could not be read. Nothing was changed."
if printf '%s\n' "$listing" | grep -v -e "^$PKG/" -e "^$PKG\$" | grep -q .; then
    fail "The package holds files outside its folder, so it was not used. Nothing was changed."
fi
mkdir -p "$TMP/x" && tar -xzf "$TMP/$ASSET" -C "$TMP/x" --no-same-owner 2>>"$LOG" ||
    fail "The package could not be unpacked (is the drive full?). Nothing was changed."
NEW="$TMP/x/$PKG"
for f in ports_launcher 7zzs; do
    [ -f "$NEW/$f" ] && [ ! -L "$NEW/$f" ] || fail "The package is missing $f. Nothing was changed."
done
commit_of() { [ -f "$1" ] && sed -n 's/^commit=//p' "$1" | head -n 1 | cut -c1-7; }
NEW_COMMIT="$(commit_of "$NEW/BUILD_INFO")"
NEW_DATE="$([ -f "$NEW/BUILD_INFO" ] && sed -n 's/^build_date=//p' "$NEW/BUILD_INFO" | head -n 1)"
say "package build: ${NEW_COMMIT:-unknown} ${NEW_DATE:-}"

# Only this folder's program counts; another Ports Launcher installed elsewhere is left alone.
our_pids() {
    local d exe
    for d in /proc/[0-9]*; do
        exe="$(readlink "$d/exe" 2>/dev/null)" || continue
        exe="${exe% (deleted)}"
        [ "$exe" = "$APP" ] && echo "${d#/proc/}"
    done
}
put() { # put <source> <name> <mode>: replace $DIR/<name> in one rename
    cp -- "$1" "$DIR/.$2.new" && chmod "$3" "$DIR/.$2.new" && mv -f -- "$DIR/.$2.new" "$DIR/$2" && return 0
    rm -f -- "$DIR/.$2.new"
    return 1
}

NOTES=""
if [ -e "$APP" ] || [ -L "$APP" ] || [ -d "$DIR" ]; then
    if [ -e "$APP" ] || [ -L "$APP" ]; then MODE=update; else MODE=into; fi
    OLD_COMMIT="$(commit_of "$DIR/BUILD_INFO")"
    say "existing folder $DIR (mode $MODE, build ${OLD_COMMIT:-unknown})"

    pids="$(our_pids)"
    if [ -n "$pids" ]; then
        step 75 "Waiting for Deck Ports Manager to close..."
        confirm "Deck Ports Manager is open. It has to close for the update.

Quit it yourself now (tray icon > Quit) and then choose Continue, or choose Continue and the installer closes it." ||
            finish 2 info cancelled "Update cancelled. Nothing was changed."
        i=0
        while [ -n "$(our_pids)" ] && [ "$i" -lt 10 ]; do sleep 1; i=$((i + 1)); done
        pids="$(our_pids)"
        if [ -n "$pids" ]; then
            say "closing the running copy ($pids)"
            kill $pids 2>/dev/null
            sleep 3
            pids="$(our_pids)"
            [ -n "$pids" ] && kill -9 $pids 2>/dev/null && sleep 1
        fi
        [ -z "$(our_pids)" ] || fail "Deck Ports Manager would not close. Nothing was changed. Quit it from its tray icon and run the installer again."
    fi

    step 80 "Installing the new program files..."
    mkdir -p -- "$DIR/.previous" || fail "Could not create $(pretty "$DIR")/.previous. Nothing was changed."
    had=""
    for f in ports_launcher 7zzs; do
        if [ -f "$DIR/$f" ]; then
            cp -p -- "$DIR/$f" "$DIR/.previous/$f" || fail "Could not back up $f. Nothing was changed."
            had="$had $f"
        fi
    done
    for f in ports_launcher 7zzs; do
        if ! put "$NEW/$f" "$f" 755; then
            for g in ports_launcher 7zzs; do
                case " $had " in
                    *" $g "*) cp -p -- "$DIR/.previous/$g" "$DIR/$g" ;;
                    *) rm -f -- "$DIR/$g" ;;
                esac
            done
            fail "Could not install $f (is the drive full?). The previous version was put back."
        fi
    done
    if [ -f "$NEW/ports_launcher_updater.sh" ]; then
        put "$NEW/ports_launcher_updater.sh" ports_launcher_updater.sh 755 ||
            NOTES="$NOTES
The in-app updater could not be replaced; run this installer again to fix that."
    fi
    if [ -f "$NEW/BUILD_INFO" ]; then
        put "$NEW/BUILD_INFO" BUILD_INFO 644 || NOTES="$NOTES
BUILD_INFO could not be replaced, so the app may offer this build as an update."
    fi
    rm -f -- "$DIR/update-failed.txt"
    if [ "$MODE" = into ]; then
        # A folder with no program in it: add the starter catalogs only where there are none.
        for src in "$NEW"/*; do
            name="$(basename -- "$src")"
            case "$name" in ports_launcher | 7zzs | ports_launcher_updater.sh | BUILD_INFO) continue ;; esac
            [ -f "$src" ] && [ ! -e "$DIR/$name" ] && cp -- "$src" "$DIR/$name"
        done
    fi
else
    MODE=fresh
    step 80 "Installing into $(pretty "$DIR")..."
    chmod 755 "$NEW" "$NEW/ports_launcher" "$NEW/7zzs" || fail "Could not prepare the files. Nothing was changed."
    [ -f "$NEW/ports_launcher_updater.sh" ] && chmod 755 "$NEW/ports_launcher_updater.sh"
    # -T: fails rather than moving into a folder that appeared in the meantime.
    mv -T -- "$NEW" "$DIR" 2>>"$LOG" || fail "Could not create $(pretty "$DIR"). Nothing was changed."
    PARENT_MADE=""
fi
say "installed: $MODE"

# ---------------------------------------------------------------- app menu and desktop shortcut
step 90 "Adding it to the app menu..."
EXE="$(realpath -e -- "$APP")" || EXE="$APP"
# Same text as the app's own register_desktop_entry (src/ui/linux_chrome.rs, desktop_exec_arg), so the
# app's write at startup changes nothing.
exe_arg="$EXE"
exe_arg="${exe_arg//\\/\\\\}"
exe_arg="${exe_arg//\"/\\\"}"
exe_arg="${exe_arg//\`/\\\`}"
exe_arg="${exe_arg//\$/\\\$}"
exe_arg="${exe_arg//%/%%}"
exe_arg="\"$exe_arg\""
entry() { # entry <Name>
    printf '[Desktop Entry]\nType=Application\nName=%s\nExec=%s\nIcon=ports_launcher\nCategories=Game;\nTerminal=false\nStartupWMClass=ports_launcher\n' "$1" "$exe_arg"
}
write_entry() { # write_entry <file> <Name> <mode>: only when different, in one rename
    local f="$1" t="$1.dpm-new"
    entry "$2" >"$t" 2>/dev/null || { rm -f -- "$t"; return 1; }
    if cmp -s -- "$t" "$f"; then rm -f -- "$t"; chmod "$3" -- "$f"; return 0; fi
    chmod "$3" -- "$t" && mv -f -- "$t" "$f" || { rm -f -- "$t"; return 1; }
}
in_home() { case "$1/" in "$HOME_REAL"/?*/) return 0 ;; esac; return 1; }

DATA_HOME="${XDG_DATA_HOME:-}"
[ -n "$DATA_HOME" ] || DATA_HOME="$HOME/.local/share"
DATA_HOME="$(realpath -m -- "$DATA_HOME")"
MENU_FILE="$DATA_HOME/applications/ports_launcher.desktop"
MENU_OK=""
if in_home "$DATA_HOME"; then
    mkdir -p -- "$DATA_HOME/applications" && write_entry "$MENU_FILE" "Ports Launcher" 644 && MENU_OK=1
fi
[ -n "$MENU_OK" ] || NOTES="$NOTES
The app-menu entry could not be written; the app adds it itself when it starts."

DESK="$(xdg-user-dir DESKTOP 2>/dev/null)"
{ [ -n "$DESK" ] && [ "$DESK" != "$HOME" ]; } || DESK="$HOME/Desktop"
DESK="$(realpath -m -- "$DESK")"
SHORTCUT="$DESK/$PKG.desktop"
DESK_OK=""
if [ -d "$DESK" ] && in_home "$DESK"; then
    # A fresh install adds the shortcut; later runs only refresh one that is still there, and never add
    # a second one when the desktop already has a launcher for this program.
    if [ "$MODE" != update ] || [ -f "$SHORTCUT" ]; then
        dup=""
        for d in "$DESK"/*.desktop; do
            [ -f "$d" ] && [ "$d" != "$SHORTCUT" ] && grep -qxF "Exec=$exe_arg" "$d" 2>/dev/null && dup="$d"
        done
        if [ -n "$dup" ]; then
            say "desktop already has a launcher for it: $dup"
        elif write_entry "$SHORTCUT" "Deck Ports Manager" 755; then
            DESK_OK=1
        fi
    fi
fi

# ---------------------------------------------------------------- start it
LAUNCHED=""
if [ "${DPM_NO_LAUNCH:-}" = 1 ]; then
    say "not starting the app (DPM_NO_LAUNCH=1)"
else
    step 96 "Starting Deck Ports Manager..."
    # Its own session, so closing this installer (or its terminal) does not take the app with it.
    if command -v setsid >/dev/null 2>&1; then
        (cd "$DIR" && setsid -f "$APP" >/dev/null 2>&1 </dev/null)
    else
        (cd "$DIR" && nohup "$APP" >/dev/null 2>&1 </dev/null &)
    fi
    i=0
    while [ -z "$(our_pids)" ] && [ "$i" -lt 10 ]; do sleep 0.5; i=$((i + 1)); done
    if [ -n "$(our_pids)" ]; then LAUNCHED=1; say "app started"; else say "app did not stay running"; fi
fi

# ---------------------------------------------------------------- tell the user
where="$(pretty "$DIR")"
build="${NEW_COMMIT:-unknown}${NEW_DATE:+, built $NEW_DATE}"
case "$MODE" in
    update)
        if [ -n "${OLD_COMMIT:-}" ] && [ "$OLD_COMMIT" = "$NEW_COMMIT" ]; then
            head_text="Deck Ports Manager was reinstalled (same build, $build)."
        else
            head_text="Deck Ports Manager was updated from build ${OLD_COMMIT:-unknown} to $build."
        fi
        head_text="$head_text
Your ports, saves and settings were not touched. The previous program is kept in $where/.previous."
        word=updated
        ;;
    *)
        head_text="Deck Ports Manager is installed in $where (build $build)."
        word=installed
        ;;
esac
if [ "${DPM_NO_LAUNCH:-}" = 1 ]; then
    open_text=""
elif [ -n "$LAUNCHED" ]; then
    open_text="It is opening now."
else
    open_text="It did not open by itself: start it from the app menu."
fi
find_text="Later, open it from the app menu (Games > Ports Launcher)"
[ -n "$DESK_OK" ] && find_text="$find_text or the \"Deck Ports Manager\" icon on the desktop"
find_text="$find_text. To update, run this installer again or use Settings > Force update in the app."
text="$head_text${open_text:+
$open_text}

$find_text$NOTES"
if [ -n "$NOTES" ] || { [ "${DPM_NO_LAUNCH:-}" != 1 ] && [ -z "$LAUNCHED" ]; }; then
    finish 0 warning "$word" "$text"
fi
finish 0 info "$word" "$text"
