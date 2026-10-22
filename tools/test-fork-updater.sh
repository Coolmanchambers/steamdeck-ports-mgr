#!/usr/bin/env bash
# Tests tools/fork-updater.sh against fake installs and fake releases. Linux only; needs gcc.
# Local: bash tools/test-fork-updater.sh   CI runs it in ci.yml. Works only inside $T, which it deletes at the end.
set -u
SRC="${SRC:-$(dirname "$(readlink -f "$0")")/fork-updater.sh}"
T="${T:-$HOME/dpm-test/self-updater}"
PKG=steamdeck-ports-mgr
ASSET=$PKG-linux-x86_64.tar.gz
pass=0; failn=0
ok()  { echo "  PASS $1"; pass=$((pass+1)); }
bad() { echo "  FAIL $1"; failn=$((failn+1)); diag; }
check() { if eval "$2"; then ok "$1"; else bad "$1"; fi; }
app_running() { for p in $(pgrep -x ports_launcher); do [ "$(readlink -f /proc/$p/exe)" = "$T/app/ports_launcher" ] && return 0; done; return 1; }
count_app() { n=0; for p in $(pgrep -x ports_launcher); do [ "$(readlink -f /proc/$p/exe)" = "$T/app/ports_launcher" ] && n=$((n+1)); done; echo $n; }

# On a failed check: what was running at that moment (CI flaked once on the copy count, 2026-10-21, and the
# old output could not say why). Every process whose program or working folder is under $T, with its age.
diag() {
  echo "    count_app=$(count_app)"
  echo "    processes under $T (pid ppid state age-s command [program] [folder]):"
  for d in /proc/[0-9]*; do
    p=${d#/proc/}
    exe=$(readlink -f "$d/exe" 2>/dev/null); cwd=$(readlink -f "$d/cwd" 2>/dev/null)
    case "$exe|$cwd" in
      "$T"/*|*"|$T"/*) echo "      $(ps -o pid=,ppid=,stat=,etimes=,args= -p "$p" 2>/dev/null) [$exe] [$cwd]" ;;
    esac
  done
  [ -f "$T/app/updater.log" ] && { echo "    updater.log, last lines:"; tail -n 6 "$T/app/updater.log" | sed 's/^/      /'; }
}

stop_all() { for p in $(pgrep -x ports_launcher); do case "$(readlink -f /proc/$p/exe)" in "$T"/*) kill "$p" 2>/dev/null;; esac; done; sleep 1; }

# Stand-ins that behave like the real program: compiled binaries that keep running with no arguments.
build_fakes() {
  mkdir -p "$T/bin"
  printf '#include <unistd.h>\nint main(void){for(;;)pause();}\n' > "$T/bin/old.c"
  printf '#include <stdio.h>\n#include <unistd.h>\nint main(void){FILE*f=fopen("new-started","w");if(f)fclose(f);for(;;)pause();}\n' > "$T/bin/new.c"
  # Like the real app after Force update: still closing when the updater starts, gone 2 s later.
  # Only the first start (marked by quit-once) does this; a restart keeps running.
  printf '#include <unistd.h>\nint main(void){if(access("quit-once",F_OK)==0){unlink("quit-once");sleep(2);return 0;}for(;;)pause();}\n' > "$T/bin/slow.c"
  gcc -O0 -o "$T/bin/old" "$T/bin/old.c" && gcc -O0 -o "$T/bin/new" "$T/bin/new.c" && gcc -O0 -o "$T/bin/slow" "$T/bin/slow.c"
}

make_app() {  # $1 = running | quit (the real app quits itself before the updater runs)
  stop_all; rm -rf "$T/app" "$T/other"; mkdir -p "$T/app/Library/game" "$T/app/Saves Backup" "$T/app/cache" "$T/other"
  cp "$T/bin/old" "$T/other/ports_launcher"; ( "$T/other/ports_launcher" >/dev/null 2>&1 & )
  cd "$T/app" || exit 1
  cp "$T/bin/old" ports_launcher; echo old7z > 7zzs
  echo save-bytes > Library/game/save.dat; echo zip > "Saves Backup/b.zip"
  echo '{"s":1}' > state.json; echo '{"cat":"old"}' > ports.json; echo '{"t":1}' > themes.json
  echo '{"local":1}' > ports.local.json; echo c > cache/a
  tr -d '\r' < "$SRC" > ports_launcher_updater.sh; chmod 755 ports_launcher_updater.sh
  [ "$1" = running ] && ( ./ports_launcher >/dev/null 2>&1 & )
  sleep 1
  find Library "Saves Backup" cache state.json ports.json themes.json ports.local.json -type f -exec sha256sum {} + | sort > "$T/protected.before"
}

make_release() {  # $1 = good | badsum | missing
  rm -rf "$T/rel"; mkdir -p "$T/rel/b/$PKG"; cd "$T/rel/b" || exit 1
  cp "$T/bin/new" "$PKG/ports_launcher"
  echo new7z > "$PKG/7zzs"
  { tr -d '\r' < "$SRC"; echo "# v2"; } > "$PKG/ports_launcher_updater.sh"
  printf 'commit=abc123\nbuild_date=2026-10-21\n' > "$PKG/BUILD_INFO"
  echo '{"cat":"NEW, must not be installed"}' > "$PKG/ports.json"
  [ "$1" = missing ] && rm "$PKG/7zzs"
  tar -czf "$T/rel/$ASSET" "$PKG"
  (cd "$T/rel" && sha256sum "$ASSET" > "$ASSET.sha256")
  [ "$1" = badsum ] && sed -i 's/^0/X/; s/^[^X]/0/; s/^X/1/' "$T/rel/$ASSET.sha256"  # always changes the first hex digit
}

run_updater() { (cd "$T/app" && DPM_RELEASE_BASE="file://$T/rel" sh ./ports_launcher_updater.sh); sleep 3; }
protected_same() { (cd "$T/app" && find Library "Saves Backup" cache state.json ports.json themes.json ports.local.json -type f -exec sha256sum {} + | sort) | diff -q - "$T/protected.before" >/dev/null; }
other_alive() { for p in $(pgrep -x ports_launcher); do [ "$(readlink -f /proc/$p/exe)" = "$T/other/ports_launcher" ] && return 0; done; return 1; }

rm -rf "$T"; build_fakes || { echo "could not compile stand-ins"; exit 1; }

echo "== 1. good update, app still running when the updater starts"
make_app running; echo "old failure" > "$T/app/update-failed.txt"; make_release good; run_updater
check "new ports_launcher installed" 'cmp -s "$T/app/ports_launcher" "$T/bin/new"'
check "exactly one copy running, and it is the new one" '[ "$(count_app)" = 1 ] && [ -f "$T/app/new-started" ]'
check "new 7zzs installed" 'grep -q new7z "$T/app/7zzs"'
check "old files kept in .previous" 'grep -q old7z "$T/app/.previous/7zzs" && cmp -s "$T/app/.previous/ports_launcher" "$T/bin/old"'
check "BUILD_INFO installed" 'grep -q commit=abc123 "$T/app/BUILD_INFO"'
check "old update-failed.txt removed on success" '[ ! -f "$T/app/update-failed.txt" ]'
check "updater replaced itself" 'grep -q "^# v2" "$T/app/ports_launcher_updater.sh"'
check "protected files byte-identical (Library, saves, state, catalogs, cache)" 'protected_same'
check "Ports Launcher in another folder left running" 'other_alive'
check "log says done" 'grep -q "=== done" "$T/app/updater.log"'

echo "== 2. tampered checksum, app already quit (the real case)"
make_app quit; make_release badsum; run_updater
check "old version reopened" '[ "$(count_app)" = 1 ] && cmp -s "$T/app/ports_launcher" "$T/bin/old"'
check "7zzs unchanged" 'grep -q old7z "$T/app/7zzs"'
check "update-failed.txt explains" 'grep -q "checksum does not match" "$T/app/update-failed.txt"'
check "protected files byte-identical" 'protected_same'
check "Ports Launcher in another folder left running" 'other_alive'

echo "== 3. package missing 7zzs, app still running"
make_app running; make_release missing; run_updater
check "app still running, not started twice" '[ "$(count_app)" = 1 ]'
check "7zzs unchanged" 'grep -q old7z "$T/app/7zzs"'
check "update-failed.txt explains" 'grep -q "missing program files" "$T/app/update-failed.txt"'

echo "== 4. download fails (no release at that address), app already quit"
make_app quit; rm -rf "$T/rel"; mkdir -p "$T/rel"; run_updater
check "old version reopened" '[ "$(count_app)" = 1 ]'
check "update-failed.txt explains" 'grep -q "download failed" "$T/app/update-failed.txt"'

echo "== 5. download fails instantly while the app is still closing"
make_app quit; cp "$T/bin/slow" "$T/app/ports_launcher"; touch "$T/app/quit-once"
( cd "$T/app" && ./ports_launcher >/dev/null 2>&1 & ); sleep 0.3
rm -rf "$T/rel"; mkdir -p "$T/rel"; run_updater
check "app reopened once it had finished closing" '[ "$(count_app)" = 1 ]'
check "update-failed.txt explains" 'grep -q "download failed" "$T/app/update-failed.txt"'

echo "== last log lines (case 5)"; tail -2 "$T/app/updater.log"
stop_all; rm -rf "$T"
echo "RESULT: $pass passed, $failn failed"
[ "$failn" -eq 0 ]
