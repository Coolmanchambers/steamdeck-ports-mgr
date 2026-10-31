#!/usr/bin/env bash
# Startup smoke test: runs the packaged Linux build on a virtual display (Xvfb) with no tray host
# and no desktop session, the way a bare Game Mode launch looks, then checks that:
#   - it is still running after STARTUP_SECONDS, with no panic;
#   - it skipped the tray (no tray host here) instead of letting Slint try and fail;
#   - its window is mapped and the screenshot is not blank;
#   - it opens on Home; Tab goes to All ports, then the Deck-ready list without a crash (whether it downloads is only
#     reported), then Home again;
#   - Settings is a page of groups (Settings, Settings: Ports ...): Steam > Artwork sources opens and its Back row returns to
#     the Steam page, B goes up to Settings, Saves > Cloud backup opens its first page and B returns to the Saves page;
#   - the background checkup finds the seeded setup's problems and Home shows its "Fix N problems" tile; A on the tile and
#     Settings > Fix problems open the checkup, B leaves it;
#   - while Settings is open the main window shows its dim layer; Return, a click and Escape on it ask for the Settings window
#     by its id; closing Settings clears it (page and dialog windows are titled "Deck Ports Manager: <page>");
#   - Ports > Import ports opens and closes without a crash;
#   - the Port page: a right-click on a Home tile opens it, a card page opens and B returns, B goes Home; the lists' Home
#     button goes Home from Deck-ready opened via Add a port; B twice from a search in All ports clears it, then goes Home;
#   - Ports > Add a port opens the page and B returns to the Ports page; Updates > Check for Updates flips in place and keeps
#     the cursor on it;
#   - Ports > Game files opens without a crash; Ports > Kept files opens its page and B returns to the Ports page;
#   - Ports > Ports folder opens, its "Choose another folder..." opens the folder picker, and
#     closing the picker returns to the Ports folder list; then, end to end, it moves the two seeded
#     ports into ~/Games/Ports (New folder here, Use this folder, Move, Keep for now twice) and checks the
#     setting, the copy (program bits), the kept old copies and their records, and a relaunch;
#   - a fresh start (no fork-settings.json) opens the setup wizard by itself, Skip setup leaves the
#     main window, and quitting the wizard half-way leaves setup unfinished; every other run seeds
#     setup_done so it goes straight to the main window; the wizard's pages (Import ports included) stay in
#     one window;
#   - Close (Escape in the windowed view) quits it, since there is no tray to hide into.
#   - a copy started in big mode: Y on Settings shows the app (the layer), Return brings Settings back with its cursor;
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
# Setup is marked finished here, so this run goes straight to the main window. The fresh-start check
# further down (its own folder, no fork-settings.json) is the one that sees the wizard.
echo '{"setup_done": true}' > "$APP_DIR/fork-settings.json"

Xvfb "$DISPLAY" -screen 0 1280x800x24 -nolisten tcp >"$OUT/xvfb.log" 2>&1 &
XVFB=$!
APP=""
cleanup() { kill $APP ${FRESH:-} ${BIG:-} "$XVFB" 2>/dev/null; rm -rf "$APP_DIR" "$HOME_DIR" "${FRESH_DIR:-}" "${BIG_DIR:-}"; }
trap cleanup EXIT
for _ in $(seq 1 50); do xdpyinfo >/dev/null 2>&1 && break; sleep 0.2; done
kill -0 "$XVFB" 2>/dev/null && xdpyinfo >/dev/null 2>&1 || { cat "$OUT/xvfb.log"; setup_fail "Xvfb $DISPLAY did not come up"; }

fail=0
check() { if eval "$2"; then echo "ok:   $1"; else echo "FAIL: $1"; fail=1; fi; }

# ---- First start: the setup wizard ----
# A folder with no fork-settings.json is a first start (and so is an old install that never ran the
# wizard). The wizard must open by itself; quitting it half-way must leave setup unfinished so it comes
# back; Skip setup (confirmed) must end it and leave the main window. The pages are driven by keyboard:
# each page asks one question and the cursor starts on its recommended (or kept) choice, Return chooses,
# Down moves to the next choice, and B (Escape) goes back one page (on the first page it asks to skip).
# The runs use an empty home folder, so Import ports finds nothing there.
FRESH_DIR="$(mktemp -d)" || setup_fail "mktemp"
mkdir -p "$FRESH_DIR/home"
cp "$APP_DIR/ports_launcher" "$APP_DIR/7zzs" "$FRESH_DIR/"
for f in ports.json themes.json; do [ -f "$APP_DIR/$f" ] && cp "$APP_DIR/$f" "$FRESH_DIR/"; done
FRESH=""
start_fresh() {
    env -i HOME="$FRESH_DIR/home" PATH="$PATH" DISPLAY="$DISPLAY" XDG_RUNTIME_DIR="$FRESH_DIR" RUST_BACKTRACE=1 \
        "$FRESH_DIR/ports_launcher" >>"$OUT/fresh-app.log" 2>&1 &
    FRESH=$!
}
find_window() {  # $1 = name regex; waits up to 20 s
    local w
    for _ in $(seq 1 100); do
        w=$(xdotool search --onlyvisible --name "$1" 2>/dev/null | tail -1)
        [ -n "$w" ] && { echo "$w"; return 0; }
        sleep 0.2
    done
    return 1
}
press_keys() {  # $1 = window id, then the keys. Without a window manager a dialog gets keys only after an explicit focus.
    local win="$1"; shift
    xdotool windowfocus "$win" 2>/dev/null; sleep 0.5
    xdotool key --delay 150 "$@"
}
# Import ports opens by itself on the second step; its first page is "found" or "nothing found".
# Skip is the second choice of the first page; the nothing-found page has Next highlighted.
skip_import() {
    local w; w=$(find_window '^Deck Ports Manager: Import ports: (nothing found|found)$') || return 1
    case "$(xdotool getwindowname "$w")" in
        *"nothing found") press_keys "$w" Return ;;                  # Next is highlighted on a first run
        *) press_keys "$w" Down Return ;;
    esac
}

start_fresh
wiz=$(find_window '^Deck Ports Manager: Setup wizard') || wiz=""
import -window root "$OUT/fresh-wizard.png" 2>/dev/null
check "a fresh start opens the setup wizard by itself" '[ -n "$wiz" ] && kill -0 "$FRESH" 2>/dev/null && ! grep -q "panicked" "$OUT/fresh-app.log"'
check "the wizard opens on the ports folder question (no welcome page)" '[ "$(xdotool getwindowname "$wiz" 2>/dev/null)" = "Deck Ports Manager: Setup wizard: Ports folder" ]'
if [ -n "$wiz" ]; then
    # Park the pointer over the second choice. A pointer that merely rests there must not take the highlight
    # from the keyboard, so Return on the first choice (the folder in use, kept) must go on to Import ports.
    eval "$(xdotool getwindowgeometry --shell "$wiz")"
    xdotool mousemove $((X + WIDTH / 2)) $((Y + 315))
    press_keys "$wiz" Return
    imp=$(find_window '^Deck Ports Manager: Import ports: ') || imp=""
    import -window root "$OUT/fresh-import-screen.png" 2>/dev/null
    check "keeping the folder goes on to Import ports (a resting pointer does not move the highlight)" '[ -n "$imp" ] && [ -z "$(xdotool search --onlyvisible --name "^Deck Ports Manager: Choose the ports folder" 2>/dev/null)" ]'
    if [ -n "$imp" ]; then
        imp=$(find_window '^Deck Ports Manager: Import ports: (nothing found|found)$') || imp=""   # the scan gives way to its first page
        check "Import ports pages are drawn in the wizard's own window (one window, no new window per page)" '[ -n "$imp" ] && [ "$imp" = "$wiz" ]'
        press_keys "$imp" Escape
        next=$(find_window '^Deck Ports Manager: Setup wizard: Ports folder') || next=""
        check "B on the first Import ports page goes back to the ports folder question" '[ -n "$next" ]'
        check "B comes back in that same window" '[ -n "$next" ] && [ "$next" = "$wiz" ]'
        if [ -n "$next" ]; then
            # Positive control: the pointer still rests over the second choice. A few small real moves must now
            # take the highlight, so Return opens the folder picker. If the layout ever shifted so the pointer was
            # nowhere near a choice, the check above would pass for the wrong reason and this one fails.
            for _ in 1 2 3; do xdotool mousemove_relative -- 6 0; sleep 0.3; done
            press_keys "$next" Return
            picker=$(find_window '^Deck Ports Manager: Choose the ports folder') || picker=""
            import -window root "$OUT/fresh-ports-picker.png" 2>/dev/null
            check "moving the pointer over a choice does take the highlight (positive control: Pick a folder opens the picker)" '[ -n "$picker" ]'
            if [ -n "$picker" ]; then
                press_keys "$picker" Escape
                next=$(find_window '^Deck Ports Manager: Setup wizard: Ports folder') || next=""
                check "cancelling the picker returns to the same question (not a Settings menu)" '[ -n "$next" ] && [ -z "$(xdotool search --onlyvisible --name "^Deck Ports Manager: Ports folder$" 2>/dev/null)" ]'
            fi
        fi
        wiz="$next"
    fi
fi
if [ -n "$wiz" ]; then
    press_keys "$wiz" Escape   # B on the first page
    ask=$(find_window '^Deck Ports Manager: Setup wizard: skip setup\?') || ask=""
    import -window root "$OUT/fresh-skip-question.png" 2>/dev/null
    check "B on the first page asks before skipping setup" '[ -n "$ask" ]'
fi
kill "$FRESH" 2>/dev/null; wait "$FRESH" 2>/dev/null; FRESH=""
check "quitting half-way leaves setup unfinished" '! grep -qs "setup_done" "$FRESH_DIR/fork-settings.json"'

start_fresh
wiz=$(find_window '^Deck Ports Manager: Setup wizard') || wiz=""
check "the wizard comes back at the next start" '[ -n "$wiz" ]'
if [ -n "$wiz" ]; then
    press_keys "$wiz" Escape
    ask=$(find_window '^Deck Ports Manager: Setup wizard: skip setup\?') || ask=""
    if [ -n "$ask" ]; then
        press_keys "$ask" Down Return   # Skip setup (the second choice; the cursor starts on Keep going)
        for _ in $(seq 1 50); do grep -qs '"setup_done": true' "$FRESH_DIR/fork-settings.json" && break; sleep 0.2; done
    fi
fi
sleep 1
import -window root "$OUT/fresh-after-skip.png" 2>/dev/null
check "confirmed Skip setup is remembered" 'grep -qs "\"setup_done\": true" "$FRESH_DIR/fork-settings.json"'
check "no wizard window is left after Skip setup" '[ -z "$(xdotool search --onlyvisible --name "^Deck Ports Manager: Setup wizard" 2>/dev/null)" ]'
check "the main window is still there after Skip setup" 'kill -0 "$FRESH" 2>/dev/null && xdotool search --onlyvisible --name "^Ports Launcher$" >/dev/null 2>&1'
kill "$FRESH" 2>/dev/null; wait "$FRESH" 2>/dev/null; FRESH=""

# Walk the whole wizard to Finish: keep the folder, skip Import ports, go on past Steam (the game files page is passed
# over: no installed port needs a file; the empty home has no Steam folder, so its page offers Try again, then Continue), Finish.
# setup_done is written, the wizard closes and the main window stays. B on the last page goes back one page.
rm -f "$FRESH_DIR/fork-settings.json"
start_fresh
walked=yes
w=$(find_window '^Deck Ports Manager: Setup wizard: Ports folder$') || walked="no (never saw the ports folder question)"
first_page="$w"
[ "$walked" = yes ] && press_keys "$w" Return                      # keep the folder in use
[ "$walked" = yes ] && { skip_import || walked="no (never saw Import ports)"; }
if [ "$walked" = yes ]; then
    w=$(find_window '^Deck Ports Manager: Setup wizard: Steam$') || walked="no (never saw the Steam page)"
    # Next on "nothing found" closes Import ports first, then the wizard draws its next page: the closed window is
    # taken back (one window across a close-first hand-off), not a new one.
    [ "$walked" = yes ] && check "after Import ports closes and hands back, the wizard's next page is the same window" '[ -n "$first_page" ] && [ "$w" = "$first_page" ]'
    [ "$walked" = yes ] && press_keys "$w" Down Return             # Continue (Try again is first)
fi
[ "$walked" = yes ] && { w=$(find_window '^Deck Ports Manager: Setup wizard: all done$') || walked="no (never saw the last page)"; }
if [ "$walked" = yes ]; then
    import -window root "$OUT/fresh-last-page.png" 2>/dev/null
    press_keys "$w" Escape
    back=$(find_window '^Deck Ports Manager: Setup wizard: Steam$') || back=""
    check "B on the last page goes back one page" '[ -n "$back" ]'
    # B returns the cursor to where it was on that page (Continue, chosen on the way forward), so Return alone goes on.
    [ -n "$back" ] && { press_keys "$back" Return; w=$(find_window '^Deck Ports Manager: Setup wizard: all done$') || walked="no (never came back to the last page: B did not return the cursor to Continue)"; }
fi
check "walking every page reaches the last page ($walked)" '[ "$walked" = yes ]'
[ "$walked" = yes ] && press_keys "$w" Return                      # Finish
for _ in $(seq 1 50); do grep -qs '"setup_done": true' "$FRESH_DIR/fork-settings.json" && break; sleep 0.2; done
sleep 1
check "Finish writes setup_done" 'grep -qs "\"setup_done\": true" "$FRESH_DIR/fork-settings.json"'
check "the wizard is gone and the main window is there after Finish" '[ -z "$(xdotool search --onlyvisible --name "^Deck Ports Manager: Setup wizard" 2>/dev/null)" ] && kill -0 "$FRESH" 2>/dev/null && xdotool search --onlyvisible --name "^Ports Launcher$" >/dev/null 2>&1'
kill "$FRESH" 2>/dev/null; wait "$FRESH" 2>/dev/null; FRESH=""
rm -rf "$FRESH_DIR"

# ---- The main run: setup already finished, so it goes straight to the main window ----
# A bare session: no D-Bus session bus (so no tray host), no Wayland, no desktop name.
env -i HOME="$HOME_DIR" PATH="$PATH" DISPLAY="$DISPLAY" XDG_RUNTIME_DIR="$APP_DIR" RUST_BACKTRACE=1 \
    "$APP_DIR/ports_launcher" >"$OUT/app.log" 2>&1 &
APP=$!

sleep "$STARTUP_SECONDS"

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
        # The app opens on Home; Tab goes to All ports (the catalog, one switch away), then the Deck-ready list (it loads
        # in the background), then Home again.
        import -window root "$OUT/home.png" 2>/dev/null
        check "the app opens on Home" 'grep -q "^list: Home$" "$OUT/app.log"'
        xdotool key Tab; sleep 1
        import -window root "$OUT/all-ports.png" 2>/dev/null
        check "the catalog (All ports) is one switch away" '[ "$(grep "^list: " "$OUT/app.log" | sed -n 2p)" = "list: All ports" ]'
        xdotool key Tab; sleep "${DECK_READY_SECONDS:-8}"
        import -window root "$OUT/deck-ready.png" 2>/dev/null
        dr_alive=no; kill -0 "$APP" 2>/dev/null && dr_alive=yes
        check "Deck-ready list opens and the app keeps running" '[ "$dr_alive" = yes ] && ! grep -q "panicked" "$OUT/app.log" && grep -q "^list: Deck-ready$" "$OUT/app.log"'
        if [ -f "$APP_DIR/cache/deck-ready.json" ]; then echo "info: Deck-ready list downloaded and saved"; else echo "info: Deck-ready list not saved (no network here?)"; fi
        xdotool key Tab; sleep 1
        check "the lists go round back to Home" '[ "$(grep "^list: " "$OUT/app.log" | tail -1)" = "list: Home" ]'
        # The plus tile ("Add a port") is the last tile on Home: A opens the Add a port page, B returns to Home.
        xdotool windowfocus "$win" 2>/dev/null; sleep 0.4; xdotool key --delay 250 Right Right Right Right Return; sleep 1.5
        ap=$(xdotool search --name "^Deck Ports Manager: Add a port$" 2>/dev/null | head -1)
        import -window root "$OUT/add-a-port.png" 2>/dev/null
        check "A on the Add a port tile opens the Add a port page" '[ -n "$ap" ] && kill -0 "$APP" 2>/dev/null && ! grep -q "panicked" "$OUT/app.log"'
        if [ -n "$ap" ]; then
            press_keys "$ap" Escape; sleep 1.5
            check "B returns from Add a port to Home" '[ -z "$(xdotool search --name "^Deck Ports Manager: Add a port$" 2>/dev/null)" ] && kill -0 "$APP" 2>/dev/null && [ -n "$(xdotool search --onlyvisible --name "^Ports Launcher$" 2>/dev/null)" ]'
        fi
        # Home's "Fix N problems" tile: the seeded setup has problems (no Steam here, a game file missing), so the background
        # look put the tile on Home. With nothing played it is the first tile; A opens the checkup, B leaves it.
        for _ in $(seq 1 20); do grep -q '^checkup: background look' "$OUT/app.log" && break; sleep 0.5; done
        check "the background checkup found something to fix" 'grep -qE "^checkup: background look: .*(thing|to do now|reminder)" "$OUT/app.log"'
        xdotool windowfocus "$win" 2>/dev/null; sleep 0.4; xdotool key --delay 250 Left Left Left Left Left; sleep 0.8
        import -window root "$OUT/home-fix-tile.png" 2>/dev/null
        xdotool key --delay 250 Return
        fp=$(find_window '^Deck Ports Manager: Fix problems$') || fp=""
        sleep 2
        import -window root "$OUT/fix-problems.png" 2>/dev/null
        check "A on Home's Fix problems tile opens the checkup and it looks over the setup" '[ -n "$fp" ] && kill -0 "$APP" 2>/dev/null && ! grep -q "panicked" "$OUT/app.log" && grep -q "^checkup: looked" "$OUT/app.log"'
        if [ -n "$fp" ]; then
            press_keys "$fp" Escape; sleep 1.5
            check "B leaves the checkup for Home" '[ -z "$(xdotool search --name "^Deck Ports Manager: Fix problems$" 2>/dev/null)" ] && kill -0 "$APP" 2>/dev/null && [ -n "$(xdotool search --onlyvisible --name "^Ports Launcher$" 2>/dev/null)" ]'
        fi
        # The Port page: a right-click on a port's tile (the second tile is a port, with or without the Fix tile first) opens
        # it; a card's page opens and B comes back; B on the Port page returns to Home.
        eval "$(xdotool getwindowgeometry --shell "$win" 2>/dev/null)"
        cols=$(( WIDTH / 160 )); [ "$cols" -lt 2 ] && cols=2; [ "$cols" -gt 8 ] && cols=8
        tx=$(( X + 12 + (WIDTH - 24) * 3 / (2 * cols) )); ty=$(( Y + 200 ))
        xdotool windowfocus "$win" 2>/dev/null; sleep 0.3
        xdotool mousemove $((X + 40)) $((Y + 600)); sleep 0.3; xdotool mousemove "$tx" "$ty"; sleep 0.3; xdotool mousemove $((tx + 5)) $((ty + 5)); sleep 0.3; xdotool click 3
        pp=$(find_window '^Deck Ports Manager: Port: ') || pp=""
        sleep 2
        import -window root "$OUT/port-page.png" 2>/dev/null
        check "a right-click on a Home tile opens its Port page" '[ -n "$pp" ] && kill -0 "$APP" 2>/dev/null && grep -q "^port page: opened for" "$OUT/app.log"'
        if [ -n "$pp" ]; then
            press_keys "$pp" Down Down Return; sleep 1.5
            pc=$(find_window ': Program to start$') || pc=""
            check "a card of the Port page opens its own page (Program to start)" '[ -n "$pc" ]'
            press_keys "$(find_window '^Deck Ports Manager: Port: ')" Escape; sleep 1.2
            check "B on a card page returns to the Port page" '[ -z "$(xdotool search --onlyvisible --name ": Program to start$" 2>/dev/null)" ] && [ -n "$(xdotool search --onlyvisible --name "^Deck Ports Manager: Port: " 2>/dev/null)" ]'
            press_keys "$(find_window '^Deck Ports Manager: Port: ')" Escape; sleep 1.5
            check "B on the Port page returns to Home" '[ -z "$(xdotool search --onlyvisible --name "^Deck Ports Manager: Port: " 2>/dev/null)" ] && [ "$(grep "^list: " "$OUT/app.log" | tail -1)" = "list: Home" ] && [ -n "$(xdotool search --onlyvisible --name "^Ports Launcher$" 2>/dev/null)" ]'
        fi
        # The lists' Home button: Add a port > Deck-ready ports, type a search, click "< Home" (top left) -> Home.
        xdotool windowfocus "$win" 2>/dev/null; sleep 0.4; xdotool key --delay 250 Right Right Right Right Right Right Right Return; sleep 1.5
        ap3=$(find_window '^Deck Ports Manager: Add a port$') || ap3=""
        if [ -n "$ap3" ]; then
            press_keys "$ap3" Return; sleep 2
            xdotool windowfocus "$win" 2>/dev/null; sleep 0.4; xdotool type --delay 120 "zel"; sleep 1
            eval "$(xdotool getwindowgeometry --shell "$win" 2>/dev/null)"
            xdotool mousemove $((X + 40)) $((Y + 300)); sleep 0.3; xdotool mousemove $((X + 45)) $((Y + 72)); sleep 0.3; xdotool mousemove $((X + 47)) $((Y + 74)); sleep 0.3; xdotool click 1; sleep 1.5
            import -window root "$OUT/home-button.png" 2>/dev/null
            check "Deck-ready from Add a port, a search, then the Home button goes Home" '[ "$(grep "^list: " "$OUT/app.log" | tail -2 | head -1)" = "list: Deck-ready" ] && [ "$(grep "^list: " "$OUT/app.log" | tail -1)" = "list: Home" ] && [ -z "$(xdotool search --name "^Deck Ports Manager: Add a port$" 2>/dev/null)" ]'
        else
            check "the Add a port tile opens its page again" false
        fi
        # B twice from a search with text in All ports: the first clears the search, the second goes Home.
        xdotool windowfocus "$win" 2>/dev/null; sleep 0.4; xdotool key Tab; sleep 1; xdotool type --delay 120 "pin"; sleep 0.8
        xdotool key Escape; sleep 1; xdotool key Escape; sleep 1.2
        check "B twice from a search in All ports: cleared, then Home" '[ "$(grep "^list: " "$OUT/app.log" | tail -2 | head -1)" = "list: search cleared" ] && [ "$(grep "^list: " "$OUT/app.log" | tail -1)" = "list: Home" ] && kill -0 "$APP" 2>/dev/null'
        # Settings is a page of groups: the title bar's Settings button opens it, then the keyboard picks a group and a card
        # (Down presses; Up first, since a page remembers where the cursor was). The pages are one window each, found by
        # name ("Settings", "Settings: Ports"...). Without a window manager keys reach a window only after an explicit
        # focus (press_keys). The Down counts come from src/fork/settings_menu.rs when it is next to this script.
        MENU_SRC="$(dirname "$(readlink -f "$0")")/../src/fork/settings_menu.rs"
        downs() { local out="" i; for i in $(seq 1 "$1"); do out="$out Down"; done; echo "$out"; }
        group_downs() {
            local n=""
            [ -f "$MENU_SRC" ] && n=$(grep -m1 'pub const ALL: \[Group' "$MENU_SRC" | sed 's/.*= \[//; s/\].*//' | tr ',' '\n' | awk -v g="Group::$1" '$1==g {print NR - 1; exit}')
            [ -z "$n" ] && case "$1" in Ports) n=0 ;; Steam) n=1 ;; Saves) n=2 ;; App) n=3 ;; Updates) n=4 ;; esac
            echo "$n"
        }
        entry_downs() {  # $1 = group, $2 = entry
            local n=""
            [ -f "$MENU_SRC" ] && n=$(grep -m1 "Group::$1 => &\[" "$MENU_SRC" | sed 's/.*&\[//; s/\].*//' | tr ',' '\n' | awk -v e="Entry::$2" '$1==e {print NR - 1; exit}')
            [ -z "$n" ] && case "$1:$2" in Ports:AddPort|Updates:CheckUpdates) n=0 ;; Ports:ImportPorts) n=1 ;; Ports:PortsFolder) n=2 ;; Ports:GameFiles) n=3 ;; Ports:KeptFiles) n=5 ;; Steam:ArtSources) n=1 ;; Saves:CloudBackup) n=2 ;; esac
            echo "$n"
        }
        open_settings_page() {
            eval "$(xdotool getwindowgeometry --shell "$win" 2>/dev/null)"
            xdotool windowfocus "$win" 2>/dev/null; sleep 0.4
            # The button only reacts to a click once the pointer has been seen over it (no window manager here).
            xdotool mousemove $((X + 250)) $((Y + 150)); sleep 0.3; xdotool mousemove $((X + 312)) $((Y + 16)); sleep 0.5; xdotool click 1
            find_window '^Deck Ports Manager: Settings$' >/dev/null
        }
        settings_group() {  # $1 = group: from the first page into that group's page
            local s; s=$(find_window '^Deck Ports Manager: Settings$') || return 1
            press_keys "$s" Up Up Up Up Up $(downs "$(group_downs "$1")") Return
            find_window "^Deck Ports Manager: Settings: $1\$" >/dev/null
        }
        settings_entry() {  # $1 = group, $2 = entry: from the first page into the card
            settings_group "$1" || return 1
            local g; g=$(find_window "^Deck Ports Manager: Settings: $1\$") || return 1
            press_keys "$g" Up Up Up Up Up $(downs "$(entry_downs "$1" "$2")") Return
        }
        close_settings() {  # B on a group page, B on the first page
            local w
            for _ in 1 2; do
                w=$(xdotool search --onlyvisible --name '^Deck Ports Manager: Settings' 2>/dev/null | tail -1); [ -n "$w" ] || break
                press_keys "$w" Escape; sleep 1.2
            done
        }
        eval "$(xdotool getwindowgeometry --shell "$win" 2>/dev/null)"
        if [ -n "${X:-}" ] && [ -n "${Y:-}" ]; then
            # Settings > Fix problems is the card after the groups: it opens the checkup, B leaves it.
            n_groups=5
            [ -f "$MENU_SRC" ] && n_groups=$(grep -m1 'pub const ALL: \[Group' "$MENU_SRC" | sed 's/.*= \[//; s/\].*//' | tr ',' '\n' | grep -c 'Group::')
            open_settings_page
            sw=$(find_window '^Deck Ports Manager: Settings$') && press_keys "$sw" Up Up Up Up Up Up $(downs "$n_groups") Return
            fp=$(find_window '^Deck Ports Manager: Fix problems$') || fp=""
            sleep 2
            import -window root "$OUT/settings-fix-problems.png" 2>/dev/null
            check "Settings > Fix problems opens the checkup" '[ -n "$fp" ] && kill -0 "$APP" 2>/dev/null && ! grep -q "panicked" "$OUT/app.log"'
            if [ -n "$fp" ]; then press_keys "$fp" Escape; sleep 1.5; fi
            xdotool windowfocus "$win" 2>/dev/null; sleep 0.5
            # One window: while Settings is open the main window shows a dim layer ("Settings is open", "Go back to it").
            # The main window is brought in front of Settings (as a click on it does on the Deck); Return there, and a click
            # on the layer, each ask the window manager to bring the Settings window back, by its window id. There is no
            # window manager here, so the request (logged with the id) is what is checked, not the stacking. Closing
            # Settings clears the layer.
            layer_lines() { grep -c "^dialog layer: back to \"Settings\" (x11 window $1)" "$OUT/app.log"; }
            open_settings_page
            sw=$(find_window '^Deck Ports Manager: Settings$') || sw=""
            sleep 0.8
            check "opening Settings puts the layer on the main window" '[ -n "$sw" ] && [ "$(grep "^dialog layer: " "$OUT/app.log" | tail -1)" = "dialog layer: shown \"Settings is open\"" ]'
            if [ -n "$sw" ]; then
                want=$(printf '0x%x' "$sw")
                xdotool windowraise "$win"; xdotool windowfocus "$win" 2>/dev/null; sleep 0.6
                import -window "$win" "$OUT/layer-windowed.png" 2>/dev/null
                before=$(layer_lines "$want")
                xdotool key Return; sleep 0.8
                check "Return on the layer brings Settings back (window $want)" '[ "$(layer_lines "$want")" -eq $((before + 1)) ] && [ -n "$(xdotool search --name "^Deck Ports Manager: Settings$" 2>/dev/null)" ]'
                eval "$(xdotool getwindowgeometry --shell "$win" 2>/dev/null)"
                xdotool windowraise "$win"; sleep 0.3
                xdotool mousemove $((X + WIDTH / 2)) $((Y + HEIGHT / 2)); sleep 0.3; xdotool mousemove_relative -- 4 4; sleep 0.3; xdotool click 1; sleep 0.8
                check "a click on the layer brings Settings back (window $want)" '[ "$(layer_lines "$want")" -eq $((before + 2)) ]'
                xdotool windowraise "$win"; xdotool windowfocus "$win" 2>/dev/null; sleep 0.4; xdotool key Escape; sleep 0.8
                check "Escape (B) on the layer brings Settings back too, and closes nothing (window $want)" '[ "$(layer_lines "$want")" -eq $((before + 3)) ] && [ -n "$(xdotool search --name "^Deck Ports Manager: Settings$" 2>/dev/null)" ] && kill -0 "$APP" 2>/dev/null'
                check "the layer does not act on the main window behind it (still Home, Settings still open)" '[ "$(grep "^list: " "$OUT/app.log" | tail -1)" = "list: Home" ] && [ -n "$(xdotool search --name "^Deck Ports Manager: Settings$" 2>/dev/null)" ] && kill -0 "$APP" 2>/dev/null'
                xdotool windowraise "$sw"; sleep 0.3
                press_keys "$sw" Escape; sleep 1.5
                check "closing Settings clears the layer" '[ -z "$(xdotool search --name "^Deck Ports Manager: Settings" 2>/dev/null)" ] && [ "$(grep "^dialog layer: " "$OUT/app.log" | tail -1)" = "dialog layer: hidden" ]'
                import -window "$win" "$OUT/layer-cleared.png" 2>/dev/null
            fi
            xdotool windowfocus "$win" 2>/dev/null; sleep 0.5
            # Settings > Ports > Import ports opens (its scan runs on a worker thread), shows the screen and closes again.
            open_settings_page && settings_entry Ports ImportPorts; sleep 4
            import -window root "$OUT/import-ports.png" 2>/dev/null
            imp=$(xdotool search --name "^Deck Ports Manager: Import ports: " 2>/dev/null | head -1)
            imp_alive=no; kill -0 "$APP" 2>/dev/null && imp_alive=yes
            check "Import ports screen opens and the app keeps running" '[ "$imp_alive" = yes ] && ! grep -q "panicked" "$OUT/app.log" && [ -n "$imp" ]'
            if [ -n "$imp" ]; then
                xdotool windowfocus "$imp" 2>/dev/null; sleep 0.3; xdotool key Escape
                for _wait in $(seq 1 15); do [ -z "$(xdotool search --name "^Deck Ports Manager: Import ports: " 2>/dev/null)" ] && break; sleep 0.2; done
                # Import ports was opened from Settings > Ports: leaving it comes back to that page.
                sleep 1
                check "leaving Import ports returns to the Ports page of Settings" '[ -n "$(xdotool search --name "^Deck Ports Manager: Settings: Ports$" 2>/dev/null)" ]'
                close_settings
                xdotool windowfocus "$win" 2>/dev/null; sleep 0.5
            fi
            # Settings > Steam > Artwork sources opens and shows its rows, and its Back row returns to the Steam page.
            open_settings_page && settings_entry Steam ArtSources; sleep 2
            import -window root "$OUT/art-sources.png" 2>/dev/null
            art=$(xdotool search --name "^Deck Ports Manager: Artwork sources$" 2>/dev/null | head -1)
            art_alive=no; kill -0 "$APP" 2>/dev/null && art_alive=yes
            check "Artwork sources screen opens and the app keeps running" '[ "$art_alive" = yes ] && ! grep -q "panicked" "$OUT/app.log" && [ -n "$art" ]'
            if [ -n "$art" ]; then
                eval "$(xdotool getwindowgeometry --shell "$art")"; AX=$X; AY=$Y
                xdotool mousemove $((AX + 100)) $((AY + 63 + 42 * 9)) click 1; sleep 1.5
                check "Back leaves Artwork sources for the Steam settings page" '[ -z "$(xdotool search --name "^Deck Ports Manager: Artwork sources$" 2>/dev/null)" ] && [ -n "$(xdotool search --name "^Deck Ports Manager: Settings: Steam$" 2>/dev/null)" ]'
                # B on the Steam page goes up to Settings (one level). Then Settings > Saves > Cloud backup opens a page titled
                # "Cloud backup" (the intro, or "isn't set up in this build" when the build has no Google client), and B
                # returns to the Saves page.
                sg=$(xdotool search --name "^Deck Ports Manager: Settings: Steam$" 2>/dev/null | head -1)
                if [ -n "$sg" ]; then
                    press_keys "$sg" Escape; sleep 1.5
                    check "B goes up from the Steam page to Settings" '[ -z "$(xdotool search --name "^Deck Ports Manager: Settings: Steam$" 2>/dev/null)" ] && [ -n "$(xdotool search --name "^Deck Ports Manager: Settings$" 2>/dev/null)" ]'
                    settings_entry Saves CloudBackup; sleep 2
                    import -window root "$OUT/cloud-backup.png" 2>/dev/null
                    cb=$(xdotool search --name "^Deck Ports Manager: Cloud backup$" 2>/dev/null | head -1)
                    check "Cloud backup screen opens and the app keeps running" '[ -n "$cb" ] && kill -0 "$APP" 2>/dev/null && ! grep -q "panicked" "$OUT/app.log"'
                    if [ -n "$cb" ]; then
                        press_keys "$cb" Escape; sleep 1.5
                        check "B leaves Cloud backup for the Saves settings page" '[ -z "$(xdotool search --name "^Deck Ports Manager: Cloud backup$" 2>/dev/null)" ] && [ -n "$(xdotool search --name "^Deck Ports Manager: Settings: Saves$" 2>/dev/null)" ]'
                    fi
                fi
                close_settings
                eval "$(xdotool getwindowgeometry --shell "$win")"
            fi
            # Settings > Ports > Add a port opens the Add a port page, and B returns to the Ports page (one level up).
            open_settings_page && settings_entry Ports AddPort; sleep 1.5
            ap2=$(xdotool search --name "^Deck Ports Manager: Add a port$" 2>/dev/null | head -1)
            check "Settings > Ports > Add a port opens the Add a port page" '[ -n "$ap2" ] && kill -0 "$APP" 2>/dev/null'
            if [ -n "$ap2" ]; then
                press_keys "$ap2" Escape; sleep 1.5
                check "B on Add a port returns to the Ports page of Settings" '[ -z "$(xdotool search --name "^Deck Ports Manager: Add a port$" 2>/dev/null)" ] && [ -n "$(xdotool search --name "^Deck Ports Manager: Settings: Ports$" 2>/dev/null)" ]'
            fi
            close_settings
            # Settings > Ports > Kept files lists what uninstalls kept; B returns to the Ports page.
            open_settings_page && settings_entry Ports KeptFiles; sleep 1.5
            kf=$(xdotool search --name "^Deck Ports Manager: Settings: Ports: Kept files$" 2>/dev/null | head -1)
            import -window root "$OUT/kept-files.png" 2>/dev/null
            check "Settings > Ports > Kept files opens its page" '[ -n "$kf" ] && kill -0 "$APP" 2>/dev/null && ! grep -q "panicked" "$OUT/app.log"'
            if [ -n "$kf" ]; then
                press_keys "$kf" Escape; sleep 1.5
                check "B on Kept files returns to the Ports page of Settings" '[ -z "$(xdotool search --name "^Deck Ports Manager: Settings: Ports: Kept files$" 2>/dev/null)" ] && [ -n "$(xdotool search --name "^Deck Ports Manager: Settings: Ports$" 2>/dev/null)" ]'
            fi
            close_settings
            # A switch flips in place and keeps the cursor: Settings > Updates > Check for Updates. Two presses of A flip it twice,
            # which they only do if the cursor is still on it after the first (state.json holds the value).
            release_sync() { grep -o '"release_sync": *[a-z]*' "$APP_DIR/state.json" 2>/dev/null | grep -o '[a-z]*$' | tail -1; }
            open_settings_page && settings_entry Updates CheckUpdates; sleep 1
            rs1=$(release_sync)
            ug=$(xdotool search --name "^Deck Ports Manager: Settings: Updates$" 2>/dev/null | head -1)
            if [ -n "$ug" ]; then
                press_keys "$ug" Return; sleep 1; rs2=$(release_sync)
                press_keys "$ug" Return; sleep 1; rs3=$(release_sync)
                echo "release_sync: ${rs1:-none} -> ${rs2:-none} -> ${rs3:-none}" >>"$OUT/focus.log"
                import -window root "$OUT/switch-flipped.png" 2>/dev/null
                check "a switch flips in place and the cursor stays on it (release_sync ${rs1:-none} -> ${rs2:-none} -> ${rs3:-none})" '[ -n "$rs2" ] && [ -n "$rs3" ] && [ "$rs2" != "$rs3" ] && [ -n "$(xdotool search --name "^Deck Ports Manager: Settings: Updates$" 2>/dev/null)" ]'
            else
                check "the Updates page opens" 'false'
            fi
            close_settings
            # Settings > Ports > Game files opens without a crash.
            open_settings_page && settings_entry Ports GameFiles; sleep 2
            import -window root "$OUT/game-files.png" 2>/dev/null
            gf_alive=no; kill -0 "$APP" 2>/dev/null && gf_alive=yes
            check "Game files screen opens and the app keeps running" '[ "$gf_alive" = yes ] && ! grep -q "panicked" "$OUT/app.log" && [ -n "$(xdotool search --name "^Deck Ports Manager: Game files$" 2>/dev/null)" ]'
            # Settings > Ports > Ports folder, then its "Choose another folder..." row (third) opens the picker;
            # the picker's close button cancels back to the Ports folder list.
            # A dialog's own close button (top right), since keys need focus and there is no window manager.
            close_named() {
                local w; w=$(xdotool search --name "$1" 2>/dev/null | head -1); [ -n "$w" ] || return 0
                eval "$(xdotool getwindowgeometry --shell "$w")"; xdotool mousemove $((X + WIDTH - 16)) $((Y + 16)) click 1; sleep 1
            }
            close_named "^Deck Ports Manager: Game files$"
            eval "$(xdotool getwindowgeometry --shell "$win")"
            open_settings_page && settings_entry Ports PortsFolder; sleep 2
            pf=$(xdotool search --name "^Deck Ports Manager: Ports folder$" 2>/dev/null | head -1)
            check "Ports folder screen opens and the app keeps running" 'kill -0 "$APP" 2>/dev/null && ! grep -q "panicked" "$OUT/app.log" && [ -n "$pf" ]'
            if [ -n "$pf" ]; then
                eval "$(xdotool getwindowgeometry --shell "$pf")"; PX=$X; PY=$Y
                xdotool mousemove $((PX + 100)) $((PY + 63 + 42 * 2)) click 1; sleep 2
                import -window root "$OUT/folder-picker.png" 2>/dev/null
                picker=$(xdotool search --name "^Deck Ports Manager: Choose the ports folder$" 2>/dev/null | head -1)
                check "Choose another folder... opens the folder picker" '[ -n "$picker" ] && kill -0 "$APP" 2>/dev/null'
                if [ -n "$picker" ]; then
                    eval "$(xdotool getwindowgeometry --shell "$picker")"
                    xdotool mousemove $((X + WIDTH - 16)) $((Y + 16)) click 1; sleep 1.5
                    check "closing the picker returns to the Ports folder list" '[ -z "$(xdotool search --name "^Deck Ports Manager: Choose the ports folder$" 2>/dev/null)" ] && [ -n "$(xdotool search --name "^Deck Ports Manager: Ports folder$" 2>/dev/null)" ]'
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
                    pf=$(wait_window "^Deck Ports Manager: Ports folder$")
                    if [ -n "$pf" ]; then
                        eval "$(xdotool getwindowgeometry --shell "$pf")"
                        xdotool mousemove $((X + 100)) $((Y + 63 + 42 * 2)) click 1; sleep 2
                        E2E=1
                        # From "Use this folder": Up skips the heading to the Games place (Places: Home, Games).
                        key_to "^Deck Ports Manager: Choose the ports folder$" Up Return
                        key_to "^Deck Ports Manager: Choose the ports folder$" Down Return
                        nf=$(wait_window "^Deck Ports Manager: New folder in")
                        if [ -n "$nf" ]; then xdotool windowfocus "$nf" 2>/dev/null; sleep 0.4; xdotool type --delay 80 "Ports"; sleep 0.3; xdotool key Return; sleep 1.5; fi
                        key_to "^Deck Ports Manager: Choose the ports folder$" Return
                        key_to "^Deck Ports Manager: Switch the ports folder" Return
                        key_to "^Deck Ports Manager: Remove the old copy now" Return
                        key_to "^Deck Ports Manager: Remove the old copy now" Return
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

# One window, big mode: a full-screen page offers "Show the app" (Y). It steps aside for the main window and its layer;
# Return on the layer brings the page back as it was, the cursor where it was (Return there opens the group it was on).
# A fresh copy started in big mode; with no window manager here the script sizes the main window to the screen itself.
BIG_DIR="$(mktemp -d -p "${SMOKE_HOME_BASE:-$HOME}" dpm-smoke-big.XXXXXX)" || setup_fail "mktemp big"
mkdir -p "$BIG_DIR/home"
cp "$BIN_DIR/ports_launcher" "$BIN_DIR/7zzs" "$BIG_DIR/"
for f in ports.json themes.json; do [ -f "$BIN_DIR/$f" ] && cp "$BIN_DIR/$f" "$BIG_DIR/"; done
echo '{"setup_done": true}' > "$BIG_DIR/fork-settings.json"
echo '{"ui": {"fullscreen": true}}' > "$BIG_DIR/state.json"
env -i HOME="$BIG_DIR/home" PATH="$PATH" DISPLAY="$DISPLAY" XDG_RUNTIME_DIR="$BIG_DIR" RUST_BACKTRACE=1 \
    "$BIG_DIR/ports_launcher" >"$OUT/app-big.log" 2>&1 &
BIG=$!
sleep 8
bwin=$(xdotool search --name "^Ports Launcher$" 2>/dev/null | head -1)
if [ -n "$bwin" ]; then
    xdotool windowsize "$bwin" 1280 800; sleep 1; xdotool windowfocus "$bwin" 2>/dev/null; sleep 0.4
    eval "$(xdotool getwindowgeometry --shell "$bwin" 2>/dev/null)"
    xdotool mousemove $((X + 250)) $((Y + 150)); sleep 0.3; xdotool mousemove $((X + WIDTH - 88)) $((Y + 18)); sleep 0.5; xdotool click 1
    bs=$(find_window '^Deck Ports Manager: Settings$') || bs=""
    check "big mode: the gear opens Settings" '[ -n "$bs" ]'
    if [ -n "$bs" ]; then
        sleep 1; import -window "$bs" "$OUT/page-show-app.png" 2>/dev/null
        press_keys "$bs" Up Up Up Up Up $(downs "$(group_downs Saves)") y; sleep 1
        check "big mode: Y on Settings shows the app (the page steps aside, the layer is up)" 'grep -q "^dialog layer: showing the app" "$OUT/app-big.log" && [ "$(grep "^dialog layer: shown" "$OUT/app-big.log" | tail -1)" = "dialog layer: shown \"Settings is open\"" ]'
        xdotool windowraise "$bwin"; xdotool windowfocus "$bwin" 2>/dev/null; sleep 0.4; xdotool key Return; sleep 0.8
        check "big mode: Return on the layer brings Settings back" 'grep -q "^dialog layer: back to \"Settings\" (x11 window $(printf 0x%x "$bs"))" "$OUT/app-big.log"'
        xdotool windowraise "$bs"; press_keys "$bs" Return
        check "big mode: the cursor was kept (Return opens the group it was on)" '[ -n "$(find_window "^Deck Ports Manager: Settings: Saves\$")" ]'
    fi
else
    check "big mode: the main window opens" 'false'
fi
kill "$BIG" 2>/dev/null; wait "$BIG" 2>/dev/null; rm -rf "$BIG_DIR"

echo "--- app output (last 20 lines) ---"; tail -20 "$OUT/app.log"
[ "$fail" -eq 0 ] && echo "SMOKE TEST: PASS" || echo "SMOKE TEST: FAIL"
exit "$fail"
