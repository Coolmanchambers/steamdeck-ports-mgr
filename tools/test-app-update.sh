#!/usr/bin/env bash
# In-app update regression test: the real app, started from an old build, runs its updater from the
# UI (Settings > Force update) and ends up on the new build. Offline and deterministic: the "release"
# is a folder served over file:// (the updater honors DPM_RELEASE_BASE for https:// and file://, and
# the app passes its environment on to the updater), so there is no GitHub API and no real release.
#
# The freshly packaged folder is the NEW build. The OLD build is the same program with a marker
# appended after the ELF (it still runs, but has a different hash), the same updater with a marker
# line, a different BUILD_INFO, and the old install's own catalogs. No second cargo build: the
# updater is binary-agnostic, so what is under test is the path app -> updater -> swap -> restart,
# and the hash and BUILD_INFO tell the two builds apart.
# Compatibility gap: the "old" updater here is the new script (plus a marker line), and the "old"
# program is the new program, so a change of package layout between releases (a renamed or moved
# file in the tarball, a new required file) is NOT caught. It does not prove an old release's code
# can be updated by a new one; the real-release run in scripts/update_e2e does that by hand.
#
# Case 1: a good release. Checks that the app quits by itself with exit 0 (an updater SIGTERM or
#   SIGKILL after its 10 s wait would not pass), updater.log ends with "=== done", BUILD_INFO,
#   the program, 7zzs and the updater now match the new package, .previous/ holds the old program,
#   the restarted process runs the new file, and Library, saves, catalogs and state are untouched
#   (state.json is rewritten by the app itself, so it is compared as JSON apart from the timestamp
#   the app stamps when it launches the updater).
# Case 2: the same release with a tampered .sha256. Nothing may change, update-failed.txt must say
#   why, and the app must reopen on the old build.
#
# Not covered on purpose: the automatic "update found at startup" path asks the GitHub API for the
# latest release, which is neither offline nor deterministic. Unit tests cover its decision
# (is_newer_date, should_check_launcher_update); the seeded state.json here switches that check off.
# Also not covered: an updater that starts the app twice. The app's single-instance lock (a socket
# in XDG_RUNTIME_DIR) makes the second copy exit at once, which would mask it here; that case is
# covered by tools/test-fork-updater.sh.
#
# Run it locally (Linux, or WSL; needs xvfb, x11-utils, xdotool, imagemagick, curl, python3):
#   bash tools/test-app-update.sh <packaged folder, e.g. dist/steamdeck-ports-mgr> [output folder]
# The folder needs ports_launcher, 7zzs, ports_launcher_updater.sh, ports.json and themes.json
# (BUILD_INFO is made up if it is missing). It starts its own Xvfb on a free display (DISPLAY_NUM pins one).
# Exit 0 = pass, 1 = the app or updater failed a check, 2 = the test setup failed (not the app's fault).
set -u
BIN_DIR="$(readlink -f "${1:?folder with the packaged build}")"
OUT="$(readlink -f "${2:-app-update-out}")"
SRC_DIALOGS="$(dirname "$(readlink -f "$0")")/../src/app/dialogs.rs"
PKG=steamdeck-ports-mgr
ASSET="$PKG-linux-x86_64.tar.gz"

setup_fail() { echo "SETUP FAILED: $1"; exit 2; }
for tool in Xvfb xdpyinfo xwininfo xdotool import curl tar sha256sum pgrep python3 mktemp; do
    command -v "$tool" >/dev/null || setup_fail "$tool is not installed"
done
for f in ports_launcher 7zzs ports_launcher_updater.sh ports.json themes.json; do
    [ -f "$BIN_DIR/$f" ] || setup_fail "$f is missing in $BIN_DIR"
done
mkdir -p "$OUT" || setup_fail "can't create $OUT"
W="$(mktemp -d)" && W="$(readlink -f "$W")" || setup_fail "mktemp"

# How many Down presses reach "Force update" in the Settings list: its position in SETTINGS_ENTRIES.
# Read from the source when it is next to this script. If the source is there but the list can't be
# read (the code was restructured), that is a setup failure, not a reason to guess.
if [ -f "$SRC_DIALOGS" ]; then
    DOWNS=$(awk '/const SETTINGS_ENTRIES/ {on=1; next} on && /^\];/ {exit} on && /SettingsEntry::ForceUpdate/ {print n+0; exit} on && /SettingsEntry::/ {n++}' "$SRC_DIALOGS")
    { [ -n "$DOWNS" ] && [ "$DOWNS" -ge 0 ] 2>/dev/null; } || setup_fail "could not find ForceUpdate in SETTINGS_ENTRIES in $SRC_DIALOGS (got '${DOWNS:-}'); update the awk in this script"
else
    DOWNS=9; echo "note: $SRC_DIALOGS is not here (package tested without the source), assuming $DOWNS"
fi
echo "Force update is $DOWNS Down presses into the Settings list"

APP=""; XVFB=""
cleanup() {
    # Only processes this test started: the programs under $W, matched by their full path.
    for p in $(pgrep -x ports_launcher 2>/dev/null); do
        case "$(readlink -f "/proc/$p/exe" 2>/dev/null)" in "$W"/*) kill "$p" 2>/dev/null ;; esac
    done
    [ -n "$XVFB" ] && kill "$XVFB" 2>/dev/null
    rm -rf "$W"
}
trap cleanup EXIT

# ---- the NEW package, served as a fake release ----
PK="$W/pkgroot/$PKG"
mkdir -p "$PK" || setup_fail "mkdir"
cp "$BIN_DIR/ports_launcher" "$BIN_DIR/7zzs" "$BIN_DIR/ports_launcher_updater.sh" "$PK/" || setup_fail "copy of the package"
chmod 755 "$PK/ports_launcher" "$PK/7zzs" "$PK/ports_launcher_updater.sh"
# Catalogs in the package differ by a trailing newline (still valid JSON): the updater must never install them.
for f in ports.json themes.json; do cp "$BIN_DIR/$f" "$PK/$f" && printf '\n' >> "$PK/$f" || setup_fail "copy of $f"; done
if [ -f "$BIN_DIR/BUILD_INFO" ]; then cp "$BIN_DIR/BUILD_INFO" "$PK/BUILD_INFO"; else
    printf 'commit=new-test-build\nbuild_date=2026-01-01\nrun=test-app-update\n' > "$PK/BUILD_INFO"; fi
OLD_BUILD_INFO=$'commit=0000000000000000000000000000000000000000\nbuild_date=2000-01-01\nrun=test-app-update (old build)\n'
printf '%s' "$OLD_BUILD_INFO" > "$W/old-build-info"
cmp -s "$W/old-build-info" "$PK/BUILD_INFO" && setup_fail "old and new BUILD_INFO are identical"

REL_GOOD="$W/rel-good"; REL_BAD="$W/rel-bad"
mkdir -p "$REL_GOOD" "$REL_BAD" || setup_fail "mkdir"
tar -C "$W/pkgroot" -czf "$REL_GOOD/$ASSET" "$PKG" || setup_fail "tar"
(cd "$REL_GOOD" && sha256sum "$ASSET" > "$ASSET.sha256") || setup_fail "sha256sum"
cp "$REL_GOOD/$ASSET" "$REL_BAD/$ASSET"
# Always changes the first hex digit, so the checksum is well formed but wrong.
sed 's/^0/X/; s/^[^X]/0/; s/^X/1/' "$REL_GOOD/$ASSET.sha256" > "$REL_BAD/$ASSET.sha256"
cmp -s "$REL_GOOD/$ASSET.sha256" "$REL_BAD/$ASSET.sha256" && setup_fail "could not tamper with the checksum"
NEW_SHA=$(sha256sum "$PK/ports_launcher" | cut -d' ' -f1)
NEW_7Z=$(sha256sum "$PK/7zzs" | cut -d' ' -f1)

# ---- the OLD install ----
install_old() {  # $1 = folder
    local d="$1"
    mkdir -p "$d/Library/game" "$d/Saves Backup/game" "$d/cache" || setup_fail "mkdir $d"
    cp "$BIN_DIR/ports_launcher" "$d/ports_launcher" && printf '\0dpm-test-old-build\n' >> "$d/ports_launcher" || setup_fail "old program"
    cp "$BIN_DIR/7zzs" "$d/7zzs" && printf '\0dpm-test-old-build\n' >> "$d/7zzs" || setup_fail "old 7zzs"
    cp "$BIN_DIR/ports_launcher_updater.sh" "$d/ports_launcher_updater.sh" && printf '# old updater\n' >> "$d/ports_launcher_updater.sh" || setup_fail "old updater"
    chmod 755 "$d/ports_launcher" "$d/7zzs" "$d/ports_launcher_updater.sh"
    cp "$BIN_DIR/ports.json" "$BIN_DIR/themes.json" "$d/" || setup_fail "old catalogs"
    cp "$W/old-build-info" "$d/BUILD_INFO"
    head -c 4096 /dev/urandom > "$d/Library/game/save.dat"
    head -c 2048 /dev/urandom > "$d/Saves Backup/game/backup.zip"
    echo '{"local": "ports.local.json must survive"}' > "$d/ports.local.json"
    # Setup was finished on this install, so the first-run wizard does not cover the main window.
    echo '{"setup_done": true}' > "$d/fork-settings.json"
    echo 'cache bytes' > "$d/cache/entry"
    # No release or catalog checks over the network; one installed port to prove state.json survives.
    printf '{"release_sync": false, "catalog_sync": false, "last_launcher_update_check": "%s", "launcher_update_available": false,
 "installed": {"dpm-test-port": {"installed_tag": "v1", "installed_at": "2026-01-01T00:00:00+00:00", "favorite_exe": null, "update": true, "playtime_seconds": 42, "last_played_at": ""}}}\n' \
        "$(date -u +%Y-%m-%dT%H:%M:%S+00:00)" > "$d/state.json"
}
# The seeded files, named one by one (a missing one makes sha256sum fail and the comparison with it).
protected_sums() { (cd "$1" && sha256sum Library/game/save.dat "Saves Backup/game/backup.zip" cache/entry ports.json themes.json ports.local.json 2>&1); }
state_same() {  # $1 = state.json, $2 = a saved copy; equal as JSON apart from the launch-time stamp
    python3 -I -c 'import json,sys
a, b = (json.load(open(p)) for p in sys.argv[1:3])
for d in (a, b): d.pop("last_launcher_update_check", None)
sys.exit(a != b)' "$1" "$2"
}

# ---- the display ----
# A free display number is picked at random and retried if Xvfb refuses it, so a second Xvfb or test
# on the same machine (a shared dev box, WSL) is not disturbed. DISPLAY_NUM pins one.
XVFB=""
for attempt in $(seq 1 20); do
    n="${DISPLAY_NUM:-$((100 + RANDOM % 800))}"
    [ -e "/tmp/.X$n-lock" ] && { [ -n "${DISPLAY_NUM:-}" ] && break; continue; }
    Xvfb ":$n" -screen 0 1280x800x24 -nolisten tcp >"$OUT/xvfb.log" 2>&1 &
    XVFB=$!
    export DISPLAY=":$n"
    for _ in $(seq 1 25); do xdpyinfo >/dev/null 2>&1 && break; kill -0 "$XVFB" 2>/dev/null || break; sleep 0.2; done
    kill -0 "$XVFB" 2>/dev/null && xdpyinfo >/dev/null 2>&1 && break
    kill "$XVFB" 2>/dev/null; XVFB=""; sleep 0.5
done
[ -n "$XVFB" ] || { cat "$OUT/xvfb.log"; setup_fail "no Xvfb display came up"; }
echo "virtual display $DISPLAY"

fail=0
case_fail=0
check() { if eval "$2"; then echo "ok:   $1"; else echo "FAIL: $1"; fail=1; case_fail=1; fi; }
wait_for() { local t=$1; shift; for _ in $(seq 1 $((t * 5))); do eval "$@" && return 0; sleep 0.2; done; return 1; }
shot() { import -window root "$OUT/$CASE/$1.png" 2>/dev/null; }
sha() { sha256sum "$1" 2>/dev/null | cut -d' ' -f1; }

# The program under test is whichever $APP/ports_launcher runs; other copies on the machine are ignored.
our_pid() {
    local p
    for p in $(pgrep -x ports_launcher); do
        [ "$(readlink -f "/proc/$p/exe" 2>/dev/null | sed 's/ (deleted)$//')" = "$APP/ports_launcher" ] && echo "$p"
    done | head -1
}
# A process that has exited but is not reaped yet (zombie) is not alive.
alive() { [ -r "/proc/$1/status" ] && ! grep -q "^State:.*Z" "/proc/$1/status"; }
count_pids() {
    local p n=0
    for p in $(pgrep -x ports_launcher); do
        [ "$(readlink -f "/proc/$p/exe" 2>/dev/null | sed 's/ (deleted)$//')" = "$APP/ports_launcher" ] && n=$((n + 1))
    done
    echo "$n"
}
main_win() { xdotool search --name '^Ports Launcher$' 2>/dev/null | head -1; }
settings_win() { xdotool search --onlyvisible --name '^Settings$' 2>/dev/null | tail -1; }
last_line() { tail -n 1 "$APP/updater.log" 2>/dev/null; }

launch() {  # $1 = DPM_RELEASE_BASE; a bare session like Game Mode: no D-Bus, so no tray host
    env -i HOME="$W/$CASE/home" PATH="$PATH" DISPLAY="$DISPLAY" XDG_RUNTIME_DIR="$W/$CASE/run" RUST_BACKTRACE=1 DPM_RELEASE_BASE="$1" \
        "$APP/ports_launcher" >>"$OUT/$CASE/app.log" 2>&1 &
    OLD_PID=$!   # env execs the program, so this is the app's own PID and a child of this shell (wait works)
}

# The old app must quit by itself and exit 0 (the updater would otherwise SIGTERM it after 10 s).
old_app_quit_cleanly() {
    wait_for 20 "! alive $OLD_PID" || return 1
    wait "$OLD_PID"; OLD_EXIT=$?
    echo "      (old app exit code $OLD_EXIT)"
    [ "$OLD_EXIT" -eq 0 ]
}

open_settings_and_force_update() {
    eval "$(xdotool getwindowgeometry --shell "$(main_win)")"
    local dlg="" try
    for try in 1 2 3 4; do   # the title bar's Settings button; retried in case the first click beats the event loop
        xdotool mousemove $((X + WIDTH - 70)) $((Y + 16)) click 1
        wait_for 3 'dlg=$(settings_win); [ -n "$dlg" ]' && break
    done
    xdotool mousemove 0 0   # park the pointer so hovering can't move the list selection
    shot settings
    echo "settings dialog window: ${dlg:-none} ($(xdotool getwindowname "$dlg" 2>/dev/null))"
    [ -n "$dlg" ] && [ "$(xdotool getwindowname "$dlg" 2>/dev/null)" = "Settings" ] || return 1
    xdotool windowfocus "$dlg"; sleep 0.4
    local keys="" i
    for i in $(seq 1 "$DOWNS"); do keys="$keys Down"; done
    [ -n "$keys" ] && xdotool key --delay 120 $keys
    sleep 0.5; shot force-selected
    xdotool key Return
}

diagnostics() {
    echo "--- diagnostics for $CASE"
    echo "processes:"; ps -eo pid,ppid,stat,etime,args | grep -E "[0-9] (/[^ ]*/ports_launcher( |$)|\[ports_launcher\]|Xvfb $DISPLAY )"
    echo "windows:"; xwininfo -root -tree 2>/dev/null | grep -E '"' | head -20
    echo "install folder:"; ls -la "$APP" 2>&1 | head -30; ls -la "$APP/.previous" 2>&1 | head
    echo "updater.log:"; cat "$APP/updater.log" 2>&1
    echo "update-failed.txt:"; cat "$APP/update-failed.txt" 2>&1
    echo "app.log (last 20):"; tail -20 "$OUT/$CASE/app.log" 2>&1
    echo "BUILD_INFO:"; cat "$APP/BUILD_INFO" 2>&1
}

begin_case() {  # $1 = name, $2 = release folder
    CASE="$1"; APP="$W/$1/app"; mkdir -p "$OUT/$CASE" "$W/$1/home" "$W/$1/run" && chmod 700 "$W/$1/run" || setup_fail "mkdir for $1"; : > "$OUT/$CASE/app.log"
    install_old "$APP"
    OLD_SHA=$(sha "$APP/ports_launcher"); OLD_7Z=$(sha "$APP/7zzs")
    protected_sums "$APP" > "$W/$CASE.protected"
    case_fail=0
    launch "file://$2"
    wait_for 30 '[ -n "$(main_win)" ]' || { diagnostics; setup_fail "the old app showed no window within 30 s (see $OUT/$CASE/app.log)"; }
    sleep 3   # let it finish its first frames before clicking
    [ "$(our_pid)" = "$OLD_PID" ] || { diagnostics; setup_fail "the old app (pid $OLD_PID) is not running"; }
    echo "old app pid $OLD_PID, hash ${OLD_SHA:0:12}, new package hash ${NEW_SHA:0:12}"
    [ "$OLD_SHA" != "$NEW_SHA" ] || setup_fail "old and new programs are identical"
}

end_case() {
    local p; p=$(our_pid); [ -n "$p" ] && kill "$p" 2>/dev/null
    [ -n "$p" ] && wait_for 10 "! alive $p"; sleep 0.5
    cp "$APP/updater.log" "$APP/update-failed.txt" "$APP/BUILD_INFO" "$OUT/$CASE/" 2>/dev/null
    [ "$case_fail" -eq 0 ] || diagnostics | tee "$OUT/$CASE/diagnostics.txt"
}

# ---------------------------------------------------------------------------------------------
echo "== case 1: Force update from the UI with a good release"
begin_case case1-good "$REL_GOOD"
shot before
if open_settings_and_force_update; then
    check "the app quit by itself with exit 0 for the update" 'old_app_quit_cleanly'
    cp "$APP/state.json" "$W/state.after-close"   # the old app has written its state for the last time
    check "no panic in the old app's output" '! grep -q panicked "$OUT/$CASE/app.log"'
    check "updater.log ends with '=== done'" 'wait_for 60 "[ \"\$(last_line)\" = \"=== done\" ]"'
    check "the app was restarted (one new copy)" 'wait_for 30 "[ \"\$(count_pids)\" -eq 1 ] && [ \"\$(our_pid)\" != \"$OLD_PID\" ]"'
    check "the restarted app shows its window" 'wait_for 30 "[ -n \"\$(main_win)\" ]"'
    sleep 3; shot after
    check "BUILD_INFO matches the new package" 'cmp -s "$APP/BUILD_INFO" "$PK/BUILD_INFO"'
    check "the program file is the new one" '[ "$(sha "$APP/ports_launcher")" = "$NEW_SHA" ]'
    check "7zzs is the new one" '[ "$(sha "$APP/7zzs")" = "$NEW_7Z" ]'
    check "the updater replaced itself with the new one" 'cmp -s "$APP/ports_launcher_updater.sh" "$PK/ports_launcher_updater.sh"'
    check ".previous/ holds the old program" '[ "$(sha "$APP/.previous/ports_launcher")" = "$OLD_SHA" ]'
    check ".previous/ holds the old 7zzs" '[ "$(sha "$APP/.previous/7zzs")" = "$OLD_7Z" ]'
    check "the restarted process runs the new file" '[ "$(sha "/proc/$(our_pid)/exe")" = "$NEW_SHA" ]'
    check "no update-failed.txt" '[ ! -e "$APP/update-failed.txt" ]'
    check "Library, saves, catalogs and cache are byte-identical" 'protected_sums "$APP" | diff -q - "$W/$CASE.protected" >/dev/null'
    check "state.json unchanged (as JSON, ignoring the launch stamp)" 'state_same "$APP/state.json" "$W/state.after-close"'
    check "state.json still lists the installed port" 'grep -q dpm-test-port "$APP/state.json"'
    check "no temporary files left in the install folder" '[ -z "$(ls -A "$APP" | grep -E "^\.(ports_launcher|7zzs|ports_launcher_updater\.sh)\.new$")" ]'
else
    echo "FAIL: could not open the Settings dialog"; fail=1
fi
end_case

echo "== case 2: tampered checksum, nothing may change"
begin_case case2-badsum "$REL_BAD"
shot before
if open_settings_and_force_update; then
    check "the app quit by itself with exit 0 for the update" 'old_app_quit_cleanly'
    cp "$APP/state.json" "$W/state.after-close"
    check "no panic in the old app's output" '! grep -q panicked "$OUT/$CASE/app.log"'
    check "update-failed.txt written" 'wait_for 60 "[ -s \"$APP/update-failed.txt\" ]"'
    check "update-failed.txt says the checksum did not match" 'grep -qi "checksum does not match" "$APP/update-failed.txt"'
    check "the app reopened (one new copy)" 'wait_for 40 "[ \"\$(count_pids)\" -eq 1 ] && [ \"\$(our_pid)\" != \"$OLD_PID\" ]"'
    check "the reopened app shows its window" 'wait_for 30 "[ -n \"\$(main_win)\" ]"'
    sleep 3; shot after
    check "updater.log records the failure and never says done" 'grep -q "^FAILED: checksum does not match" "$APP/updater.log" && ! grep -q "^=== done" "$APP/updater.log"'
    check "the program file is still the old one" '[ "$(sha "$APP/ports_launcher")" = "$OLD_SHA" ]'
    check "7zzs and the updater are still the old ones" '[ "$(sha "$APP/7zzs")" = "$OLD_7Z" ] && tail -n 1 "$APP/ports_launcher_updater.sh" | grep -q "^# old updater$"'
    check "BUILD_INFO is still the old one" 'cmp -s "$APP/BUILD_INFO" "$W/old-build-info"'
    check "no .previous/ (nothing was swapped)" '[ ! -e "$APP/.previous" ]'
    check "the reopened process runs the old file" '[ "$(sha "/proc/$(our_pid)/exe")" = "$OLD_SHA" ]'
    check "Library, saves, catalogs and cache are byte-identical" 'protected_sums "$APP" | diff -q - "$W/$CASE.protected" >/dev/null'
    check "state.json unchanged (as JSON, ignoring the launch stamp)" 'state_same "$APP/state.json" "$W/state.after-close"'
else
    echo "FAIL: could not open the Settings dialog"; fail=1
fi
end_case

[ "$fail" -eq 0 ] && echo "APP UPDATE TEST: PASS" || echo "APP UPDATE TEST: FAIL"
exit "$fail"
