#!/usr/bin/env bash
# Tests the one-file installer (steamdeck-ports-mgr-installer.desktop) offline, in fake home folders.
# Every run goes through the .desktop file's own Exec line: a small parser that follows the
# desktop-entry spec (string escapes, then quoting, then %k) turns it into the command to run, so the
# real packaging is what gets tested, not tools/installer.sh on its own. GLib's launcher runs it once
# too when available. Releases come from file:// (DPM_RELEASE_BASE), no GitHub.
#
#   bash tools/test-installer.sh <installer .desktop> <package .tar.gz> [work folder]
#
# Linux only; needs python3, gcc and flock. Works only inside the work folder (default
# $HOME/dpm-test/installer-test), which it deletes at the start and the end.
set -u
DESKTOP="$(readlink -f "${1:?installer .desktop file}")"
PACKAGE="$(readlink -f "${2:?package .tar.gz}")"
W="${3:-$HOME/dpm-test/installer-test}"
SRC="$(dirname "$(readlink -f "$0")")/installer.sh"
PKG=steamdeck-ports-mgr
ASSET=$PKG-linux-x86_64.tar.gz
LOGNAME_=steamdeck-ports-mgr-installer.log
pass=0; failn=0
ok()  { echo "  PASS $1"; pass=$((pass+1)); }
bad() { echo "  FAIL $1"; failn=$((failn+1)); [ -f "$W/last.out" ] && sed 's/^/      | /' "$W/last.out" | tail -n 25; }
check() { if eval "$2"; then ok "$1"; else bad "$1"; fi; }

[ -f "$DESKTOP" ] && [ -f "$PACKAGE" ] || { echo "SETUP FAILED: missing installer or package"; exit 2; }
for t in python3 gcc tar sha256sum flock; do command -v "$t" >/dev/null || { echo "SETUP FAILED: $t not installed"; exit 2; }; done
REAL_MV="$(command -v mv)"; REAL_CURL="$(command -v curl)"
case "$W" in /*) ;; *) W="$PWD/$W" ;; esac
rm -rf "$W"; mkdir -p "$W/bin" || exit 2

# --- the Exec line, read the way a desktop-entry launcher reads it
cat > "$W/bin/run_exec.py" <<'PY'
import os, shlex, subprocess, sys
path, mode = sys.argv[1], sys.argv[2]
lines = open(path, encoding="utf-8").read().split("\n")
group, value = None, None
for line in lines:
    if line.startswith("#") or not line.strip():
        continue
    if line.startswith("["):
        group = line.strip()
        continue
    if group == "[Desktop Entry]" and line.split("=", 1)[0].strip() == "Exec":
        value = line.split("=", 1)[1]
        break
if value is None:
    sys.exit("no Exec key in [Desktop Entry]")
# 1. string escapes (\s \n \t \r \\)
out, i = [], 0
while i < len(value):
    c = value[i]
    if c == "\\" and i + 1 < len(value):
        n = value[i + 1]
        out.append({"s": " ", "n": "\n", "t": "\t", "r": "\r", "\\": "\\"}.get(n, "\\" + n))
        i += 2
        continue
    out.append(c); i += 1
unescaped = "".join(out)
# 2. quoting: double-quoted args with \" \` \$ \\ escapes; reserved characters only inside quotes
reserved = set(" \t\n\"'\\><~|&;$*?#()`")
args, cur, i, in_arg = [], [], 0, False
while i < len(unescaped):
    c = unescaped[i]
    if c == '"':
        i += 1; in_arg = True
        while True:
            if i >= len(unescaped):
                sys.exit("unterminated quote")
            c = unescaped[i]
            if c == "\\" and i + 1 < len(unescaped) and unescaped[i + 1] in '"`$\\':
                cur.append(unescaped[i + 1]); i += 2; continue
            if c == '"':
                i += 1; break
            cur.append(c); i += 1
        continue
    if c in " \t":
        if in_arg:
            args.append("".join(cur)); cur, in_arg = [], False
        i += 1; continue
    if c in reserved:
        sys.exit("reserved character %r outside quotes" % c)
    cur.append(c); in_arg = True; i += 1
if in_arg:
    args.append("".join(cur))
# 3. field codes: %k alone is the file's path, %% is %, nothing else is used
argv = []
for a in args:
    if a == "%k":
        argv.append(path); continue
    if "%" in a.replace("%%", ""):
        sys.exit("unexpected field code in %r" % a)
    argv.append(a.replace("%%", "%"))
if mode == "argv":
    for a in argv:
        print(a)
    sys.exit(0)
if mode == "sh-argv":
    # The same value handed to a POSIX shell (what a launcher falls back to) must give the same argv.
    line = unescaped.replace("%k", shlex.quote(path)).replace("%%", "%")
    r = subprocess.run(["sh", "-c", "set -- " + line + '; for a in "$@"; do printf "%s\\n" "$a"; done'],
                       capture_output=True)
    sys.stdout.write(r.stdout.decode())
    sys.exit(r.returncode)
sys.exit(subprocess.run(argv).returncode)
PY

# Stand-in program for the launch and running-copy cases: keeps running, writes a marker where it starts.
printf '#include <stdio.h>\n#include <unistd.h>\nint main(void){FILE*f=fopen("stub-started","w");if(f)fclose(f);for(;;)pause();}\n' > "$W/bin/stub.c"
gcc -O0 -o "$W/bin/stub" "$W/bin/stub.c" || { echo "SETUP FAILED: gcc"; exit 2; }

# Releases: the real package, stand-in packages with the stub as their program, broken ones.
make_rel() { # make_rel <dir> <tar.gz>
    mkdir -p "$1"; cp "$2" "$1/$ASSET"; (cd "$1" && sha256sum "$ASSET" > "$ASSET.sha256")
}
stub_pkg() { # stub_pkg <name> [tweak...]: build $W/<name>.tar.gz and release dir $W/<name>rel
    local d="$W/$1pkg"
    rm -rf "$d"; mkdir -p "$d/$PKG"
    cp "$W/bin/stub" "$d/$PKG/ports_launcher"; echo stub7z > "$d/$PKG/7zzs"
    printf '#!/bin/sh\n# stub updater\n' > "$d/$PKG/ports_launcher_updater.sh"
    printf 'commit=5bb5bb5aaaa\nbuild_date=2026-10-22\n' > "$d/$PKG/BUILD_INFO"
    echo '{}' > "$d/$PKG/ports.json"; echo '{"themes":1}' > "$d/$PKG/themes.json"
    case "${2:-}" in
        no7z) rm "$d/$PKG/7zzs" ;;
        link) ln -s /etc/passwd "$d/$PKG/link" ;;
        outside) echo x > "$d/extra.txt" ;;
    esac
    if [ "${2:-}" = outside ]; then tar -C "$d" -czf "$W/$1.tar.gz" "$PKG" extra.txt; else tar -C "$d" -czf "$W/$1.tar.gz" "$PKG"; fi
    make_rel "$W/$1rel" "$W/$1.tar.gz"
}
make_rel "$W/rel" "$PACKAGE"
stub_pkg stub
stub_pkg no7z no7z
stub_pkg link link
stub_pkg outside outside
make_rel "$W/badrel" "$PACKAGE"
sed -i 's/^0/X/; s/^[^X]/0/; s/^X/1/' "$W/badrel/$ASSET.sha256"   # always changes the first hex digit
mkdir -p "$W/nosumrel"; cp "$PACKAGE" "$W/nosumrel/$ASSET"
mkdir -p "$W/pkgx"; tar -xzf "$PACKAGE" -C "$W/pkgx" || { echo "SETUP FAILED: package unreadable"; exit 2; }
NEWBIN="$W/pkgx/$PKG/ports_launcher"
NEW_COMMIT="$(sed -n 's/^commit=//p' "$W/pkgx/$PKG/BUILD_INFO" | cut -c1-7)"

new_home() { rm -rf "$W/$1"; mkdir -p "$W/$1/Desktop"; H="$W/$1"; }
# run_installer <home> <release dir or URL> [VAR=value ...]: through the Exec line, no display, no launch.
# RUN_DESKTOP (another copy of the launcher), STDIN_FILE and ULIMIT_F (file size limit, KB) adjust a run.
run_installer() {
    local home="$1" rel="$2"; shift 2
    case "$rel" in *://*) ;; *) rel="file://$rel" ;; esac
    local pre=()
    [ -n "${ULIMIT_F:-}" ] && pre=(bash -c 'ulimit -f "$0" && exec "$@"' "$ULIMIT_F")
    timeout 180 "${pre[@]}" env -i HOME="$home" PATH="$PATH" LANG=C.UTF-8 DPM_RELEASE_BASE="$rel" DPM_NO_LAUNCH=1 "$@" \
        python3 "$W/bin/run_exec.py" "${RUN_DESKTOP:-$DESKTOP}" run > "$W/last.out" 2>&1 < "${STDIN_FILE:-/dev/null}"
    RC=$?
}
snap() { (cd "$1" 2>/dev/null && find . -printf '%p %y %m\n' | sort && find . -type f -exec sha256sum {} + | sort) | grep -v "$LOGNAME_"; }
# One line per path: "<path> TAB <type> <mode> [sha256]", for a diff that names what changed.
snapall() { (cd "$1" && { find . -type f -exec sha256sum {} + | sed -E 's/^([0-9a-f]{64})  (.*)$/S\t\2\t\1/'
    find . -printf 'M\t%p\t%y %m\n'; } | awk -F'\t' '$1 == "S" { s[$2] = $3; next } { print $2 "\t" $3 " " s[$2] }' | LC_ALL=C sort); }
pids_of() { for d in /proc/[0-9]*; do [ "$(readlink "$d/exe" 2>/dev/null)" = "$1" ] && echo "${d#/proc/}"; done; }
expected_exec() { python3 - "$1" <<'PY'
import sys
p = sys.argv[1]
print('Exec="' + "".join("\\" + c if c in '"`$\\' else ("%%" if c == "%" else c) for c in p) + '"')
PY
}
# The menu entry the app writes (src/ui/linux_chrome.rs desktop_entry_text) for program $1.
expected_entry() {
    printf "[Desktop Entry]\nType=Application\nName=Deck Ports Manager\n%s\nIcon=steamdeck-ports-mgr\nCategories=Game;\nTerminal=false\nStartupWMClass=ports_launcher\n" "$(expected_exec "$1")"
}
upstream_entry() { # upstream's old entry for program $1
    printf "[Desktop Entry]\nType=Application\nName=Ports Launcher\n%s\nIcon=ports_launcher\nCategories=Game;\nTerminal=false\nStartupWMClass=ports_launcher\n" "$(expected_exec "$1")"
}

echo "== 0. the launcher file itself"
python3 "$W/bin/run_exec.py" "$DESKTOP" argv > "$W/argv.txt" 2> "$W/last.out"
check "Exec line parses under the desktop-entry rules" '[ -s "$W/argv.txt" ]'
check "Exec runs bash with the launcher's own path as \$1" '[ "$(head -n 1 "$W/argv.txt")" = bash ] && [ "$(tail -n 1 "$W/argv.txt")" = "$DESKTOP" ]'
python3 "$W/bin/run_exec.py" "$DESKTOP" sh-argv > "$W/sh-argv.txt" 2>>"$W/last.out"
check "a POSIX shell reads the Exec value to the same arguments" 'cmp -s "$W/argv.txt" "$W/sh-argv.txt"'
sed -n '/^#BEGIN_DPM_INSTALLER/,/^#END_DPM_INSTALLER/s/^#|//p' "$DESKTOP" > "$W/embedded.sh"
if [ -f "$SRC" ]; then
    check "embedded script is tools/installer.sh, byte for byte" 'tr -d "\r" < "$SRC" | cmp -s - "$W/embedded.sh"'
fi
check "embedded script passes bash -n" 'bash -n "$W/embedded.sh"'
check "Terminal=false, one [Desktop Entry] group, no other group" '[ "$(grep -c "^\[" "$DESKTOP")" = 1 ] && grep -qx "Terminal=false" "$DESKTOP"'
if command -v desktop-file-validate >/dev/null; then
    cp "$DESKTOP" "$W/check.desktop"
    desktop-file-validate "$W/check.desktop" > "$W/last.out" 2>&1; v=$?
    cat "$W/last.out"
    check "desktop-file-validate accepts it" '[ "$v" = 0 ] && ! grep -q "error:" "$W/last.out"'
else
    echo "  (desktop-file-validate not installed: skipped)"
fi

echo "== 1. fresh install"
new_home h1; D="$H/Applications/$PKG"
mkdir -p "$H/.local/share/applications"
upstream_entry "$D/ports_launcher" > "$H/.local/share/applications/ports_launcher.desktop"   # from a build before 2026-10-22
mkdir -p "$H/Applications/.$PKG-installer.OLD123"; echo junk > "$H/Applications/.$PKG-installer.OLD123/x"   # a killed run's leftovers
run_installer "$H" "$W/rel"
check "exit 0 and says installed" '[ "$RC" = 0 ] && grep -q "RESULT: installed" "$W/last.out"'
check "program files there and executable" '[ -x "$D/ports_launcher" ] && [ -x "$D/7zzs" ] && [ -x "$D/ports_launcher_updater.sh" ]'
check "program is the package's, byte for byte" 'cmp -s "$D/ports_launcher" "$NEWBIN"'
check "BUILD_INFO, ports.json, themes.json there" '[ -f "$D/BUILD_INFO" ] && [ -f "$D/ports.json" ] && [ -f "$D/themes.json" ]'
check "nothing left behind in ~/Applications (an old temporary folder swept)" '[ "$(ls -A "$H/Applications")" = "$PKG" ]'
check "app-menu entry steamdeck-ports-mgr.desktop has the exact text the app writes" \
  'expected_entry "$D/ports_launcher" | cmp -s - "$H/.local/share/applications/$PKG.desktop"'
check "our old upstream-named menu entry removed, nothing else in applications/" '[ "$(ls -A "$H/.local/share/applications")" = "$PKG.desktop" ]'
check "desktop shortcut added with the same text, executable (trusted), one only" \
  '[ -x "$H/Desktop/$PKG.desktop" ] && [ "$(ls "$H/Desktop" | wc -l)" = 1 ] && cmp -s "$H/Desktop/$PKG.desktop" "$H/.local/share/applications/$PKG.desktop"'
check "log written to ~/.cache" 'grep -q "RESULT: installed" "$H/.cache/$LOGNAME_"'
check "result names the build and the menu place" 'grep -q "build $NEW_COMMIT" "$W/last.out" && grep -q "Games > Deck Ports Manager" "$W/last.out"'

echo "== 2. second run updates; only the program files change"
cd "$D" || exit 2
mkdir -p Library/game/save "Saves Backup/Update Backups" cache/art
echo save-bytes > Library/game/save/slot1.dat; echo zip > "Saves Backup/Update Backups/b.zip"
echo '{"s":1}' > state.json; echo '{"key":"x"}' > secrets.json; chmod 600 secrets.json
echo '{"f":1}' > fork-settings.json; echo '{"q":[]}' > steam-queue.json; echo '{"ports":[]}' > ports.local.json
echo '{"cat":"mine"}' > ports.json; echo '{"t":"mine"}' > themes.json; echo c > cache/art/a.png
cp "$NEWBIN" ports_launcher; echo "OLD-BUILD-MARKER" >> ports_launcher; echo old7z > 7zzs
printf 'commit=0ld0ld0\nbuild_date=2026-10-01\n' > BUILD_INFO; echo "old failure" > update-failed.txt
echo '#!/bin/sh' > ports_launcher_updater.sh
cd - >/dev/null || exit 2
cp "$D/ports_launcher" "$W/old-program"
snapall "$D" > "$W/before.all"
rm "$H/Desktop/$PKG.desktop"; echo "[Desktop Entry]" > "$H/Desktop/other.desktop"
run_installer "$H" "$W/rel"
snapall "$D" > "$W/after.all"
changed="$(diff "$W/before.all" "$W/after.all" | sed -n 's/^[<>] //p' | cut -f1 | sort -u)"
echo "$changed" | sed 's/^/      changed: /'
check "exit 0 and says updated from the old build" '[ "$RC" = 0 ] && grep -q "RESULT: updated" "$W/last.out" && grep -q "from build 0ld0ld0 to $NEW_COMMIT" "$W/last.out"'
check "new program installed" 'cmp -s "$D/ports_launcher" "$NEWBIN" && [ -x "$D/ports_launcher" ]'
check ".previous holds the old program and 7zzs" 'cmp -s "$D/.previous/ports_launcher" "$W/old-program" && grep -q old7z "$D/.previous/7zzs"'
check "7zzs, updater and BUILD_INFO are the package's" 'cmp -s "$D/7zzs" "$W/pkgx/$PKG/7zzs" && cmp -s "$D/ports_launcher_updater.sh" "$W/pkgx/$PKG/ports_launcher_updater.sh" && cmp -s "$D/BUILD_INFO" "$W/pkgx/$PKG/BUILD_INFO"'
check "whole folder: only the four program files, .previous/ and update-failed.txt changed" \
  '[ -n "$changed" ] && ! echo "$changed" | grep -Evq "^\./(ports_launcher|7zzs|ports_launcher_updater\.sh|BUILD_INFO|update-failed\.txt|\.previous(/.*)?)$" && echo "$changed" | grep -qx "./ports_launcher"'
check "stale update-failed.txt removed (as the updater does)" '[ ! -e "$D/update-failed.txt" ]'
check "a removed desktop shortcut is not added back on update" '[ ! -e "$H/Desktop/$PKG.desktop" ] && [ "$(ls "$H/Desktop")" = other.desktop ]'
check "no temporary files left" '[ -z "$(ls -A "$H/Applications" | grep -v "^$PKG$")" ] && [ -z "$(ls -A "$D" | grep "\.new$")" ]'

echo "== 3. bad packages: nothing changes"
s1="$(snap "$H")"
run_installer "$H" "$W/badrel"
check "tampered .sha256: exit 1, says failed and why" '[ "$RC" = 1 ] && grep -q "RESULT: failed" "$W/last.out" && grep -q "checksum does not match" "$W/last.out"'
check "every file in the home folder unchanged (only the log differs)" '[ "$s1" = "$(snap "$H")" ]'
run_installer "$H" "$W/nosumrel"
check "missing .sha256: checksum download failed, unchanged" '[ "$RC" = 1 ] && grep -q "checksum download failed" "$W/last.out" && [ "$s1" = "$(snap "$H")" ]'
run_installer "$H" "$W/outsiderel"
check "a file outside the package folder: refused, unchanged" '[ "$RC" = 1 ] && grep -q "outside its folder" "$W/last.out" && [ "$s1" = "$(snap "$H")" ]'
run_installer "$H" "$W/linkrel"
check "a symlink in the package: refused, unchanged" '[ "$RC" = 1 ] && grep -q "links or special files" "$W/last.out" && [ "$s1" = "$(snap "$H")" ]'
run_installer "$H" "$W/no7zrel"
check "package without 7zzs, update: refused, unchanged" '[ "$RC" = 1 ] && grep -q "missing program files" "$W/last.out" && [ "$s1" = "$(snap "$H")" ]'
new_home h3
run_installer "$H" "$W/no7zrel"
check "package without 7zzs, fresh: refused, nothing created" '[ "$RC" = 1 ] && grep -q "missing 7zzs" "$W/last.out" && [ ! -e "$H/Applications" ]'

echo "== 4. no network, bad address, full drive, outside home: nothing changes"
H="$W/h1"; s2="$(snap "$H")"
run_installer "$H" "file://$W/nothing-here"
check "missing release: exit 1, download failed (mentions a full drive too)" '[ "$RC" = 1 ] && grep -q "download failed" "$W/last.out" && grep -q "drive is full" "$W/last.out"'
check "home folder unchanged" '[ "$s2" = "$(snap "$H")" ]'
run_installer "$H" "https://127.0.0.1:9"
check "unreachable server: exit 1, download failed, unchanged" '[ "$RC" = 1 ] && grep -q "download failed" "$W/last.out" && [ "$s2" = "$(snap "$H")" ]'
ULIMIT_F=2048 run_installer "$H" "$W/rel"
check "no room for the download (2 MB file limit): exit 1, download failed, unchanged" '[ "$RC" = 1 ] && grep -q "drive is full" "$W/last.out" && [ "$s2" = "$(snap "$H")" ]'
new_home h4; rmdir "$H/Desktop"
run_installer "$H" "file://$W/nothing-here"
check "fresh home: no Applications folder or shortcut left behind" '[ "$RC" = 1 ] && [ ! -e "$H/Applications" ] && [ ! -e "$H/.local" ]'
run_installer "$H" "file://$W/nothing-here" DPM_INSTALL_DIR="$H/a/b/c/dpm"
check "deep install folder: every folder this run created removed again" '[ "$RC" = 1 ] && [ ! -e "$H/a" ]'
run_installer "$H" "$W/stubrel" DPM_INSTALL_DIR="$W/outside-home"
check "install folder outside home refused, nothing created" '[ "$RC" = 1 ] && grep -q "inside your home folder" "$W/last.out" && [ ! -e "$W/outside-home" ]'
mkdir -p "$W/sdcard"; ln -s "$W/sdcard" "$H/Applications"
run_installer "$H" "$W/stubrel"
check "~/Applications linked to an SD card: says so, nothing written there" \
  '[ "$RC" = 1 ] && grep -q "Your Applications folder is a link to $W/sdcard, which is outside your home folder (an SD card?)" "$W/last.out" && grep -q "Manual install" "$W/last.out" && [ -z "$(ls -A "$W/sdcard")" ]'

echo "== 5. a running copy from that folder is closed first; one elsewhere is left alone"
new_home h5; D="$H/Applications/$PKG"
mkdir -p "$H/.local/share/applications"
upstream_entry "$H/Applications/Ports Launcher/ports_launcher" > "$H/.local/share/applications/ports_launcher.desktop"   # the original app's
cp "$H/.local/share/applications/ports_launcher.desktop" "$W/original-entry"
run_installer "$H" "$W/stubrel"
check "the original Ports Launcher's menu entry is left byte-identical" 'cmp -s "$H/.local/share/applications/ports_launcher.desktop" "$W/original-entry"'
mkdir -p "$H/other"; cp "$W/bin/stub" "$H/other/ports_launcher"
(cd "$H/other" && setsid -f ./ports_launcher >/dev/null 2>&1 </dev/null)
(cd "$D" && setsid -f ./ports_launcher >/dev/null 2>&1 </dev/null)
sleep 0.5
check "stand-in running from the install folder" '[ -n "$(pids_of "$D/ports_launcher")" ]'
printf 'n\n' > "$W/answer-no"
s5="$(snap "$D")"
STDIN_FILE="$W/answer-no" run_installer "$H" "$W/stubrel" DPM_UI=text DPM_IN_TERMINAL=1
check "terminal window: the close question asked, \"n\" cancels and changes nothing" \
  '[ "$RC" = 2 ] && grep -q "Continue? \[Y/n\]" "$W/last.out" && grep -q "RESULT: cancelled" "$W/last.out" && [ -n "$(pids_of "$D/ports_launcher")" ] && [ "$s5" = "$(snap "$D")" ]'
run_installer "$H" "$W/stubrel"
check "exit 0" '[ "$RC" = 0 ] && grep -q "RESULT: updated" "$W/last.out"'
check "the copy from the install folder was closed (and not restarted: DPM_NO_LAUNCH)" '[ -z "$(pids_of "$D/ports_launcher")" ]'
check "the copy in another folder still runs" '[ -n "$(pids_of "$H/other/ports_launcher")" ]'
check "says it was the same build" 'grep -q "reinstalled (same build" "$W/last.out"'
for p in $(pids_of "$H/other/ports_launcher"); do kill "$p"; done

echo "== 6. launch: the app starts detached and outlives the installer"
new_home h6; D="$H/Applications/$PKG"
run_installer "$H" "$W/stubrel" DPM_NO_LAUNCH=
sleep 0.5
p="$(pids_of "$D/ports_launcher" | head -n 1)"
check "exit 0, says it is opening" '[ "$RC" = 0 ] && grep -q "It is opening now" "$W/last.out"'
check "app running from the install folder after the installer exited" '[ -n "$p" ] && [ -f "$D/stub-started" ]'
check "in its own session (closing the installer does not close it)" '[ -n "$p" ] && [ "$(ps -o sid= -p "$p" | tr -d " ")" = "$p" ]'
check "the app does not hold the installer's lock" 'flock -n "$H/.cache/steamdeck-ports-mgr-installer.lock" true'
[ -n "$p" ] && kill "$p"

echo "== 7. odd characters in the install path: menu entry still matches the app's escaping"
new_home h7; D="$H/My Apps/\$x 100% \"q\" \`b\`/dpm"
run_installer "$H" "$W/stubrel" DPM_INSTALL_DIR="$D"
check "exit 0" '[ "$RC" = 0 ] && [ -x "$D/ports_launcher" ]'
check "Exec line escaped like desktop_exec_arg" 'grep -qxF "$(expected_exec "$D/ports_launcher")" "$H/.local/share/applications/$PKG.desktop"'

echo "== 8. dialogs: kdialog, zenity and terminal (stand-ins that log their arguments)"
mkdir -p "$W/fakebin" "$W/fakekon"
cat > "$W/fakebin/kdialog" <<'EOF'
#!/bin/sh
printf 'kdialog' >> "$UILOG"; for a in "$@"; do printf ' [%s]' "$a" >> "$UILOG"; done; echo >> "$UILOG"
case "$*" in *--progressbar*) [ -n "${KD_NOPROG:-}" ] && exit 1; echo "org.kde.kdialog-4242 /ProgressDialog" ;; esac
exit 0
EOF
cat > "$W/fakebin/dbus-send" <<'EOF'
#!/bin/sh
printf 'dbus-send' >> "$UILOG"; for a in "$@"; do printf ' [%s]' "$a" >> "$UILOG"; done; echo >> "$UILOG"
[ -n "${DBUS_FAIL:-}" ] && exit 1
exit 0
EOF
cat > "$W/fakebin/zenity" <<'EOF'
#!/bin/sh
printf 'zenity' >> "$UILOG"; for a in "$@"; do printf ' [%s]' "$a" >> "$UILOG"; done; echo >> "$UILOG"
case "$*" in *--progress*) [ -n "${ZENITY_GONE:-}" ] && exit 0; while IFS= read -r l; do echo "zenity-stdin $l" >> "$UILOG"; done ;; esac
exit 0
EOF
cat > "$W/fakekon/konsole" <<'EOF'
#!/bin/sh
printf 'konsole' >> "$UILOG"; for a in "$@"; do printf ' [%s]' "$a" >> "$UILOG"; done; echo >> "$UILOG"
while [ "$#" -gt 0 ] && [ "$1" != -e ]; do shift; done; shift
exec "$@" > "$UILOG.term" 2>&1 < /dev/null
EOF
chmod 755 "$W/fakebin/"* "$W/fakekon/"*
export UILOG="$W/ui.log"
new_home h8; : > "$UILOG"
PATH="$W/fakebin:$PATH" run_installer "$H" "$W/stubrel" DISPLAY=:77 UILOG="$UILOG"
check "kdialog chosen when a display and kdialog exist; install worked" '[ "$RC" = 0 ] && [ -x "$H/Applications/$PKG/ports_launcher" ]'
check "kdialog progress bar opened, then labels and values over D-Bus, then closed" \
  'grep -q "^kdialog .*\[--progressbar\]" "$UILOG" && grep -q "\[--dest=org.kde.kdialog-4242\] \[/ProgressDialog\] \[org.kde.kdialog.ProgressDialog.setLabelText\] \[string:Downloading" "$UILOG" && grep -q "\[string:value\] \[variant:int32:62\]" "$UILOG" && grep -q "ProgressDialog.close" "$UILOG"'
check "result shown in a kdialog message box, after the progress bar closed" \
  'grep -qx "kdialog \[--title\] \[Deck Ports Manager\] \[--msgbox\] \[Deck Ports Manager is installed in ~/Applications/$PKG (build 5bb5bb5, built 2026-10-22)." "$UILOG" && [ "$(grep -n "ProgressDialog.close" "$UILOG" | tail -n 1 | cut -d: -f1)" -lt "$(grep -n -- "--msgbox" "$UILOG" | cut -d: -f1)" ]'
: > "$UILOG"
PATH="$W/fakebin:$PATH" run_installer "$H" "$W/badrel" DISPLAY=:77 UILOG="$UILOG"
check "a failure is shown in a kdialog error box" '[ "$RC" = 1 ] && grep -q "^kdialog \[--title\] \[Deck Ports Manager\] \[--error\] \[The checksum does not match" "$UILOG"'
new_home "h8 <a&b>"; : > "$UILOG"
PATH="$W/fakebin:$PATH" run_installer "$H" "$W/stubrel" DISPLAY=:77 UILOG="$UILOG" DPM_INSTALL_DIR="$H/x<y>&z"
check "kdialog text with a < goes out as escaped rich text (& < > escaped, lines as <br>)" \
  '[ "$RC" = 0 ] && grep -q "\[--msgbox\] \[<qt>Deck Ports Manager is installed in ~/x&lt;y&gt;&amp;z (build 5bb5bb5, built 2026-10-22).<br>" "$UILOG"'
new_home h8n; : > "$UILOG"
PATH="$W/fakebin:$PATH" run_installer "$H" "$W/stubrel" DISPLAY=:77 UILOG="$UILOG" KD_NOPROG=1 DBUS_FAIL=1
check "no session bus (no progress bar, dbus-send failing): installs, result still shown" \
  '[ "$RC" = 0 ] && [ -x "$H/Applications/$PKG/ports_launcher" ] && ! grep -q "^dbus-send" "$UILOG" && grep -q "\[--msgbox\]" "$UILOG"'
new_home h8d; : > "$UILOG"
PATH="$W/fakebin:$PATH" run_installer "$H" "$W/stubrel" DISPLAY=:77 UILOG="$UILOG" DBUS_FAIL=1
check "progress bar there but dbus-send failing: installs, result shown" '[ "$RC" = 0 ] && grep -q "^dbus-send" "$UILOG" && grep -q "\[--msgbox\]" "$UILOG"'
mv "$W/fakebin/kdialog" "$W/fakebin/kdialog.off"; new_home h8z; : > "$UILOG"
PATH="$W/fakebin:$PATH" run_installer "$H" "$W/stubrel" DISPLAY=:77 UILOG="$UILOG"
sleep 0.5   # the progress stand-in reads its stdin in the background
check "zenity used when there is no kdialog; progress fed through stdin up to 100" \
  '[ "$RC" = 0 ] && grep -q "^zenity .*\[--progress\]" "$UILOG" && grep -q "^zenity-stdin # Downloading" "$UILOG" && grep -q "^zenity-stdin 100$" "$UILOG"'
check "result shown in a zenity info box" 'grep -q "^zenity \[--info\] \[--title=Deck Ports Manager\] \[--width=480\] \[--text=Deck Ports Manager is installed" "$UILOG"'
new_home h8g; : > "$UILOG"
PATH="$W/fakebin:$PATH" run_installer "$H" "$W/stubrel" DISPLAY=:77 UILOG="$UILOG" ZENITY_GONE=1
check "progress window closed by the user (zenity gone): the install still completes" \
  '[ "$RC" = 0 ] && grep -q "RESULT: installed" "$H/.cache/$LOGNAME_" && [ -x "$H/Applications/$PKG/ports_launcher" ]'
mv "$W/fakebin/kdialog.off" "$W/fakebin/kdialog"
new_home h8k; : > "$UILOG"
PATH="$W/fakekon:$PATH" run_installer "$H" "$W/stubrel" DISPLAY=:77 UILOG="$UILOG"
check "no dialog program: the same steps run as text in a Konsole window" \
  '[ "$RC" = 0 ] && grep -q "^konsole \[-e\] \[bash\] \[-c\]" "$UILOG" && grep -q "RESULT: installed" "$UILOG.term" && grep -q "Press Enter to close" "$UILOG.term" && [ -x "$H/Applications/$PKG/ports_launcher" ]'

# A real desktop-entry implementation as a second opinion. (Not `gio launch`: up to at least GLib 2.72 it
# loads the file as a bare key file, so %k expands to nothing there.) KDE's KIO, the Deck's launcher,
# expands %k to the entry's path the same way; that part is only proven on a Deck.
if python3 -c 'import gi; gi.require_version("Gio", "2.0"); from gi.repository import Gio' 2>/dev/null; then
    echo "== 9. GLib's launcher (Gio.DesktopAppInfo) runs the file"
    new_home h9
    timeout 120 env -i HOME="$H" PATH="$PATH" LANG=C.UTF-8 DPM_RELEASE_BASE="file://$W/stubrel" DPM_NO_LAUNCH=1 DPM_UI=text \
        python3 -c 'import sys, gi; gi.require_version("Gio", "2.0"); from gi.repository import Gio
sys.exit(0 if Gio.DesktopAppInfo.new_from_filename(sys.argv[1]).launch([], None) else 1)' "$DESKTOP" > "$W/last.out" 2>&1
    for _ in $(seq 1 60); do grep -q "RESULT:" "$H/.cache/$LOGNAME_" 2>/dev/null && break; sleep 0.5; done
    check "GLib launch: installed through GLib's Exec and %k handling" 'grep -q "RESULT: installed" "$H/.cache/$LOGNAME_" && [ -x "$H/Applications/$PKG/ports_launcher" ]'
else
    echo "  (python3 GObject bindings not installed: GLib launch skipped)"
fi

echo "== 11. the launcher saved under browser-style names still runs itself"
mkdir -p "$W/dl"
i=0
for name in "steamdeck-ports-mgr-installer (1).desktop" "steamdeck-ports-mgr-installer(1).desktop" "it's \"q\" \$x %k \`b\`.desktop"; do
    i=$((i + 1)); cp "$DESKTOP" "$W/dl/$name"; new_home "h11-$i"
    RUN_DESKTOP="$W/dl/$name" run_installer "$H" "$W/stubrel"
    check "\"$name\": installed" '[ "$RC" = 0 ] && grep -q "RESULT: installed" "$W/last.out" && [ -x "$H/Applications/$PKG/ports_launcher" ]'
done
python3 "$W/bin/run_exec.py" "$W/dl/$name" argv > "$W/a1.txt"; python3 "$W/bin/run_exec.py" "$W/dl/$name" sh-argv > "$W/a2.txt"
check "a shell reads that last name's Exec the same way" 'cmp -s "$W/a1.txt" "$W/a2.txt" && [ "$(tail -n 1 "$W/a1.txt")" = "$W/dl/$name" ]'

echo "== 12. one installer at a time"
new_home h12; mkdir -p "$H/.cache"
flock "$H/.cache/steamdeck-ports-mgr-installer.lock" sleep 30 & holder=$!
sleep 0.3
run_installer "$H" "$W/stubrel"
check "lock held: \"already running\", exit 3, nothing created" \
  '[ "$RC" = 3 ] && grep -q "The installer is already running. This second copy did nothing." "$W/last.out" && [ ! -e "$H/Applications" ]'
kill "$holder" 2>/dev/null; wait "$holder" 2>/dev/null
mkdir -p "$W/slowcurl"; printf '#!/bin/sh\nsleep 2\nexec "%s" "$@"\n' "$REAL_CURL" > "$W/slowcurl/curl"; chmod 755 "$W/slowcurl/curl"
new_home h12b
for n in 1 2; do
    (timeout 120 env -i HOME="$H" PATH="$W/slowcurl:$PATH" LANG=C.UTF-8 DPM_RELEASE_BASE="file://$W/stubrel" DPM_NO_LAUNCH=1 \
        python3 "$W/bin/run_exec.py" "$DESKTOP" run > "$W/c$n.out" 2>&1 </dev/null) &
    eval "cp$n=\$!"; sleep 0.5
done
wait "$cp1"; wait "$cp2"
cat "$W/c1.out" "$W/c2.out" > "$W/last.out"
check "two at once: one installed, the other said it was already running" \
  '[ "$(grep -c "RESULT: installed" "$W/last.out")" = 1 ] && [ "$(grep -c "RESULT: already running" "$W/last.out")" = 1 ] && [ -x "$H/Applications/$PKG/ports_launcher" ]'
check "no temporary folders left" '[ "$(ls -A "$H/Applications")" = "$PKG" ]'

echo "== 13. desktop launcher already there; a folder that exists without a program"
new_home h13; D="$H/Applications/$PKG"
expected_entry "$D/ports_launcher" | sed 's/^Name=.*/Name=My own launcher/' > "$H/Desktop/mine.desktop"
cp "$H/Desktop/mine.desktop" "$W/mine.copy"
mkdir -p "$D"; echo notes > "$D/notes.txt"; echo '{"cat":"kept"}' > "$D/ports.json"
run_installer "$H" "$W/stubrel"
check "folder without a program: program added, says installed" '[ "$RC" = 0 ] && grep -q "RESULT: installed" "$W/last.out" && [ -x "$D/ports_launcher" ] && [ -x "$D/7zzs" ]'
check "its own files kept, missing starter catalog added" \
  'grep -q kept "$D/ports.json" && grep -q notes "$D/notes.txt" && grep -q themes "$D/themes.json"'
check "a desktop launcher for this program already there: no second one, it is untouched" \
  '[ "$(ls "$H/Desktop")" = mine.desktop ] && cmp -s "$H/Desktop/mine.desktop" "$W/mine.copy"'

echo "== 14. stopped (TERM) in the middle of the swap: the previous program is put back"
new_home h14; D="$H/Applications/$PKG"
run_installer "$H" "$W/stubrel"
cp "$W/bin/stub" "$D/ports_launcher"; echo "OLD" >> "$D/ports_launcher"; echo old7z > "$D/7zzs"
cp "$D/ports_launcher" "$W/old14"
mkdir -p "$W/fakemv"
printf '#!/bin/sh\ncase "$*" in *.7zzs.new*) kill -TERM "$PPID" ;; esac\nexec "%s" "$@"\n' "$REAL_MV" > "$W/fakemv/mv"; chmod 755 "$W/fakemv/mv"
PATH="$W/fakemv:$PATH" run_installer "$H" "$W/stubrel"
check "exit 130, says interrupted" '[ "$RC" = 130 ] && grep -q "RESULT: interrupted" "$W/last.out"'
check "both program files are the previous ones again, no .new files" \
  'cmp -s "$D/ports_launcher" "$W/old14" && grep -q old7z "$D/7zzs" && [ -z "$(ls -A "$D" | grep -E "\.(new|restore)$")" ]'

# The real app, started by the installer on a virtual display, must find its menu entry already right:
# its own register_desktop_entry rewrites the file only when the bytes differ.
XVFB=""
if command -v Xvfb >/dev/null && command -v xdpyinfo >/dev/null; then
    echo "== 10. the real app started by the installer keeps the installer's menu entry"
    for _ in $(seq 1 20); do
        n=$((100 + RANDOM % 800)); [ -e "/tmp/.X$n-lock" ] && continue
        Xvfb ":$n" -screen 0 1280x800x24 -nolisten tcp >"$W/xvfb.log" 2>&1 &
        XVFB=$!
        for _ in $(seq 1 25); do DISPLAY=":$n" xdpyinfo >/dev/null 2>&1 && break; kill -0 "$XVFB" 2>/dev/null || break; sleep 0.2; done
        kill -0 "$XVFB" 2>/dev/null && DISPLAY=":$n" xdpyinfo >/dev/null 2>&1 && break
        kill "$XVFB" 2>/dev/null; XVFB=""; sleep 0.5
    done
    if [ -n "$XVFB" ]; then
        new_home h10; D="$H/Applications/$PKG"; mkdir -m 700 "$H/run"
        run_installer "$H" "$W/rel" DPM_NO_LAUNCH= DPM_UI=text DISPLAY=":$n" XDG_RUNTIME_DIR="$H/run"
        menu="$H/.local/share/applications/$PKG.desktop"
        # Case 1 proved the installer writes this text; here the app must end up with the very same.
        expected_entry "$D/ports_launcher" > "$W/menu.before"
        check "exit 0, the real app is running" '[ "$RC" = 0 ] && grep -q "It is opening now" "$W/last.out" && [ -n "$(pids_of "$D/ports_launcher")" ]'
        for _ in $(seq 1 40); do ls "$H"/.local/share/icons/hicolor/*/apps/$PKG.png >/dev/null 2>&1 && break; sleep 0.5; done
        sleep 1
        check "the app ran its own desktop registration (steamdeck-ports-mgr.png written)" 'ls "$H"/.local/share/icons/hicolor/*/apps/$PKG.png >/dev/null 2>&1'
        check "and its menu entry is byte-identical to the installer's (one entry, no upstream-named one)" \
          'cmp -s "$menu" "$W/menu.before" && [ "$(ls "$H/.local/share/applications")" = "$PKG.desktop" ]'
        for p in $(pids_of "$D/ports_launcher"); do kill "$p"; done
    else
        echo "  (no Xvfb display came up: skipped)"
    fi
else
    echo "  (Xvfb not installed: real-app check skipped)"
fi

for d in /proc/[0-9]*; do case "$(readlink "$d/exe" 2>/dev/null)" in "$W"/*) kill "${d#/proc/}" 2>/dev/null ;; esac; done
[ -n "$XVFB" ] && kill "$XVFB" 2>/dev/null
rm -rf "$W"
echo "RESULT: $pass passed, $failn failed"
[ "$failn" -eq 0 ]
