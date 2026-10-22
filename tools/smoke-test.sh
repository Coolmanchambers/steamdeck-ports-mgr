#!/usr/bin/env bash
# Startup smoke test: runs the packaged Linux build on a virtual display (Xvfb) with no tray host
# and no desktop session, the way a bare Game Mode launch looks, then checks that:
#   - it is still running after STARTUP_SECONDS, with no panic;
#   - it skipped the tray (no tray host here) instead of letting Slint try and fail;
#   - its window is mapped and the screenshot is not blank;
#   - Tab opens the Deck-ready list without a crash (whether it downloads is only reported), Tab returns;
#   - Settings > Artwork sources... opens and its Back row returns to Settings;
#   - Settings > Game files... opens without a crash;
#   - Close (Escape in the windowed view) quits it, since there is no tray to hide into.
# Saves the app's output and a screenshot. Needs xvfb, x11-utils, xdotool and imagemagick.
#
#   bash tools/smoke-test.sh <folder with ports_launcher and 7zzs> [output folder]
# Exit 0 = pass, 1 = the app failed a check, 2 = the test setup failed (not the app's fault).
set -u
BIN_DIR="$(readlink -f "${1:?folder with ports_launcher}")"
OUT="$(readlink -f "${2:-smoke-out}")"
STARTUP_SECONDS="${STARTUP_SECONDS:-15}"
DISPLAY_NUM="${DISPLAY_NUM:-99}"
export DISPLAY=":$DISPLAY_NUM"

setup_fail() { echo "SETUP FAILED: $1"; exit 2; }
for tool in Xvfb xdpyinfo xwininfo xdotool import identify; do
    command -v "$tool" >/dev/null || setup_fail "$tool is not installed"
done

mkdir -p "$OUT" || setup_fail "can't create $OUT"
APP_DIR="$(mktemp -d)" || setup_fail "mktemp"
cp "$BIN_DIR/ports_launcher" "$BIN_DIR/7zzs" "$APP_DIR/" || setup_fail "ports_launcher or 7zzs missing in $BIN_DIR"
for f in ports.json themes.json; do [ -f "$BIN_DIR/$f" ] && cp "$BIN_DIR/$f" "$APP_DIR/"; done

Xvfb "$DISPLAY" -screen 0 1280x800x24 -nolisten tcp >"$OUT/xvfb.log" 2>&1 &
XVFB=$!
APP=""
cleanup() { kill $APP "$XVFB" 2>/dev/null; rm -rf "$APP_DIR"; }
trap cleanup EXIT
for _ in $(seq 1 50); do xdpyinfo >/dev/null 2>&1 && break; sleep 0.2; done
kill -0 "$XVFB" 2>/dev/null && xdpyinfo >/dev/null 2>&1 || { cat "$OUT/xvfb.log"; setup_fail "Xvfb $DISPLAY did not come up"; }

# A bare session: no D-Bus session bus (so no tray host), no Wayland, no desktop name.
env -i HOME="$HOME" PATH="$PATH" DISPLAY="$DISPLAY" XDG_RUNTIME_DIR="$APP_DIR" RUST_BACKTRACE=1 \
    "$APP_DIR/ports_launcher" >"$OUT/app.log" 2>&1 &
APP=$!

sleep "$STARTUP_SECONDS"
fail=0
check() { if eval "$2"; then echo "ok:   $1"; else echo "FAIL: $1"; fail=1; fi; }

alive=no; kill -0 "$APP" 2>/dev/null && alive=yes
import -window root "$OUT/screenshot.png" 2>/dev/null
xwininfo -root -tree >"$OUT/windows.txt" 2>&1
colors=$(identify -format '%k' "$OUT/screenshot.png" 2>/dev/null || echo 0)

check "still running after ${STARTUP_SECONDS}s" '[ "$alive" = yes ]'
check "no panic" '! grep -q "panicked" "$OUT/app.log"'
check "skipped the tray (no tray host)" 'grep -q "no system tray here; closing the window quits" "$OUT/app.log"'
check "Slint was not asked for a tray" '! grep -q "Failed to create system tray icon" "$OUT/app.log"'
check "window mapped" 'grep -q "\"Ports Launcher\"" "$OUT/windows.txt"'
check "screenshot not blank ($colors colours)" '[ "${colors:-0}" -ge 32 ]'

if [ "$alive" = yes ]; then
    # No window manager here, so focus the window directly.
    win=$(xdotool search --name "^Ports Launcher$" 2>/dev/null | head -1)
    if [ -n "$win" ]; then
        xdotool windowfocus "$win" 2>/dev/null; sleep 0.5
        # Tab opens the Deck-ready list (it loads in the background), Tab again goes back.
        xdotool key Tab; sleep "${DECK_READY_SECONDS:-8}"
        import -window root "$OUT/deck-ready.png" 2>/dev/null
        dr_alive=no; kill -0 "$APP" 2>/dev/null && dr_alive=yes
        check "Deck-ready list opens and the app keeps running" '[ "$dr_alive" = yes ] && ! grep -q "panicked" "$OUT/app.log"'
        if [ -f "$APP_DIR/cache/deck-ready.json" ]; then echo "info: Deck-ready list downloaded and saved"; else echo "info: Deck-ready list not saved (no network here?)"; fi
        xdotool key Tab; sleep 1
        # Settings (gear in the title bar) > Game files... opens without a crash. Dialogs get no keyboard
        # focus without a window manager, so this clicks, relative to the main window.
        eval "$(xdotool getwindowgeometry --shell "$win" 2>/dev/null)"
        if [ -n "${X:-}" ] && [ -n "${Y:-}" ]; then
            # Settings > Artwork sources... opens and shows its rows, and its Back row returns to Settings.
            xdotool mousemove $((X + 312)) $((Y + 16)) click 1; sleep 1
            xdotool mousemove $((X + 71)) $((Y + 353)) click 1; sleep 2
            import -window root "$OUT/art-sources.png" 2>/dev/null
            art=$(xdotool search --name "^Artwork sources$" 2>/dev/null | head -1)
            art_alive=no; kill -0 "$APP" 2>/dev/null && art_alive=yes
            check "Artwork sources screen opens and the app keeps running" '[ "$art_alive" = yes ] && ! grep -q "panicked" "$OUT/app.log" && [ -n "$art" ]'
            if [ -n "$art" ]; then
                eval "$(xdotool getwindowgeometry --shell "$art")"; AX=$X; AY=$Y
                xdotool mousemove $((AX + 100)) $((AY + 63 + 42 * 9)) click 1; sleep 1.5
                check "Back leaves Artwork sources for Settings" '[ -z "$(xdotool search --name "^Artwork sources$" 2>/dev/null)" ] && [ -n "$(xdotool search --name "^Settings$" 2>/dev/null)" ]'
                eval "$(xdotool getwindowgeometry --shell "$win")"
            fi
            xdotool mousemove $((X + 312)) $((Y + 16)) click 1; sleep 1
            xdotool mousemove $((X + 71)) $((Y + 310)) click 1; sleep 2
            import -window root "$OUT/game-files.png" 2>/dev/null
            gf_alive=no; kill -0 "$APP" 2>/dev/null && gf_alive=yes
            check "Game files screen opens and the app keeps running" '[ "$gf_alive" = yes ] && ! grep -q "panicked" "$OUT/app.log" && [ -n "$(xdotool search --name "^Game files$" 2>/dev/null)" ]'
        fi
        # The steps above can leave a dialog open (Game files). With no window manager, an Escape sent
        # while it is open can land in the dialog and close only that, so close dialogs first and wait
        # for each to go, then focus the main window explicitly.
        visible_windows() { xdotool search --onlyvisible --name . 2>/dev/null; }
        other_windows() {
            local w name
            for w in $(visible_windows); do
                name=$(xdotool getwindowname "$w" 2>/dev/null)
                [ -n "$name" ] && [ "$name" != "Ports Launcher" ] && echo "$w"
            done
        }
        for _try in 1 2 3; do
            dlg=$(other_windows | tail -1)
            [ -n "$dlg" ] || break
            echo "closing dialog $dlg: $(xdotool getwindowname "$dlg" 2>/dev/null)" >>"$OUT/focus.log"
            xdotool windowfocus "$dlg" 2>/dev/null; sleep 0.3; xdotool key Escape
            for _wait in $(seq 1 15); do visible_windows | grep -qx "$dlg" || break; sleep 0.2; done
        done
        echo "windows still open besides the main one: [$(other_windows | tr '\n' ' ')]" >>"$OUT/focus.log"
        # Closing dialogs must not have ended the app, or the Close check below would pass for the wrong reason.
        check "still running after closing dialogs" 'kill -0 "$APP" 2>/dev/null'
        xdotool windowfocus "$win" 2>/dev/null; sleep 0.5
        echo "focus before the final Escape: $(xdotool getwindowfocus getwindowname 2>&1)" >>"$OUT/focus.log"
        # Escape is Close in the windowed view.
        xdotool key Escape
    fi
    for _ in $(seq 1 25); do kill -0 "$APP" 2>/dev/null || break; sleep 0.2; done
    if kill -0 "$APP" 2>/dev/null; then
        # Keep what is needed to see why (app.log and windows.txt are already in the output folder).
        xwininfo -root -tree >"$OUT/windows-at-close.txt" 2>&1
        import -window root "$OUT/close-failed.png" 2>/dev/null
        { echo "focus when Close did not quit: $(xdotool getwindowfocus getwindowname 2>&1)"; } >>"$OUT/focus.log"
        echo "--- focus.log"; cat "$OUT/focus.log"
        check "Close quits when there is no tray" 'false'
    else
        wait "$APP"; code=$?
        check "Close quits when there is no tray (exit code $code)" '[ "$code" -eq 0 ]'
    fi
    APP=""
fi

echo "--- app output (last 20 lines) ---"; tail -20 "$OUT/app.log"
[ "$fail" -eq 0 ] && echo "SMOKE TEST: PASS" || echo "SMOKE TEST: FAIL"
exit "$fail"
