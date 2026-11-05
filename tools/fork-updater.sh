#!/bin/sh
# steamdeck-ports-mgr updater. CI ships this file as `ports_launcher_updater.sh` next to the program;
# the app runs it from Settings > Force update or the footer's Update button, then quits.
#
# It replaces only `ports_launcher`, `7zzs` and itself, after checking the download's SHA-256.
# It never touches Library/, Saves Backup/, state.json, ports.json, themes.json, ports.local.json
# or cache/. The previous program files are kept in .previous/ for a manual rollback.
# Whatever happens, it tries to leave the program running, because the app has already quit.
#
# DPM_RELEASE_BASE (https:// or file:// only) overrides where the package comes from; tests use it.

REPO="Coolmanchambers/steamdeck-ports-mgr"
PKG="steamdeck-ports-mgr"
ASSET="$PKG-linux-x86_64.tar.gz"
BASE="https://github.com/$REPO/releases/latest/download"
case "${DPM_RELEASE_BASE:-}" in
    https://* | file://*) BASE="$DPM_RELEASE_BASE" ;;
esac

APP_DIR="$(dirname "$(readlink -f "$0")")"
cd "$APP_DIR" || exit 1
SELF="$(basename "$0")"
APP="$APP_DIR/ports_launcher"
if touch "$APP_DIR/updater.log" 2>/dev/null; then exec >>"$APP_DIR/updater.log" 2>&1; else exec >/dev/null 2>&1; fi
echo "=== $(date '+%Y-%m-%d %H:%M:%S') update started in $APP_DIR"

# Only this folder's program counts: another Ports Launcher install elsewhere is left alone.
our_pids() {
    for p in $(pgrep -x ports_launcher 2>/dev/null); do
        exe="$(readlink -f "/proc/$p/exe" 2>/dev/null)"
        exe="${exe% (deleted)}"
        [ "$exe" = "$APP" ] && echo "$p"
    done
}
restart_app() {
    # A fast failure can land while the app is still closing; let it finish before deciding.
    i=0
    while [ -n "$(our_pids)" ] && [ "$i" -lt 10 ]; do sleep 1; i=$((i + 1)); done
    if [ -x "$APP" ] && [ -z "$(our_pids)" ]; then
        nohup "$APP" >/dev/null 2>&1 &
        echo "started the program"
    fi
}
fail() {
    echo "FAILED: $1"
    echo "$(date '+%Y-%m-%d %H:%M:%S') update failed: $1 (details in updater.log)" > "$APP_DIR/update-failed.txt"
    restart_app
    exit 1
}

for tool in curl tar sha256sum pgrep; do
    command -v "$tool" >/dev/null 2>&1 || fail "$tool is not installed"
done

TMP_DIR="$(mktemp -d)" || fail "could not create a temporary folder"
# The swap (data safety review D11, the pattern of tools/installer.sh): both new files are staged as .<name>.new and
# synced first; a failure or an INT/TERM/HUP during the renames puts the previous program back from .previous/
# (written beside the file, then renamed over it), and a file that was not there before is removed again.
HAD=""
SWAPPING=""
rollback_program() {
    for g in ports_launcher 7zzs; do
        rm -f "./.$g.new"
        case " $HAD " in
            *" $g "*) cp -p ".previous/$g" "./.$g.restore" && mv -f "./.$g.restore" "./$g" ;;
            *) rm -f "./$g" ;;
        esac
    done
}
cleanup() {
    rm -f "./.ports_launcher.new" "./.7zzs.new" "./.ports_launcher.restore" "./.7zzs.restore"
    rm -rf "$TMP_DIR"
}
on_signal() {
    if [ -n "$SWAPPING" ]; then
        rollback_program
        SWAPPING=""
        fail "the update was stopped part way, restored the previous version"
    fi
    exit 130
}
trap cleanup EXIT
trap on_signal INT TERM HUP

fetch() { curl -fsSL --connect-timeout 15 --max-time 300 --retry 3 --retry-delay 5 -o "$2" "$1"; }
echo "downloading $BASE/$ASSET"
fetch "$BASE/$ASSET" "$TMP_DIR/$ASSET" || fail "download failed, nothing was changed"
fetch "$BASE/$ASSET.sha256" "$TMP_DIR/$ASSET.sha256" || fail "checksum download failed, nothing was changed"
(cd "$TMP_DIR" && sha256sum -c "$ASSET.sha256") || fail "checksum does not match, nothing was changed"

mkdir -p "$TMP_DIR/x"
tar -xzf "$TMP_DIR/$ASSET" -C "$TMP_DIR/x" "$PKG/ports_launcher" "$PKG/7zzs" || fail "package is missing program files, nothing was changed"
tar -xzf "$TMP_DIR/$ASSET" -C "$TMP_DIR/x" "$PKG/ports_launcher_updater.sh" "$PKG/BUILD_INFO" 2>/dev/null
NEW="$TMP_DIR/x/$PKG"
[ -f "$NEW/BUILD_INFO" ] && sed 's/^/  /' "$NEW/BUILD_INFO"

# The app quits itself after starting this script. Wait for it, then ask, then insist.
i=0
while [ -n "$(our_pids)" ] && [ "$i" -lt 10 ]; do sleep 1; i=$((i + 1)); done
pids="$(our_pids)"
if [ -n "$pids" ]; then
    kill $pids 2>/dev/null
    sleep 3
    pids="$(our_pids)"
    [ -n "$pids" ] && kill -9 $pids 2>/dev/null && sleep 1
fi
[ -z "$(our_pids)" ] || fail "the program would not stop, nothing was changed"

mkdir -p .previous || fail "could not create .previous, nothing was changed"
for f in ports_launcher 7zzs; do
    if [ -f "$f" ]; then
        cp -p "$f" ".previous/$f" || fail "could not back up $f, nothing was changed"
        HAD="$HAD $f"
    fi
done

for f in ports_launcher 7zzs; do
    if ! { cp "$NEW/$f" "./.$f.new" && chmod 755 "./.$f.new"; }; then
        rm -f ./.ports_launcher.new ./.7zzs.new
        fail "could not stage the new $f (drive full?), nothing was changed"
    fi
done
sync 2>/dev/null
SWAPPING=1
for f in ports_launcher 7zzs; do
    if ! mv -f "./.$f.new" "./$f"; then
        rollback_program
        SWAPPING=""
        fail "could not install $f, restored the previous version"
    fi
done
SWAPPING=""
if [ -f "$NEW/BUILD_INFO" ]; then
    { cp "$NEW/BUILD_INFO" ./.BUILD_INFO.new && mv -f ./.BUILD_INFO.new ./BUILD_INFO; } || { rm -f ./.BUILD_INFO.new; echo "warning: BUILD_INFO could not be updated (the program is updated)"; }
fi
rm -f "$APP_DIR/update-failed.txt"
echo "installed new ports_launcher and 7zzs"

restart_app

# Replace this script last: the shell keeps reading the already-open old file until it exits.
if [ -s "$NEW/ports_launcher_updater.sh" ]; then
    cp "$NEW/ports_launcher_updater.sh" "./.$SELF.new" && chmod 755 "./.$SELF.new" && mv -f "./.$SELF.new" "./$SELF"
    echo "updated $SELF"
fi
echo "=== done"
