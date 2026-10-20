#!/bin/sh
# steamdeck-ports-mgr updater. CI ships this file as `ports_launcher_updater.sh` next to the program;
# the app runs it from Settings > Force update or the footer's Update button, then quits.
#
# It replaces only `ports_launcher`, `7zzs` and itself, after checking the download's SHA-256.
# It never touches Library/, Saves Backup/, state.json, ports.json, themes.json, ports.local.json
# or cache/. The previous program files are kept in .previous/ for a manual rollback.
#
# DPM_RELEASE_BASE overrides where the package is downloaded from (used by tests).

REPO="Coolmanchambers/steamdeck-ports-mgr"
PKG="steamdeck-ports-mgr"
ASSET="$PKG-linux-x86_64.tar.gz"
BASE="${DPM_RELEASE_BASE:-https://github.com/$REPO/releases/latest/download}"

APP_DIR="$(dirname "$(readlink -f "$0")")"
cd "$APP_DIR" || exit 1
SELF="$(basename "$0")"
exec >>"$APP_DIR/updater.log" 2>&1
echo "=== $(date '+%Y-%m-%d %H:%M:%S') update started in $APP_DIR"

STOPPED=""
restart_app() {
    if [ -n "$STOPPED" ] && [ -x ./ports_launcher ]; then
        nohup ./ports_launcher >/dev/null 2>&1 &
        echo "restarted the program"
    fi
}
fail() {
    echo "FAILED: $1"
    restart_app
    exit 1
}

for tool in curl tar sha256sum; do
    command -v "$tool" >/dev/null 2>&1 || fail "$tool is not installed"
done

TMP_DIR="$(mktemp -d)" || fail "could not create a temporary folder"
trap 'rm -rf "$TMP_DIR"' EXIT

echo "downloading $BASE/$ASSET"
curl -fsSL -o "$TMP_DIR/$ASSET" "$BASE/$ASSET" || fail "download failed"
curl -fsSL -o "$TMP_DIR/$ASSET.sha256" "$BASE/$ASSET.sha256" || fail "checksum download failed"
(cd "$TMP_DIR" && sha256sum -c "$ASSET.sha256") || fail "checksum does not match, nothing was changed"

mkdir -p "$TMP_DIR/x"
tar -xzf "$TMP_DIR/$ASSET" -C "$TMP_DIR/x" "$PKG/ports_launcher" "$PKG/7zzs" || fail "package is missing program files, nothing was changed"
tar -xzf "$TMP_DIR/$ASSET" -C "$TMP_DIR/x" "$PKG/ports_launcher_updater.sh" 2>/dev/null
NEW="$TMP_DIR/x/$PKG"

# The app quits itself after starting this script; give it a moment, then make sure.
i=0
while pgrep -x ports_launcher >/dev/null 2>&1 && [ "$i" -lt 10 ]; do sleep 1; i=$((i + 1)); done
if pgrep -x ports_launcher >/dev/null 2>&1; then
    pkill -x ports_launcher
    sleep 2
fi
STOPPED=1

mkdir -p .previous
for f in ports_launcher 7zzs; do
    [ -f "$f" ] && cp -p "$f" ".previous/$f"
done

for f in ports_launcher 7zzs; do
    if ! { cp "$NEW/$f" "./.$f.new" && chmod 755 "./.$f.new" && mv -f "./.$f.new" "./$f"; }; then
        rm -f "./.$f.new"
        for g in ports_launcher 7zzs; do
            [ -f ".previous/$g" ] && cp -p ".previous/$g" "./$g"
        done
        fail "could not install $f, restored the previous version"
    fi
done
echo "installed new ports_launcher and 7zzs"

nohup ./ports_launcher >/dev/null 2>&1 &
echo "started the new program"

# Replace this script last: the shell keeps reading the already-open old file until it exits.
if [ -s "$NEW/ports_launcher_updater.sh" ]; then
    cp "$NEW/ports_launcher_updater.sh" "./.$SELF.new" && chmod 755 "./.$SELF.new" && mv -f "./.$SELF.new" "./$SELF"
    echo "updated $SELF"
fi
echo "=== done"
