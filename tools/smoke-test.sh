#!/usr/bin/env bash
# Startup smoke test: runs the packaged Linux build on a virtual display (Xvfb) with no tray host
# and no desktop session, the way a bare Game Mode launch looks, then checks that:
#   - it is still running after STARTUP_SECONDS, with no panic;
#   - it skipped the tray (no tray host here) instead of letting Slint try and fail;
#   - its window is mapped and the screenshot is not blank;
#   - Tab opens the Deck-ready list without a crash (whether it downloads is only reported), Tab returns;
#   - Settings > Artwork sources... opens and its Back row returns to Settings;
#   - Settings > Import ports... opens and closes without a crash;
#   - Settings > Game files... opens without a crash;
#   - Settings > Ports folder... opens, its "Choose another folder..." opens the folder picker, and
#     closing the picker returns to the Ports folder list; then, end to end, it moves the two seeded
#     ports into ~/Games/Ports (New folder here, Use this folder, Move, Keep for now twice) and checks the
#     setting, the copy (program bits), the kept old copies and their records, and a relaunch;
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
# A home of its own (the end-to-end move creates ~/Games/Ports) and two installed catalog ports.
# Outside the app folder (a ports folder inside it is refused) and outside /tmp (refused as a system folder).
HOME_DIR="$(mktemp -d -p "${SMOKE_HOME_BASE:-$HOME}" dpm-smoke-home.XXXXXX)" || setup_fail "mktemp home"
mkdir -p "$HOME_DIR/Games" "$APP_DIR/Library/SpaceCadetPinball" "$APP_DIR/Library/Abuse_2025" || setup_fail "mkdir $HOME_DIR"
printf '#!/bin/sh\necho pinball\n' > "$APP_DIR/Library/SpaceCadetPinball/SpaceCadetPinball" && chmod 755 "$APP_DIR/Library/SpaceCadetPinball/SpaceCadetPinball"
echo data > "$APP_DIR/Library/Abuse_2025/x"
E2E=0

Xvfb "$DISPLAY" -screen 0 1280x800x24 -nolisten tcp >"$OUT/xvfb.log" 2>&1 &
XVFB=$!
APP=""
cleanup() { kill $APP "$XVFB" 2>/dev/null; rm -rf "$APP_DIR" "$HOME_DIR"; }
trap cleanup EXIT
for _ in $(seq 1 50); do xdpyinfo >/dev/null 2>&1 && break; sleep 0.2; done
kill -0 "$XVFB" 2>/dev/null && xdpyinfo >/dev/null 2>&1 || { cat "$OUT/xvfb.log"; setup_fail "Xvfb $DISPLAY did not come up"; }

# A bare session: no D-Bus session bus (so no tray host), no Wayland, no desktop name.
env -i HOME="$HOME_DIR" PATH="$PATH" DISPLAY="$DISPLAY" XDG_RUNTIME_DIR="$APP_DIR" RUST_BACKTRACE=1 \
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
            # Settings > Import ports... opens (its scan runs on a worker thread), shows the screen and closes again.
            xdotool mousemove $((X + 312)) $((Y + 16)) click 1; sleep 1
            xdotool mousemove $((X + 71)) $((Y + 311)) click 1; sleep 4
            import -window root "$OUT/import-ports.png" 2>/dev/null
            imp=$(xdotool search --name "^Import ports$" 2>/dev/null | head -1)
            imp_alive=no; kill -0 "$APP" 2>/dev/null && imp_alive=yes
            check "Import ports screen opens and the app keeps running" '[ "$imp_alive" = yes ] && ! grep -q "panicked" "$OUT/app.log" && [ -n "$imp" ]'
            if [ -n "$imp" ]; then
                xdotool windowfocus "$imp" 2>/dev/null; sleep 0.3; xdotool key Escape
                for _wait in $(seq 1 15); do [ -z "$(xdotool search --name "^Import ports$" 2>/dev/null)" ] && break; sleep 0.2; done
                xdotool windowfocus "$win" 2>/dev/null; sleep 0.5
                eval "$(xdotool getwindowgeometry --shell "$win" 2>/dev/null)"
                # The gear only reacts to a click once the pointer has been seen over it (no window manager here).
                xdotool mousemove $((X + 250)) $((Y + 150)); sleep 0.3; xdotool mousemove $((X + 312)) $((Y + 16)); sleep 0.5
            fi
            # Settings > Artwork sources... opens and shows its rows, and its Back row returns to Settings.
            xdotool mousemove $((X + 312)) $((Y + 16)) click 1; sleep 1
            xdotool mousemove $((X + 71)) $((Y + 395)) click 1; sleep 2
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
            xdotool mousemove $((X + 71)) $((Y + 353)) click 1; sleep 2
            import -window root "$OUT/game-files.png" 2>/dev/null
            gf_alive=no; kill -0 "$APP" 2>/dev/null && gf_alive=yes
            check "Game files screen opens and the app keeps running" '[ "$gf_alive" = yes ] && ! grep -q "panicked" "$OUT/app.log" && [ -n "$(xdotool search --name "^Game files$" 2>/dev/null)" ]'
            # Settings > Ports folder... (row 5), then its "Choose another folder..." row (third) opens the picker;
            # the picker's close button cancels back to the Ports folder list.
            # A dialog's own close button (top right), since keys need focus and there is no window manager.
            close_named() {
                local w; w=$(xdotool search --name "$1" 2>/dev/null | head -1); [ -n "$w" ] || return 0
                eval "$(xdotool getwindowgeometry --shell "$w")"; xdotool mousemove $((X + WIDTH - 16)) $((Y + 16)) click 1; sleep 1
            }
            close_named "^Game files$"
            eval "$(xdotool getwindowgeometry --shell "$win")"
            xdotool mousemove $((X + 312)) $((Y + 16)) click 1; sleep 1
            xdotool mousemove $((X + 71)) $((Y + 227)) click 1; sleep 2
            pf=$(xdotool search --name "^Ports folder$" 2>/dev/null | head -1)
            check "Ports folder screen opens and the app keeps running" 'kill -0 "$APP" 2>/dev/null && ! grep -q "panicked" "$OUT/app.log" && [ -n "$pf" ]'
            if [ -n "$pf" ]; then
                eval "$(xdotool getwindowgeometry --shell "$pf")"; PX=$X; PY=$Y
                xdotool mousemove $((PX + 100)) $((PY + 63 + 42 * 2)) click 1; sleep 2
                import -window root "$OUT/folder-picker.png" 2>/dev/null
                picker=$(xdotool search --name "^Choose the ports folder$" 2>/dev/null | head -1)
                check "Choose another folder... opens the folder picker" '[ -n "$picker" ] && kill -0 "$APP" 2>/dev/null'
                if [ -n "$picker" ]; then
                    eval "$(xdotool getwindowgeometry --shell "$picker")"
                    xdotool mousemove $((X + WIDTH - 16)) $((Y + 16)) click 1; sleep 1.5
                    check "closing the picker returns to the Ports folder list" '[ -z "$(xdotool search --name "^Choose the ports folder$" 2>/dev/null)" ] && [ -n "$(xdotool search --name "^Ports folder$" 2>/dev/null)" ]'
                    # End to end: Choose another folder > Games > New folder here "Ports" > Use this folder >
                    # Move 2 ports here > Keep for now twice. Keys go to the focused dialog (no window manager).
                    wait_window() {
                        local w
                        for _ in $(seq 1 "${2:-50}"); do
                            w=$(xdotool search --onlyvisible --name "$1" 2>/dev/null | head -1)
                            if [ -n "$w" ]; then echo "$w"; return 0; fi
                            sleep 0.2
                        done
                        return 1
                    }
                    key_to() {
                        local name="$1" w; shift
                        w=$(wait_window "$name" 150) || { echo "e2e: no window $name" >>"$OUT/e2e.log"; return 1; }
                        xdotool windowfocus "$w" 2>/dev/null; sleep 0.4; xdotool key --delay 200 "$@"; sleep 1.5
                        STEP=$((${STEP:-0} + 1)); import -window root "$OUT/e2e-$STEP.png" 2>/dev/null; echo "e2e: step $STEP keys $* to $name ($w), focus now $(xdotool getwindowfocus getwindowname 2>&1)" >>"$OUT/e2e.log"
                    }
                    pf=$(wait_window "^Ports folder$")
                    if [ -n "$pf" ]; then
                        eval "$(xdotool getwindowgeometry --shell "$pf")"
                        xdotool mousemove $((X + 100)) $((Y + 63 + 42 * 2)) click 1; sleep 2
                        E2E=1
                        # From "Use this folder": Up skips the heading to the Games place (Places: Home, Games).
                        key_to "^Choose the ports folder$" Up Return
                        key_to "^Choose the ports folder$" Down Return
                        nf=$(wait_window "^New folder in")
                        if [ -n "$nf" ]; then xdotool windowfocus "$nf" 2>/dev/null; sleep 0.4; xdotool type --delay 80 "Ports"; sleep 0.3; xdotool key Return; sleep 1.5; fi
                        key_to "^Choose the ports folder$" Return
                        key_to "^Switch the ports folder" Return
                        key_to "^Remove the old copy now" Return
                        key_to "^Remove the old copy now" Return
                        import -window root "$OUT/move-done.png" 2>/dev/null
                        cp "$APP_DIR/fork-settings.json" "$APP_DIR/port-moves.json" "$OUT/" 2>/dev/null
                        check "end to end: the new ports folder is saved" 'grep -q "\"ports_folder\": \"$HOME_DIR/Games/Ports\"" "$APP_DIR/fork-settings.json"'
                        check "end to end: program bits kept in the copy (755)" '[ "$(stat -c %a "$HOME_DIR/Games/Ports/SpaceCadetPinball/SpaceCadetPinball" 2>/dev/null)" = 755 ]'
                        check "end to end: the old copy is kept" '[ -f "$APP_DIR/Library/SpaceCadetPinball/SpaceCadetPinball" ]'
                        check "end to end: both old copies recorded" '[ "$(grep -c "\"old_path\"" "$APP_DIR/port-moves.json" 2>/dev/null)" = 2 ]'
                        check "end to end: no staging folder left" '[ -d "$HOME_DIR/Games/Ports" ] && [ -z "$(find "$HOME_DIR/Games/Ports" -maxdepth 1 -name ".*.dpm-moving-*" 2>/dev/null)" ]'
                    fi
                fi
            fi
            eval "$(xdotool getwindowgeometry --shell "$win")"
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
    if [ "$E2E" = 1 ]; then
        # Started again, the app finds the new ports folder.
        env -i HOME="$HOME_DIR" PATH="$PATH" DISPLAY="$DISPLAY" XDG_RUNTIME_DIR="$APP_DIR" RUST_BACKTRACE=1 \
            "$APP_DIR/ports_launcher" >"$OUT/app-relaunch.log" 2>&1 &
        APP=$!
        sleep 6
        check "relaunch: the moved ports folder is available" 'grep -q "ports folder: $HOME_DIR/Games/Ports (available)" "$OUT/app-relaunch.log"'
        kill "$APP" 2>/dev/null; wait "$APP" 2>/dev/null; APP=""
    fi
fi

echo "--- app output (last 20 lines) ---"; tail -20 "$OUT/app.log"
[ "$fail" -eq 0 ] && echo "SMOKE TEST: PASS" || echo "SMOKE TEST: FAIL"
exit "$fail"
