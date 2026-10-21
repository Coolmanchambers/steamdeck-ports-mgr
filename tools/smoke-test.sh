#!/usr/bin/env bash
# Startup smoke test: runs the real Linux build on a virtual display (Xvfb) with no tray host and no
# desktop session, the way a bare Game Mode launch might look, and checks it is still running after
# STARTUP_SECONDS. Saves a screenshot and the app's output. Needs xvfb and imagemagick.
#
#   bash tools/smoke-test.sh <folder with ports_launcher and 7zzs> [output folder]
set -u
BIN_DIR="$(readlink -f "${1:?folder with ports_launcher}")"
OUT="$(readlink -f "${2:-smoke-out}")"
STARTUP_SECONDS="${STARTUP_SECONDS:-15}"
DISPLAY_NUM="${DISPLAY_NUM:-99}"

mkdir -p "$OUT"
APP_DIR="$(mktemp -d)"
cp "$BIN_DIR/ports_launcher" "$BIN_DIR/7zzs" "$APP_DIR/"
for f in ports.json themes.json; do [ -f "$BIN_DIR/$f" ] && cp "$BIN_DIR/$f" "$APP_DIR/"; done

Xvfb ":$DISPLAY_NUM" -screen 0 1280x800x24 -nolisten tcp >"$OUT/xvfb.log" 2>&1 &
XVFB=$!
APP=""
cleanup() { kill $APP "$XVFB" 2>/dev/null; rm -rf "$APP_DIR"; }
trap cleanup EXIT
sleep 2

# A bare session: no D-Bus session bus (so no tray host), no Wayland, only the virtual X display.
env -i HOME="$HOME" PATH="$PATH" DISPLAY=":$DISPLAY_NUM" XDG_RUNTIME_DIR="$APP_DIR" RUST_BACKTRACE=1 \
    "$APP_DIR/ports_launcher" >"$OUT/app.log" 2>&1 &
APP=$!

sleep "$STARTUP_SECONDS"
alive=no
kill -0 "$APP" 2>/dev/null && alive=yes
DISPLAY=":$DISPLAY_NUM" import -window root "$OUT/screenshot.png" 2>/dev/null

echo "app alive after ${STARTUP_SECONDS}s: $alive"
echo "--- app output (last 20 lines) ---"; tail -20 "$OUT/app.log"
fail=0
[ "$alive" = yes ] || { echo "FAIL: the app exited during startup"; fail=1; }
if grep -q "panicked" "$OUT/app.log"; then echo "FAIL: the app panicked"; fail=1; fi
[ -f "$APP_DIR/state.json" ] && echo "state.json created" || { echo "FAIL: no state.json"; fail=1; }
[ -s "$OUT/screenshot.png" ] && echo "screenshot saved" || echo "note: no screenshot"
[ "$fail" -eq 0 ] && echo "SMOKE TEST: PASS" || echo "SMOKE TEST: FAIL"
exit "$fail"
